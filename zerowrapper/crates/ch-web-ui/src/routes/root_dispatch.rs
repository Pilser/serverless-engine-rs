use std::sync::Arc;

use topcoat::Result;
use topcoat::context::{Cx, CxBuilder, app_context};
use topcoat::router::{
    Body, FromRequest, Layer, LayerFuture, Next, Response, StatusCode, headers, to_bytes, uri,
};
use topcoat::router::content::websocket::{WebSocket, WebSocketUpgrade};

use crate::context::AppContext;
use crate::ws::handler::handle_ws_connection;

/// Root-only dispatch layer: lets an embedded frontend that may only fetch `/`
/// reach the API, WebSocket, file/media and asset endpoints via query params
/// on the root URL instead of sub-paths. Registered at `/` so it wraps every
/// route; requests that are not root-dispatched pass through to the normal
/// page pipeline (which renders the chat app).
///
///   GET  /?api=history&user_id=...        -> history
///   POST /?api=upload                     -> upload
///   GET  /?ws=1                           -> WebSocket upgrade
///   GET  /?file={hash}                    -> cached file/media
///   GET  /?asset={bundled-filename}       -> bundled asset (JS/CSS)
///   GET  /                                -> root HTML (chat, via next)
#[derive(Debug, Clone, Default)]
pub struct RootDispatchLayer;

fn query_value(cx: &Cx, key: &str) -> Option<String> {
    let query = uri(cx).query().unwrap_or("");
    query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let k = parts.next()?;
            let v = parts.next().unwrap_or("");
            Some((k.to_string(), v.to_string()))
        })
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

fn url_decode(s: &str) -> String {
    s.replace("%20", " ")
        .replace("%3A", ":")
        .replace("%2F", "/")
        .replace("%3F", "?")
        .replace("%3D", "=")
        .replace("%26", "&")
        .replace("%40", "@")
}

fn sanitize_key(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '@' { c } else { '_' })
        .collect()
}

impl Layer for RootDispatchLayer {
    fn path(&self) -> &topcoat::router::Path {
        topcoat::router::Path::new("/")
    }

    fn handle<'a>(&'a self, cx: &'a mut CxBuilder, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        Box::pin(async move {
            // WebSocket upgrade: browser connects to `/?ws=1`.
            if query_value(cx, "ws").is_some() {
                return root_ws(cx, body).await;
            }

            // Bundled asset: `/?asset=app-<hash>.js`
            if let Some(name) = query_value(cx, "asset") {
                return serve_asset(&name).await;
            }

            // Cached file/media: `/?file=<hash>`
            if let Some(hash) = query_value(cx, "file") {
                return serve_file(cx, &hash).await;
            }

            // API: `/?api=history&user_id=...` or `/?api=upload`
            if let Some(api) = query_value(cx, "api") {
                return dispatch_api(cx, &api, body).await;
            }

            // Everything else (including unknown sub-paths) falls through to
            // the normal page pipeline, which renders the chat app at `/`.
            next.run(cx, body).await
        })
    }
}

async fn root_ws(cx: &Cx, body: Body) -> Result<Response> {
    let app: &AppContext = app_context(cx);
    let registry = app.registry.clone();
    let zeroclaw = app.zeroclaw.clone();
    let agent = Arc::new(app.agent.clone());
    let whiteboard = app.whiteboard().cloned();

    let upgrade = WebSocketUpgrade::from_request(cx, body).await?;
    upgrade.on_upgrade(move |socket: WebSocket| async move {
        handle_ws_connection(socket, registry, zeroclaw, agent, whiteboard).await;
    })
}

async fn dispatch_api(cx: &Cx, api: &str, body: Body) -> Result<Response> {
    match api {
        "history" => api_history(cx).await,
        "upload" => api_upload(cx, body).await,
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::json!({"error": "unknown api"}).to_string()))
            .unwrap()),
    }
}

/// Mirrors `api/history.rs` but keyed from the root query params.
async fn api_history(cx: &Cx) -> Result<Response> {
    let app: &AppContext = app_context(cx);
    let user_id = query_value(cx, "user_id").map(|v| url_decode(&v)).unwrap_or_default();
    let _role = query_value(cx, "role").map(|v| url_decode(&v)).unwrap_or_else(|| "public".to_string());

    if user_id.is_empty() {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::json!({"error": "Missing user_id"}).to_string()))
            .unwrap());
    }

    let session_id = format!("web.{}.{}", app.agent, sanitize_key(&user_id));

    match app.zeroclaw.get_session_messages(session_id.clone(), Some(100)).await {
        Ok(result) => Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&result).unwrap_or_default()))
            .unwrap()),
        Err(e) => {
            tracing::warn!("history: zeroclaw error: {}", e);
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "session_id": session_id,
                        "messages": [],
                        "total": 0,
                        "start": 0
                    })
                    .to_string(),
                ))
                .unwrap())
        }
    }
}

/// Mirrors `api/upload.rs` but keyed from the root query params.
async fn api_upload(cx: &Cx, body: Body) -> Result<Response> {
    let app: &AppContext = app_context(cx);
    let filename = headers(cx)
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();

    let body_bytes = match to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"error": format!("Failed to read body: {}", e)}).to_string(),
                ))
                .unwrap());
        }
    };

    if body_bytes.is_empty() {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::json!({"error": "Empty file"}).to_string()))
            .unwrap());
    }

    tokio::fs::create_dir_all(&app.upload_dir).await.unwrap_or_default();

    let now = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let safe_name: String = filename
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let ext = std::path::Path::new(&safe_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let stem = std::path::Path::new(&safe_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let dest_name = format!("{}_{}.{}", stem, now, ext);
    let dest_path = app.upload_dir.join(&dest_name);

    tracing::info!("Saving uploaded file to {:?}", dest_path);

    if let Err(e) = tokio::fs::write(&dest_path, &body_bytes).await {
        return Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": format!("Failed to save file: {}", e)}).to_string(),
            ))
            .unwrap());
    }

    let abs_path = std::fs::canonicalize(&dest_path)
        .unwrap_or_else(|_| dest_path.clone());

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "filename": safe_name,
                "path": abs_path.to_string_lossy().to_string(),
                "size": body_bytes.len(),
                "mime_type": mime_guess::from_path(&safe_name).first_or_octet_stream().to_string()
            })
            .to_string(),
        ))
        .unwrap())
}

async fn serve_asset(name: &str) -> Result<Response> {
    let assets_dir = std::env::var("TOPCOAT_ASSETS_DIR").unwrap_or_else(|_| "assets".to_string());
    let path = std::path::Path::new(&assets_dir).join(name);
    let mime = if name.ends_with(".js") {
        "text/javascript"
    } else if name.ends_with(".css") {
        "text/css"
    } else {
        "application/octet-stream"
    };
    match tokio::fs::read(&path).await {
        Ok(data) => Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", mime)
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .body(Body::from(data))
            .unwrap()),
        Err(_) => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not found"))
            .unwrap()),
    }
}

async fn serve_file(cx: &Cx, hash: &str) -> Result<Response> {
    let app: &AppContext = app_context(cx);
    let meta = match app.cache.get_meta(hash) {
        Ok(Some(m)) => m,
        _ => {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("File not found"))
                .unwrap());
        }
    };
    match tokio::fs::read(&meta.original_path).await {
        Ok(data) => Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", &meta.mime_type)
            .header("Cache-Control", "public, max-age=3600")
            .body(Body::from(data))
            .unwrap()),
        Err(_) => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("File not found"))
            .unwrap()),
    }
}
