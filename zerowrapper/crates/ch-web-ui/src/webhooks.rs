use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chrono::{Duration, NaiveDateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::time::Duration as StdDuration;
use tokio::time;
use zw_db::SessionStore;

type HmacSha256 = Hmac<Sha256>;

const BACKOFF: [u64; 5] = [0, 1, 5, 30, 120];
const MAX_ATTEMPTS: i64 = 5;

pub fn spawn_webhook_worker(store: SessionStore) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { run_worker(store).await })
}

pub fn waos_signature(body: &str, secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body.as_bytes());
    STANDARD.encode(mac.finalize().into_bytes())
}

pub fn waos_signature_hex(body: &[u8], secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn fmt_iso(now: &NaiveDateTime) -> String {
    now.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn ssrf_blocked(url: &str) -> bool {
    !zw_db::netguard::valid_url(url)
}

fn next_attempt(attempts: i64, now: &NaiveDateTime) -> Option<String> {
    if attempts + 1 > MAX_ATTEMPTS {
        return None;
    }
    let backoff = BACKOFF[(attempts.min(4)) as usize];
    Some(fmt_iso(&(*now + Duration::seconds(backoff as i64))))
}

async fn run_worker(store: SessionStore) {
    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("webhook worker: failed to build http client: {e}");
            return;
        }
    };
    let mut secrets: HashMap<String, HashMap<String, String>> = HashMap::new();
    loop {
        time::sleep(StdDuration::from_secs(2)).await;
        let now = Utc::now().naive_utc();
        let now_iso = fmt_iso(&now);
        let due = match store.system().srv_hook_due(&now_iso, 64).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("webhook worker: due query failed: {e}");
                continue;
            }
        };
        for d in due {
            if ssrf_blocked(&d.url) {
                let _ = store.system().srv_hook_mark(d.id, d.attempts + 1, Some("blocked"), None, None).await;
                tracing::warn!("webhook worker: blocked SSRF target {}", d.url);
                continue;
            }
            if !secrets.contains_key(&d.board_id) {
                if let Ok(hooks) = store.hook_urls(&d.board_id).await {
                    secrets.insert(d.board_id.clone(), hooks.into_iter().map(|(u, s)| (u, s.unwrap_or_default())).collect());
                }
            }
            let secret = secrets
                .get(&d.board_id)
                .and_then(|m| m.get(&d.url))
                .cloned()
                .unwrap_or_default();
            let mut body: serde_json::Value = match serde_json::from_str(&d.event) {
                Ok(v) => v,
                Err(_) => {
                    let next = next_attempt(d.attempts, &now);
                    let _ = store.system().srv_hook_mark(d.id, d.attempts + 1, Some("serde-fail"), next.as_deref(), None).await;
                    continue;
                }
            };
            body["hooks_secret"] = serde_json::json!(secret);
            let body_str = body.to_string();
            let sig = waos_signature(&body_str, &secret);
            let result = client
                .post(&d.url)
                .header("X-WAOS-Signature", sig)
                .body(body_str)
                .timeout(StdDuration::from_secs(10))
                .send()
                .await;
            match result {
                Ok(resp) if resp.status().is_success() => {
                    let status = resp.status().as_u16().to_string();
                    let _ = store
                        .system()
                        .srv_hook_mark(d.id, d.attempts + 1, Some(&status), None, Some(&now_iso))
                        .await;
                }
                Ok(resp) => {
                    let status = resp.status().as_u16().to_string();
                    let next = next_attempt(d.attempts, &now);
                    let _ = store.system().srv_hook_mark(d.id, d.attempts + 1, Some(&status), next.as_deref(), None).await;
                }
                Err(e) => {
                    tracing::warn!("webhook worker: POST {} failed: {e}", d.url);
                    let next = next_attempt(d.attempts, &now);
                    let _ = store.system().srv_hook_mark(d.id, d.attempts + 1, Some("http-error"), next.as_deref(), None).await;
                }
            }
        }
    }
}