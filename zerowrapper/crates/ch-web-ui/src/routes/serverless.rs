use topcoat::Result;
use topcoat::router::{Body, Response, StatusCode, route, path_param, to_bytes, headers, uri};
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;
use zw_db::serverless::{Agg, FilterCond, Op, gen_file_id, parse_filter, srv_board_file_path, srv_board_files_dir};

#[path_param]
pub(crate) struct BoardId(str);

#[path_param]
struct Seq(str);

fn json_response(status: StatusCode, v: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from(v.to_string()))
        .unwrap()
}

pub(crate) fn srv_json_response(status: StatusCode, v: serde_json::Value) -> Response {
    json_response(status, v)
}

async fn board_pool(ctx: &AppContext, board_id: &str) -> Result<std::sync::Arc<zw_db::DbPool>, Response> {
    if let Some((tpl, ver)) = zw_db::templates::parse_version_sid(board_id) {
        let materials = match ctx.materials() {
            Some(m) => m,
            None => return Err(json_response(StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({"error": "materials unavailable"}))),
        };
        if !zw_materials::layout::manifest_path(materials.root(), &tpl, ver).is_file() {
            return Err(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"})));
        }
        return match materials.version_pool(&tpl, ver).await {
            Ok(p) => Ok(p),
            Err(e) => {
                tracing::warn!("materials board_pool: {e}");
                Err(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"})))
            }
        };
    }
    match ctx.db.board_pool(board_id).await {
        Ok(p) => Ok(p),
        Err(e) => {
            tracing::warn!("board_pool: {e}");
            Err(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"})))
        }
    }
}

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

fn params_to_json<'a>(records: &[zw_db::serverless::SrvRecord]) -> Vec<serde_json::Value> {
    records
        .iter()
        .map(|r| {
            let mut v = serde_json::json!({"seq": r.seq, "payload": r.payload, "created_at": r.created_at});
            if let Some(score) = r.score {
                v["score"] = serde_json::json!(score);
            }
            if let Some(w) = &r.writer {
                v["writer"] = serde_json::json!(w);
            }
            if let Some(s) = &r.snippet {
                v["snippet"] = serde_json::json!(s);
            }
            v
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

#[route(POST "/api/srv/{board_id}/submit")]
pub async fn srv_submit(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let params = query_params(cx);
    let token = bearer_token(cx);
    if let Err(resp) = rate_action(cx, ctx, board_id, &token, "submit").await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let body_bytes = match to_bytes(body, 1_000_000).await {
        Ok(b) => b,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };
    let mut payload: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(p) => p,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };

    match pool.srv_app_by_id(board_id).await {
        Ok(Some(_)) => {}
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    }

    if let Err(e) = pool.srv_validate(board_id, &payload).await {
        return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()})));
    }

    let identity = match authorize_write(cx, ctx, board_id, true).await {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    let writer = request_writer(cx, &identity);
    let scope = match write_scope(cx, ctx, board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };
    if let Some(s) = &scope {
        force_scope(&mut payload, s);
    }

    let upsert = params.get("upsert").is_some()
        || matches!(params.get("upsert").map(|v| *v), Some("1") | Some("true"));

    match pool.srv_record_insert(board_id, &payload, writer.as_deref(), upsert).await {
        Ok(seq) => {
            publish_record_event(ctx, board_id, "created", seq).await;
            Ok(json_response(StatusCode::CREATED, serde_json::json!({"ok": true, "seq": seq})))
        }
        Err(e) => {
            tracing::warn!("srv_submit: insert error: {}", e);
            Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "insert failed"})))
        }
    }
}

async fn require_public_readable(ctx: &AppContext, board_id: &str) -> Result<Option<zw_db::serverless::SrvApp>, Response> {
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Err(resp),
    };
    match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) if app.public_reads => Ok(Some(app)),
        Ok(Some(_)) => Err(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "private board"}))),
        _ => Err(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    }
}

pub(crate) enum SrvIdentity {
    LegacyAdmin,
    AppKey { role: String, writer: Option<String>, scope: Option<String> },
}

fn role_ge(role: &str, need: &str) -> bool {
    fn rank(r: &str) -> i32 {
        match r {
            "admin" => 3,
            "writer" => 2,
            "list" => 1,
            "reader" => 1,
            _ => 0,
        }
    }
    rank(role) >= rank(need)
}

fn customer_scope(identity: &SrvIdentity) -> Option<String> {
    match identity {
        SrvIdentity::AppKey { role, scope, .. } if role == "customer" => scope.clone(),
        _ => None,
    }
}

fn scope_cond(scope: &str) -> zw_db::filters::FilterCond {
    zw_db::filters::FilterCond {
        field: "$.customer_id".to_string(),
        op: zw_db::filters::Op::Eq,
        value: serde_json::json!(scope),
    }
}

async fn read_scope(cx: &Cx, ctx: &AppContext, board_id: &str) -> Result<Option<String>, Response> {
    match authorize_read(cx, ctx, board_id).await? {
        Some(id) => Ok(customer_scope(&id)),
        None => Ok(None),
    }
}

async fn write_scope(cx: &Cx, ctx: &AppContext, board_id: &str) -> Result<Option<String>, Response> {
    match authorize_write(cx, ctx, board_id, true).await? {
        Some(id) => Ok(customer_scope(&id)),
        None => Ok(None),
    }
}

fn refuse_customer(scope: &Option<String>) -> Result<(), Response> {
    if scope.is_some() {
        return Err(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "not permitted for customer keys"})));
    }
    Ok(())
}

fn record_owned(record: &zw_db::serverless::SrvRecord, scope: &str) -> bool {
    record
        .payload
        .get("customer_id")
        .and_then(|v| v.as_str())
        .map(|c| c == scope)
        .unwrap_or(false)
}

fn force_scope(payload: &mut serde_json::Value, scope: &str) {
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("customer_id".to_string(), serde_json::json!(scope));
    }
}

fn bearer_token(cx: &Cx) -> Option<String> {
    let headers = headers(cx);
    if let Some(token) = headers.get("Authorization").and_then(|v| v.to_str().ok()).and_then(|s| {
        let mut parts = s.splitn(2, ' ');
        if parts.next()?.eq_ignore_ascii_case("bearer") {
            let rest = parts.next()?.trim();
            if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            }
        } else {
            None
        }
    }) {
        return Some(token);
    }
    if let Some(token) = headers
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
        .and_then(ws_subprotocol_key)
    {
        return Some(token);
    }
    query_params(cx)
        .get("key")
        .map(|k| k.to_string())
        .filter(|k| !k.is_empty())
}

fn ws_subprotocol_key(proto: &str) -> Option<String> {
    let mut parts = proto.split(',').map(str::trim);
    while let Some(p) = parts.next() {
        if p.eq_ignore_ascii_case("bearer") {
            let key = parts.next()?.trim();
            if key.is_empty() {
                return None;
            }
            return Some(key.to_string());
        }
    }
    None
}

fn rate_client_key(cx: &Cx, board_id: &str, token: &Option<String>) -> String {
    if let Some(t) = token {
        return format!("{}:kw:{}", board_id, &zw_db::rate::token_digest(t)[..16]);
    }
    let ip = headers(cx)
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next().map(str::trim))
        .filter(|ip| !ip.is_empty());
    match ip {
        Some(ip) => format!("{}:ip:{}", board_id, ip),
        None => format!("{}:anon", board_id),
    }
}

async fn rate_action(cx: &Cx, ctx: &AppContext, board_id: &str, token: &Option<String>, action: &str) -> Result<(), Response> {
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Err(resp),
    };
    let cfg = match pool.srv_rate_config(board_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("rate_action: config error: {}", e);
            return Err(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "rate limit failed"})));
        }
    };
    let key = rate_client_key(cx, board_id, token);
    let (allowed, retry_after, remaining) = ctx.rates.allow(&key, action, cfg.limit(action), cfg.per_day).await;
    if allowed {
        return Ok(());
    }
    Err(Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header("Retry-After", retry_after.to_string())
        .header("X-RateLimit-Remaining", remaining.to_string())
        .body(Body::from(serde_json::json!({"error": "rate limited", "retry_after": retry_after}).to_string()))
        .unwrap())
}

pub(crate) async fn authorize_identity(cx: &Cx, ctx: &AppContext, board_id: &str) -> Result<Option<SrvIdentity>, Response> {
    let Some(token) = bearer_token(cx) else {
        return Ok(None);
    };
    if token == board_id {
        return Ok(Some(SrvIdentity::LegacyAdmin));
    }
    match ctx.db.system().srv_key_auth(&token).await {
        Ok(Some(key)) if key.board_id == board_id => {
            Ok(Some(SrvIdentity::AppKey { role: key.role, writer: key.writer, scope: key.scope }))
        }
        Ok(Some(_)) => Err(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "key not valid for this board"}))),
        Ok(None) => Err(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "unauthorized"}))),
        Err(e) => {
            tracing::warn!("authorize_identity: auth error: {}", e);
            Err(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "auth failed"})))
        }
    }
}

async fn authorize_write(cx: &Cx, ctx: &AppContext, board_id: &str, enforce: bool) -> Result<Option<SrvIdentity>, Response> {
    let identity = match authorize_identity(cx, ctx, board_id).await {
        Ok(id) => id,
        Err(resp) => return Err(resp),
    };
    if let Some(SrvIdentity::AppKey { role, .. }) = &identity {
        if !role_ge(role, "writer") && role != "customer" {
            return Err(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "write requires an authorization key"})));
        }
    }
    if enforce && identity.is_none() {
        let has_keys = match ctx.db.system().srv_board_has_keys(board_id).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("authorize_write: board has_keys error: {}", e);
                return Err(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "auth check failed"})));
            }
        };
        if has_keys {
            return Err(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "write requires an authorization key"})));
        }
    }
    Ok(identity)
}

fn request_writer(cx: &Cx, identity: &Option<SrvIdentity>) -> Option<String> {
    if let Some(SrvIdentity::AppKey { writer: Some(w), .. }) = identity {
        return Some(w.clone());
    }
    headers(cx)
        .get("X-Writer")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| query_params(cx).get("writer").map(|w| w.to_string()))
        .filter(|w| !w.is_empty())
}

pub(crate) async fn authorize_read(cx: &Cx, ctx: &AppContext, board_id: &str) -> Result<Option<SrvIdentity>, Response> {
    let identity = match authorize_identity(cx, ctx, board_id).await {
        Ok(id) => id,
        Err(resp) => return Err(resp),
    };
    match &identity {
        Some(SrvIdentity::LegacyAdmin) => Ok(identity),
        Some(SrvIdentity::AppKey { role, .. }) if role_ge(role, "reader") => Ok(identity),
        Some(SrvIdentity::AppKey { role, .. }) if role == "customer" => Ok(identity),
        Some(SrvIdentity::AppKey { .. }) => {
            Err(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "read requires an authorization key"})))
        }
        None => match require_public_readable(ctx, board_id).await {
            Ok(_) => Ok(None),
            Err(resp) => Err(resp),
        },
    }
}

async fn require_admin(cx: &Cx, ctx: &AppContext, board_id: &str) -> Result<(), Response> {
    let identity = match authorize_identity(cx, ctx, board_id).await {
        Ok(Some(id)) => id,
        Ok(None) => return Err(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"}))),
        Err(resp) => return Err(resp),
    };
    let is_admin = match &identity {
        SrvIdentity::LegacyAdmin => true,
        SrvIdentity::AppKey { role, .. } => role == "admin",
    };
    if !is_admin {
        return Err(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"})));
    }
    Ok(())
}

async fn board_owner(ctx: &AppContext, board_id: &str) -> Result<String, Response> {
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Err(resp),
    };
    match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => Ok(app.owner_key),
        Ok(None) => Err(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
        Err(_) => Err(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "owner lookup failed"}))),
    }
}

async fn parse_json_body(body: Body, cap: usize) -> Result<serde_json::Value, Response> {
    match to_bytes(body, cap).await {
        Ok(b) => match serde_json::from_slice(&b) {
            Ok(v) => Ok(v),
            Err(_) => Err(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
        },
        Err(_) => Err(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    }
}

#[route(OPTIONS "/api/srv/{board_id}/{*rest}")]
pub async fn srv_preflight(_cx: &Cx) -> Result<Response> {
    Ok(Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS")
        .header("Access-Control-Allow-Headers", "Authorization, Content-Type, X-Writer")
        .header("Access-Control-Max-Age", "86400")
        .body(Body::empty())
        .unwrap())
}

#[route(GET "/api/srv/{board_id}/records")]
pub async fn srv_records(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let params = query_params(cx);
    let token = bearer_token(cx);
    let action = match params.get("q").map(|q| url_decode(q)).filter(|q| !q.is_empty()) {
        Some(_) => "search",
        None => "read",
    };
    if let Err(resp) = rate_action(cx, ctx, board_id, &token, action).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let limit: i64 = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(50).clamp(1, 500);
    let offset: i64 = params.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0).max(0);
    let hl = params.get("hl").and_then(|v| if *v == "1" || *v == "on" || *v == "true" { Some(()) } else { None }).is_some();
    let writer = params.get("writer").map(|w| url_decode(w)).filter(|w| !w.is_empty());
        let order: Vec<(String, bool)> = params
            .get("order")
            .map(|o| url_decode(o))
            .filter(|o| !o.is_empty())
            .into_iter()
            .flat_map(|o| {
                let dir = match params.get("dir").map(|d| *d) {
                    Some("asc") => false,
                    Some("desc") => true,
                    _ => false,
                };
                o.split(',')
                    .map(|p| (p.to_string(), dir))
                    .filter(|(p, _)| !p.is_empty())
                    .collect::<Vec<_>>()
            })
            .collect();
        let orders: Vec<(&str, bool)> = order.iter().map(|(p, d)| (p.as_str(), *d)).collect();

    let scope = match read_scope(cx, ctx, board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };

    let records = if let Some(q) = params.get("q").map(|q| url_decode(q)).filter(|q| !q.is_empty()) {
        let mut conds = if let Some(f) = params.get("filter").map(|f| url_decode(f)).filter(|f| !f.is_empty()) {
            let parsed: serde_json::Value = match serde_json::from_str(&f) {
                Ok(v) => v,
                Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid filter json"}))),
            };
            match parse_filter(&parsed) {
                Ok(sf) => sf.conds,
                Err(e) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
            }
        } else {
            Vec::new()
        };
        if let Some(s) = &scope {
            conds.push(scope_cond(s));
        }
        if let Some(w) = &writer {
            conds.insert(0, FilterCond { field: "$.writer".to_string(), op: Op::Eq, value: serde_json::json!(w) });
        }
        match pool.srv_record_search(board_id, &q, &conds, limit, offset, hl).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("srv_records: search error: {}", e);
                return Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "search failed"})));
            }
        }
    } else if let Some(f) = params.get("filter").map(|f| url_decode(f)).filter(|f| !f.is_empty()) {
        let parsed: serde_json::Value = match serde_json::from_str(&f) {
            Ok(v) => v,
            Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid filter json"}))),
        };
        let mut filter = match parse_filter(&parsed) {
            Ok(sf) => sf,
            Err(e) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
        };
        if let Some(s) = &scope {
            filter.conds.push(scope_cond(s));
        }
        if let Some(w) = &writer {
            filter.conds.push(FilterCond { field: "$.writer".to_string(), op: Op::Eq, value: serde_json::json!(w) });
        }
        match pool
            .srv_record_filter(board_id, &filter.conds, &orders, limit, offset)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("srv_records: filter error: {}", e);
                return Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "query failed"})));
            }
        }
    } else if let Some(w) = &writer {
        let mut conds = vec![FilterCond { field: "$.writer".to_string(), op: Op::Eq, value: serde_json::json!(w) }];
        if let Some(s) = &scope {
            conds.push(scope_cond(s));
        }
        match pool.srv_record_filter(board_id, &conds, &[], limit, offset).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("srv_records: writer filter error: {}", e);
                return Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "query failed"})));
            }
        }
    } else {
        let conds: Vec<FilterCond> = scope.iter().map(|s| scope_cond(s)).collect();
        match pool.srv_record_filter(board_id, &conds, &[], limit, offset).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("srv_records: list error: {}", e);
                return Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "list failed"})));
            }
        }
    };

    Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "records": params_to_json(&records)})))
}

#[route(GET "/api/srv/{board_id}/aggregate")]
pub async fn srv_aggregate(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let params = query_params(cx);
    let token = bearer_token(cx);
    if let Err(resp) = rate_action(cx, ctx, board_id, &token, "read").await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let agg = match params.get("op").and_then(|o| Agg::parse(o).ok()) {
        Some(a) => a,
        None => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "missing or invalid op (count, sum, avg, min, max)"}))),
    };
    let field = params.get("field").map(|s| s.to_string());
    let group_by = params.get("group_by").map(|s| s.to_string());

    let scope = match read_scope(cx, ctx, board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };
    if let Err(resp) = refuse_customer(&scope) {
        return Ok(resp);
    }

    let conds = if let Some(f) = params.get("filter").map(|f| url_decode(f)).filter(|f| !f.is_empty()) {
        let parsed: serde_json::Value = match serde_json::from_str(&f) {
            Ok(v) => v,
            Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid filter json"}))),
        };
        match parse_filter(&parsed) {
            Ok(sf) => sf.conds,
            Err(e) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
        }
    } else {
        Vec::new()
    };

    match pool
        .srv_record_aggregate(board_id, &conds, agg, field.as_deref(), group_by.as_deref())
        .await
    {
        Ok(rows) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "aggregate": rows}))),
        Err(e) => {
            tracing::warn!("srv_aggregate: error: {}", e);
            Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "aggregate failed"})))
        }
    }
}

#[route(GET "/api/srv/{board_id}/record")]
pub async fn srv_record(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let params = query_params(cx);
    let token = bearer_token(cx);
    if let Err(resp) = rate_action(cx, ctx, board_id, &token, "read").await {
        return Ok(resp);
    }

    let seq: i64 = match params.get("seq").and_then(|s| s.parse().ok()) {
        Some(s) => s,
        None => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "missing or invalid seq"}))),
    };

    let scope = match read_scope(cx, ctx, board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };

    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };

    match pool.srv_record_get(board_id, seq).await {
        Ok(Some(record)) => {
            if let Some(s) = &scope {
                if !record_owned(&record, s) {
                    return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"})));
                }
            }
            let records_json = params_to_json(&[record]);
            let rec = records_json[0].clone();
            Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "record": rec})))
        }
        Ok(None) => Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
        Err(e) => {
            tracing::warn!("srv_record: get error: {}", e);
            Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "get failed"})))
        }
    }
}

#[route(PUT "/api/srv/{board_id}/records/{seq}")]
pub async fn srv_record_put(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let token = bearer_token(cx);
    if let Err(resp) = rate_action(cx, ctx, board_id, &token, "submit").await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let seq: i64 = match path_param::<Seq>(cx).parse() {
        Ok(s) => s,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid seq"}))),
    };

    match pool.srv_app_by_id(board_id).await {
        Ok(Some(_)) => {}
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    }

    let identity = match authorize_write(cx, ctx, board_id, true).await {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    let scope = match write_scope(cx, ctx, board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };

    let body_bytes = match to_bytes(body, 1_000_000).await {
        Ok(b) => b,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };
    let mut payload: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(p) => p,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };
    if let Some(s) = &scope {
        force_scope(&mut payload, s);
    }

    if let Err(e) = pool.srv_validate(board_id, &payload).await {
        return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()})));
    }

    match pool.srv_record_get(board_id, seq).await {
        Ok(Some(rec)) => {
            if let Some(s) = &scope {
                if !record_owned(&rec, s) {
                    return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"})));
                }
            }
        }
        Ok(None) => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
        Err(e) => {
            tracing::warn!("srv_record_put: get error: {}", e);
            return Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "get failed"})));
        }
    }

    let writer = request_writer(cx, &identity);

    match pool.srv_record_set(board_id, seq, &payload, writer.as_deref()).await {
        Ok(()) => {
            publish_record_event(ctx, board_id, "updated", seq).await;
            Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "seq": seq})))
        }
        Err(e) => {
            tracing::warn!("srv_record_put: set error: {}", e);
            Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()})))
        }
    }
}

#[route(PATCH "/api/srv/{board_id}/records/{seq}")]
pub async fn srv_record_patch_route(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let token = bearer_token(cx);
    if let Err(resp) = rate_action(cx, ctx, board_id, &token, "submit").await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let seq: i64 = match path_param::<Seq>(cx).parse() {
        Ok(s) => s,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid seq"}))),
    };

    match pool.srv_app_by_id(board_id).await {
        Ok(Some(_)) => {}
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    }

    let _identity = match authorize_write(cx, ctx, board_id, true).await {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    let scope = match write_scope(cx, ctx, board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };

    let body_bytes = match to_bytes(body, 1_000_000).await {
        Ok(b) => b,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };
    let ops: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(p) => p,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };

    match pool.srv_record_get(board_id, seq).await {
        Ok(Some(rec)) => {
            if let Some(s) = &scope {
                if !record_owned(&rec, s) {
                    return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"})));
                }
            }
        }
        Ok(None) => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
        Err(e) => {
            tracing::warn!("srv_record_patch_route: get error: {}", e);
            return Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "get failed"})));
        }
    }

    match pool.srv_record_patch(board_id, seq, &ops).await {
        Ok(merged) => {
            publish_record_event(ctx, board_id, "updated", seq).await;
            Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "seq": seq, "payload": merged})))
        }
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

async fn persist_upload(
    ctx: &AppContext,
    board_id: &str,
    app: &zw_db::serverless::SrvApp,
    filename: &str,
    content_type: &str,
    data: &[u8],
    extra: serde_json::Map<String, serde_json::Value>,
    writer: Option<&str>,
    scope: Option<&str>,
) -> Response {
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    let handle = match ctx.whiteboard() {
        Some(h) => h,
        None => return json_response(StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({"error": "whiteboard unavailable"})),
    };
    let root = handle.dir();

    let file_id = gen_file_id();
    let dest_dir = srv_board_files_dir(root, &app.owner_key, board_id);
    if let Err(e) = tokio::fs::create_dir_all(&dest_dir).await {
        tracing::warn!("srv_upload: create dir error: {}", e);
        return json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "save failed"}));
    }
    let dest_path = srv_board_file_path(root, &app.owner_key, board_id, &file_id);
    if let Err(e) = tokio::fs::write(&dest_path, data).await {
        tracing::warn!("srv_upload: write error: {}", e);
        return json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "save failed"}));
    }

    let mut payload = extra;
    payload.insert("file".to_string(), serde_json::json!(format!("files/{}.bin", file_id)));
    payload.insert("name".to_string(), serde_json::json!(filename));
    payload.insert("type".to_string(), serde_json::json!(content_type));
    payload.insert("size".to_string(), serde_json::json!(data.len()));
    if let Some(s) = scope {
        payload.insert("customer_id".to_string(), serde_json::json!(s));
    }

    match pool.srv_record_insert(board_id, &serde_json::Value::Object(payload), writer, false).await {
        Ok(seq) => {
            publish_record_event(ctx, board_id, "created", seq).await;
            json_response(
                StatusCode::CREATED,
                serde_json::json!({"ok": true, "seq": seq, "file": format!("files/{}.bin", file_id)}),
            )
        },
        Err(e) => {
            tracing::warn!("srv_upload: insert error: {}", e);
            json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "insert failed"}))
        }
    }
}

#[route(POST "/api/srv/{board_id}/upload")]
pub async fn srv_upload(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let token = bearer_token(cx);
    if let Err(resp) = rate_action(cx, ctx, board_id, &token, "upload").await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };

    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };

    let identity = match authorize_write(cx, ctx, board_id, true).await {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    let writer = request_writer(cx, &identity);
    let scope = match write_scope(cx, ctx, board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };
    let scope_ref = scope.as_deref();

    let content_type = headers(cx)
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let body_bytes = match to_bytes(body, 25_000_000).await {
        Ok(b) => b,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid request"}))),
    };

    if body_bytes.is_empty() {
        return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "empty file"})));
    }

    if let Some(boundary) = crate::routes::multipart::boundary_from_content_type(&content_type) {
        let parts = match crate::routes::multipart::parse(&body_bytes, &boundary) {
            Ok(p) => p,
            Err(e) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": format!("multipart: {e}")}))),
        };
        let mut extra = serde_json::Map::new();
        let mut file_field: Option<crate::routes::multipart::MultipartField> = None;
        for part in parts {
            if part.filename.is_some() {
                if file_field.is_none() {
                    file_field = Some(part);
                }
            } else if !part.name.is_empty() {
                extra.insert(part.name, serde_json::Value::String(String::from_utf8_lossy(&part.data).to_string()));
            }
        }
        let Some(file) = file_field else {
            return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "no file part in multipart form"})));
        };
        let filename = file.filename.unwrap_or_else(|| "upload".to_string());
        let file_type = file.content_type.unwrap_or_else(|| "application/octet-stream".to_string());
        return Ok(persist_upload(ctx, board_id, &app, &filename, &file_type, &file.data, extra, writer.as_deref(), scope_ref).await);
    }

    let filename = headers(cx)
        .get("X-Filename")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("upload")
        .to_string();

    Ok(persist_upload(ctx, board_id, &app, &filename, &content_type, &body_bytes, serde_json::Map::new(), writer.as_deref(), scope_ref).await)
}

#[route(GET "/api/srv/{board_id}/file")]
pub async fn srv_file(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let params = query_params(cx);

    let file = match params.get("file") {
        Some(f) if f.starts_with("files/") && f.ends_with(".bin") => f.to_string(),
        _ => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid file"}))),
    };
    let file_id = file
        .strip_prefix("files/")
        .and_then(|rest| rest.strip_suffix(".bin"))
        .unwrap_or_default()
        .to_string();
    if !file_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid file"})));
    }

    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };

    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) if app.public_reads => app,
        Ok(Some(_)) => return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "private board"}))),
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };

    let handle = match ctx.whiteboard() {
        Some(h) => h,
        None => return Ok(json_response(StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({"error": "whiteboard unavailable"}))),
    };
    let root = handle.dir();
    let files_dir = srv_board_files_dir(root, &app.owner_key, board_id);
    let path = srv_board_file_path(root, &app.owner_key, board_id, &file_id);
    if !path.starts_with(&files_dir) {
        return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "forbidden"})));
    }
    if !path.is_file() {
        return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"})));
    }

    let mut content_type = "application/octet-stream".to_string();
    let mut filename = file_id.clone();
    if let Ok(records) = pool.srv_record_query_raw(board_id, "$.file", Some(&file)).await {
        if let Some(first) = records.first() {
            if let Some(t) = first.payload["type"].as_str() {
                content_type = t.to_string();
            }
            if let Some(n) = first.payload["name"].as_str() {
                filename = n.to_string();
            }
        }
    }

    let data = match tokio::fs::read(&path).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("srv_file: read error: {}", e);
            return Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "read failed"})));
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", content_type)
        .header("Content-Disposition", format!("inline; filename=\"{}\"", filename))
        .body(Body::from(data))
        .unwrap())
}

#[route(POST "/api/srv/{board_id}/keys")]
pub async fn srv_keys_issue(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let token = bearer_token(cx);
    if let Err(resp) = rate_action(cx, ctx, board_id, &token, "submit").await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };

    let identity = match authorize_identity(cx, ctx, board_id).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"})))
        }
        Err(resp) => return Ok(resp),
    };
    let is_admin = match &identity {
        SrvIdentity::LegacyAdmin => true,
        SrvIdentity::AppKey { role, .. } => role == "admin",
    };
    if !is_admin {
        return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"})));
    }

    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };

    let body_bytes = match to_bytes(body, 1_000_000).await {
        Ok(b) => b,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };
    let req: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };
    let role = req.get("role").and_then(|r| r.as_str()).unwrap_or("writer").to_string();
    let writer = req.get("writer").and_then(|w| w.as_str()).map(str::to_string);
    let scope = req.get("scope").and_then(|s| s.as_str()).map(str::to_string);

    match ctx.db.key_issue(board_id, &app.owner_key, &role, writer.as_deref(), scope.as_deref()).await {
        Ok((bucket, secret)) => Ok(json_response(
            StatusCode::CREATED,
            serde_json::json!({"ok": true, "bucket": bucket, "key": secret, "role": role, "writer": writer, "scope": scope}),
        )),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(POST "/api/srv/{board_id}/hooks")]
pub async fn srv_hooks_register(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);

    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };

    let identity = match authorize_identity(cx, ctx, board_id).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"})))
        }
        Err(resp) => return Ok(resp),
    };
    let is_admin = match &identity {
        SrvIdentity::LegacyAdmin => true,
        SrvIdentity::AppKey { role, .. } => role == "admin",
    };
    if !is_admin {
        return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"})));
    }

    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };

    let body_bytes = match to_bytes(body, 1_000_000).await {
        Ok(b) => b,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };
    let req: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };
    let url = match req.get("url").and_then(|u| u.as_str()) {
        Some(u) if u.starts_with("http://") || u.starts_with("https://") => u.to_string(),
        _ => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "url must start with http:// or https://"}))),
    };
    let secret = req.get("secret").and_then(|s| s.as_str()).filter(|s| !s.is_empty()).map(str::to_string);

    match pool.srv_hook_register(board_id, &app.owner_key, &url, secret.as_deref()).await {
        Ok(()) => Ok(json_response(StatusCode::CREATED, serde_json::json!({"ok": true}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(DELETE "/api/srv/{board_id}/hooks")]
pub async fn srv_hooks_remove(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let params = query_params(cx);

    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };

    let identity = match authorize_identity(cx, ctx, board_id).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"})))
        }
        Err(resp) => return Ok(resp),
    };
    let is_admin = match &identity {
        SrvIdentity::LegacyAdmin => true,
        SrvIdentity::AppKey { role, .. } => role == "admin",
    };
    if !is_admin {
        return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"})));
    }

    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };

    let url = params.get("url").map(|u| url_decode(u)).filter(|u| !u.is_empty());
    match url {
        Some(u) => match pool.srv_hook_remove(board_id, &app.owner_key, &u).await {
            Ok(true) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true}))),
            Ok(false) => Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "hook not found"}))),
            Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
        },
        None => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "missing url query param"}))),
    }
}

#[route(POST "/api/srv/{board_id}/call")]
pub async fn srv_call(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);

    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };

    let identity = match authorize_identity(cx, ctx, board_id).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "authorization required"})))
        }
        Err(resp) => return Ok(resp),
    };
    let is_admin = match &identity {
        SrvIdentity::LegacyAdmin => true,
        SrvIdentity::AppKey { role, .. } => role == "admin",
    };
    if !is_admin {
        return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"})));
    }

    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };

    let body_bytes = match to_bytes(body, 1_000_000).await {
        Ok(b) => b,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };
    let req: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };

    let url = match req.get("url").and_then(|u| u.as_str()) {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "missing url"}))),
    };
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("GET").to_string();
    let headers: Vec<(String, String)> = match req.get("headers").and_then(|h| h.as_object()) {
        Some(map) => map.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect(),
        None => Vec::new(),
    };
    let body_str = match req.get("body") {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(v) => Some(v.to_string()),
        None => None,
    };
    let timeout_ms = req.get("timeout_ms").and_then(|t| t.as_u64()).unwrap_or(0);

    match pool
        .srv_http_call(board_id, &app.owner_key, &method, &url, &headers, body_str.as_deref(), timeout_ms)
        .await
    {
        Ok(res) => Ok(json_response(
            StatusCode::OK,
            serde_json::json!({
                "ok": true,
                "status": res.status,
                "duration_ms": res.duration_ms,
                "headers": res.headers,
                "body": res.body,
            }),
        )),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(POST "/api/srv/{board_id}/rate")]
pub async fn srv_rate_config(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);

    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };

    let identity = match authorize_identity(cx, ctx, board_id).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"})))
        }
        Err(resp) => return Ok(resp),
    };
    let is_admin = match &identity {
        SrvIdentity::LegacyAdmin => true,
        SrvIdentity::AppKey { role, .. } => role == "admin",
    };
    if !is_admin {
        return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"})));
    }

    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };

    let body_bytes = match to_bytes(body, 1_000_000).await {
        Ok(b) => b,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };
    let req: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };

    if req.get("clear").and_then(|v| v.as_bool()).unwrap_or(false) {
        return match pool.srv_rate_set(board_id, &app.owner_key, None).await {
            Ok(()) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true}))),
            Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
        };
    }

    let mut cfg = serde_json::Map::new();
    for field in ["submit", "upload", "search", "read", "per_day"] {
        if let Some(v) = req.get(field).and_then(|v| v.as_u64()) {
            cfg.insert(field.to_string(), serde_json::json!(v));
        }
    }
    match pool.srv_rate_set(board_id, &app.owner_key, Some(&serde_json::Value::Object(cfg).to_string())).await {
        Ok(()) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

async fn publish_record_event(ctx: &AppContext, board_id: &str, kind: &str, seq: i64) {
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(_) => return,
    };
    let record = match pool.srv_record_get_raw(board_id, seq).await {
        Ok(Some(record)) => Some(record),
        _ => None,
    };
    let event = crate::realtime::record_event(kind, board_id, seq, record.as_ref());
    ctx.realtime.publish(&event);
    enqueue_hook_events(ctx, board_id, &event).await;
    dispatch_recipes(&pool, board_id, kind, seq, record.as_ref()).await;
}

async fn dispatch_recipes(
    pool: &std::sync::Arc<zw_db::DbPool>,
    board_id: &str,
    kind: &str,
    seq: i64,
    record: Option<&zw_db::serverless::SrvRecord>,
) {
    let payload = record
        .map(|r| r.payload.clone())
        .unwrap_or_else(|| serde_json::json!({}));
    let writer = record.and_then(|r| r.writer.clone());
    zw_recipe::run_recipes_for_record(pool, board_id, kind, seq, &payload, writer.as_deref()).await;
}

async fn dispatch_inbound(pool: &std::sync::Arc<zw_db::DbPool>, board_id: &str, payload: &serde_json::Value) {
    let recipes = match pool.srv_recipe_list(board_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("dispatch_inbound: list error: {}", e);
            return;
        }
    };
    let mut secrets = zw_recipe::Secrets::new();
    if let Ok(map) = pool.srv_secrets_map(board_id).await {
        for (name, value) in map {
            secrets.insert(&name, &value);
        }
    }
    let engine = zw_recipe::Engine { db: pool };
    for sr in recipes {
        let Some(recipe) = zw_recipe::recipe_from_srv(&sr) else { continue };
        if !matches!(recipe.event_kind(), Some(zw_recipe::EventKind::Inbound)) {
            continue;
        }
        let ctx = zw_recipe::EventContext::inbound(board_id, payload);
        let outcome = engine.run_recipe(&recipe, &secrets, &ctx, false).await;
        if !outcome.report.logs.is_empty() {
            tracing::debug!("dispatch_inbound: recipe {} logs: {:?}", recipe.name, outcome.report.logs);
        }
    }
}

#[route(POST "/api/srv/{board_id}/events")]
pub async fn srv_events_inbound(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let body_bytes = match to_bytes(body, 10_000_000).await {
        Ok(b) => b,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid body"}))),
    };
    let payload: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid json"}))),
    };

    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let secret = match pool.srv_webhook_secret_get(board_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("srv_events_inbound: secret get error: {}", e);
            return Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "secret lookup failed"})));
        }
    };
    if let Some(secret) = &secret {
        let provided = headers(cx)
            .get("X-Hub-Signature-256")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.strip_prefix("sha256=").unwrap_or(s).to_string());
        let expected = crate::webhooks::waos_signature_hex(&body_bytes, secret);
        let ok = provided.as_ref().map(|p| p.eq_ignore_ascii_case(&expected)).unwrap_or(false);
        if !ok {
            return Ok(json_response(StatusCode::UNAUTHORIZED, serde_json::json!({"error": "invalid signature"})));
        }
    }

    dispatch_inbound(&pool, board_id, &payload).await;
    Ok(json_response(StatusCode::ACCEPTED, serde_json::json!({"ok": true})))
}

async fn enqueue_hook_events(ctx: &AppContext, board_id: &str, event: &str) {
    match ctx.db.hook_urls(board_id).await {
        Ok(hooks) => {
            for (url, _) in hooks {
                let _ = ctx.db.hook_enqueue(board_id, &url, event).await;
            }
        }
        Err(e) => tracing::warn!("enqueue_hook_events: urls error: {}", e),
    }
}

#[route(DELETE "/api/srv/{board_id}/records/{seq}")]
pub async fn srv_record_delete_route(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx).to_string();
    let token = bearer_token(cx);
    if let Err(resp) = rate_action(cx, ctx, &board_id, &token, "submit").await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, &board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let seq: i64 = match path_param::<Seq>(cx).parse() {
        Ok(s) => s,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid seq"}))),
    };
    match pool.srv_app_by_id(&board_id).await {
        Ok(Some(_)) => {}
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    }
    if let Err(resp) = authorize_write(cx, ctx, &board_id, true).await {
        return Ok(resp);
    }
    let scope = match write_scope(cx, ctx, &board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };
    if let Some(s) = &scope {
        match pool.srv_record_get(&board_id, seq).await {
            Ok(Some(rec)) if record_owned(&rec, s) => {}
            _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
        }
    }
    match pool.srv_record_delete_one(&board_id, seq).await {
        Ok(true) => {
            let event = crate::realtime::record_event("deleted", &board_id, seq, None);
            ctx.realtime.publish(&event);
            enqueue_hook_events(ctx, &board_id, &event).await;
            dispatch_recipes(&pool, &board_id, "deleted", seq, None).await;
            Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "seq": seq})))
        }
        Ok(false) => Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
        Err(e) => {
            tracing::warn!("srv_record_delete_route: delete error: {}", e);
            Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "delete failed"})))
        }
    }
}

#[route(GET "/api/srv/{board_id}/join")]
pub async fn srv_join(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx).to_string();
    let params = query_params(cx);
    let limit: i64 = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(50).clamp(1, 500);
    let offset: i64 = params.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0).max(0);

    let scope = match read_scope(cx, ctx, &board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };
    if let Err(resp) = refuse_customer(&scope) {
        return Ok(resp);
    }

    let pool = match board_pool(ctx, &board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };

    let conds = if let Some(f) = params.get("filter").map(|f| url_decode(f)).filter(|f| !f.is_empty()) {
        let parsed: serde_json::Value = match serde_json::from_str(&f) {
            Ok(v) => v,
            Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid filter json"}))),
        };
        match parse_filter(&parsed) {
            Ok(sf) => sf.conds,
            Err(e) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
        }
    } else {
        Vec::new()
    };

    match pool.srv_join_list(&board_id, &conds, limit, offset).await {
        Ok(rows) => {
            let mut records = Vec::new();
            for (rec, joined) in rows {
                let mut v = serde_json::json!({"seq": rec.seq, "payload": rec.payload, "created_at": rec.created_at});
                if let Some(j) = joined {
                    v["joined"] = j;
                }
                records.push(v);
            }
            Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "records": records})))
        }
        Err(e) => {
            tracing::warn!("srv_join: error: {}", e);
            Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "join failed"})))
        }
    }
}

#[route(GET "/api/srv/{board_id}/audit")]
pub async fn srv_audit(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx).to_string();
    match authorize_identity(cx, ctx, &board_id).await {
        Ok(Some(SrvIdentity::LegacyAdmin)) => {}
        Ok(Some(SrvIdentity::AppKey { role, .. })) if role == "admin" => {}
        Ok(_) => return Ok(json_response(StatusCode::FORBIDDEN, serde_json::json!({"error": "admin authorization required"}))),
        Err(resp) => return Ok(resp),
    }
    let pool = match board_pool(ctx, &board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(&board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    let params = query_params(cx);
    let since = params.get("since").map(|s| s.to_string()).filter(|s| !s.is_empty());
    let limit: i64 = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(100).clamp(1, 1000);
    match pool.srv_audit_list(&board_id, &app.owner_key, since.as_deref(), limit).await {
        Ok(rows) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "audit": rows}))),
        Err(e) => {
            tracing::warn!("srv_audit: {}", e);
            Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": "audit failed"})))
        }
    }
}

#[route(GET "/api/srv/{board_id}/config")]
pub async fn srv_config(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    let owner = app.owner_key.clone();
    let rate = pool.srv_rate_config(board_id).await;
    let ttl = pool.srv_ttl_get(board_id).await;
    let audit = pool.srv_audit_enabled(board_id).await;
    let link = pool.srv_link_get(board_id).await;
    let hooks = pool.srv_hook_list(board_id, &owner).await;
    let keys = ctx.db.system().srv_key_list(board_id, &owner).await;
    let computed = pool.srv_computed_get(board_id).await;
    let validate = pool.srv_validate_get(board_id).await;
    let redact = pool.srv_redact_get(board_id).await;
    let recipes = pool.srv_recipe_list(board_id).await;
    let secrets = pool.srv_secret_list(board_id).await;
    let webhook_secret_set = pool.srv_webhook_secret_get(board_id).await.map(|s| s.is_some()).unwrap_or(false);
    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "ok": true,
            "app": app,
            "rate": rate.unwrap_or_default(),
            "ttl": ttl.unwrap_or_else(|_| zw_db::ttl::SrvTtl { seconds: None, field: None }),
            "audit": audit.unwrap_or(false),
            "link": link.unwrap_or(None),
            "hooks": hooks.unwrap_or_default(),
            "keys": keys.unwrap_or_default(),
            "computed": computed.unwrap_or(None),
            "validate": validate.unwrap_or(None),
            "redact": redact.unwrap_or(None),
            "recipes": recipes.unwrap_or_default(),
            "secrets": secrets.unwrap_or_default(),
            "webhook_secret_set": webhook_secret_set,
        }),
    ))
}

#[route(PUT "/api/srv/{board_id}/ttl")]
pub async fn srv_ttl_set_route(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    let req = match parse_json_body(body, 1_000_000).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let seconds = req.get("seconds").and_then(|s| s.as_i64());
    let field = req.get("field").and_then(|f| f.as_str()).filter(|f| !f.is_empty());
    match pool.srv_ttl_set(board_id, &app.owner_key, seconds, field).await {
        Ok(()) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "seconds": seconds, "field": field}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(DELETE "/api/srv/{board_id}/ttl")]
pub async fn srv_ttl_clear_route(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    match pool.srv_ttl_set(board_id, &app.owner_key, None, None).await {
        Ok(()) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(PUT "/api/srv/{board_id}/link")]
pub async fn srv_link_set_route(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    let req = match parse_json_body(body, 1_000_000).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let (Some(on), Some(to), Some(via)) = (
        req.get("on").and_then(|v| v.as_str()),
        req.get("to").and_then(|v| v.as_str()),
        req.get("via").and_then(|v| v.as_str()),
    ) else {
        return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "expected on, to and via fields"})));
    };
    match pool.srv_link_set(board_id, &app.owner_key, on, to, via).await {
        Ok(()) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "on": on, "to": to, "via": via}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(DELETE "/api/srv/{board_id}/link")]
pub async fn srv_link_clear_route(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    match pool.srv_link_clear(board_id, &app.owner_key).await {
        Ok(_) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(PUT "/api/srv/{board_id}/audit")]
pub async fn srv_audit_set_route(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    let req = match parse_json_body(body, 1_000_000).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let enabled = match req.get("enabled").and_then(|e| e.as_bool()) {
        Some(e) => e,
        None => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "expected boolean enabled field"}))),
    };
    match pool.srv_app_audit_set(board_id, &app.owner_key, enabled).await {
        Ok(()) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "enabled": enabled}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

async fn set_config_kind(ctx: &AppContext, cx: &Cx, body: Body, kind: &str, clear: bool) -> Result<Response> {
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    let json = if clear {
        None
    } else {
        let req = match parse_json_body(body, 1_000_000).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };
        Some(req.to_string())
    };
    let res = match kind {
        "computed" => pool.srv_app_set_computed(board_id, &app.owner_key, json.as_deref()).await,
        "validate" => pool.srv_app_set_validate(board_id, &app.owner_key, json.as_deref()).await,
        "redact" => pool.srv_app_set_redact(board_id, &app.owner_key, json.as_deref()).await,
        _ => unreachable!(),
    };
    match res {
        Ok(()) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "kind": kind}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(PUT "/api/srv/{board_id}/computed")]
pub async fn srv_computed_set_route(cx: &Cx, body: Body) -> Result<Response> {
    set_config_kind(app_context(cx), cx, body, "computed", false).await
}

#[route(DELETE "/api/srv/{board_id}/computed")]
pub async fn srv_computed_clear_route(cx: &Cx, _body: Body) -> Result<Response> {
    set_config_kind(app_context(cx), cx, Body::empty(), "computed", true).await
}

#[route(PUT "/api/srv/{board_id}/validate")]
pub async fn srv_validate_set_route(cx: &Cx, body: Body) -> Result<Response> {
    set_config_kind(app_context(cx), cx, body, "validate", false).await
}

#[route(DELETE "/api/srv/{board_id}/validate")]
pub async fn srv_validate_clear_route(cx: &Cx, _body: Body) -> Result<Response> {
    set_config_kind(app_context(cx), cx, Body::empty(), "validate", true).await
}

#[route(PUT "/api/srv/{board_id}/redact")]
pub async fn srv_redact_set_route(cx: &Cx, body: Body) -> Result<Response> {
    set_config_kind(app_context(cx), cx, body, "redact", false).await
}

#[route(DELETE "/api/srv/{board_id}/redact")]
pub async fn srv_redact_clear_route(cx: &Cx, _body: Body) -> Result<Response> {
    set_config_kind(app_context(cx), cx, Body::empty(), "redact", true).await
}

#[route(PUT "/api/srv/{board_id}/webhook_secret")]
pub async fn srv_webhook_secret_set_route(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    let req = match parse_json_body(body, 1_000_000).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let secret = match req.get("secret").and_then(|s| s.as_str()) {
        Some(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    };
    if secret.as_ref().map(|s| s.len()) < Some(16) {
        return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "secret must be at least 16 chars"})));
    }
    match pool.srv_app_set_webhook_secret(board_id, &app.owner_key, secret.as_deref()).await {
        Ok(()) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "set": secret.is_some()}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(DELETE "/api/srv/{board_id}/webhook_secret")]
pub async fn srv_webhook_secret_clear_route(cx: &Cx, _body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    match pool.srv_app_set_webhook_secret(board_id, &app.owner_key, None).await {
        Ok(()) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "set": false}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(PATCH "/api/srv/{board_id}/app")]
pub async fn srv_app_patch(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    let req = match parse_json_body(body, 1_000_000).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let title = req.get("title").and_then(|t| t.as_str()).filter(|t| !t.is_empty());
    let schema: Option<Option<&str>> = if req.get("clear_schema").and_then(|v| v.as_bool()).unwrap_or(false) {
        Some(None)
    } else {
        req.get("schema").and_then(|s| s.as_str()).map(Some)
    };
    let unique_key: Option<Option<&str>> = if req.get("clear_unique").and_then(|v| v.as_bool()).unwrap_or(false) {
        Some(None)
    } else {
        req.get("unique").and_then(|u| u.as_str()).map(Some)
    };
    let public_reads = req.get("public_reads").and_then(|p| p.as_bool());
    match pool
        .srv_app_update(
            board_id,
            &app.owner_key,
            title,
            schema,
            public_reads,
            unique_key,
        )
        .await
    {
        Ok(()) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(GET "/api/srv/{board_id}/keys")]
pub async fn srv_keys_list(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let owner = match board_owner(ctx, board_id).await {
        Ok(o) => o,
        Err(resp) => return Ok(resp),
    };
    match ctx.db.system().srv_key_list(board_id, &owner).await {
        Ok(keys) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "keys": keys}))),
        Err(e) => Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(DELETE "/api/srv/{board_id}/keys")]
pub async fn srv_keys_revoke(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let owner = match board_owner(ctx, board_id).await {
        Ok(o) => o,
        Err(resp) => return Ok(resp),
    };
    let bucket = query_params(cx).get("bucket").map(|b| b.to_string()).filter(|b| !b.is_empty());
    let Some(bucket) = bucket else {
        return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "missing bucket query param"})));
    };
    match ctx.db.system().srv_key_revoke(board_id, &owner, &bucket).await {
        Ok(true) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true}))),
        Ok(false) => Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "key not found"}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(GET "/api/srv/{board_id}/hooks")]
pub async fn srv_hooks_list(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let app = match pool.srv_app_by_id(board_id).await {
        Ok(Some(app)) => app,
        _ => return Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "not found"}))),
    };
    match pool.srv_hook_list(board_id, &app.owner_key).await {
        Ok(hooks) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "hooks": hooks}))),
        Err(e) => Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(GET "/api/srv/{board_id}/hooks/deliveries")]
pub async fn srv_hooks_deliveries(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let owner = match board_owner(ctx, board_id).await {
        Ok(o) => o,
        Err(resp) => return Ok(resp),
    };
    let url = query_params(cx).get("url").map(|u| url_decode(u)).filter(|u| !u.is_empty());
    match ctx.db.system().srv_hook_deliveries(board_id, &owner, url.as_deref()).await {
        Ok(rows) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "deliveries": rows}))),
        Err(e) => Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(GET "/api/srv/{board_id}/recipes")]
pub async fn srv_recipes_list(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    match pool.srv_recipe_list(board_id).await {
        Ok(recipes) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "recipes": recipes}))),
        Err(e) => Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(POST "/api/srv/{board_id}/recipes")]
pub async fn srv_recipes_add(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let req = match parse_json_body(body, 1_000_000).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let name = match req.get("name").and_then(|n| n.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "missing name"}))),
    };
    let when = match req.get("when").and_then(|w| w.as_str()) {
        Some(w) if !w.is_empty() => w.to_string(),
        _ => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "missing when"}))),
    };
    let match_json = req.get("match").map(|m| m.to_string());
    let dedup = req.get("dedup_on").and_then(|d| d.as_str()).map(str::to_string);
    let enabled = req.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true);
    let actions = match req.get("actions") {
        Some(a) if a.is_array() => a.to_string(),
        _ => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "actions must be a json array"}))),
    };
    match pool
        .srv_recipe_add(board_id, &name, &when, match_json.as_deref(), enabled, dedup.as_deref(), &actions)
        .await
    {
        Ok(id) => Ok(json_response(StatusCode::CREATED, serde_json::json!({"ok": true, "recipe_id": id}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(PATCH "/api/srv/{board_id}/recipes/{name}")]
pub async fn srv_recipes_set_enabled(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let name = path_param::<BoardId>(cx);
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let req = match parse_json_body(body, 1_000_000).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let enabled = match req.get("enabled").and_then(|e| e.as_bool()) {
        Some(e) => e,
        None => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "expected boolean enabled field"}))),
    };
    match pool.srv_recipe_set_enabled(board_id, name, enabled).await {
        Ok(true) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "enabled": enabled}))),
        Ok(false) => Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "recipe not found"}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(DELETE "/api/srv/{board_id}/recipes/{name}")]
pub async fn srv_recipes_remove(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let name = path_param::<BoardId>(cx);
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    match pool.srv_recipe_remove(board_id, name).await {
        Ok(true) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true}))),
        Ok(false) => Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "recipe not found"}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(GET "/api/srv/{board_id}/secrets")]
pub async fn srv_secrets_list(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    match pool.srv_secret_list(board_id).await {
        Ok(secrets) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "secrets": secrets}))),
        Err(e) => Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(POST "/api/srv/{board_id}/secrets")]
pub async fn srv_secrets_set(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let req = match parse_json_body(body, 1_000_000).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let name = match req.get("name").and_then(|n| n.as_str()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "missing name"}))),
    };
    let value = match req.get("value").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "missing value"}))),
    };
    match pool.srv_secret_set(board_id, &name, &value).await {
        Ok(()) => Ok(json_response(StatusCode::CREATED, serde_json::json!({"ok": true, "set": name.to_uppercase()}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(DELETE "/api/srv/{board_id}/secrets/{name}")]
pub async fn srv_secrets_remove(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    if let Err(resp) = require_admin(cx, ctx, board_id).await {
        return Ok(resp);
    }
    let name = path_param::<BoardId>(cx);
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    match pool.srv_secret_remove(board_id, name).await {
        Ok(true) => Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true}))),
        Ok(false) => Ok(json_response(StatusCode::NOT_FOUND, serde_json::json!({"error": "secret not found"}))),
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(POST "/api/srv/{board_id}/bulk")]
pub async fn srv_bulk(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let token = bearer_token(cx);
    if let Err(resp) = rate_action(cx, ctx, board_id, &token, "submit").await {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let identity = match authorize_write(cx, ctx, board_id, true).await {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };
    let writer = request_writer(cx, &identity);
    let scope = match write_scope(cx, ctx, board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };
    let req = match parse_json_body(body, 10_000_000).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    let records: Vec<serde_json::Value> = match req.get("records").and_then(|r| r.as_array()) {
        Some(arr) if !arr.is_empty() => arr.clone(),
        _ => {
            return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "expected non-empty records array"})))
        }
    };
    let records: Vec<serde_json::Value> = match &scope {
        Some(s) => {
            let mut rs = records;
            for r in &mut rs {
                force_scope(r, s);
            }
            rs
        }
        None => records,
    };
    let upsert = req.get("upsert").and_then(|u| u.as_bool()).unwrap_or(false);
    match pool.srv_record_bulk_insert(&board_id, &records, writer.as_deref(), upsert).await {
        Ok(seqs) => {
            for seq in &seqs {
                publish_record_event(ctx, board_id, "created", *seq).await;
            }
            Ok(json_response(StatusCode::CREATED, serde_json::json!({"ok": true, "inserted": seqs.len(), "seqs": seqs})))
        }
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}

#[route(DELETE "/api/srv/{board_id}/records")]
pub async fn srv_records_delete(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let board_id = path_param::<BoardId>(cx);
    let scope = match write_scope(cx, ctx, board_id).await {
        Ok(s) => s,
        Err(resp) => return Ok(resp),
    };
    if let Err(resp) = refuse_customer(&scope) {
        return Ok(resp);
    }
    let pool = match board_pool(ctx, board_id).await {
        Ok(p) => p,
        Err(resp) => return Ok(resp),
    };
    let params = query_params(cx);
    let Some(filter) = params.get("filter").map(|f| url_decode(f)).filter(|f| !f.is_empty()) else {
        return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "expected filter query param"})));
    };
    let parsed: serde_json::Value = match serde_json::from_str(&filter) {
        Ok(v) => v,
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": "invalid filter json"}))),
    };
    let sf = match parse_filter(&parsed) {
        Ok(sf) => sf,
        Err(e) => return Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    };
    match pool.srv_record_delete_filter(&board_id, &sf.conds).await {
        Ok(deleted) => {
            ctx.realtime.publish(&crate::realtime::record_event("deleted_filter", board_id, 0, None));
            Ok(json_response(StatusCode::OK, serde_json::json!({"ok": true, "deleted": deleted})))
        }
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": e.to_string()}))),
    }
}