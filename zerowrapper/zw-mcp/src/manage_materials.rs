use async_trait::async_trait;
use serde_json::{json, Value};
use zw_core::{McpToolProvider, ToolOutput};
use zw_materials::TemplateStore;

use crate::materials_help::{ABOUT_TEXT, ACTIONS, GLOBAL_USAGE, action_help};

pub struct ManageMaterialsProvider {
    store: TemplateStore,
}

impl ManageMaterialsProvider {
    pub fn new(store: TemplateStore) -> Self {
        Self { store }
    }

    async fn execute(&self, tokens: &[String], raw: &str) -> Result<Value, Value> {
        let mut session: Option<String> = None;
        let mut body: Vec<String> = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i] == "--session" {
                if i + 1 < tokens.len() {
                    session = Some(tokens[i + 1].clone());
                    i += 2;
                } else {
                    return Err(err_json(
                        "missing value for --session",
                        raw,
                        "--session <session_id>",
                    ));
                }
            } else {
                body.push(tokens[i].clone());
                i += 1;
            }
        }

        if body.is_empty() {
            return Err(err_json("no action given", raw, GLOBAL_USAGE));
        }

        let action = body[0].clone();
        let rest: Vec<&String> = body[1..].iter().collect();

        if action == "--help" {
            return Ok(ok(json!({ "help": GLOBAL_USAGE })));
        }
        if rest.first().map(|s| s.as_str()) == Some("--help") {
            return Ok(ok(json!({ "help": action_help(&action) })));
        }

        let need_session = matches!(
            action.as_str(),
            "publish" | "embed" | "clone" | "commit" | "update"
        );
        let sid = if need_session {
            match session {
                Some(s) if !s.is_empty() => s,
                _ => {
                    return Err(err_json(
                        "--session <sid> is required on this command",
                        raw,
                        &format!("mat --session <sid> {action}"),
                    ));
                }
            }
        } else {
            session.unwrap_or_default()
        };

        match action.as_str() {
            "about" => Ok(ok(json!({ "about": ABOUT_TEXT }))),
            "list" => Ok(ok(self.store.list().await.map_err(|e| err_json(&e.to_string(), raw, "mat list"))?)),
            "show" => {
                let tpl = req_pos(&rest, raw, "show <template_id>")?;
                let ver = opt_ver(&rest, raw)?;
                Ok(ok(self.store.show(&tpl, ver).await.map_err(|e| err_json(&e.to_string(), raw, "mat show"))?))
            }
            "versions" => {
                let tpl = req_pos(&rest, raw, "versions <template_id>")?;
                Ok(ok(json!({ "template_id": tpl, "versions": self.store.versions(&tpl).await.map_err(|e| err_json(&e.to_string(), raw, "mat versions"))? })))
            }
            "create" => {
                let tpl = req_pos(&rest, raw, "create <template_id>")?;
                let name = flag(&rest, "--name").unwrap_or_else(|| tpl.clone());
                let kind = flag(&rest, "--kind").unwrap_or_else(|| "app".to_string());
                let desc = flag(&rest, "--desc").unwrap_or_default();
                let ver = self
                    .store
                    .ensure_template(&tpl, &name, &kind, &desc)
                    .await
                    .map_err(|e| err_json(&e.to_string(), raw, "mat create"))?;
                Ok(ok(json!({ "template_id": tpl, "version": ver, "name": name, "kind": kind, "description": desc })))
            }
            "publish" => {
                let board = req_flag(&rest, raw, "--board", "publish --board <board_id> --tpl <tpl>")?;
                let tpl = req_flag(&rest, raw, "--tpl", "publish --board <board_id> --tpl <tpl>")?;
                let name = flag(&rest, "--name").unwrap_or_else(|| tpl.clone());
                let kind = flag(&rest, "--kind").unwrap_or_else(|| "app".to_string());
                let desc = flag(&rest, "--desc").unwrap_or_default();
                let note = flag(&rest, "--note").unwrap_or_else(|| "initial publish".to_string());
                Ok(ok(self.store.publish(&sid, &board, &tpl, &name, &kind, &desc, &note).await.map_err(|e| err_json(&e.to_string(), raw, "mat publish"))?))
            }
            "embed" => {
                let tpl = req_flag(&rest, raw, "--tpl", "embed --tpl <tpl>")?;
                let ver = req_ver(&rest, raw, "embed --tpl <tpl> --version N")?;
                Ok(ok(self.store.embed(&tpl, ver, &sid).await.map_err(|e| err_json(&e.to_string(), raw, "mat embed"))?))
            }
            "clone" => {
                let tpl = req_flag(&rest, raw, "--tpl", "clone --tpl <tpl> --version N")?;
                let ver = req_ver(&rest, raw, "clone --tpl <tpl> --version N")?;
                Ok(ok(self.store.clone(&tpl, ver, &sid).await.map_err(|e| err_json(&e.to_string(), raw, "mat clone"))?))
            }
            "commit" => {
                let tpl = req_flag(&rest, raw, "--tpl", "commit --tpl <tpl>")?;
                let note = flag(&rest, "--note").unwrap_or_else(|| "commit".to_string());
                Ok(ok(self.store.commit(&sid, &tpl, &note).await.map_err(|e| err_json(&e.to_string(), raw, "mat commit"))?))
            }
            "update" => {
                let tpl = req_flag(&rest, raw, "--tpl", "update --tpl <tpl> --version N")?;
                let ver = req_ver(&rest, raw, "update --tpl <tpl> --version N")?;
                Ok(ok(self.store.update(&tpl, ver, &sid).await.map_err(|e| err_json(&e.to_string(), raw, "mat update"))?))
            }
            "remove" => {
                let tpl = req_pos(&rest, raw, "remove <template_id>")?;
                self.store.remove(&tpl).await.map_err(|e| err_json(&e.to_string(), raw, "mat remove"))?;
                Ok(ok(json!({ "removed": tpl })))
            }
            "query" => {
                let tpl = req_pos(&rest, raw, "query <template_id>")?;
                let ver = opt_ver(&rest, raw)?;
                let what = flag(&rest, "--what").unwrap_or_else(|| "info".to_string());
                let limit = flag(&rest, "--limit").and_then(|v| v.parse().ok()).unwrap_or(50);
                let offset = flag(&rest, "--offset").and_then(|v| v.parse().ok()).unwrap_or(0);
                Ok(ok(self.store.query(&tpl, ver.unwrap_or(0), &what, limit, offset).await.map_err(|e| err_json(&e.to_string(), raw, "mat query"))?))
            }
            "srv" => self.execute_srv(&rest, raw).await,
            other => Err(unknown_action(raw, other)),
        }
    }

    async fn execute_srv(&self, rest: &[&String], raw: &str) -> Result<Value, Value> {
        if rest.first().map(|s| s.as_str()) == Some("--help") {
            return Ok(ok(json!({ "help": crate::srv::srv_usage() })));
        }
        if rest.len() < 2 {
            return Err(err_json(
                "usage: mat srv <group> <verb> <tpl>[@ver] [args]",
                raw,
                "mat srv <group> <verb> <tpl>[@ver]",
            ));
        }
        let group = rest[0].as_str();
        let verb = rest[1].as_str();
        if verb == "--help" {
            return crate::srv::execute_srv(self.store.templates(), self.store.root(), "", &rest, raw).await;
        }
        let token = rest
            .get(2)
            .ok_or_else(|| err_json("missing <tpl>[@ver]", raw, "mat srv <group> <verb> <tpl>[@ver]"))?;
        let (tpl, ver) = match zw_db::templates::parse_version_sid(token) {
            Some((t, v)) => (t, v),
            None => (token.to_string(), 0),
        };
        let ver = if ver > 0 {
            ver
        } else {
            self.store
                .head(&tpl)
                .await
                .map_err(|e| err_json(&e.to_string(), raw, "mat srv"))?
        };
        if !self
            .store
            .version_exists(&tpl, ver)
            .await
            .map_err(|e| err_json(&e.to_string(), raw, "mat srv"))?
        {
            return Err(err_json(&format!("version {tpl}@{ver} does not exist"), raw, "mat srv <group> <verb> <tpl>[@ver]"));
        }
        let board_sid = zw_db::templates::version_sid(&tpl, ver);
        match group {
            "apps" if verb == "create" || verb == "delete" => Err(err_json(
                "templates own a single app; change it via commit/update or apps update|rate|ttl|hooks|keys|audit|link",
                raw,
                "mat srv apps <verb>",
            )),
            "files" => self.execute_files(&tpl, ver, verb, &rest[3..], raw).await,
            _ => crate::srv::execute_srv(
                self.store.templates(),
                self.store.root(),
                &board_sid,
                &rest,
                raw,
            )
            .await,
        }
    }

    async fn execute_files(&self, tpl: &str, ver: i64, verb: &str, args: &[&String], raw: &str) -> Result<Value, Value> {
        if verb == "--help" {
            return Ok(ok(json!({ "help": "mat srv files <verb> <tpl>[@ver] [args]\nlist <tpl>[@ver]\nget <tpl>[@ver] <file_id> --out <path>" })));
        }
        let dir = self.store.files_dir(tpl, ver);
        match verb {
            "list" => {
                let files = zw_materials::store::list_files(&dir)
                    .map_err(|e| err_json(&e.to_string(), raw, "mat srv files list"))?;
                let files: Vec<Value> = files
                    .into_iter()
                    .map(|(f, s)| json!({ "file": f, "size": s }))
                    .collect();
                Ok(ok(json!({ "template_id": tpl, "version": ver, "files": files })))
            }
            "get" => {
                let file_id = args.first().ok_or_else(|| err_json("missing <file_id>", raw, "mat srv files get <tpl>[@ver] <file_id> --out <path>"))?;
                let out = flag(args, "--out").ok_or_else(|| err_json("missing --out <path>", raw, "mat srv files get <tpl>[@ver] <file_id> --out <path>"))?;
                let src = dir.join(file_id);
                if !src.is_file() {
                    return Err(err_json("file does not exist", raw, "mat srv files get"));
                }
                if let Some(parent) = std::path::Path::new(&out).parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent).map_err(|e| err_json(&e.to_string(), raw, "mat srv files get --out"))?;
                    }
                }
                std::fs::copy(&src, &out).map_err(|e| err_json(&e.to_string(), raw, "mat srv files get"))?;
                Ok(ok(json!({ "template_id": tpl, "version": ver, "saved": out })))
            }
            other => Err(err_json(&format!("unknown files verb '{other}'"), raw, "mat srv files <verb>")),
        }
    }
}

#[async_trait]
impl McpToolProvider for ManageMaterialsProvider {
    fn name(&self) -> &str {
        "manage_materials"
    }

    fn description(&self) -> &str {
        "CLI tool for public, versioned, reusable template materials (mat ...). Run CLI commands here (never the shell tool). Use --session <sid> where noted. Start with 'mat about' or 'mat --help'."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "A CLI command to run inside this tool only (not the shell tool), e.g. 'mat list', 'mat --session web.x clone --tpl myapp --version 2'."
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

fn strip_program_name(mut tokens: Vec<String>) -> Vec<String> {
    if tokens.first().map(|s| s.as_str()) == Some("mat") {
        tokens.remove(0);
    }
    tokens
}

fn ok(v: Value) -> Value {
    json!({ "status": "ok", "result": v })
}

fn err_json(error: &str, command: &str, usage: &str) -> Value {
    json!({
        "status": "error",
        "error": error,
        "command": command,
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
    let mut j = err_json(&format!("unknown action '{action}'"), raw, GLOBAL_USAGE);
    if let Some((d, _)) = best {
        j["did_you_mean"] = json!(d);
        j["hint"] = json!(format!("try: mat {d}"));
    }
    j
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

fn flag(rest: &[&String], name: &str) -> Option<String> {
    for (i, tok) in rest.iter().enumerate() {
        if tok.as_str() == name {
            return rest.get(i + 1).map(|v| v.trim_matches('"').to_string());
        }
    }
    None
}

fn req_flag(rest: &[&String], raw: &str, name: &str, usage: &str) -> Result<String, Value> {
    flag(rest, name).ok_or_else(|| err_json(&format!("missing {name}"), raw, usage))
}

fn req_pos(rest: &[&String], raw: &str, usage: &str) -> Result<String, Value> {
    rest.first()
        .map(|s| s.to_string())
        .ok_or_else(|| err_json("missing argument", raw, usage))
}

fn opt_ver(rest: &[&String], raw: &str) -> Result<Option<i64>, Value> {
    match flag(rest, "--version") {
        Some(v) => v.parse::<i64>().map(Some).map_err(|_| err_json(&format!("'{v}' is not an integer"), raw, "--version N")),
        None => Ok(None),
    }
}

fn req_ver(rest: &[&String], raw: &str, usage: &str) -> Result<i64, Value> {
    match opt_ver(rest, raw)? {
        Some(v) if v > 0 => Ok(v),
        _ => Err(err_json("a positive --version N is required", raw, usage)),
    }
}
