# Migration Subagent Worker Checklist

The shared recipe every parallel migration sub-agent follows. Each agent owns a
**disjoint set of tables** (see `MIGRATION_STRATEGY.md` §4 for the groups) and
must complete the full checklist per table. The goal: no silent corruption —
every table is count-checked, referentially verified, graph-synced, and
spot-checked against real source rows.

## Your inputs

- **Board id (tenant):** `<BOARD>` (set in `tools/migrate_iico/config.py`)
- **Your table list:** from the group assignment
- **Engine daemon:** `http://127.0.0.1:7070` (REST) / MCP at `/mcp`
- **Source:** SSH to `iico@192.168.88.101` (password `12345678`), psql inside
  `supabase-db` container, db `postgres`, user `supabase_admin`

## Before you start

1. Confirm the daemon is up: `curl http://127.0.0.1:7070/healthz`
2. Confirm your tables are NOT already in `done.json` (workdir
   `/tmp/migrate_iico`): `python3 -c "import json;print(json.load(open('/tmp/migrate_iico/done.json')))"`
3. Confirm every target table of your `*_id` refs is already migrated (check
   `GRAPH_EDGES` in `config.py` — the referential check needs the target's
   transformed dump to exist). If a target isn't migrated yet, stop and report.

## Per-table checklist

### Step 1 — Small test (3 rows)

```bash
cd tools/migrate_iico
python3 migrate.py --workdir /tmp/migrate_iico --tables <table> --limit 3
```

Inspect `transformed/<table>.json`:
- `*_id` refs are still UUIDs (NOT emails).
- No stray nulls (nulls are stripped).
- The 3 records look correct in the engine:
  `srv records query <BOARD> <table> --limit 3` (or the REST equivalent).

### Step 2 — Full load

```bash
python3 migrate.py --workdir /tmp/migrate_iico --tables <table>
```

This runs: extract → transform → bulk load (count-checked) → **referential
verify** (`__verify__`, fails on orphan `*_id` refs) → **graph sync**
(`__sync__`, creates `RELATED_*` edges via MCP `graph.sync`).

### Step 3 — Verify counts

```bash
curl -s "http://127.0.0.1:7070/api/srv/<BOARD>/tables/<table>/aggregate?op=count" \
  -H "Authorization: Bearer <BOARD>"
```
Must equal the source count. Cross-check with the source:
```bash
sshpass -p '12345678' ssh -o StrictHostKeyChecking=no iico@192.168.88.101 \
  'docker exec -i supabase-db psql -U supabase_admin -d postgres -t -A -c "select count(*) from <table>"'
```

### Step 4 — Spot-check real rows (anti-hallucination)

Pick 3 rows from the source (via SSH psql) and 3 from the engine — compare
field-by-field (not just counts). Pay attention to:
- Dates/timestamps survived.
- `*_id` values match the source exactly (no rewrite).
- Text fields didn't get mangled (encoding).
- Numeric fields are numeric in the engine.

```bash
# source sample
sshpass ... 'docker exec -i supabase-db psql -U supabase_admin -d postgres -t -A \
  -c "select * from <table> order by random() limit 3"'
# engine sample
srv records query <BOARD> <table> --limit 3
```

### Step 5 — Graph verification

For each ref in `GRAPH_EDGES[<table>]`:
```bash
# pick a source record's node id
srv records query <BOARD> <table> --filter '{"id":{"eq":"<uuid>"}}'
# traverse out — expect a RELATED_<FIELD> edge to the target table's node
srv graph traverse <BOARD> <node_id> --dir out
```
- Every non-null `*_id` ref should have produced an edge.
- Traverse the reverse direction (`--dir in`) from a target node and confirm
  the related records appear.

### Step 6 — Update the status doc

Append/update your tables in `MIGRATION_STRATEGY.md` §5 status table:
rows loaded, edges synced, verification notes.

## Rules

- **Never modify** `config.py` `GRAPH_EDGES` for another agent's tables.
- **Never delete** another agent's table or board.
- If a table fails the referential check: **stop, do not force-load**. Report
  the orphan sample. It may mean the target table name in `GRAPH_EDGES` is
  wrong (check the source schema) — fix the map, not the data.
- If the source count is 0, load the empty table and mark done (nothing to
  verify).
- Keep the daemon running; if it dies, restart with:
  `SRV_DB=helix SRV_HELIX_URL=http://127.0.0.1:7979 srv daemon start`

## Reporting back

Return per table: `{table, source_count, engine_count, edges_created,
referential: pass/fail, spot_check: pass/fail, notes}`.
