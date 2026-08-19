pub mod diff;
pub mod layout;
pub mod manifest;
pub mod ops;
pub mod query;
pub mod snapshot;
pub mod store;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use zw_db::DbPool;
use zw_db::SessionStore;

#[derive(Clone)]
pub struct TemplateStore {
    templates: SessionStore,
    sessions: SessionStore,
    root: Arc<PathBuf>,
    sessions_root: Arc<PathBuf>,
    system: DbPool,
}

impl TemplateStore {
    pub fn new(whiteboard_dir: PathBuf, system: DbPool) -> Self {
        let root = layout::templates_root(&whiteboard_dir);
        let templates = SessionStore::new(root.clone(), system.clone());
        let sessions = SessionStore::new(whiteboard_dir.clone(), system.clone());
        Self {
            templates,
            sessions,
            root: Arc::new(root),
            sessions_root: Arc::new(whiteboard_dir),
            system,
        }
    }

    pub fn system(&self) -> &DbPool {
        &self.system
    }

    pub fn templates(&self) -> &SessionStore {
        &self.templates
    }

    pub fn sessions(&self) -> &SessionStore {
        &self.sessions
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn sessions_root(&self) -> &Path {
        &self.sessions_root
    }
}
