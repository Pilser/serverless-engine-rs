use super::{arg_str, ok};
use engine::model::Principal;
use engine::ServerlessEngine;
use serde_json::{json, Value as Json};

pub fn list(
    engine: &ServerlessEngine,
    _principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let files = engine.list_assets(board).map_err(|e| e.to_string())?;
    let out: Vec<Json> = files
        .into_iter()
        .map(|f| json!({ "key": f.key, "size": f.size }))
        .collect();
    ok(json!({ "files": out }))
}

pub fn put(
    engine: &ServerlessEngine,
    _principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let path = arg_str(arguments, "path")?;
    let content = arguments
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    engine
        .put_asset(board, path, content.as_bytes())
        .map_err(|e| e.to_string())?;
    ok(json!({ "asset": path, "bytes": content.len() }))
}

pub fn get(
    engine: &ServerlessEngine,
    _principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let path = arg_str(arguments, "path")?;
    let Some((bytes, content_type)) = engine.get_asset(board, path).map_err(|e| e.to_string())?
    else {
        return Err(format!("asset '{path}' not found"));
    };
    ok(json!({
        "asset": path,
        "content_type": content_type,
        "content": String::from_utf8_lossy(&bytes).to_string(),
    }))
}

pub fn delete(
    engine: &ServerlessEngine,
    _principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let path = arg_str(arguments, "path")?;
    let deleted = engine.delete_asset(board, path).map_err(|e| e.to_string())?;
    ok(json!({ "asset": path, "deleted": deleted }))
}