# MCP Usage Guide

## MCP Configuration for Clients

**MCP Server URL:** `http://127.0.0.1:7070/mcp` (configured in `~/.mcp.json`)

```json
{
  "mcpServers": {
    "serverless-engine": {
      "transport": "http",
      "url": "http://127.0.0.1:7070/mcp"
    }
  }
}
```

**Client Connection:**
- Connects to `{daemon_url}/mcp` via POST JSON-RPC
- `daemon_url` from `SERVERLESS_DAEMON_URL` env var (default: `http://127.0.0.1:7070`)
- Sends: `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"<tool>","arguments":{...}}}`
- Reads response from `result.content[0].text`

**MCP Methods:**
- `tools/list` — returns all available tools
- `tools/call {name, arguments}` — calls a specific tool

---

## About

**serverlessEngine-rs** — a standalone, embeddable serverless platform.

- **Boards are apps**; each board is one HelixDB tenant.
- **Records** are JSON documents stored as graph nodes under the table label.
- **Recipes** are event-driven automation; triggers fire actions on record create/update/delete/inbound/cron.
- **Keys** authorize read/write/admin access per board.
- **Graph** links nodes into a traversable network via edge labels.
- **Jobs** are cron-scheduled actions (HTTP/poll).
- **Files** are static assets for front-end hosting (index.html, app.js, etc.).
- **Secrets** are AES-256-GCM encrypted; resolved via `{secret:name}` in recipe actions.

### First Steps

```
srv apps create 'my app'             create a board (returns b_<id>)
srv records submit b_<id> <table> '{"name":"x"}'   insert a record
srv records query b_<id> <table> --filter '...'      query records
srv records search b_<id> <table> 'laptop'           full-text search
srv graph link b_<id> <from> KNOWS <to>               link nodes into a graph
srv graph traverse b_<id> <from> --dir out            walk the graph
srv --help                           see every command
srv help <group> <verb>              full spec of one verb
```

---

## Help Levels

This guide has **4 help levels**. Each level shows more detail.

### Level 1 — Help Index

Shows all top-level groups and their verbs.

```
usage: srv <group> <verb> [args]

apps:
  srv apps create <board> <title> [--schema ...] [--unique_key ...]
  srv apps list <board>
  srv apps show <board>
  srv apps delete <board>
  srv apps resources <board>
  srv apps update <board> [--schema ...] [--unique_key ...] [--computed ...] [--validate ...] [--redact ...] [--ttl_seconds ...] [--audit ...] [--public_reads ...] [--rate ...]

endpoints:
  srv endpoints list [--dir asc|desc]

tables:
  srv tables create <board> <table> [--schema ...] [--unique_key ...]
  srv tables list <board>
  srv tables delete <board> <table>

records:
  srv records submit <board> <table> [--payload ...]
  srv records import <board> <table> --format json|csv --data ...
  srv records list <board> <table> [--limit ...] [--offset ...] [--dir asc|desc]
  srv records query <board> <table> --filter '{...}' [--order ...] [--dir asc|desc]
  srv records search <board> <table> <q>
  srv records get <board> <table> <seq>
  srv records delete <board> <table> <seq>
  srv records aggregate <board> <table> --op sum --field price [--group ...]

secrets:
  srv secrets set <board> <name> <value>
  srv secrets list <board>
  srv secrets show <board> <name>
  srv secrets delete <board> <name>

auth:
  srv auth signup <board> <email> [password <pass> | --password_hash '$2a$...'] [--role reader|writer|admin]
  srv auth login <board> <email> <password>
  srv auth me <token>
  srv auth set_role <board> <email> <role>

users:
  srv users list <board>

keys:
  srv keys list <board>
  srv keys show <bucket>

links:
  srv links list <board>
  srv links show <board>

recipes:
  srv recipes list <board>
  srv recipes show <board> <name>
  srv recipes add <board> <name> --when '{"event":"record.created"}' --actions '[{"$compute":{"$.greeted":true}}]'

files:
  srv files list <board> [--slug <name>] [--no_subapps]
  srv files put <board> <path> --content '<h1>hi</h1>' [--slug <name>] [--title <title>]
  srv files get <board> <path>
  srv files export <board> [--slug <name>] [--dir <path>]
  srv files import <board> [--slug <name>] [--dir <path>]
  srv files upload <board> --filename <name> --content_base64 <b64> [--table <t>] [--folder <dir>]
  srv files delete <board> <path> [--slug <name>]

subapps:
  srv subapps list <board>
  srv subapps remove <board> <slug> [--prune]

graph:
  srv graph link <board> <from> <label> <to> [--props '{"since":"2024"}']
  srv graph unlink <board> <edge>
  srv graph delete <board> <node>
  srv graph sync <board> <table>
  srv graph traverse <board> <from> [--dir out|in|both] [--label <label>] [--depth N]
  srv graph search_edges <board> <label> <property> <q> [--limit N]
  srv graph schema <board>

jobs:
  srv jobs add <board> <name> <schedule> [--action '{"type":"http","url":"..."}']
  srv jobs list <board>
  srv jobs show <board> <name>
  srv jobs runs <board> [--job <name>] [--limit N]
  srv jobs remove <board> <name>
```

**Usage:** `srv --help`

---

### Level 2 — Group Help

Shows all verbs in a single group.

**Usage:** `srv help <group>`

**Example: `srv help records`**

```
srv records submit <board> <table> [--payload ...]
srv records import <board> <table> --format json|csv --data ...
srv records list <board> <table> [--limit ...] [--offset ...] [--dir asc|desc]
srv records query <board> <table> --filter '{...}' [--order ...] [--dir asc|desc]
srv records search <board> <table> <q>
srv records get <board> <table> <seq>
srv records delete <board> <table> <seq>
srv records aggregate <board> <table> --op sum --field price [--group ...]
```

**Usage:** `srv help records`

**Example: `srv help records query`**

```
srv records query b_abc --filter '{"price":{"gte":500}}' --limit 10
```

---

### Level 3 — Verb Help

Shows full specification for one verb: positional args, flags, body JSON, response, examples.

**Usage:** `srv help <group> <verb>`

**Example: `srv help records.query`**

```
srv records.query
positional:
  board         Board         (required)    The board id.
  table         Name          (required)    The table name.
  filter        Json          default=None  Filter JSON (see help json filter).
  limit         Int           default=50    Max records (1..=500).
  allow_unfiltered
                              default=false Skip the unfiltered-scan guardrail (warns when a filter-less query would return >1000 rows).

flags:  none

json body: none

response:
  {"ok":true,"records":[{"seq":1,"payload":{},"created_at":"..."}]}

example: srv records query b_abc --filter '{"price":{"gte":500}}' --limit 10
  → {"ok":true,"records":[]}
```

**Usage:** `srv help records query`

---

### Level 4 — Reference

Full command reference: every verb's help, one after another.

**Usage:** `srv reference`

Or from this guide: the complete listing lives in the skill at
`.commandcode/skills/serverless-engine/references/reference.md`.

---
