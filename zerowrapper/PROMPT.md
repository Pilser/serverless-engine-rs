# Prompt for Implementation Agent

```markdown
Implement the zeroRapper project according to the architecture in ARCHITECTURE.md.
The zerowrapper/ folder already exists with the ARCHITECTURE.md file. Create all
crates, Cargo.toml files, and source code.

## Reference code in waos/

The waos/ folder in the same workspace has working examples of:

1. **Unix socket connection to zeroclaw**:
   - `waos/lib/src/bridge/zeroclaw/client.rs` — full ZeroClawClient using
     `tokio::net::UnixStream`. Connect, initialize, session/new, session/prompt
     with streaming notifications. Copy this pattern exactly for `zw-zeroclaw`.
   - `waos/lib/src/bridge/zeroclaw/protocol.rs` — JSON-RPC types matching
     zeroclaw's RPC protocol. Use these types directly.
   - `waos/lib/src/bridge/zeroclaw/agent.rs` — shows how to manage sessions,
     build session IDs per user, handle typing events, and relay messages.

2. **Turso database**:
   - Turso is at `../turso/bindings/rust/` in the workspace — a pure-Rust
     SQLite-compatible database engine. NOT libsql. The crate is called `turso`.
   - Usage: `turso::Builder::new_local(path).build().await?.connect()?`
   - Query: `conn.query("SELECT ...", params).await`
   - Params: `turso::params::Params::Positional(vec![turso::Value::from(...)])`
   - Rows: `rows.next().await` returns `Result<Option<Row>>`, `row.get_value(0)`
   - See `waos/lib/src/bridge/zeroclaw/agent.rs` lines 76-118 for example usage
     with sessions, contacts, and accounts tables.
   - See `waos/db/src/main_schema.rs` for schema pattern.
   - Add `turso = { workspace = true }` to the workspace Cargo.toml.
   - VERSION: hyper 0.14, not hyper 1.x

3. **Hyper HTTP server** (for ch-web channel):
   - `waos/bin/Cargo.toml` has `hyper = { version = "0.14", features = ["server", "http1", "tcp", "stream"] }`
   - Use hyper 0.14 for the web channel.
   - For WebSocket support with hyper 0.14, use `tokio-tungstenite` with
     `hyper` integration (`hyper-tungstenite` or manual upgrade).

## Workspace layout

Create this in zerowrapper/:

```
zerowrapper/
├── Cargo.toml                  # workspace root
├── config.toml
├── ARCHITECTURE.md            # already exists
│
├── src/
│   └── main.rs                # binary entry point
│
├── zw-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── config.rs
│       ├── error.rs
│       ├── types.rs
│       └── traits.rs
│
├── zw-db/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── migrations.rs
│       ├── users.rs
│       ├── sessions.rs
│       └── cache.rs
│
├── zw-cache/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       └── disk.rs
│
├── zw-mcp/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── protocol.rs
│       └── dispatch.rs
│
├── zw-channels/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── router.rs
│       └── zeroclaw_bridge.rs
│
└── crates/
    ├── mcp-render-file/
    │   ├── Cargo.toml
    │   └── src/lib.rs
    │
    └── ch-web/
        ├── Cargo.toml
        └── src/
            ├── lib.rs
            ├── cache_routes.rs
            ├── ws_handler.rs
            └── health.rs
```

## Workspace Cargo.toml

```toml
[workspace]
resolver = "2"
members = [
    "zw-core",
    "zw-db",
    "zw-cache",
    "zw-mcp",
    "zw-channels",
    "crates/mcp-render-file",
    "crates/ch-web",
]

[workspace.dependencies]
turso = { path = "../turso/bindings/rust" }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
uuid = { version = "1", features = ["v4"] }
chrono = "0.4"
hyper = { version = "0.14", features = ["server", "http1", "tcp", "stream"] }
hyper-tungstenite = "0.8"  # or use tungstenite directly with hyper upgrade
tokio-tungstenite = "0.20"
sha2 = "0.10"
base64 = "0.22"
mime_guess = "2"

[workspace.package]
edition = "2021"
```

## Traits to implement

### In zw-core/traits.rs:

```rust
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait McpToolProvider: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    async fn call(&self, args: Value) -> Result<ToolOutput>;
}

pub struct ToolOutput {
    pub text: String,
}

#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(self: Box<Self>, ctx: ChannelContext) -> Result<()>;
}

pub struct ChannelContext {
    pub registry: ChannelRegistryHandle,  // cloneable handle
    pub zeroclaw: ZeroClawHandle,         // cloneable handle
}

// Cloneable handles using mpsc channels internally
#[derive(Clone)]
pub struct ChannelRegistryHandle { tx: mpsc::Sender<ChannelMessage> }

#[derive(Clone)]
pub struct ZeroClawHandle { tx: mpsc::Sender<ZeroClawCommand> }

pub struct ChannelMessage {
    pub channel: String,
    pub session_id: SessionId,
    pub user_id: UserId,
    pub content: String,
    pub attachments: Vec<Attachment>,
}

pub enum ZeroClawCommand {
    Prompt {
        session_id: String,
        prompt: String,
        reply_tx: oneshot::Sender<Result<String>>,
    },
    NewSession {
        session_id: String,
        agent_alias: String,
        reply_tx: oneshot::Sender<Result<()>>,
    },
}
```

## MCP stdio protocol

The MCP server reads JSON-RPC from stdin, writes to stdout.
Reference: zeroclaw's own MCP implementation at
`zeroclaw/crates/zeroclaw-tools/src/mcp_protocol.rs` and `mcp_client.rs`.

```rust
// stdin/stdout frames:
// → {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"zeroclaw","version":"x"}}}
// ← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"zeroRapper","version":"0.1.0"}}}

// → {"jsonrpc":"2.0","id":2,"method":"notifications/initialized","params":{}}
// (no response — notification)

// → {"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}
// ← {"jsonrpc":"2.0","id":3,"result":{"tools":[{"name":"render_file_cached","description":"...","inputSchema":{...}}]}}

// → {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"render_file_cached","arguments":{"path":"...","session_id":"..."}}}
// ← {"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"..."}]}}
```

## ZeroClaw Unix RPC protocol

Connect to `<data_dir>/daemon.sock` via `tokio::net::UnixStream`.
Newline-delimited JSON-RPC 2.0. Same format as ACP but over Unix socket.

Reference: `waos/lib/src/bridge/zeroclaw/client.rs` for the exact protocol.

```rust
// Connect
let stream = UnixStream::connect(path).await?;
let (reader, writer) = stream.into_split();

// initialize
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol_version":1}}
← {"jsonrpc":"2.0","id":1,"result":{"protocol_version":1,"server_version":"x.y.z","capabilities":[]}}

// session/new
→ {"jsonrpc":"2.0","id":2,"method":"session/new","params":{"agent_alias":"myagent","session_id":"user_abc"}}
← {"jsonrpc":"2.0","id":2,"result":{"session_id":"user_abc","agent_alias":"myagent","message_count":0,"workspace_dir":"/tmp/..."}}

// session/prompt (streaming — reads notifications until TurnComplete)
→ {"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"session_id":"user_abc","prompt":"hello"}}
← {"jsonrpc":"2.0","method":"session/update","params":{"type":"agent_message_chunk","session_id":"user_abc","text":"Hel"}}
← {"jsonrpc":"2.0","method":"session/update","params":{"type":"tool_call","session_id":"user_abc","tool_call_id":"...","name":"render_file_cached","raw_input":{...}}}
← {"jsonrpc":"2.0","method":"session/update","params":{"type":"tool_result","session_id":"user_abc","tool_call_id":"...","name":"render_file_cached","raw_output":"..."}}
← {"jsonrpc":"2.0","method":"session/update","params":{"type":"agent_message_chunk","session_id":"user_abc","text":"lo..."}}
← {"jsonrpc":"2.0","id":3,"result":{"session_id":"user_abc","stop_reason":"completed","content":"Hello world"}}
```

NOTE: The prompt response (with matching id) arrives AFTER all notifications.
It's the final frame. Keep reading until you get a response with matching id.

## Database schema

Use turso (libsql-compatible) for all persistence. Create on startup:

```sql
CREATE TABLE IF NOT EXISTS users (
    id          TEXT PRIMARY KEY,
    external_id TEXT UNIQUE,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id),
    agent_alias TEXT NOT NULL,
    account     TEXT,
    chat_id     TEXT,
    channel     TEXT NOT NULL DEFAULT 'web',
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    last_active TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS cache_entries (
    session_id    TEXT NOT NULL,
    cache_key     TEXT NOT NULL,
    original_path TEXT NOT NULL,
    mime_type     TEXT NOT NULL,
    file_size     INTEGER NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    last_accessed TEXT,
    PRIMARY KEY (session_id, cache_key),
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);
```

## main.rs structure

```rust
#[tokio::main]
async fn main() -> Result<()> {
    // 1. Load config
    let config = WrapperConfig::load("config.toml")?;

    // 2. Open database
    let db = zw_db::DbPool::open(&config.db_url).await?;
    db.migrate().await?;

    // 3. Init cache
    let cache = Arc::new(zw_cache::CacheManager::new(&config.cache_dir, db.clone()));

    // 4. Create MCP registry
    let mut mcp = zw_mcp::McpRegistry::new();

    // 5. Register tools
    mcp.register_provider(Box::new(
        mcp_render_file::RenderFileProvider::new(cache.clone(), db.clone())
    ));

    // 6. Create channel registry + zeroclaw bridge
    let zeroclaw = zw_channels::ZeroClawBridge::new(&config.zeroclaw_socket, db.clone());
    let mut channels = zw_channels::ChannelRegistry::new(zeroclaw.clone());

    // 7. Register channels
    channels.register(Box::new(
        ch_web::WebChannel::new(&config.web, cache.clone())
    ));

    // 8. Start zeroclaw bridge
    let zc_handle = tokio::spawn(async move { zeroclaw.run().await });

    // 9. Start channel registry
    let ch_handle = tokio::spawn(async move { channels.run().await });

    // 10. Start MCP stdio loop (blocks until stdin closes)
    mcp.run_stdio().await?;

    Ok(())
}
```

## ch-web: hyper 0.14 HTTP/WS server

```rust
// lib.rs
pub struct WebChannel {
    bind: String,
    cache: Arc<CacheManager>,
    cache_dir: PathBuf,
}

impl WebChannel {
    pub fn new(cfg: &WebConfig, cache: Arc<CacheManager>) -> Self { ... }
}

// Implement Channel — start() spawns the hyper server
#[async_trait]
impl Channel for WebChannel {
    fn name(&self) -> &str { "web" }
    async fn start(self: Box<Self>, ctx: ChannelContext) -> Result<()> {
        let addr: SocketAddr = self.bind.parse()?;
        let make_svc = make_service_fn(|_| {
            let ctx = ctx.clone();
            let cache_dir = self.cache_dir.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    handle_request(req, ctx.clone(), cache_dir.clone())
                }))
            }
        });
        let server = Server::bind(&addr).serve(make_svc);
        server.await?;
        Ok(())
    }
}
```

Handle these routes:
- `GET /health` → 200 OK
- `GET /cache/{session_id}/{filename}` → read file from cache_dir/session_id/filename, return with correct MIME type
- `GET /ws` → upgrade to WebSocket via tungstenite, relay messages to ChannelRegistryHandle

For WebSocket upgrade with hyper 0.14 + tokio-tungstenite:

```rust
use tokio_tungstenite::accept_async;
use hyper::upgrade::Upgraded;
use hyper::body::HttpBody as _; // not needed for 0.14

// In the request handler, if it's a WS upgrade:
if req.headers().get("upgrade").and_then(|v| v.to_str().ok()) == Some("websocket") {
    match hyper::upgrade::on(req).await {
        Ok(upgraded) => {
            let ws = accept_async(upgraded).await?;
            // ws is a WebSocketStream<Upgraded>
            // Read messages and send to ctx.registry
            // Receive from ctx.registry and write to ws
        }
        Err(e) => ...
    }
}
```

## Important notes

1. **turso is NOT libsql.** It's the workspace-local crate at `../turso/bindings/rust`.
   Import as `turso`, use `turso::Builder::new_local(path)` not `libsql::Builder`.
   It uses `turso::Value` and `turso::params::Params`, NOT rusqlite or libsql types.

2. **The zeroclaw Unix RPC protocol** uses NEWLINE-DELIMITED JSON. Each frame
   is exactly one line. Not WebSocket frames, not HTTP — just lines on a Unix stream.

3. **stdin/stdout for MCP** must NOT print anything except JSON-RPC lines to stdout.
   All logging/debugging goes to stderr. Use `tracing` with a subscriber that writes to
   stderr, not stdout.

4. **Session ID format** for zeroclaw should follow the waos pattern:
   `{channel}.{agent_alias}.{account}_{chat_id}_{sender_jid}` sanitized.

5. **No HTTP in MCP.** The MCP transport is stdio only, not HTTP.
```
