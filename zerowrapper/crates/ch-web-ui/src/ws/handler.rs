use std::sync::Arc;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use topcoat::router::content::websocket::{Message, WebSocket};
use zw_core::{Attachment, ChannelMessage, ChannelRegistryHandle, ZeroClawHandle, StreamEvent};

fn spawn_whiteboard_forwarding(
    ws_sender: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    handle: zw_whiteboard::WhiteboardHandle,
    session_id: String,
    spawned: &mut Option<String>,
) {
    if spawned.as_deref() == Some(session_id.as_str()) {
        return;
    }
    *spawned = Some(session_id.clone());
    let ws = ws_sender.clone();
    tokio::spawn(async move {
        let mut rx = handle.subscribe();
        loop {
            match rx.recv().await {
                Ok(zw_whiteboard::WhiteboardEvent::BoardUpdate {
                    session_id: sid,
                    state,
                }) => {
                    if sid != session_id {
                        continue;
                    }
                    let flat = serde_json::json!({
                        "type": "whiteboard_update",
                        "session_id": sid,
                        "state": state,
                    });
                    let mut sender = ws.lock().await;
                    if sender.send(Message::Text(flat.to_string().into())).await.is_err() {
                        break;
                    }
                }
                Ok(zw_whiteboard::WhiteboardEvent::WhiteboardUpdate {
                    session_id: sid,
                    board,
                }) => {
                    if sid != session_id {
                        continue;
                    }
                    let flat = serde_json::json!({
                        "type": "whiteboard_update",
                        "session_id": sid,
                        "body_md": board.body_md,
                        "pins": board.pins,
                    });
                    let mut sender = ws.lock().await;
                    if sender.send(Message::Text(flat.to_string().into())).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

pub async fn handle_ws_connection(
    socket: WebSocket,
    registry: ChannelRegistryHandle,
    zeroclaw: Arc<ZeroClawHandle>,
    agent: Arc<String>,
    whiteboard: Option<zw_whiteboard::WhiteboardHandle>,
) {
    let (ws_sender, mut ws_receiver) = socket.split();
    let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));

    let mut current_session_id = String::new();
    let mut whiteboard_forwarding_for: Option<String> = None;

    while let Some(msg) = ws_receiver.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) => {
                let mut sender = ws_sender.lock().await;
                let _ = sender.send(Message::Pong(vec![].into())).await;
                continue;
            }
            Err(e) => {
                tracing::warn!("WebSocket error: {}", e);
                break;
            }
            _ => continue,
        };

        let parsed: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                let mut sender = ws_sender.lock().await;
                let err = serde_json::json!({"type": "error", "error": format!("Invalid JSON: {}", e)});
                let _ = sender.send(Message::Text(err.to_string().into())).await;
                continue;
            }
        };

        if parsed.get("type").and_then(|v| v.as_str()) == Some("cancel") {
            let sid = parsed.get("session_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    if current_session_id.is_empty() { None } else { Some(current_session_id.as_str()) }
                });
            if let Some(sid) = sid {
                let _ = zeroclaw.cancel_session(sid.to_string()).await;
            }
            continue;
        }

        if parsed.get("type").and_then(|v| v.as_str()) == Some("whiteboard_sync") {
            let sid = parsed.get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !sid.is_empty() {
                current_session_id = sid.clone();
                if let Some(handle) = whiteboard.clone() {
                    if let Err(e) = handle.ensure(&sid).await {
                        tracing::warn!("whiteboard ensure on sync failed: {}", e);
                    }
                    spawn_whiteboard_forwarding(&ws_sender, handle, sid.clone(), &mut whiteboard_forwarding_for);
                }
                let flat = match &whiteboard {
                    Some(handle) => match handle.latest_state(&sid).await {
                        Some(state) => serde_json::json!({
                            "type": "whiteboard_update",
                            "session_id": sid,
                            "state": state,
                        }),
                        None => match handle.latest(&sid).await {
                            Some(board) => serde_json::json!({
                                "type": "whiteboard_update",
                                "session_id": sid,
                                "body_md": board.body_md,
                                "pins": board.pins,
                            }),
                            None => serde_json::json!({
                                "type": "whiteboard_update",
                                "session_id": sid,
                                "body_md": String::new(),
                                "pins": Vec::<String>::new(),
                            }),
                        },
                    },
                    None => serde_json::json!({
                        "type": "whiteboard_update",
                        "session_id": sid,
                        "body_md": String::new(),
                        "pins": Vec::<String>::new(),
                    }),
                };
                let mut sender = ws_sender.lock().await;
                let _ = sender.send(Message::Text(flat.to_string().into())).await;
            }
            continue;
        }

        let session_id = parsed.get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let user_id = parsed.get("user_id")
            .and_then(|v| v.as_str())
            .unwrap_or("anonymous");
        let content = parsed.get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let role = parsed.get("roles")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|r| r.as_str()).collect::<Vec<_>>().join(","))
            .or_else(|| parsed.get("role").and_then(|v| v.as_str()).map(str::to_string))
            .unwrap_or_else(|| "public".to_string());

        let attachments = parsed.get("attachments")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        let filename = a.get("filename").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if filename.is_empty() && a.get("path").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
                            return None;
                        }
                        Some(Attachment {
                            filename,
                            mime_type: a.get("mime_type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            data: Vec::new(),
                            path: a.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        })
                    })
                    .collect::<Vec<Attachment>>()
            })
            .unwrap_or_default();

        let msg = ChannelMessage {
            channel: "web".to_string(),
            session_id: session_id.clone(),
            user_id: user_id.to_string(),
            content: content.to_string(),
            agent_alias: agent.to_string(),
            role: role.to_string(),
            attachments,
        };

        current_session_id = session_id;
        if let Some(handle) = whiteboard.clone() {
            spawn_whiteboard_forwarding(&ws_sender, handle, current_session_id.clone(), &mut whiteboard_forwarding_for);
        }
        let (event_tx, mut event_rx) = mpsc::channel::<StreamEvent>(256);
        let ws = ws_sender.clone();
        let registry = registry.clone();

        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                if let Ok(json) = serde_json::to_string(&event) {
                    let mut sender = ws.lock().await;
                    if sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
        });

        tokio::spawn(async move {
            let _ = registry.route_incoming_stream(msg, event_tx).await;
        });
    }

    let mut sender = ws_sender.lock().await;
    let _ = sender.close().await;
}
