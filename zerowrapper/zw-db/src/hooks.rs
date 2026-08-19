use crate::DbPool;
use crate::netguard::valid_url;
use serde::Serialize;
use turso::params::Params;
use turso::Value;

#[derive(Debug, Clone, Serialize)]
pub struct SrvHook {
    pub board_id: String,
    pub url: String,
    pub has_secret: bool,
    pub created_at: String,
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

async fn read_hook(rows: &mut turso::Rows) -> anyhow::Result<Option<SrvHook>> {
    if let Some(row) = rows.next().await? {
        let board_id = match row.get_value(0)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid board_id")),
        };
        let url = match row.get_value(1)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid url")),
        };
        let has_secret = match row.get_value(2)? {
            Value::Integer(i) => i != 0,
            _ => false,
        };
        let created_at = match row.get_value(3)? {
            Value::Text(s) => s,
            _ => return Err(anyhow::anyhow!("invalid created_at")),
        };
        Ok(Some(SrvHook { board_id, url, has_secret, created_at }))
    } else {
        Ok(None)
    }
}

impl DbPool {
    pub(crate) async fn hooks_check_owned(&self, board_id: &str, owner: &str) -> anyhow::Result<()> {
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

    pub async fn srv_hook_register(&self, board_id: &str, owner: &str, url: &str, secret: Option<&str>) -> anyhow::Result<()> {
        if !valid_url(url) {
            anyhow::bail!("webhook url must start with http:// or https://");
        }
        self.hooks_check_owned(board_id, owner).await?;
        let conn = self.conn().await;
        let b = sq(board_id);
        let u = sq(url);
        let s = match secret {
            Some(s) => format!("'{}'", sq(s)),
            None => "NULL".to_string(),
        };
        let sql = format!("INSERT INTO wb_hooks (board_id, url, secret) VALUES ('{b}', '{u}', {s})");
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_hook_remove(&self, board_id: &str, owner: &str, url: &str) -> anyhow::Result<bool> {
        self.hooks_check_owned(board_id, owner).await?;
        let conn = self.conn().await;
        let affected = conn
            .execute(
                "DELETE FROM wb_hooks WHERE board_id = ?1 AND url = ?2",
                Params::Positional(vec![Value::from(board_id), Value::from(url)]),
            )
            .await?;
        if affected == 0 {
            anyhow::bail!("hook not found on board {board_id}");
        }
        Ok(true)
    }

    pub async fn srv_hook_list(&self, board_id: &str, owner: &str) -> anyhow::Result<Vec<SrvHook>> {
        self.hooks_check_owned(board_id, owner).await?;
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT board_id, url, (secret IS NOT NULL), created_at \
                 FROM wb_hooks WHERE board_id = ?1 ORDER BY created_at",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(hook) = read_hook(&mut rows).await? {
            out.push(hook);
        }
        Ok(out)
    }

    pub async fn srv_hook_urls(&self, board_id: &str) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT url, secret FROM wb_hooks WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let url = match row.get_value(0)? {
                Value::Text(s) => s,
                _ => return Err(anyhow::anyhow!("invalid url")),
            };
            let secret = match row.get_value(1)? {
                Value::Text(s) => Some(s),
                _ => None,
            };
            out.push((url, secret));
        }
        Ok(out)
    }
}