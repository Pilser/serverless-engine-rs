use tokio::sync::broadcast;
use zw_db::serverless::SrvRecord;

pub struct Broker {
    tx: broadcast::Sender<String>,
}

impl Broker {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }

    pub fn publish(&self, event: &str) {
        let _ = self.tx.send(event.to_string());
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

pub fn record_event(kind: &str, board: &str, seq: i64, record: Option<&SrvRecord>) -> String {
    let record_json: serde_json::Value = match record {
        Some(r) => serde_json::to_value(r).unwrap_or(serde_json::Value::Null),
        None => serde_json::Value::Null,
    };
    serde_json::json!({
        "board": board,
        "type": format!("record.{}", kind),
        "seq": seq,
        "record": record_json,
    })
    .to_string()
}