use crate::DbPool;
use serde::Serialize;
use sha2::Digest;
use turso::params::Params;
use turso::Value;

#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    pub id: i64,
    pub event: String,
    pub actor: Option<String>,
    pub writer: Option<String>,
    pub payload_hash: Option<String>,
    pub ts: String,
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

fn hex(digest: &[u8]) -> String {
    const MAP: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push(MAP[(b >> 4) as usize] as char);
        out.push(MAP[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn audit_hash(payload: &serde_json::Value) -> String {
    let mut h = sha2::Sha256::new();
    h.update(serde_json::to_string(payload).unwrap_or_default().as_bytes());
    hex(&h.finalize())
}

/// SHA-256 hex of an arbitrary string (used for secret fingerprints and dedup keys).
pub fn audit_hash_str(s: &str) -> String {
    let mut h = sha2::Sha256::new();
    h.update(s.as_bytes());
    hex(&h.finalize())
}

impl DbPool {
    pub async fn srv_app_audit_set(&self, board_id: &str, owner: &str, on: bool) -> anyhow::Result<()> {
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
        let v = if on { 1 } else { 0 };
        conn.execute(
            &format!("UPDATE wb_apps SET audit = {v} WHERE board_id = '{}'", sq(board_id)),
            Params::Positional(vec![]),
        )
        .await?;
        Ok(())
    }

    pub async fn srv_audit_enabled(&self, board_id: &str) -> anyhow::Result<bool> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT audit FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Integer(i) => Ok(i != 0),
                _ => Ok(false),
            },
            None => Ok(false),
        }
    }

    pub async fn srv_audit_append(
        &self,
        board_id: &str,
        event: &str,
        actor: Option<&str>,
        writer: Option<&str>,
        payload_hash: Option<&str>,
    ) -> anyhow::Result<()> {
        if !self.srv_audit_enabled(board_id).await? {
            return Ok(());
        }
        let conn = self.conn().await;
        let b = sq(board_id);
        let e = sq(event);
        let a = match actor {
            Some(a) => format!("'{}'", sq(a)),
            None => "NULL".to_string(),
        };
        let w = match writer {
            Some(w) => format!("'{}'", sq(w)),
            None => "NULL".to_string(),
        };
        let h = match payload_hash {
            Some(h) => format!("'{}'", sq(h)),
            None => "NULL".to_string(),
        };
        let sql = format!(
            "INSERT INTO wb_audit (board_id, event, actor, writer, payload_hash) VALUES ('{b}', '{e}', {a}, {w}, {h})"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_audit_list(
        &self,
        board_id: &str,
        owner: &str,
        since: Option<&str>,
        limit: i64,
    ) -> anyhow::Result<Vec<AuditRow>> {
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
        let limit = limit.clamp(1, 1000);
        let mut sql = String::from("SELECT id, event, actor, writer, payload_hash, ts FROM wb_audit WHERE board_id = ?1");
        let mut params = vec![Value::from(board_id)];
        if let Some(s) = since {
            if !s.is_empty() {
                sql.push_str(" AND ts >= ?2");
                params.push(Value::from(s));
            }
        }
        sql.push_str(" ORDER BY id DESC LIMIT ?");
        params.push(Value::from(limit));
        let mut rows = conn.query(&sql, Params::Positional(params)).await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let id = match row.get_value(0)? {
                Value::Integer(i) => i,
                _ => 0,
            };
            let event = match row.get_value(1)? {
                Value::Text(s) => s,
                _ => String::new(),
            };
            let actor = match row.get_value(2)? {
                Value::Text(s) => Some(s),
                _ => None,
            };
            let writer = match row.get_value(3)? {
                Value::Text(s) => Some(s),
                _ => None,
            };
            let payload_hash = match row.get_value(4)? {
                Value::Text(s) => Some(s),
                _ => None,
            };
            let ts = match row.get_value(5)? {
                Value::Text(s) => s,
                _ => String::new(),
            };
            out.push(AuditRow { id, event, actor, writer, payload_hash, ts });
        }
        Ok(out)
    }
}