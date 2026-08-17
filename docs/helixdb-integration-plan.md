# HelixDB Integration Plan

Goal: back the serverless engine's `Database` trait with **HelixDB** (graph + vector +
full-text) running in **Docker**, persisted via **MinIO**, reached through a **new Rust
crate that posts raw `POST /v2/query` JSON** — no Helix SDK dependency.

Reference skills are in the submodule at **`tools/helixdb-skills/`** (HelixDB/skills).
A separate agent will implement the new crate; this document is its contract.

---

## 1. Why this approach (and why no SDK)

HelixDB v3 exposes a raw HTTP endpoint `POST /v2/query` that takes a plain JSON
envelope. We call that directly so the engine's storage layer has **zero Helix SDK
dependency** — just HTTP (the server already uses `reqwest`). This keeps the engine
embeddable and the Helix wiring behind one adapter.

- Envelope reference: `tools/helixdb-skills/docs/dynamic-query-examples.md`
- Rust DSL reference (for reading/authoring query shapes): `tools/helixdb-skills/docs/dsl-cheatsheet.md`
- Query optimization: `tools/helixdb-skills/skills/helix-query-optimize/SKILL.md`
- Agent that should author Helix query bodies: `helix-query-rust`, `helix-query-json-dynamic`

---

## 2. Target runtime (Docker + MinIO)

HelixDB runs as a container. Local/disk mode persists via a CLI-managed MinIO.

- Image: `ghcr.io/helixdb/helixdb:v0.0.3` (per the skills README)
- Dev instance: `helix init local` / `helix start dev` (in-memory) — enough to develop the adapter
- Persistence later: `--disk` mode backed by MinIO
- The daemon talks to Helix over HTTP at a configured base URL (e.g. `http://127.0.0.1:<port>/v2/query`)

> The Docker/MinIO *setup* is out of scope for the agent writing the crate. Only the
> **in-crate implementation** matters here.

---

## 3. New crate: `crates/helixdb`

A pure-Rust crate (workspace member) that:

1. Implements a **small HTTP client** for `POST /v2/query` (JSON in/out). `reqwest` is
   fine (already used by `server`).
2. Exposes a typed builder API mirroring the Helix JSON envelope:
   - `read_batch` / `write_batch`
   - anchors: `g().n(label)`, `.n_with_label`, `.e(id)`
   - traversal: `.out(edge)`, `.in_`, `.both`, `.repeat`
   - predicates: `eq`, `gt`, `gte`, `lt`, `lte`, `in`, `has_key`, `is_null`
   - projection: `.project`, `.value_map`
   - ordering/paging: `.order_by`, `.skip`, `.limit`, `.range`
   - mutations: `add_n`, `add_e`, `set_property`, `drop`
   - text/vector search: `text_search_nodes_with`, `vector_search_nodes_with`
3. Translates a `Query`/`Row`/`SrvFilter` into that envelope and back.

Crate layout (one concern per file, ≤200 lines/file):
```
crates/helixdb/
  Cargo.toml
  src/lib.rs        # re-exports, error type, client
  src/client.rs     # POST /v2/query HTTP transport + envelope
  src/request.rs    # JSON envelope builder (read/write batches)
  src/tenant.rs     # board_id -> Helix tenantId; tenant scoping on every request
  src/namespaces.rs # __srv__ reserved prefix; engine wb_* <-> reserved label mapping
  src/predicate.rs  # FilterCond -> Helix predicate
  src/row.rs        # engine Row <-> Helix node/edge mapping
```

---

## 4. Connect to the engine — exact wiring

The engine's `Database` trait already abstracts storage. Add a new adapter and select it
by config. The wiring point is **`crates/cli/src/main.rs`**:

- `main.rs:101` — `let backend = if cfg.backend == "memory" { "memory" } else { "turso" };`
  → change to a match that also handles `"helix"`.
- `main.rs:145` `backend_impl(...)` — add a branch:
  ```rust
  if backend == "helix" {
      Box::new(server::db::helix::HelixDatabase::new(base_url))
  } else { /* turso or memory */ }
  ```
- `config.rs` — `SRV_DB=helix` already flows through `Config.backend` (no change needed;
  add `SRV_HELIX_URL` env for the base URL).

New adapter file: **`crates/server/src/db/helix.rs`** (registered in `db/mod.rs`),
implementing `engine::storage::database::Database` by delegating to `crates/helixdb`.

### Multi-tenant model (critical — read first)

This is a **multi-tenant engine**. A **board == one tenant == one app**. When a board
is created it becomes a tenant; the board is where the app builds its **schema**,
**recipes**, **keys**, **secrets**, **jobs**, **webhooks**, and issues its **API queries**.

**Today there are two storage layers:**

| Layer | Where | Contents |
|---|---|---|
| **System / global** | `system.db` | board registry (`srv_board_index`, `wb_apps`) — which boards exist + owner keys. Shared. |
| **Per-board** | `boards/{board_id}.db` (one file per tenant) | app data rows (`wb_records`) **and** per-board metadata: `wb_tables` (schema), `wb_recipes`, `wb_keys`, `wb_app_secrets`, `wb_jobs`, `wb_hooks`, `wb_users` |

**Schema / recipes / keys are scoped per board:**
- Tables: `wb_tables`, keyed `scoped_key(board_id, table)` = `"{board_id}/{table}"`
- Recipes: `wb_recipes`, filtered by `board_cond(board_id)` + recipe name
- Keys / secrets / jobs / hooks / users: same `board_id` scoping

**API queries are NOT stored.** They are built at request time from the engine's query IR
(`SrvFilter`/`FilterCond`) driven by REST (`/api/srv/{board}/tables/{t}/query?filter=...`)
and MCP tools (`records_query`, `records_list`, `records_search`). Only **schema**
(`wb_tables`) and **recipes** (`wb_recipes`) are persisted — both per board.

#### HelixDB tenant mapping

**Board id → Helix tenant partition.** Every board gets its own Helix tenant scoping so
no two tenants' data, schema, or recipes mix:

- All engine reads/writes for a board carry the board's Helix `tenantId`.
- The engine's `board_cond(board_id)` and `scoped_key(board_id, ...)` map to the Helix
  tenant scope + label.

#### Reserved metadata namespace (collision safety)

Engine metadata tables live in the same storage as app data (just different table names):
`wb_apps, wb_records, wb_tables, wb_keys, wb_recipes, wb_recipe_runs, wb_app_secrets,
wb_hooks, wb_hook_deliveries, wb_links, wb_audit, wb_jobs, wb_job_runs, wb_users,
wb_sessions`.

**In Helix these must be isolated from user-created labels so they never collide.** A user
could create a table named `recipes` or `users`; that must not collide with engine
recipes/users. Rules:

- Engine metadata → a **reserved label namespace**, e.g. prefix with `__srv__`
  (`__srv__table`, `__srv__recipe`, `__srv__key`, `__srv__user`, `__srv__audit`, ...).
- User tables → their raw label (`recipes`, `users`, `learners`, ...) in the **user**
  namespace.
- The `crates/helixdb` crate owns the `__srv__` prefix constant + a sanitizer that maps
  engine `wb_*` table names to reserved labels and rejects/rewrites any user attempt to
  use the reserved prefix.

### Data model mapping (relational -> graph)

Represent each engine table as a **node label**; each `*_id` reference becomes an **edge**:

| Engine (table + FK) | HelixDB |
|---|---|
| `learners` row | node `Learner` (properties = payload) |
| `classes` row | node `Class` |
| `learners.class_id` | edge `Learner -[:ENROLLED_IN]-> Class` |
| `dormitory_residents.learner_id/dormitory_id` | edge `Learner -[:RESIDES_IN]-> Dormitory` |
| `guardians` ↔ `learners` | edge `Guardian -[:GUARDIAN_OF]-> Learner` |
| `teacher_assignments.(teacher,class,subject)` | `Teacher -[:TEACHES]-> Class`, `Class -[:USES]-> Subject` |

A generic `*_id` column on a table is a **candidate edge**: match the column name to the
target node label, else keep it as a property. Joins become `out`/`in_` traversals.

### Query IR mapping

The engine's existing `SrvFilter` (`FilterCond`/`Op`) maps directly to Helix predicates:
- `Op::Eq/Ne/Gt/Gte/Lt/Lte/In/Contains` → `Predicate::eq/.../is_in_param`
- `Op::Search` → `text_search_nodes_with(...)` / BM25
- aggregates (`count/sum/avg/min/max`, `group_by`) → `count`/`aggregate_by` on the batch
- `orders`/`limit`/`offset` → `.order_by`, `.skip`, `.limit`, `.range`

---

## 5. Implementation order (for the implementing agent)

1. Add `crates/helixdb` + workspace entry; implement the `POST /v2/query` client + envelope builder.
2. Implement `tenant.rs` (board→Helix tenantId scoping) and `namespaces.rs` (`__srv__` reserved prefix, `wb_*`→reserved-label mapping, user-label sanitizer). **Do this before the data mapping** — it is the isolation contract.
3. Smoke-test against a local Helix (in-memory container) with a tiny read + write.
4. Add `server/db/helix.rs` implementing `Database` (insert/get/query/delete/upsert) over `crates/helixdb`, threading `board_id`→tenant and metadata→`__srv__` labels throughout.
5. Wire `SRV_DB=helix` in `crates/cli/src/main.rs` (`backend_impl` + backend match).
6. Verify with the existing engine tests (the engine's behavioral contract) against the Helix backend — including multi-board isolation (two boards must not see each other's data, schema, or recipes).

---

## 6. Resources for the implementing agent

- **Skills submodule (read first):** `tools/helixdb-skills/`
  - `README.md` — skill list + setup (Docker/MinIO)
  - `docs/dynamic-query-examples.md` — the `POST /v2/query` envelope (our transport)
  - `docs/dsl-cheatsheet.md` — query shape reference (Rust DSL as the semantic model)
  - `docs/cypher-rosetta.md`, `docs/gremlin-rosetta.md` — traversal semantics
  - `skills/helix-query-rust/`, `skills/helix-query-json-dynamic/`, `skills/helix-query-optimize/`
- **Engine contract:** `crates/engine/src/storage/database.rs` (`Database` trait, `Query`, `Row`, `SrvFilter`)
- **Existing adapters to mirror:** `crates/server/src/db/turso.rs`, `crates/engine/src/storage/memory.rs`
- **Wiring:** `crates/cli/src/main.rs` (`backend_impl`), `crates/server/src/db/mod.rs`, `crates/server/src/config.rs`

---

## 7. Out of scope for the crate agent

- Docker / MinIO provisioning (a separate step; container will already be running)
- Changing the engine's `Database` trait or query IR
- Frontend / MCP changes
