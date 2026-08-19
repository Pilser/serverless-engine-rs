use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::layout::manifest_path;

#[derive(Serialize, Deserialize, Clone)]
pub struct Manifest {
    pub template_id: String,
    pub name: String,
    pub kind: String,
    pub description: String,
    pub version: i64,
    pub capabilities: Value,
    pub created_at: String,
}

pub fn load(root: &Path, tpl: &str, ver: i64) -> anyhow::Result<Manifest> {
    let path = manifest_path(root, tpl, ver);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("missing manifest for {tpl}@{ver}"))?;
    let manifest: Manifest = serde_json::from_str(&text)?;
    Ok(manifest)
}

pub fn save(root: &Path, tpl: &str, manifest: &Manifest) -> anyhow::Result<()> {
    let path = manifest_path(root, tpl, manifest.version);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(manifest)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}
