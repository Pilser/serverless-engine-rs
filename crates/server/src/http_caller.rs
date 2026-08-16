use engine::http::HttpCaller;
use serde_json::Value as Json;

pub struct ReqwestCaller {
    client: reqwest::blocking::Client,
}

impl ReqwestCaller {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder().build()?,
        })
    }
}

impl HttpCaller for ReqwestCaller {
    fn call(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Json,
        timeout_ms: u64,
    ) -> anyhow::Result<(u16, Json)> {
        let mut req = self
            .client
            .post(url)
            .timeout(std::time::Duration::from_millis(timeout_ms));
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if !body.is_null() {
            req = req.header("content-type", "application/json");
            req = req.body(body.to_string());
        }
        let resp = req.send()?;
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        let parsed: Json = serde_json::from_str(&text)
            .unwrap_or_else(|_| if text.is_empty() { Json::Null } else { Json::String(text) });
        Ok((status, parsed))
    }
}