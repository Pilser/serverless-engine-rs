use std::path::Path;

use crate::{ParsedBoard, Pin};
use crate::parser::{extract_inline_images, ParsedMarkdown};

pub fn resolve_board(
    dir: &Path,
    parsed: &ParsedMarkdown,
    cache: &zw_cache::CacheManager,
) -> anyhow::Result<ParsedBoard> {
    let mut pin_paths = parsed.pin_paths.clone();
    pin_paths.extend(extract_inline_images(&parsed.body_md));

    let mut pins = Vec::new();
    let mut rewrites: Vec<(String, String)> = Vec::new();
    for path in &pin_paths {
        let abs = dir.join(path);
        let canonical = std::fs::canonicalize(&abs).unwrap_or(abs);
        if !canonical.is_file() {
            tracing::warn!("whiteboard pin missing: {}", path);
            continue;
        }
        let cached = cache.resolve(canonical.to_string_lossy().as_ref())?;
        let url = format!("?file={}", cached.hash);
        let name = canonical
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        pins.push(Pin {
            name: name.clone(),
            url: url.clone(),
            mime: cached.meta.mime_type.clone(),
            size: cached.meta.file_size.max(0) as u64,
        });
        rewrites.push((path.clone(), url));
    }

    let mut body_md = parsed.body_md.clone();
    for (path, url) in &rewrites {
        body_md = body_md.replace(&format!("({path})"), &format!("({url})"));
    }

    Ok(ParsedBoard { body_md, pins })
}
