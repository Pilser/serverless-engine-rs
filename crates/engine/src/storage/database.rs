use crate::model::Key;
use crate::storage::ir::SrvFilter;
use serde_json::{json, Value as Json};
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
    Blob(Vec<u8>),
    Json(Json),
}

#[derive(Debug, Clone)]
pub struct Row {
    pub key: Key,
    pub data: Json,
}

impl Row {
    pub fn new(key: Key, data: Json) -> Self {
        Self { key, data }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Query {
    pub filter: SrvFilter,
    pub orders: Vec<(String, bool)>,
    pub limit: usize,
    pub offset: usize,
    pub ttl: Option<TtlClause>,
}

#[derive(Debug, Clone, Default)]
pub struct TtlClause {
    pub seconds: Option<i64>,
    pub field: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Cursor {
    pub rows: Vec<Row>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FtsCaps {
    pub bm25: bool,
    pub ngram: bool,
    pub phrase: bool,
    pub analyzers: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TxCaps {
    pub savepoints: bool,
    pub cross_shard: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ReplicationCaps {
    pub embedded_replica: bool,
    pub multi_writer: bool,
    pub cdc: bool,
}

#[derive(Debug, Clone, Default)]
pub struct JsonCaps {
    pub jsonb: bool,
    pub json_path: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DatabaseCaps {
    pub fts: FtsCaps,
    pub vector: bool,
    pub transactions: TxCaps,
    pub replication: ReplicationCaps,
    pub json: JsonCaps,
}

impl DatabaseCaps {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn full() -> Self {
        Self {
            fts: FtsCaps { bm25: true, ngram: true, phrase: true, analyzers: true },
            vector: true,
            transactions: TxCaps { savepoints: true, cross_shard: true },
            replication: ReplicationCaps { embedded_replica: true, multi_writer: true, cdc: true },
            json: JsonCaps { jsonb: true, json_path: true },
        }
    }
}

pub trait Database: Send + Sync + 'static {
    fn adapter(&self) -> &'static str;

    fn capabilities(&self) -> DatabaseCaps;

    fn insert(&mut self, table: &str, row: Row) -> anyhow::Result<i64>;

    fn get(&self, table: &str, pk: &Key) -> anyhow::Result<Option<Row>>;

    fn update(&mut self, table: &str, pk: &Key, patch: &Json) -> anyhow::Result<()>;

    fn delete(&mut self, table: &str, filter: &SrvFilter) -> anyhow::Result<usize>;

    fn query(&self, table: &str, q: &Query) -> anyhow::Result<Cursor>;

    fn upsert(&mut self, table: &str, key: &str, row: Row) -> anyhow::Result<i64>;

    /// Run an aggregate over `table`, optionally grouped. Default implementation
    /// loads rows and aggregates in Rust; a SQL-capable adapter overrides it.
    /// `filter` carries only the user's conditions (matched against payload).
    fn aggregate(
        &self,
        table: &str,
        board: &str,
        table_name: &str,
        filter: &SrvFilter,
        agg: crate::storage::ir::Agg,
        field: Option<&str>,
        group_by: Option<&str>,
        ttl: Option<TtlClause>,
    ) -> anyhow::Result<Vec<Json>> {
        let q = Query {
            filter: crate::storage::ir::SrvFilter {
                conds: vec![
                    crate::storage::ir::FilterCond {
                        field: "$.board_id".to_string(),
                        op: crate::storage::ir::Op::Eq,
                        value: Json::String(board.to_string()),
                    },
                    crate::storage::ir::FilterCond {
                        field: "$.table".to_string(),
                        op: crate::storage::ir::Op::Eq,
                        value: Json::String(table_name.to_string()),
                    },
                ],
            },
            orders: vec![],
            limit: usize::MAX,
            offset: 0,
            ttl: ttl.clone(),
        };
        let rows = self.query(table, &q)?.rows;
        aggregate_rows(&rows, filter, agg, field, group_by, ttl)
    }
}

/// Aggregate `rows` (record JSON documents) in Rust. Used by the default
/// `Database::aggregate` and by adapters that do not push aggregates to SQL.
pub fn aggregate_rows(
    rows: &[Row],
    filter: &SrvFilter,
    agg: crate::storage::ir::Agg,
    field: Option<&str>,
    group_by: Option<&str>,
    ttl: Option<TtlClause>,
) -> anyhow::Result<Vec<Json>> {
    use crate::storage::ir::{normalize_path, scalar_text, Agg};
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Acc {
        rows: u64,
        n: u64,
        sum: f64,
        min: f64,
        max: f64,
        has: bool,
    }
    impl Acc {
        fn add(&mut self, v: Option<f64>) {
            self.rows += 1;
            if let Some(v) = v {
                self.n += 1;
                self.sum += v;
                if !self.has {
                    self.min = v;
                    self.max = v;
                    self.has = true;
                } else {
                    self.min = self.min.min(v);
                    self.max = self.max.max(v);
                }
            }
        }
    }
    fn agg_value(agg: Agg, acc: &Acc) -> Json {
        match agg {
            Agg::Count => Json::from(acc.rows),
            Agg::Sum => {
                if acc.has { Json::from(acc.sum) } else { Json::Null }
            }
            Agg::Avg => {
                if acc.has { Json::from(acc.sum / acc.n as f64) } else { Json::Null }
            }
            Agg::Min => {
                if acc.has { Json::from(acc.min) } else { Json::Null }
            }
            Agg::Max => {
                if acc.has { Json::from(acc.max) } else { Json::Null }
            }
        }
    }
    fn field_num(payload: &Json, path: &str) -> Option<f64> {
        crate::expr::get_path(payload, path).as_f64()
    }
    fn aggregate_ttl_alive(ttl: Option<&TtlClause>, rec: &Json) -> bool {
        let Some(ttl) = ttl else { return true };
        let created = rec.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        let payload = rec.get("payload").cloned().unwrap_or(Json::Null);
        let age_dead = match ttl.seconds {
            Some(n) if n > 0 && !created.is_empty() => {
                created < crate::crud::subtract_seconds(&crate::crud::now_str(), n).as_str()
            }
            _ => false,
        };
        let field_dead = match &ttl.field {
            Some(f) => {
                let value = crate::expr::get_path(&payload, &crate::storage::ir::normalize_path(f));
                !value.is_null() && crate::storage::ir::scalar_text(&value) < crate::crud::now_str()
            }
            None => false,
        };
        !(age_dead || field_dead)
    }
    let field_path = field.map(normalize_path);
    let group_path = group_by.map(normalize_path);

    if group_path.is_none() {
        let mut acc = Acc::default();
        for row in rows {
            let rec: Json = row.data.clone();
            let payload = rec.get("payload").cloned().unwrap_or(Json::Null);
            if !aggregate_ttl_alive(ttl.as_ref(), &rec) || !filter.matches(&payload) {
                continue;
            }
            let v = field_path.as_deref().and_then(|p| field_num(&payload, p));
            acc.add(v);
        }
        return Ok(vec![json!({ "value": agg_value(agg, &acc) })]);
    }

    let mut groups: BTreeMap<String, (Json, Acc)> = BTreeMap::new();
    for row in rows {
        let rec: Json = row.data.clone();
        let payload = rec.get("payload").cloned().unwrap_or(Json::Null);
        if !aggregate_ttl_alive(ttl.as_ref(), &rec) || !filter.matches(&payload) {
            continue;
        }
        let gv = group_path
            .as_deref()
            .map(|p| crate::expr::get_path(&payload, p))
            .unwrap_or(Json::Null);
        let key = scalar_text(&gv);
        let entry = groups.entry(key).or_insert((gv, Acc::default()));
        let v = field_path.as_deref().and_then(|p| field_num(&payload, p));
        entry.1.add(v);
    }
    let mut out = Vec::new();
    for (_, (g, acc)) in groups {
        out.push(json!({ "group": g, "value": agg_value(agg, &acc) }));
    }
    Ok(out)
}
