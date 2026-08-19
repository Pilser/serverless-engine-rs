use crate::DbPool;
use serde::Serialize;
use sha2::{Digest, Sha256};
use turso::params::Params;
use turso::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct SrvApiKey {
    pub bucket: String,
    pub board_id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
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

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

fn key_hash(secret: &str) -> String {
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    hex(&h.finalize())
}

fn valid_role(role: &str) -> bool {
    matches!(role, "customer" | "reader" | "writer" | "list" | "admin")
}

impl DbPool {
    async fn keys_check_owned(&self, board_id: &str, owner: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT owner_key FROM srv_board_index WHERE board_id = ?1",
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
}

async fn read_key(rows: &mut turso::Rows) -> anyhow::Result<Option<SrvApiKey>> {
    if let Some(row) = rows.next().await? {
        let bucket = match row.get_value(0)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid bucket")),
        };
        let board_id = match row.get_value(1)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid board_id")),
        };
        let role = match row.get_value(2)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid role")),
        };
        let writer = match row.get_value(3)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let scope = match row.get_value(4)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let created_at = match row.get_value(5)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid created_at")),
        };
        let revoked_at = match row.get_value(6)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        Ok(Some(SrvApiKey { bucket, board_id, role, writer, scope, created_at, revoked_at }))
    } else {
        Ok(None)
    }
}

impl DbPool {
    pub async fn srv_key_issue(&self, board_id: &str, owner: &str, role: &str, writer: Option<&str>, scope: Option<&str>) -> anyhow::Result<(String, String)> {
        self.keys_check_owned(board_id, owner).await?;
        if !valid_role(role) {
            anyhow::bail!("invalid role '{role}' (expected customer, reader, writer, list, admin)");
        }
        if role == "customer" && scope.is_none() {
            anyhow::bail!("customer keys require a scope (customer id)");
        }
        if role != "customer" && scope.is_some() {
            anyhow::bail!("only customer keys may carry a scope");
        }
        let conn = self.conn().await;
        let bucket = Uuid::new_v4().simple().to_string();
        let secret = Uuid::new_v4().simple().to_string();
        let b = sq(board_id);
        let h = sq(&key_hash(&secret));
        let r = sq(role);
        let w = match writer {
            Some(w) => format!("'{}'", sq(w)),
            None => "NULL".to_string(),
        };
        let s = match scope {
            Some(s) => format!("'{}'", sq(s)),
            None => "NULL".to_string(),
        };
        let sql = format!(
            "INSERT INTO wb_keys (bucket, board_id, key_hash, role, writer, scope) VALUES ('{bucket}', '{b}', '{h}', '{r}', {w}, {s})"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok((bucket, secret))
    }

    pub async fn srv_key_list(&self, board_id: &str, owner: &str) -> anyhow::Result<Vec<SrvApiKey>> {
        self.keys_check_owned(board_id, owner).await?;
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT bucket, board_id, role, writer, scope, created_at, revoked_at \
                 FROM wb_keys WHERE board_id = ?1 ORDER BY created_at DESC",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(key) = read_key(&mut rows).await? {
            out.push(key);
        }
        Ok(out)
    }

    pub async fn srv_key_revoke(&self, board_id: &str, owner: &str, bucket: &str) -> anyhow::Result<bool> {
        self.keys_check_owned(board_id, owner).await?;
        let conn = self.conn().await;
        let affected = conn
            .execute(
                "UPDATE wb_keys SET revoked_at = datetime('now') WHERE board_id = ?1 AND bucket = ?2 AND revoked_at IS NULL",
                Params::Positional(vec![Value::from(board_id), Value::from(bucket)]),
            )
            .await?;
        if affected == 0 {
            anyhow::bail!("key {bucket} not found on board {board_id}");
        }
        Ok(true)
    }

    pub async fn srv_key_auth(&self, secret: &str) -> anyhow::Result<Option<SrvApiKey>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT bucket, board_id, role, writer, scope, created_at, revoked_at \
                 FROM wb_keys WHERE key_hash = ?1 AND revoked_at IS NULL LIMIT 1",
                Params::Positional(vec![Value::from(key_hash(secret))]),
            )
            .await?;
        read_key(&mut rows).await
    }

    pub async fn srv_board_has_keys(&self, board_id: &str) -> anyhow::Result<bool> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT 1 FROM wb_keys WHERE board_id = ?1 AND revoked_at IS NULL LIMIT 1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        match rows.next().await? {
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
}