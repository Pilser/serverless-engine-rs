use sha2::{Digest, Sha256};
use std::path::PathBuf;
use crate::FileMeta;

/// Hash a path string to produce a stable cache key.
/// SHA-256 of the path bytes, truncated to 8 bytes, URL-safe base64 encoded (11 chars).
pub fn hash_path(path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    let result = hasher.finalize();
    let truncated = &result[..8];
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, truncated)
}

fn meta_path(cache_dir: &PathBuf, hash: &str) -> PathBuf {
    let prefix = &hash[..2.min(hash.len())];
    let dir = cache_dir.join("meta").join(prefix);
    dir.join(hash)
}

pub fn write_meta(cache_dir: &PathBuf, hash: &str, meta: &FileMeta) -> anyhow::Result<()> {
    let path = meta_path(cache_dir, hash);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(meta)?;
    std::fs::write(&path, &json)?;
    Ok(())
}

pub fn read_meta(cache_dir: &PathBuf, hash: &str) -> anyhow::Result<Option<FileMeta>> {
    let path = meta_path(cache_dir, hash);
    if path.exists() {
        let json = std::fs::read_to_string(&path)?;
        let meta: FileMeta = serde_json::from_str(&json)?;
        Ok(Some(meta))
    } else {
        Ok(None)
    }
}
