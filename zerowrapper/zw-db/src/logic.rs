use crate::DbPool;
use serde_json::{Map, Value};
use turso::params::Params;
use turso::Value as TursoValue;
use zw_expr::ast::Expr;

fn sq(s: &str) -> String {
    s.replace('\'', "''")
}

fn col(v: Option<&str>) -> String {
    match v {
        Some(s) => format!("'{}'", sq(s)),
        None => "NULL".to_string(),
    }
}

async fn check_owner(conn: &mut turso::Connection, board_id: &str, owner: &str) -> anyhow::Result<()> {
    let mut rows = conn
        .query(
            "SELECT owner_key FROM wb_apps WHERE board_id = ?1",
            Params::Positional(vec![TursoValue::from(board_id)]),
        )
        .await?;
    match rows.next().await? {
        Some(row) => match row.get_value(0)? {
            TursoValue::Text(s) => {
                if s != owner {
                    anyhow::bail!("board {board_id} is not owned by {owner}");
                }
            }
            _ => return Err(anyhow::anyhow!("invalid owner_key")),
        },
        None => anyhow::bail!("board {board_id} not found"),
    }
    Ok(())
}

async fn get_config_json(rows: &mut turso::Rows) -> anyhow::Result<Option<Option<String>>> {
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    match row.get_value(0)? {
        TursoValue::Text(s) => Ok(Some(Some(s))),
        TursoValue::Null => Ok(Some(None)),
        _ => Ok(Some(None)),
    }
}

impl DbPool {
    pub async fn srv_app_set_computed(&self, board_id: &str, owner: &str, json: Option<&str>) -> anyhow::Result<()> {
        let mut conn = self.conn().await;
        check_owner(&mut conn, board_id, owner).await?;
        conn.execute(
            &format!("UPDATE wb_apps SET computed_json = {} WHERE board_id = '{}'", col(json), sq(board_id)),
            Params::Positional(vec![]),
        )
        .await?;
        Ok(())
    }

    pub async fn srv_app_set_validate(&self, board_id: &str, owner: &str, json: Option<&str>) -> anyhow::Result<()> {
        let mut conn = self.conn().await;
        check_owner(&mut conn, board_id, owner).await?;
        conn.execute(
            &format!("UPDATE wb_apps SET validate_json = {} WHERE board_id = '{}'", col(json), sq(board_id)),
            Params::Positional(vec![]),
        )
        .await?;
        Ok(())
    }

    pub async fn srv_app_set_redact(&self, board_id: &str, owner: &str, json: Option<&str>) -> anyhow::Result<()> {
        let mut conn = self.conn().await;
        check_owner(&mut conn, board_id, owner).await?;
        conn.execute(
            &format!("UPDATE wb_apps SET redact_json = {} WHERE board_id = '{}'", col(json), sq(board_id)),
            Params::Positional(vec![]),
        )
        .await?;
        Ok(())
    }

    pub async fn srv_computed_get(&self, board_id: &str) -> anyhow::Result<Option<Value>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT computed_json FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![TursoValue::from(board_id)]),
            )
            .await?;
        Ok(match get_config_json(&mut rows).await? {
            Some(Some(s)) => serde_json::from_str(&s).ok(),
            _ => None,
        })
    }

    pub async fn srv_validate_get(&self, board_id: &str) -> anyhow::Result<Option<Value>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT validate_json FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![TursoValue::from(board_id)]),
            )
            .await?;
        Ok(match get_config_json(&mut rows).await? {
            Some(Some(s)) => serde_json::from_str(&s).ok(),
            _ => None,
        })
    }

    pub async fn srv_redact_get(&self, board_id: &str) -> anyhow::Result<Option<Value>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT redact_json FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![TursoValue::from(board_id)]),
            )
            .await?;
        Ok(match get_config_json(&mut rows).await? {
            Some(Some(s)) => serde_json::from_str(&s).ok(),
            _ => None,
        })
    }

    pub async fn srv_app_set_webhook_secret(&self, board_id: &str, owner: &str, secret: Option<&str>) -> anyhow::Result<()> {
        let mut conn = self.conn().await;
        check_owner(&mut conn, board_id, owner).await?;
        conn.execute(
            &format!("UPDATE wb_apps SET webhook_secret = {} WHERE board_id = '{}'", col(secret), sq(board_id)),
            Params::Positional(vec![]),
        )
        .await?;
        Ok(())
    }

    pub async fn srv_webhook_secret_get(&self, board_id: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT webhook_secret FROM wb_apps WHERE board_id = ?1",
                Params::Positional(vec![TursoValue::from(board_id)]),
            )
            .await?;
        match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                TursoValue::Text(s) if !s.is_empty() => Ok(Some(s)),
                _ => Ok(None),
            },
            None => Ok(None),
        }
    }
}

fn compile(field: &str, expr: &str) -> anyhow::Result<Expr> {
    zw_expr::parse(expr).map_err(|e| anyhow::anyhow!("computed field '{field}': {e}"))
}

/// Apply `computed` fields (map of field -> expr) to a payload, in place.
/// Missing inputs resolve to Null; computed fields may reference earlier ones.
pub fn apply_computed(computed: &Value, payload: &mut Value) -> anyhow::Result<()> {
    let Some(map) = computed.as_object() else {
        anyhow::bail!("computed must be an object of {{field: expr}}");
    };
    for (field, expr) in map {
        let expr = expr.as_str().ok_or_else(|| anyhow::anyhow!("computed field '{field}' must be a string expression"))?;
        let e = compile(field, expr)?;
        let v = zw_expr::apply(&e, payload)?;
        set_nested(payload, field, v)?;
    }
    Ok(())
}

/// Run `validate` rules (array of {when, error}) against a payload.
/// Returns the first failing message, or None if all pass.
pub fn validate_rules(rules: &Value, payload: &Value) -> anyhow::Result<Option<String>> {
    let Some(arr) = rules.as_array() else {
        anyhow::bail!("validate must be an array of {{when, error}}");
    };
    for rule in arr {
        let when = rule
            .get("when")
            .and_then(|w| w.as_str())
            .ok_or_else(|| anyhow::anyhow!("validate rule needs a \"when\" expression"))?;
        let msg = rule
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("validation failed")
            .to_string();
        if !zw_expr::truthy(when, payload).map_err(|e| anyhow::anyhow!("validate 'when': {e}"))? {
            continue;
        }
        return Ok(Some(msg));
    }
    Ok(None)
}

/// Redact `redact` paths (array of json paths) in place by replacing with "***".
pub fn apply_redact(redact: &Value, payload: &mut Value) {
    let Some(paths) = redact.as_array() else {
        return;
    };
    for p in paths {
        if let Some(path) = p.as_str() {
            let segs = match zw_expr::path::parse_path(path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if segs.is_empty() {
                *payload = Value::String("***".to_string());
                continue;
            }
            set_last(payload, &segs, Value::String("***".to_string()));
        }
    }
}

fn set_nested(payload: &mut Value, field: &str, value: Value) -> anyhow::Result<()> {
    let path = if field.starts_with('$') { field.to_string() } else { format!("$.{field}") };
    let segs = zw_expr::path::parse_path(&path)?;
    if segs.is_empty() {
        *payload = value;
        return Ok(());
    }
    set_last(payload, &segs, value);
    Ok(())
}

fn set_last(payload: &mut Value, segs: &[zw_expr::ast::PathSeg], value: Value) {
    use zw_expr::ast::PathSeg;
    if segs.is_empty() {
        return;
    }
    let mut cur = payload;
    for seg in &segs[..segs.len() - 1] {
        match seg {
            PathSeg::Key(k) => {
                if !cur.is_object() {
                    *cur = Value::Object(Map::new());
                }
                let obj = cur.as_object_mut().unwrap();
                cur = obj.entry(k.clone()).or_insert(Value::Object(Map::new()));
            }
            PathSeg::Index(i) => {
                if !cur.is_array() {
                    *cur = Value::Array(Vec::new());
                }
                let arr = cur.as_array_mut().unwrap();
                if *i >= arr.len() {
                    arr.resize(*i + 1, Value::Null);
                }
                cur = &mut arr[*i];
            }
            PathSeg::Wild => return,
        }
    }
    match &segs[segs.len() - 1] {
        PathSeg::Key(k) => {
            if !cur.is_object() {
                *cur = Value::Object(Map::new());
            }
            cur.as_object_mut().unwrap().insert(k.clone(), value);
        }
        PathSeg::Index(i) => {
            if !cur.is_array() {
                *cur = Value::Array(Vec::new());
            }
            let arr = cur.as_array_mut().unwrap();
            if *i >= arr.len() {
                arr.resize(*i + 1, Value::Null);
            }
            arr[*i] = value;
        }
        PathSeg::Wild => {}
    }
}
