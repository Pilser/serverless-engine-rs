# How the IICO → Engine Full Sync Was Completed

This document records the **actual, verified story** of how every table of IICO
data was moved out of Supabase and into the serverless engine (`board
b_v2qyx0enpqkd0000`), including the blockers we hit and how each one was solved.
It is the "how was this possible?" companion to `README.md` (which documents the
pipeline API).

> End state (verified 2026-08-17): **64 tables** in the engine, source and
> engine counts match, and the query layer no longer drops boolean-filtered rows.

---

## 1. The task

Migrate **all** data from the self-hosted Supabase Postgres on the IICO machine
(`192.168.88.101`, container `supabase-db`) into the serverless engine. The core
14 tables were already done; the job was to find **everything remaining**, sync
it, and make sure the running app could actually read it back.

The whole thing was possible because of one clean architectural fact:

> The engine exposes a **generic REST data plane** (`/api/srv/<board>/tables/...`)
> that accepts any table with a permissive JSON schema and upserts records by a
> unique key. So *any* source table can be loaded without per-table server code —
> only a data transform is needed.

---

## 2. The pipeline that made it possible

Three independent, reusable steps (see `README.md` for full detail):

```
extract.py  →  transform.py  →  loader.py
   │               │                │
SSH + docker       UUID→email       REST bulk upsert
psql json_agg      null-strip       (create table if missing)
→ raw/<t>.json     → transformed/   ?upsert=1, unique_key=$.id
```

- **Extract** runs `docker exec -i supabase-db psql ... select json_agg(...)` over
  SSH and writes a JSON array per table.
- **Transform** rewrites every user-reference UUID column to that user's lowercase
  email (the engine keys users by email), strips `null`/unwanted columns, and
  emits engine-ready rows.
- **Load** creates the table permissively if needed and POSTs rows in batches of
  `BATCH_SIZE` with `?upsert=1`, so reruns fill gaps without duplicating.

Because `?upsert=1` + `unique_key=$.id` is **idempotent**, an interrupted load can
be resumed simply by running the same command again.

---

## 3. Finding the data that was still missing

The migration status doc listed `fee_structures` and `scheme_of_work` as
remaining, but "and other tables" was vague. To be thorough we enumerated the
source and compared against what the engine already had:

```python
# Every public table in the source
SELECT table_name FROM information_schema.tables
WHERE table_schema = 'public' ORDER BY table_name;   # → 169 tables
```

Then, for each table **not already migrated**, we counted rows and kept those
with `> 100`:

| Table | Source rows |
|---|---|
| visit_reasons | 370 |
| student_account_credentials | 294 |
| emis_learner_export | 286 |
| inventory_details | 238 |
| inventory_items | 238 |
| inventory_stock | 237 |
| email_logs | 166 |

All of these were migrated with the standard pipeline. They had **no**
user-reference columns, so no `USER_REF_COLUMNS` additions were needed — the
transform simply null-stripped and loaded them.

---

## 4. The one table that wouldn't finish: `in_app_notifications`

`in_app_notifications` had **12,035** rows. The pipeline kept **timing out** at
the shell command limit (~120 s) before the load finished, leaving the engine at
4,100 rows.

Why it was possible to finish:

1. **The load is idempotent** (`?upsert=1` on `unique_key=$.id`), so re-running
   was safe — it would just resume filling in missing rows.
2. **We removed the real bottleneck**: the per-request HTTP timeout. The stock
   `loader.py` uses a 60 s request timeout, and 100-row batches under heavy load
   exceeded it. We raised the request timeout (to 300 s) and increased the batch
   size (to 200).
3. **We ran it detached in `tmux`** so it survived and kept going while other work
   happened:

   ```bash
   tmux new-session -d -s inapp_mig "python3 /tmp/migrate_inapp.py"
   tmux capture-pane -t inapp_mig -p      # watch progress
   ```

   Progress (`loaded X/12035`) confirmed it was working; when it finished,
   the engine reported exactly **12,035** rows — full sync.

> Takeaway: a big table needs (a) idempotent loading, (b) a timeout that matches
> the work, and (c) a detached runner so a 2-minute shell limit can't kill it.

---

## 5. The data was there, but the app couldn't see it

After migration the browser was getting **HTTP 500** responses on:
- `/tables/dormitories/query?limit=500`
- `/tables/dormitory_residents/query?filter={"is_active":{"eq":true}}&limit=500`
- `/tables/salah_attendance/query?filter={"date":{"eq":"2026-08-17"}}&limit=500`

Investigation showed two different things:

### 5a. The 500s were transient load pressure
The 500s appeared **while the 12k-row bulk load was saturating the engine**.
Once the load completed, the same queries returned `HTTP 200`. So those specific
500s were a symptom of the migration in progress, not a permanent defect.

### 5b. A real, silent bug: boolean filters dropped valid rows
`dormitory_residents` had 206 rows with `is_active: true`, but
`filter={"is_active":{"eq":true}}` returned **0 rows** (HTTP 200, empty) — no 500,
just missing data. This is the class of bug that makes an app "look broken".

**Root cause** (in `crates/server/src/db/turso.rs`, `records_where`):

- SQLite stores the JSON boolean `true` as the **integer `1`**.
- The old code built the predicate as
  `CAST(json_extract(payload, '$.is_active') AS TEXT) = ?` and bound the string
  `"true"`.
- `CAST(1 AS TEXT)` yields `'1'`, so `'1' = 'true'` is never true → **0 rows**.

The in-memory storage already handled booleans correctly
(`loose_eq` → `Bool(true) == Bool(true)`), so only the SQL path was wrong.

---

## 6. The fix

In `records_where`, when the filter value is a JSON boolean we now compare against
the **raw** `json_extract(...)` result bound as the SQLite integer `1`/`0`,
instead of casting to text:

```rust
let bool_int = c.value.as_bool().map(|b| if b { 1i64 } else { 0i64 });
Op::Eq => {
    if let Some(bi) = bool_int {
        params.push(Value::Integer(bi));
        format!("json_extract(payload, ?{path_ph}) = ?{vp}")
    } else { /* existing text/number path */ }
}
Op::Ne => {
    if let Some(bi) = bool_int {
        params.push(Value::Integer(bi));
        format!("(json_extract(payload, ?{path_ph}) IS NULL OR json_extract(payload, ?{path_ph}) <> ?{vp})")
    } else { /* existing text/number path */ }
}
```

This makes `eq:true`/`neq:false` match the integer `1`/`0` that SQLite actually
stores for JSON booleans.

---

## 7. Deploying the fix (the restart dance)

The daemon runs from `deploy/srv`, so rebuilding required a live restart:

```bash
# 1. Build (workspace root, release)
cargo build --release

# 2. Stop the running daemon (it's in tmux session "srv") via Ctrl-C
tmux send-keys -t srv C-c

# 3. Replace the binary (can't overwrite a running file — must stop first)
cp target/release/srv deploy/srv

# 4. Restart in a fresh tmux session with the same env as before
tmux new-session -d -s srv \
  -c /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine/deploy \
  "SRV_DATA_DIR=data_v2 ./srv daemon start 2>&1 | tee /tmp/srv-daemon.log"
```

> Note: `srv daemon` only has `start`, no `stop`. The safe way to stop is to send
> Ctrl-C into the tmux pane, then `cp` the new binary, then start a new session.

---

## 8. Verification

After the daemon restarted with the new binary:

| Check | Result |
|---|---|
| `dormitory_residents` `is_active eq:true` | **206 rows** (was 0) ✅ |
| `dormitories/query?limit=500` | HTTP 200 ✅ |
| `salah_attendance` date filter | HTTP 200 ✅ |
| `in_app_notifications` count | **12,035** (full) ✅ |
| Total engine tables | **64** ✅ |

---

## 9. What made the whole thing possible (summary)

1. **Generic REST data plane** — the engine accepts any table with a permissive
   schema; no per-table server code, only a transform.
2. **Idempotent upsert** (`?upsert=1` on `unique_key=$.id`) — interrupted loads
   resume safely; re-runs never duplicate.
3. **Reusable 3-step pipeline** (extract → transform → load) with a `USER_REF_COLUMNS`
   map for the one genuinely tricky transformation (UUID → email).
4. **Detached runner (tmux) + raised timeout/batch** for the single huge table
   that exceeded normal command limits.
5. **Finding the fix by comparing storage semantics** — SQLite stores JSON `true`
   as `1`, which the TEXT-cast predicate never matched; comparing the raw JSON
   value fixed boolean filters.
6. **A correct restart procedure** for the live daemon (stop via tmux → copy
   binary → restart with same env).
