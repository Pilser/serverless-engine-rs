use topcoat::Result;
use topcoat::router::{Body, Response, StatusCode, route, to_bytes};
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;

#[route(POST "/api/register")]
pub async fn register(cx: &Cx, body: Body) -> Result<Response> {
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
    if phone.is_empty() {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": "Missing phone"}).to_string(),
            ))
            .unwrap());
    }

    let name = parsed.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty());

    match ctx.db.system().upsert_user(phone, name, None).await {
        Ok(_) => {
            let display_name = name.unwrap_or(phone);
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"ok": true, "phone": phone, "name": display_name})
                        .to_string(),
                ))
                .unwrap())
        }
        Err(e) => {
            tracing::warn!("register: db error: {}", e);
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
