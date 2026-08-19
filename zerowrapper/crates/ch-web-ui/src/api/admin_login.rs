use topcoat::Result;
use topcoat::router::{Body, Response, StatusCode, route, to_bytes};
use topcoat::context::Cx;
use topcoat::context::app_context;

use crate::context::AppContext;

#[route(POST "/api/admin/login")]
pub async fn admin_login(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);

    let bytes = match to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"error": "Failed to read body"}).to_string(),
                ))
                .unwrap());
        }
    };

    let parsed: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"error": "Invalid JSON"}).to_string(),
                ))
                .unwrap());
        }
    };

    let phone = parsed.get("phone").and_then(|v| v.as_str()).unwrap_or("");
    let password = parsed
        .get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if phone.is_empty() || password.is_empty() {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": "Missing phone or password"}).to_string(),
            ))
            .unwrap());
    }

    match ctx.db.system().verify_password(phone, password).await {
        Ok(true) => match ctx.db.system().get_user(phone).await {
            Ok(Some(user)) => {
                let display_name = if user.name.is_empty() {
                    phone.to_string()
                } else {
                    user.name
                };
                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "ok": true,
                            "phone": user.id,
                            "name": display_name,
                            "role": user.role
                        })
                        .to_string(),
                    ))
                    .unwrap())
            }
            Ok(None) => Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"error": "User not found"}).to_string(),
                ))
                .unwrap()),
            Err(e) => {
                tracing::warn!("admin_login: db error: {}", e);
                Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"error": "Database error"}).to_string(),
                    ))
                    .unwrap())
            }
        },
        Ok(false) => Ok(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": "Invalid credentials"}).to_string(),
            ))
            .unwrap()),
        Err(e) => {
            tracing::warn!("admin_login: db error: {}", e);
            Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"error": "Database error"}).to_string(),
                ))
                .unwrap())
        }
    }
}
