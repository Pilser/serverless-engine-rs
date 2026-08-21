# Production-folder

Self-contained, **portable** Docker deploy for the two server binaries:

| Service | What it is | Host port |
|---------|-----------|-----------|
| `srv`   | Serverless engine daemon (API + MCP) | 7070 |
| `zw`    | Zerowrapper / Jerboa AI chat | 9033 |

Everything inside this folder is **relative-path based** — no absolute paths in
compose/Dockerfiles — so the whole folder can be copied to another machine and
started with Docker only. Bind-mounted state stays mounted.

## Layout

```
Production-folder/
├── compose.yml          # one file, both services
├── Dockerfile.srv       # srv image (debian/glibc)
├── Dockerfile.zw        # zw image (debian)
├── .env                 # per-machine config (copy from .env.example)
├── .env.example
├── srv/
│   ├── srv              # the daemon binary
│   └── data/            # daemon data (boards + object store) — host mount
└── zw/
    ├── zw               # the zw binary
    ├── assets/          # topcoat asset bundle (hashed JS/CSS + manifest)
    └── data/            # chat DB + cache — host mount
```

## Deploy

```bash
cd Production-folder
cp .env.example .env      # first time: edit ports/secrets/host paths
docker compose up -d --build
```

Verify:

```bash
curl http://127.0.0.1:7070/healthz                      # {"ok":true}
curl -X POST http://127.0.0.1:7070/mcp -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
curl http://127.0.0.1:9033/                             # Jerboa chat HTML
```

Update after rebuilding a binary:

```bash
# replace Production-folder/srv/srv and/or Production-folder/zw/zw + zw/assets
docker compose up -d --build
```

## What must stay mounted (external host deps)

These are NOT containers in this compose — they live on the host and are
bind-mounted in:

1. **HelixDB + MinIO** — the storage backend the daemon talks to
   (`SRV_HELIX_URL`). Two ways to wire it:
   - **Container on the same compose network (recommended):** add helixdb + minio
     as services here (or `docker network connect <this-net> <helix-container>`)
     and set `SRV_HELIX_URL=http://<helix-container-name>:8080`. Container-to-
     container traffic is allowed by Docker even when a host firewall is on.
   - **Host gateway:** set `SRV_HELIX_URL=http://<gateway-ip>:7979`. The gateway
     is NOT always 172.17.0.1 — compose custom networks use a different subnet
     (`docker inspect <srv-container> | grep Gateway`). NOTE: if the host runs
     ufw with `DEFAULT_FORWARD_POLICY="DROP"`, container→host-gateway TCP is
     silently dropped — use the container-name approach instead, or open the
     port: `sudo ufw allow from 172.16.0.0/12 to any port 7979`.
2. **zeroclaw daemon socket** — `SOCKET_DIR` → mounted at
   `/home/vox/.zeroclaw/data` inside zw. The zeroclaw daemon must be running on
   the host and its socket must exist.
3. **zeroclaw agents** — `AGENTS_DIR` → mounted at `/home/vox/.zeroclaw/agents`
   (agent workspaces: jerboa identity/memory/sessions).

If any of these is missing, the related service starts but can't do its job:
- srv: `http error: error sending request for url (http://127.0.0.1:7979/v2/query)` → HelixDB/MinIO down.
- zw: chat opens but agent turns fail → zeroclaw socket or agents dir missing.

## Portability to another machine

1. Copy the whole `Production-folder/` to the target machine (rsync/scp).
2. `cp .env.example .env` and edit:
   - `SRV_PUBLIC_URL` (this machine's LAN IP / domain)
   - `SRV_SECRET_KEY` (32 bytes)
   - `SOCKET_DIR` / `AGENTS_DIR` / `ZEROWRAPPER__ZEROCLAW_SOCKET` (this
     machine's zeroclaw paths)
3. Ensure HelixDB + MinIO and the zeroclaw daemon run on that machine.
4. `docker compose up -d --build`.

The `srv/data` and `zw/data` folders move WITH the folder (copied), so state
comes along.

## Notes

- Both images are **debian-based (glibc)** — the srv binary needs real glibc
  `_Unwind_*` symbols; alpine/musl + libc6-compat is NOT enough.
- Containers run as `1000:1000` so host-mounted data dirs stay writable.
- `srv` binds `SRV_HOST=0.0.0.0` inside the container; the port is exposed via
  compose. Do NOT set `SRV_HOST=127.0.0.1` in .env — it would break host access.
- The zw image carries a COPY of `zw/data` as seed; the compose mount
  `./zw/data:/data` overrides it with the live host data.
- Hot-swap binaries without a rebuild: uncomment the bind-mount lines in
  compose.yml and `docker compose up -d` (no `--build`).
