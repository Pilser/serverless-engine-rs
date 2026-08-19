use std::sync::Arc;
use topcoat::Result;
use topcoat::router::{Response, path_param, route, uri};
use topcoat::router::content::websocket::WebSocketUpgrade;
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;
use crate::realtime::Broker;
use crate::routes::serverless::BoardId;
use crate::ws::srv_realtime::handle_srv_ws;

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

#[route(GET "/api/srv/{board_id}/ws")]
pub async fn srv_ws(cx: &Cx, upgrade: WebSocketUpgrade) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx).to_string();
    let params = query_params(cx);
    let after: i64 = params.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
    let filter = params.get("filter").map(|f| url_decode(f)).filter(|f| !f.is_empty());

    let allowed = crate::routes::serverless::authorize_read(cx, ctx, &board_id).await.is_ok();

    let store = ctx.db.clone();
    let realtime: Arc<Broker> = ctx.realtime.clone();
    let upgrade = upgrade.protocols(["bearer"]);
    upgrade.on_upgrade(move |socket| async move {
        handle_srv_ws(socket, board_id, after, filter, allowed, store, realtime).await;
    })
}