use super::rest::{err_json, json_response, ok_json, BoxBodyResp};
use crate::Server;
use bytes::Bytes;
use engine::model::Principal;
use hyper::{Response, StatusCode};
use serde_json::{json, Value as Json};

fn parse_body(body: &Bytes) -> Result<Json, Response<BoxBodyResp>> {
    serde_json::from_slice(body).map_err(|_| err_json(StatusCode::BAD_REQUEST, "invalid json"))
}

pub fn signup(
    server: &Server,
    board: &str,
    principal: &Principal,
    body: &Bytes,
) -> Response<BoxBodyResp> {
    let req = match parse_body(body) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let email = req.get("email").and_then(|e| e.as_str()).unwrap_or("").to_string();
    let password = req.get("password").and_then(|p| p.as_str()).unwrap_or("").to_string();
    let role = req.get("role").and_then(|r| r.as_str()).unwrap_or("reader").to_string();
    match server
        .engine
        .lock()
        .unwrap()
        .signup_user(board, &email, &password, &role, principal)
    {
        Ok(user) => json_response(StatusCode::CREATED, json!({ "ok": true, "user": user })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub fn login(server: &Server, board: &str, body: &Bytes) -> Response<BoxBodyResp> {
    let req = match parse_body(body) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let email = req.get("email").and_then(|e| e.as_str()).unwrap_or("").to_string();
    let password = req.get("password").and_then(|p| p.as_str()).unwrap_or("").to_string();
    match server.engine.lock().unwrap().login_user(board, &email, &password) {
        Ok((token, jwt)) => ok_json(json!({ "ok": true, "token": token, "jwt": jwt })),
        Err(e) => err_json(StatusCode::UNAUTHORIZED, &e.to_string()),
    }
}

pub fn logout(server: &Server, _board: &str, body: &Bytes) -> Response<BoxBodyResp> {
    let req = match parse_body(body) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let token = req.get("token").and_then(|t| t.as_str()).unwrap_or("").to_string();
    match server.engine.lock().unwrap().logout_user(&token) {
        Ok(_) => ok_json(json!({ "ok": true })),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub fn me(server: &Server, board: &str, token: Option<&str>) -> Response<BoxBodyResp> {
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return err_json(StatusCode::UNAUTHORIZED, "missing session token");
    };
    match server.engine.lock().unwrap().user_by_token(board, token) {
        Ok(Some(user)) => ok_json(json!({ "ok": true, "user": user })),
        Ok(None) => err_json(StatusCode::UNAUTHORIZED, "invalid or expired session"),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}
