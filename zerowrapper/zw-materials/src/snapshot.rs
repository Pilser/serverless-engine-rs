use std::path::Path;

use anyhow::bail;
use serde_json::{json, Value};
use zw_db::DbPool;

pub struct SnapshotReport {
    pub records: i64,
    pub files: usize,
    pub hooks: usize,
    pub capabilities: Value,
}

pub async fn snapshot_board(
    src: &DbPool,
    src_board: &str,
    src_files_root: &Path,
    dst: &DbPool,
    dst_board: &str,
    dst_owner: &str,
    dst_files_dir: &Path,
) -> anyhow::Result<SnapshotReport> {
    let app = match src.srv_app_by_id(src_board).await? {
        Some(app) => app,
        None => bail!("source board {src_board} not found"),
    };

    if dst.srv_app_by_id(dst_board).await?.is_some() {
        dst.srv_app_update(
            dst_board,
            dst_owner,
            Some(&app.title),
            Some(app.schema_json.as_deref()),
            Some(app.public_reads),
            Some(app.unique_key.as_deref()),
        )
        .await?;
    } else {
        dst.srv_app_create_with_id(
            dst_board,
            dst_owner,
            &app.title,
            app.schema_json.as_deref(),
            app.public_reads,
            app.unique_key.as_deref(),
        )
        .await?;
    }

    let rate = src.srv_rate_config(src_board).await?;
    dst.srv_rate_set(dst_board, dst_owner, Some(&serde_json::to_string(&rate)?))
        .await?;

    let ttl = src.srv_ttl_get(src_board).await?;
    dst.srv_ttl_set(dst_board, dst_owner, ttl.seconds, ttl.field.as_deref())
        .await?;

    let audit_on = src.srv_audit_enabled(src_board).await?;
    dst.srv_app_audit_set(dst_board, dst_owner, audit_on).await?;

    let hooks = src.srv_hook_urls(src_board).await?;
    for (url, _has_secret) in &hooks {
        dst.srv_hook_register(dst_board, dst_owner, url, None).await?;
    }

    let mut records = 0i64;
    loop {
        let batch = src.srv_record_list_raw(src_board, 500, None, records).await?;
        let n = batch.len();
        for r in &batch {
            dst.srv_record_insert(dst_board, &r.payload, r.writer.as_deref(), false)
                .await?;
        }
        records += n as i64;
        if n < 500 {
            break;
        }
    }

    let files = copy_tree(src_files_root, dst_files_dir)?;

    let capabilities = json!({
        "schema": app.schema_json.is_some(),
        "unique_key": app.unique_key.clone().unwrap_or_default(),
        "public_reads": app.public_reads,
        "rate": rate,
        "ttl": { "seconds": ttl.seconds, "field": ttl.field },
        "hooks": hooks.len(),
        "files": files,
        "records": records,
    });

    Ok(SnapshotReport {
        records,
        files,
        hooks: hooks.len(),
        capabilities,
    })
}

pub fn copy_tree(src: &Path, dst: &Path) -> anyhow::Result<usize> {
    if !src.is_dir() {
        return Ok(0);
    }
    std::fs::create_dir_all(dst)?;
    let mut count = 0;
    let mut stack = vec![src.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(src)?;
            let target = dst.join(rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &target)?;
            count += 1;
        }
    }
    Ok(count)
}

pub fn remove_tree(path: &Path) -> anyhow::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}
