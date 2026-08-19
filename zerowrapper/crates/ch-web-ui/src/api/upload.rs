use topcoat::Result;
use topcoat::router::{Body, Response, StatusCode, route, to_bytes, headers};
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;

#[route(POST "/api/upload")]
pub async fn upload(cx: &Cx, body: Body) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);

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
                    serde_json::json!({"error": format!("Failed to read body: {}", e)})
                        .to_string(),
                ))
                .unwrap());
        }
    };

    if body_bytes.is_empty() {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": "Empty file"}).to_string(),
            ))
            .unwrap());
    }

    tokio::fs::create_dir_all(&ctx.upload_dir).await.unwrap_or_default();

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
    let dest_path = ctx.upload_dir.join(&dest_name);

    tracing::info!("Saving uploaded file to {:?}", dest_path);

    if let Err(e) = tokio::fs::write(&dest_path, &body_bytes).await {
        return Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({"error": format!("Failed to save file: {}", e)})
                    .to_string(),
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
