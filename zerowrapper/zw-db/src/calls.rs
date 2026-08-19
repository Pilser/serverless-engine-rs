use crate::DbPool;
use crate::netguard::valid_url;
use serde::Serialize;
use turso::params::Params;
use turso::Value;
use std::time::Duration;

const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const MAX_TIMEOUT_MS: u64 = 60_000;

#[derive(Debug, Clone, Serialize)]
pub struct CallResult {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub duration_ms: u64,
}

impl DbPool {
    pub async fn srv_http_call(
        &self,
        board_id: &str,
        owner: &str,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
        timeout_ms: u64,
    ) -> anyhow::Result<CallResult> {
        if !valid_url(url) {
            anyhow::bail!("url must be a public http(s) address (private/loopback targets are blocked)");
        }
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

        let method = match method.to_uppercase().as_str() {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "PATCH" => reqwest::Method::PATCH,
            "DELETE" => reqwest::Method::DELETE,
            other => anyhow::bail!("unsupported http method '{other}'"),
        };
        let effective = if timeout_ms == 0 { DEFAULT_TIMEOUT_MS } else { timeout_ms };
        let timeout = Duration::from_millis(effective.clamp(500, MAX_TIMEOUT_MS));

        let client = reqwest::Client::builder().timeout(timeout).build()?;
        let mut req = client.request(method, url);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some(b) = body {
            req = req.body(b.to_string());
        }

        let started = std::time::Instant::now();
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => anyhow::bail!("request failed: {e}"),
        };
        let status = resp.status().as_u16();
        let resp_headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|s| (k.as_str().to_string(), s.to_string())))
            .collect();
        let content_len = resp.content_length().unwrap_or(0);
        if content_len > MAX_RESPONSE_BYTES {
            anyhow::bail!("response too large ({content_len} bytes, max {MAX_RESPONSE_BYTES})");
        }
        let bytes = resp.bytes().await?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            anyhow::bail!("response too large ({} bytes, max {MAX_RESPONSE_BYTES})", bytes.len());
        }
        let body = String::from_utf8_lossy(&bytes).to_string();
        let duration_ms = started.elapsed().as_millis() as u64;

        Ok(CallResult { status, headers: resp_headers, body, duration_ms })
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    async fn http_call_rejects_private_targets_and_bad_method() {
        let dir = std::env::temp_dir().join(format!("zwdb_call_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = crate::DbPool::open(dir.join("test.db").to_str().unwrap()).await.unwrap();
        pool.migrate().await.unwrap();
        pool.migrate_session().await.unwrap();
        let board = pool.srv_app_create("owner", "t", None, true, None).await.unwrap();

        let err = pool.srv_http_call(&board, "owner", "GET", "http://169.254.169.254/latest/meta-data/", &[], None, 0).await;
        assert!(err.is_err(), "cloud-metadata link-local target must be blocked");

        let err = pool.srv_http_call(&board, "owner", "GET", "http://127.0.0.1:9/x", &[], None, 0).await;
        assert!(err.is_err(), "loopback target must be blocked");

        let err = pool.srv_http_call(&board, "owner", "OPTIONS", "https://example.com/x", &[], None, 0).await;
        assert!(err.is_err(), "non-whitelisted method must be rejected");

        let err = pool.srv_http_call(&board, "mallory", "GET", "https://example.com/x", &[], None, 0).await;
        assert!(err.is_err(), "non-owner cannot call");

        std::fs::remove_dir_all(&dir).ok();
    }
}
