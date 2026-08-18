# IICO → Engine Migration Guide

How to move data from the self-hosted Supabase instance on the IICO machine into
the serverless engine, using the `tools/migrate_iico/` migration toolkit.

This guide is written so **any agent (or human) without prior context** can add
the remaining ~40 tables (attendance, report_cards, fee_payments, employee/HR,
etc.) in the same way the core tables were migrated.

---

## 1. Overview of the pipeline

```
┌─────────────┐   SSH + docker exec   ┌─────────────┐   transform    ┌────────────┐   REST bulk   ┌──────────────┐
│ iico machine │ ──────────────────▶  │  raw/*.json  │ ────────────▶  │ transformed │ ────────────▶  │ serverless   │
│  supabase-db │   (psql JSON dump)   │  (source)    │  UUID→email    │   *.json    │  (upsert)      │ engine board  │
└─────────────┘                       └─────────────┘                └────────────┘               └──────────────┘
   extract.py                             └──────────── transform.py ──────────┘
                                          └────────────────── loader.py ───────────────────────────┘
```

Each table goes through **3 independent steps**, all orchestrated by `migrate.py`:

| Step | Module | What it does |
|---|---|---|
| 1. Extract | `extract.py` | SSH to the source machine, dump a table as JSON array → `raw/<table>.json` |
| 2. Transform | `transform.py` | Rewrite user-reference UUIDs → lowercase email; drop nulls / unwanted cols → `transformed/<table>.json` |
| 3. Load | `loader.py` | Create the table (permissive schema) if missing, bulk-insert in batches → engine |

---

## 2. Location & files

All under `tools/migrate_iico/`:

| File | Role |
|---|---|
| `migrate.py` | Orchestrator: runs extract → transform → load per table, with checkpoints |
| `config.py` | **All config lives here** — source creds, engine target, batch size, the UUID→email column map, recreate list |
| `extract.py` | SSH + `docker exec ... psql ... json_agg` to pull a table |
| `transform.py` | UUID→email rewrite + null/column cleanup |
| `loader.py` | Creates tables, bulk-loads records, imports auth users with bcrypt hashes |

---

## 3. Configuration (`config.py`)

### Source (the IICO machine)
```python
SRC_HOST        = "192.168.88.101"   # SSH host
SRC_USER        = "iico"             # SSH user
SRC_SSH_PASS    = "12345678"         # SSH password
SRC_DB_CONTAINER = "supabase-db"     # docker container holding postgres
SRC_DB_USER     = "supabase_admin"   # db user (superuser)
SRC_DB_NAME     = "postgres"         # db name
```
These can be overridden by env vars `IICO_HOST`, `IICO_SSH_USER`, `IICO_SSH_PASS`,
`IICO_DB_CONTAINER`, `IICO_DB_USER`, `IICO_DB_NAME`.

### Destination (the engine)
```python
ENGINE_BASE = "http://127.0.0.1:7070"            # engine daemon
BOARD       = "<board id>"                        # the iico HelixDB tenant
OWNER_TOKEN = BOARD                               # board id bearer = owner role
```
Override via `ENGINE_BASE`, `BOARD`, `OWNER_TOKEN`. Create the board first with
`srv apps create 'iico'` and set `BOARD` to the returned id.

### Behaviour
```python
BATCH_SIZE = 100                # records per bulk POST (env: MIGRATE_BATCH_SIZE)
SKIP_EXISTING_USERS = True      # users that already exist are skipped, not errors
```

### The graph edge map: `GRAPH_EDGES`
```python
GRAPH_EDGES = {
    # table            -> { ref_column: target_table }
    "learners":    {"class_id": "classes", "user_id": "profiles"},
    "attendance":  {"class_id": "classes", "learner_id": "learners"},
    # ... ~60 tables
}
```
This is the heart of the graph model. **`*_id` UUID references are KEPT as-is**
(no email rewrite — the frontend `deploy/iico` joins by UUID, and the graph
turns refs into `RELATED_*` edges). After a table loads, the pipeline:

1. **Verifies referential integrity**: every `*_id` value in the table must
   exist in the target table's `id` set (from its transformed dump). Orphans
   fail loudly — this is the anti-hallucination guard.
2. **Runs `srv graph sync <board> <table>`** (via the MCP endpoint) to create
   `RELATED_<FIELD>` edges in the board's HelixDB tenant. Idempotent.

**When you add a new table**, add its `*_id` refs to `GRAPH_EDGES`.

### `RECREATE_TABLES`
```python
RECREATE_TABLES = {"learners"}   # tables with a stale strict schema in the engine
```
Tables in this set are **dropped and recreated** (permissive schema) before
loading. Add a table here if the engine already has it with a strict schema
(e.g. a required column) that would reject real source rows.

### `DROP_COLUMNS` (optional)
```python
DROP_COLUMNS = {
    "profiles": ["updated_at"],
    "classes":  ["updated_at"],
}
```
Columns listed here are removed from each row before loading.

---

## 4. How to run it

### Pre-requisites
- Engine daemon is running (REST on `127.0.0.1:7070`).
- `sshpass` installed on the machine running the tools (check: `which sshpass`).
- The iico machine reachable over SSH with the creds in `config.py`.
- The board id in `config.py` matches the tenant you're loading into.

### Common commands

```bash
cd tools/migrate_iico

# Import ALL auth users (email + password_hash + role) — do this first
python3 migrate.py --workdir /tmp/migrate_iico --users

# Migrate one or more specific tables (comma-separated)
python3 migrate.py --workdir /tmp/migrate_iico --tables attendance,report_cards

# Run the curated "core" set (see CORE_TABLES in migrate.py)
python3 migrate.py --workdir /tmp/migrate_iico --all
```

### Safe test on small data (do this for every NEW table first!)
```bash
# Extract only 3 rows of a table, load them, verify. Nothing is committed to done.json.
python3 migrate.py --workdir /tmp/migrate_iico --tables attendance --limit 3
```
Then inspect the transformed output and the loaded records in the engine to
confirm no hallucination before doing the full load.

### Re-run / resume
- **Checkpoint:** completed tables are recorded in `<workdir>/done.json`.
  Reruns **skip** completed tables.
- **Force re-run:** `--force` re-extracts and reloads.
- **Idempotent:** loading uses `?upsert=1` with `unique_key=$.id`, so re-running
  a partially-loaded table just fills in the rest without duplicating rows.

### Environment overrides (optional)
```bash
IICO_HOST=192.168.88.101 BOARD=b_... MIGRATE_BATCH_SIZE=50 \
  python3 migrate.py --workdir /tmp/migrate_iico --tables attendance
```

---

## 5. Adding the remaining ~40 tables (step-by-step)

The core tables are already migrated. The rest (attendance, report_cards,
fee_payments, employee/HR, library, finance, discipline, inventory, etc.) are
done the **exact same way**. For each new table:

### Step 5.1 — Identify `*_id` reference columns
Every column ending in `_id` that points at another table's `id` is a candidate
edge. Query the source to confirm:
```sql
select table_name, column_name, data_type from information_schema.columns
where column_name like '%_id' and table_schema='public' order by table_name;
```
Add the table + refs to `GRAPH_EDGES` in `config.py`:
```python
"new_table": {"learner_id": "learners", "teacher_id": "profiles"},
```

### Step 5.2 — (Optional) Add to `CORE_TABLES`
If you want the table included in `--all`, add its name to the `CORE_TABLES`
list in `migrate.py`.

### Step 5.3 — Small test first
```bash
python3 migrate.py --workdir /tmp/migrate_iico --tables <new_table> --limit 3
```
Check:
- `transformed/<new_table>.json` — `*_id` refs are still UUIDs; no stray nulls.
- The 3 records loaded in the engine (via MCP/REST) look correct.

### Step 5.4 — Full load
```bash
python3 migrate.py --workdir /tmp/migrate_iico --tables <new_table>
```
This also runs the referential check (`__verify__`) and graph sync
(`__sync__`) automatically.

### Step 5.5 — Verify counts + edges
The loader already fails loudly if `loaded != source`. Re-check counts and the
materialized graph:
```bash
curl -s "http://127.0.0.1:7070/api/srv/<BOARD>/tables/<new_table>/aggregate?op=count" \
  -H "Authorization: Bearer <BOARD>"
srv graph traverse <BOARD> <node_id> --dir out   # confirm RELATED_* edges
```

---

## 6. Known `*_id` reference columns (full audit)

From the earlier information_schema scan — the reference columns that become
`RELATED_*` edges (kept as UUIDs, documented in `GRAPH_EDGES`):

| Table | user-ref columns |
|---|---|
| profiles | id, suspended_by |
| account_appeals | user_id |
| advance_requests | user_id |
| akhlaaq_reports | teacher_id |
| appointments | created_by |
| asset_custodians | employee_id |
| assets | assigned_to_staff |
| attendance | recorded_by |
| audit_log | user_id |
| class_timetables | teacher_id |
| classes | teacher_id |
| clinic_visits | recorded_by |
| device_employee_mapping | user_id |
| digital_homework | teacher_id |
| direct_messages | to_user |
| dormitories | matron_staff_id |
| dormitory_attendance | recorded_by |
| email_schedules | created_by |
| employee_advances | employee_id |
| employees | profile_id |
| health_visits | recorded_by |
| in_app_notifications | created_by, user_id |
| learners | user_id |
| leave_requests | user_id |
| lesson_observations | teacher_id |
| lesson_plans | teacher_id |
| lesson_register | teacher_id |
| library_books | created_by |
| library_members | profile_id |
| meal_records | recorded_by |
| medical_incidents | recorded_by |
| notifications | user_id |
| parent_learner_links | parent_user_id |
| payments | recorded_by |
| payroll_lines | employee_id |
| ple_mock_tests | created_by |
| print_order_permissions | user_id |
| print_orders | created_by |
| quran_progress | teacher_id |
| salah_attendance | recorded_by |
| salaries | user_id |
| scheduled_notifications | created_by |
| scheme_of_work | teacher_id |
| staff_attendance | user_id |
| staff_letters | to_user |
| syllabus_coverage | teacher_id |
| tailor_tasks | recorded_by |
| teacher_assignments | teacher_id |
| visitors | created_by |

**Note:** these are already in `config.py`. Only add a table here if it's **not**
already present. If a source table has a user-ref column not listed here, add it.

---

## 7. Users / passwords (the special case)

Auth users (email + bcrypt password) are **not** loaded like data tables. They
use a dedicated dual-mode signup:

```bash
python3 migrate.py --workdir /tmp/migrate_iico --users
```

This reads `auth.users` (email + `encrypted_password` as `password_hash` + role
from `profiles`) and calls `POST /api/srv/<BOARD>/auth/signup` with
`password_hash`, so each user keeps their **real existing password** (the engine
verifies with `bcrypt::verify`, and Supabase hashes are `$2a$10$` bcrypt).

- Already-existing users are skipped (idempotent).
- Users without a stored hash get `MigrationTempPass2026` — change it after import.

---

## 8. Troubleshooting

### "payload failed schema: <field> is a required property"
The engine table has a stale strict schema. Add the table to `RECREATE_TABLES`
in `config.py` (drops + recreates it permissively), then re-run.

### "duplicate value '...' for unique field $.id"
The table already has that id. This is handled by `?upsert=1` (idempotent). If
you see it as a hard error, the table was created without a `unique_key` — set
`unique_key: "$.id"` (the loader does this for new tables).

### Tool "hangs forever"
The engine was previously O(n²) on bulk inserts. This is **fixed** (FTS off +
transaction batching + targeted unique lookup). If it hangs again:
1. Check the daemon is alive: `curl http://127.0.0.1:7070/healthz`
2. Reduce `MIGRATE_BATCH_SIZE` (e.g. `50` or `10`).
3. Check `/tmp/srv-daemon.log` for a crash.

### Count mismatch (loaded != source)
The loader fails loudly. This usually means some rows failed schema validation.
Check the daemon log, then re-run (upsert is idempotent).

### SSH/auth failures
Verify `sshpass` is installed and the creds in `config.py` are correct. Test:
```bash
sshpass -p '<SRC_SSH_PASS>' ssh -o StrictHostKeyChecking=no <SRC_USER>@<SRC_HOST> 'echo ok'
```

---

## 9. Reference: engine REST contract used by the loader

All requests carry `Authorization: Bearer <BOARD>` (board id → owner role).

| Purpose | Method + path | Body |
|---|---|---|
| Create table (permissive) | `POST /api/srv/<BOARD>/tables` | `{"name", "schema", "unique_key"}` |
| Drop table | `DELETE /api/srv/<BOARD>/tables/<t>` | — |
| Check table exists | `GET /api/srv/<BOARD>/tables/<t>` | — |
| Bulk insert (upsert) | `POST /api/srv/<BOARD>/tables/<t>/bulk?upsert=1` | `{"records": [...]}` |
| Count rows | `GET /api/srv/<BOARD>/tables/<t>/aggregate?op=count` | — |
| Signup w/ hash | `POST /api/srv/<BOARD>/auth/signup` | `{"email","role","password_hash"}` or `{"email","password"}` |

---

## 10. Migration status

> **Superseded:** the turso-era migration (counts below) is obsolete — the
> storage backend is now HelixDB and the data is re-migrated fresh. See
> `MIGRATION_STRATEGY.md` for the graph-first plan and the live status table.

Historical turso-era counts (for reference only):

| Table | Count |
|---|---|
| profiles | 343 |
| user_roles | 344 |
| learners | 294 |
| term_results | 2334 |
| classes | 7 |
| subjects | 26 |
| guardians | 42 |
| auth users | 343 |

**HelixDB graph-first migration:** pending — follow `MIGRATION_STRATEGY.md`
(manual core tables first, then parallel sub-agents per table group using the
worker checklist in `SUBAGENT_WORKER.md`).
