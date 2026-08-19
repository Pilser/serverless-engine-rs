use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};
use zw_db::DbPool;

pub async fn build_diff(
    a: &DbPool,
    a_board: &str,
    a_files: &Path,
    b: &DbPool,
    b_board: &str,
    b_files: &Path,
) -> Result<Value> {
    let a_app = a.srv_app_by_id(a_board).await?;
    let b_app = b.srv_app_by_id(b_board).await?;

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    if let (Some(fa), Some(fb)) = (file_map(a_files)?, file_map(b_files)?) {
        for (p, s) in &fb {
            match fa.get(p) {
                None => added.push(p.clone()),
                Some(prev) if prev != s => changed.push(p.clone()),
                _ => {}
            }
        }
        for p in fa.keys() {
            if !fb.contains_key(p) {
                removed.push(p.clone());
            }
        }
    }
    added.sort();
    removed.sort();
    changed.sort();

    let a_seqs = seq_set(a, a_board).await?;
    let b_seqs = seq_set(b, b_board).await?;
    let data = json!({
        "added_records": b_seqs.difference(&a_seqs).count(),
        "removed_records": a_seqs.difference(&b_seqs).count(),
    });

    let a_cfg = app_cfg(&a_app);
    let b_cfg = app_cfg(&b_app);
    let a_rate = match a.srv_rate_config(a_board).await {
        Ok(cfg) => serde_json::to_string(&cfg).unwrap_or_default(),
        Err(_) => String::new(),
    };
    let b_rate = match b.srv_rate_config(b_board).await {
        Ok(cfg) => serde_json::to_string(&cfg).unwrap_or_default(),
        Err(_) => String::new(),
    };
    let a_ttl = match a.srv_ttl_get(a_board).await {
        Ok(ttl) => format!("{:?}|{:?}", ttl.seconds, ttl.field),
        Err(_) => String::new(),
    };
    let b_ttl = match b.srv_ttl_get(b_board).await {
        Ok(ttl) => format!("{:?}|{:?}", ttl.seconds, ttl.field),
        Err(_) => String::new(),
    };

    Ok(json!({
        "files": { "added": added, "removed": removed, "changed": changed },
        "config": {
            "schema": same_or_changed(a_cfg.schema, b_cfg.schema),
            "public_reads": same_or_changed(a_cfg.public_reads, b_cfg.public_reads),
            "unique_key": same_or_changed(a_cfg.unique_key, b_cfg.unique_key),
            "rate": same_or_changed(a_rate, b_rate),
            "ttl": same_or_changed(a_ttl, b_ttl),
        },
        "data": data,
    }))
}

struct AppCfg {
    schema: String,
    public_reads: String,
    unique_key: String,
}

fn app_cfg(app: &Option<zw_db::serverless::SrvApp>) -> AppCfg {
    match app {
        Some(app) => AppCfg {
            schema: app.schema_json.clone().unwrap_or_default(),
            public_reads: app.public_reads.to_string(),
            unique_key: app.unique_key.clone().unwrap_or_default(),
        },
        None => AppCfg {
            schema: String::new(),
            public_reads: String::new(),
            unique_key: String::new(),
        },
    }
}

fn same_or_changed(a: String, b: String) -> &'static str {
    if a == b { "same" } else { "changed" }
}

pub fn human_summary(diff: &Value) -> String {
    let empty = json!({});
    let files = diff.get("files").unwrap_or(&empty);
    let data = diff.get("data").unwrap_or(&empty);
    let added = files.get("added").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    let removed = files.get("removed").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    let changed = files.get("changed").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    let added_rec = data.get("added_records").and_then(|v| v.as_i64()).unwrap_or(0);
    let removed_rec = data.get("removed_records").and_then(|v| v.as_i64()).unwrap_or(0);
    format!("files: +{added} -{removed} ~{changed}; records: +{added_rec} -{removed_rec}")
}

fn file_map(root: &Path) -> Result<Option<BTreeMap<String, u64>>> {
    if !root.is_dir() {
        return Ok(None);
    }
    let mut map = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(root)?.to_string_lossy().to_string();
            map.insert(rel, entry.metadata()?.len());
        }
    }
    Ok(Some(map))
}

async fn seq_set(pool: &DbPool, board: &str) -> Result<BTreeSet<i64>> {
    let mut set = BTreeSet::new();
    let mut offset = 0i64;
    loop {
        let batch = pool.srv_record_list_raw(board, 2000, None, offset).await?;
        let n = batch.len();
        for r in &batch {
            set.insert(r.seq);
        }
        offset += n as i64;
        if n < 2000 {
            break;
        }
    }
    Ok(set)
}
