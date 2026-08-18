# supabase_migrate — Supabase → serverless-engine converter

Converts a Supabase/Postgres migration folder into serverless-engine **tenant
boards + tables** and imports data over the MCP HTTP endpoint.

## New model (post-refactor)

The engine now models **board = tenant**. One tenant board owns many **tables**
(collections). `records.submit/import/query/list` all take `board` + `table`.

## Pipeline

1. **`schema`** — parse all `*.sql` migrations into a per-table model
   (accumulates `CREATE TABLE` + `ALTER TABLE ... ADD COLUMN`, enums, PKs,
   unique keys, checks, FKs).
2. **`boards`** — create ONE tenant board, then a `tables.create` per SQL table
   with a JSON Schema (Draft 2020-12) derived from the Postgres columns and
   `unique_key` from the PK/single-column unique constraint.
3. **`data`** — parse static `INSERT INTO ... VALUES` rows and submit them as
   records into the tenant board's table. Auto-generates UUID PKs that Postgres
   would have defaulted. Skips the `00000000000000_consolidated_schema.sql`
   file (it is a concatenation of the real migrations and would double-import).

## Usage

```
python3 -m supabase_migrate schema <migrations_dir>
python3 -m supabase_migrate boards <migrations_dir> [--tables a,b,c] [--tenant <title>] [--mcp URL]
python3 -m supabase_migrate data <migrations_dir> --table <t> --board <tenant> [--mcp URL]
```

Alternatively, for bulk loading existing dumps (JSON array/JSONL or CSV), use the
daemon's built-in **`records.import`** MCP tool directly:

```json
{"name":"records.import","arguments":{
  "board":"b_...","table":"classes","format":"csv",
  "data":"name,level,room,capacity\nPrimary 1 (P1),1,Room 101,40\n..."}}
```

- `format` = `json` (array or JSONL) or `csv` (header row; cells typed).
- `upsert` = boolean; dedupes on `unique_key` conflicts (requires `unique_key`
  to be configured on the table).
- Note: `records.import` validates against the table schema but does NOT
  auto-generate missing PKs — supply `id` or use the converter's `data` command.

## Mapping (engine model)

The conversion is **relational → document**:

- Postgres table  → engine `table` inside a tenant `board`
- Postgres column → JSON Schema property (string/number/boolean)
- Postgres enum   → `"enum": [...]`
- `NOT NULL`      → `"required"` + (nullable omitted from payload)
- PK / unique col → `unique_key` (e.g. `$.id`)
- `CHECK`         → `validate` rules (reject on negation of the check)
- Triggers/RLS/RPCs → NOT auto-converted; port deliberately as recipes/keys.

### What is NOT automatic (behavioral layer)

Postgres enforces these in the DB engine; the serverless engine models them as
separate primitives that must be ported intentionally:

| Postgres | serverless-engine equivalent |
|---|---|
| `TRIGGER` (94)      | `recipes` on `record.created/updated/deleted` (`$set`, `$compute`, `$upsert_other`, `$notify`) |
| RLS `POLICY` (721)  | table `keys`/`scopes`, `public_reads`, recipe `match` filters |
| RPC `FUNCTION` (35) | recipes with `$call`/`$transaction` + `jobs` (cron) |
| FK `REFERENCES`     | implicit via JSON `id` fields; or `links` |
| view / aggregate    | `records.query` filters + `computed` at read time |

## Live tenant mapping (data_v2)

> **Superseded:** the board `b_jxbd3jatmqkd0000` was a turso-era test tenant.
> The storage backend is now **HelixDB** (one board = one tenant) and the IICO
> data is re-migrated fresh via `tools/migrate_iico/` (live-SSH extract +
> graph-sync). This converter remains useful for **porting the schema from the
> SQL migration files** (enums, checks, validate rules, recipes) into a new
> board — run `boards` first, then load live data with `migrate_iico` and
> materialize edges with `srv graph sync`.

## Dependencies

Pure Python stdlib + `urllib`. No pip install needed.
