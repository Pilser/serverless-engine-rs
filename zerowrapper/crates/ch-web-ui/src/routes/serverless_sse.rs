use std::sync::Arc;
use topcoat::Result;
use topcoat::router::{IntoResponse, Response, StatusCode, path_param, route, uri};
use topcoat::router::content::sse::{Event, KeepAlive, Sse};
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;
use crate::realtime::Broker;
use crate::routes::serverless::BoardId;
use crate::ws::srv_realtime::filter_matches;

fn query_params(cx: &Cx) -> std::collections::HashMap<&str, &str> {
    uri(cx)
        .query()
        .unwrap_or("")
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?;
            let val = parts.next().unwrap_or("");
            Some((key, val))
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn board_event(board_id: &str, kind: &str, seq: i64, record: Option<&zw_db::serverless::SrvRecord>) -> Event {
    let payload = crate::realtime::record_event(kind, board_id, seq, record);
    Event::new().event("record").data(payload)
}

#[route(GET "/api/srv/{board_id}/events")]
pub async fn srv_sse_events(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx).to_string();
    let params = query_params(cx);
    let after: i64 = params.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
    let filter = params.get("filter").map(|f| url_decode(f)).filter(|f| !f.is_empty());
    let filter_json: Option<serde_json::Value> = filter.and_then(|f| serde_json::from_str(&f).ok());

    match crate::routes::serverless::authorize_identity(cx, ctx, &board_id).await {
        Ok(Some(crate::routes::serverless::SrvIdentity::AppKey { role, .. })) if role == "customer" => {
            return Ok(crate::routes::serverless::srv_json_response(
                StatusCode::FORBIDDEN,
                serde_json::json!({"error": "not permitted for customer keys"}),
            ))
        }
        Ok(_) => {}
        Err(resp) => return Ok(resp),
    }
    if let Err(resp) = crate::routes::serverless::authorize_read(cx, ctx, &board_id).await {
        return Ok(resp);
    }

    let store = ctx.db.clone();
    let realtime: Arc<Broker> = ctx.realtime.clone();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    tokio::spawn(async move {
        let mut live = realtime.subscribe();
        if let Ok(pool) = store.board_pool(&board_id).await {
            if let Ok(records) = pool.srv_records_after(&board_id, after).await {
                for record in records {
                    if !filter_matches(&filter_json, &record.payload) {
                        continue;
                    }
                    if tx.send(board_event(&board_id, "created", record.seq, Some(&record))).is_err() {
                        return;
                    }
                }
            }
        }
        loop {
            match live.recv().await {
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
                    if tx.send(Event::new().event("record").data(event)).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });

    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|event| {
            let item: topcoat::Result<Event> = Ok(event);
            (item, rx)
        })
    });
    let sse = Sse::new(stream).keep_alive(KeepAlive::new());
    sse.into_response(cx)
}
