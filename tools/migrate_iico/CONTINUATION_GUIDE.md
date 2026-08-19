# IICO → Serverless Engine — Continuation Guide (Phase 3)

**Updated:** 2026-08-18 — data migration COMPLETE; now porting business logic
(approval recipes) and wiring the frontend to the engine. This doc is the
living status: what's done, what's in flight, what's remaining, and where every
piece of the work lives.

---

## 1. Where everything lives

```
serverless-engine/
  crates/
    helixdb/                  # Helix HTTP client (POST /v2/query)
    engine/                   # core engine (crud, auth, recipes, expr, storage traits)
    server/                   # daemon (REST/MCP/WS) + db/helix.rs Helix adapter
    mcp/                      # MCP tool server (agent control surface)
    cli/                      # `srv` CLI (MCP client over HTTP)
  deploy/
    helix-iico-data/          # daemon SRV_DATA_DIR (object store; records live in Helix/MinIO)
    iico/                     # the IICO frontend (git clone, own repo)
      SUPABASE_TO_ENGINE_MIGRATION_PLAN.md   # the phase-by-phase plan (kept current)
      src/lib/engineClient.ts                # engine auth/API facade (login, session, basePath)
      src/integrations/supabase/client.ts    # supabase-js-shaped facade → engine REST
      src/hooks/useAuth.tsx                  # auth state wired to engine
      scripts/build-static.sh                # build + push to board assets
  tools/migrate_iico/
    CONTINUATION_GUIDE.md      # THIS FILE (living status)
    compare_counts.py          # source vs engine row counts
    config.py / extract.py / transform.py / loader.py / migrate.py
    data/raw/                  # 176 local source dumps (+ _users.json, the 10 auth users)
    data/transformed/          # loader input
  docs/
    blocking-client-async-runtime.md   # §8: daemon resource-bound fixes (READ THIS)
    problems/recipe-writeback-drops-computed-fields.md  # recipe write-back saga (RESOLVED)
```

---

## 2. Current state (2026-08-18)

- **Board:** `b_7z5lr8zaurkd0000` (HelixDB tenant). **Records: 21683 across
  176 tables — ALL loaded & verified** (match=176, partial=0, missing=0).
- **Real data lives in Helix/MinIO** (`helix-db` bucket, ~52M), NOT in
  `SRV_DATA_DIR` (that only holds the object store, now
  `deploy/helix-iico-data/objects`).
- **Auth users:** the 10 real Supabase auth users imported (bcrypt hashes,
  `data/raw/_users.json`). `admin@alheib.com` / `admin123` is the known admin
  login. Engine auth fixed: `auth.me` works (was "cannot route wb_sessions get").
- **Daemon:** runs in tmux `srv`, port 7070, data dir
  `deploy/helix-iico-data`, `SRV_BG_*` loops OFF (idle: 0% CPU, ~14MB RSS).
- **Frontend:** cloned at `deploy/iico/`, now **served by the engine** at
  `http://127.0.0.1:7070/srv/b_7z5lr8zaurkd0000/` (relative-base static build,
  index + ~271 assets pushed). Login screen renders; auth is engine-backed.

---

## 3. DONE (verified)

### 3.1 Engine hardening (daemon no longer CPU/memory bound)
- `table_ttl_clause` returns `None` for no-TTL tables → Helix aggregate pushdown
  + query cache actually engage (was full O(n) scan per count).
- `resources` count = ONE Helix aggregate (`count_records`), not 176 per-table
  scans; `/resources` ~0.6s. Also fixed a self-deadlock (`board_of()` while
  holding the engine Mutex).
- Background loops (TTL sweeper/job scheduler/webhook worker) are opt-in:
  `SRV_BG_TTL=1 / SRV_BG_JOBS=1 / SRV_BG_HOOKS=1`. WS resource ticker 60s,
  SSE keepalive 60s.

### 3.2 Auth (Phase 1)
- 10 auth users imported with bcrypt hashes + roles (admin, center_director,
  accountant, storekeeper, direct_manager, office_manager, gateman, teacher,
  parent, orphan_supervisor).
- `auth.set_role` added (REST `POST /auth/role` + MCP tool) — replaces
  `assign_user_role`. Expires the user's sessions on role change.
- Helix adapter `get()` routes bare-UUID keys across tenants (wb_sessions etc.)
  so `auth.me` works.
- Full 28-role set collected from SQL; frontend `roleConfig.ts` matches.

### 3.3 Frontend serving + auth wiring
- `vite.config.ts`: `base: "./"`, manualChunks as function (Vite 8),
  `import.meta.dirname`.
- `src/lib/engineClient.ts`: auth facade + `basePath()`/`goTo()` (board prefix).
- `src/hooks/useAuth.tsx`: signIn/signOut/session-restore via engine.
- `scripts/build-static.sh`: build + push to `PUT /api/srv/<board>/assets/`.
- Router `basename={basePath()}` + all 15 hard `window.location.href` redirects
  preserve `/srv/<board>`.
- `src/integrations/supabase/client.ts`: full supabase-js-shaped facade over the
  engine REST (query builder: eq/neq/gt/gte/lt/lte/in/ilike/like/is/not/or/
  match/filter/contains, order/limit/range/single/maybeSingle, insert/update/
  delete/upsert with id→seq resolution, rpc→/call, realtime poll, storage stub).
  Build passes; CRUD verified against the live daemon.
- Fixed pre-existing build breakers (duplicate imports in 6 files, missing
  jspdf, Vite 8 manualChunks).

### 3.4 Phase 2 — Approval state machines (11 recipes, VERIFIED)
All defined via MCP `recipes.add`, `when: record.updated + table`, `match` on
current stage, `$set` stage+status. One PATCH cascades through the chain.

| Chain | Table | Recipes | Flow (verified) |
|---|---|---|---|
| Stock | inventory_transactions | stock_mgr_approve, stock_dir_approve, stock_acct_verify, stock_dispatch, stock_gate_verify, stock_complete | submitted → manager_approved → director_approved → accountant_verified → dispatched → gate_verified → completed |
| Custody | employee_advances | custody_dir_approve, custody_acct_verify, custody_final_approve | submitted → director_approved → accountant_verified → final_approved |
| Liquidity | liquidity_requests | liquidity_dir_approve, liquidity_final_approve | submitted → director_approved → final_approved |

### 3.5 Phase 3 — Trigger derivations (partially done)
- **`update_inventory_stock` → recipes** `stock_add_qty` (+ on restock/return)
  and `stock_sub_qty` (− on issuance/damage/adjustment) via `$patch_other` with
  `{{$.delta}}` substitution. Verified: 10 → 15 (restock +5) → 12 (issuance −3).
- **`generate_tracking_number` → recipe** `gen_tracking_number` (computed
  `tracking_number` = `concat(if(type…), randhex(8))`). Verified: ISS-/REC-/TRX-.
- **Engine fixes that made this work** (see
  `docs/problems/recipe-writeback-drops-computed-fields.md`):
  - `write_node` updated-branch now emits one `set_property` per prop (data +
    mirrors), `existing` FIRST in the batch.
  - `record_set_raw` deep-merges the working payload into the current payload
    (multiple create recipes accumulate instead of clobbering).
  - expr funcs added: `substr`, `replace`, `uuid`, `randhex`.

---

## 4. IN PROGRESS / NEXT

### Phase 3 remaining — auto-derivation triggers (recipes)
The SQL triggers below still need recipe ports (each verified with a live test):

- [x] `sync_learner_profile` — learners.full_name → profiles.full_name on
      learner update (recipe `$patch_other`; VERIFIED).
- [x] `auto_assign_dormitory_residents` — on learner create with `dormitory`
      name: `$resolve_other` (name→dormitories.id, NEW engine action) +
      `$upsert_other` → dormitory_residents (VERIFIED).
- [x] `auto_create_library_member` — on profile insert: `$upsert_other` →
      library_members row (VERIFIED).
- [x] `auto_create_student_account` — on learner insert without user_id: recipe
      auto_student_account computes `student.<admno>@sised.sc.ug`, `$create_user`
      (NEW engine action: auth user + role student, password `Alheb@<admno>`),
      sets learner.user_id = email. VERIFIED (student can log in).
- [ ] `handle_new_user` / `handle_new_user_role` — profile + role defaults
      (mostly absorbed by Phase 1 auth; verify).
- [ ] `update_updated_at_column` — DEFERRED: needs engine `now()` string expr +
      per-table computed setter (only board-level `records` computed exists).
      The migrated data already carries `updated_at`; the app mostly reads it.
      Decision: add `now()` to expr funcs + expose per-table computed (small
      engine change) OR leave as-is (stale on write).
- [ ] `proc_audit_log` — engine audit flag per board (off currently).

### Phase 4 remaining — frontend data layer (facade exists, needs live data pass)
- [x] LIVE VERIFIED 2026-08-18 (Chrome via agent-browser): admin@alheib.com logs
      in, dashboard renders the full nav, Learners & Classes shows real migrated
      data (learner names, fee amounts). Data reads/writes work through the
      facade (eq/gte/in/order/ilike/aggregate + insert/resolveSeq/patch/delete).
- [x] Fixed runtime crashes: QueryBuilder.then() now returns a real Promise
      (.then().catch() chains); added top-level removeChannel + channel registry.
- [x] apiFetch prefixes all relative paths with /api/srv/<board> (was hitting
      the daemon root -> 404 on every call).
- [x] RPCs mapped to engine ops: advance_*_request -> PATCH stage (recipes
      cascade), assign_user_role -> /auth/role, create_asset -> insert,
      delete_* -> delete. Unknown RPCs -> "not yet ported".
- [x] Fixed browser-reported 404/500s + MIME blocks:
  - `is` null operator added to the engine (`{"field":{"is":null}}` and
    `{"field":null}` now match nulls) — fixes the term_results 400.
  - Missing tables return 404 (not 500); the facade treats 404 as an empty
    result — fixes homework/site_settings 500s.
  - Storage (upload/download/getPublicUrl) wired to the engine files REST;
    /file resolves downloads by record name; bucket tables created.
  - index.html serves no-cache; build-static.sh clears stale assets (kills
    the old-chunk MIME-block / stale-JS issues).
  - PageGuide conditional hooks fixed (React #310 crash after login).
  - Verified in Chrome: boss login -> dashboard -> Learners & Classes with
    0 console errors (no MIME blocks, no 404/500 XHRs).
- [ ] Known latency: login ~2.5s (bcrypt verify on cost-10 hashes + Helix
      round-trips). The app's Sign In button can appear stuck-disabled until
      it resolves. Investigate bcrypt cost / caching if it bothers users.
- [x] PERF FIX (2026-08-18): the slowness was NOT the blocking-client doc
      (that's a correctness issue, already fixed). Root cause: the Helix
      blocking client had pool_max_idle_per_host(0) -> a fresh TCP connection
      per request. With keep-alive enabled (30s idle), login dropped 6.9s ->
      0.43s and a 12-query dashboard burst finished in 0.19s wall (was 1-10s
      per query). Also: realtime poll 15s -> 30s; removed 2 corrupt
      site_settings nodes (test leftovers) causing "missing field seq" 500s.
      Browser-verified: dashboard + Learners & Classes load in <1s, 0 errors.
- [ ] Still stubbed: storage.remove; some RPCs (get_tailor_stats,
      tailor_materials_cost, etc.) return "not yet ported".

### Phase 5 — graph optimization (future)
- Graph-based stock: derive quantity by traversing `RELATED_ITEM` edges instead
  of the `inventory_stock` counter. **Blocked on:** `graph_resolve_target` needs
  an `item_id → inventory_items` alias (currently only pluralize + user/teacher
  → profiles). Documented as the recommended direction; deferred.
- Port JOIN-heavy views (active_gate_passes, notify_teachers…) to traversals.

---

## 5. How to run / verify

```bash
# daemon (tmux srv)
cd /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine
SRV_DB=helix SRV_HELIX_URL=http://127.0.0.1:7979 SRV_PORT=7070 \
  SRV_DATA_DIR=deploy/helix-iico-data ./target/release/srv daemon start
curl http://127.0.0.1:7070/healthz                 # {"ok":true}
curl http://127.0.0.1:7070/srv/b_7z5lr8zaurkd0000/  # the app

# MCP (agent control surface) — 43 tools
curl -X POST http://127.0.0.1:7070/mcp -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
# e.g. recipes.add / auth.set_role / records.query / graph.sync — full help in
# the tool descriptions (enriched 2026-08-18).

# counts vs source
cd tools/migrate_iico && python3 compare_counts.py   # expect 176/176 match

# rebuild after Rust change
cargo build --release    # from workspace root, no -p; never cargo test/clean
```

---

## 6. Key engine files to touch for Phase 3/4 work

| Area | File |
|---|---|
| Recipe dispatch/write-back | `crates/engine/src/automation.rs`, `crud.rs` (`record_set_raw`) |
| Helix adapter (write_node/get/query) | `crates/server/src/db/helix.rs` |
| Expr functions | `crates/engine/src/expr/funcs.rs` |
| MCP tool specs (help text) | `crates/engine/src/registry.rs` |
| REST auth/tables | `crates/server/src/transport/rest_auth.rs`, `rest_tables.rs` |
| Frontend engine facade | `deploy/iico/src/lib/engineClient.ts` |
| Frontend supabase facade | `deploy/iico/src/integrations/supabase/client.ts` |
| Frontend auth hook | `deploy/iico/src/hooks/useAuth.tsx` |
| Router base | `deploy/iico/src/App.tsx` |

---

## 7. Known issues / gotchas

- **Daemon background loops are OFF** (`SRV_BG_*`). Enable per feature in Phase 6.
- **`deploy/data`, `data_v2`, `data_backup_*` are OLD Turso-era SQLite** for
  different boards — not the current HelixDB data. Don't use them.
- **`inventory_stock` is a denormalized counter** — kept for now; the graph
  derivation is the future fix.
- **Helix clock-skew** can wipe MinIO data (see old doc) — keep local dumps.
- **Recipes fire per table via `when.table` (top-level arg), NOT inside `when`
  object** — the MCP `recipes.add` reads `table` from arguments.
- **`$patch_other`/`$upsert_other` substitute `{{$.path}}`** from the triggering
  payload; `$compute` uses expr syntax; `$set_state` sets `state` (not stage).
- **`records.query` filter param is URL-encoded JSON** (not base64) — the facade
  sends `encodeURIComponent(JSON.stringify(...))`.
- **update/delete by app UUID id → resolve engine seq first** (the facade does
  this; the REST PATCH/DELETE take numeric seq).
- Known source orphans (assets.category_id→inventory_categories; 7
  in_app_notifications.user_id) — verified, do NOT re-extract.

---

## 8. All docs created/updated this session

| Doc | Purpose |
|---|---|
| `docs/problems/recipe-writeback-drops-computed-fields.md` | The recipe write-back bug saga: root causes + fixes + graph future |
| `docs/blocking-client-async-runtime.md` §8 | Resource-bound daemon paths found + fixed (keep §1-7) |
| `deploy/iico/SUPABASE_TO_ENGINE_MIGRATION_PLAN.md` | The phase-by-phase plan, kept current |
| `deploy/iico/scripts/P4_OPCODE_TASK.md` | The opencode task that built the supabase facade |
| `tools/migrate_iico/compare_counts.py` | source-vs-engine count checker |
| `this file` | the living continuation guide |
