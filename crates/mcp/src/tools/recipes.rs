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
    let recipes = engine.list_recipes(board).map_err(|e| e.to_string())?;
    let out: Vec<Json> = recipes
        .into_iter()
        .map(|r| json!({ "name": r.name, "enabled": r.enabled }))
        .collect();
    ok(json!({ "recipes": out }))
}

pub fn add(
    engine: &mut ServerlessEngine,
    _principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let name = arg_str(arguments, "name")?;
    let recipe = engine::model::Recipe {
        name: name.to_string(),
        when_json: arguments.get("when").cloned().unwrap_or(Json::Null),
        match_json: arguments.get("match").cloned(),
        enabled: arguments.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true),
        dedup_on: arguments.get("dedup_on").and_then(|d| d.as_str()).map(String::from),
        actions_json: arguments.get("actions").cloned(),
    };
    engine.add_recipe(board, &recipe).map_err(|e| e.to_string())?;
    ok(json!({ "name": name }))
}