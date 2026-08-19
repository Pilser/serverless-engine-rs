use serde_json::Value;
use zw_db::DbPool;
use zw_db::recipes::SrvRecipe;

use crate::context::EventContext;
use crate::rule::{EventKind, Recipe};
use crate::secrets::Secrets;
use crate::Engine;

/// Convert a persisted recipe row into the engine [`Recipe`] type.
pub fn recipe_from_srv(sr: &SrvRecipe) -> Option<Recipe> {
    let actions = sr.actions_json.as_array()?.clone();
    Some(Recipe {
        name: sr.name.clone(),
        when: sr.when_json.clone(),
        match_json: sr.match_json.clone(),
        enabled: sr.enabled,
        dedup_on: sr.dedup_on.clone(),
        actions,
    })
}

/// Run every recipe whose event matches `kind` for the given record write.
///
/// Shared by the HTTP route handlers and the `wb` CLI so both paths trigger
/// the same automation on `record.created` / `record.updated` / `record.deleted`.
pub async fn run_recipes_for_record(
    pool: &DbPool,
    board_id: &str,
    kind: &str,
    seq: i64,
    payload: &Value,
    writer: Option<&str>,
) {
    let event_kind = match kind {
        "created" => EventKind::Created,
        "updated" => EventKind::Updated,
        "deleted" => EventKind::Deleted,
        _ => return,
    };
    let recipes = match pool.srv_recipe_list(board_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("run_recipes_for_record: list error: {}", e);
            return;
        }
    };
    if recipes.is_empty() {
        return;
    }
    let mut secrets = Secrets::new();
    if let Ok(map) = pool.srv_secrets_map(board_id).await {
        for (name, value) in map {
            secrets.insert(&name, &value);
        }
    }
    let engine = Engine { db: pool };
    for sr in recipes {
        let Some(recipe) = recipe_from_srv(&sr) else { continue };
        if !matches!(recipe.event_kind(), Some(k) if k == event_kind) {
            continue;
        }
        let ctx = EventContext {
            kind: event_kind,
            board_id: board_id.to_string(),
            seq: Some(seq),
            payload: payload.clone(),
            writer: writer.map(|s| s.to_string()),
            cron_name: None,
        };
        let outcome = engine.run_recipe(&recipe, &secrets, &ctx, false).await;
        if !outcome.report.logs.is_empty() {
            tracing::debug!(
                "run_recipes_for_record: recipe {} logs: {:?}",
                recipe.name,
                outcome.report.logs
            );
        }
    }
}
