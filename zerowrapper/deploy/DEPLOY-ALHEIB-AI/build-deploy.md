# Build & Deploy Guide — Jerboa (DEPLOY-ALHEIB-AI)

How to build the Rust service and deploy it to the Docker-backed Jerboa site.
The Jerboa site is a **bare chat-only** deployment: no landing page, no header/footer/nav,
no per-site config YAMLs — just the chat area (plus the whiteboard panel, theme toggle and
background dots). Agent name: `jerboa`.

It also supports **root-only embedding**: the frontend may be embedded where only `/` is
reachable. A `RootDispatchLayer` intercepts root-relative query URLs so API, WebSocket,
file/media and bundled-asset requests all work through `/`:

| Request (embedded)                | Handled by                                    |
|-----------------------------------|-----------------------------------------------|
| `GET /`                           | root HTML (chat app)                          |
| `GET /?asset=app-<hash>.js`       | bundled JS/CSS asset                          |
| `GET /?api=history&user_id=...`   | chat history                                  |
| `POST /?api=upload`               | file upload (`X-Filename` header)             |
| `GET /?ws=1`                      | WebSocket upgrade                             |
| `GET /?file=<hash>`               | cached file/media                             |

## Prerequisites

- `cargo` (Rust toolchain) — builds the `zw` release binary.
- `topcoat` (`~/.cargo/bin/topcoat`) — extracts hashed assets from the binary into the
  deploy folder the container serves.
- `docker` + `docker compose` — run the container.
- `bun` — only needed when changing the frontend JS/CSS (see the two-part build below).

Key paths:

- Working copy: `/home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine/zerowrapper`
- Crate: `crates/ch-web-ui`
- JS entry: `crates/ch-web-ui/assets/js/chat/app-entry.js`
- Deploy dir (self-contained): `deploy/DEPLOY-ALHEIB-AI/`
  - `compose.yml` — the compose file for this site
  - `jerboa.env` — env file (load with `--env-file jerboa.env`)
  - `assets/` — topcoat bundle (hashed `.js`/`.css` + `manifest.toml`)
  - `data/` — runtime DB (`zerowrapper.db`) + `cache/`
  - `zw` — the release binary (copied here after build)
- Runtime zeroclaw socket/agents (shared with other sites): `/home/vox/.zeroclaw/{data,agents}`

## Layout

This site is **self-contained**: it does **not** mount per-domain config YAMLs
(`landing.yaml`, `sitesettings.yaml`) — the app falls back to built-in defaults.
There is no `/app/config` mount and no `CONFIG_DIR`/`ZEROWRAPPER_CONFIG_DIR` in the env.

| env var                    | jerboa                                             | mounted as        |
|----------------------------|----------------------------------------------------|-------------------|
| `INSTANCE_NAME`            | `jerboa` (compose project name + container name)   | —                 |
| `HOST_PORT`                | `9092` (host) → `9090` (container)                 | —                 |
| `ZW_BIN`                   | `deploy/DEPLOY-ALHEIB-AI/zw`                       | `/usr/local/bin/zw` (ro) |
| `BUNDLE_DIR`               | `deploy/DEPLOY-ALHEIB-AI/assets`                   | `/app/bundle` (ro) |
| `DATA_DIR`                 | `deploy/DEPLOY-ALHEIB-AI/data`                     | same path (rw)    |
| `SOCKET_DIR`               | `/home/vox/.zeroclaw/data`                         | same path         |
| `AGENTS_DIR`               | `/home/vox/.zeroclaw/agents`                       | same path (rw)    |
| `TOPCOAT_ASSETS_DIR`       | `/app/bundle`                                      | —                 |
| `ZEROWRAPPER_CH_WEB__AGENT`| `jerboa`                                           | —                 |
| `ZEROWRAPPER_CH_WEB__BIND` | `0.0.0.0:9090` (in-container)                      | —                 |

Removed env vars (not needed for this bare-chat site):

- `ZEROWRAPPER_CH_WEB__ACCOUNTS` — owner/admin login removed (email-based sessions only)
- `ZEROWRAPPER_CH_WEB__ALLOWED_ORIGINS` — defaults to `["*"]`
- `ZEROWRAPPER_CONFIG_DIR` / `ZEROWRAPPER_LANDING_YAML` / `ZEROWRAPPER_SETTINGS_YAML` — no per-site config
- `SOCKET` (helper var; compose uses `SOCKET_DIR`)

## Two-part build

The project is **Rust (topcoat) + JS (bundled by bun)**. The Rust binary registers the JS
asset at compile time; the JS must be bundled by bun *before* `cargo build --release`.

### 1. Bundle frontend JS (bun) — only if you changed JS/CSS

```bash
cd crates/ch-web-ui/assets/js/chat
bun build app-entry.js \
  --format esm \
  --external marked \
  --external highlight.js \
  --external katex \
  --external mermaid \
  --outfile app.js
```

CDN libraries (marked, katex, mermaid, highlight.js) stay external via the importmap; the
output `app.js` is what the server serves.

Optional: after editing any Rust file, sanity-check with `cargo check` (fast).

### 2. Build the release binary (cargo)

```bash
cd /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine/zerowrapper
cargo build --release
```

This embeds the current `app.js`/`styles.css` into the `zw` binary's asset registry.

## Deploy

### 3. Bundle assets into the Jerboa folder (topcoat)

```bash
cd /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine/zerowrapper
topcoat asset bundle --bin zw --release --out deploy/DEPLOY-ALHEIB-AI/assets
```

Writes hashed files (e.g. `app-<hash>.js`) + `manifest.toml` into `DEPLOY-ALHEIB-AI/assets/`,
which the container mounts read-only as `/app/bundle`.

### 4. Copy the binary (atomic, safe while the container runs)

```bash
cd /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine/zerowrapper
cp target/release/zw deploy/DEPLOY-ALHEIB-AI/zw.new
mv -f deploy/DEPLOY-ALHEIB-AI/zw.new deploy/DEPLOY-ALHEIB-AI/zw
```

### 5. Recreate the container (docker compose)

The project name comes from `INSTANCE_NAME` in the env file (`jerboa`), so no `-p` flag:

```bash
cd deploy/DEPLOY-ALHEIB-AI
docker compose -f compose.yml --env-file jerboa.env up -d --force-recreate
```

### 6. Verify

```bash
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:9092/   # 200
curl -s http://localhost:9092/ | grep -o 'app-[a-f0-9]*\.js' | head -1   # current asset hash
```

Assets are served under `/_topcoat/assets/` (not the site root). To confirm a change is
live, download that asset and grep for the new JS:

```bash
curl -s http://localhost:9092/_topcoat/assets/<app-hash>.js | grep '<your-symbol>'
```

## Notes

- No `landing.yaml`/`sitesettings.yaml`: the app logs a warning and uses built-in defaults
  (`title: "Jerboa"`, no examples/FAQ). The defaults live in `src/main.rs`.
- The whiteboard panel, theme toggle and background dots are kept on this bare-chat page.
- Session ID is `web.{agent}.{email}` (the `@` in the email is preserved).
- Frontend change shows old behavior after deploy → hard-refresh (assets are hash-versioned).
- `docker compose up -d` reports "Running" without recreating → add `--force-recreate`.
