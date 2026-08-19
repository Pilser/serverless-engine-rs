use topcoat::Result;
use topcoat::router::{Body, Response, StatusCode, route, uri};
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;

fn sanitize_key(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '@' { c } else { '_' })
        .collect()
}

fn url_decode(s: &str) -> String {
    s.replace("%20", " ")
        .replace("%3A", ":")
        .replace("%2F", "/")
        .replace("%3F", "?")
        .replace("%3D", "=")
        .replace("%26", "&")
}

#[route(GET "/api/history")]
pub async fn history(cx: &Cx) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);

    let query = uri(cx).query().unwrap_or("");
    let params: std::collections::HashMap<&str, &str> = query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?;
            let val = parts.next().unwrap_or("");
            Some((key, val))
        })
        .collect();

    let user_id = params.get("user_id").map(|v| url_decode(v)).unwrap_or_default();
    let _role = params.get("role").map(|v| url_decode(v)).unwrap_or_else(|| "public".to_string());

    if user_id.is_empty() {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": "Missing user_id"}).to_string(),
            ))
            .unwrap());
    }

    let session_id = format!("web.{}.{}", ctx.agent, sanitize_key(&user_id));

    match ctx.zeroclaw.get_session_messages(session_id.clone(), Some(100)).await {
        Ok(result) => {
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&result).unwrap_or_default()))
                .unwrap())
        }
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
