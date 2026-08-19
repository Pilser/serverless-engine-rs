use std::sync::Arc;
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use zw_cache::CacheManager;
use zw_core::{McpToolProvider, ToolOutput};

#[derive(Serialize)]
pub struct RenderedFile {
    pub url: String,
    pub mime_type: String,
    pub file_size: i64,
    pub original_path: String,
    pub cached: bool,
}

pub struct WebFilesRenderProvider {
    cache: Arc<CacheManager>,
}

impl WebFilesRenderProvider {
    pub fn new(cache: Arc<CacheManager>) -> Self {
        Self { cache }
    }
}

#[async_trait]
impl McpToolProvider for WebFilesRenderProvider {
    fn name(&self) -> &str {
        "generate_file_urls"
    }

    fn description(&self) -> &str {
        "Generate HTTP URLs for local files (images, videos, PDFs, etc.) so they can be embedded inline in markdown responses, e.g. ![Alt Text](/files/hash)"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Array of absolute paths to files to render"
                }
            },
            "required": ["files"]
        })
    }

    async fn call(&self, args: Value) -> anyhow::Result<ToolOutput> {
        let paths = args.get("files")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing required argument: files"))?;

        let mut rendered = Vec::new();
        let mut skipped = Vec::new();

        for file_val in paths {
            let file_path = file_val
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Each file entry must be a string"))?;

            let path = std::path::Path::new(file_path);

            if !path.is_absolute() {
                skipped.push(format!(
                    "Skipped '{}': path must be absolute (e.g. /home/user/file.jpg)",
                    file_path
                ));
                rendered.push(RenderedFile {
                    url: String::new(),
                    mime_type: String::new(),
                    file_size: 0,
                    original_path: file_path.to_string(),
                    cached: false,
                });
                continue;
            }

            if !path.exists() {
                skipped.push(format!(
                    "Skipped '{}': file does not exist",
                    file_path
                ));
                rendered.push(RenderedFile {
                    url: String::new(),
                    mime_type: String::new(),
                    file_size: 0,
                    original_path: file_path.to_string(),
                    cached: false,
                });
                continue;
            }

            let cached = match self.cache.resolve(file_path) {
                Ok(c) => c,
                Err(e) => {
                    skipped.push(format!("Skipped '{}': {}", file_path, e));
                    rendered.push(RenderedFile {
                        url: String::new(),
                        mime_type: String::new(),
                        file_size: 0,
                        original_path: file_path.to_string(),
                        cached: false,
                    });
                    continue;
                }
            };

            rendered.push(RenderedFile {
                url: format!("/files/{}", cached.hash),
                mime_type: cached.meta.mime_type,
                file_size: cached.meta.file_size,
                original_path: String::new(),
                cached: true,
            });
        }

        let mut result = serde_json::json!({ "files": rendered });
        if !skipped.is_empty() {
            result["skipped"] = serde_json::json!(skipped);
        }

        Ok(ToolOutput::new(result.to_string()))
    }
}

pub struct GetFilePathProvider {
    cache: Arc<CacheManager>,
}

impl GetFilePathProvider {
    pub fn new(cache: Arc<CacheManager>) -> Self {
        Self { cache }
    }
}

#[async_trait]
impl McpToolProvider for GetFilePathProvider {
    fn name(&self) -> &str {
        "get_file_paths"
    }

    fn description(&self) -> &str {
        "Look up original file paths by cache hash. Accepts an array of hashes, returns file path + hash for each. Errors per hash don't affect others."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "hashes": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Array of cache hashes to look up"
                }
            },
            "required": ["hashes"]
        })
    }

    async fn call(&self, args: Value) -> anyhow::Result<ToolOutput> {
        let hashes = args.get("hashes")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing required argument: hashes"))?;

        let mut files = Vec::new();
        let mut errors = Vec::new();

        for hash_val in hashes {
            let hash = match hash_val.as_str() {
                Some(h) => h,
                None => {
                    errors.push("Entry is not a string".to_string());
                    continue;
                }
            };

            match self.cache.get_meta(hash) {
                Ok(Some(meta)) => {
                    files.push(serde_json::json!({
                        "hash": hash,
                        "original_path": meta.original_path
                    }));
                }
                Ok(None) => {
                    errors.push(format!("Hash '{}' not found in cache", hash));
                }
                Err(e) => {
                    errors.push(format!("Hash '{}': {}", hash, e));
                }
            }
        }

        let mut result = serde_json::json!({ "files": files });
        if !errors.is_empty() {
            result["errors"] = serde_json::json!(errors);
        }

        Ok(ToolOutput::new(result.to_string()))
    }
}
