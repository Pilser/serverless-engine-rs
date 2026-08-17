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
    let keys = engine.list_keys(&board).map_err(|e| e.to_string())?;
    let out: Vec<Json> = keys
        .into_iter()
        .map(|k| {
            json!({
                "name": k.bucket,
                "role": k.role,
                "scope": k.scope,
                "revoked": k.revoked_at.is_some(),
            })
        })
        .collect();
    ok(json!({ "keys": out }))
}