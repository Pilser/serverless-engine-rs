use crate::Server;
use bytes::Bytes;
use engine::events::EventKind;
use engine::model::{Principal, Record};
use engine::{parse_filter, Agg, FilterCond, Op, SrvFilter};
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

fn orders_from(params: &HashMap<String, String>) -> Vec<(String, bool)> {
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

fn records_json(records: &[Record]) -> Vec<Json> {
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
        let principal = principal_of(&server, board, &headers, None);
        if let Some(resp) = static_site(&server, board, &rel, &principal) {
            return Ok(resp);
        }
    }

    let resp = route(&server, &method, &segs, &headers, &body_bytes, &params).await;
    Ok(resp)
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
    let fetched = engine.get_asset(board, &target).ok().flatten().or_else(|| {
        if !target.ends_with(".html") {
            engine.get_asset(board, &format!("{target}/index.html")).ok().flatten()
        } else {
            None
        }
    });
    let Some((data, content_type)) = fetched else {
        return Some(err_json(StatusCode::NOT_FOUND, "not found"));
    };
    let body = Full::new(Bytes::from(data))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
    Some(
        with_cors(
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", content_type)
                .header("cache-control", "public, max-age=3600")
                .body(BoxBody::new(body))
                .unwrap(),
        ),
    )
}

fn cors_response() -> Response<BoxBodyResp> {
    json_response(StatusCode::NO_CONTENT, Json::Null)
}

fn rate_action(
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

async fn route(
    server: &Server,
    method: &Method,
    segs: &[&str],
    headers: &http::HeaderMap,
    body: &Bytes,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if segs.first() == Some(&"healthz") && *method == Method::GET {
        return ok_json(json!({ "ok": true }));
    }
    if segs.len() < 3 || segs[0] != "api" || segs[1] != "srv" {
        return err_json(StatusCode::NOT_FOUND, "not found");
    }
    let board = segs[2];
    let tail = &segs[3..];

    if *method == Method::GET && matches!(tail, ["events"] | ["events", "stream"]) {
        let after: i64 = params.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
        let sse = super::sse::sse_response(
            server.engine.clone(),
            server.broker.clone(),
            board.to_string(),
            after,
        );
        let body = sse.map(|s| http_body_util::combinators::BoxBody::new(s));
        return body;
    }

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

    match (method.as_str(), tail) {
        ("GET", []) => app_info(server, board, &principal),
        ("DELETE", []) => app_delete(server, board, &principal),
        ("PATCH", ["app"]) => app_patch(server, board, &principal, body),
        ("POST", ["submit"]) => submit(server, board, &principal, body, params),
        ("GET", ["records"]) => records(server, board, &principal, params),
        ("DELETE", ["records"]) => records_delete(server, board, &principal, params),
        ("GET", ["record"]) => record(server, board, &principal, params),
        ("GET", ["records", "after"]) => records_after(server, board, &principal, params),
        ("PUT", ["records", seq]) => record_put(server, board, &principal, seq, body, params),
        ("PATCH", ["records", seq]) => record_patch(server, board, &principal, seq, body, params),
        ("DELETE", ["records", seq]) => record_delete(server, board, &principal, seq, params),
        ("POST", ["bulk"]) => bulk(server, board, &principal, body, params),
        ("GET", ["aggregate"]) => aggregate(server, board, &principal, params),
        ("GET", ["resources"]) => crate::resources::resources(server, board, &principal),
        ("POST", ["auth", "signup"]) => crate::transport::rest_auth::signup(server, board, &principal, body),
        ("POST", ["auth", "login"]) => crate::transport::rest_auth::login(server, board, body),
        ("POST", ["auth", "logout"]) => crate::transport::rest_auth::logout(server, board, body),
        ("GET", ["auth", "me"]) => {
            crate::transport::rest_auth::me(server, board, crate::identity::keys::token(headers).as_deref())
        }
        ("GET", ["join"]) => join(server, board, &principal, params),
        ("POST", ["upload"]) => upload(server, board, &principal, body, headers),
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

fn submit(
    server: &Server,
    board: &str,
    principal: &Principal,
    body: &Bytes,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if let Err(e) = rate_action(server, board, principal, "submit") {
        return err_json(StatusCode::TOO_MANY_REQUESTS, &e.to_string());
    }
    let exists = server.engine.lock().unwrap().get_app(board).ok().flatten().is_some();
    if !exists {
        return err_json(StatusCode::NOT_FOUND, "not found");
    }
    if !require_write(principal) {
        return err_json(StatusCode::FORBIDDEN, "writer authorization required");
    }
    let payload: Json = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(_) => return err_json(StatusCode::BAD_REQUEST, "invalid json"),
    };
    let upsert = params
        .get("upsert")
        .map(|v| matches!(v.as_str(), "1" | "true"))
        .unwrap_or(false);
    let writer = principal.writer.clone();
    let inserted = {
        let mut engine = server.engine.lock().unwrap();
        engine.insert_record(board, payload.clone(), writer.as_deref(), upsert, principal)
    };
    match inserted {
        Ok(seq) => ok_json(json!({ "ok": true, "seq": seq })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

fn records(
    server: &Server,
    board: &str,
    principal: &Principal,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if let Err(e) = rate_action(server, board, principal, if params.contains_key("q") { "search" } else { "read" }) {
        return err_json(StatusCode::TOO_MANY_REQUESTS, &e.to_string());
    }
    let app = server.engine.lock().unwrap().get_app(board).ok().flatten();
    let Some(app) = app else {
        return err_json(StatusCode::NOT_FOUND, "not found");
    };
    if !app.public_reads && !require_read(principal) {
        return err_json(StatusCode::FORBIDDEN, "private board");
    }
    let limit: usize = params
        .get("limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let offset: usize = params.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    let orders = orders_from(params);
    let mut extra_conds: Vec<FilterCond> = Vec::new();
    if let Some(s) = &principal.scope {
        extra_conds.push(FilterCond {
            field: "$.customer_id".to_string(),
            op: Op::Eq,
            value: Json::String(s.clone()),
        });
    }

    let engine = server.engine.lock().unwrap();

    if let Some(q) = params.get("q").map(|q| url_decode(q)).filter(|q| !q.is_empty()) {
        let mut conds = match params.get("filter").map(|f| url_decode(f)) {
            Some(f) if !f.is_empty() => match parse_filter_param(&f) {
                Ok(sf) => sf.conds,
                Err(e) => return err_json(StatusCode::BAD_REQUEST, &e.to_string()),
            },
            _ => Vec::new(),
        };
        conds.extend(extra_conds);
        let snippet = params
            .get("hl")
            .map(|v| matches!(v.as_str(), "1" | "on" | "true"))
            .unwrap_or(false);
        let sf = SrvFilter { conds };
        match engine.search_records(board, q.as_str(), &sf, limit, offset, snippet) {
            Ok(rs) => ok_json(json!({ "ok": true, "records": records_json(&rs) })),
            Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        }
    } else {
        let filter = match params.get("filter").map(|f| url_decode(f)) {
            Some(f) if !f.is_empty() => match parse_filter_param(&f) {
                Ok(sf) => sf,
                Err(e) => return err_json(StatusCode::BAD_REQUEST, &e.to_string()),
            },
            _ => SrvFilter::new(),
        };
        let mut conds = filter.conds;
        conds.extend(extra_conds);
        let sf = SrvFilter { conds };
        match engine.query_records(board, &sf, &orders, limit, offset) {
            Ok(rs) => ok_json(json!({ "ok": true, "records": records_json(&rs) })),
            Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        }
    }
}

fn records_delete(
    server: &Server,
    board: &str,
    principal: &Principal,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if !require_write(principal) {
        return err_json(StatusCode::FORBIDDEN, "writer authorization required");
    }
    let filter = match params.get("filter").map(|f| url_decode(f)) {
        Some(f) if !f.is_empty() => match parse_filter_param(&f) {
            Ok(sf) => sf,
            Err(e) => return err_json(StatusCode::BAD_REQUEST, &e.to_string()),
        },
        _ => return err_json(StatusCode::BAD_REQUEST, "expected filter query param"),
    };
    match server.engine.lock().unwrap().delete_records(board, &filter) {
        Ok(deleted) => ok_json(json!({ "ok": true, "deleted": deleted })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

fn record(
    server: &Server,
    board: &str,
    principal: &Principal,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if let Err(e) = rate_action(server, board, principal, "read") {
        return err_json(StatusCode::TOO_MANY_REQUESTS, &e.to_string());
    }
    let seq: i64 = match params.get("seq").and_then(|s| s.parse().ok()) {
        Some(s) => s,
        None => return err_json(StatusCode::BAD_REQUEST, "missing or invalid seq"),
    };
    let app = server.engine.lock().unwrap().get_app(board).ok().flatten();
    let Some(app) = app else {
        return err_json(StatusCode::NOT_FOUND, "not found");
    };
    if !app.public_reads && !require_read(principal) {
        return err_json(StatusCode::FORBIDDEN, "private board");
    }
    match server.engine.lock().unwrap().get_record(board, seq) {
        Ok(Some(record)) => {
            if let Some(s) = &principal.scope {
                if !record_owned(&record, s) {
                    return err_json(StatusCode::NOT_FOUND, "not found");
                }
            }
            let rec = records_json(&[record]).into_iter().next().unwrap();
            ok_json(json!({ "ok": true, "record": rec }))
        }
        Ok(None) => err_json(StatusCode::NOT_FOUND, "not found"),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn records_after(
    server: &Server,
    board: &str,
    principal: &Principal,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if !require_read(principal) {
        return err_json(StatusCode::FORBIDDEN, "read authorization required");
    }
    let after: i64 = params.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
    match server.engine.lock().unwrap().records_after(board, after) {
        Ok(rs) => ok_json(json!({ "ok": true, "records": records_json(&rs) })),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn check_scope_owns(server: &Server, board: &str, seq: i64, principal: &Principal) -> Option<Response<BoxBodyResp>> {
    if let Some(s) = &principal.scope {
        match server.engine.lock().unwrap().get_record(board, seq) {
            Ok(Some(rec)) if record_owned(&rec, s) => None,
            _ => Some(err_json(StatusCode::NOT_FOUND, "not found")),
        }
    } else {
        None
    }
}

fn record_put(
    server: &Server,
    board: &str,
    principal: &Principal,
    seq: &str,
    body: &Bytes,
    _params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if let Err(e) = rate_action(server, board, principal, "submit") {
        return err_json(StatusCode::TOO_MANY_REQUESTS, &e.to_string());
    }
    let seq: i64 = match seq.parse() {
        Ok(s) => s,
        Err(_) => return err_json(StatusCode::BAD_REQUEST, "invalid seq"),
    };
    if !require_write(principal) {
        return err_json(StatusCode::FORBIDDEN, "writer authorization required");
    }
    if let Some(resp) = check_scope_owns(server, board, seq, principal) {
        return resp;
    }
    let payload: Json = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(_) => return err_json(StatusCode::BAD_REQUEST, "invalid json"),
    };
    let writer = principal.writer.clone();
    let updated = {
        let mut engine = server.engine.lock().unwrap();
        engine.set_record(board, seq, payload, writer.as_deref())
    };
    match updated {
        Ok(()) => ok_json(json!({ "ok": true, "seq": seq })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

fn record_patch(
    server: &Server,
    board: &str,
    principal: &Principal,
    seq: &str,
    body: &Bytes,
    _params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if let Err(e) = rate_action(server, board, principal, "submit") {
        return err_json(StatusCode::TOO_MANY_REQUESTS, &e.to_string());
    }
    let seq: i64 = match seq.parse() {
        Ok(s) => s,
        Err(_) => return err_json(StatusCode::BAD_REQUEST, "invalid seq"),
    };
    if !require_write(principal) {
        return err_json(StatusCode::FORBIDDEN, "writer authorization required");
    }
    if let Some(resp) = check_scope_owns(server, board, seq, principal) {
        return resp;
    }
    let ops: Json = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(_) => return err_json(StatusCode::BAD_REQUEST, "invalid json"),
    };
    let writer = principal.writer.clone();
    let patched = {
        let mut engine = server.engine.lock().unwrap();
        engine.patch_record(board, seq, &ops, writer.as_deref())
    };
    match patched {
        Ok(merged) => ok_json(json!({ "ok": true, "seq": seq, "payload": merged })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

fn record_delete(
    server: &Server,
    board: &str,
    principal: &Principal,
    seq: &str,
    _params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if let Err(e) = rate_action(server, board, principal, "submit") {
        return err_json(StatusCode::TOO_MANY_REQUESTS, &e.to_string());
    }
    let seq: i64 = match seq.parse() {
        Ok(s) => s,
        Err(_) => return err_json(StatusCode::BAD_REQUEST, "invalid seq"),
    };
    if !require_write(principal) {
        return err_json(StatusCode::FORBIDDEN, "writer authorization required");
    }
    if let Some(resp) = check_scope_owns(server, board, seq, principal) {
        return resp;
    }
    let deleted = {
        let mut engine = server.engine.lock().unwrap();
        engine.delete_record(board, seq)
    };
    match deleted {
        Ok(true) => ok_json(json!({ "ok": true, "seq": seq })),
        Ok(false) => err_json(StatusCode::NOT_FOUND, "not found"),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn bulk(
    server: &Server,
    board: &str,
    principal: &Principal,
    body: &Bytes,
    _params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if let Err(e) = rate_action(server, board, principal, "submit") {
        return err_json(StatusCode::TOO_MANY_REQUESTS, &e.to_string());
    }
    if !require_write(principal) {
        return err_json(StatusCode::FORBIDDEN, "writer authorization required");
    }
    let req: Json = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return err_json(StatusCode::BAD_REQUEST, "invalid json"),
    };
    let records: Vec<Json> = match req.get("records").and_then(|r| r.as_array()) {
        Some(arr) if !arr.is_empty() => arr.clone(),
        _ => return err_json(StatusCode::BAD_REQUEST, "expected non-empty records array"),
    };
    let upsert = req.get("upsert").and_then(|u| u.as_bool()).unwrap_or(false);
    let writer = principal.writer.clone();
    let inserted = {
        let mut engine = server.engine.lock().unwrap();
        engine.bulk_insert(board, records, writer.as_deref(), upsert, principal)
    };
    match inserted {
        Ok(seqs) => ok_json(json!({ "ok": true, "inserted": seqs.len(), "seqs": seqs })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

fn aggregate(
    server: &Server,
    board: &str,
    principal: &Principal,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if let Err(e) = rate_action(server, board, principal, "read") {
        return err_json(StatusCode::TOO_MANY_REQUESTS, &e.to_string());
    }
    if !require_read(principal) {
        return err_json(StatusCode::FORBIDDEN, "read authorization required");
    }
    let agg = match params.get("op").and_then(|o| Agg::parse(o).ok()) {
        Some(a) => a,
        None => return err_json(StatusCode::BAD_REQUEST, "missing or invalid op (count, sum, avg, min, max)"),
    };
    let field = params.get("field").map(|s| s.to_string());
    let group_by = params.get("group_by").map(|s| s.to_string());
    let conds = match params.get("filter").map(|f| url_decode(f)) {
        Some(f) if !f.is_empty() => match parse_filter_param(&f) {
            Ok(sf) => sf,
            Err(e) => return err_json(StatusCode::BAD_REQUEST, &e.to_string()),
        },
        _ => SrvFilter::new(),
    };
    match server.engine.lock().unwrap().aggregate_records(board, &conds, agg, field.as_deref(), group_by.as_deref()) {
        Ok(rows) => ok_json(json!({ "ok": true, "aggregate": rows })),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn join(
    server: &Server,
    board: &str,
    principal: &Principal,
    params: &HashMap<String, String>,
) -> Response<BoxBodyResp> {
    if !require_read(principal) {
        return err_json(StatusCode::FORBIDDEN, "read authorization required");
    }
    if principal.role == "customer" {
        return err_json(StatusCode::FORBIDDEN, "not permitted for customer keys");
    }
    let limit: usize = params
        .get("limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let offset: usize = params.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    let conds = match params.get("filter").map(|f| url_decode(f)) {
        Some(f) if !f.is_empty() => match parse_filter_param(&f) {
            Ok(sf) => sf,
            Err(e) => return err_json(StatusCode::BAD_REQUEST, &e.to_string()),
        },
        _ => SrvFilter::new(),
    };
    match server.engine.lock().unwrap().join_list(board, &conds, limit, offset) {
        Ok(records) => {
            let rows: Vec<Json> = records
                .iter()
                .map(|r| json!({ "seq": r.seq, "payload": r.payload, "created_at": r.created_at }))
                .collect();
            ok_json(json!({ "ok": true, "records": rows }))
        }
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn upload(
    server: &Server,
    board: &str,
    principal: &Principal,
    body: &Bytes,
    headers: &http::HeaderMap,
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
        engine.upload(board, &filename, &content_type, body, &meta)
    };
    match uploaded {
        Ok(seq) => ok_json(json!({ "ok": true, "seq": seq })),
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
    let full = if file.starts_with("files/") {
        format!("{board}/{file}")
    } else {
        file
    };
    let app = server.engine.lock().unwrap().get_app(board).ok().flatten();
    let Some(app) = app else {
        return err_json(StatusCode::NOT_FOUND, "not found");
    };
    if !app.public_reads && !require_read(principal) {
        return err_json(StatusCode::FORBIDDEN, "private board");
    }
    let engine = server.engine.lock().unwrap();
    match engine.download(board, &full) {
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
    _server: &Server,
    _board: &str,
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
    let headers: Vec<(String, String)> = match req.get("headers").and_then(|h| h.as_object()) {
        Some(map) => map
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
        None => Vec::new(),
    };
    match engine::automation::srv_http_call(&url, &headers, &req.get("body").cloned().unwrap_or(Json::Null)) {
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
    let _ = engine.dispatch_recipes(board, EventKind::Inbound, None, Some(payload));
    ok_json(json!({ "ok": true }))
}