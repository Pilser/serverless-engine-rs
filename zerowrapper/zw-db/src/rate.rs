use crate::DbPool;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tokio::sync::Mutex;
use turso::params::Params;
use turso::Value;

const ACTION_WINDOW_SECS: i64 = 60;
const DAY_WINDOW_SECS: i64 = 86400;
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, Serialize)]
pub struct RateConfig {
    pub submit: u64,
    pub upload: u64,
    pub search: u64,
    pub read: u64,
    pub per_day: u64,
}

impl Default for RateConfig {
    fn default() -> Self {
        Self {
            submit: 120,
            upload: 20,
            search: 240,
            read: 600,
            per_day: 100_000,
        }
    }
}

impl RateConfig {
    pub fn parse(json: &serde_json::Value) -> RateConfig {
        let mut cfg = RateConfig::default();
        if let Some(v) = json.get("submit").and_then(|v| v.as_u64()) {
            cfg.submit = v;
        }
        if let Some(v) = json.get("upload").and_then(|v| v.as_u64()) {
            cfg.upload = v;
        }
        if let Some(v) = json.get("search").and_then(|v| v.as_u64()) {
            cfg.search = v;
        }
        if let Some(v) = json.get("read").and_then(|v| v.as_u64()) {
            cfg.read = v;
        }
        if let Some(v) = json.get("per_day").and_then(|v| v.as_u64()) {
            cfg.per_day = v;
        }
        cfg
    }

    pub fn limit(&self, action: &str) -> u64 {
        match action {
            "submit" => self.submit,
            "upload" => self.upload,
            "search" => self.search,
            _ => self.read,
        }
    }
}

pub fn token_digest(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    let digest = h.finalize();
    const MAP: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest.iter() {
        out.push(MAP[(b >> 4) as usize] as char);
        out.push(MAP[(b & 0x0f) as usize] as char);
    }
    out
}

#[derive(Debug, Clone, Default)]
struct Window {
    count: u64,
    start: i64,
}

#[derive(Default)]
pub struct RateLimiter {
    inner: Mutex<HashMap<String, Window>>,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl RateLimiter {
    pub async fn allow(&self, key: &str, action: &str, limit: u64, per_day: u64) -> (bool, u64, u64) {
        let mut inner = self.inner.lock().await;
        let now = now_secs();
        if inner.len() > MAX_ENTRIES {
            inner.retain(|_, w| now - w.start < DAY_WINDOW_SECS);
        }
        let action_key = format!("{key}:{action}");
        let day_key = format!("{key}:day");
        let mut action_win = match inner.get(&action_key) {
            Some(w) if now - w.start < ACTION_WINDOW_SECS => w.clone(),
            _ => Window { count: 0, start: now },
        };
        action_win.count += 1;
        let mut day_win = match inner.get(&day_key) {
            Some(w) if now - w.start < DAY_WINDOW_SECS => w.clone(),
            _ => Window { count: 0, start: now },
        };
        day_win.count += 1;

        let action_allowed = action_win.count <= limit;
        let day_allowed = day_win.count <= per_day;
        let allowed = action_allowed && day_allowed;
        let action_reset = (action_win.start + ACTION_WINDOW_SECS - now).max(0) as u64;
        let day_reset = (day_win.start + DAY_WINDOW_SECS - now).max(0) as u64;
        let retry_after = if !day_allowed { day_reset } else { action_reset };
        let remaining = limit
            .saturating_sub(action_win.count)
            .min(per_day.saturating_sub(day_win.count));

        inner.insert(action_key, action_win);
        inner.insert(day_key, day_win);
        (allowed, retry_after, remaining)
    }
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

impl DbPool {
    async fn rate_check_owned(&self, board_id: &str, owner: &str) -> anyhow::Result<()> {
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

    pub async fn srv_rate_set(&self, board_id: &str, owner: &str, json: Option<&str>) -> anyhow::Result<()> {
        self.rate_check_owned(board_id, owner).await?;
        let conn = self.conn().await;
        let value = match json {
            Some(j) => format!("'{}'", sq(j)),
            None => "NULL".to_string(),
        };
        let b = sq(board_id);
        let sql = format!("UPDATE wb_apps SET rate_json = {value} WHERE board_id = '{b}'");
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_rate_config(&self, board_id: &str) -> anyhow::Result<RateConfig> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT rate_json FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Text(s) => {
                    let json = serde_json::from_str::<serde_json::Value>(&s).unwrap_or(serde_json::Value::Null);
                    Ok(RateConfig::parse(&json))
                }
                _ => Ok(RateConfig::default()),
            },
            None => Ok(RateConfig::default()),
        }
    }
}