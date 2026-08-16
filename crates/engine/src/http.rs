use serde_json::{json, Value as Json};
use std::sync::OnceLock;

pub trait HttpCaller: Send + Sync + 'static {
    fn call(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Json,
        timeout_ms: u64,
    ) -> anyhow::Result<(u16, Json)>;
}

pub struct StubCaller;

impl HttpCaller for StubCaller {
    fn call(
        &self,
        _url: &str,
        _headers: &[(String, String)],
        _body: &Json,
        _timeout_ms: u64,
    ) -> anyhow::Result<(u16, Json)> {
        Ok((200, json!({"ok": true})))
    }
}

static CALLER: OnceLock<Box<dyn HttpCaller>> = OnceLock::new();

pub fn install(caller: Box<dyn HttpCaller>) {
    let _ = CALLER.set(caller);
}

pub fn http_call(
    url: &str,
    headers: &[(String, String)],
    body: &Json,
    timeout_ms: u64,
) -> anyhow::Result<(u16, Json)> {
    match CALLER.get() {
        Some(c) => c.call(url, headers, body, timeout_ms),
        None => StubCaller.call(url, headers, body, timeout_ms),
    }
}