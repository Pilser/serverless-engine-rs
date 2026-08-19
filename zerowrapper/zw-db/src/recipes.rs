use crate::DbPool;
use serde::Serialize;
use turso::params::Params;
use turso::Value;

#[derive(Debug, Clone, Serialize)]
pub struct SrvRecipe {
    pub id: i64,
    pub board_id: String,
    pub name: String,
    pub when_json: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_json: Option<serde_json::Value>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedup_on: Option<String>,
    pub actions_json: serde_json::Value,
    pub created_at: String,
}

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

async fn read_recipe(rows: &mut turso::Rows) -> anyhow::Result<Option<SrvRecipe>> {
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let id = match row.get_value(0)? { Value::Integer(i) => i, _ => 0 };
    let board_id = match row.get_value(1)? { Value::Text(s) => s, _ => String::new() };
    let name = match row.get_value(2)? { Value::Text(s) => s, _ => String::new() };
    let when_json = match row.get_value(3)? { Value::Text(s) => s, _ => String::new() };
    let match_json = match row.get_value(4)? {
        Value::Text(s) => serde_json::from_str(&s).ok(),
        _ => None,
    };
    let enabled = match row.get_value(5)? { Value::Integer(i) => i != 0, _ => true };
    let dedup_on = match row.get_value(6)? { Value::Text(s) => Some(s), _ => None };
    let actions_json = match row.get_value(7)? {
        Value::Text(s) => serde_json::from_str(&s).unwrap_or(serde_json::Value::Array(Vec::new())),
        _ => serde_json::Value::Array(Vec::new()),
    };
    let created_at = match row.get_value(8)? { Value::Text(s) => s, _ => String::new() };
    Ok(Some(SrvRecipe { id, board_id, name, when_json, match_json, enabled, dedup_on, actions_json, created_at }))
}

const RECIPE_COLS: &str = "id, board_id, name, when_json, match_json, enabled, dedup_on, actions_json, created_at";

impl DbPool {
    pub async fn srv_recipe_list(&self, board_id: &str) -> anyhow::Result<Vec<SrvRecipe>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                &format!("SELECT {RECIPE_COLS} FROM wb_recipes WHERE board_id = ?1 ORDER BY id"),
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(r) = read_recipe(&mut rows).await? {
            out.push(r);
        }
        Ok(out)
    }

    pub async fn srv_recipe_add(&self, board_id: &str, name: &str, when_json: &str, match_json: Option<&str>, enabled: bool, dedup_on: Option<&str>, actions_json: &str) -> anyhow::Result<i64> {
        let conn = self.conn().await;
        let b = sq(board_id);
        let n = sq(name);
        let w = sq(when_json);
        let m = match match_json {
            Some(m) => format!("'{}'", sq(m)),
            None => "NULL".to_string(),
        };
        let e = if enabled { 1 } else { 0 };
        let d = match dedup_on {
            Some(d) => format!("'{}'", sq(d)),
            None => "NULL".to_string(),
        };
        let a = sq(actions_json);
        let sql = format!(
            "INSERT INTO wb_recipes (board_id, name, when_json, match_json, enabled, dedup_on, actions_json) \
             VALUES ('{b}', '{n}', '{w}', {m}, {e}, {d}, '{a}')"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(conn.last_insert_rowid())
    }

    pub async fn srv_recipe_set_enabled(&self, board_id: &str, name: &str, enabled: bool) -> anyhow::Result<bool> {
        let conn = self.conn().await;
        let e = if enabled { 1 } else { 0 };
        let affected = conn
            .execute(
                &format!("UPDATE wb_recipes SET enabled = {e} WHERE board_id = '{b}' AND name = '{n}'",
                    b = sq(board_id), n = sq(name)),
                Params::Positional(vec![]),
            )
            .await?;
        Ok(affected > 0)
    }

    pub async fn srv_recipe_remove(&self, board_id: &str, name: &str) -> anyhow::Result<bool> {
        let conn = self.conn().await;
        let affected = conn
            .execute(
                &format!("DELETE FROM wb_recipes WHERE board_id = '{b}' AND name = '{n}'",
                    b = sq(board_id), n = sq(name)),
                Params::Positional(vec![]),
            )
            .await?;
        Ok(affected > 0)
    }

    pub async fn srv_recipe_ran(&self, board_id: &str, recipe: &str, dedup_key: &serde_json::Value) -> anyhow::Result<bool> {
        let key = crate::audit::audit_hash_str(&serde_json::to_string(dedup_key).unwrap_or_default());
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT 1 FROM wb_recipe_runs WHERE board_id = ?1 AND recipe = ?2 AND dedup_key = ?3 LIMIT 1",
                Params::Positional(vec![Value::from(board_id), Value::from(recipe), Value::from(key)]),
            )
            .await?;
        Ok(rows.next().await?.is_some())
    }

    pub async fn srv_recipe_mark_ran(&self, board_id: &str, recipe: &str, dedup_key: &serde_json::Value) -> anyhow::Result<()> {
        let key = crate::audit::audit_hash_str(&serde_json::to_string(dedup_key).unwrap_or_default());
        let conn = self.conn().await;
        let b = sq(board_id);
        let r = sq(recipe);
        let k = sq(&key);
        conn.execute(
            &format!("INSERT OR IGNORE INTO wb_recipe_runs (board_id, recipe, dedup_key) VALUES ('{b}', '{r}', '{k}')"),
            Params::Positional(vec![]),
        )
        .await?;
        Ok(())
    }

    pub async fn srv_recipe_runs_log_insert(
        &self,
        board_id: &str,
        recipe: &str,
        triggered_at: &str,
        duration_ms: i64,
        status: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let b = sq(board_id);
        let r = sq(recipe);
        let t = sq(triggered_at);
        let st = sq(status);
        let m = sq(message);
        let sql = format!(
            "INSERT INTO wb_recipe_runs_log (board_id, recipe, triggered_at, duration_ms, status, message) \
             VALUES ('{b}', '{r}', '{t}', {duration_ms}, '{st}', '{m}')"
        );
        conn.execute(&sql, Params::Positional(vec![])).await?;
        Ok(())
    }

    pub async fn srv_recipe_runs_log(&self, board_id: &str, recipe: Option<&str>, limit: i64) -> anyhow::Result<Vec<RecipeRunLog>> {
        let conn = self.conn().await;
        let (sql, params) = match recipe {
            Some(rn) => (
                "SELECT id, board_id, recipe, triggered_at, duration_ms, status, message \
                 FROM wb_recipe_runs_log WHERE board_id = ?1 AND recipe = ?2 ORDER BY id DESC LIMIT ?3"
                    .to_string(),
                Params::Positional(vec![Value::from(board_id), Value::from(rn), Value::Integer(limit)]),
            ),
            None => (
                "SELECT id, board_id, recipe, triggered_at, duration_ms, status, message \
                 FROM wb_recipe_runs_log WHERE board_id = ?1 ORDER BY id DESC LIMIT ?2"
                    .to_string(),
                Params::Positional(vec![Value::from(board_id), Value::Integer(limit)]),
            ),
        };
        let mut rows = conn.query(&sql, params).await?;
        let mut out = Vec::new();
        while let Some(r) = read_recipe_run_log(&mut rows).await? {
            out.push(r);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecipeRunLog {
    pub id: i64,
    pub board_id: String,
    pub recipe: String,
    pub triggered_at: String,
    pub duration_ms: i64,
    pub status: String,
    pub message: String,
}

async fn read_recipe_run_log(rows: &mut turso::Rows) -> anyhow::Result<Option<RecipeRunLog>> {
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let id = match row.get_value(0)? {
        Value::Integer(i) => i,
        _ => 0,
    };
    let board_id = match row.get_value(1)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    let recipe = match row.get_value(2)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    let triggered_at = match row.get_value(3)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    let duration_ms = match row.get_value(4)? {
        Value::Integer(i) => i,
        _ => 0,
    };
    let status = match row.get_value(5)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    let message = match row.get_value(6)? {
        Value::Text(s) => s,
        _ => String::new(),
    };
    Ok(Some(RecipeRunLog {
        id,
        board_id,
        recipe,
        triggered_at,
        duration_ms,
        status,
        message,
    }))
}
