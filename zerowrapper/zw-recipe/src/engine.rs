use crate::actions::{ActionKind, RunReport, apply_action};
use crate::context::EventContext;
use crate::rule::{EventKind, Recipe};
use crate::secrets::Secrets;
use serde_json::{Map, Value};
use zw_db::DbPool;

const MAX_ACTIONS: usize = 50;

#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    pub matched: bool,
    pub report: RunReport,
}

pub struct Engine<'a> {
    pub db: &'a DbPool,
}

impl<'a> Engine<'a> {
    pub fn new(db: &'a DbPool) -> Self {
        Self { db }
    }

    pub async fn run_recipes(
        &self,
        recipes: &[Recipe],
        secrets: &Secrets,
        ctx: &EventContext,
        dry_run: bool,
    ) -> Vec<(String, RunOutcome)> {
        let mut out = Vec::new();
        for recipe in recipes {
            let outcome = self.run_recipe(recipe, secrets, ctx, dry_run).await;
            out.push((recipe.name.clone(), outcome));
        }
        out
    }

    pub async fn run_recipe(
        &self,
        recipe: &Recipe,
        secrets: &Secrets,
        ctx: &EventContext,
        dry_run: bool,
    ) -> RunOutcome {
        let start = std::time::Instant::now();
        if !recipe.enabled {
            return RunOutcome { matched: false, report: RunReport::default() };
        }
        let matches_event = match recipe.event_kind() {
            Some(k) => k == ctx.kind,
            None => false,
        };
        if !matches_event {
            return RunOutcome { matched: false, report: RunReport::default() };
        }
        if ctx.kind == EventKind::Cron {
            if let Some(want) = &ctx.cron_name {
                if !recipe.is_cron_for(want) {
                    return RunOutcome { matched: false, report: RunReport::default() };
                }
            }
        }
        if let Some(f) = &recipe.match_json {
            if let Err(e) = crate::actions::match_public(f, &ctx.payload) {
                let mut report = RunReport::default();
                report.logs.push(format!("recipe match error: {e}"));
                return RunOutcome { matched: true, report };
            }
            if !crate::actions::match_public(f, &ctx.payload).unwrap_or(false) {
                return RunOutcome { matched: true, report: RunReport::default() };
            }
        }
        let mut payload = ctx.payload.clone();
        let mut report = RunReport::default();
        if let Some(key) = &recipe.dedup_on {
            let k = zw_expr::get_path(&payload, key);
            if !k.is_null() && self.already_ran(&ctx.board_id, recipe, &k).await {
                return RunOutcome { matched: true, report: RunReport::default() };
            }
            if !dry_run {
                self.mark_ran(&ctx.board_id, recipe, &k).await;
            }
        }
        let mut i = 0;
        for action in &recipe.actions {
            if i >= MAX_ACTIONS {
                report.logs.push("max actions reached, stopping".to_string());
                break;
            }
            let Some((kind, detail)) = action_kind(action) else {
                report.logs.push(format!("skipping unknown action {}", action));
                continue;
            };
            let action_obj = action.get(detail).cloned().unwrap_or_else(|| Value::Object(Map::new()));
            if let Some(cond) = action_condition(action) {
                match zw_expr::truthy(&cond, &payload) {
                    Ok(true) => {}
                    Ok(false) => {
                        report.actions_run.push(format!("{kind:?} skipped (condition false)"));
                        continue;
                    }
                    Err(e) => {
                        report.actions_run.push(format!("{kind:?} skipped (bad condition: {e})"));
                        continue;
                    }
                }
            }
            if let Err(e) = apply_action(&kind, &action_obj, &mut payload, secrets, &mut report, ctx) {
                report.logs.push(format!("{kind:?}: {e}"));
                break;
            }
            if !dry_run {
                let external = match kind {
                    ActionKind::UpsertOther
                    | ActionKind::PatchOther
                    | ActionKind::Transaction
                    | ActionKind::Notify
                    | ActionKind::Call
                    | ActionKind::Schedule => crate::secrets::substitute_event(secrets, ctx, &action_obj),
                    _ => Ok(action_obj.clone()),
                };
                match external {
                    Ok(ext) => {
                        if let Err(e) = self.execute_external(&kind, &ext, ctx, &mut report).await {
                            report.logs.push(format!("{kind:?}: {e}"));
                            break;
                        }
                    }
                    Err(e) => {
                        report.logs.push(format!("{kind:?}: {e}"));
                        break;
                    }
                }
            }
            i += 1;
        }
        if !dry_run && report.changed && ctx.kind != EventKind::Inbound && ctx.kind != EventKind::Cron {
            if let (Some(seq), true) = (ctx.seq, payload != ctx.payload) {
                match self.db.srv_record_set(&ctx.board_id, seq, &payload, None).await {
                    Ok(()) => report.actions_run.push("persisted updated record".to_string()),
                    Err(e) => report.logs.push(format!("persist: {e}")),
                }
            }
        }
        if !dry_run {
            let status = if report.logs.iter().any(|l| l.to_lowercase().contains("error")) {
                "error"
            } else {
                "ok"
            };
            let message = report.logs.join(" | ");
            let duration_ms = start.elapsed().as_millis() as i64;
            let triggered_at = chrono::Utc::now().naive_utc().format("%Y-%m-%d %H:%M:%S").to_string();
            if let Err(e) = self
                .db
                .srv_recipe_runs_log_insert(&ctx.board_id, &recipe.name, &triggered_at, duration_ms, status, &message)
                .await
            {
                tracing::warn!("recipe run log insert failed: {e}");
            }
        }
        RunOutcome { matched: true, report }
    }

    async fn execute_external(
        &self,
        kind: &ActionKind,
        action: &Value,
        ctx: &EventContext,
        report: &mut RunReport,
    ) -> anyhow::Result<()> {
        match kind {
            ActionKind::UpsertOther => self.external_upsert(action, ctx).await?,
            ActionKind::PatchOther => self.external_patch_other(action, ctx).await?,
            ActionKind::Transaction => self.external_transaction(action, ctx).await?,
            ActionKind::Notify | ActionKind::Call => self.external_call(action, ctx, report).await?,
            ActionKind::Schedule => self.external_schedule(action).await?,
            _ => {}
        }
        Ok(())
    }

    async fn external_upsert(&self, action: &Value, ctx: &EventContext) -> anyhow::Result<()> {
        let map = action.as_object().ok_or_else(|| anyhow::anyhow!("$upsert_other must be an object"))?;
        let board = map
            .get("board")
            .and_then(|b| b.as_str())
            .unwrap_or(&ctx.board_id);
        let payload = map.get("payload").cloned().unwrap_or_else(|| Value::Object(Map::new()));
        self.db.srv_record_insert(board, &payload, Some(&format!("recipe:{}", ctx.board_id)), true).await?;
        Ok(())
    }

    async fn external_patch_other(&self, action: &Value, ctx: &EventContext) -> anyhow::Result<()> {
        let map = action.as_object().ok_or_else(|| anyhow::anyhow!("$patch_other must be an object"))?;
        let board = map
            .get("board")
            .and_then(|b| b.as_str())
            .unwrap_or(&ctx.board_id);
        let filter = map
            .get("filter")
            .ok_or_else(|| anyhow::anyhow!("$patch_other needs \"filter\""))?;
        let patch = map
            .get("patch")
            .ok_or_else(|| anyhow::anyhow!("$patch_other needs \"patch\""))?;
        let filter = zw_db::filters::parse_filter(&filter)
            .map_err(|e| anyhow::anyhow!("$patch_other filter: {e}"))?;
        self.db.srv_record_patch_first(board, &filter.conds, patch).await?;
        Ok(())
    }

    async fn external_transaction(&self, action: &Value, ctx: &EventContext) -> anyhow::Result<()> {
        let steps = action
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("$transaction must be an array of {{filter, patch}} steps"))?;
        if steps.is_empty() {
            anyhow::bail!("$transaction needs at least one step");
        }
        let mut db_steps: Vec<(Vec<zw_db::filters::FilterCond>, serde_json::Value)> = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            let map = step
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("$transaction step {i} must be an object"))?;
            let board = map
                .get("board")
                .and_then(|b| b.as_str())
                .unwrap_or(&ctx.board_id);
            if board != ctx.board_id {
                anyhow::bail!("$transaction step {i}: cross-board writes are not supported (same-board only)");
            }
            let filter = map
                .get("filter")
                .ok_or_else(|| anyhow::anyhow!("$transaction step {i} needs \"filter\""))?;
            let patch = map
                .get("patch")
                .ok_or_else(|| anyhow::anyhow!("$transaction step {i} needs \"patch\""))?;
            let filter = zw_db::filters::parse_filter(&filter)
                .map_err(|e| anyhow::anyhow!("$transaction step {i} filter: {e}"))?;
            db_steps.push((filter.conds, patch.clone()));
        }
        self.db.srv_record_patch_many(&ctx.board_id, &db_steps).await?;
        Ok(())
    }

    async fn external_call(&self, action: &Value, ctx: &EventContext, report: &mut RunReport) -> anyhow::Result<()> {
        let map = action.as_object().ok_or_else(|| anyhow::anyhow!("$call/$notify must be an object"))?;
        let url = map.get("url").and_then(|u| u.as_str()).ok_or_else(|| anyhow::anyhow!("missing url"))?;
        let method = map.get("method").and_then(|m| m.as_str()).unwrap_or("POST");
        let timeout = map.get("timeout_ms").and_then(|t| t.as_u64()).unwrap_or(15_000);
        let mut headers: Vec<(String, String)> = Vec::new();
        if let Some(h) = map.get("headers") {
            match h {
                Value::Object(o) => {
                    for (k, v) in o {
                        headers.push((k.clone(), v.as_str().unwrap_or("").to_string()));
                    }
                }
                Value::Array(arr) => {
                    for el in arr {
                        if let Value::Array(pair) = el {
                            if pair.len() == 2 {
                                headers.push((pair[0].as_str().unwrap_or("").to_string(), pair[1].as_str().unwrap_or("").to_string()));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let body = map
            .get("body")
            .map(|b| serde_json::to_string(b).unwrap_or_else(|_| b.to_string()));
        let owner = self.board_owner(&ctx.board_id).await?;
        let res = self.db.srv_http_call(&ctx.board_id, &owner, method, url, &headers, body.as_deref(), timeout).await?;
        report.logs.push(format!("call {url} -> {} ({}ms)", res.status, res.duration_ms));
        Ok(())
    }

    async fn external_schedule(&self, action: &Value) -> anyhow::Result<()> {
        let map = action.as_object().ok_or_else(|| anyhow::anyhow!("$schedule must be an object"))?;
        let name = map.get("name").and_then(|n| n.as_str()).unwrap_or("recipe");
        let at = map.get("at").and_then(|a| a.as_str())
            .or_else(|| map.get("schedule").and_then(|s| s.as_str()))
            .ok_or_else(|| anyhow::anyhow!("$schedule needs \"at\" (cron or @every)"))?;
        let action_json = map.get("action").cloned().unwrap_or(Value::Null);
        self.db.srv_job_add("", name, at, &serde_json::to_string(&action_json)?).await?;
        Ok(())
    }

    async fn already_ran(&self, board_id: &str, recipe: &Recipe, key: &Value) -> bool {
        matches!(self.db.srv_recipe_ran(board_id, &recipe.name, key).await, Ok(true))
    }

    async fn mark_ran(&self, board_id: &str, recipe: &Recipe, key: &Value) {
        let _ = self.db.srv_recipe_mark_ran(board_id, &recipe.name, key).await;
    }

    async fn board_owner(&self, board_id: &str) -> anyhow::Result<String> {
        let app = self
            .db
            .srv_app_by_id(board_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("board {board_id} not found"))?;
        Ok(app.owner_key)
    }
}

pub fn action_kind(action: &Value) -> Option<(ActionKind, &str)> {
    let obj = action.as_object()?;
    let key = obj.keys().next()?.as_str();
    let kind = ActionKind::parse(key)?;
    Some((kind, key))
}

pub fn action_condition(action: &Value) -> Option<String> {
    let obj = action.as_object()?;
    let (_, v) = obj.iter().next()?;
    v.as_object()
        .and_then(|m| m.get("when"))
        .and_then(|w| w.as_str())
        .map(|s| s.to_string())
}
