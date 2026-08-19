pub fn verb_help(group: &str, verb: &str) -> Option<&'static str> {
    match (group, verb) {
        ("apps", "create") => Some(APPS_CREATE),
        ("apps", "list") => Some(APPS_LIST),
        ("apps", "show") => Some(APPS_SHOW),
        ("apps", "schema") => Some(APPS_SCHEMA),
        ("apps", "find") => Some("srv apps find <board_id>\n\nlooks up who owns a board and the session to reach it: returns owner_session, owner_key and public_reads, plus a hint to retry with --session <owner>"),
        ("apps", "update") => Some(APPS_UPDATE),
        ("apps", "delete") => Some(APPS_DELETE),
        ("apps", "export") => Some(APPS_EXPORT),
        ("apps", "import") => Some(APPS_IMPORT),
        ("apps", "keys") => Some(APPS_KEYS),
        ("apps", "hooks") => Some(APPS_HOOKS),
        ("apps", "rate") => Some(APPS_RATE),
        ("apps", "ttl") => Some(APPS_TTL),
        ("apps", "link") => Some(APPS_LINK),
        ("apps", "audit") => Some(APPS_AUDIT),
        ("apps", "jobs") => Some(APPS_JOBS),
        ("apps", "computed") => Some(APPS_COMPUTED),
        ("apps", "validate") => Some(APPS_VALIDATE),
        ("apps", "redact") => Some(APPS_REDACT),
        ("recipes", "list") => Some(RECIPES_LIST),
        ("recipes", "add") => Some(RECIPES_ADD),
        ("recipes", "enable") => Some(RECIPES_ENABLE),
        ("recipes", "remove") => Some(RECIPES_REMOVE),
        ("secrets", "set") => Some(SECRETS_SET),
        ("secrets", "list") => Some(SECRETS_LIST),
        ("secrets", "remove") => Some(SECRETS_REMOVE),
        ("records", "submit") => Some(RECORDS_SUBMIT),
        ("records", "bulk") => Some(RECORDS_BULK),
        ("records", "list") => Some(RECORDS_LIST),
        ("records", "get") => Some(RECORDS_GET),
        ("records", "update") => Some(RECORDS_UPDATE),
        ("records", "patch") => Some(RECORDS_PATCH),
        ("records", "query") => Some(RECORDS_QUERY),
        ("records", "search") => Some(RECORDS_SEARCH),
        ("records", "count") => Some(RECORDS_COUNT),
        ("records", "aggregate") => Some(RECORDS_AGGREGATE),
        ("records", "delete") => Some(RECORDS_DELETE),
        ("records", "audit") => Some(RECORDS_AUDIT),
        ("records", "call") => Some(RECORDS_CALL),
        ("files", "list") => Some(FILES_LIST),
        ("files", "get") => Some(FILES_GET),
        _ => None,
    }
}

const APPS_CREATE: &str = "\
srv apps create --title \"<title>\" [--json <schema>] [--public-reads on|off] [--unique '<$.path>']\n\
\n\
creates a new serverless app/board. Returns a secret board_id you use as the admin token.\n\
\n\
flags:\n\
  --title \"<title>\"       human-readable name (default 'Untitled')\n\
  --json <schema>         optional JSON Schema object; every record is validated against it\n\
  --public-reads on|off   on = anyone can read records without a key; default off (private)\n\
  --unique '<$.path>'     JSONPath of a field that must be unique; duplicate submits are rejected\n\
                          (or replaced if you pass --upsert on submit)\n\
example: srv apps create --title \"Orders\" --json '{\"type\":\"object\",\"properties\":{\"sku\":{\"type\":\"string\"}}}' --unique '$.sku'\n";

const APPS_LIST: &str = "\
srv apps list\n\
\n\
lists every board owned by this session (board_id, title, schema, public-reads, record count, created_at).\n";

const APPS_SHOW: &str = "\
srv apps show <board_id>\n\
\n\
shows the full configuration of one board (title, schema_json, public_reads, unique_key, owner, count, created_at).\n";

const APPS_SCHEMA: &str = "\
srv apps schema <board_id>\n\
\n\
prints just the JSON Schema of a board (or null if none was set at creation).\n";

const APPS_UPDATE: &str = "\
srv apps update <board_id> [--title \"t\"] [--json <schema> | --clear-schema] [--public-reads on|off] [--unique '<$.path>' | --clear-unique]\n\
\n\
updates a board's metadata. All flags optional; only supplied ones change.\n\
\n\
flags:\n\
  --title \"t\"            new title\n\
  --json <schema>         set/replace the validation schema\n\
  --clear-schema          remove the schema (no more validation)\n\
  --public-reads on|off   toggle public reads\n\
  --unique '<$.path>'     set a unique-key path\n\
  --clear-unique          remove the unique-key constraint\n";

const APPS_DELETE: &str = "\
srv apps delete <board_id>\n\
\n\
permanently deletes a board and all of its records, files, hooks, keys and audit trail. Irreversible.\n";

const APPS_EXPORT: &str = "\
srv apps export <board_id> [--out <path>]\n\
\n\
dumps the whole board as JSON: app config (title, schema, public_reads, unique_key), rate, ttl, audit flag,\n\
link, hooks (urls only; secrets are not recoverable) and every record payload.\n\
\n\
  --out <path>   write the dump to a file instead of returning it inline (large boards)\n\
restore with 'srv apps import' (creates a NEW board). Uploaded files are listed by metadata only.\n";

const APPS_IMPORT: &str = "\
srv apps import '<json>|<path>|-' [--out <path>]\n\
\n\
creates a new app/board from a dump produced by 'srv apps export'. The source can be inline JSON,\n\
a file path, or '-' to read the dump from stdin. Rate, ttl, audit, link and hooks are restored where possible;\n\
webhook secrets are re-registered without a secret. Returns the new board_id.\n";

const APPS_JOBS: &str = "\
srv apps jobs <board_id> add --name <n> --schedule '<cron|@every 30s>' --action '<json>' | list | remove <job_id>\n\
\n\
scheduled jobs: a global worker runs due jobs and reschedules them.\n\
\n\
sub-verbs:\n\
  add --name <n> --schedule <schedule> --action <json>\n\
      schedules a job on a board. schedule is a 5-field cron ('min hour dom month dow', * */n a-b a,b,c)\n\
      or an interval '@every <N><s|m|h|d>' (e.g. '@every 30s', '@every 5m', '@every 1h').\n\
      action json:\n\
        {\"type\":\"http\",\"url\":...,\"method\":\"GET\",\"headers\":[[\"k\",\"v\"]],\"body\":...,\"timeout_ms\":15000}\n\
            -> performs an outbound request (SSRF guard); the response is NOT stored.\n\
        {\"type\":\"poll\",\"url\":...,\"path\":\"$.field\",...}\n\
            -> performs the request, then extracts 'path' from the JSON response and inserts the\n\
               resulting object as a NEW record on this board (writer = job name).\n\
               This is how an app 'listens' to an external API on a schedule.\n\
  list                show the board's jobs (id, name, schedule, next_run_at, last_run status)\n\
  runs [<job_id>] [--limit N]   show the run history of all jobs, or one job by id (newest first)\n\
  remove <job_id>     delete a job by its numeric id\n\
examples:\n\
  srv apps jobs <board_id> add --name refresh --schedule '@every 5m' --action '{\"type\":\"http\",\"url\":\"https://api.example.com/ping\"}'\n\
  srv apps jobs <board_id> add --name sync --schedule '*/10 * * * *' --action '{\"type\":\"poll\",\"url\":\"https://api.example.com/feed\",\"path\":\"$.data\"}'\n";

const APPS_KEYS: &str = "\
srv apps keys <board_id> list | issue --role writer|reader|admin [--writer <id>] | revoke --bucket <bucket>\n\
\n\
manages API keys for a board. Keys are bearer tokens used against /api/srv/<board_id>/... instead of the board_id.\n\
\n\
sub-verbs:\n\
  list                                  show issued keys (bucket, role, writer, created, revoked)\n\
  issue --role writer|reader|admin      create a new key. reader = read-only (incl. private boards),\n\
                                        writer = can submit/patch, admin = manage config.\n\
                                        Optionally bind --writer <id> to attribute writes to that writer.\n\
                                        The secret key is returned ONCE; store it.\n\
  revoke --bucket <bucket>              invalidate a previously-issued key by its bucket id\n\
\n\
once a board has keys, unauthenticated writes are rejected (they need a valid key).\n";

const APPS_HOOKS: &str = "\
srv apps hooks <board_id> register <url> [--secret <s>] | list | remove <url> | deliveries [--url <u>] | replay <url>\n\
\n\
webhooks: record mutations (created/updated/deleted) are POSTed as JSON to registered URLs.\n\
\n\
sub-verbs:\n\
  register <url> [--secret <s>]   add a webhook. URL must be public http(s) (private targets blocked).\n\
                                  The request body is signed with X-WAOS-Signature = HMAC-SHA256(base64) of the body\n\
                                  using your secret; the secret is also injected as body.hooks_secret.\n\
  list                            show registered webhooks\n\
  remove <url>                    unregister a webhook\n\
  deliveries [--url <u>]          show delivery attempts (status, attempts, next retry)\n\
  replay <url>                    reschedule undelivered attempts for a webhook (backoff, max 5 tries)\n";

const APPS_RATE: &str = "\
srv apps rate <board_id> '<{\"submit\":60,...}>' | srv apps rate <board_id> --clear\n\
\n\
per-action rate limits for a board (sliding window, keyed by caller).\n\
\n\
actions: submit, upload, search, and a general read bucket. Provide a JSON object of action->requests per minute,\n\
plus an optional \"per_day\" cap. Limits not supplied fall back to defaults.\n\
example: srv apps rate <board_id> '{\"submit\":60,\"search\":30,\"per_day\":100000}'\n\
--clear resets the board to default limits.\n";

const APPS_TTL: &str = "\
srv apps ttl <board_id> [<seconds>] [--ttl-until '<$.path>'] | srv apps ttl <board_id> --clear\n\
\n\
auto-expiry for records. A record is considered expired (and hidden/deleted) if EITHER rule matches:\n\
  - created more than <seconds> ago\n\
  - the JSONPath field '<$.path>' holds a timestamp older than now\n\
\n\
with no <seconds> and no --ttl-until, prints the current TTL config.\n\
--clear removes both rules. Expired records are excluded from reads and swept periodically.\n\
example: srv apps ttl <board_id> 3600 --ttl-until '$.expires_at'\n";

const APPS_LINK: &str = "\
srv apps link <board_id> --on '$.k' --to <parent_board> --via '$.k' | srv apps link <board_id> --clear\n\
\n\
joins: embed the parent board's record payload into child records read via GET /api/srv/<board>/join.\n\
\n\
  --on '<path>'        JSONPath in the CHILD record used as the lookup key (e.g. '$.sku')\n\
  --to <parent_board>  the board whose records provide the joined payload\n\
  --via '<path>'       JSONPath in the PARENT record to match the child key against\n\
\n\
with no flags, prints the current link config; --clear removes it. Child records that match no parent\n\
resolve to null in the joined field. GET /api/srv/<board>/join returns child records with the parent payload embedded.\n";

const APPS_AUDIT: &str = "\
srv apps audit <board_id> on|off | srv apps audit <board_id> (list)\n\
\n\
toggles the immutable audit trail for a board. When on, every record created/updated/deleted is logged\n\
with who (writer), when, and a SHA-256 payload hash. History persists after turning off; only new events stop.\n\
  on|off                 enable/disable recording\n\
  (no argument)          list the audit trail (see 'srv records audit' for since/limit options)\n";

const RECORDS_SUBMIT: &str = "\
srv records submit <board_id> <json> [--writer <id>] [--upsert]\n\
\n\
inserts a new record. The payload is validated against the board's schema if one is set.\n\
Returns the new record's seq.\n\
\n\
flags:\n\
  --writer <id>   attribute this record to a writer id\n\
  --upsert        if the board has a --unique key and the value already exists, replace that record (keeps its seq)\n\
                  instead of rejecting the duplicate\n\
example: srv records submit <board_id> '{\"sku\":\"A1\",\"price\":10}' --writer alice\n";

const RECORDS_BULK: &str = "\
srv records bulk <board_id> '<json array>' [--writer <id>] [--upsert]\n\
\n\
inserts many records in one call. Each object is validated against the board's schema;\n\
a failure aborts the whole batch. Returns the list of new seqs.\n\
example: srv records bulk <board_id> '[{\"sku\":\"A1\"},{\"sku\":\"A2\"},{\"sku\":\"A3\"}]'\n";

const RECORDS_LIST: &str = "\
srv records list <board_id> [--limit N] [--offset N]\n\
\n\
lists records newest-first. limit defaults to 50 (max 200), offset for paging.\n\
example: srv records list <board_id> --limit 100 --offset 100\n";

const RECORDS_GET: &str = "\
srv records get <board_id> --seq N\n\
\n\
fetches a single record by its sequence number. Returns null if it doesn't exist.\n";

const RECORDS_UPDATE: &str = "\
srv records update <board_id> <seq> '<json>' [--writer <id>]\n\
\n\
replaces a record's payload entirely (identity and created_at preserved). Validated against the schema.\n\
example: srv records update <board_id> 1 '{\"sku\":\"A1\",\"price\":11}'\n";

const RECORDS_PATCH: &str = "\
srv records patch <board_id> <seq> '<ops>'\n\
\n\
applies a partial JSON patch to a record (MongoDB-style $ ops).\n\
ops json:\n\
  {\"$set\":{\"color\":\"black\"}, \"$inc\":{\"votes\":1}, \"$dec\":{\"x\":1}, \"$mul\":{\"x\":2}, \"$unset\":[\"field\",\"a.b[0]\"]}\n\
  or a plain object = shallow top-level merge ($set of leaf keys).\n\
returns the resulting validated payload.\n";

const RECORDS_QUERY: &str = "\
srv records query <board_id> '<$.path>' [--eq value] | srv records query <board_id> --filter '<json>' [--order '<$.path>' asc|desc]* [--limit N] [--offset N]\n\
\n\
reads records by JSONPath or by a filter object.\n\
  path form:   query <board_id> '$.name'            -> all records where $.name exists\n\
               query <board_id> '$.name' --eq alice -> equal to 'alice'\n\
  filter form: query <board_id> --filter '{\"$.price\":{\"gte\":500}}'\n\
  --order '<$.path>' asc|desc   repeatable: multi-column sorting, applied left to right\n\
filter ops: eq neq gt gte lt lte contains not_contains in search\n\
filter forms: {\"field\":\"$.price\",\"op\":\"gte\",\"value\":500} | {\"$.price\":{\"gte\":500}} | {\"search\":\"q\",\"price\":{\"gte\":100}}\n";

const RECORDS_SEARCH: &str = "\
srv records search <board_id> '<query>' [--limit N] [--offset N] [--hl]\n\
\n\
full-text search (BM25-ranked) across the whole record payload; case-insensitive, substring-tolerant.\n\
  --hl   include highlight snippets in the response\n\
example: srv records search <board_id> 'laptop'\n";

const RECORDS_COUNT: &str = "\
srv records count <board_id>\n\
\n\
returns the number of records on the board.\n";

const RECORDS_AGGREGATE: &str = "\
srv records aggregate <board_id> --op sum|avg|min|max|count --field '<$.path>' [--group_by '<$.path>'] [--filter '<json>']\n\
\n\
computes an aggregate over the board's records.\n\
  --op        sum|avg|min|max|count\n\
  --field     JSONPath to the numeric field (ignored for count)\n\
  --group_by  optional JSONPath to group rows by\n\
  --filter    optional filter json (same ops as query)\n\
example: srv records aggregate <board_id> --op sum --field '$.price' --group_by '$.category'\n";

const RECORDS_DELETE: &str = "\
srv records delete <board_id> [--seq N | --all | --filter '<json>']\n\
\n\
deletes one record (--seq N), every record (--all), or every record matching a filter (--filter '<json>').\n\
--filter uses the same ops as query: delete <board_id> --filter '{\"$.status\":{\"eq\":\"archived\"}}'\n\
returns the number of rows removed for --filter.\n";

const RECORDS_AUDIT: &str = "\
srv records audit <board_id> [--since <iso-ts>] [--limit N]\n\
\n\
reads the board's audit trail (see 'srv apps audit <board_id> on' to enable recording).\n\
  --since <iso-ts>   only events at/after this timestamp (format 'YYYY-MM-DD HH:MM:SS')\n\
  --limit N          max rows (default 100, max 1000)\n\
each row: id, event (created/updated/deleted), actor, writer, payload_hash, ts.\n";

const RECORDS_CALL: &str = "\
srv records call <board_id> <url> [--method GET|POST|PUT|PATCH|DELETE] [--header 'Name: Value']* [--body '<json>'] [--timeout N]\n\
\n\
makes an outbound HTTP request to an external API and returns the full response so the app can read it\n\
(connect to API providers, sensors, etc.). Private/loopback targets are blocked (SSRF guard).\n\
\n\
flags:\n\
  --method           GET (default) | POST | PUT | PATCH | DELETE\n\
  --header 'K: V'    add a request header; repeat for multiple headers\n\
  --body '<json>'    request body (for POST/PUT/PATCH/DELETE)\n\
  --timeout N        timeout in ms (clamped 500..60000; default 10000)\n\
returns: { status, headers, body, duration_ms }.\n\
example: srv records call <board_id> https://api.example.com/things --method POST --header 'X-Api-Key: sk-123' --body '{\"sensor\":\"temp\"}'\n";

const FILES_LIST: &str = "\
srv files list <board_id>\n\
\n\
lists uploaded files for a board (file id + size in bytes).\n";

const FILES_GET: &str = "\
srv files get <board_id> <file_id> --out <path>\n\
\n\
downloads an uploaded file to a local path (creates parent dirs).\n\
example: srv files get <board_id> files/f_abc123.bin --out /tmp/photo.png\n";

const APPS_COMPUTED: &str = "\
srv apps computed <board_id> '<json>' | --clear | (no args shows current)\n\
\n\
sets the computed-fields config: a map of output field -> expression evaluated on every record write.\n\
\n\
  with a json argument: replace the config\n\
  --clear: remove the config\n\
  no args: print the current config\n\
example: srv apps computed <board_id> '{\"total\":\"$.price * $.qty\"}'\n\
expressions use the same language as filters/paths ($.price, string ops, math).\n";

const APPS_VALIDATE: &str = "\
srv apps validate <board_id> '<json>' | --clear | (no args shows current)\n\
\n\
sets validation rules evaluated on every record write: an array of {\"when\":\"expr\",\"error\":\"msg\"}.\n\
The first rule whose expression is truthy rejects the write with its error message.\n\
\n\
  with a json argument: replace the rules\n\
  --clear: remove the rules\n\
  no args: print the current rules\n\
example: srv apps validate <board_id> '[{\"when\":\"$.total > 1000\",\"error\":\"order too large\"}]'\n";

const APPS_REDACT: &str = "\
srv apps redact <board_id> '<json>' | --clear | (no args shows current)\n\
\n\
sets redact paths: an array of JSONPaths whose values are masked as \"***\" in record payloads.\n\
Applied when records are returned from reads.\n\
\n\
  with a json argument: replace the paths\n\
  --clear: remove the paths\n\
  no args: print the current paths\n\
example: srv apps redact <board_id> '[\"$.ssn\",\"$.card.number\"]'\n";

const RECIPES_LIST: &str = "\
srv recipes list <board_id>\n\
\n\
lists the board's recipes (id, name, when, match, enabled, dedup, actions).\n\
\n\
other sub-verbs:\n\
  runs [<name>] [--limit N]   show the run history of all recipes, or one by name (newest first)\n";

const RECIPES_ADD: &str = "\
srv recipes add <board_id> --name <n> --when '<event>' [--match '<json>'] [--dedup '<$.path>'] [--actions '<json>']\n\
\n\
creates an event-driven recipe on the board.\n\
\n\
  --name <n>           unique recipe name\n\
  --when <event>       record.created | record.updated | record.deleted | inbound | cron:<job_name>\n\
  --match '<json>'     optional filter the event payload must pass (same ops as query)\n\
  --dedup '<$.path>'   optional JSONPath whose value dedupes runs (already-processed keys are skipped)\n\
  --actions '<json>'   optional array of action objects, e.g. [{\"$set\":{\"status\":\"paid\"}}]\n\
example: srv recipes add <board_id> --name on_payment --when record.created --match '{\"$.status\":{\"eq\":\"paid\"}}' --dedup '$.id' --actions '[{\"$set\":{\"approved\":true}}]'\n";

const RECIPES_ENABLE: &str = "\
srv recipes enable <board_id> <name> on|off\n\
\n\
enables or disables a recipe by name.\n\
example: srv recipes enable <board_id> on_payment off\n";

const RECIPES_REMOVE: &str = "\
srv recipes remove <board_id> <name>\n\
\n\
permanently deletes a recipe by name.\n";

const SECRETS_SET: &str = "\
srv secrets set <board_id> <NAME> <value>\n\
\n\
stores an encrypted secret (API key/token) for the board. The name is stored uppercased;\n\
the value is AES-GCM encrypted at rest and never returned by list. Stored secrets are only\n\
decrypted in-memory when a recipe engine runs.\n\
example: srv secrets set <board_id> STRIPE sk_test_xyz\n";

const SECRETS_LIST: &str = "\
srv secrets list <board_id>\n\
\n\
lists secret names, fingerprints and creation time. Values are never shown.\n";

const SECRETS_REMOVE: &str = "\
srv secrets remove <board_id> <NAME>\n\
\n\
deletes a stored secret by its (uppercased) name.\n";
