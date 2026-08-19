pub mod disk;

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    pub mime_type: String,
    pub file_size: i64,
    pub original_path: String,
}

pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    pub fn new(cache_dir: &PathBuf) -> Self {
        std::fs::create_dir_all(cache_dir).ok();
        Self {
            cache_dir: cache_dir.clone(),
        }
    }

    /// Get or create metadata for a file path. Returns the cache hash.
    /// If metadata already exists for this path, returns cached version.
    pub fn resolve(&self, original_path: &str) -> anyhow::Result<CachedFile> {
        let hash = disk::hash_path(original_path);

        if let Some(meta) = disk::read_meta(&self.cache_dir, &hash)? {
            if meta.original_path == original_path {
                return Ok(CachedFile { hash, meta });
            }
        }

        let path = std::path::Path::new(original_path);
        let mime_type = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        let file_size = std::fs::metadata(path)?.len() as i64;

        let meta = FileMeta {
            mime_type,
            file_size,
            original_path: original_path.to_string(),
        };
        disk::write_meta(&self.cache_dir, &hash, &meta)?;

        Ok(CachedFile { hash, meta })
    }

    /// Read metadata for a cached hash.
    pub fn get_meta(&self, hash: &str) -> anyhow::Result<Option<FileMeta>> {
        disk::read_meta(&self.cache_dir, hash)
    }

    /// Get file size of the original file (fresh stat, not cached).
    pub fn current_size(&self, meta: &FileMeta) -> i64 {
        std::fs::metadata(&meta.original_path)
            .map(|m| m.len() as i64)
            .unwrap_or(meta.file_size)
    }

    pub fn cache_dir(&self) -> &PathBuf {
        &self.cache_dir
    }
}

pub struct CachedFile {
    pub hash: String,
    pub meta: FileMeta,
}
