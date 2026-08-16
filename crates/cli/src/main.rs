mod client;
mod registry;
mod render;
mod studio;


use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> anyhow::Result<String> {
    let daemon = daemon_url();
    let specs = registry::load();

    if args.is_empty() || args[0] == "--help" || args[0] == "help" {
        return Ok(registry::help_index(&specs));
    }
    if args[0] == "about" {
        return Ok(registry::about());
    }
    if args[0] == "--version" || args[0] == "version" {
        return Ok(format!("srv {} — serverless engine CLI\n", server::engine_version()));
    }
    if args[0] == "daemon" {
        return cmd_daemon(&args[1..]);
    }
    if args[0] == "studio" {
        return studio::run(&daemon);
    }
    if args[0] == "reference" {
        return Ok(registry::reference(&specs));
    }

    let Some(group) = args.first() else {
        return Ok(registry::help_index(&specs));
    };
    if args.get(1).map(|s| s.as_str()) == Some("--help") {
        return Ok(registry::group_help(&specs, group));
    }
    let Some(verb) = args.get(1) else {
        return Ok(registry::group_help(&specs, group));
    };
    if verb == "--help" {
        return Ok(registry::group_help(&specs, group));
    }
    if args.get(2).map(|s| s.as_str()) == Some("--help") {
        let name = format!("{group}.{verb}");
        return Ok(registry::verb_help(&specs, &name));
    }

    let tail = &args[2..];
    match registry::parse_call(&specs, group, verb, tail) {
        Ok(call) => {
            let rt = tokio::runtime::Runtime::new().map_err(|e| anyhow::anyhow!("{e}"))?;
            let out = rt.block_on(client::call(&daemon, &call))?;
            Ok(render::render(&call, &out))
        }
        Err(e) => Ok(registry::error_text(&specs, group, verb, &e)),
    }
}

fn cmd_daemon(args: &[String]) -> anyhow::Result<String> {
    match args.first().map(|s| s.as_str()) {
        Some("start") => {
            let cfg = server::config::Config::load();
            let data_dir = args_flag(&args[1..], "--db")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| cfg.data_dir.clone());
            let backend = if cfg.backend == "memory" { "memory" } else { "turso" };
            let database: Box<dyn engine::storage::database::Database> =
                Box::new(engine::storage::cache::CachedDatabase::new(backend_impl(
                    backend, &data_dir,
                )));
            let object_store: Box<dyn engine::storage::object_store::ObjectStore> =
                Box::new(engine::storage::cache::CachedObjectStore::new(Box::new(
                    server::store::fs::FsObjectStore::new(data_dir.join("objects")),
                )));
            let engine = std::sync::Arc::new(std::sync::Mutex::new(
                engine::ServerlessEngine::new(database, object_store),
            ));
            let caller = server::http_caller::ReqwestCaller::new()?;
            engine::http::install(Box::new(caller));
            let server = server::Server::from_engine(engine);
            let report_server = server.clone();
            let mcp = std::sync::Arc::new(
                mcp::McpServer::new(server.engine.clone()).with_reporter(std::sync::Arc::new(
                    move |board| server::resources::resource_report(&report_server, board),
                )),
            );
            let addr = cfg.addr();
            println!(
                "daemon listening on http://{addr}/mcp (backend: {backend}, db: {})",
                data_dir.display()
            );
            let rt = tokio::runtime::Runtime::new().map_err(|e| anyhow::anyhow!("{e}"))?;
            rt.block_on(async {
                server.spawn_background();
            });
            rt.block_on(serve_combined(server, mcp, addr)).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(String::new())
        }
        other => Ok(format!(
            "usage: srv daemon start [--db <dir>]\n\
             note: daemon hosts the engine + MCP over HTTP; other surfaces connect to it.\n\
             --db <dir>   data directory (default ./data; or $SRV_DATA_DIR / .env)\n\
             env: SRV_HOST SRV_PORT SRV_DATA_DIR SRV_DB SRV_HTTP_TIMEOUT_MS (see .env.example)\n\
             got: '{:?}'\n",
            other
        )),
    }
}

fn backend_impl(backend: &str, data_dir: &std::path::Path) -> Box<dyn engine::storage::database::Database> {
    if backend == "memory" {
        Box::new(engine::storage::memory::InMemoryDatabase::new())
    } else {
        Box::new(server::db::turso::TursoDatabase::new(data_dir.to_path_buf()))
    }
}

fn args_flag(args: &[String], flag: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == flag {
            return it.next().cloned();
        }
    }
    None
}

async fn serve_combined(
    server: server::Server,
    mcp: std::sync::Arc<mcp::McpServer>,
    addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    use hyper::service::service_fn;
    use hyper_util::rt::{TokioExecutor, TokioIo};

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("serverless engine + mcp listening on http://{addr}");
    loop {
        let (stream, _) = listener.accept().await?;
        let srv = std::sync::Arc::new(server.clone());
        let mcp = mcp.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| {
                let srv = srv.clone();
                let mcp = mcp.clone();
                let path = req.uri().path().to_string();
                async move {
                    if path == "/mcp" {
                        Ok::<_, std::io::Error>(mcp::transport::handle_hyper(mcp, req).await)
                    } else if server::transport::ws::is_ws(&req) {
                        Ok::<_, std::io::Error>(server::transport::ws::ws_upgrade(srv.clone(), req))
                    } else {
                        server::transport::rest::handle(srv, req)
                            .await
                            .map_err(|never| match never {})
                    }
                }
            });
            let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
            if let Err(err) = builder.serve_connection_with_upgrades(io, service).await {
                tracing::warn!("connection error: {err}");
            }
        });
    }
}

fn daemon_url() -> String {
    std::env::var("SERVERLESS_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:7070".to_string())
}