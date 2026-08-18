# Engine REST Contract — for building the drop-in client

The engine daemon runs at `http://127.0.0.1:7070`. All endpoints below are relative
to `/api/srv/<BOARD>`. Set `BOARD = "<board id>"` (the iico HelixDB tenant — create
it with `srv apps create 'iico'`; the old turso-era board `b_6sboua6pnqkd0000` is
obsolete).

## Auth (returns token + jwt)

- `POST /api/srv/<BOARD>/auth/signup` body `{email, password, role?}` → `{ok, user}`
- `POST /api/srv/<BOARD>/auth/login` body `{email, password}` → `{ok, token, jwt}`
- `POST /api/srv/<BOARD>/auth/logout` body `{token}` → `{ok}`
- `GET  /api/srv/<BOARD>/auth/me` with `Authorization: Bearer <token>` → `{ok, user}`
  - `user` = `{board_id, email, role, created_at}`

Session token is a UUID; send it as `Authorization: Bearer <token>` on all requests.

## Tables (collections inside the tenant board)

- `GET  /api/srv/<BOARD>/tables` → `{ok, tables:[{table, unique_key, schema_json, ...}]}`
- `POST /api/srv/<BOARD>/tables` body `{table, schema?, unique_key?}` → create table
- `GET  /api/srv/<BOARD>/tables/<t>` → `{ok, table}`
- `DELETE /api/srv/<BOARD>/tables/<t>` → drop table

## Records

- `POST /api/srv/<BOARD>/tables/<t>/submit?upsert=1|true` body = JSON payload → `{ok, seq}`
- `POST /api/srv/<BOARD>/tables/<t>/bulk` body `{records:[...]}` → `{ok, seqs}`
- `POST /api/srv/<BOARD>/tables/<t>/import` body `{format, data, separator?, upsert?}` → report
- `GET  /api/srv/<BOARD>/tables/<t>/records?limit&offset&before` → `{ok, records}`
- `GET  /api/srv/<BOARD>/tables/<t>/query?filter&order&dir&limit&offset&q&hl` → `{ok, records}`
- `GET  /api/srv/<BOARD>/tables/<t>/record?seq=N` → `{ok, record}`
- `PUT  /api/srv/<BOARD>/tables/<t>/records/<seq>` body = full payload → `{ok, seq}`
- `PATCH /api/srv/<BOARD>/tables/<t>/records/<seq>` body = partial object → `{ok, seq, payload}`
- `DELETE /api/srv/<BOARD>/tables/<t>/records/<seq>` → `{ok}`
- `GET  /api/srv/<BOARD>/tables/<t>/aggregate?op=count|sum|avg|min|max&field&group&filter` → `{ok, aggregate}`

Each record in responses: `{seq, payload, created_at, writer?}`. The app's data rows live
in `payload`. `seq` is the engine primary key per table.

## Query filter format (`filter` query param, URL-encoded JSON)

Either:
- `{ "field": { "eq": v } }` — map of field → operator map
- `{ "field": v }` — shorthand equality
- `[ {field,op,value}, ... ]` — array of condition objects
- `{ "search": "text", "field": {op: v}, ... }` — full-text + filters

Operators: `eq, neq, gt, gte, lt, lte, contains, not_contains, in, search`.
Field paths use `$.field` or `field`. Implicit AND for multiple top-level keys.

`order` param: comma-separated field names; `dir` = `asc`|`desc` (applies to all).
`limit` default 50 (max 500 on query, 200 on list). `offset` paging.

## Files / storage

- `POST /api/srv/<BOARD>/upload` multipart/raw body, `Content-Type` + `X-Filename` headers → `{ok, seq}`
- `GET  /api/srv/<BOARD>/file?file=<name>` → file bytes
- `PUT  /api/srv/<BOARD>/assets/<path>` body = bytes → store static asset
- `GET  /api/srv/<BOARD>/assets/<path>` → asset bytes (served as a site when public_reads)

## Realtime

- `GET /api/srv/<BOARD>/events?after=N` → SSE stream of events
- `POST /api/srv/<BOARD>/events` body → inbound webhook event
