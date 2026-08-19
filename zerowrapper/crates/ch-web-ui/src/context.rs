use std::sync::Arc;
use zw_core::{ChannelRegistryHandle, ZeroClawHandle};
use zw_db::SessionStore;
use zw_db::rate::RateLimiter;
use zw_cache::CacheManager;
use crate::config::{SiteSettings, LandingData};

#[derive(Clone)]
pub struct AppContext {
    pub registry: ChannelRegistryHandle,
    pub zeroclaw: Arc<ZeroClawHandle>,
    pub db: SessionStore,
    pub cache: Arc<CacheManager>,
    pub rates: Arc<RateLimiter>,
    pub agent: String,
    pub upload_dir: std::path::PathBuf,
    pub config_dir: std::path::PathBuf,
    pub settings: SiteSettings,
    pub landing: LandingData,
    pub whiteboard: Option<zw_whiteboard::WhiteboardHandle>,
    pub materials: Option<zw_materials::TemplateStore>,
    pub mcp: Option<zw_mcp::dispatch::McpProviders>,
    pub realtime: std::sync::Arc<crate::realtime::Broker>,
}

impl AppContext {
    pub fn new(
        registry: ChannelRegistryHandle,
        zeroclaw: Arc<ZeroClawHandle>,
        db: SessionStore,
        cache: Arc<CacheManager>,
        agent: &str,
        upload_dir: std::path::PathBuf,
        config_dir: std::path::PathBuf,
        settings: SiteSettings,
        landing: LandingData,
    ) -> Self {
        Self {
            registry,
            zeroclaw,
            db,
            cache,
            rates: Arc::new(RateLimiter::default()),
            agent: agent.to_string(),
            upload_dir,
            config_dir,
            settings,
            landing,
            whiteboard: None,
            materials: None,
            mcp: None,
            realtime: std::sync::Arc::new(crate::realtime::Broker::new()),
        }
    }

    pub fn with_whiteboard(mut self, handle: zw_whiteboard::WhiteboardHandle) -> Self {
        self.whiteboard = Some(handle);
        self
    }

    pub fn with_mcp(mut self, providers: zw_mcp::dispatch::McpProviders) -> Self {
        self.mcp = Some(providers);
        self
    }

    pub fn with_materials(mut self, store: zw_materials::TemplateStore) -> Self {
        self.materials = Some(store);
        self
    }

    pub fn whiteboard(&self) -> Option<&zw_whiteboard::WhiteboardHandle> {
        self.whiteboard.as_ref()
    }

    pub fn materials(&self) -> Option<&zw_materials::TemplateStore> {
        self.materials.as_ref()
    }
}
