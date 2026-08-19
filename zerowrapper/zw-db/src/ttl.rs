use crate::DbPool;
use serde::Serialize;
use turso::params::Params;
use turso::Value;

#[derive(Debug, Clone, Serialize)]
pub struct SrvTtl {
    pub seconds: Option<i64>,
    pub field: Option<String>,
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

impl DbPool {
    async fn ttl_check_owned(&self, board_id: &str, owner: &str) -> anyhow::Result<()> {
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
        Ok(())
    }

    pub async fn srv_ttl_set(&self, board_id: &str, owner: &str, seconds: Option<i64>, field: Option<&str>) -> anyhow::Result<()> {
        self.ttl_check_owned(board_id, owner).await?;
        let conn = self.conn().await;
        let secs = match seconds {
            Some(n) => n.to_string(),
            None => "NULL".to_string(),
        };
        let f = match field {
            Some(p) if !p.is_empty() => format!("'{}'", sq(p)),
            _ => "NULL".to_string(),
        };
        let b = sq(board_id);
        let sql = format!("UPDATE wb_apps SET ttl_seconds = {secs}, ttl_field = {f} WHERE board_id = '{b}'");
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_ttl_get(&self, board_id: &str) -> anyhow::Result<SrvTtl> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT ttl_seconds, ttl_field FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        match rows.next().await? {
            Some(row) => {
                let seconds = match row.get_value(0)? {
                    Value::Integer(i) => Some(i),
                    _ => None,
                };
                let field = match row.get_value(1)? {
                    Value::Text(s) => Some(s),
                    _ => None,
                };
                Ok(SrvTtl { seconds, field })
            }
            None => Ok(SrvTtl { seconds: None, field: None }),
        }
    }

    pub async fn srv_ttl_clause(&self, board_id: &str, params: &mut Vec<Value>) -> anyhow::Result<Option<String>> {
        let ttl = self.srv_ttl_get(board_id).await?;
        Ok(ttl_frag(&ttl, params))
    }

    pub async fn srv_ttl_sweep(&self) -> anyhow::Result<u64> {
        let conn = self.conn().await;
        let mut total: u64 = 0;
        let secs: Vec<i64> = {
            let mut rows = conn
                .query(
                    "SELECT DISTINCT ttl_seconds FROM wb_apps WHERE ttl_seconds IS NOT NULL",
                    Params::Positional(vec![]),
                )
                .await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                if let Value::Integer(i) = row.get_value(0)? {
                    out.push(i);
                }
            }
            out
        };
        for n in secs {
            let sql = format!(
                "DELETE FROM wb_records WHERE board_id IN (SELECT board_id FROM wb_apps WHERE ttl_seconds = {n}) \
                 AND created_at < datetime('now', '-' || {n} || ' seconds')"
            );
            total += conn.execute(&sql, Params::Positional(vec![])).await? as u64;
        }
        let boards: Vec<(String, String)> = {
            let mut rows = conn
                .query(
                    "SELECT board_id, ttl_field FROM wb_apps WHERE ttl_field IS NOT NULL",
                    Params::Positional(vec![]),
                )
                .await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                let board_id = match row.get_value(0)? {
                    Value::Text(s) => s,
                    _ => continue,
                };
                let field = match row.get_value(1)? {
                    Value::Text(s) => s,
                    _ => continue,
                };
                out.push((board_id, field));
            }
            out
        };
        for (board_id, field) in boards {
            total += conn
                .execute(
                    "DELETE FROM wb_records WHERE board_id = ?1 AND json_extract(payload, ?2) IS NOT NULL \
                     AND CAST(json_extract(payload, ?3) AS TEXT) < datetime('now')",
                    Params::Positional(vec![Value::from(board_id), Value::from(field.clone()), Value::from(field)]),
                )
                .await? as u64;
        }
        Ok(total)
    }
}

/// Build the TTL expiry fragment from an already-loaded `SrvTtl` (no DB
/// access), appending any bound params. Safe to call while a transaction holds
/// the connection.
pub fn ttl_frag(ttl: &SrvTtl, params: &mut Vec<Value>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(n) = ttl.seconds {
        parts.push(format!("created_at < datetime('now', '-' || {n} || ' seconds')"));
    }
    if let Some(f) = &ttl.field {
        let p1 = params.len() + 1;
        params.push(Value::from(f.clone()));
        let p2 = params.len() + 1;
        params.push(Value::from(f.clone()));
        parts.push(format!(
            "json_extract(payload, ?{p1}) IS NOT NULL AND CAST(json_extract(payload, ?{p2}) AS TEXT) < datetime('now')"
        ));
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(" OR "))
}