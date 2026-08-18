use crate::{Error, Result};
use serde_json::{json, Value as Json};

/// POST /v2/query transport. Sends one direct JSON envelope; parses the
/// normalized response (one top-level member per returned batch name).
#[derive(Debug, Clone)]
pub struct Client {
    base: String,
    http: reqwest::blocking::Client,
    timeout_ms: u64,
}

impl Client {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let base = base_url.into().trim_end_matches('/').to_string();
        let http = reqwest::blocking::Client::builder()
            .build()
            .map_err(Error::Http)?;
        Ok(Self { base, http, timeout_ms: 30_000 })
    }

    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Execute a raw envelope (used by the typed builders via [`Self::execute`]).
    pub fn post(&self, request_type: &str, query_name: &str, body: Json) -> Result<Response> {
        let envelope = json!({
            "request_type": request_type,
            "query_name": query_name,
            "query": body,
        });
        self.execute(envelope)
    }

    /// Send an envelope built by the request module. Returns the parsed
    /// response object: `{ name: [rows...] }`. Retries transient transport
    /// errors (connection drops under sustained load) up to 3 times.
    pub fn execute(&self, envelope: Json) -> Result<Response> {
        let url = format!("{}/v2/query", self.base);
        let mut last_err: Option<Error> = None;
        for attempt in 0..3 {
            match self.try_execute(&url, &envelope) {
                Ok(r) => return Ok(r),
                Err(e) => {
                    // Only retry transport-level failures; HTTP status errors
                    // (e.g. index_not_found) are deterministic.
                    let retryable = matches!(e, Error::Http(_));
                    last_err = Some(e);
                    if !retryable {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100 * (attempt + 1)));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Other("request failed".into())))
    }

    fn try_execute(&self, url: &str, envelope: &Json) -> Result<Response> {
        let resp = self
            .http
            .post(url)
            .timeout(std::time::Duration::from_millis(self.timeout_ms))
            .json(envelope)
            .send()
            .map_err(Error::Http)?;
        let status = resp.status();
        let text = resp.text().map_err(Error::Http)?;
        if !status.is_success() {
            return Err(Error::Status { status: status.as_u16(), body: text });
        }
        let value: Json = serde_json::from_str(&text)
            .map_err(|e| Error::InvalidResponse(format!("bad json: {e}: {text}")))?;
        Ok(Response::new(value))
    }
}

/// Parsed normalized response: one member per returned batch name.
#[derive(Debug, Clone)]
pub struct Response {
    pub value: Json,
}

impl Response {
    pub fn new(value: Json) -> Self {
        Self { value }
    }

    /// Rows for a returned batch name.
    pub fn rows(&self, name: &str) -> Vec<Json> {
        self.value
            .get(name)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    }

    /// Raw value for a returned batch name (e.g. a count scalar).
    pub fn get(&self, name: &str) -> Option<&Json> {
        self.value.get(name)
    }
}
