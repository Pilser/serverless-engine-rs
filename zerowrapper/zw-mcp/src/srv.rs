use std::path::Path;
use std::sync::Arc;
use serde_json::{json, Value};
use zw_db::serverless::{Agg, Op, SrvFilter, parse_filter};
use zw_db::serverless::SrvApp;
use zw_db::DbPool;
use zw_db::SessionStore;

const USAGE: &str =
    "usage: srv <group> <verb> [args]\n\
     groups:\n\
       apps      manage the app/board itself (create, schema, keys, hooks, rate, ttl, link, audit, jobs, export/import)\n\
       records   read/write/search/aggregate the records, call external APIs\n\
       recipes   event-driven automation (record.created/updated/deleted, inbound, cron)\n\
       secrets   store encrypted API keys/tokens for a board\n\
       files     list/download uploaded files\n\
     \n\
     apps:\n\
       srv apps create --title \"<title>\" [--json <schema>] [--public-reads on|off] [--unique '<$.path>']\n\
       srv apps list\n\
       srv apps show <board_id>\n\
       srv apps schema <board_id>\n\
       srv apps update <board_id> [--title \"<title>\"] [--json <schema> | --clear-schema] [--public-reads on|off] [--unique '<$.path>' | --clear-unique]\n\
       srv apps delete <board_id>\n\
       srv apps export <board_id> [--out <path>]\n\
       srv apps import '<json>|<path>|-' [--out <path>]\n\
       srv apps keys <board_id> list | issue --role writer|reader|admin [--writer <id>] | revoke --bucket <bucket>\n\
       srv apps hooks <board_id> register <url> [--secret <s>] | list | remove <url> | deliveries [--url <u>] | replay <url>\n\
       srv apps rate <board_id> '<{\"submit\":60,...}>' | srv apps rate <board_id> --clear\n\
       srv apps ttl <board_id> [<seconds>] [--ttl-until '<$.path>'] | srv apps ttl <board_id> --clear\n\
       srv apps link <board_id> --on '$.k' --to <parent> --via '$.k' | srv apps link <board_id> --clear\n\
       srv apps audit <board_id> on|off | srv apps audit <board_id> (list)\n\
       srv apps jobs <board_id> add --name <n> --schedule '<cron|@every 30s>' --action '<json>' | list | remove <job_id>\n\
     records:\n\
       srv records submit <board_id> <json> [--writer <id>] [--upsert]\n\
       srv records bulk <board_id> '<json array>' [--writer <id>] [--upsert]\n\
       srv records list <board_id> [--limit N] [--offset N]\n\
       srv records get <board_id> --seq N\n\
       srv records update <board_id> <seq> '<json>' [--writer <id>]\n\
       srv records patch <board_id> <seq> '<ops>'\n\
       srv records query <board_id> '<$.path>' [--eq value]\n\
       srv records query <board_id> --filter '<json>' [--order '<$.path>' asc|desc]* [--limit N] [--offset N]\n\
       srv records search <board_id> '<query>' [--limit N] [--offset N] [--hl]\n\
       srv records count <board_id>\n\
       srv records aggregate <board_id> --op sum|avg|min|max|count --field '<$.path>' [--group_by '<$.path>'] [--filter '<json>']\n\
       srv records delete <board_id> [--seq N | --all | --filter '<json>']\n\
       srv records audit <board_id> [--since <iso-ts>] [--limit N]\n\
       srv records call <board_id> <url> [--method GET|POST|PUT|PATCH|DELETE] [--header 'Name: Value']* [--body '<json>'] [--timeout N]\n\
     filter ops: eq neq gt gte lt lte contains not_contains in search\n\
     filter json: {\"field\":\"$.price\",\"op\":\"gte\",\"value\":500} | {\"$.price\":{\"gte\":500}} | {\"search\":\"q\",\"price\":{\"gte\":100}}\n\
     search = ranked full-text (BM25) across the whole record payload, combinable with other ops.\n\
     jobs action json: {\"type\":\"http\",\"url\":...,\"method\":\"GET\",\"headers\":[[\"k\",\"v\"]],\"body\":...,\"timeout_ms\":15000}\n\
     jobs action type poll: {\"type\":\"poll\",\"url\":...,\"path\":\"$.field\",...} inserts the extracted object as a new record.\n\
     files:\n\
       srv files list <board_id>\n\
       srv files get <board_id> <file_id> --out <path>\n\
     recipes:\n\
       srv recipes list <board_id>\n\
       srv recipes add <board_id> --name <n> --when '<event>' [--match '<json>'] [--dedup '<$.path>'] [--actions '<json>']\n\
       srv recipes enable <board_id> <name> on|off\n\
       srv recipes remove <board_id> <name>\n\
     secrets:\n\
       srv secrets set <board_id> <NAME> <value>\n\
       srv secrets list <board_id>\n\
       srv secrets remove <board_id> <NAME>\n\
     apps config verbs:\n\
       srv apps computed <board_id> '<json>' | --clear | (show)\n\
       srv apps validate <board_id> '<json>' | --clear | (show)\n\
       srv apps redact <board_id> '<json>' | --clear | (show)\n\
     recipe events: record.created, record.updated, record.deleted, inbound, cron:<job_name>\n\
     recipe actions json (array): [{\"$set\":{\"status\":\"paid\"}}, {\"$call\":{\"url\":\"https://...\",\"method\":\"POST\"}}]";

type CmdResult = Result<Value, Value>;

const APPS_USAGE: &str =
    "srv apps <verb> [args]\n\
     create --title \"<title>\" [--json <schema>] [--public-reads on|off] [--unique '<$.path>']\n\
     list\n\
     show <board_id>\n\
     schema <board_id>\n\
     update <board_id> [--title \"t\"] [--json <schema> | --clear-schema] [--public-reads on|off] [--unique '<$.path>' | --clear-unique]\n\
     delete <board_id>\n\
     export <board_id> [--out <path>]\n\
     import '<json>|<path>|-' (creates a new app from a dump; stdin when '-'; reads a file when the path exists)\n\
     keys <board_id> list | issue --role writer|reader|admin [--writer <id>] | revoke --bucket <bucket>\n\
     hooks <board_id> register <url> [--secret <s>] | list | remove <url> | deliveries [--url <u>] | replay <url>\n\
     rate <board_id> '<{\"submit\":60,...}>' | rate <board_id> --clear\n\
     ttl <board_id> [<seconds>] [--ttl-until '<$.path>'] | ttl <board_id> --clear\n\
     link <board_id> --on '$.k' --to <parent> --via '$.k' | link <board_id> --clear\n\
     audit <board_id> on|off | audit <board_id> (list)\n\
     jobs <board_id> add --name <n> --schedule '<cron|@every 30s>' --action '<json>' | list | remove <job_id>\n\
     cron: 5 fields (min hour dom month dow); * */n a-b a,b,c; or '@every <N><s|m|h|d>'.\n\
     jobs action: {\"type\":\"http\",\"url\":...,\"method\":\"GET\",\"headers\":[[\"k\",\"v\"]],\"body\":...,\"timeout_ms\":15000}\n\
     jobs action type \"poll\": additionally takes \"path\" (dotted JSON path) and stores the extracted object as a record.\n";

const RECORDS_USAGE: &str =
    "srv records <verb> [args]\n\
     submit <board_id> <json> [--writer <id>] [--upsert]\n\
     bulk <board_id> '<json array>' [--writer <id>] [--upsert]\n\
     list <board_id> [--limit N] [--offset N]\n\
     get <board_id> --seq N\n\
     update <board_id> <seq> '<json>' [--writer <id>]\n\
     patch <board_id> <seq> '<ops>'\n\
     query <board_id> '<$.path>' [--eq value]\n\
     query <board_id> --filter '<json>' [--order '<$.path>' asc|desc]* [--limit N] [--offset N]\n\
     search <board_id> '<query>' [--limit N] [--offset N] [--hl]\n\
     count <board_id>\n\
     aggregate <board_id> --op sum|avg|min|max|count --field '<$.path>' [--group_by '<$.path>'] [--filter '<json>']\n\
     delete <board_id> [--seq N | --all | --filter '<json>']\n\
     audit <board_id> [--since <iso-ts>] [--limit N]\n\
     call <board_id> <url> [--method GET|POST|PUT|PATCH|DELETE] [--header 'Name: Value']* [--body '<json>'] [--timeout N]\n\
     query <path> uses JSONPath: '<$.name>', '<$.a.b>', '<$.arr[0]>' (leading $ required).\n\
     patch json: {\"$set\":{\"color\":\"black\"},\"$inc\":{\"votes\":1},\"$dec\":{...},\"$mul\":{...},\"$unset\":[\"field\",\"a.b[0]\"]}\n\
     writer filter form: {\"writer\":{\"eq\":\"alice\"}}\n\
     --unique: extra submit of the same unique value is rejected unless --upsert; upsert replaces the existing record keeping its seq.\n\
     filter ops: eq neq gt gte lt lte contains not_contains in search\n\
     filter json: {\"field\":\"$.price\",\"op\":\"gte\",\"value\":500} | {\"$.price\":{\"gte\":500}} | {\"search\":\"q\",\"price\":{\"gte\":5}}\n\
     search does ranked full-text (BM25) across the whole record payload; combines with other filter ops.\n";

const FILES_USAGE: &str =
    "srv files <verb> [args]\n\
     list <board_id>\n\
     get <board_id> <file_id> --out <path>\n";

const RECIPES_USAGE: &str =
    "srv recipes <verb> [args]\n\
     list <board_id>\n\
     add <board_id> --name <n> --when '<event>' [--match '<json>'] [--dedup '<$.path>'] [--actions '<json>']\n\
     enable <board_id> <name> on|off\n\
     remove <board_id> <name>\n\
     events: record.created, record.updated, record.deleted, inbound, cron:<job_name>\n\
     actions json is an array of action objects, e.g. [{\"$set\":{\"status\":\"paid\"}}]\n";

const SECRETS_USAGE: &str =
    "srv secrets <verb> [args]\n\
     set <board_id> <NAME> <value>\n\
     list <board_id>\n\
     remove <board_id> <NAME>\n\
     names are stored uppercased; values are encrypted at rest and never listed back.\n";

fn err_json(error: &str, command: &str, usage: &str, hint: &str) -> Value {
    json!({
        "status": "error",
        "error": error,
        "command": command,
        "hint": hint,
        "usage": usage,
    })
}

fn val_after<'a>(args: &'a [&String], flag: &str) -> Option<String> {
    for (i, tok) in args.iter().enumerate() {
        if tok.as_str() == flag {
            return args.get(i + 1).map(|v| v.as_str().trim_matches('"').to_string());
        }
    }
    None
}

fn has_flag(args: &[&String], flag: &str) -> bool {
    args.iter().any(|t| t.as_str() == flag)
}

fn orders_from(args: &[&String]) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for (i, tok) in args.iter().enumerate() {
        if tok.as_str() != "--order" {
            continue;
        }
        let Some(path) = args.get(i + 1).map(|p| unquote(p)).filter(|p| !p.is_empty()) else {
            continue;
        };
        let desc = match args.get(i + 2).map(|d| d.as_str()) {
            Some("desc") => true,
            _ => false,
        };
        out.push((path, desc));
    }
    out
}

fn split_search(conds: &[zw_db::filters::FilterCond]) -> Option<(String, Vec<zw_db::filters::FilterCond>)> {
    let mut query = None;
    let mut others = Vec::new();
    for c in conds {
        if c.op == Op::Search {
            query = c.value.as_str().map(|s| s.to_string());
        } else {
            others.push(c.clone());
        }
    }
    query.map(|q| (q, others))
}

fn unquote(s: &str) -> String {
    let s = s.trim_start_matches('\"').trim_start_matches('\'');
    let s = s.trim_end_matches('\"').trim_end_matches('\'');
    s.to_string()
}

/// Rebuild a flag's value from the possibly-many whitespace-split tokens that follow
/// it (JSON schemas and titles contain spaces), stopping at the next `--` flag.
fn joined_flag(args: &[&String], flag: &str) -> Option<String> {
    for (i, tok) in args.iter().enumerate() {
        if tok.as_str() == flag {
            let parts: Vec<&str> = args[i + 1..].iter().map(|v| v.as_str()).collect();
            let parts: Vec<&str> = parts.iter().take_while(|p| !p.starts_with("--")).map(|p| *p).collect();
            if parts.is_empty() {
                return None;
            }
            return Some(unquote(&parts.join(" ")));
        }
    }
    None
}

/// Collect every value that follows a repeatable flag (e.g. `--header 'A: 1' --header 'B: 2'`).
fn all_flags(args: &[&String], flag: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, tok) in args.iter().enumerate() {
        if tok.as_str() == flag {
            if let Some(v) = args.get(i + 1) {
                out.push(unquote(v));
            }
        }
    }
    out
}

fn public_mode(args: &[&String], raw: &str, verb: &str) -> Result<bool, Value> {
    match val_after(args, "--public-reads") {
        Some(v) if v == "on" => Ok(true),
        Some(v) if v == "off" => Ok(false),
        Some(other) => Err(err_json(
            &format!("expected on or off for --public-reads, got '{other}'"),
            raw,
            verb,
            verb,
        )),
        None => Ok(false),
    }
}

async fn require_board(store: &SessionStore, board_id: &str, sid: &str, raw: &str, read: bool) -> Result<(SrvApp, Arc<DbPool>), Value> {
    let pool = store
        .board_pool(board_id)
        .await
        .map_err(|e| err_json(&e.to_string(), raw, "srv <group> <verb>", "srv apps list"))?;
    if let Some(app) = pool
        .srv_app_owned(board_id, sid)
        .await
        .map_err(|e| err_json(&e.to_string(), raw, "srv <group> <verb>", "srv apps list"))?
    {
        return Ok((app, pool));
    }
    // Not owned by this session. Public boards may be read cross-session.
    if read {
        if let Ok(Some(app)) = pool.srv_app_by_id(board_id).await {
            if app.public_reads {
                return Ok((app, pool));
            }
        }
    }
    let owner = store
        .session_id_for_board(board_id)
        .await
        .ok()
        .filter(|o| o.as_str() != sid);
    let msg = match owner {
        Some(o) => format!("board {board_id} is owned by session {o} — retry with --session {o}"),
        None => format!("board {board_id} not found or not owned"),
    };
    Err(err_json(
        &msg,
        raw,
        "srv <group> <verb>",
        "srv apps list",
    ))
}

/// Shared handler for `srv apps computed|validate|redact <board_id> [json|--clear]`.
/// With a JSON argument it sets the config; with --clear it removes it;
/// with neither it prints the current config.
async fn config_verb(store: &SessionStore, args: &[&String], sid: &str, raw: &str, kind: &str) -> CmdResult {
    let board_id = args.first().ok_or_else(|| {
        err_json(
            &format!("missing <board_id> for srv apps {kind}"),
            raw,
            &format!("srv apps {kind} <board_id> '<json>'"),
            &format!("srv apps {kind} <board_id> '{{\"total\":\"$.price * $.qty\"}}'"),
        )
    })?;
    let (app, pool) = require_board(store, board_id, sid, raw, false).await?;
    let rest = &args[1..];
    if has_flag(rest, "--clear") {
        let set = match kind {
            "computed" => pool.srv_app_set_computed(board_id, &app.owner_key, None).await,
            "validate" => pool.srv_app_set_validate(board_id, &app.owner_key, None).await,
            "redact" => pool.srv_app_set_redact(board_id, &app.owner_key, None).await,
            _ => unreachable!(),
        };
        set.map_err(|e| err_json(&e.to_string(), raw, &format!("srv apps {kind} --clear"), &format!("srv apps {kind} <board_id> --clear")))?;
        return Ok(json!({ "status": "ok", "clear": true }));
    }
    if let Some(first) = rest.first().filter(|t| !t.starts_with("--")) {
        let raw_json = unquote(first);
        let parsed: Value = serde_json::from_str(&raw_json)
            .map_err(|e| err_json(&format!("invalid json for srv apps {kind}: {e}"), raw, &format!("srv apps {kind} <board_id> '<json>'"), &format!("srv apps {kind} <board_id> '{{\"total\":\"$.price * $.qty\"}}'")))?;
        let set = match kind {
            "computed" => pool.srv_app_set_computed(board_id, &app.owner_key, Some(&raw_json)).await,
            "validate" => pool.srv_app_set_validate(board_id, &app.owner_key, Some(&raw_json)).await,
            "redact" => pool.srv_app_set_redact(board_id, &app.owner_key, Some(&raw_json)).await,
            _ => unreachable!(),
        };
        set.map_err(|e| err_json(&e.to_string(), raw, &format!("srv apps {kind}"), &format!("srv apps {kind} <board_id> '<json>'")))?;
        return Ok(json!({ "status": "ok", "kind": kind, "set": parsed }));
    }
    let cur = match kind {
        "computed" => pool.srv_computed_get(board_id).await,
        "validate" => pool.srv_validate_get(board_id).await,
        "redact" => pool.srv_redact_get(board_id).await,
        _ => unreachable!(),
    }
    .map_err(|e| err_json(&e.to_string(), raw, &format!("srv apps {kind}"), &format!("srv apps {kind} <board_id>")))?;
    Ok(json!({ "status": "ok", "kind": kind, "config": cur }))
}

async fn cmd_apps(store: &SessionStore, sid: &str, verb: &str, args: &[&String], raw: &str) -> CmdResult {
    if verb == "--help" {
        return Ok(json!({ "status": "ok", "help": APPS_USAGE }));
    }
    if args.first().map(|s| s.as_str()) == Some("--help") {
        if let Some(h) = crate::srv_help::verb_help("apps", verb) {
            return Ok(json!({ "status": "ok", "help": h }));
        }
        return Err(err_json(
            &format!("no detailed help for 'srv apps {verb}'"),
            raw,
            "srv apps <verb>",
            APPS_USAGE,
        ));
    }
    match verb {
        "create" => {
            let title = joined_flag(args, "--title").unwrap_or_else(|| "Untitled".to_string());
            let schema = joined_flag(args, "--json");
            let public_reads = public_mode(args, raw, "srv apps create")?;
            let unique_key = joined_flag(args, "--unique");
            let board_id = store
                .create_board(sid, &title, schema.as_deref(), public_reads, unique_key.as_deref())
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "create", "create a new app"))?;
            Ok(json!({ "status": "ok", "board_id": board_id }))
        }
        "list" => {
            let pool = store.session(sid).await.map_err(|e| err_json(&e.to_string(), raw, "list", "list apps"))?;
            let apps = pool.srv_app_list(sid).await.map_err(|e| err_json(&e.to_string(), raw, "list", "list apps"))?;
            Ok(json!({ "status": "ok", "apps": apps }))
        }
        "show" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps show <board_id>", "srv apps show <board_id>")
            })?;
            let (app, _pool) = require_board(store, board_id, sid, raw, true).await?;
            Ok(json!({ "status": "ok", "app": app }))
        }
        "schema" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps schema <board_id>", "srv apps schema <board_id>")
            })?;
            let (app, _pool) = require_board(store, board_id, sid, raw, true).await?;
            Ok(json!({
                "status": "ok",
                "board_id": app.board_id,
                "schema": app.schema_json,
            }))
        }
        "find" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps find <board_id>", "srv apps find <board_id>")
            })?;
            let owner = store
                .session_id_for_board(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps find", "srv apps find <board_id>"))?;
            let pool = store
                .session(&owner)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps find", "srv apps find <board_id>"))?;
            let app = pool
                .srv_app_by_id(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps find", "srv apps find <board_id>"))?
                .ok_or_else(|| err_json(&format!("board {board_id} not found"), raw, "srv apps find", "srv apps find <board_id>"))?;
            Ok(json!({
                "status": "ok",
                "board_id": app.board_id,
                "title": app.title,
                "owner_session": owner,
                "owner_key": app.owner_key,
                "public_reads": app.public_reads,
                "hint": format!("retry with --session {owner}"),
            }))
        }
        "update" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps update <board_id>", "srv apps update <board_id>")
            })?;
            let (_app, pool) = require_board(store, board_id, sid, raw, false).await?;
            let title = joined_flag(args, "--title");
            let schema: Option<Option<String>> = if has_flag(args, "--clear-schema") {
                Some(None)
            } else if let Some(s) = joined_flag(args, "--json") {
                Some(Some(s))
            } else {
                None
            };
            let unique_key: Option<Option<String>> = if has_flag(args, "--clear-unique") {
                Some(None)
            } else if let Some(u) = joined_flag(args, "--unique") {
                Some(Some(u))
            } else {
                None
            };
            let public_reads: Option<bool> = match val_after(args, "--public-reads") {
                Some(v) if v == "on" => Some(true),
                Some(v) if v == "off" => Some(false),
                Some(other) => {
                    return Err(err_json(
                        &format!("expected on or off for --public-reads, got '{other}'"),
                        raw,
                        "srv apps update <board_id>",
                        "srv apps update <board_id>",
                    ))
                }
                None => None,
            };
            pool.srv_app_update(
                board_id,
                sid,
                title.as_deref(),
                schema.as_ref().map(|s| s.as_deref()),
                public_reads,
                unique_key.as_ref().map(|u| u.as_deref()),
            )
            .await
            .map_err(|e| err_json(&e.to_string(), raw, "srv apps update <board_id>", "srv apps update <board_id>"))?;
            Ok(json!({ "status": "ok", "updated": true }))
        }
        "delete" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps delete <board_id>", "srv apps delete <board_id>")
            })?;
            require_board(store, board_id, sid, raw, false).await?;
            store.delete_board(sid, board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps delete", "srv apps delete <board_id>"))?;
            Ok(json!({ "status": "ok", "deleted": true }))
        }
        "export" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps export <board_id> [--out <path>]", "srv apps export <board_id>")
            })?;
            let (app, pool) = require_board(store, board_id, sid, raw, true).await?;
            let rate = pool
                .srv_rate_config(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps export", "srv apps export <board_id>"))?;
            let ttl = pool
                .srv_ttl_get(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps export", "srv apps export <board_id>"))?;
            let audit = pool
                .srv_audit_enabled(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps export", "srv apps export <board_id>"))?;
            let link = pool
                .srv_link_get(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps export", "srv apps export <board_id>"))?;
            let hooks = pool
                .srv_hook_list(board_id, sid)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps export", "srv apps export <board_id>"))?;
            let mut records: Vec<Value> = Vec::new();
            let mut offset = 0i64;
            loop {
                let batch = pool
                    .srv_record_list(board_id, 500, None, offset)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps export", "srv apps export <board_id>"))?;
                if batch.is_empty() {
                    break;
                }
                let n = batch.len() as i64;
                records.extend(batch.into_iter().map(|r| serde_json::to_value(&r).unwrap_or(Value::Null)));
                offset += n;
            }
            let dump = json!({
                "format": "waos-srv-app",
                "app": app,
                "rate": rate,
                "ttl": ttl,
                "audit": audit,
                "link": link,
                "hooks": hooks,
                "records": records,
            });
            if let Some(out) = val_after(args, "--out") {
                std::fs::write(&out, serde_json::to_string_pretty(&dump).unwrap_or_default())
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps export", "srv apps export <board_id> --out <path>"))?;
                Ok(json!({ "status": "ok", "board_id": board_id, "records": records.len(), "wrote": out }))
            } else {
                Ok(json!({ "status": "ok", "dump": dump }))
            }
        }
        "import" => {
            let src = args
                .first()
                .map(|s| unquote(s))
                .filter(|s| !s.is_empty())
                .ok_or_else(|| err_json("missing '<json>|<path>|-'", raw, "srv apps import '<json>|<path>|-'", "srv apps import dump.json"))?;
            let text = if src == "-" {
                let mut buf = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps import", "srv apps import -"))?;
                buf
            } else if Path::new(&src).is_file() {
                std::fs::read_to_string(&src)
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps import", "srv apps import <path>"))?
            } else {
                src.clone()
            };
            let dump: Value = serde_json::from_str(&text).map_err(|_| {
                err_json("invalid dump json", raw, "srv apps import '<json>|<path>|-'", "srv apps import dump.json")
            })?;
            let title = dump
                .get("app")
                .and_then(|a| a.get("title"))
                .and_then(|t| t.as_str())
                .unwrap_or("Imported app")
                .to_string();
            let schema = dump.get("app").and_then(|a| a.get("schema_json")).and_then(|s| s.as_str());
            let public_reads = dump.get("app").and_then(|a| a.get("public_reads")).and_then(|b| b.as_bool()).unwrap_or(false);
            let unique = dump.get("app").and_then(|a| a.get("unique_key")).and_then(|u| u.as_str());
            let board_id = store
                .create_board(sid, &title, schema, public_reads, unique)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps import", "srv apps import <json>"))?;
            let pool = store
                .board_pool(&board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps import", "srv apps import <json>"))?;
            let mut recs: Vec<Value> = Vec::new();
            if let Some(records) = dump.get("records").and_then(|r| r.as_array()) {
                for r in records {
                    if let Some(p) = r.get("payload").and_then(|p| p.as_object()) {
                        recs.push(Value::Object(p.clone()));
                    }
                }
            }
            if !recs.is_empty() {
                pool.srv_record_bulk_insert(&board_id, &recs, None, false)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps import", "srv apps import <json>"))?;
            }
            if let Some(rate) = dump.get("rate") {
                let _ = pool.srv_rate_set(&board_id, sid, Some(&serde_json::to_string(rate).unwrap_or_default())).await;
            }
            if let Some(secs) = dump.get("ttl").and_then(|t| t.get("seconds")).and_then(|s| s.as_i64()) {
                let field = dump.get("ttl").and_then(|t| t.get("field")).and_then(|f| f.as_str());
                pool.srv_ttl_set(&board_id, sid, Some(secs), field)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps import", "srv apps import <json>"))?;
            }
            if let Some(a) = dump.get("audit").and_then(|x| x.as_bool()) {
                pool.srv_app_audit_set(&board_id, sid, a)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps import", "srv apps import <json>"))?;
            }
            if let Some(link) = dump.get("link") {
                if let (Some(f), Some(p), Some(v)) = (
                    link.get("from_key").and_then(|x| x.as_str()),
                    link.get("parent_board").and_then(|x| x.as_str()),
                    link.get("parent_key").and_then(|x| x.as_str()),
                ) {
                    pool.srv_link_set(&board_id, sid, f, p, v)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps import", "srv apps import <json>"))?;
                }
            }
            if let Some(hooks) = dump.get("hooks").and_then(|x| x.as_array()) {
                for h in hooks {
                    if let Some(u) = h.get("url").and_then(|x| x.as_str()) {
                        let _ = pool.srv_hook_register(&board_id, sid, u, None).await;
                    }
                }
            }
            Ok(json!({ "status": "ok", "board_id": board_id, "records": recs.len() }))
        }
        "keys" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps keys <board_id> <verb>", "srv apps keys <board_id> list")
            })?;
            let sub = args.get(1).map(|s| s.as_str()).unwrap_or("");
            let rest = &args[2..];
            match sub {
                "list" => {
                    let keys = store
                        .key_list(board_id, sid)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps keys list", "srv apps keys <board_id> list"))?;
                    let keys: Vec<Value> = keys
                        .iter()
                        .map(|k| json!({
                            "bucket": k.bucket,
                            "role": k.role,
                            "writer": k.writer,
                            "scope": k.scope,
                            "created_at": k.created_at,
                            "revoked": k.revoked_at,
                        }))
                        .collect();
                    Ok(json!({ "status": "ok", "keys": keys }))
                }
                "issue" => {
                    let role = val_after(rest, "--role").unwrap_or_else(|| "writer".to_string());
                    if role == "customer" {
                        return Err(err_json(
                            "customer keys are storefront REST only",
                            raw,
                            "srv apps keys issue",
                            "srv apps keys <board_id> issue --role admin|writer|reader|list",
                        ));
                    }
                    let writer = joined_flag(rest, "--writer");
                    let (bucket, secret) = store
                        .key_issue(board_id, sid, &role, writer.as_deref(), None)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps keys issue", "srv apps keys <board_id> issue --role writer"))?;
                    Ok(json!({ "status": "ok", "key": secret, "bucket": bucket }))
                }
                "revoke" => {
                    let bucket = val_after(rest, "--bucket").ok_or_else(|| {
                        err_json(
                            "missing --bucket <bucket>",
                            raw,
                            "srv apps keys revoke --bucket <bucket>",
                            "srv apps keys <board_id> revoke --bucket <bucket>",
                        )
                    })?;
                    let revoked = store
                        .key_revoke(board_id, sid, &bucket)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps keys revoke", "srv apps keys <board_id> revoke --bucket <bucket>"))?;
                    Ok(json!({ "status": "ok", "revoked": revoked }))
                }
                other => Err(err_json(
                    &format!("unknown apps keys verb '{other}'"),
                    raw,
                    "srv apps keys <verb>",
                    USAGE,
                )),
            }
        }
        "hooks" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps hooks <board_id> <verb>", "srv apps hooks <board_id> list")
            })?;
            let sub = args.get(1).map(|s| s.as_str()).unwrap_or("");
            let rest = &args[2..];
            let (_app, pool) = require_board(store, board_id, sid, raw, false).await?;
            match sub {
                "register" => {
                    let url = rest.first().ok_or_else(|| {
                        err_json("missing <url>", raw, "srv apps hooks <board_id> register <url> [--secret ~]", "srv apps hooks <board_id> register https://example.com/webhook --secret s3cret")
                    })?;
                    let secret = joined_flag(rest, "--secret");
                    pool.srv_hook_register(board_id, sid, url, secret.as_deref())
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps hooks register", "srv apps hooks <board_id> register <url>"))?;
                    Ok(json!({ "status": "ok", "registered": true }))
                }
                "list" => {
                    let hooks = pool
                        .srv_hook_list(board_id, sid)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps hooks list", "srv apps hooks <board_id> list"))?;
                    Ok(json!({ "status": "ok", "hooks": hooks }))
                }
                "remove" => {
                    let url = rest.first().ok_or_else(|| {
                        err_json("missing <url>", raw, "srv apps hooks <board_id> remove <url>", "srv apps hooks <board_id> remove https://example.com/webhook")
                    })?;
                    pool.srv_hook_remove(board_id, sid, url)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps hooks remove", "srv apps hooks <board_id> remove <url>"))?;
                    Ok(json!({ "status": "ok", "removed": true }))
                }
                "deliveries" => {
                    let url = joined_flag(rest, "--url");
                    let deliveries = store
                        .hook_deliveries(board_id, sid, url.as_deref())
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps hooks deliveries", "srv apps hooks <board_id> deliveries [--url <u>]"))?;
                    Ok(json!({ "status": "ok", "deliveries": deliveries }))
                }
                "replay" => {
                    let url = rest.first().ok_or_else(|| {
                        err_json("missing <url>", raw, "srv apps hooks <board_id> replay <url>", "srv apps hooks <board_id> replay https://example.com/webhook")
                    })?;
                    let rescheduled = store
                        .hook_reschedule(board_id, sid, url)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps hooks replay", "srv apps hooks <board_id> replay <url>"))?;
                    Ok(json!({ "status": "ok", "rescheduled": rescheduled }))
                }
                other => Err(err_json(
                    &format!("unknown apps hooks verb '{other}'"),
                    raw,
                    "srv apps hooks <verb>",
                    USAGE,
                )),
            }
        }
        "rate" => {
            let board_id = args.first().ok_or_else(|| {
                err_json(
                    "missing <board_id>",
                    raw,
                    "srv apps rate <board_id> '<json>' | --clear",
                    "srv apps rate <board_id> '{\"submit\":60}'",
                )
            })?;
            let (_app, pool) = require_board(store, board_id, sid, raw, false).await?;
            if has_flag(args, "--clear") {
                pool.srv_rate_set(board_id, sid, None)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps rate --clear", "srv apps rate <board_id> --clear"))?;
                return Ok(json!({ "status": "ok", "clear": true }));
            }
            if args.len() < 2 {
                return Err(err_json(
                    "missing '<json>' rate config",
                    raw,
                    "srv apps rate <board_id> '<json>'",
                    "srv apps rate <board_id> '{\"submit\":60}'",
                ));
            }
            let json_tokens: Vec<&str> = args[1..]
                .iter()
                .map(|v| v.as_str())
                .take_while(|t| !t.starts_with("--"))
                .collect();
            let json_str = unquote(&json_tokens.join(" "));
            let cfg: Value = serde_json::from_str(&json_str).map_err(|_| {
                err_json(
                    "invalid JSON rate config",
                    raw,
                    "srv apps rate <board_id> '<json>'",
                    "srv apps rate <board_id> '{\"submit\":60}'",
                )
            })?;
            pool.srv_rate_set(board_id, sid, Some(&cfg.to_string()))
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps rate", "srv apps rate <board_id> '<json>'"))?;
            let merged = pool
                .srv_rate_config(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps rate", "srv apps rate <board_id> '<json>'"))?;
            let rate = serde_json::to_value(&merged).unwrap_or(Value::Null);
            Ok(json!({ "status": "ok", "rate": rate }))
        }
        "ttl" => {
            let board_id = args.first().ok_or_else(|| {
                err_json(
                    "missing <board_id>",
                    raw,
                    "srv apps ttl <board_id> [<seconds>] [--ttl-until '<$.path>']",
                    "srv apps ttl <board_id> 3600 --ttl-until '$.expires_at'",
                )
            })?;
            let (_app, pool) = require_board(store, board_id, sid, raw, false).await?;
            if has_flag(args, "--clear") {
                pool.srv_ttl_set(board_id, sid, None, None)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps ttl --clear", "srv apps ttl <board_id> --clear"))?;
                return Ok(json!({ "status": "ok", "clear": true }));
            }
            let field = joined_flag(args, "--ttl-until");
            let seconds: Option<i64> = args.get(1).and_then(|t| t.parse().ok());
            if let Some(tok) = args.get(1).filter(|t| !t.starts_with("--")) {
                if seconds.is_none() {
                    return Err(err_json(
                        &format!("'{tok}' is not an integer number of seconds"),
                        raw,
                        "srv apps ttl <board_id> [<seconds>]",
                        "srv apps ttl <board_id> 3600",
                    ));
                }
            }
            if seconds.is_none() && field.is_none() {
                let ttl = pool
                    .srv_ttl_get(board_id)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps ttl", "srv apps ttl <board_id>"))?;
                return Ok(json!({ "status": "ok", "ttl": ttl }));
            }
            pool.srv_ttl_set(board_id, sid, seconds, field.as_deref())
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps ttl", "srv apps ttl <board_id> <seconds>"))?;
            let ttl = pool
                .srv_ttl_get(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps ttl", "srv apps ttl <board_id>"))?;
            Ok(json!({ "status": "ok", "ttl": ttl }))
        }
        "link" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps link <board_id> --on '$.k' --to <parent> --via '$.k'", "srv apps link <child> --on '$.sku' --to <parent> --via '$.sku'")
            })?;
            let (_app, pool) = require_board(store, board_id, sid, raw, false).await?;
            if has_flag(args, "--clear") {
                pool.srv_link_clear(board_id, sid)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps link --clear", "srv apps link <child> --clear"))?;
                return Ok(json!({ "status": "ok", "clear": true }));
            }
            let from = joined_flag(args, "--on");
            let parent = joined_flag(args, "--to");
            let via = joined_flag(args, "--via");
            if from.is_none() && parent.is_none() && via.is_none() {
                let link = pool
                    .srv_link_get(board_id)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv apps link", "srv apps link <child>"))?;
                return Ok(json!({ "status": "ok", "link": link }));
            }
            if from.is_none() || parent.is_none() || via.is_none() {
                return Err(err_json(
                    "link needs --on '$.k' --to <parent_board> --via '$.k'",
                    raw,
                    "srv apps link <child> --on '$.sku' --to <parent> --via '$.sku'",
                    "srv apps link <child> --on '$.sku' --to <parent> --via '$.sku'",
                ));
            }
            pool.srv_link_set(board_id, sid, from.as_deref().unwrap(), parent.as_deref().unwrap(), via.as_deref().unwrap())
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps link", "srv apps link <child> --on '$.sku' --to <parent> --via '$.sku'"))?;
            let link = pool
                .srv_link_get(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps link", "srv apps link <child>"))?;
            Ok(json!({ "status": "ok", "link": link }))
        }
        "audit" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps audit <board_id>", "srv apps audit <board_id> on")
            })?;
            let on_off = args.get(1).map(|s| s.as_str()).unwrap_or_default();
            let (_app, pool) = require_board(store, board_id, sid, raw, false).await?;
            let enabled = match on_off {
                "on" | "1" | "true" => true,
                "off" | "0" | "false" => false,
                _ => {
                    let rows = pool
                        .srv_audit_list(board_id, sid, None, 100)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps audit", "srv apps audit <board_id> on|off"))?;
                    return Ok(json!({ "status": "ok", "audit": rows }));
                }
            };
            pool.srv_app_audit_set(board_id, sid, enabled)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv apps audit", "srv apps audit <board_id> on|off"))?;
            Ok(json!({ "status": "ok", "audit_enabled": enabled }))
        }
        "jobs" => {
            let board_id = args.first().ok_or_else(|| {
                err_json("missing <board_id>", raw, "srv apps jobs <board_id> <verb>", "srv apps jobs <board_id> list")
            })?;
            let sub = args.get(1).map(|s| s.as_str()).unwrap_or("");
            let rest = &args[2..];
            match sub {
                "add" => {
                    let name = joined_flag(rest, "--name").filter(|n| !n.is_empty()).ok_or_else(|| {
                        err_json("missing --name <name>", raw, "srv apps jobs <board_id> add --name <n> --schedule <cron> --action <json>", "srv apps jobs <board_id> add --name refresh --schedule '@every 30s' --action '{\"type\":\"http\",\"url\":\"https://api.example.com/x\"}'")
                    })?;
                    let schedule = joined_flag(rest, "--schedule").filter(|s| !s.is_empty()).ok_or_else(|| {
                        err_json("missing --schedule <cron|@every Ns>", raw, "srv apps jobs <board_id> add", "srv apps jobs <board_id> add --schedule '*/5 * * * *'")
                    })?;
                    let action = joined_flag(rest, "--action").filter(|a| !a.is_empty()).ok_or_else(|| {
                        err_json("missing --action '<json>'", raw, "srv apps jobs <board_id> add", "srv apps jobs <board_id> add --action '{\"type\":\"poll\",\"url\":\"https://api.example.com/x\",\"path\":\"$.data\"}'")
                    })?;
                    require_board(store, board_id, sid, raw, false).await?;
                    let (id, next_run_at) = store
                        .system()
                        .srv_job_add(board_id, &name, &schedule, &action)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps jobs add", "srv apps jobs <board_id> add"))?;
                    Ok(json!({ "status": "ok", "job_id": id, "schedule": schedule, "next_run_at": next_run_at }))
                }
                "list" => {
                    require_board(store, board_id, sid, raw, false).await?;
                    let jobs = store
                        .system()
                        .srv_job_list(Some(board_id))
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps jobs list", "srv apps jobs <board_id> list"))?;
                    Ok(json!({ "status": "ok", "jobs": jobs }))
                }
                "runs" => {
                    require_board(store, board_id, sid, raw, false).await?;
                    let job_id = args.get(2).and_then(|v| v.parse::<i64>().ok());
                    let limit: i64 = joined_flag(rest, "--limit")
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(50);
                    let runs = store
                        .system()
                        .srv_job_runs(board_id, job_id, limit)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps jobs runs", "srv apps jobs <board_id> runs [<job_id>] [--limit N]"))?;
                    Ok(json!({ "status": "ok", "runs": runs }))
                }
                "remove" => {
                    let id: i64 = rest
                        .first()
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| err_json("missing <job_id>", raw, "srv apps jobs <board_id> remove <job_id>", "srv apps jobs <board_id> remove 3"))?;
                    require_board(store, board_id, sid, raw, false).await?;
                    let removed = store
                        .system()
                        .srv_job_remove(id)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv apps jobs remove", "srv apps jobs <board_id> remove <job_id>"))?;
                    Ok(json!({ "status": "ok", "removed": removed }))
                }
                other => Err(err_json(
                    &format!("unknown apps jobs verb '{other}'"),
                    raw,
                    "srv apps jobs <verb>",
                    USAGE,
                )),
            }
        }
        "computed" => config_verb(store, args, sid, raw, "computed").await,
        "validate" => config_verb(store, args, sid, raw, "validate").await,
        "redact" => config_verb(store, args, sid, raw, "redact").await,
        other => Err(err_json(
            &format!("unknown apps verb '{other}'"),
            raw,
            "srv apps <verb>",
            USAGE,
        )),
    }
}

async fn cmd_records(store: &SessionStore, sid: &str, verb: &str, args: &[&String], raw: &str) -> CmdResult {
    if verb == "--help" {
        return Ok(json!({ "status": "ok", "help": RECORDS_USAGE }));
    }
    if args.first().map(|s| s.as_str()) == Some("--help") {
        if let Some(h) = crate::srv_help::verb_help("records", verb) {
            return Ok(json!({ "status": "ok", "help": h }));
        }
        return Err(err_json(
            &format!("no detailed help for 'srv records {verb}'"),
            raw,
            "srv records <verb>",
            RECORDS_USAGE,
        ));
    }
    let board_id = args
        .first()
        .ok_or_else(|| err_json("missing <board_id>", raw, "srv records <verb>", "srv records list"))?;
    let read = matches!(verb, "list" | "get" | "query" | "search" | "count" | "aggregate");
    let (app, pool) = require_board(store, board_id, sid, raw, read).await?;
    let rest = &args[1..];
    match verb {
        "audit" => cmd_audit_records(&pool, board_id, &app.owner_key, rest, raw).await,
        "call" => {
            let url = rest
                .iter()
                .take_while(|t| !t.starts_with("--"))
                .map(|v| unquote(v))
                .find(|u| !u.is_empty())
                .ok_or_else(|| err_json("missing <url>", raw, "srv records call <board_id> <url>", "srv records call <board_id> https://api.example.com/x"))?;
            let method = val_after(rest, "--method").unwrap_or_else(|| "GET".to_string());
            let headers: Vec<(String, String)> = all_flags(rest, "--header")
                .into_iter()
                .filter_map(|h| h.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                .collect();
            let body = joined_flag(rest, "--body");
            let timeout_ms: u64 = val_after(rest, "--timeout").and_then(|v| v.parse().ok()).unwrap_or(0);
            let res = pool
                .srv_http_call(board_id, &app.owner_key, &method, &url, &headers, body.as_deref(), timeout_ms)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records call", "srv records call <board_id> <url>"))?;
            Ok(json!({ "status": "ok", "call": res }))
        }
        "submit" => {
            if rest.is_empty() {
                return Err(err_json(
                    "missing <json> payload",
                    raw,
                    "srv records submit <board_id> <json>",
                    "srv records submit <board_id> '{\"k\":1}'",
                ));
            }
            let json_tokens: Vec<_> = rest
                .iter()
                .take_while(|t| !t.starts_with("--"))
                .map(|v| v.as_str())
                .collect();
            let json_str = unquote(&json_tokens.join(" "));
            let payload: Value = serde_json::from_str(&json_str).map_err(|_| {
                err_json(
                    "invalid JSON payload",
                    raw,
                    "srv records submit <board_id> <json>",
                    "srv records submit <board_id> '{\"k\":1}'",
                )
            })?;
            let writer = joined_flag(rest, "--writer");
            pool.srv_validate(board_id, &payload)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records submit", "srv records submit <board_id> <json>"))?;
            let seq = pool
                .srv_record_insert(board_id, &payload, writer.as_deref(), has_flag(rest, "--upsert"))
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records submit", "srv records submit <board_id> <json>"))?;
            zw_recipe::run_recipes_for_record(&pool, board_id, "created", seq, &payload, writer.as_deref()).await;
            Ok(json!({ "status": "ok", "seq": seq }))
        }
        "bulk" => {
            if rest.is_empty() {
                return Err(err_json(
                    "missing '<json array>' payload",
                    raw,
                    "srv records bulk <board_id> '<json array>' [--writer <id>] [--upsert]",
                    "srv records bulk <board_id> '[{\"k\":1},{\"k\":2}]'",
                ));
            }
            let json_tokens: Vec<_> = rest
                .iter()
                .take_while(|t| !t.starts_with("--"))
                .map(|v| v.as_str())
                .collect();
            let json_str = unquote(&json_tokens.join(" "));
            let parsed: Value = serde_json::from_str(&json_str).map_err(|_| {
                err_json(
                    "invalid JSON payload",
                    raw,
                    "srv records bulk <board_id> '<json array>'",
                    "srv records bulk <board_id> '[{\"k\":1},{\"k\":2}]'",
                )
            })?;
            let records: Vec<Value> = match parsed {
                Value::Array(a) => a,
                _ => {
                    return Err(err_json(
                        "expected a JSON array of record objects",
                        raw,
                        "srv records bulk <board_id> '<json array>'",
                        "srv records bulk <board_id> '[{\"k\":1},{\"k\":2}]'",
                    ))
                }
            };
            let writer = joined_flag(rest, "--writer");
            let seqs = pool
                .srv_record_bulk_insert(&board_id, &records, writer.as_deref(), has_flag(rest, "--upsert"))
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records bulk", "srv records bulk <board_id> '<json array>'"))?;
            for (record, seq) in records.iter().zip(&seqs) {
                zw_recipe::run_recipes_for_record(&pool, board_id, "created", *seq, record, writer.as_deref()).await;
            }
            Ok(json!({ "status": "ok", "inserted": seqs.len(), "seqs": seqs }))
        }
        "list" => {
            let limit: i64 = val_after(rest, "--limit")
                .and_then(|v| v.parse().ok())
                .filter(|l| *l > 0)
                .unwrap_or(50);
            let offset: i64 = val_after(rest, "--offset")
                .and_then(|v| v.parse().ok())
                .filter(|o| *o >= 0)
                .unwrap_or(0);
            let records = pool
                .srv_record_list(board_id, limit, None, offset)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records list", "srv records list <board_id>"))?;
            Ok(json!({ "status": "ok", "records": records }))
        }
        "get" => {
            let seq: i64 = rest
                .iter()
                .position(|t| t.as_str() == "--seq")
                .and_then(|i| rest.get(i + 1))
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| err_json("missing --seq N", raw, "srv records get <board_id> --seq N", "srv records get <board_id> --seq 1"))?;
            let record = pool
                .srv_record_get(board_id, seq)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records get", "srv records get <board_id> --seq N"))?;
            Ok(json!({ "status": "ok", "record": record }))
        }
        "update" => {
            let seq: i64 = rest
                .first()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| err_json("missing <seq>", raw, "srv records update <board_id> <seq> <json>", "srv records update <board_id> 1 '{\"k\":1}'"))?;
            let json_tokens: Vec<_> = rest[1..]
                .iter()
                .take_while(|t| !t.starts_with("--"))
                .map(|v| v.as_str())
                .collect();
            let json_str = unquote(&json_tokens.join(" "));
            let payload: Value = serde_json::from_str(&json_str).map_err(|_| {
                err_json(
                    "invalid JSON payload",
                    raw,
                    "srv records update <board_id> <seq> <json>",
                    "srv records update <board_id> 1 '{\"k\":1}'",
                )
            })?;
            let writer = joined_flag(rest, "--writer");
            pool.srv_record_set(board_id, seq, &payload, writer.as_deref())
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records update", "srv records update <board_id> <seq> <json>"))?;
            zw_recipe::run_recipes_for_record(&pool, board_id, "updated", seq, &payload, writer.as_deref()).await;
            Ok(json!({ "status": "ok", "seq": seq }))
        }
        "patch" => {
            let seq: i64 = rest
                .first()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| err_json("missing <seq>", raw, "srv records patch <board_id> <seq> <ops>", "srv records patch <board_id> 1 '{\"$inc\":{\"votes\":1}}'"))?;
            let ops_str = unquote(&rest[1..].iter().map(|v| v.as_str()).collect::<Vec<_>>().join(" "));
            let ops: Value = serde_json::from_str(&ops_str).map_err(|_| {
                err_json(
                    "invalid JSON ops",
                    raw,
                    "srv records patch <board_id> <seq> <ops>",
                    "srv records patch <board_id> 1 '{\"$inc\":{\"votes\":1}}'",
                )
            })?;
            let merged = pool
                .srv_record_patch(board_id, seq, &ops)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records patch", "srv records patch <board_id> <seq> <ops>"))?;
            zw_recipe::run_recipes_for_record(&pool, board_id, "updated", seq, &merged, None).await;
            Ok(json!({ "status": "ok", "seq": seq, "payload": merged }))
        }
        "query" => {
            if let Some(f) = joined_flag(rest, "--filter") {
                let parsed: Value = serde_json::from_str(&f).map_err(|_| {
                    err_json(
                        "invalid --filter json",
                        raw,
                        "srv records query <board_id> --filter <json>",
                        "srv records query <board_id> --filter '{\"$.price\":{\"gte\":500}}'",
                    )
                })?;
                let filter = parse_filter(&parsed)
                    .map_err(|e| err_json(&e.to_string(), raw, "srv records query", "srv records query <board_id> --filter <json>"))?;
                let limit: i64 = val_after(rest, "--limit")
                    .and_then(|v| v.parse().ok())
                    .filter(|l| *l > 0)
                    .unwrap_or(50);
                let offset: i64 = val_after(rest, "--offset")
                    .and_then(|v| v.parse().ok())
                    .filter(|o| *o >= 0)
                    .unwrap_or(0);
                if let Some((q, others)) = split_search(&filter.conds) {
                    let records = pool
                        .srv_record_search(&board_id, &q, &others, limit, offset, false)
                        .await
                        .map_err(|e| err_json(&e.to_string(), raw, "srv records query", "srv records query <board_id> --filter <json>"))?;
                    return Ok(json!({ "status": "ok", "records": records }));
                }
                let order_owned = orders_from(rest);
                let orders: Vec<(&str, bool)> = order_owned.iter().map(|(p, d)| (p.as_str(), *d)).collect();
                let records = pool
                    .srv_record_filter(&board_id, &filter.conds, &orders, limit, offset)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv records query", "srv records query <board_id> --filter <json>"))?;
                return Ok(json!({ "status": "ok", "records": records }));
            }
            let path = rest
                .first()
                .map(|p| unquote(p))
                .filter(|p| !p.is_empty())
                .ok_or_else(|| {
                    err_json(
                        "missing json path",
                        raw,
                        "srv records query <board_id> <path>",
                        "srv records query <board_id> '$.name'",
                    )
                })?;
            let eq = joined_flag(rest, "--eq");
            let records = pool
                .srv_record_query(&board_id, &path, eq.as_deref())
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records query", "srv records query <board_id> <path>"))?;
            Ok(json!({ "status": "ok", "records": records }))
        }
        "search" => {
            if rest.is_empty() {
                return Err(err_json(
                    "missing <query>",
                    raw,
                    "srv records search <board_id> '<query>'",
                    "srv records search <board_id> 'laptop'",
                ));
            }
let limit: i64 = val_after(rest, "--limit")
                .and_then(|v| v.parse().ok())
                .filter(|l| *l > 0)
                .unwrap_or(50);
            let offset: i64 = val_after(rest, "--offset")
                .and_then(|v| v.parse().ok())
                .filter(|o| *o >= 0)
                .unwrap_or(0);
            let query = rest
                .iter()
                .take_while(|t| !t.starts_with("--"))
                .map(|t| unquote(t))
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();
            if query.is_empty() {
                return Err(err_json("empty <query>", raw, "srv records search <board_id> '<query>'", "srv records search <board_id> 'laptop'"));
            }
            let records = pool
                .srv_record_search(&board_id, &query, &[], limit, offset, has_flag(rest, "--hl"))
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records search", "srv records search <board_id> '<query>'"))?;
            Ok(json!({ "status": "ok", "records": records, "searched": query }))
        }
        "aggregate" => {
            let agg = val_after(rest, "--op")
                .and_then(|o| Agg::parse(&o).ok())
                .ok_or_else(|| err_json("missing --op sum|avg|min|max|count", raw, "srv records aggregate", "srv records aggregate <board_id> --op sum --field '$.price'"))?;
            let field = joined_flag(rest, "--field");
            let group_by = joined_flag(rest, "--group_by");
            let conds = match joined_flag(rest, "--filter") {
                Some(f) => {
                    let parsed: Value = serde_json::from_str(&f).map_err(|_| {
                        err_json("invalid --filter json", raw, "srv records aggregate", "srv records aggregate <board_id> --op sum")
                    })?;
                    parse_filter(&parsed)
                        .map_err(|e| err_json(&e.to_string(), raw, "srv records aggregate", "srv records aggregate <board_id> --op sum"))?
                }
                None => SrvFilter::default(),
            };
            let rows = pool
                .srv_record_aggregate(&board_id, &conds.conds, agg, field.as_deref(), group_by.as_deref())
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records aggregate", "srv records aggregate <board_id> --op sum"))?;
            Ok(json!({ "status": "ok", "aggregate": rows }))
        }
        "count" => {
            let count = pool
                .srv_record_count(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records count", "srv records count <board_id>"))?;
            Ok(json!({ "status": "ok", "count": count }))
        }
        "delete" => {
            if let Some(f) = joined_flag(rest, "--filter") {
                let parsed: Value = serde_json::from_str(&f).map_err(|_| {
                    err_json(
                        "invalid --filter json",
                        raw,
                        "srv records delete <board_id> --filter <json>",
                        "srv records delete <board_id> --filter '{\"$.status\":{\"eq\":\"archived\"}}'",
                    )
                })?;
                let filter = parse_filter(&parsed)
                    .map_err(|e| err_json(&e.to_string(), raw, "srv records delete --filter", "srv records delete <board_id> --filter <json>"))?;
                let deleted = pool
                    .srv_record_delete_filter(&board_id, &filter.conds)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "srv records delete --filter", "srv records delete <board_id> --filter <json>"))?;
                return Ok(json!({ "status": "ok", "deleted": deleted }));
            }
            let seq: Option<i64> = if has_flag(rest, "--all") {
                None
            } else if let Some(v) = val_after(rest, "--seq") {
                Some(v.parse().map_err(|_| {
                    err_json("invalid --seq value", raw, "srv records delete --seq N", "srv records delete <board_id> --all")
                })?)
            } else {
                return Err(err_json(
                    "expected '--seq <n>', '--all' or '--filter <json>'",
                    raw,
                    "srv records delete <board_id> [--seq N | --all | --filter <json>]",
                    "srv records delete <board_id> --all",
                ));
            };
            pool.srv_record_delete(board_id, seq)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv records delete", "srv records delete <board_id>"))?;
            Ok(json!({ "status": "ok", "deleted": true }))
        }
        other => Err(err_json(
            &format!("unknown records verb '{other}'"),
            raw,
            "srv records <verb>",
            USAGE,
        )),
    }
}

async fn cmd_audit_records(db: &DbPool, board_id: &str, owner: &str, rest: &[&String], raw: &str) -> CmdResult {
    let since = val_after(rest, "--since").map(|s| s.to_string());
    let limit: i64 = val_after(rest, "--limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(100)
        .clamp(1, 1000);
    let rows = db
        .srv_audit_list(board_id, owner, since.as_deref(), limit)
        .await
        .map_err(|e| err_json(&e.to_string(), raw, "srv records audit", "srv records audit <board_id>"))?;
    Ok(json!({ "status": "ok", "audit": rows }))
}

async fn cmd_recipes(store: &SessionStore, sid: &str, verb: &str, args: &[&String], raw: &str) -> CmdResult {
    if verb == "--help" {
        return Ok(json!({ "status": "ok", "help": RECIPES_USAGE }));
    }
    let board_id = args
        .first()
        .ok_or_else(|| err_json("missing <board_id>", raw, "srv recipes <verb>", "srv recipes list <board_id>"))?;
    let read = verb == "list";
    let (_app, pool) = require_board(store, board_id, sid, raw, read).await?;
    let rest = &args[1..];
    match verb {
        "list" => {
            let recipes = pool
                .srv_recipe_list(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv recipes list", "srv recipes list <board_id>"))?;
            Ok(json!({ "status": "ok", "recipes": recipes }))
        }
        "add" => {
            let name = val_after(rest, "--name").ok_or_else(|| {
                err_json("missing --name <n>", raw, "srv recipes add", "srv recipes add <board_id> --name sync --when 'record.created'")
            })?;
            let when = val_after(rest, "--when").ok_or_else(|| {
                err_json("missing --when <event>", raw, "srv recipes add", "srv recipes add <board_id> --when 'record.created'")
            })?;
            let match_json = val_after(rest, "--match");
            let dedup = val_after(rest, "--dedup");
            let actions = val_after(rest, "--actions").unwrap_or_else(|| "[]".to_string());
            serde_json::from_str::<Value>(&actions)
                .map_err(|e| err_json(&format!("invalid --actions json: {e}"), raw, "srv recipes add", "srv recipes add --actions '[{\"$set\":{\"status\":\"paid\"}}]'"))?;
            let id = pool
                .srv_recipe_add(board_id, &name, &when, match_json.as_deref(), true, dedup.as_deref(), &actions)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv recipes add", "srv recipes add <board_id> --name <n> --when '<event>'"))?;
            Ok(json!({ "status": "ok", "recipe_id": id }))
        }
        "enable" => {
            let name = rest
                .first()
                .ok_or_else(|| err_json("missing <name>", raw, "srv recipes enable", "srv recipes enable <board_id> sync on"))?;
            let on = match rest.get(1).map(|v| v.as_str()) {
                Some("on") => true,
                Some("off") => false,
                _ => {
                    return Err(err_json(
                        "expected on or off",
                        raw,
                        "srv recipes enable <board_id> <name> on|off",
                        "srv recipes enable <board_id> sync on",
                    ))
                }
            };
            let enabled = pool
                .srv_recipe_set_enabled(board_id, name, on)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv recipes enable", "srv recipes enable <board_id> <name> on"))?;
            Ok(json!({ "status": "ok", "enabled": enabled }))
        }
        "runs" => {
            let recipe = match rest.first() {
                Some(n) if !n.starts_with("--") => Some(n.clone()),
                _ => None,
            };
            let limit: i64 = joined_flag(rest, "--limit")
                .and_then(|n| n.parse().ok())
                .unwrap_or(50);
            let runs = pool
                .srv_recipe_runs_log(board_id, recipe.map(|x| x.as_str()), limit)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv recipes runs", "srv recipes <board_id> runs [<name>] [--limit N]"))?;
            Ok(json!({ "status": "ok", "runs": runs }))
        }
        "remove" => {
            let name = rest
                .first()
                .ok_or_else(|| err_json("missing <name>", raw, "srv recipes remove", "srv recipes remove <board_id> sync"))?;
            let removed = pool
                .srv_recipe_remove(board_id, name)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv recipes remove", "srv recipes remove <board_id> <name>"))?;
            Ok(json!({ "status": "ok", "removed": removed }))
        }
        other => Err(err_json(
            &format!("unknown recipes verb '{other}'"),
            raw,
            "srv recipes <verb>",
            RECIPES_USAGE,
        )),
    }
}

async fn cmd_secrets(store: &SessionStore, sid: &str, verb: &str, args: &[&String], raw: &str) -> CmdResult {
    if verb == "--help" {
        return Ok(json!({ "status": "ok", "help": SECRETS_USAGE }));
    }
    let board_id = args
        .first()
        .ok_or_else(|| err_json("missing <board_id>", raw, "srv secrets <verb>", "srv secrets list <board_id>"))?;
    let (_app, pool) = require_board(store, board_id, sid, raw, false).await?;
    let rest = &args[1..];
    match verb {
        "set" => {
            let name = rest
                .first()
                .ok_or_else(|| err_json("missing <NAME>", raw, "srv secrets set", "srv secrets set <board_id> STRIPE sk_test_..."))?;
            let value = rest
                .get(1)
                .ok_or_else(|| err_json("missing <value>", raw, "srv secrets set", "srv secrets set <board_id> STRIPE sk_test_..."))?;
            pool.srv_secret_set(board_id, name, value)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv secrets set", "srv secrets set <board_id> <NAME> <value>"))?;
            Ok(json!({ "status": "ok", "set": name.to_uppercase() }))
        }
        "list" => {
            let secrets = pool
                .srv_secret_list(board_id)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv secrets list", "srv secrets list <board_id>"))?;
            Ok(json!({ "status": "ok", "secrets": secrets }))
        }
        "remove" => {
            let name = rest
                .first()
                .ok_or_else(|| err_json("missing <NAME>", raw, "srv secrets remove", "srv secrets remove <board_id> STRIPE"))?;
            let removed = pool
                .srv_secret_remove(board_id, name)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "srv secrets remove", "srv secrets remove <board_id> <NAME>"))?;
            Ok(json!({ "status": "ok", "removed": removed }))
        }
        other => Err(err_json(
            &format!("unknown secrets verb '{other}'"),
            raw,
            "srv secrets <verb>",
            SECRETS_USAGE,
        )),
    }
}

async fn cmd_files(store: &SessionStore, dir: &Path, sid: &str, verb: &str, args: &[&String], raw: &str) -> CmdResult {
    if verb == "--help" {
        return Ok(json!({ "status": "ok", "help": FILES_USAGE }));
    }
    if args.first().map(|s| s.as_str()) == Some("--help") {
        if let Some(h) = crate::srv_help::verb_help("files", verb) {
            return Ok(json!({ "status": "ok", "help": h }));
        }
        return Err(err_json(
            &format!("no detailed help for 'srv files {verb}'"),
            raw,
            "srv files <verb>",
            FILES_USAGE,
        ));
    }
    let board_id = args
        .first()
        .ok_or_else(|| err_json("missing <board_id>", raw, "srv files <verb>", "srv files list"))?;
    require_board(store, board_id, sid, raw, true).await?;
    let rest = &args[1..];
    match verb {
        "list" => {
            let files = zw_db::serverless::srv_list_files(dir, sid, board_id)
                .map_err(|e| err_json(&e.to_string(), raw, "srv files list", "srv files list <board_id>"))?;
            let files: Vec<_> = files
                .into_iter()
                .map(|(file, size)| json!({ "file": file, "size": size }))
                .collect();
            Ok(json!({ "status": "ok", "files": files }))
        }
        "get" => {
            let file_id = rest.first().ok_or_else(|| {
                err_json(
                    "missing <file_id>",
                    raw,
                    "srv files get <board_id> <file_id> --out <path>",
                    "srv files get <board_id> <file_id> --out /tmp/x",
                )
            })?;
            let out = val_after(rest, "--out").ok_or_else(|| {
                err_json(
                    "missing --out <path>",
                    raw,
                    "srv files get <board_id> <file_id> --out <path>",
                    "srv files get <board_id> <file_id> --out /tmp/x",
                )
            })?;
            let src = zw_db::serverless::srv_board_file_path(dir, sid, board_id, file_id);
            if !src.is_file() {
                return Err(err_json(
                    "file does not exist",
                    raw,
                    "srv files get <board_id> <file_id>",
                    "srv files list <board_id>",
                ));
            }
            if let Some(parent) = Path::new(&out).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| err_json(&e.to_string(), raw, "srv files get --out", "srv files get --out /tmp/x"))?;
                }
            }
            std::fs::copy(&src, &out)
                .map_err(|e| err_json(&e.to_string(), raw, "srv files get", "srv files get <board_id> <file_id>"))?;
            Ok(json!({ "status": "ok", "saved": out }))
        }
        other => Err(err_json(
            &format!("unknown files verb '{other}'"),
            raw,
            "srv files <verb>",
            USAGE,
        )),
    }
}
pub fn srv_usage() -> &'static str {
    USAGE
}

pub async fn execute_srv(
    store: &SessionStore,
    dir: &Path,
    sid: &str,
    rest: &[&String],
    raw: &str,
) -> Result<serde_json::Value, serde_json::Value> {
    let group = rest
        .first()
        .ok_or_else(|| err_json("missing group: apps|records|files", raw, "srv <group> <verb>", "srv apps list"))?;
    let verb = rest
        .get(1)
        .ok_or_else(|| err_json("missing verb", raw, "srv <group> <verb>", "srv apps list"))?;
    let args = &rest[2..];
    match group.as_str() {
        "apps" => cmd_apps(store, sid, verb, args, raw).await,
        "records" => cmd_records(store, sid, verb, args, raw).await,
        "recipes" => cmd_recipes(store, sid, verb, args, raw).await,
        "secrets" => cmd_secrets(store, sid, verb, args, raw).await,
        "files" => cmd_files(store, dir, sid, verb, args, raw).await,
        other => Err(err_json(&format!("unknown group '{other}'"), raw, "srv <group> <verb>", USAGE)),
    }
}