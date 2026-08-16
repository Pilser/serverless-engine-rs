use engine::model::Key;
use engine::storage::database::{Cursor, Database, DatabaseCaps, Query, Row, TtlClause};
use engine::storage::ir::{normalize_path, scalar_text, Agg, FilterCond, Op, SrvFilter};
use engine::tables::{
    TABLE_APPS, TABLE_AUDIT, TABLE_HOOK_DELIVERIES, TABLE_INDEX, TABLE_JOBS, TABLE_JOB_RUNS,
    TABLE_KEYS, TABLE_LINKS, TABLE_RECORDS, TABLE_SESSIONS, TABLE_USERS,
};
use serde_json::{json, Value as Json};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use tokio::runtime::{Builder, Runtime};
use turso::params::Params;
use turso::Value;

const SYSTEM_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS srv_board_index (
    board_id TEXT PRIMARY KEY,
    owner_key TEXT NOT NULL,
    created_at TEXT NOT NULL
);
";

const BOARD_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS wb_records (
    seq INTEGER PRIMARY KEY,
    board_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    writer TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_wb_records_board ON wb_records(board_id);
";

fn is_system(table: &str) -> bool {
    matches!(
        table,
        TABLE_KEYS
            | TABLE_JOBS
            | TABLE_JOB_RUNS
            | TABLE_HOOK_DELIVERIES
            | TABLE_INDEX
            | TABLE_USERS
            | TABLE_SESSIONS
    )
}

fn kv_key(table: &str, k: &Key) -> String {
    let kr = match k {
        Key::Int(v) => format!("int:{v}"),
        Key::Text(s) => format!("text:{s}"),
    };
    format!("{table}/{kr}")
}

fn board_from_key(table: &str, k: &Key) -> Option<String> {
    let Key::Text(s) = k else {
        return None;
    };
    if table == TABLE_APPS || table == TABLE_AUDIT || table == TABLE_LINKS {
        return Some(s.clone());
    }
    s.split_once('/').map(|(b, _)| b.to_string())
}

fn board_from_filter(f: &SrvFilter) -> Option<String> {
    for c in &f.conds {
        if c.field == "$.board_id" && c.op == Op::Eq {
            if let Some(s) = c.value.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn filter_owner(f: &SrvFilter) -> Option<String> {
    for c in &f.conds {
        if c.field == "$.owner_key" && c.op == Op::Eq {
            if let Some(s) = c.value.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn like_fragment(raw: &Json) -> String {
    let mut out = String::new();
    for ch in scalar_text(raw).chars() {
        if ch == '\\' || ch == '%' || ch == '_' {
            out.push('\\');
        }
        out.push(ch);
    }
    format!("%{}%", out)
}

fn records_where(
    board_id: &str,
    conds: &[FilterCond],
    ttl: Option<&TtlClause>,
) -> anyhow::Result<(String, Vec<Value>, Option<usize>)> {
    let mut params = vec![Value::from(board_id)];
    let mut parts = vec!["board_id = ?1".to_string()];
    let mut search_ph: Option<usize> = None;

    for c in conds {
        if c.op == Op::Search {
            let qph = params.len() + 1;
            params.push(Value::from(scalar_text(&c.value)));
            parts.push(format!("fts_match(payload, ?{qph})"));
            search_ph = Some(qph);
            continue;
        }
        let column = match c.field.as_str() {
            "$.seq" => Some("seq"),
            "$.writer" => Some("writer"),
            "$.created_at" => Some("created_at"),
            "$.board_id" => None,
            _ => None,
        };
        if c.field == "$.board_id" {
            continue;
        }
        if let Some(col) = column {
            let vp = params.len() + 1;
            let frag = match c.op {
                Op::Eq => {
                    params.push(col_val(col, &c.value));
                    format!("{col} = ?{vp}")
                }
                Op::Ne => {
                    params.push(col_val(col, &c.value));
                    format!("{col} <> ?{vp}")
                }
                Op::Gt => {
                    params.push(col_val(col, &c.value));
                    format!("{col} > ?{vp}")
                }
                Op::Gte => {
                    params.push(col_val(col, &c.value));
                    format!("{col} >= ?{vp}")
                }
                Op::Lt => {
                    params.push(col_val(col, &c.value));
                    format!("{col} < ?{vp}")
                }
                Op::Lte => {
                    params.push(col_val(col, &c.value));
                    format!("{col} <= ?{vp}")
                }
                Op::Contains => {
                    params.push(Value::from(like_fragment(&c.value)));
                    format!("LOWER({col}) LIKE LOWER(?{vp}) ESCAPE '\\'")
                }
                Op::NotContains => {
                    params.push(Value::from(like_fragment(&c.value)));
                    format!("LOWER({col}) NOT LIKE LOWER(?{vp}) ESCAPE '\\'")
                }
                Op::In => {
                    let arr = c.value.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                    let mut phs = Vec::new();
                    for el in arr {
                        let p = params.len() + 1;
                        params.push(col_val(col, el));
                        phs.push(format!("?{p}"));
                    }
                    format!("{col} IN ({})", phs.join(", "))
                }
                Op::Search => unreachable!(),
            };
            parts.push(frag);
            continue;
        }
        let path_ph = params.len() + 1;
        params.push(Value::from(normalize_path(&c.field)));
        let cast_text = format!("CAST(json_extract(payload, ?{path_ph}) AS TEXT)");
        let cast_num = format!("CAST(json_extract(payload, ?{path_ph}) AS REAL)");
        let target = if c.value.is_number() { cast_num.clone() } else { cast_text.clone() };
        let vp = params.len() + 1;
        let frag = match c.op {
            Op::Search => unreachable!(),
            Op::Eq => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{cast_text} = ?{vp}")
            }
            Op::Ne => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{cast_text} <> ?{vp}")
            }
            Op::Gt => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{target} > ?{vp}")
            }
            Op::Gte => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{target} >= ?{vp}")
            }
            Op::Lt => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{target} < ?{vp}")
            }
            Op::Lte => {
                params.push(Value::from(scalar_text(&c.value)));
                format!("{target} <= ?{vp}")
            }
            Op::Contains => {
                params.push(Value::from(like_fragment(&c.value)));
                format!("LOWER({cast_text}) LIKE LOWER(?{vp}) ESCAPE '\\'")
            }
            Op::NotContains => {
                params.push(Value::from(like_fragment(&c.value)));
                format!("LOWER({cast_text}) NOT LIKE LOWER(?{vp}) ESCAPE '\\'")
            }
            Op::In => {
                let arr = c.value.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                let mut phs = Vec::new();
                for el in arr {
                    let p = params.len() + 1;
                    params.push(Value::from(scalar_text(el)));
                    phs.push(format!("?{p}"));
                }
                format!("{cast_text} IN ({})", phs.join(", "))
            }
        };
        parts.push(frag);
    }

    if let Some(t) = ttl {
        if let Some(frag) = records_ttl_fragment(t, &mut params) {
            parts.push(format!("NOT ({frag})"));
        }
    }

    Ok((parts.join(" AND "), params, search_ph))
}

fn records_ttl_fragment(t: &TtlClause, params: &mut Vec<Value>) -> Option<String> {
    let mut bits = Vec::new();
    if let Some(n) = t.seconds {
        if n > 0 {
            let mp = params.len() + 1;
            params.push(Value::from(format!("-{n} seconds")));
            bits.push(format!("created_at < datetime('now', ?{mp})"));
        }
    }
    if let Some(f) = &t.field {
        let p1 = params.len() + 1;
        let p2 = params.len() + 2;
        params.push(Value::from(normalize_path(f)));
        params.push(Value::from(normalize_path(f)));
        bits.push(format!(
            "json_extract(payload, ?{p1}) IS NOT NULL AND CAST(json_extract(payload, ?{p2}) AS TEXT) < datetime('now')"
        ));
    }
    if bits.is_empty() {
        None
    } else {
        Some(bits.join(" OR "))
    }
}

fn col_val(col: &str, v: &Json) -> Value {
    if col == "seq" {
        Value::Integer(v.as_i64().unwrap_or_else(|| scalar_text(v).parse().unwrap_or(0)))
    } else {
        Value::from(scalar_text(v))
    }
}

fn compare_scalar(a: &Json, b: &Json) -> Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => scalar_text(a).cmp(&scalar_text(b)),
    }
}

fn compare_rows(a: &Row, b: &Row, orders: &[(String, bool)]) -> Ordering {
    for (field, desc) in orders {
        let va = engine::expr::get_path(&a.data, field);
        let vb = engine::expr::get_path(&b.data, field);
        let ord = compare_scalar(&va, &vb);
        if ord != Ordering::Equal {
            return if *desc { ord.reverse() } else { ord };
        }
    }
    b.key.cmp(&a.key)
}

fn sort_and_slice(mut rows: Vec<Row>, orders: &[(String, bool)], limit: usize, offset: usize) -> Cursor {
    if orders.is_empty() {
        rows.sort_by(|a, b| b.key.cmp(&a.key));
    } else {
        rows.sort_by(|a, b| compare_rows(a, b, orders));
    }
    let start = offset.min(rows.len());
    let has_more = start + limit < rows.len();
    let end = (start + limit).min(rows.len());
    Cursor { rows: rows[start..end].to_vec(), has_more }
}

struct DbPool {
    conn: Arc<turso::Connection>,
    _db: turso::Database,
}

fn open_pool(rt: &Runtime, path: &str) -> anyhow::Result<DbPool> {
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let db = rt
        .block_on(turso::Builder::new_local(path).experimental_index_method(true).build())?;
    let conn = db.connect()?;
    Ok(DbPool { conn: Arc::new(conn), _db: db })
}

struct Worker {
    rt: Runtime,
    data_dir: PathBuf,
    system: Option<Arc<DbPool>>,
    boards: HashMap<String, Arc<DbPool>>,
    registry: HashMap<String, (String, String)>,
}

type Job = Box<dyn FnOnce(&mut Worker) + Send + 'static>;

impl Worker {
    fn new(data_dir: PathBuf) -> Self {
        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        Self {
            rt,
            data_dir,
            system: None,
            boards: HashMap::new(),
            registry: HashMap::new(),
        }
    }

    fn system(&mut self) -> anyhow::Result<Arc<DbPool>> {
        if let Some(p) = &self.system {
            return Ok(p.clone());
        }
        let path = self.data_dir.join("system.db");
        let pool = Arc::new(open_pool(&self.rt, &path.to_string_lossy())?);
        let conn = pool.conn.clone();
        self.rt.block_on(async {
            conn.execute_batch(SYSTEM_SCHEMA).await?;
            let mut rows = conn
                .query(
                    "SELECT board_id, owner_key, created_at FROM srv_board_index",
                    Params::Positional(vec![]),
                )
                .await?;
            let mut reg = HashMap::new();
            while let Some(r) = rows.next().await? {
                let b = match r.get_value(0)? {
                    Value::Text(s) => s,
                    _ => continue,
                };
                let o = match r.get_value(1)? {
                    Value::Text(s) => s,
                    _ => continue,
                };
                let c = match r.get_value(2)? {
                    Value::Text(s) => s,
                    _ => String::new(),
                };
                reg.insert(b, (o, c));
            }
            self.registry = reg;
            Ok::<_, anyhow::Error>(())
        })?;
        self.system = Some(pool.clone());
        Ok(pool)
    }

    fn board(&mut self, board_id: &str) -> anyhow::Result<Arc<DbPool>> {
        if let Some(p) = self.boards.get(board_id) {
            return Ok(p.clone());
        }
        let dir = self.data_dir.join("boards");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{board_id}.db"));
        let pool = Arc::new(open_pool(&self.rt, &path.to_string_lossy())?);
        let conn = pool.conn.clone();
        self.rt.block_on(async {
            conn.execute_batch(BOARD_SCHEMA).await?;
            Ok::<_, anyhow::Error>(())
        })?;
        self.boards.insert(board_id.to_string(), pool.clone());
        Ok(pool)
    }

    fn register_board(&mut self, board_id: &str, owner_key: &str, created_at: &str) -> anyhow::Result<()> {
        let pool = self.system()?;
        let conn = pool.conn.clone();
        let b = board_id.to_string();
        let o = owner_key.to_string();
        let c = created_at.to_string();
        self.rt.block_on(async {
            conn.execute(
                "INSERT OR REPLACE INTO srv_board_index (board_id, owner_key, created_at) VALUES (?1, ?2, ?3)",
                Params::Positional(vec![Value::from(b.clone()), Value::from(o.clone()), Value::from(c.clone())]),
            )
            .await?;
            Ok::<_, anyhow::Error>(())
        })?;
        self.registry.insert(b, (o, c));
        Ok(())
    }

    fn unregister_board(&mut self, board_id: &str) -> anyhow::Result<()> {
        let pool = self.system()?;
        let conn = pool.conn.clone();
        let b = board_id.to_string();
        self.rt.block_on(async {
            conn.execute(
                "DELETE FROM srv_board_index WHERE board_id = ?1",
                Params::Positional(vec![Value::from(b.clone())]),
            )
            .await?;
            Ok::<_, anyhow::Error>(())
        })?;
        self.registry.remove(board_id);
        self.boards.remove(board_id);
        let base = self.data_dir.join("boards").join(format!("{board_id}.db"));
        let _ = std::fs::remove_file(&base);
        let _ = std::fs::remove_file(base.with_extension("db-wal"));
        let _ = std::fs::remove_file(base.with_extension("db-shm"));
        Ok(())
    }

    fn kv_put(&mut self, pool: &Arc<DbPool>, key: &str, data: &Json) -> anyhow::Result<()> {
        let conn = pool.conn.clone();
        let k = key.to_string();
        let d = data.to_string();
        self.rt.block_on(async {
            conn.execute(
                "INSERT OR REPLACE INTO kv (key, data) VALUES (?1, ?2)",
                Params::Positional(vec![Value::from(k), Value::from(d)]),
            )
            .await?;
            Ok::<_, anyhow::Error>(())
        })
    }

    fn kv_get(&mut self, pool: &Arc<DbPool>, key: &str) -> anyhow::Result<Option<Json>> {
        let conn = pool.conn.clone();
        let k = key.to_string();
        self.rt.block_on(async {
            let mut rows = conn
                .query(
                    "SELECT data FROM kv WHERE key = ?1",
                    Params::Positional(vec![Value::from(k)]),
                )
                .await?;
            match rows.next().await? {
                Some(r) => match r.get_value(0)? {
                    Value::Text(s) => Ok(Some(
                        serde_json::from_str(&s).unwrap_or(Json::Null),
                    )),
                    _ => Ok(None),
                },
                None => Ok(None),
            }
        })
    }

    fn kv_delete_keys(&mut self, pool: &Arc<DbPool>, keys: Vec<String>) -> anyhow::Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let conn = pool.conn.clone();
        self.rt.block_on(async {
            for key in keys {
                conn.execute(
                    "DELETE FROM kv WHERE key = ?1",
                    Params::Positional(vec![Value::from(key)]),
                )
                .await?;
            }
            Ok::<_, anyhow::Error>(())
        })
    }

    fn kv_rows(&mut self, pool: &Arc<DbPool>, table: &str) -> anyhow::Result<Vec<Row>> {
        let conn = pool.conn.clone();
        let prefix = format!("{table}/");
        self.rt.block_on(async {
            let mut rows = conn
                .query("SELECT key, data FROM kv ORDER BY key", Params::Positional(vec![]))
                .await?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().await? {
                let (Some(key), Some(data)) = (row_text(&r, 0)?, row_text(&r, 1)?) else {
                    continue;
                };
                if !key.starts_with(&prefix) {
                    continue;
                }
                if let Ok(data) = serde_json::from_str::<Json>(&data) {
                    let k = key
                        .strip_prefix(&prefix)
                        .map(parse_key_text)
                        .unwrap_or(Key::text(&key));
                    out.push(Row::new(k, data));
                }
            }
            Ok::<_, anyhow::Error>(out)
        })
    }

    fn records_all(&mut self, pool: &Arc<DbPool>, board_id: &str) -> anyhow::Result<Vec<Row>> {
        let conn = pool.conn.clone();
        let b = board_id.to_string();
        self.rt.block_on(async {
            let mut rows = conn
                .query(
                    "SELECT seq, payload, writer, created_at FROM wb_records WHERE board_id = ?1 ORDER BY seq",
                    Params::Positional(vec![Value::from(b)]),
                )
                .await?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().await? {
                let seq = match r.get_value(0)? {
                    Value::Integer(i) => i,
                    _ => continue,
                };
                let payload = row_text(&r, 1)?.unwrap_or_else(|| "null".to_string());
                let writer = row_text(&r, 2)?;
                let created = row_text(&r, 3)?.unwrap_or_default();
                let data = json!({
                    "board_id": board_id,
                    "seq": seq,
                    "payload": serde_json::from_str::<Json>(&payload).unwrap_or(Json::Null),
                    "created_at": created,
                    "writer": writer,
                });
                out.push(Row::new(Key::int(seq), data));
            }
            Ok::<_, anyhow::Error>(out)
        })
    }

    fn records_query_sql(
        &mut self,
        pool: &Arc<DbPool>,
        board_id: &str,
        q: &Query,
    ) -> anyhow::Result<Cursor> {
        let (where_sql, mut params, search_ph) = records_where(board_id, &q.filter.conds, q.ttl.as_ref())?;
        let mut sql = format!(
            "SELECT seq, payload, writer, created_at FROM wb_records WHERE {where_sql}"
        );
        if let Some(ph) = search_ph {
            sql.push_str(&format!(" ORDER BY fts_score(payload, ?{ph}) DESC"));
        } else if !q.orders.is_empty() {
            let mut terms = Vec::new();
            for (field, desc) in &q.orders {
                let op = params.len() + 1;
                params.push(Value::from(normalize_path(field)));
                let dir = if *desc { "DESC" } else { "ASC" };
                terms.push(format!("json_extract(payload, ?{op}) {dir}"));
            }
            terms.push("seq DESC".to_string());
            sql.push_str(&format!(" ORDER BY {}", terms.join(", ")));
        } else {
            sql.push_str(" ORDER BY seq DESC");
        }
        let cap = i64::MAX as usize;
        if q.limit < cap {
            let lp = params.len() + 1;
            params.push(Value::Integer(q.limit as i64));
            sql.push_str(&format!(" LIMIT ?{lp}"));
        }
        if q.offset > 0 {
            let op = params.len() + 1;
            params.push(Value::Integer(q.offset as i64));
            sql.push_str(&format!(" OFFSET ?{op}"));
        }
        let conn = pool.conn.clone();
        self.rt.block_on(async {
            let mut rows = conn.query(&sql, Params::Positional(params)).await?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().await? {
                let seq = match r.get_value(0)? {
                    Value::Integer(i) => i,
                    _ => continue,
                };
                let payload = row_text(&r, 1)?.unwrap_or_else(|| "null".to_string());
                let writer = row_text(&r, 2)?;
                let created = row_text(&r, 3)?.unwrap_or_default();
                let data = json!({
                    "board_id": board_id,
                    "seq": seq,
                    "payload": serde_json::from_str::<Json>(&payload).unwrap_or(Json::Null),
                    "created_at": created,
                    "writer": writer,
                });
                out.push(Row::new(Key::int(seq), data));
            }
            Ok::<_, anyhow::Error>(Cursor { rows: out, has_more: false })
        })
    }

    fn records_aggregate_sql(
        &mut self,
        pool: &Arc<DbPool>,
        board_id: &str,
        filter: &SrvFilter,
        agg: Agg,
        field: Option<&str>,
        group_by: Option<&str>,
        ttl: Option<&TtlClause>,
    ) -> anyhow::Result<Vec<Json>> {
        let (where_sql, mut params, _) = records_where(board_id, &filter.conds, ttl)?;
        let target = if agg == Agg::Count {
            "COUNT(*)".to_string()
        } else {
            let f = field.ok_or_else(|| anyhow::anyhow!("field required for {agg:?}"))?;
            let ph = params.len() + 1;
            params.push(Value::from(normalize_path(f)));
            let name = match agg {
                Agg::Count => "COUNT",
                Agg::Sum => "SUM",
                Agg::Avg => "AVG",
                Agg::Min => "MIN",
                Agg::Max => "MAX",
            };
            format!("{name}(CAST(json_extract(payload, ?{ph}) AS REAL))")
        };
        let sql = match group_by {
            Some(g) => {
                let g1 = params.len() + 1;
                let g2 = params.len() + 2;
                params.push(Value::from(normalize_path(g)));
                params.push(Value::from(normalize_path(g)));
                format!(
                    "SELECT json_extract(payload, ?{g1}) AS grp, {target} AS value FROM wb_records WHERE {where_sql} GROUP BY json_extract(payload, ?{g2}) ORDER BY 1"
                )
            }
            None => format!("SELECT {target} AS value FROM wb_records WHERE {where_sql}"),
        };
        let conn = pool.conn.clone();
        self.rt.block_on(async {
            let mut rows = conn.query(&sql, Params::Positional(params)).await?;
            let mut out = Vec::new();
            while let Some(r) = rows.next().await? {
                if group_by.is_some() {
                    out.push(json!({
                        "group": value_json(r.get_value(0)?),
                        "value": value_json(r.get_value(1)?),
                    }));
                } else {
                    out.push(json!({ "value": value_json(r.get_value(0)?) }));
                }
            }
            Ok::<_, anyhow::Error>(out)
        })
    }

    fn records_delete(&mut self, pool: &Arc<DbPool>, board_id: &str, filter: &SrvFilter) -> anyhow::Result<usize> {
        let all = self.records_all(pool, board_id)?;
        let mut seqs: Vec<i64> = all
            .iter()
            .filter(|r| filter.matches(&r.data))
            .map(|r| match r.key {
                Key::Int(v) => v,
                _ => 0,
            })
            .collect();
        if seqs.is_empty() {
            return Ok(0);
        }
        let count = seqs.len();
        let conn = pool.conn.clone();
        self.rt.block_on(async {
            for seq in seqs.drain(..) {
                conn.execute(
                    "DELETE FROM wb_records WHERE seq = ?1",
                    Params::Positional(vec![Value::Integer(seq)]),
                )
                .await?;
            }
            Ok::<_, anyhow::Error>(())
        })?;
        Ok(count)
    }

    fn record_insert(&mut self, pool: &Arc<DbPool>, row: Row) -> anyhow::Result<i64> {
        let data = &row.data;
        let board = data
            .get("board_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("record missing board_id"))?
            .to_string();
        let seq = match &row.key {
            Key::Int(v) => *v,
            _ => data.get("seq").and_then(|v| v.as_i64()).unwrap_or(0),
        };
        let payload = data.get("payload").cloned().unwrap_or(Json::Null).to_string();
        let writer = data.get("writer").and_then(|v| v.as_str()).map(str::to_string);
        let created = data
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let conn = pool.conn.clone();
        let b = board.clone();
        self.rt.block_on(async {
            conn.execute(
                "INSERT OR REPLACE INTO wb_records (seq, board_id, payload, writer, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                Params::Positional(vec![
                    Value::Integer(seq),
                    Value::from(b),
                    Value::from(payload),
                    match &writer {
                        Some(w) => Value::from(w.clone()),
                        None => Value::Null,
                    },
                    Value::from(created),
                ]),
            )
            .await?;
            Ok::<_, anyhow::Error>(())
        })?;
        Ok(seq)
    }
}

fn row_text(r: &turso::Row, idx: usize) -> anyhow::Result<Option<String>> {
    match r.get_value(idx)? {
        Value::Text(s) => Ok(Some(s)),
        Value::Null => Ok(None),
        Value::Integer(i) => Ok(Some(i.to_string())),
        Value::Real(f) => Ok(Some(f.to_string())),
        Value::Blob(b) => Ok(Some(String::from_utf8_lossy(&b).to_string())),
    }
}

fn value_json(v: Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Integer(i) => Json::from(i),
        Value::Real(f) => Json::from(f),
        Value::Text(s) => Json::String(s),
        Value::Blob(b) => Json::String(String::from_utf8_lossy(&b).to_string()),
    }
}

fn parse_key_text(s: &str) -> Key {
    if let Some(rest) = s.strip_prefix("int:") {
        if let Ok(v) = rest.parse::<i64>() {
            return Key::int(v);
        }
    }
    if let Some(rest) = s.strip_prefix("text:") {
        return Key::text(rest.to_string());
    }
    Key::text(s.to_string())
}

pub struct TursoDatabase {
    tx: mpsc::SyncSender<Job>,
}

impl TursoDatabase {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(data_dir.join("boards")).ok();
        let (tx, rx) = mpsc::sync_channel::<Job>(256);
        let wdir = data_dir.clone();
        std::thread::spawn(move || {
            let mut worker = Worker::new(wdir);
            while let Ok(job) = rx.recv() {
                job(&mut worker);
            }
        });
        Self { tx }
    }

    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Worker) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let (tx, rx) = mpsc::sync_channel::<anyhow::Result<T>>(1);
        self.tx
            .send(Box::new(move |w| {
                let _ = tx.send(f(w));
            }))
            .map_err(|_| anyhow::anyhow!("db worker stopped"))?;
        rx.recv().map_err(|_| anyhow::anyhow!("db worker error"))?
    }
}

impl Database for TursoDatabase {
    fn adapter(&self) -> &'static str {
        "turso"
    }

    fn capabilities(&self) -> DatabaseCaps {
        let mut c = DatabaseCaps::default();
        c.fts.ngram = true;
        c.fts.bm25 = true;
        c.transactions.savepoints = true;
        c.json.json_path = true;
        c
    }

    fn insert(&mut self, table: &str, row: Row) -> anyhow::Result<i64> {
        let table = table.to_string();
        let row = row;
        self.call(move |w| {
            if table == TABLE_RECORDS {
                let pool = {
                    let board = row
                        .data
                        .get("board_id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow::anyhow!("record missing board_id"))?
                        .to_string();
                    w.board(&board)?
                };
                return w.record_insert(&pool, row);
            }
            if is_system(&table) {
                let pool = w.system()?;
                let key = kv_key(&table, &row.key);
                return w.kv_put(&pool, &key, &row.data).map(|_| 0);
            }
            let board = board_from_key(&table, &row.key)
                .ok_or_else(|| anyhow::anyhow!("cannot route {table} insert"))?;
            let pool = w.board(&board)?;
            let key = kv_key(&table, &row.key);
            w.kv_put(&pool, &key, &row.data)?;
            if table == TABLE_APPS {
                let owner = row
                    .data
                    .get("owner_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let created = row
                    .data
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                w.register_board(&board, &owner, &created)?;
            }
            Ok(0)
        })
    }

    fn get(&self, table: &str, pk: &Key) -> anyhow::Result<Option<Row>> {
        let table = table.to_string();
        let pk = pk.clone();
        self.call(move |w| {
            if is_system(&table) {
                let pool = w.system()?;
                let key = kv_key(&table, &pk);
                let data = w.kv_get(&pool, &key)?;
                return Ok(data.map(|d| Row::new(pk, d)));
            }
            let board = board_from_key(&table, &pk)
                .ok_or_else(|| anyhow::anyhow!("cannot route {table} get"))?;
            let pool = w.board(&board)?;
            let key = kv_key(&table, &pk);
            let data = w.kv_get(&pool, &key)?;
            Ok(data.map(|d| Row::new(pk, d)))
        })
    }

    fn update(&mut self, table: &str, pk: &Key, patch: &Json) -> anyhow::Result<()> {
        let table = table.to_string();
        let pk = pk.clone();
        let patch = patch.clone();
        self.call(move |w| {
            if is_system(&table) {
                let pool = w.system()?;
                return w.kv_put(&pool, &kv_key(&table, &pk), &patch);
            }
            let board = board_from_key(&table, &pk)
                .ok_or_else(|| anyhow::anyhow!("cannot route {table} update"))?;
            let pool = w.board(&board)?;
            w.kv_put(&pool, &kv_key(&table, &pk), &patch)
        })
    }

    fn delete(&mut self, table: &str, filter: &SrvFilter) -> anyhow::Result<usize> {
        let table = table.to_string();
        let filter = filter.clone();
        self.call(move |w| {
            if table == TABLE_RECORDS {
                let board = board_from_filter(&filter)
                    .ok_or_else(|| anyhow::anyhow!("records delete missing board"))?;
                let pool = w.board(&board)?;
                return w.records_delete(&pool, &board, &filter);
            }
            if is_system(&table) {
                let pool = w.system()?;
                let rows = w.kv_rows(&pool, &table)?;
                let keys: Vec<String> = rows
                    .iter()
                    .filter(|r| filter.matches(&r.data))
                    .map(|r| kv_key(&table, &r.key))
                    .collect();
                w.kv_delete_keys(&pool, keys.clone())?;
                return Ok(keys.len());
            }
            let board = board_from_filter(&filter);
            let boards: Vec<String> = match board {
                Some(b) => vec![b],
                None => w.registry.keys().cloned().collect(),
            };
            let mut deleted = 0usize;
            for b in boards {
                let pool = w.board(&b)?;
                let rows = w.kv_rows(&pool, &table)?;
                let keys: Vec<String> = rows
                    .iter()
                    .filter(|r| filter.matches(&r.data))
                    .map(|r| kv_key(&table, &r.key))
                    .collect();
                deleted += keys.len();
                w.kv_delete_keys(&pool, keys)?;
                if table == TABLE_APPS {
                    drop(pool);
                    w.unregister_board(&b)?;
                }
            }
            Ok(deleted)
        })
    }

    fn query(&self, table: &str, q: &Query) -> anyhow::Result<Cursor> {
        let table = table.to_string();
        let q = Query {
            filter: q.filter.clone(),
            orders: q.orders.clone(),
            limit: q.limit,
            offset: q.offset,
            ttl: q.ttl.clone(),
        };
        self.call(move |w| {
            if table == TABLE_RECORDS {
                let board = board_from_filter(&q.filter)
                    .ok_or_else(|| anyhow::anyhow!("records query missing board"))?;
                let pool = w.board(&board)?;
                return w.records_query_sql(&pool, &board, &q);
            }
            if is_system(&table) {
                let pool = w.system()?;
                let mut rows = w.kv_rows(&pool, &table)?;
                rows.retain(|r| q.filter.matches(&r.data));
                return Ok(sort_and_slice(rows, &q.orders, q.limit, q.offset));
            }
            if table == TABLE_APPS {
                w.system()?;
                let board = board_from_filter(&q.filter);
                let owner = filter_owner(&q.filter);
                let boards: Vec<String> = match (board, owner) {
                    (Some(b), _) => vec![b],
                    (None, Some(o)) => w
                        .registry
                        .iter()
                        .filter(|(_, (owner_key, _))| owner_key == &o)
                        .map(|(b, _)| b.clone())
                        .collect(),
                    (None, None) => w.registry.keys().cloned().collect(),
                };
                let mut rows = Vec::new();
                for b in boards {
                    let pool = w.board(&b)?;
                    let key = kv_key(&table, &Key::text(&b));
                    if let Some(d) = w.kv_get(&pool, &key)? {
                        if q.filter.matches(&d) {
                            rows.push(Row::new(Key::text(&b), d));
                        }
                    }
                }
                return Ok(sort_and_slice(rows, &q.orders, q.limit, q.offset));
            }
            let board = board_from_filter(&q.filter);
            let boards: Vec<String> = match board {
                Some(b) => vec![b],
                None => w.registry.keys().cloned().collect(),
            };
            let mut rows = Vec::new();
            for b in boards {
                let pool = w.board(&b)?;
                let mut br = w.kv_rows(&pool, &table)?;
                br.retain(|r| q.filter.matches(&r.data));
                rows.extend(br);
            }
            Ok(sort_and_slice(rows, &q.orders, q.limit, q.offset))
        })
    }

    fn aggregate(
        &self,
        table: &str,
        board: &str,
        filter: &SrvFilter,
        agg: engine::storage::ir::Agg,
        field: Option<&str>,
        group_by: Option<&str>,
        ttl: Option<TtlClause>,
    ) -> anyhow::Result<Vec<Json>> {
        if table != TABLE_RECORDS {
            let q = Query {
                filter: SrvFilter {
                    conds: vec![FilterCond {
                        field: "$.board_id".to_string(),
                        op: Op::Eq,
                        value: Json::String(board.to_string()),
                    }],
                },
                orders: vec![],
                limit: usize::MAX,
                offset: 0,
                ttl: ttl.clone(),
            };
            let rows = self.query(table, &q)?.rows;
            return engine::storage::database::aggregate_rows(
                &rows,
                filter,
                agg,
                field,
                group_by,
                ttl,
            );
        }
        let board = board.to_string();
        let filter = filter.clone();
        let field = field.map(str::to_string);
        let group_by = group_by.map(str::to_string);
        self.call(move |w| {
            let pool = w.board(&board)?;
            w.records_aggregate_sql(
                &pool,
                &board,
                &filter,
                agg,
                field.as_deref(),
                group_by.as_deref(),
                ttl.as_ref(),
            )
        })
    }

    fn upsert(&mut self, table: &str, key: &str, row: Row) -> anyhow::Result<i64> {
        let table = table.to_string();
        let key = key.to_string();
        self.call(move |w| {
            let board = board_from_key(&table, &row.key)
                .ok_or_else(|| anyhow::anyhow!("cannot route {table} upsert"))?;
            let pool = w.board(&board)?;
            let rows = w.kv_rows(&pool, &table)?;
            let unique = engine::expr::get_path(&row.data, &key);
            let existing = rows
                .iter()
                .find(|r| engine::expr::get_path(&r.data, &key) == unique)
                .map(|r| r.key.clone());
            match existing {
                Some(ek) => {
                    let k = kv_key(&table, &ek);
                    w.kv_put(&pool, &k, &row.data)?;
                }
                None => {
                    let auto = Key::text(format!("upsert-{}", uuid::Uuid::new_v4()));
                    let k = kv_key(&table, &auto);
                    w.kv_put(&pool, &k, &row.data)?;
                }
            }
            Ok(0)
        })
    }
}
