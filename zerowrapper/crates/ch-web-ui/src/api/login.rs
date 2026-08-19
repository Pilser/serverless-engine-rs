use topcoat::Result;
use topcoat::router::{Body, Response, StatusCode, route, uri};
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;

fn url_decode(s: &str) -> String {
    s.replace("%20", " ")
        .replace("%3A", ":")
        .replace("%2F", "/")
        .replace("%3F", "?")
        .replace("%3D", "=")
        .replace("%26", "&")
}

#[route(GET "/api/login")]
pub async fn login(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);

    let query = uri(cx).query().unwrap_or("");
    let phone = query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            if parts.next()? == "phone" {
                parts.next().map(url_decode)
            } else {
                None
            }
        })
        .next()
        .unwrap_or_default();

    if phone.is_empty() {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": "Missing phone"}).to_string(),
            ))
            .unwrap());
    }

    match ctx.db.system().get_user(&phone).await {
        Ok(Some(user)) => {
            let display_name = if user.name.is_empty() {
                phone.clone()
            } else {
                user.name
            };
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "exists": true,
                        "phone": user.id,
                        "name": display_name,
                        "role": user.role
                    })
                    .to_string(),
                ))
                .unwrap())
        }
        Ok(None) => Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"exists": false, "phone": phone}).to_string(),
            ))
            .unwrap()),
        Err(e) => {
            tracing::warn!("login: db error: {}", e);
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
