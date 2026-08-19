use async_trait::async_trait;
use serde_json::{json, Value};
use zw_core::{McpToolProvider, ToolOutput};
use zw_whiteboard::{BoardState, WhiteboardHandle};

const ACTIONS: &[&str] = &["about", "list", "open", "close", "add", "delete", "move", "set-public", "show", "srv"];

const PREVIEW_CHARS: usize = 600;

const GLOBAL_USAGE: &str =
    "wb --session <session_id> <action> [args]\n\
     for the full per-command help, run: <action> --help\n\
     actions (short):\n\
       about             what the whiteboard is + how to build serverless apps\n\
       list              list all pages, active page and lock state\n\
       open <page_no>    set the active page and load it\n\
       close             unlock the active page\n\
       add [--title \"t\"] append a new page\n\
       delete <page_no>  remove a page and renumber the rest\n\
       move <from> to <to>  reorder pages\n\
       set-public <page_no> on|off  publish a /w/<slug> snapshot\n\
       show <page_no>    preview a page (first ~600 chars)\n\
       srv apps|records|files <...>  manage serverless apps, data and files (see: srv --help)\n\\n\
       --help | <action> --help\n";

pub struct ManageWhiteboardProvider {
    handle: WhiteboardHandle,
}

impl ManageWhiteboardProvider {
    pub fn new(handle: WhiteboardHandle) -> Self {
        Self { handle }
    }

    async fn state_json(&self, sid: &str) -> Result<Value, Value> {
        let st = self
            .handle
            .list(sid)
            .await
            .map_err(|e| err_json(&format!("{e}"), sid, "list", &format!("wb --session {sid} list")))?;
        Ok(st.to_value())
    }

    async fn execute(&self, tokens: &[String], raw: &str) -> Result<Value, Value> {
        let mut session: Option<String> = None;
        let mut body: Vec<String> = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i] == "--session" {
                if i + 1 < tokens.len() {
                    session = Some(str_val_index(tokens, i + 1));
                    i += 2;
                } else {
                    return Err(err_json(
                        "missing value for --session",
                        raw,
                        "list",
                        "--session <session_id>",
                    ));
                }
            } else {
                body.push(tokens[i].clone());
                i += 1;
            }
        }

        if body.is_empty() {
            return Err(err_json("no action given", raw, "list", GLOBAL_USAGE));
        }

        let action = body[0].clone();
        let rest: Vec<&String> = body[1..].iter().collect();

        if action == "--help" {
            return Ok(json!({ "status": "ok", "help": GLOBAL_USAGE }));
        }
        if rest.first().map(|s| s.as_str()) == Some("--help") {
            return Ok(json!({ "status": "ok", "help": action_help(&action) }));
        }

        let sid = match session {
            Some(s) if !s.is_empty() => s,
            _ => {
                return Err(err_json(
                    "--session <session_id> is required on every command",
                    raw,
                    "list",
                    "--session <session_id>",
                ));
            }
        };

        match action.as_str() {
            "list" => Ok(ok_with(raw, &self.state_json(&sid).await?)),
            "open" => {
                let n = req_i64(&rest, "open <page_no>")?;
                self.handle
                    .open(&sid, n)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "open <page_no>", &close_hint(&sid)))?;
                Ok(ok_with(raw, &self.state_json(&sid).await?))
            }
            "close" => {
                self.handle
                    .close(&sid)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "close", "close the active page"))?;
                Ok(ok_with(raw, &self.state_json(&sid).await?))
            }
            "add" => {
                let title = title_flag(&rest).unwrap_or_else(|| "Untitled".to_string());
                let n = self
                    .handle
                    .add(&sid, &title)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "add [--title <title>]", "add a new page"))?;
                let mut st = self.state_json(&sid).await?;
                st["message"] = json!(format!("created page {n} ({title})"));
                Ok(ok_with(raw, &st))
            }
            "delete" => {
                let n = req_i64(&rest, "delete <page_no>")?;
                self.handle
                    .delete(&sid, n)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "delete <page_no>", &close_hint(&sid)))?;
                Ok(ok_with(raw, &self.state_json(&sid).await?))
            }
            "move" => {
                let (from, to) = parse_move(&rest, raw)?;
                self.handle
                    .move_pages(&sid, from, to)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "move <from> to <to>", &close_hint(&sid)))?;
                Ok(ok_with(raw, &self.state_json(&sid).await?))
            }
            "set-public" => {
                let n = req_i64(&rest, "set-public <page_no> on|off")?;
                let mode = rest.get(1).ok_or_else(|| err_json("missing on|off", raw, "set-public <page_no> on|off", "set-public <page_no> on"))?;
                let pub_ = match mode.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err(err_json("expected on or off", raw, "set-public <page_no> on|off", "set-public <page_no> on")),
                };
                let slug = self
                    .handle
                    .set_public(&sid, n, pub_)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "set-public <page_no> on|off", "set-public <page_no> on"))?;
                let mut st = self.state_json(&sid).await?;
                let note = match slug {
                    Some(s) => format!("page {n} is public: /w/{s}"),
                    None => format!("page {n} is no longer public; URL removed"),
                };
                st["message"] = json!(note);
                Ok(ok_with(raw, &st))
            }
            "show" => {
                let n = req_i64(&rest, "show <page_no>")?;
                let st = self.state_json(&sid).await?;
                let pages = st["pages"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let total = pages.len();
                let page = pages
                    .iter()
                    .find(|p| p["page_no"].as_i64() == Some(n))
                    .cloned()
                    .ok_or_else(|| err_json(&format!("page {n} not found"), raw, "show <page_no>", &format!("wb --session {sid} list")))?;
                let len = page["body_md"].as_str().map(|b| b.chars().count()).unwrap_or(0);
                Ok(json!({
                    "status": "ok",
                    "page_no": n,
                    "total_pages": total,
                    "title": page["title"].as_str().unwrap_or(""),
                    "preview": preview_md(page["body_md"].as_str().unwrap_or(""), PREVIEW_CHARS),
                    "body_len_chars": len,
                    "note": "preview only — read the full body from the session's .wb file with your file tool if needed",
                }))
            }
            "about" => Ok(json!({ "status": "ok", "about": ABOUT_TEXT })),
            "srv" => {
                let store = self.handle.db().ok_or_else(|| err_json(
                    "serverless store is unavailable (no whiteboard db configured)",
                    raw, "srv <action>", "srv apps list",
                ))?;
                let dir = self.handle.dir().clone();
                match crate::srv::execute_srv(&store, &dir, &sid, &rest, raw).await {
                    Ok(v) => Ok(v),
                    Err(e) => Err(e),
                }
            }
            other => Err(unknown_action(raw, other)),
        }
    }
}

trait StateValue {
    fn to_value(&self) -> Value;
}

impl StateValue for BoardState {
    fn to_value(&self) -> Value {
        json!({
            "page_no": self.pages.iter().map(|p| p.page_no).collect::<Vec<_>>(),
            "active_page": self.active_page,
            "locked": self.locked,
            "pages": self.pages,
        })
    }
}

fn ok_with(_raw: &str, st: &Value) -> Value {
    json!({ "status": "ok", "state": st })
}

fn err_json(error: &str, command: &str, usage: &str, hint: &str) -> Value {
    json!({
        "status": "error",
        "error": error,
        "command": command,
        "hint": hint,
        "usage": usage,
    })
}

fn unknown_action(raw: &str, action: &str) -> Value {
    let mut best: Option<(&str, usize)> = None;
    for a in ACTIONS {
        if a.len() >= 2 && (a.starts_with(action) || action.starts_with(a)) {
            let p = common_prefix(a, action);
            if best.map(|(_, bp)| p > bp).unwrap_or(true) {
                best = Some((a, p));
            }
        }
    }
    let mut j = err_json(
        &format!("unknown action '{action}'"),
        raw,
        GLOBAL_USAGE,
        "use --help to list actions",
    );
    if let Some((d, _)) = best {
        j["did_you_mean"] = json!(d);
        j["hint"] = json!(format!("try: wb --session <sid> {d}"));
    }
    j
}

fn action_help(action: &str) -> &'static str {
    match action {
        "about" => "wb --session <sid> about\n\nprints the full capability summary: whiteboard features (markdown, mermaid, HTML/CSS/JS, pins, multi-page) and the complete serverless-app system (auth/keys, record CRUD + HTTP endpoints, outbound API calls, webhooks, TTL, joins, audit, rate limits, realtime). Run it first.\n",
        "list" => "wb --session <sid> list\n\nlists all pages, active_page and locked state\n",
        "open" => "wb --session <sid> open <page_no>\n\nsets the agent's active page, locks it and loads its body into the session's .wb file\n",
        "close" => "wb --session <sid> close\n\nunlocks the active page so users can edit it\n",
        "add" => "wb --session <sid> add [--title \"<title>\"]\n\nappends a new page; returns its page_no\n",
        "delete" => "wb --session <sid> delete <page_no>\n\nremoves a page and renumbers the rest (fails if it is the locked active page)\n",
        "move" => "wb --session <sid> move <from> to <to>\n\nreorders pages by one-based position\n",
        "set-public" => "wb --session <sid> set-public <page_no> on|off\n\ntakes a frozen snapshot and publishes /w/<slug> (on) or removes it (off)\n",
        "show" => "wb --session <sid> show <page_no>\n\nprints page_no, total_pages, title and a short preview (first ~600 chars) — not the full body; read the .wb file for the rest\n",
        "srv" => "wb --session <sid> srv <group> <verb> [args]\n\nthe serverless-app CLI: build and manage stateless apps + their data INSIDE a whiteboard, using only this MCP tool (no backend code, no shell tool).\ngroups:\n  apps   create --title \"t\" [--json <schema>] [--public-reads on|off] [--unique '<$.path>'] | list | show <id> | schema <id> | update <id> [--title] [--json|--clear-schema] [--public-reads] [--unique|--clear-unique] | delete <id> | keys <id> list | issue --role writer|reader|admin [--writer <id>] | revoke --bucket <bucket> | hooks <id> register <url> [--secret <s>] | list | remove <url> | deliveries [--url <u>] | replay <url> | rate <id> '{\"submit\":60}' | rate <id> --clear | ttl <id> [<seconds>] [--ttl-until '<$.path>'] [--clear] | link <id> --on '$.k' --to <parent> --via '$.k' | link <id> --clear | audit <id> on|off\n  records submit <id> <json> [--writer][--upsert] | list <id> [--limit N] | get <id> --seq N | update <id> <seq> <json> | patch <id> <seq> '{ops}' ($set/$inc/$dec/$mul/$unset) | query <id> --filter '<json>' [--order][--limit] | search <id> '<query>' [--limit N] (full-text BM25) | count <id> | aggregate <id> --op sum|avg|min|max|count --field '<$.path>' [--group_by][--filter] | delete <id> [--seq N|--all] | audit <id> [--since <iso-ts>] [--limit N] | call <id> <url> [--method GET|POST|PUT|PATCH|DELETE] [--header 'Name: Value']* [--body '<json>'] [--timeout N]\n  files   list <id> | get <id> <file_id> --out <path>\nHelp: srv --help (overview) | srv <group> --help (per-group) | srv <group> <verb> --help (per-verb, most detail)\n",
        _ => GLOBAL_USAGE,
    }
}

const ABOUT_TEXT: &str =
    "The whiteboard is the live document the user sees. You manage it fully through this tool (wb ...) — never via the shell tool.\n\
     \n\
     Capabilities:\n\
     - Full Markdown: headings, lists, tables, quotes, code blocks, links, images, task lists.\n\
     - Mermaid diagrams (flowchart, sequence, gantt, class, er, etc.) rendered live.\n\
     - Inline HTML + full CSS (styled widgets, cards, layouts).\n\
     - Embedded <script> JavaScript that runs in the user's page (interactive logic, fetch calls).\n\
     - Pins: attach files (images/docs) to a page.\n\
     - Multi-page boards: wb --session <sid> add/open/close/delete/move, set-public for /w/<slug>.\n\
     \n\
     SERVERLESS APPS (no backend code — the API and DB schema are DATA, managed by this CLI):\n\
     Everything below is also visible in 'wb --session <sid> srv --help' (and 'srv apps --help' / 'srv records --help').\n\
     \n\
     1. Build the app: put HTML/CSS/JS into a page, then create the backend as data:\n\
        wb --session <sid> srv apps create --title \"My App\" --json '{\"type\":\"object\",\"properties\":{...}}' [--public-reads on] [--unique '<$.path>']\n\
        -> returns a secret board_id + stores your JSON schema (records are validated against it).\n\
        Unique key (--unique '<$.path>'): extra submit with the same value is rejected unless ?upsert=1, which replaces keeping its seq.\n\
     \n\
     2. Auth model: every /api/srv/<board_id>/... request is authorized with 'Authorization: Bearer <token>'.\n\
        - The board_id itself is the admin token (create/configure everything).\n\
        - Issue scoped keys (reader/writer/admin): POST /api/srv/<board_id>/keys  or  srv apps keys <board_id> issue --role writer\n\
          reader can read (incl. private boards), writer can submit/patch, admin manages config. Revoke via srv apps keys revoke --bucket <bucket>.\n\
        - Rate limiting: per-action windows (submit/upload/search/read) + daily cap; admin configures via srv apps rate, enforced with 429 + Retry-After.\n\
     \n\
     3. Record CRUD over HTTP (validated vs schema):\n\
        write:  POST   /api/srv/<board_id>/submit            (JSON; header X-Writer, or ?upsert=1)\n\
                PUT    /api/srv/<board_id>/records/<seq>     (full replace)\n\
                PATCH  /api/srv/<board_id>/records/<seq>     ({\"$inc\":{\"votes\":1},\"$set\":{\"a\":1},\"$unset\":[...]})\n\
                DELETE /api/srv/<board_id>/records/<seq>\n\
        read (public-reads or any key):\n\
                GET /api/srv/<board_id>/records?limit=&offset=   all records\n\
                GET /api/srv/<board_id>/records?q=<query>        full-text search (BM25-ranked, whole payload)\n\
                GET /api/srv/<board_id>/records?filter=<json>    filter e.g. {\"price\":{\"gte\":500}} (ops: eq neq gt gte lt lte contains not_contains in search)\n\
                GET /api/srv/<board_id>/records?order=<path>&dir=asc&limit=10\n\
                GET /api/srv/<board_id>/aggregate?op=sum&field=<path>&group_by=<path>&filter={...}\n\
                GET /api/srv/<board_id>/record?seq=N\n\
        files:\n\
                POST /api/srv/<board_id>/upload   (multipart or raw body; up to 25MB) -> {\"file\":\"files/<id>.bin\"}\n\
                GET  /api/srv/<board_id>/file?file=files/<id>.bin   (downloads; stored per-session under sessions/<owner>/apps/<board>/files)\n\
        realtime:\n\
                GET  /api/srv/<board_id>/ws   (WebSocket; Bearer token; streams record.created/updated/deleted, catch-up after connect)\n\
     \n\
     4. Call external APIs and read the response (connect to API providers/sensors):\n\
        POST /api/srv/<board_id>/call   body {\"url\":\"https://...\",\"method\":\"GET|POST|PUT|PATCH|DELETE\",\"headers\":{...},\"body\":{...},\"timeout_ms\":N}\n\
        -> {status, headers, body, duration_ms}. Private/loopback targets are blocked (SSRF guard).\n\
        Same via CLI:  srv records call <board_id> <url> [--method ...] [--header 'K: V']* [--body '<json>'] [--timeout N]\n\
     \n\
     5. Integrations (all admin, via CLI):\n\
        webhooks: srv apps hooks <board_id> register <url> [--secret <s>]   -> record events POSTed to the url (HMAC X-WAOS-Signature); inspect/replay via deliveries|replay.\n\
        ttl:      srv apps ttl <board_id> [<seconds>] [--ttl-until '<$.path>']   -> records auto-expire by age and/or a timestamp field.\n\
        joins:    srv apps link <board_id> --on '$.k' --to <parent_board> --via '$.k'   -> child records embed the parent's payload (GET /join).\n\
        audit:    srv apps audit <board_id> on|off   -> immutable log of created/updated/deleted events (who/when/payload-hash); read via srv records audit.\n\
     \n\
     6. You manage everything here:  wb --session <sid> srv apps|records|files ...  (run 'srv --help' for the full command list).\n\
     \n\
     Start: wb --session <sid> about | wb --session <sid> list | wb --session <sid> srv --help";

fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

fn close_hint(sid: &str) -> String {
    format!("if locked, close it first: wb --session {sid} close")
}

fn req_i64(rest: &[&String], usage: &str) -> Result<i64, Value> {
    let tok = rest
        .first()
        .ok_or_else(|| err_json("missing argument", "wb --session <sid>", usage, usage))?;
    tok.parse().map_err(|_| err_json(&format!("'{tok}' is not an integer"), "tok", usage, usage))
}

fn parse_move(rest: &[&String], raw: &str) -> Result<(i64, i64), Value> {
    let from = req_i64(&rest, "move <from> to <to>")?;
    let rest2 = &rest[1..];
    if rest2.first().map(|s| s.as_str()) != Some("to") {
        return Err(err_json("expected 'to' after <from>", raw, "move <from> to <to>", "move 3 to 1"));
    }
    let to = req_i64(&rest2[1..], "move <from> to <to>")?;
    Ok((from, to))
}

fn title_flag(rest: &[&String]) -> Option<String> {
    for (idx, tok) in rest.iter().enumerate() {
        if tok.as_str() == "--title" {
            return rest.get(idx + 1).map(|v| v.trim_matches('"').to_string());
        }
    }
    None
}

fn str_val_index(v: &[String], i: usize) -> String {
    v.get(i).cloned().unwrap_or_default()
}

fn strip_program_name(mut tokens: Vec<String>) -> Vec<String> {
    if tokens.first().map(|s| s.as_str()) == Some("wb") {
        tokens.remove(0);
    }
    tokens
}

fn preview_md(body: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    for line in body.lines() {
        if count >= limit {
            break;
        }
        let take = (limit - count).min(line.chars().count());
        out.push_str(&line.chars().take(take).collect::<String>());
        out.push('\n');
        count += take + 1;
    }
    out.trim().to_string()
}

#[async_trait]
impl McpToolProvider for ManageWhiteboardProvider {
    fn name(&self) -> &str {
        "manage_whiteboard"
    }

    fn description(&self) -> &str {
        "CLI tool for the session whiteboard and the serverless apps embedded in it. Run CLI commands here (never the shell tool), e.g. wb --session <sid> <action> [--help]. Required: --session <session_id>. Start with 'wb --session <sid> about' or '--help' for the command list."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "A CLI command to run inside this tool only (not the shell tool), e.g. 'wb --session web.x about' or 'wb --session web.x list'."
                }
            },
            "required": ["command"]
        })
    }

    async fn call(&self, args: Value) -> anyhow::Result<ToolOutput> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tokens: Vec<String> = command.split_whitespace().map(|s| s.to_string()).collect();
        let raw = command.clone();
        let tokens = strip_program_name(tokens);
        let result = match self.execute(&tokens, &raw).await {
            Ok(v) => v,
            Err(e) => e,
        };
        Ok(ToolOutput::new(result.to_string()))
    }
}