use topcoat::Result;
use topcoat::router::{Body, Response, StatusCode, route, to_bytes, headers};
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;
use zw_mcp::dispatch::handle_request;

/// Serve the MCP server over HTTP (streamable-http compatible) on the same
/// running web instance, so the DB-backed whiteboard tool stays single-process.
#[route(POST "/mcp")]
pub async fn mcp_route(cx: &Cx, body: Body) -> Result<Response> {
    if let Ok(expected) = std::env::var("ZEROWRAPPER__MCP_TOKEN") {
        if !expected.is_empty() {
            let provided = headers(cx)
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .strip_prefix("Bearer ")
                .unwrap_or("");
            if provided != expected {
                return Ok(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"jsonrpc": "2.0", "error": {"code": -32000, "message": "unauthorized"}})
                            .to_string(),
                    ))
                    .unwrap());
            }
        }
    }

    let ctx: &AppContext = app_context(cx);
    let Some(providers) = &ctx.mcp else {
        return Ok(Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"jsonrpc": "2.0", "error": {"code": -32000, "message": "mcp not configured"}})
                    .to_string(),
            ))
            .unwrap());
    };

    let body_bytes = match to_bytes(body, 1_000_000).await {
        Ok(b) => b,
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"jsonrpc": "2.0", "error": {"code": -32700, "message": format!("read body: {}", e)}})
                        .to_string(),
                ))
                .unwrap());
        }
    };

    let req: zw_mcp::protocol::JsonRpcRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({"jsonrpc": "2.0", "error": {"code": -32700, "message": format!("parse: {}", e)}})
                        .to_string(),
                ))
                .unwrap());
        }
    };

    match handle_request(providers, req).await {
        Some(resp) => {
            let json = serde_json::to_string(&resp).unwrap_or_else(|e| {
                serde_json::json!({"jsonrpc": "2.0", "error": {"code": -32700, "message": e.to_string()}}).to_string()
            });
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .header("cache-control", "no-store")
                .body(Body::from(json))
                .unwrap())
        }
        None => Ok(Response::builder()
            .status(StatusCode::ACCEPTED)
            .body(Body::empty())
            .unwrap()),
    }
}