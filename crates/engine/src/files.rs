use crate::model::Principal;
use crate::storage::database::{Database, Query};
use crate::storage::ir::{FilterCond, Op, SrvFilter};
use crate::storage::object_store::{KeyInfo, ObjectStore};
use crate::tables::TABLE_RECORDS;
use serde_json::Value as Json;
use std::time::{SystemTime, UNIX_EPOCH};

fn file_id() -> String {
    const CHARS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut out = String::from("f_");
    let mut n = nanos;
    for _ in 0..16 {
        out.push(CHARS[(n % 36) as usize] as char);
        n /= 36;
    }
    out
}

pub fn file_upload(
    db: &mut dyn Database,
    store: &dyn ObjectStore,
    board_id: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
    meta: &Json,
) -> anyhow::Result<i64> {
    let key = format!("{board_id}/files/{}.bin", file_id());
    store.put(&key, bytes)?;
    let mut payload = serde_json::json!({
        "file": key,
        "name": filename,
        "type": content_type,
        "size": bytes.len(),
    });
    if let Some(obj) = meta.as_object() {
        if let Some(p) = payload.as_object_mut() {
            for (k, v) in obj {
                p.insert(k.clone(), v.clone());
            }
        }
    }
    let principal =
        Principal { id: board_id.to_string(), role: "owner".to_string(), scope: None, writer: None };
    crate::crud::record_insert(db, board_id, payload, None, false, &principal)
}

fn content_type_for(db: &dyn Database, board_id: &str, file: &str) -> anyhow::Result<String> {
    let q = Query {
        filter: SrvFilter {
            conds: vec![
                FilterCond {
                    field: "$.board_id".to_string(),
                    op: Op::Eq,
                    value: Json::String(board_id.to_string()),
                },
                FilterCond {
                    field: "$.payload.file".to_string(),
                    op: Op::Eq,
                    value: Json::String(file.to_string()),
                },
            ],
        },
        orders: vec![],
        limit: 1,
        offset: 0,
        ttl: None,
    };
    let Some(row) = db.query(TABLE_RECORDS, &q)?.rows.into_iter().next() else {
        return Ok("application/octet-stream".to_string());
    };
    let ct = row
        .data
        .pointer("/payload/type")
        .and_then(|v| v.as_str())
        .unwrap_or("application/octet-stream");
    Ok(ct.to_string())
}

pub fn file_download(
    db: &dyn Database,
    store: &dyn ObjectStore,
    board_id: &str,
    file: &str,
) -> anyhow::Result<Option<(Vec<u8>, String)>> {
    let prefix = format!("{board_id}/files/");
    if !file.starts_with(&prefix) {
        return Ok(None);
    }
    let Some(bytes) = store.get(file)? else {
        return Ok(None);
    };
    let content_type = content_type_for(db, board_id, file)?;
    Ok(Some((bytes, content_type)))
}

pub fn file_list(store: &dyn ObjectStore, board_id: &str) -> anyhow::Result<Vec<KeyInfo>> {
    store.list(&format!("{board_id}/files/"))
}

// ---- per-board static assets (front-end hosting) ----------------------

pub fn asset_prefix(board_id: &str) -> String {
    format!("{board_id}/assets/")
}

pub fn sanitize_asset_path(rel: &str) -> anyhow::Result<String> {
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() {
        anyhow::bail!("asset path is empty");
    }
    if rel.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        anyhow::bail!("asset path contains invalid segments");
    }
    if rel.starts_with('\\') || rel.contains('\\') {
        anyhow::bail!("asset path uses backslashes");
    }
    Ok(rel.to_string())
}

pub fn asset_key(board_id: &str, rel: &str) -> anyhow::Result<String> {
    let rel = sanitize_asset_path(rel)?;
    Ok(format!("{}{rel}", asset_prefix(board_id)))
}

pub fn content_type_from_path(rel: &str) -> &'static str {
    let lower = rel.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "map" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "xml" => "application/xml",
        "csv" => "text/csv; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub fn asset_put(
    store: &dyn ObjectStore,
    board_id: &str,
    rel: &str,
    bytes: &[u8],
) -> anyhow::Result<()> {
    let key = asset_key(board_id, rel)?;
    store.put(&key, bytes)?;
    Ok(())
}

pub fn asset_get(
    store: &dyn ObjectStore,
    board_id: &str,
    rel: &str,
) -> anyhow::Result<Option<(Vec<u8>, String)>> {
    let key = asset_key(board_id, rel)?;
    let Some(bytes) = store.get(&key)? else {
        return Ok(None);
    };
    Ok(Some((bytes, content_type_from_path(&key).to_string())))
}

pub fn asset_head(store: &dyn ObjectStore, board_id: &str, rel: &str) -> anyhow::Result<Option<crate::storage::object_store::BlobMeta>> {
    let key = asset_key(board_id, rel)?;
    store.head(&key)
}

pub fn asset_delete(store: &dyn ObjectStore, board_id: &str, rel: &str) -> anyhow::Result<bool> {
    let key = asset_key(board_id, rel)?;
    if store.head(&key)?.is_none() {
        return Ok(false);
    }
    store.delete(&key)?;
    Ok(true)
}

pub fn asset_list(store: &dyn ObjectStore, board_id: &str) -> anyhow::Result<Vec<KeyInfo>> {
    Ok(store
        .list(&asset_prefix(board_id))?
        .into_iter()
        .map(|k| KeyInfo { key: k.key.trim_start_matches(&asset_prefix(board_id)).to_string(), size: k.size })
        .collect())
}