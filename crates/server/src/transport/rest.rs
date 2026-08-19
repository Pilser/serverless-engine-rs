use crate::Server;
use bytes::Bytes;
use engine::events::EventKind;
use engine::model::{Principal, Record};
use engine::{parse_filter, SrvFilter};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

pub type BoxBodyResp = BoxBody<Bytes, std::io::Error>;

struct InFlightGuard(std::sync::Arc<crate::observability::Obs>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.leave();
    }
}

pub fn json_response(status: StatusCode, value: Json) -> Response<BoxBodyResp> {
    let body = Full::new(Bytes::from(value.to_string()))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("access-control-allow-origin", "*")
        .header("access-control-allow-headers", "Authorization, Content-Type, X-Filename, X-Hub-Signature-256, X-Srv-Key")
        .header("access-control-allow-methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS")
        .body(BoxBody::new(body))
        .unwrap()
}

pub fn ok_json(value: Json) -> Response<BoxBodyResp> {
    json_response(StatusCode::OK, value)
}

pub fn with_cors(mut resp: Response<BoxBodyResp>) -> Response<BoxBodyResp> {
    resp.headers_mut().insert("access-control-allow-origin", HeaderValue::from_static("*"));
    resp.headers_mut().insert(
        "access-control-allow-headers",
        HeaderValue::from_static(
            "Authorization, Content-Type, X-Filename, X-Hub-Signature-256, X-Srv-Key",
        ),
    );
    resp.headers_mut().insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
    );
    resp
}

pub fn err_json(status: StatusCode, message: &str) -> Response<BoxBodyResp> {
    json_response(status, json!({ "error": message }))
}

pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    let hex = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn query_params(uri: &http::Uri) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(q) = uri.query() else { return map };
    for pair in q.split('&') {
        let mut parts = pair.splitn(2, '=');
        if let Some(k) = parts.next() {
            let v = parts.next().unwrap_or("");
            map.insert(url_decode(k), url_decode(v));
        }
    }
    map
}

pub fn parse_filter_param(raw: &str) -> anyhow::Result<SrvFilter> {
    let parsed: Json = serde_json::from_str(raw)
        .map_err(|_| anyhow::anyhow!("invalid filter json"))?;
    parse_filter(&parsed)
}

pub fn orders_from(params: &HashMap<String, String>) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    if let Some(o) = params.get("order") {
        let dir = match params.get("dir").map(|d| d.as_str()) {
            Some("asc") => false,
            Some("desc") => true,
            _ => false,
        };
        for p in url_decode(o).split(',') {
            if !p.is_empty() {
                out.push((p.to_string(), dir));
            }
        }
    }
    out
}

pub fn records_json(records: &[Record]) -> Vec<Json> {
    records
        .iter()
        .map(|r| {
            let mut v = json!({ "seq": r.seq, "payload": r.payload, "created_at": r.created_at });
            if let Some(score) = r.score {
                v["score"] = json!(score);
            }
            if let Some(w) = &r.writer {
                v["writer"] = json!(w);
            }
            if let Some(s) = &r.snippet {
                v["snippet"] = json!(s);
            }
            v
        })
        .collect()
}

fn record_owned(r: &Record, scope: &str) -> bool {
    r.payload
        .get("customer_id")
        .and_then(|v| v.as_str())
        .map(|c| c == scope)
        .unwrap_or(false)
}

pub fn require_write(p: &Principal) -> bool {
    match p.role.as_str() {
        "writer" | "admin" | "owner" => true,
        "customer" => p.scope.is_some(),
        _ => false,
    }
}

pub fn require_read(p: &Principal) -> bool {
    match p.role.as_str() {
        "reader" | "list" | "writer" | "admin" | "owner" => true,
        "customer" => p.scope.is_some(),
        _ => false,
    }
}

pub fn require_admin(p: &Principal) -> bool {
    matches!(p.role.as_str(), "admin" | "owner")
}

pub async fn handle(
    server: Arc<Server>,
    req: Request<Incoming>,
) -> Result<Response<BoxBodyResp>, Infallible> {
    server.obs.bump();
    server.obs.enter();
    let _guard = InFlightGuard(server.obs.clone());
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let headers = req.headers().clone();
    let uri = req.uri().clone();

    if method == Method::OPTIONS {
        return Ok(cors_response());
    }

    let mut body_bytes = Bytes::new();
    if matches!(method, Method::POST | Method::PUT | Method::PATCH) {
        body_bytes = match req.collect().await {
            Ok(b) => b.to_bytes(),
            Err(_) => return Ok(err_json(StatusCode::BAD_REQUEST, "invalid request")),
        };
    }

    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let params = query_params(&uri);

    if method == Method::GET
        && segs.len() >= 2
        && segs[0] == "srv"
        && !(segs.len() >= 3 && segs[1] == "api" && segs[2] == "srv")
    {
        let board = segs[1];
        let rel = if segs.len() > 2 { segs[2..].join("/") } else { String::new() };
        // SPA hosting: relative asset paths (base "./") in index.html only
        // resolve correctly when the page URL ends in "/". A bare board root
        // like /srv/{board} would resolve ./assets/... against /srv/ (dropping
        // the board id), so redirect to the trailing-slash form.
        if rel.is_empty() && !path.ends_with('/') {
            return Ok(with_cors(
                Response::builder()
                    .status(StatusCode::PERMANENT_REDIRECT)
                    .header("location", format!("{}/", &path))
                    .body(BoxBody::new(
                        Full::new(Bytes::new())
                            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "redirect")),
                    ))
                    .unwrap(),
            ));
        }
        // The engine + helixdb backend use a blocking reqwest client. Running
        // it on a tokio async worker stalls after ~128 requests, so static
        // site serving (which locks the engine) runs on the blocking pool.
        let server2 = server.clone();
        let board2 = board.to_string();
        let rel2 = rel.clone();
        let headers2 = headers.clone();
        let static_resp =
            tokio::task::spawn_blocking(move || static_site(&server2, &board2, &rel2, &principal_of(&server2, &board2, &headers2, None))).await;
        if let Ok(Some(resp)) = static_resp {
            return Ok(resp);
        }
    }

    // Host health for the frontend's "School VPS Node" widget (no board scope).
    // GET /api/system/health -> {cpuLoad, ramUsage, storageFree, dbStatus, lanAccess}
    if method == Method::GET && segs.len() == 3 && segs[0] == "api" && segs[1] == "system" && segs[2] == "health" {
        let h = system_health();
        let body = Full::new(Bytes::from(h))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
        return Ok(with_cors(
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .header("cache-control", "no-cache")
                .body(BoxBody::new(body))
                .unwrap(),
        ));
    }

    // Same blocking-client constraint: dispatch the whole route (engine lock +
    // blocking Helix/HTTP) to the blocking thread pool.
    let server2 = server.clone();
    let segs_owned: Vec<String> = segs.iter().map(|s| s.to_string()).collect();
    let tail_owned: Vec<String> = segs_owned.iter().skip(3).cloned().collect();
    // SSE events is the only async branch (streaming); handle it here.
    if method == Method::GET
        && (tail_owned.len() == 1 && tail_owned[0] == "events"
            || tail_owned.len() == 2 && tail_owned[0] == "events" && tail_owned[1] == "stream")
    {
        let after: i64 = params.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
        let board = &segs_owned[2];
        let sse = super::sse::sse_response(
            server.engine.clone(),
            server.broker.clone(),
            board.to_string(),
            after,
        );
        let body = sse.map(|s| http_body_util::combinators::BoxBody::new(s));
        return Ok(body);
    }
    let resp = match tokio::task::spawn_blocking(move || {
        route_blocking(&server2, &method, &segs_owned, &headers, &body_bytes, &params)
    })
    .await
    {
        Ok(resp) => resp,
        Err(_) => err_json(StatusCode::INTERNAL_SERVER_ERROR, "handler panicked"),
    };
    Ok(resp)
}

fn has_file_ext(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some(dot) = name.rfind('.') else {
        return false;
    };
    // Ignore dots in the leading position (hidden files) and require a real ext.
    dot > 0 && dot < name.len() - 1
}

fn static_site(
    server: &Server,
    board: &str,
    rel: &str,
    principal: &Principal,
) -> Option<Response<BoxBodyResp>> {
    let app = server.engine.lock().unwrap().get_app(board).ok().flatten();
    let app = app?;
    if !app.public_reads && !require_read(principal) {
        return Some(err_json(StatusCode::FORBIDDEN, "private board"));
    }
    let engine = server.engine.lock().unwrap();
    let mut target = rel.to_string();
    if rel.is_empty() {
        target = "index.html".to_string();
    }
    let mut fetched = engine.get_asset(board, &target).ok().flatten().or_else(|| {
        if !target.ends_with(".html") {
            engine.get_asset(board, &format!("{target}/index.html")).ok().flatten()
        } else {
            None
        }
    });
    // SPA fallback: an unknown path with no file extension (a client-side route)
    // serves index.html so React Router can handle it.
    if fetched.is_none() && !has_file_ext(&target) {
        fetched = engine.get_asset(board, "index.html").ok().flatten();
    }
    let Some((mut data, content_type)) = fetched else {
        return Some(err_json(StatusCode::NOT_FOUND, "not found"));
    };
    // Deep-link refreshes (e.g. /srv/<board>/director/letters) serve the root
    // index.html, but its relative "./assets/..." paths would then resolve
    // against the deep-link directory and 404. Anchor relative URLs to the
    // board root so every asset loads regardless of route depth.
    if content_type.starts_with("text/html") {
        let base = format!("<base href=\"/srv/{board}/\">");
        data = inject_base_tag(&data, &base).into_bytes();
    }
    let body = Full::new(Bytes::from(data))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
    // HTML is always revalidated (so a new build's chunk hashes are picked up);
    // hashed assets (js/css) keep the long cache.
    let cache = if content_type.starts_with("text/html") {
        "no-cache"
    } else {
        "public, max-age=3600"
    };
    Some(
        with_cors(
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", content_type)
                .header("cache-control", cache)
                .body(BoxBody::new(body))
                .unwrap(),
        ),
    )
}

fn inject_base_tag(html: &[u8], base_tag: &str) -> String {
    let src = String::from_utf8_lossy(html);
    if let Some(hi) = src.to_ascii_lowercase().find("<head") {
        if let Some(end) = src[hi..].find('>') {
            let pos = hi + end + 1;
            return format!("{}{}{}", &src[..pos], base_tag, &src[pos..]);
        }
    }
    format!("<head>{}</head>{}", base_tag, src)
}

fn cors_response() -> Response<BoxBodyResp> {
    json_response(StatusCode::NO_CONTENT, Json::Null)
}

pub fn rate_action(
    server: &Server,
    board_id: &str,
    principal: &Principal,
    action: &str,
) -> anyhow::Result<()> {
    let key = format!("{}:{action}", principal.id);
    let limits = server.rate_limits_for(&board_of(server, board_id));
    let mut limiter = server.limiter.lock().unwrap();
    limiter.check(&key, action, &limits)
}

pub fn board_of(server: &Server, board_id: &str) -> engine::Board {
    server
        .engine
        .lock()
        .unwrap()
        .get_app(board_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| engine::Board {
            board_id: board_id.to_string(),
            owner_key: String::new(),
            title: String::new(),
            schema_json: None,
            public_reads: false,
            unique_key: None,
            computed_json: None,
            validate_json: None,
            redact_json: None,
            rate_json: None,
            ttl_seconds: None,
            ttl_field: None,
            audit: false,
            webhook_secret: None,
            created_at: None,
        })
}

fn principal_of(server: &Server, board_id: &str, headers: &http::HeaderMap, scope: Option<&str>) -> Principal {
    server
        .engine
        .lock()
        .unwrap()
        .resolve_principal(board_id, crate::identity::keys::token(headers).as_deref(), scope)
        .unwrap_or_else(|_| Principal {
            id: "anon".to_string(),
            role: "none".to_string(),
            scope: None,
            writer: None,
        })
}

pub use super::rest_admin::handle_admin;

/// Blocking route dispatch. Runs on the tokio blocking thread pool (see
/// `handle`): the engine + helixdb backend use a blocking reqwest client that
/// must not run on an async worker. SSE streaming is handled in `handle`
/// before this is called.
fn route_blocking(
    server: &Server,
    method: &Method,
    segs: &[String],
    headers: &http::HeaderMap,
    body: &Bytes,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if segs.first().map(|s| s.as_str()) == Some("healthz") && *method == Method::GET {
        return ok_json(json!({ "ok": true }));
    }
    if segs.len() < 3 || segs[0] != "api" || segs[1] != "srv" {
        return err_json(StatusCode::NOT_FOUND, "not found");
    }
    let board = segs[2].as_str();
    let tail: Vec<&str> = segs[3..].iter().map(|s| s.as_str()).collect();
    let tail = tail.as_slice();

    let scope = params.get("scope").map(|s| s.as_str());
    let principal = principal_of(server, board, headers, scope);

    if let Some(resp) = super::rest_admin::handle_admin(
        server,
        method,
        board,
        tail,
        headers,
        body,
        params,
        &principal,
    ) {
        return resp;
    }

    if tail.first() == Some(&"tables") {
        return super::rest_tables::handle_tables(
            server, board, method, &tail[1..], body, params, &principal,
        );
    }

    match (method.as_str(), tail) {
        ("GET", []) => app_info(server, board, &principal),
        ("DELETE", []) => app_delete(server, board, &principal),
        ("PATCH", ["app"]) => app_patch(server, board, &principal, body),
        ("GET", ["resources"]) => crate::resources::resources(server, board, &principal),
        ("POST", ["auth", "signup"]) => crate::transport::rest_auth::signup(server, board, &principal, body),
        ("POST", ["auth", "login"]) => crate::transport::rest_auth::login(server, board, body),
        ("POST", ["auth", "logout"]) => crate::transport::rest_auth::logout(server, board, body),
        ("POST", ["auth", "role"]) => crate::transport::rest_auth::set_role(server, board, &principal, body),
        ("GET", ["auth", "me"]) => {
            crate::transport::rest_auth::me(server, board, crate::identity::keys::token(headers).as_deref())
        }
        ("POST", ["upload"]) => upload(server, board, &principal, body, headers, params),
        ("GET", ["file"]) => file(server, board, &principal, params),
        ("POST", ["call"]) => call(server, board, &principal, body),
        ("POST", ["events"]) => events_inbound(server, board, &principal, body, headers),
        ("PUT", ["assets", rest @ ..]) => asset_put(server, board, &principal, &rest.join("/"), body),
        ("GET", ["assets"]) => asset_list(server, board, &principal),
        ("GET", ["assets", rest @ ..]) => asset_get(server, board, &principal, &rest.join("/")),
        ("DELETE", ["assets", rest @ ..]) => asset_delete(server, board, &principal, &rest.join("/")),
        _ => err_json(StatusCode::NOT_FOUND, "not found"),
    }
}

fn asset_put(
    server: &Server,
    board: &str,
    principal: &Principal,
    rel: &str,
    body: &Bytes,
) -> Response<BoxBodyResp> {
    if !require_write(principal) {
        return err_json(StatusCode::FORBIDDEN, "writer authorization required");
    }
    let exists = server.engine.lock().unwrap().get_app(board).ok().flatten().is_some();
    if !exists {
        return err_json(StatusCode::NOT_FOUND, "not found");
    }
    if body.is_empty() {
        return err_json(StatusCode::BAD_REQUEST, "empty asset");
    }
    match server.engine.lock().unwrap().put_asset(board, rel, body) {
        Ok(()) => ok_json(json!({ "ok": true, "asset": rel })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

fn asset_get(
    server: &Server,
    board: &str,
    principal: &Principal,
    rel: &str,
) -> Response<BoxBodyResp> {
    let app = server.engine.lock().unwrap().get_app(board).ok().flatten();
    let Some(app) = app else {
        return err_json(StatusCode::NOT_FOUND, "not found");
    };
    if !app.public_reads && !require_read(principal) {
        return err_json(StatusCode::FORBIDDEN, "private board");
    }
    let engine = server.engine.lock().unwrap();
    match engine.get_asset(board, rel) {
        Ok(Some((data, content_type))) => {
            let body = Full::new(Bytes::from(data))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
            with_cors(
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", content_type)
                    .header("cache-control", "public, max-age=3600")
                    .body(BoxBody::new(body))
                    .unwrap(),
            )
        }
        Ok(None) => err_json(StatusCode::NOT_FOUND, "not found"),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn asset_delete(
    server: &Server,
    board: &str,
    principal: &Principal,
    rel: &str,
) -> Response<BoxBodyResp> {
    if !require_write(principal) {
        return err_json(StatusCode::FORBIDDEN, "writer authorization required");
    }
    let exists = server.engine.lock().unwrap().get_app(board).ok().flatten().is_some();
    if !exists {
        return err_json(StatusCode::NOT_FOUND, "not found");
    }
    match server.engine.lock().unwrap().delete_asset(board, rel) {
        Ok(true) => ok_json(json!({ "ok": true, "asset": rel })),
        Ok(false) => err_json(StatusCode::NOT_FOUND, "not found"),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

fn asset_list(
    server: &Server,
    board: &str,
    principal: &Principal,
) -> Response<BoxBodyResp> {
    if !require_read(principal) {
        return err_json(StatusCode::FORBIDDEN, "reader authorization required");
    }
    let exists = server.engine.lock().unwrap().get_app(board).ok().flatten().is_some();
    if !exists {
        return err_json(StatusCode::NOT_FOUND, "not found");
    }
    let engine = server.engine.lock().unwrap();
    match engine.list_assets(board) {
        Ok(keys) => {
            let list: Vec<Json> = keys
                .into_iter()
                .map(|k| json!({ "asset": k.key, "size": k.size }))
                .collect();
            ok_json(json!({ "ok": true, "assets": list }))
        }
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn app_info(server: &Server, board: &str, principal: &Principal) -> Response<BoxBodyResp> {
    let app = server.engine.lock().unwrap().get_app(board).ok().flatten();
    let Some(app) = app else {
        return err_json(StatusCode::NOT_FOUND, "not found");
    };
    if !app.public_reads && !require_read(principal) {
        return err_json(StatusCode::FORBIDDEN, "private board");
    }
    ok_json(json!({ "ok": true, "app": app }))
}

fn app_delete(server: &Server, board: &str, principal: &Principal) -> Response<BoxBodyResp> {
    if !require_admin(principal) {
        return err_json(StatusCode::FORBIDDEN, "admin authorization required");
    }
    match server.engine.lock().unwrap().delete_app(board, &principal.id) {
        Ok(()) => ok_json(json!({ "ok": true })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

fn app_patch(server: &Server, board: &str, principal: &Principal, body: &Bytes) -> Response<BoxBodyResp> {
    if !require_admin(principal) {
        return err_json(StatusCode::FORBIDDEN, "admin authorization required");
    }
    let req: Json = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return err_json(StatusCode::BAD_REQUEST, "invalid json"),
    };
    match server.engine.lock().unwrap().update_app(board, &principal.id, &req) {
        Ok(()) => ok_json(json!({ "ok": true })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

fn upload(
    server: &Server,
    board: &str,
    principal: &Principal,
    body: &Bytes,
    headers: &http::HeaderMap,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if let Err(e) = rate_action(server, board, principal, "upload") {
        return err_json(StatusCode::TOO_MANY_REQUESTS, &e.to_string());
    }
    let exists = server.engine.lock().unwrap().get_app(board).ok().flatten().is_some();
    if !exists {
        return err_json(StatusCode::NOT_FOUND, "not found");
    }
    if !require_write(principal) {
        return err_json(StatusCode::FORBIDDEN, "writer authorization required");
    }
    if body.is_empty() {
        return err_json(StatusCode::BAD_REQUEST, "empty file");
    }
    let table = params.get("table").map(|t| t.as_str()).unwrap_or("files").to_string();
    let content_type = headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let filename = headers
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();
    let meta = json!({ "name": filename, "type": content_type });
    let uploaded = {
        let mut engine = server.engine.lock().unwrap();
        engine.upload(board, &table, &filename, &content_type, body, &meta)
    };
    match uploaded {
        Ok(seq) => ok_json(json!({ "ok": true, "seq": seq, "table": table })),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn file(
    server: &Server,
    board: &str,
    principal: &Principal,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    let file = match params.get("file") {
        Some(f) if !f.is_empty() => f.clone(),
        _ => return err_json(StatusCode::BAD_REQUEST, "invalid file"),
    };
    let app = server.engine.lock().unwrap().get_app(board).ok().flatten();
    let Some(app) = app else {
        return err_json(StatusCode::NOT_FOUND, "not found");
    };
    if !app.public_reads && !require_read(principal) {
        return err_json(StatusCode::FORBIDDEN, "private board");
    }
    let table = params.get("table").map(|t| t.as_str()).unwrap_or("files").to_string();
    // Resolve the blob key: `files/...` is used as-is (board-prefixed); a bare
    // path is looked up in the table's records by `name` (the upload stores
    // `name` = the original path, `file` = the blob key).
    let full = if file.starts_with("files/") {
        format!("{board}/{file}")
    } else {
        let engine = server.engine.lock().unwrap();
        let sf = engine::storage::ir::SrvFilter {
            conds: vec![engine::storage::ir::FilterCond {
                field: "$.name".to_string(),
                op: engine::storage::ir::Op::Eq,
                value: serde_json::Value::String(file.clone()),
            }],
        };
        let found = engine
            .query_records(board, &table, &sf, &[], 1, 0)
            .ok()
            .and_then(|rs| rs.into_iter().next())
            .and_then(|r| r.payload.get("file").and_then(|v| v.as_str()).map(String::from));
        match found {
            Some(blob) => blob,
            None => file.clone(),
        }
    };
    let engine = server.engine.lock().unwrap();
    match engine.download(board, &table, &full) {
        Ok(Some((data, content_type))) => {
            let body = Full::new(Bytes::from(data))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", content_type)
                .body(BoxBody::new(body))
                .unwrap()
        }
        Ok(None) => err_json(StatusCode::NOT_FOUND, "not found"),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn call(
    server: &Server,
    board: &str,
    principal: &Principal,
    body: &Bytes,
) -> Response<BoxBodyResp> {
    if !require_admin(principal) {
        return err_json(StatusCode::FORBIDDEN, "admin authorization required");
    }
    let req: Json = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return err_json(StatusCode::BAD_REQUEST, "invalid json"),
    };
    let url = match req.get("url").and_then(|u| u.as_str()) {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => return err_json(StatusCode::BAD_REQUEST, "missing url"),
    };
    let mut headers: Vec<(String, String)> = match req.get("headers").and_then(|h| h.as_object()) {
        Some(map) => map
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
        None => Vec::new(),
    };
    let secrets = server
        .engine
        .lock()
        .unwrap()
        .secrets_map(board)
        .unwrap_or_default();
    let url = engine::automation::resolve_secret_placeholders(&secrets, &url);
    for (_, v) in headers.iter_mut() {
        *v = engine::automation::resolve_secret_placeholders(&secrets, v);
    }
    let mut call_body = req.get("body").cloned().unwrap_or(Json::Null);
    engine::automation::subst_secret_json(&secrets, &mut call_body);
    match engine::automation::srv_http_call(&url, &headers, &call_body) {
        Ok(res) => ok_json(json!({ "ok": true, "status": 200, "body": res })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

fn events_inbound(
    server: &Server,
    board: &str,
    _principal: &Principal,
    body: &Bytes,
    headers: &http::HeaderMap,
) -> Response<BoxBodyResp> {
    let payload: Json = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return err_json(StatusCode::BAD_REQUEST, "invalid json"),
    };
    let mut engine = server.engine.lock().unwrap();
    let secret = engine.get_app(board).ok().flatten().and_then(|b| b.webhook_secret);
    if let Some(secret) = secret {
        let provided = headers
            .get("X-Hub-Signature-256")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.strip_prefix("sha256=").unwrap_or(s).to_string())
            .unwrap_or_default();
        let expected = engine::webhooks::hmac_hex(&String::from_utf8_lossy(body), &secret);
        if !provided.eq_ignore_ascii_case(&expected) {
            return err_json(StatusCode::UNAUTHORIZED, "invalid signature");
        }
    }
    let _ = engine.dispatch_recipes(board, "inbound", EventKind::Inbound, None, Some(payload));
    ok_json(json!({ "ok": true }))
}

/// Host-level health for the frontend "School VPS Node" widget. Reads the
/// daemon's host (/proc) plus local disk, no engine lock needed.
fn system_health() -> String {
    let (cpu, ram_gb, _total_ram_gb) = host_cpu_ram();
    let storage_free = disk_free_gb("/");
    let body = serde_json::json!({
        "cpuLoad": format!("{cpu:.1}"),
        "ramUsage": format!("{ram_gb:.2}"),
        "storageFree": format!("{storage_free:.1}"),
        "dbStatus": "Connected",
        "lanAccess": "Active",
    });
    body.to_string()
}

fn host_cpu_ram() -> (f64, f64, f64) {
    // CPU: 1 - idle/total from /proc/stat over a short sample.
    fn proc_cpu() -> Option<(u64, u64)> {
        let s = std::fs::read_to_string("/proc/stat").ok()?;
        let line = s.lines().find(|l| l.starts_with("cpu "))?;
        let parts: Vec<u64> = line.split_whitespace().skip(1).filter_map(|p| p.parse().ok()).collect();
        if parts.len() < 4 {
            return None;
        }
        let idle = parts.get(3).copied().unwrap_or(0) + parts.get(4).copied().unwrap_or(0);
        let total: u64 = parts.iter().sum();
        Some((idle, total))
    }
    let cpu = match (proc_cpu(), proc_cpu()) {
        (Some((i1, t1)), Some((i2, t2))) if t2 > t1 => {
            let idle = i2.saturating_sub(i1) as f64;
            let total = (t2 - t1) as f64;
            ((1.0 - idle / total) * 100.0).max(0.0).min(100.0)
        }
        _ => 0.0,
    };
    // RAM from /proc/meminfo (MemTotal, MemAvailable) in GB.
    let mut total = 0u64;
    let mut available = 0u64;
    if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                total = rest.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
                available = rest.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
        }
    }
    let total_gb = total as f64 / (1024.0 * 1024.0);
    let used_gb = (total.saturating_sub(available)) as f64 / (1024.0 * 1024.0);
    (cpu, used_gb, total_gb)
}

fn disk_free_gb(path: &str) -> f64 {
    // `df -P -B1 <path>` -> free bytes on the filesystem, cheap once per call.
    if let Ok(out) = std::process::Command::new("df").args(["-P", "-B1", path]).output() {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(line) = text.lines().nth(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 4 {
                if let Ok(avail) = fields[3].parse::<u64>() {
                    return avail as f64 / (1024.0 * 1024.0 * 1024.0);
                }
            }
        }
    }
    0.0
}