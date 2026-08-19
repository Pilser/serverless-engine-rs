use crate::DbPool;
use serde::Serialize;
use turso::params::Params;
use turso::Value;

#[derive(Debug, Clone, Serialize)]
pub struct SrvDelivery {
    pub id: i64,
    pub board_id: String,
    pub url: String,
    pub event: String,
    pub attempts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<String>,
    pub created_at: String,
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

impl DbPool {
    async fn deliveries_check_owned(&self, board_id: &str, owner: &str) -> anyhow::Result<()> {
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

async fn read_delivery(rows: &mut turso::Rows) -> anyhow::Result<Option<SrvDelivery>> {
    if let Some(row) = rows.next().await? {
        let id = match row.get_value(0)? {
            Value::Integer(i) => i,
            _ => return Err(anyhow::anyhow!("invalid id")),
        };
        let board_id = match row.get_value(1)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid board_id")),
        };
        let url = match row.get_value(2)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid url")),
        };
        let event = match row.get_value(3)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid event")),
        };
        let attempts = match row.get_value(4)? {
            Value::Integer(i) => i,
            _ => return Err(anyhow::anyhow!("invalid attempts")),
        };
        let last_status = match row.get_value(5)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let delivered_at = match row.get_value(6)? {
            Value::Text(s) => Some(s),
            _ => None,
        };
        let created_at = match row.get_value(7)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid created_at")),
        };
        Ok(Some(SrvDelivery { id, board_id, url, event, attempts, last_status, delivered_at, created_at }))
    } else {
        Ok(None)
    }
}

impl DbPool {
    pub async fn srv_hook_enqueue(&self, board_id: &str, url: &str, event: &str) -> anyhow::Result<i64> {
        let conn = self.conn().await;
        let b = sq(board_id);
        let u = sq(url);
        let e = sq(event);
        let sql = format!(
            "INSERT INTO wb_hook_deliveries (board_id, url, event, attempts, next_attempt_at) \
             VALUES ('{b}', '{u}', '{e}', 0, datetime('now'))"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(conn.last_insert_rowid())
    }

    pub async fn srv_hook_due(&self, now_iso: &str, limit: i64) -> anyhow::Result<Vec<SrvDelivery>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT id, board_id, url, event, attempts, last_status, delivered_at, created_at \
                 FROM wb_hook_deliveries \
                 WHERE delivered_at IS NULL AND next_attempt_at IS NOT NULL AND next_attempt_at <= ?1 \
                 ORDER BY id ASC LIMIT ?2",
                Params::Positional(vec![Value::from(now_iso), Value::Integer(limit)]),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(d) = read_delivery(&mut rows).await? {
            out.push(d);
        }
        Ok(out)
    }

    pub async fn srv_hook_mark(&self, id: i64, attempts: i64, last_status: Option<&str>, next_attempt_at: Option<&str>, delivered_at: Option<&str>) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let st = match last_status {
            Some(s) => format!("'{}'", sq(s)),
            None => "NULL".to_string(),
        };
        let next = match next_attempt_at {
            Some(s) => format!("'{}'", sq(s)),
            None => "NULL".to_string(),
        };
        let del = match delivered_at {
            Some(s) => format!("'{}'", sq(s)),
            None => "NULL".to_string(),
        };
        let sql = format!(
            "UPDATE wb_hook_deliveries SET attempts = {attempts}, last_status = {st}, \
             next_attempt_at = {next}, delivered_at = {del} WHERE id = {id}"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_hook_reschedule(&self, board_id: &str, owner: &str, url: &str) -> anyhow::Result<i64> {
        self.deliveries_check_owned(board_id, owner).await?;
        let conn = self.conn().await;
        let affected = conn
            .execute(
                "UPDATE wb_hook_deliveries SET next_attempt_at = datetime('now') \
                 WHERE board_id = ?1 AND url = ?2 AND delivered_at IS NULL AND attempts <= 5",
                Params::Positional(vec![Value::from(board_id), Value::from(url)]),
            )
            .await?;
        Ok(affected as i64)
    }

    pub async fn srv_hook_deliveries(&self, board_id: &str, owner: &str, url: Option<&str>) -> anyhow::Result<Vec<SrvDelivery>> {
        self.deliveries_check_owned(board_id, owner).await?;
        let conn = self.conn().await;
        let sql = match url {
            Some(_) => "SELECT id, board_id, url, event, attempts, last_status, delivered_at, created_at \
                        FROM wb_hook_deliveries WHERE board_id = ?1 AND url = ?2 ORDER BY id ASC",
            None => "SELECT id, board_id, url, event, attempts, last_status, delivered_at, created_at \
                     FROM wb_hook_deliveries WHERE board_id = ?1 ORDER BY id DESC",
        };
        let params = match url {
            Some(u) => Params::Positional(vec![Value::from(board_id), Value::from(u)]),
            None => Params::Positional(vec![Value::from(board_id)]),
        };
        let mut rows = conn.query(sql, params).await?;
        let mut out = Vec::new();
        while let Some(d) = read_delivery(&mut rows).await? {
            out.push(d);
        }
        Ok(out)
    }
}