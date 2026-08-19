# ZeroClaw instance template for zerowrapper

Make ZeroClaw easy to set up and run per-project, **without touching the default
`~/.zeroclaw`** and **without using its web UI to configure models / MCP / agents /
workspace**.

The reference implementation lives in `../zeroclaw/` (cloned from
`https://github.com/zeroclaw-labs/zeroclaw`). This folder provides:

| File | Purpose |
|---|---|
| `config.toml.example` | Full, commented ZeroClaw config template (`schema_version = 3`) |
| `setup.sh` | One-shot scaffolder: builds a custom config dir + workspace + launcher |
| `README.md` | This doc |

---

## 1. How the zeroclaw binary runs in a custom path

ZeroClaw is a single Rust binary. Its config file and all runtime state are
anchored to a **config dir** that is resolved in this order:

1. Global CLI flag — `zeroclaw --config-dir <dir> daemon`
2. Env var — `ZEROCLAW_CONFIG_DIR=<dir> zeroclaw daemon`
3. Default — `~/.zeroclaw/config.toml`

Relevant env vars (see `crates/zeroclaw-runtime/src/rpc/local.rs` and
`crates/zeroclaw-config/src/schema.rs`):

| Var | Meaning | Default |
|---|---|---|
| `ZEROCLAW_CONFIG_DIR` | Directory holding `config.toml` | `~/.zeroclaw` |
| `ZEROCLAW_DATA_DIR` | Shared data dir (DBs, state) | `<config_dir>/data` |
| `ZEROCLAW_SOCKET` | Daemon Unix socket path | `<data_dir>/daemon.sock` |
| `ZEROCLAW_WORKSPACE` | Legacy workspace override (deprecated) | — |
| `ZEROCLAW_<section>__<key>` | Schema-mirror config overrides, e.g. `ZEROCLAW_providers__models__openai__main__api_key=...` | — |

> **This is the key:** set `ZEROCLAW_CONFIG_DIR` (or pass `--config-dir`) and the
> whole instance — config, databases, per-agent workspaces, and the socket
> zerowrapper connects to — lives inside your chosen directory.

---

## 2. Directory layout of a scaffolded instance

```
<target-dir>/
├── config.toml                     # zeroclaw config (models, agents, MCP, ...)
├── run-zeroclaw.sh                 # launcher: sets env + runs `zeroclaw daemon`
├── data/
│   └── daemon.sock                 # Unix socket that `zw` connects to
│   └── *.db                        # sqlite databases (brain.db, state, cost)
├── agents/
│   └── <alias>/
│       └── workspace/
│           ├── MEMORY.md           # persistent memory doc
│           ├── IDENTITY.md         # who the agent is
│           ├── SOUL.md             # persona / voice
│           └── memory/             # markdown memory (if markdown backend)
└── shared/                         # shared skills/knowledge bundles
```

Notes:

- **Memory DB path** — `[storage.sqlite.<alias>] path`; defaults to
  `<workspace_dir>/brain.db`. Markdown memory defaults to `<workspace_dir>/memory/`.
- **Per-agent workspace** — `[agents.<alias>.workspace] path` overrides the
  default `<config-dir>/agents/<alias>/workspace/`.
- **Web dashboard dist** — `[gateway] web_dist_dir` points at a built ZeroClaw
  dashboard (`index.html`). If unset the gateway auto-detects `web/dist` next to
  the binary or the CWD.

---

## 3. Setup

```sh
cd zerowrapper/zeroclaw-template
./setup.sh -d ../zeroclaw-instance -a craw
```

Then edit `../zeroclaw-instance/config.toml`:

- Set your model provider (`api_key`, `uri`, `model`) — or set the referenced env
  var, e.g. `OPENAI_API_KEY`.
- Add/remove MCP servers under `[[mcp.servers]]` and grant them to the agent via
  `[mcp_bundles.<alias>]`.
- Tune `[agents.<alias>]`, `[risk_profiles.<alias>]`, `[gateway]`.

Re-running `setup.sh` never overwrites existing files.

---

## 4. Start the daemon

```sh
ZEROCLAW_CONFIG_DIR=/path/to/zeroclaw-instance zeroclaw daemon
# or
/path/to/zeroclaw-instance/run-zeroclaw.sh
```

The socket appears at `<target>/data/daemon.sock`. Point `zw` at it:

```toml
# zerowrapper/config.toml
[zerowrapper]
zeroclaw_socket = "/path/to/zeroclaw-instance/data/daemon.sock"
```

`zw` connects over JSON-RPC on that socket (`session/new`, `session/prompt`,
`session/messages`, `session/cancel`) — see `zw-channels/src/zeroclaw_bridge.rs`.

---

## 5. Automating the config (models, MCP, agents, workspace)

All of these are driven with `--config-dir` or `ZEROCLAW_CONFIG_DIR` set, so they
only touch your instance:

### Headless key setting

```sh
zeroclaw config set providers.models.openai.main.api_key 'sk-...'
zeroclaw config set providers.models.anthropic.main.model claude-sonnet-4-5
zeroclaw config set agents.craw.model_provider anthropic.main
zeroclaw config set mcp.servers.filesystem.command npx
zeroclaw config set agents.craw.workspace.path /data/craw/workspace
```

### Env-mirror overrides (no config file writes)

```sh
ZEROCLAW_CONFIG_DIR=... ZEROCLAW_providers__models__openai__main__api_key='sk-...' zeroclaw daemon
```

### Scripted quickstart (interactive-ish, flag pre-seeded)

```sh
zeroclaw quickstart --model-provider openai --model gpt-5.4 --agent assistant
```

### Verify / inspect

```sh
zeroclaw config list                       # all properties
zeroclaw config get agents.craw.workspace  # read a value
zeroclaw doctor                            # daemon/channel diagnostics
zeroclaw self-test --quick                 # offline sanity checks
```

Secrets (`api_key`, MCP `env`, `headers`) are stored encrypted; masked input is
automatic for interactive `config set`.

---

## 6. Notes on zerowrapper integration

- `zw` currently **expects the daemon already running** — it only connects to the
  socket. Start the daemon first (section 4), then `zw`.
- The zeroclaw agent alias in `[zerowrapper.ch_web] agent = "craw"` must match an
  `[agents.<alias>]` entry in your instance's `config.toml`.
- MCP discovery: to let zeroclaw spawn `zw` itself as an MCP tool server, add a
  `[[mcp.servers]]` entry in the instance config with `command = "/path/to/zw"`
  and `args = ["--mcp"]`, plus an `[mcp_bundles.<alias>]` grant.

---

## 7. Reference pointers (inside `../zeroclaw/`)

- Minimal config: `dev/config.template.toml`
- Config schema: `crates/zeroclaw-config/src/schema.rs`
- Socket resolution: `crates/zeroclaw-runtime/src/rpc/local.rs`
- Config dir resolution: `crates/zeroclaw-config/src/schema.rs` (`default_config_dir`)
- MCP: `docs/book/src/tools/mcp.md`
- Providers: `docs/book/src/providers/configuration.md`
