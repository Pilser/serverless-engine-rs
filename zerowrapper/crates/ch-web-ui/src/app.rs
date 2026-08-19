use std::str::FromStr;
use std::path::Path;
use tokio::net::TcpListener;
use topcoat::router::{Router, Path as RoutPath, RouterBuilderDiscoverExt, tower::TowerLayer};
use topcoat::asset::{AssetBundle, RouterBuilderAssetExt};

use crate::context::AppContext;

fn load_assets() -> AssetBundle {
    AssetBundle::load().unwrap_or_else(|_| {
        let dir = std::env::var("TOPCOAT_ASSETS_DIR")
            .unwrap_or_else(|_| "assets".to_string());
        AssetBundle::load_dir(Path::new(&dir)).unwrap_or_else(|_| {
            let tmp = std::env::temp_dir().join("zw-assets");
            let _ = std::fs::create_dir_all(&tmp);
            let _ = std::fs::write(tmp.join("manifest.toml"), b"version = 1\n");
            AssetBundle::load_dir(&tmp).expect("failed to create temp asset bundle")
        })
    })
}

pub fn build_router(ctx: AppContext) -> Router {
    let cors = tower_http::cors::CorsLayer::permissive();
    Router::builder()
        .app_context(ctx)
        .assets(load_assets())
        .layer(crate::routes::root_dispatch::RootDispatchLayer)
        .layer(TowerLayer::new(RoutPath::new("/api"), cors))
        .layer(TowerLayer::new(RoutPath::new("/"), crate::cache::NoStoreHtmlLayer))
        .discover()
        .build()
}

pub async fn start(ctx: AppContext) {
    let router = build_router(ctx);
    topcoat::start(router).await.unwrap();
}

pub async fn start_on(bind: impl AsRef<str>, ctx: AppContext) -> std::io::Result<()> {
    let addr = std::net::SocketAddr::from_str(bind.as_ref())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let listener = TcpListener::bind(addr).await?;
    let router = build_router(ctx);
    topcoat::serve(listener, router).await
}
