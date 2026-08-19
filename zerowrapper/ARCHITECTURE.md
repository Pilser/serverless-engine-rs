# zeroRapper — Architecture

## Design principle: two central registries + pluggable crates

Everything plugs into one of two central registries via traits. No crate depends on another plugin crate — only on the registry it registers with.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         main.rs                                     │
│                                                                     │
│  1. Open database (zw-db)                                           │
│  2. Create MCP registry   (zw-mcp)                                  │
│  3. Create channel registry (zw-channels)                           │
│  4. Register tools:       mcp-render-file  →  zw-mcp                │
│  5. Register channels:    ch-web           →  zw-channels           │
│  6. Start zeroclaw client (zw-channels::zeroclaw_bridge)            │
│  7. Start MCP stdio loop  (zw-mcp)                                  │
│  8. Start all channels    (zw-channels)                             │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  all of the above run as tokio tasks in a single process      │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

## How zeroclaw spawns it

The `command` in zeroclaw's config must point to wherever you placed the `zw` binary:

```toml
# zeroclaw config.toml
[mcp.servers]
[mcp.servers.zeroRapper]
transport = "stdio"
command = "/home/user/bin/zw"   # path to this binary on your system
```

Zeroclaw spawns `zw` as a subprocess. Inside the single process:

```
zeroclaw ──stdio──→ zw (MCP stdin/stdout)
zeroclaw ←─sock──→ zw (Unix RPC client → daemon.sock)
user     ←─http──→ zw (web channel :9090)
```

## Crate overview

```
zerowrapper/
├── zw-core/            Foundation — traits + types + config
├── zw-db/              Database layer (libsql/turso)
├── zw-cache/           File cache engine (disk + db)
│
├── zw-mcp/             Central MCP registry (stdio server)
├── zw-channels/        Central channel registry + zeroclaw bridge
│
└── crates/
    ├── mcp-render-file/   MCP tool: render_file_cached
    ├── ch-web/            Channel: web frontend (hyper HTTP/WS)
    ├── ch-whatsapp/       Channel: WhatsApp (future)
    └── ch-email/          Channel: Email (future)
```

## Crate details

### zw-core — Foundation

```
zw-core/
  config.rs      # WrapperConfig — all sections (mcp, channels, db, cache)
  error.rs       # WrapperError enum
  types.rs       # SessionId, UserId, CacheEntry, ChannelMessage, FileHash
  traits.rs      # McpToolProvider trait + Channel trait
```

Key traits:

```rust
#[async_trait]
pub trait McpToolProvider: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    async fn call(&self, args: serde_json::Value) -> Result<ToolOutput>;
}

#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self, ctx: ChannelContext) -> Result<()>;
    async fn send(&self, session: &SessionId, msg: OutgoingMessage) -> Result<()>;
}
```

### zw-db — Central database

```
zw-db/
  lib.rs       # DbPool — shared libsql connection manager, migrate()
  users.rs     # users table CRUD
  sessions.rs  # sessions table CRUD
  cache.rs     # cache_entries table CRUD
```

Used by EVERY other crate that needs persistence. No crate talks to the database directly — only through `zw-db`.

```sql
-- users
CREATE TABLE users (
    id          TEXT PRIMARY KEY,
    external_id TEXT UNIQUE,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- sessions
CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id),
    agent_alias TEXT NOT NULL,
    account     TEXT,
    chat_id     TEXT,
    channel     TEXT NOT NULL DEFAULT 'web',
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    last_active TEXT NOT NULL DEFAULT (datetime('now'))
);

-- cache_entries
CREATE TABLE cache_entries (
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

### zw-cache — File cache engine

```
zw-cache/
  lib.rs    # CacheManager — store(), get(), list_by_session(), evict()
  disk.rs   # Content-addressed disk store
```

Standalone crate. Has no knowledge of MCP or channels. Just stores/retrieves files.

### zw-mcp — Central MCP registry (stdio server)

```
zw-mcp/
  lib.rs         # McpRegistry — register_provider(), run_stdio()
  protocol.rs    # JSON-RPC types (MCP spec: initialize, tools/list, tools/call)
  dispatch.rs    # Read stdin → route to registered provider → write stdout
```

How it works:
1. `run_stdio()` reads JSON-RPC lines from stdin
2. `initialize` → returns capabilities with tool list from all registered providers
3. `tools/list` → returns aggregated list from all providers
4. `tools/call` → dispatches to the matching `McpToolProvider::call()`

Registering a tool:
```rust
let mut mcp = McpRegistry::new();
mcp.register_provider(Box::new(RenderFileProvider::new(cache, db)));
mcp.run_stdio().await;  // reads stdin until closed
```

### zw-channels — Central channel registry + zeroclaw bridge

```
zw-channels/
  lib.rs             # ChannelRegistry — register_channel(), route_incoming()
  router.rs          # IncomingMessage → zeroclaw session, zeroclaw → channel send()
  zeroclaw_bridge.rs # ZeroClawClient (Unix RPC socket connection)
```

How it works:
1. Channels register with `ChannelRegistry`
2. When a channel receives an incoming message (web WS, WhatsApp, email):
   - Calls `registry.route_incoming(msg)`
   - Router finds/create zeroclaw session via `zeroclaw_bridge`
   - Sends `session/prompt` to zeroclaw
   - Streams response back to the channel
3. Outgoing messages from zeroclaw (tool results, agent text) go back through the same channel

### crates/mcp-render-file — MCP tool

```
mcp-render-file/
  lib.rs    # RenderFileProvider impl McpToolProvider
    1. reads file from disk
    2. stores in zw-cache
    3. records metadata in zw-db
    4. returns {"cache_key":"...","url":"/cache/..."}
```

### crates/ch-web — Web channel (hyper HTTP/WS)

```
ch-web/
  lib.rs         # WebChannel impl Channel — start() runs hyper server
  cache_routes.rs # GET /cache/{session}/{key}
  ws_handler.rs  # WebSocket for frontend users
  health.rs      # GET /health
```

- **hyper** (not axum) for HTTP
- `start()` is a tokio task that binds to `:9090`
- Forwards frontend WS messages to `ChannelRegistry::route_incoming()`
- Serves cached files from `zw-cache`

## Full flow: user sends message → agent replies

```
┌────────┐  1. WS message  ┌────────────┐  2. route_incoming()  ┌──────────────┐
│  User  │ ──────────────→ │  ch-web    │ ────────────────────→ │  zw-channels │
│ (browser)│               │  (hyper)   │                       │  router      │
└────────┘                 └────────────┘                       └──────┬───────┘
                                                                       │
                                                         3. session/prompt
                                                                       │
                                                                       ▼
                                                              ┌─────────────────┐
                                                              │ zeroclaw_bridge │
                                                              │ (Unix socket →  │
                                                              │  daemon.sock)   │
                                                              └────────┬────────┘
                                                                       │
                                                       4. tool_call → │ ← tool_result
                                                                       │
                                                                       ▼
                                                              ┌─────────────────┐
                                                              │   zw-mcp        │
                                                              │ (stdio ←→      │
                                                              │  zeroclaw)      │
                                                              └────────┬────────┘
                                                                       │
                                                            5. render_file_cached
                                                                       │
                                                                       ▼
                                                              ┌─────────────────┐
                                                              │ mcp-render-file │
                                                              │  → zw-cache     │
                                                              │  → zw-db        │
                                                              └─────────────────┘

  6. tool_result → zeroclaw → agent generates text
  7. agent text → zeroclaw_bridge → zw-channels → ch-web → user browser
```

## Dependency graph

```
main
  ├── zw-mcp ───────────┬── zw-core
  │    ↑                 │
  ├── mcp-render-file ───┤
  │    │                 │
  │    └── zw-cache ─────┤
  │         │            │
  │         └── zw-db ───┤
  │                      │
  ├── zw-channels ───────┤
  │    ↑                 │
  ├── ch-web ────────────┤
  │    │                 │
  │    └── zw-cache ─────┤
  │         │            │
  │         └── zw-db ───┤
  │                      │
  └── zw-db ─────────────┘
```

No crate depends on another plugin crate. Channels only know `zw-core` traits + `zw-channels`. MCP tools only know `zw-core` traits + `zw-mcp`.


```
zerowrapper/
├── Cargo.toml                  # [workspace]
│
├── zw-core/                    # Foundation — traits, types, config
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── config.rs           # WrapperConfig, ChannelConfig, McpToolConfig
│       ├── error.rs            # WrapperError
│       ├── types.rs            # SessionId, UserId, CacheEntry, ChannelMessage
│       └── traits.rs
│           ├── McpToolProvider  # trait for MCP tools to implement
│           └── Channel          # trait for channels to implement
│
├── zw-db/                      # Central database layer
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs              # DbPool, migrate()
│       ├── users.rs
│       ├── sessions.rs
│       └── cache_meta.rs
│
├── zw-cache/                   # File cache engine
│   ├── Cargo.toml              # deps: zw-core, zw-db
│   └── src/
│       ├── lib.rs              # CacheManager
│       └── disk.rs             # Content-addressed disk store
│
├── zw-mcp/                     # MCP stdio server + tool registry
│   ├── Cargo.toml              # deps: zw-core
│   └── src/
│       ├── lib.rs              # McpRegistry — register providers, run stdio loop
│       ├── protocol.rs         # JSON-RPC types (MCP spec)
│       ├── dispatch.rs         # Route stdin frames to registered providers
│       └── providers/
│           └── mod.rs          # Vec<Box<dyn McpToolProvider>>
│
├── zw-channels/                # Channel manager + registry
│   ├── Cargo.toml              # deps: zw-core, zw-db, zw-zeroclaw
│   └── src/
│       ├── lib.rs              # ChannelRegistry — register channels, route messages
│       ├── router.rs           # Incoming → zeroclaw, zeroclaw → channel
│       └── zeroclaw_bridge.rs  # Shared zeroclaw client bridge
│
├── crates/
│   ├── mcp-render-file/        # MCP tool: render_file_cached
│   │   ├── Cargo.toml          # deps: zw-core, zw-cache, zw-mcp
│   │   └── src/
│   │       └── lib.rs          # impl McpToolProvider
│   │
│   ├── ch-web/                 # Web frontend channel (hyper HTTP/WS)
│   │   ├── Cargo.toml          # deps: zw-core, zw-channels, hyper, tokio-tungstenite
│   │   └── src/
│   │       ├── lib.rs          # impl Channel — start http/ws server
│   │       ├── cache_routes.rs # GET /cache/{session}/{key}
│   │       ├── ws_handler.rs   # WebSocket for frontend users
│   │       └── health.rs       # GET /health
│   │
│   ├── ch-whatsapp/            # WhatsApp channel (future)
│   │   └── ...
│   │
│   └── ch-email/               # Email channel (future)
│       └── ...
│
├── src/
│   └── main.rs                 # Entry point:
│                                #   1. Open db (zw-db)
│                                #   2. Create mcp registry (zw-mcp)
│                                #   3. Register tools (mcp-render-file → zw-mcp)
│                                #   4. Create channel registry (zw-channels)
│                                #   5. Register channels (ch-web → zw-channels)
│                                #   6. Start zeroclaw RPC client
│                                #   7. Start MCP stdio loop
│                                #   8. Start all channels
│                                #   ─ all in tokio::select!
│
├── config.toml
└── ARCHITECTURE.md

```

## Adding a new MCP tool crate

```rust
// mcp-my-feature/src/lib.rs
pub struct MyFeatureProvider {
    db: DbPool,
}

#[async_trait]
impl McpToolProvider for MyFeatureProvider {
    fn name(&self) -> &str { "my_feature" }
    fn input_schema(&self) -> Value { json!({...}) }
    async fn call(&self, args: Value) -> Result<ToolOutput> { ... }
}

// In main.rs:
mcp.register_provider(Box::new(MyFeatureProvider::new(db.clone())));
```

## Adding a new channel crate

```rust
// ch-my-channel/src/lib.rs
pub struct MyChannel;

#[async_trait]
impl Channel for MyChannel {
    fn name(&self) -> &str { "my_channel" }
    async fn start(&self, ctx: ChannelContext) -> Result<()> {
        // ctx.registry — to route incoming messages
        // ctx.zeroclaw — to send prompts
        loop { /* receive messages */ }
    }
    async fn send(&self, session: &SessionId, msg: OutgoingMessage) -> Result<()> {
        /* send to your platform */
    }
}

// In main.rs:
channels.register_channel(Box::new(MyChannel));
```

## Config

```toml
[zerowrapper]
zeroclaw_socket = "/tmp/.zeroclaw/daemon.sock"
db_path = "./data/zerowrapper.db"
cache_dir = "./cache"

[zerowrapper.ch_web]
bind = "0.0.0.0:9090"
allowed_origins = ["*"]
agent = "craw"

# Future:
# [zerowrapper.ch_email]
# agent = "craw2"
```
