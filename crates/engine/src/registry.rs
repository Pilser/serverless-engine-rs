use serde_json::Value as Json;

#[derive(Debug, Clone)]
pub enum ArgType {
    Int,
    Str,
    Json,
    Path,
    Url,
    Board,
    Seq,
    Name,
}

#[derive(Debug, Clone)]
pub struct ArgSpec {
    pub name: String,
    pub r#type: ArgType,
    pub required: bool,
    pub help: String,
}

#[derive(Debug, Clone)]
pub enum FlagType {
    Bool,
    Str,
    Int,
    Json,
    Path,
    Enum,
}

#[derive(Debug, Clone)]
pub struct FlagSpec {
    pub name: String,
    pub r#type: FlagType,
    pub default: Option<String>,
    pub allowed: Vec<String>,
    pub repeatable: bool,
    pub help: String,
    pub conflicts: Vec<String>,
    pub requires: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Role {
    Public,
    Reader,
    Writer,
    Admin,
    Owner,
}

#[derive(Debug, Clone)]
pub struct Example {
    pub args: String,
    pub description: String,
    pub response: Json,
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub group: String,
    pub verb: String,
    pub summary: String,
    pub description: String,
    pub positional: Vec<ArgSpec>,
    pub flags: Vec<FlagSpec>,
    pub body_json: Option<Json>,
    pub response: Json,
    pub auth: Role,
    pub destructive: bool,
    pub dry_run: bool,
    pub examples: Vec<Example>,
    pub see_also: Vec<String>,
    pub danger_notes: Option<String>,
}

impl CommandSpec {
    pub fn name(&self) -> String {
        format!("{}.{}", self.group, self.verb)
    }
}

pub fn registry() -> Vec<CommandSpec> {
    let mut specs = Vec::new();

    specs.push(CommandSpec {
        group: "apps".into(),
        verb: "create".into(),
        summary: "Create a new serverless app (board).".into(),
        description: "Creates a board owned by the caller and returns its id.".into(),
        positional: vec![ArgSpec {
            name: "title".into(),
            r#type: ArgType::Str,
            required: true,
            help: "Human label for the app.".into(),
        }],
        flags: vec![
            FlagSpec {
                name: "schema".into(),
                r#type: FlagType::Json,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Optional JSON Schema (Draft 2020-12).".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "unique_key".into(),
                r#type: FlagType::Path,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "JSON path used for upsert conflict detection.".into(),
                conflicts: vec![],
                requires: vec![],
            },
        ],
        body_json: None,
        response: serde_json::json!({ "ok": true, "board": "b_<id>", "title": "<title>" }),
        auth: Role::Owner,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "apps create 'my app'".into(),
            description: "Create a board.".into(),
            response: serde_json::json!({ "ok": true, "board": "b_abcd1234", "title": "my app" }),
        }],
        see_also: vec!["apps.list".into(), "apps.show".into(), "apps.delete".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "apps".into(),
        verb: "list".into(),
        summary: "List apps owned by the caller.".into(),
        description: "Returns the caller's boards (id + title).".into(),
        positional: vec![],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "apps": [{ "board": "b_<id>", "title": "<title>" }] }),
        auth: Role::Owner,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "apps list".into(),
            description: "List my boards.".into(),
            response: serde_json::json!({ "ok": true, "apps": [{ "board": "b_abcd1234", "title": "my app" }] }),
        }],
        see_also: vec!["apps.create".into(), "apps.show".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "apps".into(),
        verb: "show".into(),
        summary: "Show an app's full config.".into(),
        description: "Returns the board record including schema, keys config, TTL and more.".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        }],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "board": { "board_id": "b_<id>", "title": "<title>" } }),
        auth: Role::Owner,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "apps show b_abcd1234".into(),
            description: "Show a board.".into(),
            response: serde_json::json!({ "ok": true, "board": { "board_id": "b_abcd1234", "title": "my app" } }),
        }],
        see_also: vec!["apps.list".into(), "apps.delete".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "apps".into(),
        verb: "delete".into(),
        summary: "Delete an app and all its records.".into(),
        description: "Permanently removes the board and every record, recipe, key and file on it.".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        }],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "board": "b_<id>", "deleted": true }),
        auth: Role::Owner,
        destructive: true,
        dry_run: true,
        examples: vec![Example {
            args: "apps delete b_abcd1234".into(),
            description: "Delete a board and its data.".into(),
            response: serde_json::json!({ "ok": true, "board": "b_abcd1234", "deleted": true }),
        }],
        see_also: vec!["apps.show".into()],
        danger_notes: Some("irreversible: deletes all records, recipes, keys and files on the board".into()),
    });

    specs.push(CommandSpec {
        group: "apps".into(),
        verb: "resources".into(),
        summary: "Read an app's resource usage in real time.".into(),
        description: "Returns current records, storage bytes, rate-limit usage, in-flight requests, CPU% and RAM for a board. The same report is streamed over WS with ?resources=1.".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        }],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "board": "b_<id>", "records": { "count": 0 }, "storage": { "bytes": 0 }, "rate": { "limits": {} }, "system": { "cpu_percent": 0.0 } }),
        auth: Role::Owner,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "apps resources b_abcd1234".into(),
            description: "Read live resource usage for a board.".into(),
            response: serde_json::json!({ "ok": true, "board": "b_abcd1234", "records": { "count": 42 } }),
        }],
        see_also: vec!["apps.show".into(), "apps.update".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "apps".into(),
        verb: "update".into(),
        summary: "Update an app's config.".into(),
        description: "Sets schema, unique key, computed, validate, redact, TTL, rate or audit on a board.".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        }],
        flags: vec![
            FlagSpec {
                name: "schema".into(),
                r#type: FlagType::Json,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "JSON Schema (Draft 2020-12).".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "unique_key".into(),
                r#type: FlagType::Path,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "JSON path used for upsert conflict detection.".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "computed".into(),
                r#type: FlagType::Json,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Object of {field: expression} evaluated on every write.".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "validate".into(),
                r#type: FlagType::Json,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Array of {when, error} validation rules.".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "redact".into(),
                r#type: FlagType::Json,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Array of JSON paths to mask on read (→ ***).".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "ttl_seconds".into(),
                r#type: FlagType::Int,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Auto-expire records after N seconds.".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "audit".into(),
                r#type: FlagType::Bool,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Enable the immutable audit log.".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "public_reads".into(),
                r#type: FlagType::Bool,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Allow unauthenticated reads (static front-end hosting).".into(),
                conflicts: vec![],
                requires: vec![],
            },
        ],
        body_json: None,
        response: serde_json::json!({ "ok": true, "board": "b_<id>", "updated": true }),
        auth: Role::Owner,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "apps update b_abc --computed '{\"total\":\"$.qty * $.price\"}'".into(),
            description: "Set computed fields.".into(),
            response: serde_json::json!({ "ok": true, "board": "b_abc", "updated": true }),
        }],
        see_also: vec!["apps.show".into(), "apps.delete".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "records".into(),
        verb: "submit".into(),
        summary: "Insert a record into a board.".into(),
        description: "Inserts a JSON document as a new record; returns its seq.".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        },
            ArgSpec {
                name: "table".into(),
                r#type: ArgType::Name,
                required: true,
                help: "The table name.".into(),
            },
        ],
        flags: vec![],
        body_json: Some(serde_json::json!({
            "type": "object",
            "description": "The record payload to insert."
        })),
        response: serde_json::json!({ "ok": true, "seq": 42 }),
        auth: Role::Writer,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "records submit b_abc '{\"name\":\"x\"}'".into(),
            description: "Insert a record.".into(),
            response: serde_json::json!({ "ok": true, "seq": 42 }),
        }],
        see_also: vec!["records.get".into(), "records.query".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "records".into(),
        verb: "query".into(),
        summary: "Filter records by conditions.".into(),
        description: "Returns records matching a filter, sorted and paged.".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        },
            ArgSpec {
                name: "table".into(),
                r#type: ArgType::Name,
                required: true,
                help: "The table name.".into(),
            },
        ],
        flags: vec![
            FlagSpec {
                name: "filter".into(),
                r#type: FlagType::Json,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Filter JSON (see help json filter).".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "limit".into(),
                r#type: FlagType::Int,
                default: Some("50".into()),
                allowed: vec![],
                repeatable: false,
                help: "Max records (1..=500).".into(),
                conflicts: vec![],
                requires: vec![],
            },
        ],
        body_json: None,
        response: serde_json::json!({ "ok": true, "records": [{ "seq": 1, "payload": {} }] }),
        auth: Role::Reader,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "records query b_abc --filter '{\"price\":{\"gte\":500}}' --limit 10".into(),
            description: "Query records.".into(),
            response: serde_json::json!({ "ok": true, "records": [] }),
        }],
        see_also: vec!["records.search".into(), "records.get".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "records".into(),
        verb: "get".into(),
        summary: "Fetch a single record by seq.".into(),
        description: "Returns one record or an error if missing.".into(),
        positional: vec![
            ArgSpec {
                name: "board".into(),
                r#type: ArgType::Board,
                required: true,
                help: "The board id.".into(),
            },
            ArgSpec {
                name: "table".into(),
                r#type: ArgType::Name,
                required: true,
                help: "The table name.".into(),
            },
            ArgSpec {
                name: "seq".into(),
                r#type: ArgType::Seq,
                required: true,
                help: "The record sequence number.".into(),
            },
        ],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "record": { "seq": 1, "payload": {} } }),
        auth: Role::Reader,
        destructive: false,
        dry_run: false,
        examples: vec![],
        see_also: vec!["records.list".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "records".into(),
        verb: "list".into(),
        summary: "List records newest-first.".into(),
        description: "Returns records in reverse-chronological order, paged.".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        },
            ArgSpec {
                name: "table".into(),
                r#type: ArgType::Name,
                required: true,
                help: "The table name.".into(),
            },
        ],
        flags: vec![
            FlagSpec {
                name: "limit".into(),
                r#type: FlagType::Int,
                default: Some("50".into()),
                allowed: vec![],
                repeatable: false,
                help: "Max records (1..=200).".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "offset".into(),
                r#type: FlagType::Int,
                default: Some("0".into()),
                allowed: vec![],
                repeatable: false,
                help: "Skip this many records.".into(),
                conflicts: vec![],
                requires: vec![],
            },
        ],
        body_json: None,
        response: serde_json::json!({ "ok": true, "records": [{ "seq": 1, "payload": {} }] }),
        auth: Role::Reader,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "records list b_abc --limit 10".into(),
            description: "List records.".into(),
            response: serde_json::json!({ "ok": true, "records": [] }),
        }],
        see_also: vec!["records.query".into(), "records.get".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "records".into(),
        verb: "search".into(),
        summary: "Full-text search over records.".into(),
        description: "Searches record payloads for a query string, ranked by hit count.".into(),
        positional: vec![
            ArgSpec {
                name: "board".into(),
                r#type: ArgType::Board,
                required: true,
                help: "The board id.".into(),
            },
            ArgSpec {
                name: "table".into(),
                r#type: ArgType::Name,
                required: true,
                help: "The table name.".into(),
            },
            ArgSpec {
                name: "q".into(),
                r#type: ArgType::Str,
                required: true,
                help: "Search query (space-separated tokens).".into(),
            },
        ],
        flags: vec![FlagSpec {
            name: "limit".into(),
            r#type: FlagType::Int,
            default: Some("50".into()),
            allowed: vec![],
            repeatable: false,
            help: "Max records (1..=500).".into(),
            conflicts: vec![],
            requires: vec![],
        }],
        body_json: None,
        response: serde_json::json!({ "ok": true, "records": [{ "seq": 1, "payload": {}, "score": 1 }] }),
        auth: Role::Reader,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "records search b_abc 'alice'".into(),
            description: "Search records.".into(),
            response: serde_json::json!({ "ok": true, "records": [] }),
        }],
        see_also: vec!["records.query".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "records".into(),
        verb: "delete".into(),
        summary: "Delete a single record by seq.".into(),
        description: "Removes one record; returns whether it existed.".into(),
        positional: vec![
            ArgSpec {
                name: "board".into(),
                r#type: ArgType::Board,
                required: true,
                help: "The board id.".into(),
            },
            ArgSpec {
                name: "table".into(),
                r#type: ArgType::Name,
                required: true,
                help: "The table name.".into(),
            },
            ArgSpec {
                name: "seq".into(),
                r#type: ArgType::Seq,
                required: true,
                help: "The record sequence number.".into(),
            },
        ],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "seq": 1, "deleted": true }),
        auth: Role::Writer,
        destructive: true,
        dry_run: true,
        examples: vec![Example {
            args: "records delete b_abc 1".into(),
            description: "Delete a record.".into(),
            response: serde_json::json!({ "ok": true, "seq": 1, "deleted": true }),
        }],
        see_also: vec!["records.get".into()],
        danger_notes: Some("permanently removes the record".into()),
    });

    specs.push(CommandSpec {
        group: "records".into(),
        verb: "import".into(),
        summary: "Bulk-import records from a JSON or CSV dump.".into(),
        description: "Parses a JSON array/JSONL or CSV (header row) dump and inserts each row as a record, validating against the board schema. Invalid rows are skipped and reported; valid rows are inserted.".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        },
            ArgSpec {
                name: "table".into(),
                r#type: ArgType::Name,
                required: true,
                help: "The table name.".into(),
            },
        ],
        flags: vec![
            FlagSpec {
                name: "format".into(),
                r#type: FlagType::Str,
                default: Some("json".into()),
                allowed: vec![],
                repeatable: false,
                help: "Input format: json (array or JSONL) or csv.".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "data".into(),
                r#type: FlagType::Str,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "The dump content (JSON array/JSONL, or CSV with a header row).".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "separator".into(),
                r#type: FlagType::Str,
                default: Some(",".into()),
                allowed: vec![],
                repeatable: false,
                help: "CSV field separator (default ',').".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "upsert".into(),
                r#type: FlagType::Bool,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Upsert on unique_key conflicts.".into(),
                conflicts: vec![],
                requires: vec![],
            },
        ],
        body_json: None,
        response: serde_json::json!({ "ok": true, "total": 3, "inserted": 3, "seqs": [1, 2, 3], "errors": [] }),
        auth: Role::Writer,
        destructive: false,
        dry_run: false,
        examples: vec![
            Example {
                args: "records import b_abc --format json --data '[{\"name\":\"ada\"},{\"name\":\"zed\"}]'".into(),
                description: "Import a JSON array.".into(),
                response: serde_json::json!({ "ok": true, "total": 2, "inserted": 2, "seqs": [1, 2], "errors": [] }),
            },
            Example {
                args: "records import b_abc --format csv --data 'name,score\\nada,85\\nzed,90'".into(),
                description: "Import CSV with a header row.".into(),
                response: serde_json::json!({ "ok": true, "total": 2, "inserted": 2, "seqs": [1, 2], "errors": [] }),
            },
        ],
        see_also: vec!["records.submit".into(), "records.bulk".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "tables".into(),
        verb: "create".into(),
        summary: "Create a table (collection) inside a board.".into(),
        description: "Creates a named table with an optional JSON Schema and unique key. Records live in tables; a board is a tenant that owns many tables.".into(),
        positional: vec![
            ArgSpec { name: "board".into(), r#type: ArgType::Board, required: true, help: "The board id.".into() },
            ArgSpec { name: "table".into(), r#type: ArgType::Name, required: true, help: "The table name.".into() },
        ],
        flags: vec![
            FlagSpec { name: "schema".into(), r#type: FlagType::Json, default: None, allowed: vec![], repeatable: false, help: "JSON Schema (Draft 2020-12).".into(), conflicts: vec![], requires: vec![] },
            FlagSpec { name: "unique_key".into(), r#type: FlagType::Path, default: None, allowed: vec![], repeatable: false, help: "JSON path used for upsert conflict detection.".into(), conflicts: vec![], requires: vec![] },
        ],
        body_json: None,
        response: serde_json::json!({ "ok": true, "table": { "table": "learners" } }),
        auth: Role::Admin,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "tables create b_abc learners --schema '{\"type\":\"object\"}'".into(),
            description: "Create a table with a schema.".into(),
            response: serde_json::json!({ "ok": true, "table": { "table": "learners" } }),
        }],
        see_also: vec!["tables.list".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "tables".into(),
        verb: "list".into(),
        summary: "List the tables of a board.".into(),
        description: "Returns every table (collection) defined on the board.".into(),
        positional: vec![ArgSpec { name: "board".into(), r#type: ArgType::Board, required: true, help: "The board id.".into() }],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "tables": [] }),
        auth: Role::Reader,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "tables list b_abc".into(),
            description: "List tables.".into(),
            response: serde_json::json!({ "ok": true, "tables": [] }),
        }],
        see_also: vec!["tables.create".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "tables".into(),
        verb: "delete".into(),
        summary: "Drop a table and all its records.".into(),
        description: "Permanently removes the table config and every record in it.".into(),
        positional: vec![
            ArgSpec { name: "board".into(), r#type: ArgType::Board, required: true, help: "The board id.".into() },
            ArgSpec { name: "table".into(), r#type: ArgType::Name, required: true, help: "The table name.".into() },
        ],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "deleted": true }),
        auth: Role::Admin,
        destructive: true,
        dry_run: true,
        examples: vec![Example {
            args: "tables delete b_abc learners".into(),
            description: "Drop a table.".into(),
            response: serde_json::json!({ "ok": true, "deleted": true }),
        }],
        see_also: vec!["tables.list".into()],
        danger_notes: Some("irreversibly removes all records in the table".into()),
    });

    specs.push(CommandSpec {
        group: "secrets".into(),
        verb: "set".into(),
        summary: "Store an encrypted secret for a board.".into(),
        description: "Saves a named secret (AES-256-GCM encrypted at rest). Never returned as plaintext; use {secret:name} in recipe $call actions.".into(),
        positional: vec![
            ArgSpec { name: "board".into(), r#type: ArgType::Board, required: true, help: "The board id.".into() },
            ArgSpec { name: "name".into(), r#type: ArgType::Name, required: true, help: "Secret name (stored uppercased).".into() },
            ArgSpec { name: "value".into(), r#type: ArgType::Str, required: true, help: "The secret value.".into() },
        ],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "set": "NAME" }),
        auth: Role::Admin,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "secrets set b_abc AFRICASTALKING_API_KEY sk_live_xxx".into(),
            description: "Store a secret.".into(),
            response: serde_json::json!({ "ok": true, "set": "AFRICASTALKING_API_KEY" }),
        }],
        see_also: vec!["secrets.list".into()],
        danger_notes: Some("the value is never echoed back; store carefully".into()),
    });

    specs.push(CommandSpec {
        group: "secrets".into(),
        verb: "list".into(),
        summary: "List a board's secret names and fingerprints.".into(),
        description: "Returns secret names and fingerprints only — never plaintext values.".into(),
        positional: vec![ArgSpec { name: "board".into(), r#type: ArgType::Board, required: true, help: "The board id.".into() }],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "secrets": [{ "name": "API_KEY", "fingerprint": "abc123" }] }),
        auth: Role::Admin,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "secrets list b_abc".into(),
            description: "List secret names.".into(),
            response: serde_json::json!({ "ok": true, "secrets": [] }),
        }],
        see_also: vec!["secrets.set".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "secrets".into(),
        verb: "delete".into(),
        summary: "Delete a board secret.".into(),
        description: "Removes a named secret so {secret:name} placeholders no longer resolve.".into(),
        positional: vec![
            ArgSpec { name: "board".into(), r#type: ArgType::Board, required: true, help: "The board id.".into() },
            ArgSpec { name: "name".into(), r#type: ArgType::Name, required: true, help: "Secret name.".into() },
        ],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "deleted": true }),
        auth: Role::Admin,
        destructive: true,
        dry_run: true,
        examples: vec![Example {
            args: "secrets delete b_abc API_KEY".into(),
            description: "Delete a secret.".into(),
            response: serde_json::json!({ "ok": true, "deleted": true }),
        }],
        see_also: vec!["secrets.list".into()],
        danger_notes: Some("the secret can no longer be resolved".into()),
    });

    specs.push(CommandSpec {
        group: "auth".into(),
        verb: "signup".into(),
        summary: "Create a user (plaintext password or imported bcrypt hash).".into(),
        description: "Two modes, chosen per-call: pass `password` (engine bcrypt-hashes it, >= 6 chars) or `password_hash` (an existing bcrypt hash, e.g. from Supabase, stored verbatim — requires an admin/owner caller). Exactly one of the two must be provided.".into(),
        positional: vec![
            ArgSpec { name: "board".into(), r#type: ArgType::Board, required: true, help: "The board id.".into() },
            ArgSpec { name: "email".into(), r#type: ArgType::Str, required: true, help: "The user's email (lowercased).".into() },
            ArgSpec { name: "password".into(), r#type: ArgType::Str, required: false, help: "Plaintext password (mode B: engine hashes it).".into() },
        ],
        flags: vec![
            FlagSpec { name: "password_hash".into(), r#type: FlagType::Str, default: None, allowed: vec![], repeatable: false, help: "Existing bcrypt hash to import verbatim (mode A; admin/owner only).".into(), conflicts: vec![], requires: vec![] },
            FlagSpec { name: "role".into(), r#type: FlagType::Str, default: None, allowed: vec![], repeatable: false, help: "Requested role (clamped to reader for non-admin callers; default reader).".into(), conflicts: vec![], requires: vec![] },
        ],
        body_json: None,
        response: serde_json::json!({ "ok": true, "user": { "email": "a@x.ug", "board_id": "b_abc", "role": "teacher", "created_at": "..." } }),
        auth: Role::Public,
        destructive: false,
        dry_run: false,
        examples: vec![
            Example {
                args: "auth signup b_abc a@x.ug plaintext123 --role teacher".into(),
                description: "Create a user with a plaintext password.".into(),
                response: serde_json::json!({ "ok": true, "user": { "email": "a@x.ug" } }),
            },
            Example {
                args: "auth signup b_abc ali@sised.sc.ug --password_hash '$2a$10$VjZAU2...' --role teacher".into(),
                description: "Import a user with an existing Supabase hash.".into(),
                response: serde_json::json!({ "ok": true, "user": { "email": "ali@sised.sc.ug" } }),
            },
        ],
        see_also: vec!["auth.login".into(), "auth.me".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "auth".into(),
        verb: "login".into(),
        summary: "Authenticate and get a session token + JWT.".into(),
        description: "Verifies the password and returns a revocable opaque token plus a signed JWT (HS256).".into(),
        positional: vec![
            ArgSpec { name: "board".into(), r#type: ArgType::Board, required: true, help: "The board id.".into() },
            ArgSpec { name: "email".into(), r#type: ArgType::Str, required: true, help: "The user's email.".into() },
            ArgSpec { name: "password".into(), r#type: ArgType::Str, required: true, help: "The password.".into() },
        ],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "token": "...", "jwt": "..." }),
        auth: Role::Public,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "auth login b_abc a@x.ug secret123".into(),
            description: "Log in and receive a session token + JWT.".into(),
            response: serde_json::json!({ "ok": true, "token": "...", "jwt": "..." }),
        }],
        see_also: vec!["auth.signup".into(), "auth.me".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "auth".into(),
        verb: "me".into(),
        summary: "Resolve a session token to the logged-in user.".into(),
        description: "Returns the user (email, role, board) for a valid session token, or an error if invalid or expired.".into(),
        positional: vec![
            ArgSpec { name: "board".into(), r#type: ArgType::Board, required: true, help: "The board id.".into() },
            ArgSpec { name: "token".into(), r#type: ArgType::Str, required: true, help: "Session token from auth login.".into() },
        ],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "user": { "email": "a@x.ug", "board_id": "b_abc", "role": "teacher", "created_at": "..." } }),
        auth: Role::Reader,
        destructive: false,
        dry_run: false,
        examples: vec![],
        see_also: vec!["auth.login".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "recipes".into(),
        verb: "list".into(),
        summary: "List automation recipes for a board.".into(),
        description: "Returns the recipes defined on a board.".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        }],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "recipes": [] }),
        auth: Role::Admin,
        destructive: false,
        dry_run: false,
        examples: vec![],
        see_also: vec!["recipes.add".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "recipes".into(),
        verb: "add".into(),
        summary: "Add an automation recipe to a board.".into(),
        description: "Registers a recipe that runs actions when its trigger event fires.".into(),
        positional: vec![
            ArgSpec {
                name: "board".into(),
                r#type: ArgType::Board,
                required: true,
                help: "The board id.".into(),
            },
            ArgSpec {
                name: "name".into(),
                r#type: ArgType::Name,
                required: true,
                help: "Unique recipe name.".into(),
            },
        ],
        flags: vec![
            FlagSpec {
                name: "when".into(),
                r#type: FlagType::Json,
                default: Some("\"record.created\"".into()),
                allowed: vec![],
                repeatable: false,
                help: "Trigger: event string or {event, when} object.".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "actions".into(),
                r#type: FlagType::Json,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Array of action objects ($compute/$set/$copy/$call/...).".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "match".into(),
                r#type: FlagType::Json,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Optional filter condition on the triggering payload.".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "enabled".into(),
                r#type: FlagType::Bool,
                default: Some("true".into()),
                allowed: vec![],
                repeatable: false,
                help: "Whether the recipe is active.".into(),
                conflicts: vec![],
                requires: vec![],
            },
        ],
        body_json: None,
        response: serde_json::json!({ "ok": true, "name": "<name>" }),
        auth: Role::Admin,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "recipes add b_abc 'welcome' --when '{\"event\":\"record.created\"}' --actions '[{\"$compute\":{\"$.greeted\":true}}]'".into(),
            description: "Add a recipe.".into(),
            response: serde_json::json!({ "ok": true, "name": "welcome" }),
        }],
        see_also: vec!["recipes.list".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "files".into(),
        verb: "list".into(),
        summary: "List files stored for a board.".into(),
        description: "Returns file references (id + size).".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        }],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "files": [] }),
        auth: Role::Reader,
        destructive: false,
        dry_run: false,
        examples: vec![],
        see_also: vec!["files.get".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "files".into(),
        verb: "put".into(),
        summary: "Store a static asset for a board (front-end hosting).".into(),
        description: "Writes raw bytes to the board's per-board asset namespace (e.g. index.html, app.js).".into(),
        positional: vec![
            ArgSpec {
                name: "board".into(),
                r#type: ArgType::Board,
                required: true,
                help: "The board id.".into(),
            },
            ArgSpec {
                name: "path".into(),
                r#type: ArgType::Path,
                required: true,
                help: "Asset path within the board (e.g. index.html, css/app.css).".into(),
            },
        ],
        flags: vec![FlagSpec {
            name: "content".into(),
            r#type: FlagType::Str,
            default: None,
            allowed: vec![],
            repeatable: false,
            help: "File contents (text).".into(),
            conflicts: vec![],
            requires: vec![],
        }],
        body_json: None,
        response: serde_json::json!({ "ok": true, "asset": "<path>" }),
        auth: Role::Writer,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "files put b_abc index.html --content '<h1>hi</h1>'".into(),
            description: "Store an asset.".into(),
            response: serde_json::json!({ "ok": true, "asset": "index.html" }),
        }],
        see_also: vec!["files.list".into(), "files.delete".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "files".into(),
        verb: "get".into(),
        summary: "Fetch a static asset's contents.".into(),
        description: "Returns the raw bytes of a stored asset as text.".into(),
        positional: vec![
            ArgSpec {
                name: "board".into(),
                r#type: ArgType::Board,
                required: true,
                help: "The board id.".into(),
            },
            ArgSpec {
                name: "path".into(),
                r#type: ArgType::Path,
                required: true,
                help: "Asset path within the board.".into(),
            },
        ],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "asset": "<path>", "content": "<bytes>" }),
        auth: Role::Reader,
        destructive: false,
        dry_run: false,
        examples: vec![],
        see_also: vec!["files.put".into(), "files.list".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "files".into(),
        verb: "delete".into(),
        summary: "Delete a static asset.".into(),
        description: "Removes an asset from the board's asset namespace.".into(),
        positional: vec![
            ArgSpec {
                name: "board".into(),
                r#type: ArgType::Board,
                required: true,
                help: "The board id.".into(),
            },
            ArgSpec {
                name: "path".into(),
                r#type: ArgType::Path,
                required: true,
                help: "Asset path within the board.".into(),
            },
        ],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "asset": "<path>", "deleted": true }),
        auth: Role::Writer,
        destructive: true,
        dry_run: true,
        examples: vec![],
        see_also: vec!["files.put".into(), "files.list".into()],
        danger_notes: Some("permanently removes the asset".into()),
    });

    specs.push(CommandSpec {
        group: "jobs".into(),
        verb: "add".into(),
        summary: "Schedule a cron job on a board.".into(),
        description: "Registers a cron job that runs an http/poll action on the given schedule.".into(),
        positional: vec![
            ArgSpec {
                name: "board".into(),
                r#type: ArgType::Board,
                required: true,
                help: "The board id.".into(),
            },
            ArgSpec {
                name: "name".into(),
                r#type: ArgType::Name,
                required: true,
                help: "Unique job name (per board).".into(),
            },
            ArgSpec {
                name: "schedule".into(),
                r#type: ArgType::Str,
                required: true,
                help: "Cron expression: 'sec min hour dom month dow' or '@every 30s'.".into(),
            },
        ],
        flags: vec![FlagSpec {
            name: "action".into(),
            r#type: FlagType::Json,
            default: None,
            allowed: vec![],
            repeatable: false,
            help: "Action object: {type: http|poll, url, timeout_ms, headers, body, path}.".into(),
            conflicts: vec![],
            requires: vec![],
        }],
        body_json: None,
        response: serde_json::json!({ "ok": true, "name": "<name>", "schedule": "<schedule>" }),
        auth: Role::Admin,
        destructive: false,
        dry_run: false,
        examples: vec![Example {
            args: "jobs add b_abc 'heartbeat' '*/30 * * * * *' --action '{\"type\":\"http\",\"url\":\"https://example.com/ping\"}'".into(),
            description: "Schedule a job every 30 seconds.".into(),
            response: serde_json::json!({ "ok": true, "name": "heartbeat", "schedule": "*/30 * * * * *" }),
        }],
        see_also: vec!["jobs.list".into(), "jobs.runs".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "jobs".into(),
        verb: "list".into(),
        summary: "List cron jobs for a board.".into(),
        description: "Returns jobs with their schedule and last run status.".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        }],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "jobs": [] }),
        auth: Role::Admin,
        destructive: false,
        dry_run: false,
        examples: vec![],
        see_also: vec!["jobs.add".into(), "jobs.runs".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "jobs".into(),
        verb: "runs".into(),
        summary: "List recent job runs for a board.".into(),
        description: "Returns run history (status, duration, message).".into(),
        positional: vec![ArgSpec {
            name: "board".into(),
            r#type: ArgType::Board,
            required: true,
            help: "The board id.".into(),
        }],
        flags: vec![
            FlagSpec {
                name: "job".into(),
                r#type: FlagType::Str,
                default: None,
                allowed: vec![],
                repeatable: false,
                help: "Filter runs to one job name.".into(),
                conflicts: vec![],
                requires: vec![],
            },
            FlagSpec {
                name: "limit".into(),
                r#type: FlagType::Int,
                default: Some("20".into()),
                allowed: vec![],
                repeatable: false,
                help: "Max runs (1..=500).".into(),
                conflicts: vec![],
                requires: vec![],
            },
        ],
        body_json: None,
        response: serde_json::json!({ "ok": true, "runs": [] }),
        auth: Role::Admin,
        destructive: false,
        dry_run: false,
        examples: vec![],
        see_also: vec!["jobs.list".into()],
        danger_notes: None,
    });

    specs.push(CommandSpec {
        group: "jobs".into(),
        verb: "remove".into(),
        summary: "Remove a cron job.".into(),
        description: "Deletes the job and stops future scheduling.".into(),
        positional: vec![
            ArgSpec {
                name: "board".into(),
                r#type: ArgType::Board,
                required: true,
                help: "The board id.".into(),
            },
            ArgSpec {
                name: "name".into(),
                r#type: ArgType::Name,
                required: true,
                help: "Job name.".into(),
            },
        ],
        flags: vec![],
        body_json: None,
        response: serde_json::json!({ "ok": true, "removed": true }),
        auth: Role::Admin,
        destructive: true,
        dry_run: true,
        examples: vec![],
        see_also: vec!["jobs.list".into()],
        danger_notes: Some("stops the job; run history is kept".into()),
    });

    specs
}
