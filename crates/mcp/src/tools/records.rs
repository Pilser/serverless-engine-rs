use super::{arg_i64, arg_str, ok};
use engine::model::Principal;
use engine::ServerlessEngine;
use serde_json::{json, Value as Json};

pub fn submit(
    engine: &mut ServerlessEngine,
    principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let payload = arguments
        .get("payload")
        .cloned()
        .ok_or_else(|| "missing argument 'payload'".to_string())?;
    let upsert = arguments.get("upsert").and_then(|u| u.as_bool()).unwrap_or(false);
    let seq = engine
        .insert_record(board, payload, Some(&principal.id), upsert, principal)
        .map_err(|e| e.to_string())?;
    ok(json!({ "seq": seq }))
}

pub fn get(
    engine: &ServerlessEngine,
    _principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let seq = arg_i64(arguments, "seq", 0);
    let record = engine
        .get_record(board, seq)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("record {seq} not found"))?;
    ok(serde_json::to_value(&record).map_err(|e| e.to_string())?)
}

pub fn list(
    engine: &ServerlessEngine,
    _principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let limit = arg_i64(arguments, "limit", 50).min(500).max(1) as usize;
    let offset = arg_i64(arguments, "offset", 0) as usize;
    let records = engine
        .list_records(board, limit, None, offset)
        .map_err(|e| e.to_string())?;
    let out: Vec<Json> = records
        .into_iter()
        .map(|r| json!({ "seq": r.seq, "payload": r.payload }))
        .collect();
    ok(json!({ "records": out }))
}

pub fn query(
    engine: &ServerlessEngine,
    _principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let limit = arg_i64(arguments, "limit", 50).min(500).max(1) as usize;
    let offset = arg_i64(arguments, "offset", 0) as usize;
    let filter_json = arguments.get("filter").cloned().unwrap_or(Json::Null);
    let filter = match filter_json {
        Json::Null => engine::storage::ir::SrvFilter::new(),
        other => engine::storage::ir::parse_filter(&other).map_err(|e| e.to_string())?,
    };
    let orders: Vec<(String, bool)> = arguments
        .get("order")
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| o.as_str().map(|s| (s.trim_start_matches('-').to_string(), s.starts_with('-'))))
                .collect()
        })
        .unwrap_or_default();
    let records = engine
        .query_records(board, &filter, &orders, limit, offset)
        .map_err(|e| e.to_string())?;
    let out: Vec<Json> = records
        .into_iter()
        .map(|r| json!({ "seq": r.seq, "payload": r.payload }))
        .collect();
    ok(json!({ "records": out }))
}

pub fn search(
    engine: &ServerlessEngine,
    _principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let q = arg_str(arguments, "q")?;
    let limit = arg_i64(arguments, "limit", 50).min(500).max(1) as usize;
    let offset = arg_i64(arguments, "offset", 0) as usize;
    let records = engine
        .search_records(board, q, &engine::storage::ir::SrvFilter::new(), limit, offset, false)
        .map_err(|e| e.to_string())?;
    let out: Vec<Json> = records
        .into_iter()
        .map(|r| json!({ "seq": r.seq, "payload": r.payload }))
        .collect();
    ok(json!({ "records": out }))
}

pub fn delete(
    engine: &mut ServerlessEngine,
    _principal: &Principal,
    arguments: &Json,
) -> Result<Json, String> {
    let board = arg_str(arguments, "board")?;
    let seq = arg_i64(arguments, "seq", 0);
    engine
        .delete_record(board, seq)
        .map_err(|e| e.to_string())?;
    ok(json!({ "seq": seq, "deleted": true }))
}

pub fn _unused(_: i64) {}