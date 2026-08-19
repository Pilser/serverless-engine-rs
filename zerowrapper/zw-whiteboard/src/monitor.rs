use std::collections::HashMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::ParsedBoard;

#[derive(Default)]
pub struct MonitorCore {
    pub seen: HashMap<String, FileSnapshot>,
    pub rev: HashMap<String, i64>,
    pub latest: HashMap<String, ParsedBoard>,
}

#[derive(Debug, Clone)]
pub struct FileSnapshot {
    pub hash: String,
}

impl MonitorCore {
    /// Hash the file contents (sha256). An empty hash means the file is absent
    /// or unreadable, which is treated as a distinct state from any real content.
    pub fn snapshot(&self, path: &Path) -> FileSnapshot {
        let hash = match std::fs::read(path) {
            Ok(bytes) => {
                let mut h = Sha256::new();
                h.update(&bytes);
                format!("{:x}", h.finalize())
            }
            Err(_) => String::new(),
        };
        FileSnapshot { hash }
    }

    /// Returns true when the file's content hash differs from the last seen snapshot,
    /// updating `seen` on change. Content hashing avoids missing edits that happen to
    /// preserve mtime/size (e.g. `cp --preserve`, restores, same-length rewrites).
    pub fn has_changed(&mut self, sid: &str, path: &Path) -> bool {
        let current = self.snapshot(path);
        let changed = match self.seen.get(sid) {
            Some(prev) => prev.hash != current.hash,
            None => true,
        };
        if changed {
            self.seen.insert(sid.to_string(), current);
        }
        changed
    }
}
