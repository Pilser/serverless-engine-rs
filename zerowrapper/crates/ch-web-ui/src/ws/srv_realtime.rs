use std::sync::Arc;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use topcoat::router::content::websocket::{Message, WebSocket};
use zw_db::SessionStore;

use crate::realtime::Broker;

pub async fn handle_srv_ws(
    socket: WebSocket,
    board_id: String,
    after: i64,
    filter: Option<String>,
    allowed: bool,
    store: SessionStore,
    realtime: Arc<Broker>,
) {
    let (mut sink, mut stream) = socket.split();
    if !allowed {
        let _ = sink.send(Message::text("{\"error\":\"unauthorized\"}")).await;
        return;
    }
    let filter_json: Option<serde_json::Value> = filter.and_then(|f| serde_json::from_str(&f).ok());

    let mut rx = realtime.subscribe();

    if let Ok(pool) = store.board_pool(&board_id).await {
        if let Ok(records) = pool.srv_records_after(&board_id, after).await {
            for record in records {
                if !filter_matches(&filter_json, &record.payload) {
                    continue;
                }
                let event = crate::realtime::record_event("created", &board_id, record.seq, Some(&record));
                if sink.send(Message::text(event)).await.is_err() {
                    return;
                }
            }
        }
    }

    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
            incoming = rx.recv() => match incoming {
                Ok(event) => {
                    let parsed: serde_json::Value = match serde_json::from_str(&event) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    if parsed.get("board").and_then(|b| b.as_str()) != Some(board_id.as_str()) {
                        continue;
                    }
                    if let Some(payload) = parsed.get("record").and_then(|r| r.get("payload")) {
                        if !filter_matches(&filter_json, payload) {
                            continue;
                        }
                    }
                    if sink.send(Message::text(event)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
}

pub fn filter_matches(filter: &Option<serde_json::Value>, payload: &serde_json::Value) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let Some(map) = filter.as_object() else {
        return true;
    };
    for (key, want) in map {
        let Some(actual) = value_at(payload, key) else {
            return false;
        };
        if !eq_value(actual, want) {
            return false;
        }
    }
    true
}

fn value_at<'a>(payload: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = payload;
    for seg in path.split('.') {
        cur = cur.as_object()?.get(seg)?;
    }
    Some(cur)
}

fn eq_value(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) {
        return x == y;
    }
    a == b
}