# HelixDB Query Authoring Guide

How to write correct, fast queries against the HelixDB-backed engine. This is the
**agent-facing contract**: when you build queries that run inside a board (app
space), follow these rules. Every board == one Helix tenant; every engine table is
one Helix node label.

> Applies to the `helix` storage backend (`SRV_DB=helix`), which talks to HelixDB
> over raw `POST /v2/query` JSON (no SDK). See
> [helixdb-integration-plan.md](./helixdb-integration-plan.md) for the wiring.

---

## 1. Mental model: tenant + label + node

| Engine concept | HelixDB mapping |
|---|---|
| Board (one app) | One tenant: `tenantId = board_id` |
| Engine metadata table (`wb_*`) | Reserved label `__srv__*` (e.g. `wb_keys` → `__srv__keys`) |
| App records (ALL user tables) | One label `__srv__record`; the `table` routing prop disambiguates which table a node belongs to |
| One record row | One node with label `__srv__record` |
| Record payload | Stored in the node's `data` property (JSON string) |
| Full-text | `_search` property (flattened payload text), BM25-indexed |

> **Why one label for all records?** The adapter stores every record under the
> single reserved `__srv__record` label, partitioned by the `table` property
> (plus `tenantId`). This is collision-safe (a user table named `users` can
> never clash with engine `wb_users` → `__srv__users`) and keeps tenant
> isolation uniform: a query for table `learners` on board `b_x` is
> `nodes_where { $label: __srv__record, tenantId: b_x, table: learners }`.

Every node carries:

- `tenantId` — the board id. **Always filter on it.**
- `data` — the full engine JSON blob (lossless round-trip).
- `_srv_key` — the engine key (scalar: `i64` or `string`).
- `board_id`, `table`, `seq` — scalar routing props for pushdown filters.
- `_search` — (record nodes only) flattened payload text for BM25.
- `p_<name>` — (record nodes only) mirrored top-level numeric payload fields,
  e.g. `p_price`, used for aggregate pushdown.

---

## 2. Golden rules

1. **Never query without a tenant scope.** A read/write for board `b_x` must
   filter `tenantId == "b_x"` (or pass `tenant_value` to search). Omitting it
   leaks across boards.
2. **User tables are virtual.** A "table" is the `table` routing prop on a
   `__srv__record` node, not a separate label — there is nothing to reserve or
   sanitize at the label level for app data (the `__srv__` prefix stays reserved
   for engine metadata labels).
3. **Anchor narrow, filter early.** Prefer `nodes_where` with equality on
   `$label` (`__srv__record`) + `tenantId` + `table` over label scans.
4. **`data` is opaque.** Filter on the scalar routing props (`board_id`, `table`,
   `seq`) or mirrored `p_*` props when you can; anything inside the payload is NOT
   indexable by Helix directly — the engine falls back to a Rust scan for those.
5. **Search means BM25 on `_search`.** Use `text_search_nodes` with
   `tenant_value`; never `contains`-scan for full-text.

---

## 3. The engine's query IR → Helix translation

The engine builds `SrvFilter`/`FilterCond` and hands them to the adapter. The
adapter translates:

| Engine `Op` | Helix predicate |
|---|---|
| `eq` / `ne` | `eq` / `ne` on the routing prop, `_srv_key`, or mirrored `p_*` prop |
| `gt` / `gte` / `lt` / `lte` | `gt` / `gte` / `lt` / `lte` (numeric when the value is a number) |
| `in` | `or` of `eq` predicates |
| `contains` / `not_contains` | **Rust scan** (fetch candidates, match in engine) |
| `search` | `text_search_nodes` BM25 on `_search` (index bootstrapped lazily) |
| `count`/`sum`/`avg`/`min`/`max` | `aggregate_by` pushed down when the field is a mirrored `p_*` prop; else Rust |

Routing props that push down: `board_id`, `table`, `seq`. Top-level **numeric**
payload fields are mirrored as `p_<name>` node props at write time, so
`sum(price)`/`avg(grade)`/`count` run inside Helix. Everything else — including
nested payload paths — is matched/aggregated in Rust after a tenant-scoped
fetch. This is correct but slower; keep result sets bounded.

**Practical consequence for query authors:** when you write a filter or
aggregate, prefer top-level numeric fields (mirrored) and routing props. Nested
payload-path filters and aggregates work but scan.

---

## 3b. Guardrails and the right tool per job

Agents drive the engine through the CLI (MCP tools over `/mcp`). There is **no
raw `POST /v2/query` surface** — the adapter owns that. Pick the tool that
matches the job:

| Need | Tool | Notes |
|---|---|---|
| A few rows by filter | `srv records query <board> <table> --filter '{...}'` | Blocked if an unfiltered query would return >1000 rows (see below) |
| Summary math | `srv records aggregate <board> <table> sum --field price [--group_by category] [--filter ...]` | count/sum/avg/min/max, pushed to Helix |
| Latest page | `srv records list <board> <table> --limit N` | Bounded, newest-first |
| Full-text | `srv records search <board> <table> <q>` | BM25 |
| One row | `srv records get <board> <table> <seq>` | Exact by seq |
| Bulk | `srv records import <board> <table> --format json --data '[...]'` | Streaming insert; use for large sets, not `query` |

**Guardrail:** `records.query` without a `--filter` (and without an explicit
order) checks the table size first. If it would return more than 1000 rows, it
refuses with a message pointing at `records.aggregate` / `records.list` /
`records.search`. To force it, pass `--allow_unfiltered` (not recommended for
large tables — it floods the agent's context).

---

## 4. Envelope shapes you will emit (or recognize)

The adapter speaks `POST /v2/query` directly. Canonical shapes (verified against
`ghcr.io/helixdb/helixdb:v0.0.4`):

### 4.1 Tenant-scoped node source

All app records live under `__srv__record`; the `table` prop narrows to the
virtual "table":

```json
{
  "nodes_where": {
    "predicate": {
      "and": { "predicates": [
        { "eq": { "left": { "property": "$label" }, "right": { "constant": { "string": "__srv__record" } } } },
        { "eq": { "left": { "property": "tenantId" }, "right": { "constant": { "string": "b_x" } } } },
        { "eq": { "left": { "property": "table" }, "right": { "constant": { "string": "learners" } } } }
      ]}
    }
  }
}
```

### 4.2 Add a node (write)

```json
{
  "add_n": {
    "label": "__srv__record",
    "properties": [
      ["data", { "value": { "string": "{\"payload\":{\"name\":\"Alice\"},\"table\":\"learners\"}" } }],
      ["table", { "value": { "string": "learners" } }],
      ["tenantId", { "value": { "string": "b_x" } }]
    ]
  }
}
```

Properties are ordered pairs `[name, { "value": { ... } }]`. Scalar tags:
`string`, `i64`, `f64`, `bool`, `null`. **Always include `tenantId`.**

### 4.3 Update a node (write)

```json
{
  "set_property": {
    "input": { "nodes": { "reference": { "var": "existing" } } },
    "name": "data",
    "value": { "value": { "string": "{...}" } }
  }
}
```

> **Gotcha:** the wire field is `name`, NOT `property` (differs from the skills
> docs — the SDK serializer confirmed it).

### 4.4 Delete

```json
{ "drop": { "input": { "nodes": { "reference": { "ids": [12, 13] } } } } }
```

`drop` returns an **empty array** even when it drops — never use its return to
count. Fetch ids first, then drop.

### 4.5 BM25 full-text search (read)

```json
{
  "text_search_nodes": {
    "label": "__srv__record",
    "property": "_search",
    "tenant_value": { "value": { "string": "b_x" } },
    "query_text": { "value": { "string": "laptop" } },
    "k": { "literal": 10 }
  }
}
```

Then project with `value_map(["$id", "data", "_srv_key", "$score"])` — bare
search hits only carry `$id` + `$score`.

### 4.6 Create the text index (write, once)

```json
{
  "create_index": {
    "spec": { "node_text": { "label": "__srv__record", "property": "_search", "tenant_property": "tenantId" } },
    "if_not_exists": true
  }
}
```

Index builds are **async**: `create_index` returns `operation_id`, then poll
`get_index_operation`. The engine's adapter fires `create_index` lazily on first
search and falls back to a Rust scan until the index is ready — so search always
works, and becomes BM25 once the build lands.

---

## 5. Tenant isolation contract (critical)

- A **board is a tenant**; board `b_x` reads/writes must carry `tenantId = b_x`.
- The `__srv__` label namespace is shared across tenants; isolation comes from
  the `tenantId` **property**, not the label. Two boards can both have
  `__srv__record` nodes — they are separated by `tenantId`.
- App "tables" are virtual partitions inside `__srv__record` via the `table`
  prop. `learners` on board `b_x` and `learners` on board `b_y` are different
  tenants entirely — a query that omits `tenantId` would cross boards.
- The one global scan: the board registry (`__srv__apps`) is enumerated WITHOUT
  tenant scope — that's the only label read across tenants, and it only ever
  returns board ids.

**Test you must pass:** create board A and board B, put a record in each, query
each board — A must never see B's rows, schema, recipes, keys, or jobs.

---

## 6. Authoring checklist

When you write a query that runs in an app's board:

- [ ] Tenant scope present on every read and write (`tenantId` predicate or
      `tenant_value`).
- [ ] App record queries use `$label = __srv__record` + `table = <name>` (engine
      metadata uses its own `__srv__*` label) — never mixed.
- [ ] Filter conditions use routing props (`board_id`, `table`, `seq`) where
      possible; payload-path filters are last resort (they scan in Rust).
- [ ] Search uses `text_search_nodes` on `_search` with `tenant_value`, not
      `contains`.
- [ ] Mutations are conditional (`var_empty`/`var_not_empty`) for upserts —
      there is no native `MERGE`; read-then-branch in one batch.
- [ ] `set_property` uses `name`, not `property`.
- [ ] `drop` counts are computed from a prior fetch, not from the drop response.
- [ ] Ordering/limits use `order_by` + `skip` + `limit` with `count.literal`
      bounds.

---

## 7. Environment / resources

- Daemon: `srv daemon start` with `SRV_DB=helix` + `SRV_HELIX_URL`
  (default `http://127.0.0.1:7979`).
- Adapter: `crates/server/src/db/helix.rs` (engine `Database` impl).
- Envelope client: `crates/helixdb` (builders in `request.rs`, predicates in
  `predicate.rs`, namespace contract in `namespaces.rs`).
- Reference docs: `docs/helixdb-integration-plan.md`,
  `tools/helixdb-skills/docs/dynamic-query-examples.md`,
  `tools/helixdb-skills/docs/dsl-cheatsheet.md`.
- Live validation: `curl -X POST http://127.0.0.1:7979/v2/query` with the shapes
  above; or drive through the CLI (`srv records search <board> <table> <q>`).
