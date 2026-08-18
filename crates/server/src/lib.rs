pub mod broker;
pub mod config;
pub mod db;
pub mod http_caller;
pub mod identity;
pub mod jobs;
pub mod observability;
pub mod resources;
pub mod store;
pub mod transport;

use engine::policy::{RateLimiter, RateLimits};
use engine::storage::memory::{InMemoryDatabase, InMemoryObjectStore};
use engine::ServerlessEngine;
use std::sync::{Arc, Mutex};

pub use engine;

pub struct Server {
    pub engine: Arc<Mutex<ServerlessEngine>>,
    pub broker: Arc<broker::InProcBroker>,
    pub limiter: Arc<Mutex<RateLimiter>>,
    pub obs: Arc<observability::Obs>,
}

impl Clone for Server {
    fn clone(&self) -> Self {
        Self {
            engine: Arc::clone(&self.engine),
            broker: Arc::clone(&self.broker),
            limiter: Arc::clone(&self.limiter),
            obs: Arc::clone(&self.obs),
        }
    }
}

impl Server {
    pub fn new(mut engine: ServerlessEngine) -> Self {
        let broker = Arc::new(broker::InProcBroker::new());
        engine.set_notifier(Some(broker_notifier(broker.clone())));
        Self {
            engine: Arc::new(Mutex::new(engine)),
            broker,
            limiter: Arc::new(Mutex::new(RateLimiter::new())),
            obs: Arc::new(observability::Obs::new()),
        }
    }

    pub fn from_engine(engine: Arc<Mutex<ServerlessEngine>>) -> Self {
        let broker = Arc::new(broker::InProcBroker::new());
        if let Ok(mut e) = engine.lock() {
            e.set_notifier(Some(broker_notifier(broker.clone())));
        }
        Self {
            engine,
            broker,
            limiter: Arc::new(Mutex::new(RateLimiter::new())),
            obs: Arc::new(observability::Obs::new()),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(ServerlessEngine::new(
            Box::new(InMemoryDatabase::new()),
            Box::new(InMemoryObjectStore::new()),
        ))
    }

    pub fn spawn_background(&self) {
        // These loops are opt-in: each one periodically scans the whole store,
        // so idle deployments (no TTL tables, no cron jobs, no webhooks) should
        // leave them off. Enable via SRV_BG_TTL=1 / SRV_BG_JOBS=1 / SRV_BG_HOOKS=1.
        if env_flag("SRV_BG_TTL") {
            jobs::spawn_ttl_sweeper(self.engine.clone());
        }
        if env_flag("SRV_BG_JOBS") {
            jobs::spawn_scheduler(self.engine.clone());
        }
        if env_flag("SRV_BG_HOOKS") {
            jobs::spawn_webhook_worker(self.engine.clone());
        }
    }

    pub fn rate_limits_for(&self, board: &engine::Board) -> RateLimits {
        board
            .rate_json
            .as_ref()
            .map(|j| RateLimits::from_json(j))
            .unwrap_or_default()
    }
}

pub struct ServerBuilder {
    engine: Option<ServerlessEngine>,
}

impl ServerBuilder {
    pub fn new() -> Self {
        Self { engine: None }
    }

    pub fn engine(mut self, e: ServerlessEngine) -> Self {
        self.engine = Some(e);
        self
    }

    pub fn build(self) -> Server {
        match self.engine {
            Some(e) => Server::new(e),
            None => Server::with_defaults(),
        }
    }
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub fn engine_version() -> &'static str {
    engine::version()
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1").unwrap_or(false)
}

fn broker_notifier(broker: Arc<broker::InProcBroker>) -> engine::Notify {
    Arc::new(move |board, kind, seq, payload| {
        let event = serde_json::json!({
            "board": board,
            "type": format!("record.{kind}"),
            "seq": seq,
            "record": { "payload": payload },
        });
        let _ = broker.publish(&event.to_string());
    })
}

pub async fn serve(server: Server, addr: std::net::SocketAddr) -> anyhow::Result<()> {
    use hyper::service::service_fn;
    use hyper_util::rt::{TokioExecutor, TokioIo};

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("serverless engine listening on http://{addr}");
    loop {
        let (stream, _) = listener.accept().await?;
        let srv = Arc::new(server.clone());
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| {
                let srv = srv.clone();
                async move { transport::rest::handle(srv, req).await }
            });
            let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
            if let Err(err) = builder.serve_connection(io, service).await {
                tracing::warn!("connection error: {err}");
            }
        });
    }
}