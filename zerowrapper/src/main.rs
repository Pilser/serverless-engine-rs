use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    let mcp_only = args.iter().any(|a| a == "--mcp");

    if mcp_only {
        let cache_dir = std::env::var("ZEROWRAPPER__CACHE_DIR")
            .unwrap_or_else(|_| "./cache".to_string());
        let cache_dir_path = PathBuf::from(&cache_dir);
        let cache = Arc::new(zw_cache::CacheManager::new(&cache_dir_path));

        let db_path = std::env::var("ZEROWRAPPER__DB_PATH").ok();

        let agents_dir = std::env::var("AGENTS_DIR").unwrap_or_else(|_| "agents".into());
        let agent = std::env::var("ZEROWRAPPER_CH_WEB__AGENT").unwrap_or_else(|_| "agent".into());
        let whiteboard_dir = std::path::PathBuf::from(&agents_dir).join(&agent).join("workspace");

        let mcp = zw_mcp::McpRegistry::new();
        if let Some(db_path) = db_path {
            let db = zw_db::DbPool::open(&db_path).await?;
            db.migrate().await?;
            let store = zw_db::SessionStore::new(whiteboard_dir.clone(), db.clone());
            let wb = zw_whiteboard::WhiteboardHandle::new(whiteboard_dir.clone(), cache.clone(), std::time::Duration::from_millis(500)).with_db(store);
            let materials = zw_materials::TemplateStore::new(whiteboard_dir, db);
            mcp.register_provider(Box::new(
                mcp_web_files_render::WebFilesRenderProvider::new(cache.clone())
            )).await;
            mcp.register_provider(Box::new(
                mcp_web_files_render::GetFilePathProvider::new(cache.clone())
            )).await;
            mcp.register_provider(Box::new(
                zw_mcp::manage_whiteboard::ManageWhiteboardProvider::new(wb)
            )).await;
            mcp.register_provider(Box::new(
                zw_mcp::manage_materials::ManageMaterialsProvider::new(materials)
            )).await;
        } else {
            let wb = zw_whiteboard::WhiteboardHandle::new(whiteboard_dir, cache.clone(), std::time::Duration::from_millis(500));
            tracing::warn!("ZEROWRAPPER__DB_PATH unset; manage_whiteboard will report 'db not configured'");
            mcp.register_provider(Box::new(
                mcp_web_files_render::WebFilesRenderProvider::new(cache.clone())
            )).await;
            mcp.register_provider(Box::new(
                mcp_web_files_render::GetFilePathProvider::new(cache.clone())
            )).await;
            mcp.register_provider(Box::new(
                zw_mcp::manage_whiteboard::ManageWhiteboardProvider::new(wb)
            )).await;
        }

        tracing::info!("MCP-only mode — cache_dir={}", cache_dir);
        mcp.run_stdio().await?;
        return Ok(());
    }

    let config = zw_core::WrapperConfig::load()?;
    let cache = Arc::new(zw_cache::CacheManager::new(&config.cache_dir));

    let db = zw_db::DbPool::open(&config.db_path).await?;
    db.migrate().await?;
    tracing::info!("Database migrated");

    let whiteboard_dir = config.ch_web.whiteboard_dir.clone().unwrap_or_else(|| {
        let agents_dir = std::env::var("AGENTS_DIR").unwrap_or_else(|_| "agents".into());
        std::path::PathBuf::from(agents_dir).join(&config.ch_web.agent).join("workspace")
    });
    tracing::info!("whiteboard dir: {}", whiteboard_dir.display());

    let store = zw_db::SessionStore::new(whiteboard_dir.clone(), db.clone());
    let whiteboard = zw_whiteboard::WhiteboardHandle::new(whiteboard_dir.clone(), cache.clone(), std::time::Duration::from_millis(500)).with_db(store.clone());
    let _wb_task = whiteboard.spawn();

    let mcp = zw_mcp::McpRegistry::new();
    mcp.register_provider(Box::new(
        mcp_web_files_render::WebFilesRenderProvider::new(cache.clone())
    )).await;
    mcp.register_provider(Box::new(
        mcp_web_files_render::GetFilePathProvider::new(cache.clone())
    )).await;
    mcp.register_provider(Box::new(
        zw_mcp::manage_whiteboard::ManageWhiteboardProvider::new(whiteboard.clone())
    )).await;
    let materials = zw_materials::TemplateStore::new(whiteboard_dir.clone(), db.clone());
    mcp.register_provider(Box::new(
        zw_mcp::manage_materials::ManageMaterialsProvider::new(Clone::clone(&materials))
    )).await;

    if config.mcp.is_none() {
        tracing::info!("No [zerowrapper.mcp] configured (only needed if zeroclaw daemon needs to discover this server)");
    }

    tracing::info!("Config loaded: zeroclaw_socket={}, db={}, web_channel={{{}}}",
        config.zeroclaw_socket, config.db_path, config.ch_web.bind);

    if !config.ch_web.accounts.is_empty() {
        let seeds: std::collections::HashMap<String, zw_db::AccountSeed> = config.ch_web.accounts.iter().map(|(k, v)| {
            (k.clone(), zw_db::AccountSeed { role: v.role.clone(), password: v.password.clone() })
        }).collect();
        db.seed_accounts(&seeds).await?;
    }

    let mut zeroclaw = zw_channels::ZeroClawBridge::new(&config.zeroclaw_socket, db.clone());
    zeroclaw.set_whiteboard(whiteboard.clone());
    let channels = zw_channels::ChannelRegistry::new(zeroclaw.clone());

    let primary_bind = config.ch_web.bind.clone();

    let registry_handle = channels.registry_handle();
    let zeroclaw_handle = Arc::new(channels.zeroclaw_handle());

    let landing_path = std::env::var("ZEROWRAPPER_LANDING_YAML")
        .unwrap_or_else(|_| "./landing.yaml".to_string());
    let settings_path = std::env::var("ZEROWRAPPER_SETTINGS_YAML")
        .unwrap_or_else(|_| "./sitesettings.yaml".to_string());
    let config_dir = std::env::var("ZEROWRAPPER_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("config"));
    let settings = ch_web_ui::config::load_site_settings(&settings_path)
        .unwrap_or_else(|e| {
            tracing::warn!("Failed to load {}: {e}", settings_path);
            ch_web_ui::config::SiteSettings {
                site: ch_web_ui::config::SiteInfo {
                    title: "Jerboa".into(),
                    brand: None,
                    url: "http://localhost:9092".into(),
                    local_url: None,
                    tagline: "AI agents".into(),
                    description: "Jerboa AI agents.".into(),
                    favicon: None,
                },
                footer: ch_web_ui::config::FooterConfig {
                    text: "© Jerboa".into(),
                    links: vec![],
                },
                business: None,
            }
        });
    let landing = ch_web_ui::config::load_landing(&landing_path)
        .unwrap_or_else(|e| {
            tracing::warn!("Failed to load {}: {e}", landing_path);
            ch_web_ui::config::LandingData {
                hero: ch_web_ui::config::HeroSection {
                    title: "Jerboa".into(),
                    subtitle: Some("AI agents".into()),
                    description: "Jerboa AI agents.".into(),
                    fallback: "Backend not connected.".into(),
                },
                examples: vec![],
                faq: vec![],
                about: ch_web_ui::config::AboutSection {
                    heading: "Jerboa".into(),
                    text: String::new(),
                },
            }
        });

    let ui_ctx = ch_web_ui::context::AppContext::new(
        registry_handle,
        zeroclaw_handle,
        store.clone(),
        cache.clone(),
        &config.ch_web.agent,
        config.ch_web.upload_dir.clone(),
        config_dir,
        settings,
        landing,
    )
    .with_whiteboard(whiteboard.clone())
    .with_materials(materials)
    .with_mcp(mcp.providers());
    let _webhook_worker = ch_web_ui::webhooks::spawn_webhook_worker(store.clone());
    let _ttl_worker = ch_web_ui::ttl_sweeper::spawn_ttl_sweeper(store.clone(), std::time::Duration::from_secs(30));
    let _scheduler = ch_web_ui::scheduler::spawn_scheduler(store.clone());
    let ui_handle = tokio::spawn(async move {
        if let Err(e) = ch_web_ui::app::start_on(primary_bind, ui_ctx).await {
            tracing::error!("ch-web-ui exited: {}", e);
        }
    });

    let zc_handle = tokio::spawn(async move {
        if let Err(e) = zeroclaw.run().await {
            tracing::error!("ZeroClaw bridge exited: {}", e);
        }
    });

    let ch_handle = tokio::spawn(async move {
        if let Err(e) = channels.run().await {
            tracing::error!("Channel registry exited: {}", e);
        }
    });

    tracing::info!("Starting MCP stdio loop");
    mcp.run_stdio().await?;

    let _ = zc_handle.await;
    let _ = ch_handle.await;
    let _ = ui_handle.await;

    Ok(())
}
