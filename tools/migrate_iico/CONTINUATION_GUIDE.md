# Migration Continuation Guide — IICO → HelixDB

How to continue the IICO data migration into the serverless engine (HelixDB
backend). Written for agents picking up where the last session left off.

## Where everything lives

```
serverless-engine/
  crates/                       # Rust workspace
    helixdb/                    # Helix HTTP client (POST /v2/query)
    engine/                     # core engine (Database trait, crud, graph_sync)
    server/                     # daemon (REST/MCP/WS) + db/helix.rs adapter
    mcp/                        # MCP tool server
    cli/                        # `srv` CLI (MCP client)
  tools/migrate_iico/           # migration pipeline (Python)
    config.py                   # BOARD, batch size, GRAPH_EDGES map
    extract.py                  # SSH + psql JSON dump (reuses local dumps)
    transform.py                # null-strip, drop columns (KEEPS UUIDs)
    loader.py                   # create table, bulk load, verify refs, graph sync
    migrate.py                  # orchestrator (--migrate fast path, --batch)
    data/raw/                   # LOCAL EXTRACTED SOURCE DATA (176 tables)
    data/transformed/           # LOCAL TRANSFORMED DATA (175 tables) — loader input
    SUBAGENT_WORKER.md          # per-table checklist
    MIGRATION_STATUS.md         # live per-table status
    MIGRATION_STRATEGY.md       # the graph-first plan
  docs/helixdb-query-authoring.md  # graph/query contract
  docs/blocking-client-async-runtime.md  # IMPORTANT: why blocking I/O must run on blocking threads
```

## Current state

- **Board (tenant): `b_7z5lr8zaurkd0000`** — the iico HelixDB tenant.
- **176 tables created** in the board (all source tables). Most loaded; a few
  partial (see `MIGRATION_STATUS.md` / spot-check with counts).
- **HelixDB** runs in Docker on port 7979 (container `helix-serverless-engine-dev`,
  MinIO `helix-serverless-engine-dev-minio` on the `helix-serverless-engine-dev-net`
  network). Fresh DB was reset once due to a clock-skew issue — see the note below.
- **All source data is already extracted locally** in `tools/migrate_iico/data/raw/`
  (the loader reads `data/transformed/`). No SSH needed to load — the network
  dependency is gone.

## How to run the daemon (in tmux)

```bash
cd /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine
tmux new -s srv            # or: tmux attach -t srv if it exists
# inside tmux:
SRV_DB=helix SRV_HELIX_URL=http://127.0.0.1:7979 SRV_PORT=7070 SRV_DATA_DIR=/tmp/helix-iico ./target/release/srv daemon start
# detach with Ctrl-b d
```

Check it's up: `curl http://127.0.0.1:7070/healthz` → `{"ok":true}`

**Rebuilding the daemon after a Rust change:**
```bash
cd /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine
cargo build --release        # from workspace root, no -p flags
# then kill the old daemon and restart (Ctrl-c in the tmux window, re-run the command above)
```

## How to check the DB (the way the lead was doing it)

Every command below uses `SERVERLESS_DAEMON_URL=http://127.0.0.1:7070` and the
`target/release/srv` CLI (an MCP client):

```bash
export SERVERLESS_DAEMON_URL=http://127.0.0.1:7070
B=b_7z5lr8zaurkd0000

# count rows in a table (engine-side aggregate)
./target/release/srv records aggregate $B <table> count

# query records (node_id is the Helix node id; payload is the app data)
./target/release/srv records query $B <table> --limit 5

# graph: sync *_id refs into edges (idempotent)
./target/release/srv graph sync $B <table>

# graph: traverse from a node (node_id from records query)
./target/release/srv graph traverse $B <node_id> --dir out

# raw Helix check (for debugging the adapter):
curl -s -X POST http://127.0.0.1:7979/v2/query -H 'Content-Type: application/json' \
  -d '{"request_type":"read","query_name":"c","query":{"read":{"entries":[{"query":{"name":"c","root":{"count":{"input":{"nodes_where":{"predicate":{"eq":{"left":{"property":"$label"},"right":{"constant":{"string":"__srv__record"}}}}}}}}}}],"returns":["c"]}}}'
```

## How to migrate a table (fast path)

```bash
cd /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine/tools/migrate_iico

# fast clean reload from the LOCAL dump (drops table, bulk-imports, graph syncs):
python3 migrate.py --workdir /tmp/migrate_iico --tables <t1>,<t2> --migrate --batch 100

# if you want the data from the repo's data/ instead of /tmp (same content):
# set the workdir to the repo folder:
python3 migrate.py --workdir /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine/tools/migrate_iico/data --tables <t> --migrate --batch 100
```

Notes:
- `--migrate` DROPS the table first (clean reload), so partial data is replaced.
- `--batch 100` is the fast bulk-import batch size (one Helix write batch per
  request). Larger batches are fine for small tables; keep 100 for the big ones.
- The loader reads `transformed/<t>.json`. If a transformed file is missing,
  run once WITHOUT `--migrate` so extract+transform run (they reuse local raw).

## Known bugs / where the Rust code is

If something fails, here's where to look:

| Symptom | File | Fix area |
|---|---|---|
| Daemon hangs after ~128 requests | `crates/helixdb/src/client.rs`, `crates/mcp/src/transport.rs`, `crates/server/src/transport/rest.rs`, `crates/server/src/transport/ws.rs`, `crates/server/src/jobs.rs` | Blocking reqwest client MUST run on `tokio::task::spawn_blocking`, never an async worker. See `docs/blocking-client-async-runtime.md`. |
| `invalid query JSON` from Helix | `crates/helixdb/src/request.rs`, `crates/server/src/db/helix.rs` | Envelope shapes must match the wire format. `aggregate_by` needs a `property` even for count. `drop_edge_by_id` uses `{edges:{ids:[...]}}`. |
| Writes silently not persisting (records "load" but count stays 0) | `crates/server/src/db/helix.rs` `write_node` | `_srv_key` collides across tables (each table's seq starts at 1) — the existing-lookup must also match the `table` prop. |
| Bulk import fails with `unknown variant add_n` | `crates/server/src/db/helix.rs` `bulk_insert` | Batch entries must be `{query:{name,root}}` via `request::entry`, not raw `add_n`. |
| `active_text_mutation_limit_exceeded` (512) | `crates/server/src/db/helix.rs` | Once a text index exists, `_search` writes count as mutations. Gated by `is_text_indexed`. Don't create the FTS index during bulk migration. |
| Graph sync creates 0 edges | `crates/engine/src/engine.rs` `graph_resolve_target` | Target-table discovery: `pluralize()` + `user`/`teacher`→`profiles` aliases. Refs resolve by `$.id` (UUID), then `$.seq`. |
| SSH timeout during extract | `tools/migrate_iico/extract.py` | `SSH_TIMEOUT` is 300s now. Big tables (audit_log 134k) need it. |
| Helix container clock skew (`RequestTimeTooSkewed` / `invalid clock tick`) | Docker ops | The Helix→MinIO SlateDB requires monotonic ticks. If it breaks: stop helix+minio, `docker exec minio rm -rf /data/helix-db`, recreate the bucket dir, restart both. DATA IS LOST on the Helix side — re-run the loader from local dumps (that's why we keep the data locally). |

**Critical rule:** never call `cargo clean`, never `cargo test` (the user runs
tests), and never touch `target/`. Build with plain `cargo check` /
`cargo build --release` from the workspace root.

## The graph model (what the migration produces)

- Every record = one Helix node under the `__srv__record` label, with
  `tenantId = b_7z5lr8zaurkd0000`, `table` prop, `seq`, `_srv_key`, `data`
  (full engine JSON), `_node_id` (the Helix node id, exposed in payloads).
- `*_id` payload fields (source UUIDs) become `RELATED_<FIELD>` edges via
  `srv graph sync <board> <table>` — idempotent, run after every load.
- `profiles`/`learners`/`classes`/`subjects`/`guardians` are the core entities
  and are already migrated + verified.

## To finish the migration

1. Load the remaining partial tables: compare source counts (from
   `data/raw/*.json` row counts) vs engine counts, then `--migrate` the ones
   that differ. See `MIGRATION_STATUS.md`.
2. Run `graph sync` on every table with `*_id` refs (the loader does this
   automatically after each load; re-run idempotently).
3. Verify: `records aggregate <board> <t> count` == local `len(data/raw/<t>.json)`
   for every non-empty table.
4. Update `MIGRATION_STATUS.md` per table.

## Known source-data orphans (verified against raw dumps, 2026-08-18)

These `*_id` refs exist in the source data but point at rows that are absent
from every local dump (deleted source rows). The loader's `verify_references`
step will fail on them; the load itself is complete and correct, and `graph
sync` skips them safely. Do NOT re-extract to "fix" these — they are gone from
the source DB.

- `assets.category_id` → **12 refs** point at `inventory_categories`, not
  `asset_categories`. `GRAPH_EDGES` in `config.py` was corrected
  (`assets.category_id → inventory_categories`). The 12 ids all exist there.
- `in_app_notifications.user_id` → **7 orphans** not in `profiles`:
  `00572519-77c3-4afa-b45d-afe41067b276`, `3092ced9-b2b7-4f3f-a82a-90c9e1b10eb0`,
  `479717c6-f00f-4a6b-b785-5467d0b62eb8`, `6557cc87-dbde-45ef-b020-f75d3e2fbad7`,
  `86059752-c59b-4312-b180-9bb2ece6d23a`, `961ee724-6e85-49ad-983b-31c2ece94039`,
  `9a13a5ca-e151-44b7-84a4-c69bd3ce4fcb`.
  If a future verify run fails on these, pass the table through
  `loader.py data __sync__<table>` directly (verify is source-truth only;
  sync is what materializes edges).
