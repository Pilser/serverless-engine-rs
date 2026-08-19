use topcoat::Result;
use topcoat::router::route;

#[route(GET "/health")]
pub async fn health() -> Result<String> {
    Ok(r#"{"status":"ok"}"#.to_string())
}
