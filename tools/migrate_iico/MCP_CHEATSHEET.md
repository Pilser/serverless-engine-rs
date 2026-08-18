# Interacting with the engine via MCP

The engine exposes an **MCP endpoint** at `http://127.0.0.1:7070/mcp`. An agent
with the MCP client connected can drive the engine through typed tools instead
of raw REST. This is the quick cheat-sheet; run `help` on any tool for full
parameter docs.

## Data plane (the ones you'll use most)

| Want to... | Tool |
|---|---|
| See all apps/boards you own | `apps_list` |
| Inspect a board's config | `apps_show` |
| List tables on a board | `tables_list` |
| Create a table (optional schema/unique_key) | `tables_create` |
| Insert one record | `records_submit` |
| List records (newest-first, paged) | `records_list` |
| Filter records by conditions | `records_query` |
| Full-text search | `records_search` |
| Bulk-import JSON/CSV | `records_import` |
| Get / delete one record by seq | `records_get` / `records_delete` |
| Create auth user / login / me | `auth_signup` / `auth_login` / `auth_me` |
| Manage API keys, secrets, jobs, recipes | `keys_*`, `secrets_*`, `jobs_*`, `recipes_*` |
| Store / read static assets | `files_put` / `files_get` / `files_list` |
| Link nodes into the graph | `graph_link` / `graph_traverse` / `graph_sync` / `graph_search_edges` / `graph_unlink` / `graph_delete` |
| Aggregate records (count/sum/avg/min/max) | `records_aggregate` |

## Board + auth (context you almost always need)

- The IICO tenant board id is set in `tools/migrate_iico/config.py` (`BOARD`).
  Create it with `srv apps create 'iico'` (the old turso-era board
  `b_v2qyx0enpqkd0000` is obsolete).
- **All** record/table tools take a `board` param — always pass the board id.
- Read is fine as the board owner (bearer board id); writes require the matching
  role on that board.

## Filtering quick-reference (`records_query` `filter`)

```json
{"field": "is_active", "op": "eq", "value": true}      // single condition
{"is_active": {"eq": true}}                            // shorthand map
[{"field":"class_id","op":"eq","value":"..."}, {...}]  // AND of several
```

Supported ops: `eq, neq, gt, gte, lt, lte, contains, not_contains, in, search`.

> Booleans match JSON `true`/`false` directly (engine stores them as 1/0 under
> the hood — the MCP layer handles it).

## Tips

- Use `limit` (default 50, max 500) to page; `records_query` also supports `offset`.
- Prefer `records_query` over `records_list` when you have a known field to filter on.
- For one-off checks: `tables_list` → `records_query` is enough; you rarely need raw REST.
