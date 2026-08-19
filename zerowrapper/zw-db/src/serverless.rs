use crate::DbPool;
use serde::Serialize;
use std::path::{Path, PathBuf};
use turso::params::Params;
use turso::Value;

pub use crate::filters::{Agg, FilterCond, Op, SrvFilter, parse_filter, srv_filter_where};

#[derive(Debug, Clone, Serialize)]
pub struct SrvApp {
    pub board_id: String,
    pub owner_key: String,
    pub title: String,
    pub schema_json: Option<String>,
    pub public_reads: bool,
    pub unique_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computed_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redact_json: Option<String>,
    pub record_count: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SrvRecord {
    pub seq: i64,
    pub payload: serde_json::Value,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

async fn read_app(rows: &mut turso::Rows) -> anyhow::Result<Option<SrvApp>> {
    if let Some(row) = rows.next().await? {
        let owner_key = match row.get_value(0)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid owner_key")),
        };
        let board_id = match row.get_value(1)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid board_id")),
        };
        let title = match row.get_value(2)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid title")),
        };
        let schema_json = match row.get_value(3)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let public_reads = match row.get_value(4)? {
            Value::Integer(i) => i != 0,
            _ => false,
        };
        let unique_key = match row.get_value(5)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let computed_json = match row.get_value(6)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let validate_json = match row.get_value(7)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let redact_json = match row.get_value(8)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let record_count = match row.get_value(9)? {
            Value::Integer(i) => i,
            _ => 0,
        };
        let created_at = match row.get_value(10)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid created_at")),
        };
        Ok(Some(SrvApp { board_id, owner_key, title, schema_json, public_reads, unique_key, computed_json, validate_json, redact_json, record_count, created_at }))
    } else {
        Ok(None)
    }
}

pub(crate) async fn read_record(rows: &mut turso::Rows) -> anyhow::Result<Option<SrvRecord>> {
    if let Some(row) = rows.next().await? {
        let seq = match row.get_value(0)? {
            Value::Integer(i) => i,
            _ => return Err(anyhow::anyhow!("invalid seq")),
        };
        let payload_text = match row.get_value(1)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid payload")),
        };
        let payload = serde_json::from_str::<serde_json::Value>(&payload_text)?;
        let created_at = match row.get_value(2)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid created_at")),
        };
        let writer = match row.get_value(3)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        Ok(Some(SrvRecord { seq, payload, created_at, writer, score: None, snippet: None }))
    } else {
        Ok(None)
    }
}

pub(crate) async fn read_record_scored(rows: &mut turso::Rows) -> anyhow::Result<Option<SrvRecord>> {
    if let Some(row) = rows.next().await? {
        let seq = match row.get_value(0)? {
            Value::Integer(i) => i,
            _ => return Err(anyhow::anyhow!("invalid seq")),
        };
        let payload_text = match row.get_value(1)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid payload")),
        };
        let payload = serde_json::from_str::<serde_json::Value>(&payload_text)?;
        let created_at = match row.get_value(2)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid created_at")),
        };
        let writer = match row.get_value(3)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let score = match row.get_value(4)? {
            Value::Real(f) => Some(f),
            Value::Integer(i) => Some(i as f64),
            _ => None,
        };
        let snippet = match row.get_value(5)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        Ok(Some(SrvRecord { seq, payload, created_at, writer, score, snippet }))
    } else {
        Ok(None)
    }
}

const RECORD_COLS: &str = "seq, payload, created_at, writer";

impl DbPool {
    pub async fn srv_app_create(&self, owner: &str, title: &str, schema_json: Option<&str>, public_reads: bool, unique_key: Option<&str>) -> anyhow::Result<String> {
        let conn = self.conn().await;
        let board_id = gen_board_id();
        let b = sq(&board_id);
        let o = sq(owner);
        let t = sq(title);
        let s = match schema_json {
            Some(s) => format!("'{}'", sq(s)),
            None => "NULL".to_string(),
        };
        let u = match unique_key {
            Some(u) => format!("'{}'", sq(u)),
            None => "NULL".to_string(),
        };
        let p = if public_reads { 1 } else { 0 };
        let sql = format!(
            "INSERT INTO wb_apps (board_id, owner_key, title, schema_json, public_reads, unique_key) \
             VALUES ('{b}', '{o}', '{t}', {s}, {p}, {u})"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(board_id)
    }

    pub async fn srv_app_create_with_id(&self, board_id: &str, owner: &str, title: &str, schema_json: Option<&str>, public_reads: bool, unique_key: Option<&str>) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let b = sq(board_id);
        let o = sq(owner);
        let t = sq(title);
        let s = match schema_json {
            Some(s) => format!("'{}'", sq(s)),
            None => "NULL".to_string(),
        };
        let u = match unique_key {
            Some(u) => format!("'{}'", sq(u)),
            None => "NULL".to_string(),
        };
        let p = if public_reads { 1 } else { 0 };
        let sql = format!(
            "INSERT INTO wb_apps (board_id, owner_key, title, schema_json, public_reads, unique_key) \
             VALUES ('{b}', '{o}', '{t}', {s}, {p}, {u})"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    async fn read_app_raw(&self, board_id: &str) -> anyhow::Result<Option<SrvApp>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT owner_key, board_id, title, schema_json, public_reads, unique_key, computed_json, validate_json, redact_json, \
                    (SELECT COUNT(*) FROM wb_records WHERE board_id = wb_apps.board_id), \
                    created_at \
                 FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        read_app(&mut rows).await
    }

    pub async fn srv_app_by_id(&self, board_id: &str) -> anyhow::Result<Option<SrvApp>> {
        self.read_app_raw(board_id).await
    }

    pub async fn srv_app_owned(&self, board_id: &str, owner: &str) -> anyhow::Result<Option<SrvApp>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT owner_key, board_id, title, schema_json, public_reads, unique_key, computed_json, validate_json, redact_json, \
                    (SELECT COUNT(*) FROM wb_records WHERE board_id = wb_apps.board_id), \
                    created_at \
                 FROM wb_apps WHERE board_id = ?1 AND owner_key = ?2",
                Params::Positional(vec![Value::from(board_id), Value::from(owner)]),
            )
            .await?;
        read_app(&mut rows).await
    }

    pub async fn srv_app_list(&self, owner: &str) -> anyhow::Result<Vec<SrvApp>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT owner_key, board_id, title, schema_json, public_reads, unique_key, computed_json, validate_json, redact_json, \
                    (SELECT COUNT(*) FROM wb_records WHERE board_id = wb_apps.board_id), \
                    created_at \
                 FROM wb_apps WHERE owner_key = ?1 ORDER BY created_at",
                Params::Positional(vec![Value::from(owner)]),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(app) = read_app(&mut rows).await? {
            out.push(app);
        }
        Ok(out)
    }

    pub async fn srv_app_update(&self, board_id: &str, owner: &str, title: Option<&str>, schema: Option<Option<&str>>, public_reads: Option<bool>, unique_key: Option<Option<&str>>) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT owner_key FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Text(s) => {
                    if s != owner {
                        anyhow::bail!("board {board_id} is not owned by {owner}");
                    }
                }
                _ => return Err(anyhow::anyhow!("invalid owner_key")),
            },
            None => anyhow::bail!("board {board_id} not found"),
        }
        drop(rows);
        let mut sets: Vec<String> = Vec::new();
        if let Some(t) = title {
            sets.push(format!("title = '{}'", sq(t)));
        }
        match schema {
            Some(Some(s)) => sets.push(format!("schema_json = '{}'", sq(s))),
            Some(None) => sets.push("schema_json = NULL".to_string()),
            None => {}
        }
        if let Some(b) = public_reads {
            sets.push(format!("public_reads = {}", if b { 1 } else { 0 }));
        }
        match unique_key {
            Some(Some(u)) => sets.push(format!("unique_key = '{}'", sq(u))),
            Some(None) => sets.push("unique_key = NULL".to_string()),
            None => {}
        }
        if sets.is_empty() {
            return Ok(());
        }
        let b = sq(board_id);
        let sql = format!("UPDATE wb_apps SET {} WHERE board_id = '{b}'", sets.join(", "));
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_app_delete(&self, board_id: &str, owner: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT owner_key FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        let is_owner = match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Text(s) => s == owner,
                _ => false,
            },
            None => false,
        };
        if !is_owner {
            anyhow::bail!("board {board_id} is not owned by {owner}");
        }
        drop(rows);
        let b = sq(board_id);
        let o = sq(owner);
        let sql = format!(
            "BEGIN; \
             DELETE FROM wb_records WHERE board_id = '{b}'; \
             DELETE FROM wb_apps WHERE board_id = '{b}' AND owner_key = '{o}'; \
             COMMIT;"
        );
        conn.execute_batch(&sql).await?;
        Ok(())
    }

    pub async fn srv_validate(&self, board_id: &str, payload: &serde_json::Value) -> anyhow::Result<()> {
        let app = self
            .srv_app_by_id(board_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("board {board_id} not found"))?;
        let Some(schema_text) = app.schema_json.as_deref() else {
            return Ok(());
        };
        let schema: serde_json::Value = serde_json::from_str(schema_text)
            .map_err(|e| anyhow::anyhow!("board {board_id} has an invalid schema_json: {e}"))?;
        let compiled = jsonschema::options_for::<jsonschema::json::SerdeJson>()
            .should_validate_formats(true)
            .build(&schema)
            .map_err(|e| anyhow::anyhow!("board {board_id} has an invalid schema_json: {e}"))?;
        match compiled.validate(payload) {
            Ok(()) => Ok(()),
            Err(e) => anyhow::bail!("payload failed schema: {e}"),
        }
    }

    async fn unique_from_payload(&self, _board_id: &str, unique_key: &str, payload: &serde_json::Value) -> anyhow::Result<Option<String>> {
        let value = value_at_path(payload, unique_key);
        let Some(v) = value else {
            return Ok(None);
        };
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some(scalar_string(v)))
    }

    async fn unique_holder(&self, board_id: &str, unique_key: &str, value: &str, exclude_seq: Option<i64>) -> anyhow::Result<Option<i64>> {
        let conn = self.conn().await;
        let (sql, params) = match exclude_seq {
            Some(e) => (
                "SELECT seq FROM wb_records WHERE board_id = ?1 AND CAST(json_extract(payload, ?2) AS TEXT) = ?3 AND seq <> ?4 LIMIT 1",
                Params::Positional(vec![Value::from(board_id), Value::from(unique_key), Value::from(value), Value::Integer(e)]),
            ),
            None => (
                "SELECT seq FROM wb_records WHERE board_id = ?1 AND CAST(json_extract(payload, ?2) AS TEXT) = ?3 LIMIT 1",
                Params::Positional(vec![Value::from(board_id), Value::from(unique_key), Value::from(value)]),
            ),
        };
        let mut rows = conn.query(sql, params).await?;
        match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Integer(i) => Ok(Some(i)),
                _ => Ok(None),
            },
            None => Ok(None),
        }
    }

    /// Apply computed fields + custom validate rules, then JSON-schema validate.
    /// Returns the prepared payload (computed fields materialized) ready to store.
    pub async fn prepare_payload(&self, app: &SrvApp, payload: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let mut prepared = payload.clone();
        if let Some(c) = app.computed_json.as_deref() {
            let computed: serde_json::Value = serde_json::from_str(c)
                .map_err(|e| anyhow::anyhow!("board {} has invalid computed_json: {e}", app.board_id))?;
            crate::logic::apply_computed(&computed, &mut prepared)?;
        }
        if let Some(v) = app.validate_json.as_deref() {
            let rules: serde_json::Value = serde_json::from_str(v)
                .map_err(|e| anyhow::anyhow!("board {} has invalid validate_json: {e}", app.board_id))?;
            if let Some(msg) = crate::logic::validate_rules(&rules, &prepared)? {
                anyhow::bail!("{msg}");
            }
        }
        self.srv_validate(&app.board_id, &prepared).await?;
        Ok(prepared)
    }

    pub async fn srv_record_insert(&self, board_id: &str, payload: &serde_json::Value, writer: Option<&str>, upsert: bool) -> anyhow::Result<i64> {
        let app = self
            .srv_app_by_id(board_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("board {board_id} does not exist"))?;
        let prepared = self.prepare_payload(&app, payload).await?;
        let payload = &prepared;
        let unique_val = match app.unique_key.as_deref() {
            Some(key) => self.unique_from_payload(board_id, key, payload).await?,
            None => None,
        };
        if let Some(value) = &unique_val {
            let key = app.unique_key.as_deref().unwrap_or_default();
            if let Some(existing) = self.unique_holder(board_id, key, value, None).await? {
                if !upsert {
                    anyhow::bail!("duplicate value '{value}' for unique field {key} (already record seq {existing})");
                }
                self.srv_record_set(board_id, existing, payload, writer).await?;
                return Ok(existing);
            }
        }

        let conn = self.conn().await;
        let mut seq_rows = conn
            .query(
                "SELECT COALESCE((SELECT MAX(seq) FROM wb_records WHERE board_id = ?1), 0) + 1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        let seq = match seq_rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Integer(i) => i,
                _ => return Err(anyhow::anyhow!("invalid next seq")),
            },
            None => 1,
        };
        let text = serde_json::to_string(payload)?;
        let w = match writer {
            Some(w) => format!("'{}'", sq(w)),
            None => "NULL".to_string(),
        };
        let b = sq(board_id);
        let o = sq(&app.owner_key);
        let t = sq(&text);
        let sql = format!(
            "BEGIN; \
             INSERT INTO wb_records (board_id, owner_key, seq, payload, writer) VALUES ('{b}', '{o}', {seq}, '{t}', {w}); \
             COMMIT;"
        );
        conn.execute_batch(&sql).await?;
        drop(conn);
        self.srv_audit_append(board_id, "created", None, writer, Some(&crate::audit::audit_hash(payload))).await?;
        Ok(seq)
    }

    /// Insert many records at once (one seq per record, in order). Each record is
    /// validated against the board schema; a failure aborts the batch.
    pub async fn srv_record_bulk_insert(
        &self,
        board_id: &str,
        records: &[serde_json::Value],
        writer: Option<&str>,
        upsert: bool,
    ) -> anyhow::Result<Vec<i64>> {
        if records.is_empty() {
            return Ok(Vec::new());
        }
        let mut seqs = Vec::with_capacity(records.len());
        for payload in records {
            let seq = self.srv_record_insert(board_id, payload, writer, upsert).await?;
            seqs.push(seq);
        }
        Ok(seqs)
    }

    /// Replace an existing record's payload (identity and created_at preserved).
    pub async fn srv_record_set(&self, board_id: &str, seq: i64, payload: &serde_json::Value, writer: Option<&str>) -> anyhow::Result<()> {
        let app = self
            .srv_app_by_id(board_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("board {board_id} does not exist"))?;
        let prepared = self.prepare_payload(&app, payload).await?;
        let payload = &prepared;
        if let Some(key) = app.unique_key.as_deref() {
            if let Some(value) = self.unique_from_payload(board_id, key, payload).await? {
                if let Some(existing) = self.unique_holder(board_id, key, &value, Some(seq)).await? {
                    anyhow::bail!("duplicate value '{value}' for unique field {key} (already record seq {existing})");
                }
            }
        }
        let conn = self.conn().await;
        let w = match writer {
            Some(w) => format!("'{}'", sq(w)),
            None => "NULL".to_string(),
        };
        let t = sq(&serde_json::to_string(payload)?);
        let b = sq(board_id);
        conn.execute(
            &format!("UPDATE wb_records SET payload = '{t}', writer = {w} WHERE board_id = '{b}' AND seq = {seq}"),
            Params::Positional(vec![]),
        )
        .await?;
        drop(conn);
        self.srv_audit_append(board_id, "updated", None, writer, Some(&crate::audit::audit_hash(payload))).await?;
        Ok(())
    }

    pub async fn srv_record_get_raw(&self, board_id: &str, seq: i64) -> anyhow::Result<Option<SrvRecord>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT seq, payload, created_at, writer FROM wb_records WHERE board_id = ?1 AND seq = ?2",
                Params::Positional(vec![Value::from(board_id), Value::Integer(seq)]),
            )
            .await?;
        read_record(&mut rows).await
    }

    /// Mask `redact` paths in a single record's payload (no-op when the board has none).
    pub(crate) async fn srv_record_redact_one(&self, board_id: &str, record: &mut SrvRecord) -> anyhow::Result<()> {
        if let Some(redact) = self.srv_redact_get(board_id).await? {
            crate::logic::apply_redact(&redact, &mut record.payload);
        }
        Ok(())
    }

    /// Mask `redact` paths in every record's payload (no-op when the board has none).
    pub(crate) async fn srv_record_redact(&self, board_id: &str, records: &mut [SrvRecord]) -> anyhow::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        if let Some(redact) = self.srv_redact_get(board_id).await? {
            for r in records.iter_mut() {
                crate::logic::apply_redact(&redact, &mut r.payload);
            }
        }
        Ok(())
    }

    /// Apply a patch (subset of MongoDB-style ops) to an existing record's payload.
    /// `ops` is either a plain object (shallow top-level merge, like a $set of leaf keys)
    /// or an object whose keys start with '$' and group single-path ops:
    ///   {"$set": {"a.b.c": v}, "$inc": {"votes": 1}, "$dec": {...}, "$mul": {...}, "$unset": ["a.b", ...]}
    /// Returns the resulting (validated) payload.
    pub async fn srv_record_patch(&self, board_id: &str, seq: i64, ops: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let app = self
            .srv_app_by_id(board_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("board {board_id} does not exist"))?;
        let current = self
            .srv_record_get_raw(board_id, seq)
            .await?
            .ok_or_else(|| anyhow::anyhow!("record seq {seq} does not exist"))?;
        let mut merged = current.payload.clone();

        match ops {
            serde_json::Value::Object(map) => {
                let has_dollar = map.keys().any(|k| k.starts_with('$'));
                if has_dollar {
                    apply_patch_ops(&mut merged, ops)?;
                } else {
                    for (k, v) in map {
                        merged[k.as_str()] = v.clone();
                    }
                }
            }
            other => anyhow::bail!("patch must be a JSON object, got {}", serde_json::to_string(other)?),
        }

        self.srv_validate(board_id, &merged).await?;
        if let Some(key) = app.unique_key.as_deref() {
            if let Some(value) = self.unique_from_payload(board_id, key, &merged).await? {
                if let Some(existing) = self.unique_holder(board_id, key, &value, Some(seq)).await? {
                    anyhow::bail!("duplicate value '{value}' for unique field {key} (already record seq {existing})");
                }
            }
        }
        let conn = self.conn().await;
        let t = sq(&serde_json::to_string(&merged)?);
        let b = sq(board_id);
        conn.execute(
            &format!("UPDATE wb_records SET payload = '{t}' WHERE board_id = '{b}' AND seq = {seq}"),
            Params::Positional(vec![]),
        )
        .await?;
        drop(conn);
        self.srv_audit_append(board_id, "updated", None, current.writer.as_deref(), Some(&crate::audit::audit_hash(&merged))).await?;
        Ok(merged)
    }

    pub async fn srv_record_get(&self, board_id: &str, seq: i64) -> anyhow::Result<Option<SrvRecord>> {
        let mut record = match self.srv_record_get_raw(board_id, seq).await? {
            Some(r) => r,
            None => return Ok(None),
        };
        self.srv_record_redact_one(board_id, &mut record).await?;
        Ok(Some(record))
    }

    /// Unmasked variant of `srv_record_list`, for internal consumers that must
    /// copy or process records without `redact` masking.
    pub async fn srv_record_list_raw(&self, board_id: &str, limit: i64, before: Option<i64>, offset: i64) -> anyhow::Result<Vec<SrvRecord>> {
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        let mut params = vec![Value::from(board_id)];
        let where_sql = match self.srv_ttl_clause(board_id, &mut params).await? {
            Some(ttl_frag) => format!("NOT ({ttl_frag}) AND board_id = ?1"),
            None => "board_id = ?1".to_string(),
        };
        let sql = match before {
            Some(b) => {
                let bp = params.len() + 1;
                params.push(Value::Integer(b));
                let lp = params.len() + 1;
                params.push(Value::Integer(limit));
                if offset > 0 {
                    let op = params.len() + 1;
                    params.push(Value::Integer(offset));
                    format!("SELECT {RECORD_COLS} FROM wb_records WHERE {where_sql} AND seq < ?{bp} ORDER BY seq DESC LIMIT ?{lp} OFFSET ?{op}")
                } else {
                    format!("SELECT {RECORD_COLS} FROM wb_records WHERE {where_sql} AND seq < ?{bp} ORDER BY seq DESC LIMIT ?{lp}")
                }
            }
            None => {
                let lp = params.len() + 1;
                params.push(Value::Integer(limit));
                if offset > 0 {
                    let op = params.len() + 1;
                    params.push(Value::Integer(offset));
                    format!("SELECT {RECORD_COLS} FROM wb_records WHERE {where_sql} ORDER BY seq DESC LIMIT ?{lp} OFFSET ?{op}")
                } else {
                    format!("SELECT {RECORD_COLS} FROM wb_records WHERE {where_sql} ORDER BY seq DESC LIMIT ?{lp}")
                }
            }
        };
        let conn = self.conn().await;
        let mut rows = conn.query(&sql, Params::Positional(params)).await?;
        let mut out = Vec::new();
        while let Some(record) = read_record(&mut rows).await? {
            out.push(record);
        }
        Ok(out)
    }

    pub async fn srv_record_list(&self, board_id: &str, limit: i64, before: Option<i64>, offset: i64) -> anyhow::Result<Vec<SrvRecord>> {
        let mut out = self.srv_record_list_raw(board_id, limit, before, offset).await?;
        self.srv_record_redact(board_id, &mut out).await?;
        Ok(out)
    }

    pub async fn srv_record_count(&self, board_id: &str) -> anyhow::Result<i64> {
        let mut params = vec![Value::from(board_id)];
        let where_sql = match self.srv_ttl_clause(board_id, &mut params).await? {
            Some(ttl_frag) => format!("NOT ({ttl_frag}) AND board_id = ?1"),
            None => "board_id = ?1".to_string(),
        };
        let sql = format!("SELECT COUNT(*) FROM wb_records WHERE {where_sql}");
        let conn = self.conn().await;
        let mut rows = conn
            .query(&sql, Params::Positional(params))
            .await?;
        match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Integer(i) => Ok(i),
                _ => Ok(0),
            },
            None => Ok(0),
        }
    }

    pub async fn srv_record_query(&self, board_id: &str, json_path: &str, eq: Option<&str>) -> anyhow::Result<Vec<SrvRecord>> {
        let mut out = self.srv_record_query_raw(board_id, json_path, eq).await?;
        self.srv_record_redact(board_id, &mut out).await?;
        Ok(out)
    }

    /// Unmasked variant of `srv_record_query`, for internal consumers that look up
    /// records (e.g. file metadata) without `redact` masking.
    pub async fn srv_record_query_raw(&self, board_id: &str, json_path: &str, eq: Option<&str>) -> anyhow::Result<Vec<SrvRecord>> {
        let mut params = vec![Value::from(board_id)];
        let where_sql = match self.srv_ttl_clause(board_id, &mut params).await? {
            Some(ttl_frag) => format!("NOT ({ttl_frag}) AND board_id = ?1"),
            None => "board_id = ?1".to_string(),
        };
        let where_sql = if let Some(v) = eq {
            let pp = params.len() + 1;
            params.push(Value::from(json_path));
            let vp = params.len() + 1;
            params.push(Value::from(v));
            format!("{where_sql} AND CAST(json_extract(payload, ?{pp}) AS TEXT) = ?{vp}")
        } else {
            where_sql
        };
        let sql = format!("SELECT {RECORD_COLS} FROM wb_records WHERE {where_sql} ORDER BY seq DESC");
        let conn = self.conn().await;
        let mut rows = conn.query(&sql, Params::Positional(params)).await?;
        let mut out = Vec::new();
        while let Some(record) = read_record(&mut rows).await? {
            out.push(record);
        }
        Ok(out)
    }

    pub async fn srv_record_delete(&self, board_id: &str, seq: Option<i64>) -> anyhow::Result<()> {
        let conn = self.conn().await;
        match seq {
            Some(s) => {
                conn.execute(
                    "DELETE FROM wb_records WHERE board_id = ?1 AND seq = ?2",
                    Params::Positional(vec![Value::from(board_id), Value::Integer(s)]),
                )
                .await?;
            }
            None => {
                conn.execute(
                    "DELETE FROM wb_records WHERE board_id = ?1",
                    Params::Positional(vec![Value::from(board_id)]),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Delete every record matching the given filter conditions, returning the
    /// number of rows removed.
    pub async fn srv_record_delete_filter(&self, board_id: &str, conds: &[FilterCond]) -> anyhow::Result<u64> {
        let (where_sql, mut params, _) = srv_filter_where(board_id, conds);
        let where_sql = match self.srv_ttl_clause(board_id, &mut params).await? {
            Some(ttl_frag) => format!("NOT ({ttl_frag}) AND {where_sql}"),
            None => where_sql,
        };
        let sql = format!("DELETE FROM wb_records WHERE {where_sql}");
        let conn = self.conn().await;
        let affected = conn.execute(&sql, Params::Positional(params)).await?;
        Ok(affected as u64)
    }

    /// Deletes a single record, returning whether a row was actually removed.
    pub async fn srv_record_delete_one(&self, board_id: &str, seq: i64) -> anyhow::Result<bool> {        let conn = self.conn().await;
        let affected = conn
            .execute(
                "DELETE FROM wb_records WHERE board_id = ?1 AND seq = ?2",
                Params::Positional(vec![Value::from(board_id), Value::Integer(seq)]),
            )
            .await?;
        if affected > 0 {
            drop(conn);
            self.srv_audit_append(board_id, "deleted", None, None, None).await?;
        }
        Ok(affected > 0)
    }

    /// Returns every record with `seq > after_seq` in ascending order (capped).
    pub async fn srv_records_after(&self, board_id: &str, after_seq: i64) -> anyhow::Result<Vec<SrvRecord>> {
        let mut params = vec![Value::from(board_id)];
        let where_sql = match self.srv_ttl_clause(board_id, &mut params).await? {
            Some(ttl_frag) => format!("NOT ({ttl_frag}) AND board_id = ?1"),
            None => "board_id = ?1".to_string(),
        };
        let sp = params.len() + 1;
        params.push(Value::Integer(after_seq));
        let sql = format!("SELECT {RECORD_COLS} FROM wb_records WHERE {where_sql} AND seq > ?{sp} ORDER BY seq ASC LIMIT 1000");
        let conn = self.conn().await;
        let mut rows = conn.query(&sql, Params::Positional(params)).await?;
        let mut out = Vec::new();
        while let Some(record) = read_record(&mut rows).await? {
            out.push(record);
        }
        Ok(out)
    }

    /// Find the first record matching the filter and apply a patch to it (raw
    /// write path — never re-dispatches recipes). Returns the merged payload,
    /// or `None` when nothing matches.
    pub async fn srv_record_patch_first(
        &self,
        board_id: &str,
        conds: &[FilterCond],
        ops: &serde_json::Value,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        let (where_sql, mut params, _) = srv_filter_where(board_id, conds);
        let where_sql = match self.srv_ttl_clause(board_id, &mut params).await? {
            Some(ttl_frag) => format!("NOT ({ttl_frag}) AND {where_sql}"),
            None => where_sql,
        };
        let sql = format!("SELECT seq FROM wb_records WHERE {where_sql} ORDER BY seq ASC LIMIT 1");
        let conn = self.conn().await;
        let mut rows = conn.query(&sql, Params::Positional(params)).await?;
        let seq = match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Integer(i) => i,
                _ => return Ok(None),
            },
            None => return Ok(None),
        };
        drop(rows);
        drop(conn);
        let merged = self.srv_record_patch(board_id, seq, ops).await?;
        Ok(Some(merged))
    }

    /// Apply a list of `(filter, patch)` steps atomically: all succeed or none
    /// are written. Runs under a single BEGIN IMMEDIATE transaction on the
    /// board's write lock. Returns the seqs of records actually patched.
    pub async fn srv_record_patch_many(
        &self,
        board_id: &str,
        steps: &[(Vec<FilterCond>, serde_json::Value)],
    ) -> anyhow::Result<Vec<i64>> {
        let ttl = self.srv_ttl_get(board_id).await?;
        let mut conn = self.conn().await;
        let tx = conn
            .transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate)
            .await?;
        let mut patched = Vec::new();
        for (conds, ops) in steps {
            let (where_sql, mut params, _) = srv_filter_where(board_id, conds);
            let where_sql = match crate::ttl::ttl_frag(&ttl, &mut params) {
                Some(frag) => format!("NOT ({frag}) AND {where_sql}"),
                None => where_sql,
            };
            let sel_sql = format!("SELECT seq, payload FROM wb_records WHERE {where_sql} ORDER BY seq ASC LIMIT 1");
            let mut rows = tx.query(&sel_sql, Params::Positional(params.clone())).await?;
            let Some(row) = rows.next().await? else { continue };
            let seq = match row.get_value(0)? {
                Value::Integer(i) => i,
                _ => continue,
            };
            let current: serde_json::Value = match row.get_value(1)? {
                Value::Text(s) => serde_json::from_str(&s)?,
                _ => continue,
            };
            let mut merged = current;
            match ops {
                serde_json::Value::Object(map) => {
                    let has_dollar = map.keys().any(|k| k.starts_with('$'));
                    if has_dollar {
                        apply_patch_ops(&mut merged, ops)?;
                    } else {
                        for (k, v) in map {
                            merged[k.as_str()] = v.clone();
                        }
                    }
                }
                other => anyhow::bail!("patch must be a JSON object, got {}", serde_json::to_string(other)?),
            }
            let t = sq(&serde_json::to_string(&merged)?);
            let b = sq(board_id);
            tx.execute(
                &format!("UPDATE wb_records SET payload = '{t}' WHERE board_id = '{b}' AND seq = {seq}"),
                Params::Positional(vec![]),
            )
            .await?;
            patched.push(seq);
        }
        tx.commit().await?;
        drop(conn);
        Ok(patched)
    }
}

fn scalar_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn value_at_path<'a>(v: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let segs = crate::filters::path_items(path)?;
    let mut cur = v;
    for seg in &segs {
        match seg {
            crate::filters::PathSeg::Key(k) => cur = cur.as_object()?.get(k.as_str())?,
            crate::filters::PathSeg::Idx(i) => cur = cur.as_array()?.get(*i)?,
        }
    }
    Some(cur)
}

/// Parse `$set/$inc/...` opaque patch object and apply it in place.
fn apply_patch_ops(target: &mut serde_json::Value, ops: &serde_json::Value) -> anyhow::Result<()> {
    let map = match ops {
        serde_json::Value::Object(m) => m,
        _ => anyhow::bail!("patch ops must be an object"),
    };
    for (key, value) in map {
        match key.as_str() {
            "$set" => {
                let fields = value.as_object().ok_or_else(|| anyhow::anyhow!("$set needs an object of {{path: value}}"))?;
                for (path, val) in fields {
                    let segs = crate::filters::path_items(path).ok_or_else(|| anyhow::anyhow!("invalid path '{path}'"))?;
                    set_path(target, segs, val.clone())?;
                }
            }
            "$unset" => {
                let paths: Vec<String> = match value {
                    serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(),
                    serde_json::Value::Object(m) => m.keys().map(|k| k.clone()).collect(),
                    other => anyhow::bail!("$unset requires an array of field paths, got {}", other),
                };
                for path in paths {
                    if let Some(segs) = crate::filters::path_items(&path) {
                        unset_path(target, segs);
                    }
                }
            }
            "$inc" => apply_arith(target, value, 1.0)?,
            "$dec" => apply_arith(target, value, -1.0)?,
            "$mul" => apply_mul(target, value)?,
            other => {
                if let Some(segs) = crate::filters::path_items(other) {
                    set_path(target, segs, value.clone())?;
                } else {
                    anyhow::bail!("unknown patch operator '${}' or invalid path '{other}'", other);
                }
            }
        }
    }
    Ok(())
}

fn int_json(n: f64) -> serde_json::Number {
    if n.fract() == 0.0 && n >= 0.0 {
        serde_json::Number::from(n as u64)
    } else if n.fract() == 0.0 {
        serde_json::Number::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).expect("finite")
    }
}

fn apply_arith(target: &mut serde_json::Value, value: &serde_json::Value, sign: f64) -> anyhow::Result<()> {
    let fields = value.as_object().ok_or_else(|| anyhow::anyhow!("$inc/$dec requires an object of {{path: delta}}"))?;
    for (path, delta) in fields {
        let d = match delta.as_f64() {
            Some(d) => d,
            _ => anyhow::bail!("$inc/$dec delta for '{path}' must be a number"),
        };
        let segs = crate::filters::path_items(path).ok_or_else(|| anyhow::anyhow!("invalid path '{path}'"))?;
        let cur = value_at_path(target, path).cloned()
            .unwrap_or(serde_json::Value::Number(serde_json::Number::from(0)));
        let base = cur.as_f64().unwrap_or(0.0) + sign * d;
        set_path(target, segs, serde_json::Value::Number(int_json(base)))?;
    }
    Ok(())
}

fn apply_mul(target: &mut serde_json::Value, value: &serde_json::Value) -> anyhow::Result<()> {
    let fields = value.as_object().ok_or_else(|| anyhow::anyhow!("$mul requires an object of {{path: factor}}"))?;
    for (path, factor) in fields {
        let f = factor
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("$mul factor for '{path}' must be a number"))?;
        let cur = value_at_path(target, path).cloned()
            .unwrap_or(serde_json::Value::Number(serde_json::Number::from(0)));
        let base = cur.as_f64().unwrap_or(0.0);
        let segs = crate::filters::path_items(path).ok_or_else(|| anyhow::anyhow!("invalid path '{path}'"))?;
        set_path(target, segs, serde_json::Value::Number(int_json(base * f)))?;
    }
    Ok(())
}

fn set_path(target: &mut serde_json::Value, mut segs: Vec<crate::filters::PathSeg>, value: serde_json::Value) -> anyhow::Result<()> {
    if segs.is_empty() {
        anyhow::bail!("empty patch path");
    }
    let last = segs.pop().unwrap();
    let mut cur = target;
    for seg in &segs {
        match seg {
            crate::filters::PathSeg::Key(k) => {
                let obj = cur.as_object_mut().ok_or_else(|| anyhow::anyhow!("patch path traverses a non-object"))?;
                cur = obj.entry(k.as_str()).or_insert(serde_json::Value::Object(Default::default()));
            }
            crate::filters::PathSeg::Idx(i) => {
                let arr = cur.as_array_mut().ok_or_else(|| anyhow::anyhow!("patch path traverses a non-array"))?;
                if *i >= arr.len() {
                    arr.resize(*i + 1, serde_json::Value::Null);
                }
                cur = &mut arr[*i];
            }
        }
    }
    match last {
        crate::filters::PathSeg::Key(k) => {
            let obj = cur.as_object_mut().ok_or_else(|| anyhow::anyhow!("patch path ends in a non-object"))?;
            obj.insert(k.as_str().to_string(), value);
        }
        crate::filters::PathSeg::Idx(i) => {
            let arr = cur.as_array_mut().ok_or_else(|| anyhow::anyhow!("patch path ends in a non-array"))?;
            if i >= arr.len() {
                arr.resize(i + 1, serde_json::Value::Null);
            }
            arr[i] = value;
        }
    }
    Ok(())
}

fn unset_path(target: &mut serde_json::Value, segs: Vec<crate::filters::PathSeg>) {
    let n = segs.len();
    if n == 0 {
        return;
    }
    let mut cur = target;
    for seg in &segs[..n - 1] {
        match seg {
            crate::filters::PathSeg::Idx(i) => {
                let Some(arr) = cur.as_array_mut() else { return };
                if *i >= arr.len() {
                    return;
                }
                cur = &mut arr[*i];
            }
            crate::filters::PathSeg::Key(k) => {
                let Some(obj) = cur.as_object_mut() else { return };
                let Some(next) = obj.get_mut(k.as_str()) else { return };
                cur = next;
            }
        }
    }
    match &segs[n - 1] {
        crate::filters::PathSeg::Key(k) => {
            if let Some(obj) = cur.as_object_mut() {
                obj.remove(k.as_str());
            }
        }
        crate::filters::PathSeg::Idx(i) => {
            if let Some(arr) = cur.as_array_mut() {
                if *i < arr.len() {
                    arr.remove(*i);
                }
            }
        }
    }
}

pub fn srv_session_dir(root: &Path, owner: &str) -> PathBuf {
    root.join("sessions").join(owner)
}

pub fn srv_board_files_dir(root: &Path, owner: &str, board_id: &str) -> PathBuf {
    srv_session_dir(root, owner).join("apps").join(board_id).join("files")
}

pub fn srv_board_file_path(root: &Path, owner: &str, board_id: &str, file_id: &str) -> PathBuf {
    srv_board_files_dir(root, owner, board_id).join(format!("{file_id}.bin"))
}

pub fn srv_list_files(root: &Path, owner: &str, board_id: &str) -> anyhow::Result<Vec<(String, u64)>> {
    let dir = srv_board_files_dir(root, owner, board_id);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("bin") {
            let meta = std::fs::metadata(&path)?;
            let file_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            out.push((file_id, meta.len()));
        }
    }
    Ok(out)
}

fn gen_id(prefix: &str) -> String {
    const CHARS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut s = prefix.to_string();
    let mut n = nanos;
    for _ in 0..16 {
        s.push(CHARS[(n % 36) as usize] as char);
        n /= 36;
    }
    s
}

pub fn gen_board_id() -> String {
    gen_id("b_")
}

pub fn gen_file_id() -> String {
    gen_id("f_")
}