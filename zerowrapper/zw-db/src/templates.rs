use crate::DbPool;
use turso::params::Params;
use turso::Value;

#[derive(Debug, Clone)]
pub struct TemplateRow {
    pub template_id: String,
    pub name: String,
    pub kind: String,
    pub description: String,
    pub current_version: i64,
    pub created_at: String,
    pub updated_at: String,
}

pub fn version_sid(template_id: &str, version: i64) -> String {
    format!("{template_id}@{version}")
}

pub fn parse_version_sid(sid: &str) -> Option<(String, i64)> {
    let at = sid.rfind('@')?;
    let id = &sid[..at];
    let version = sid[at + 1..].parse::<i64>().ok()?;
    if id.is_empty() || version < 1 {
        return None;
    }
    Some((id.to_string(), version))
}

impl DbPool {
    pub async fn tpl_create(&self, template_id: &str, name: &str, kind: &str, description: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "INSERT INTO templates (template_id, name, kind, description) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(template_id) DO UPDATE SET
                 name = CASE WHEN ?2 <> '' THEN ?2 ELSE name END,
                 kind = CASE WHEN ?3 <> '' THEN ?3 ELSE kind END,
                 description = ?4,
                 updated_at = datetime('now')",
            Params::Positional(vec![
                Value::from(template_id),
                Value::from(name),
                Value::from(kind),
                Value::from(description),
            ]),
        )
        .await?;
        Ok(())
    }

    pub async fn tpl_get(&self, template_id: &str) -> anyhow::Result<Option<TemplateRow>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT template_id, name, kind, description, current_version, created_at, updated_at FROM templates WHERE template_id = ?1",
                Params::Positional(vec![Value::from(template_id)]),
            )
            .await?;
        if let Some(row) = rows.next().await? {
            Ok(Some(TemplateRow {
                template_id: match row.get_value(0)? { Value::Text(s) => s, _ => String::new() },
                name: match row.get_value(1)? { Value::Text(s) => s, _ => String::new() },
                kind: match row.get_value(2)? { Value::Text(s) => s, _ => String::new() },
                description: match row.get_value(3)? { Value::Text(s) => s, _ => String::new() },
                current_version: match row.get_value(4)? { Value::Integer(n) => n, _ => 1 },
                created_at: match row.get_value(5)? { Value::Text(s) => s, _ => String::new() },
                updated_at: match row.get_value(6)? { Value::Text(s) => s, _ => String::new() },
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn tpl_list(&self) -> anyhow::Result<Vec<TemplateRow>> {
        let conn = self.conn().await;
        let mut rows = conn
            .query(
                "SELECT template_id, name, kind, description, current_version, created_at, updated_at FROM templates ORDER BY updated_at DESC",
                Params::None,
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(TemplateRow {
                template_id: match row.get_value(0)? { Value::Text(s) => s, _ => String::new() },
                name: match row.get_value(1)? { Value::Text(s) => s, _ => String::new() },
                kind: match row.get_value(2)? { Value::Text(s) => s, _ => String::new() },
                description: match row.get_value(3)? { Value::Text(s) => s, _ => String::new() },
                current_version: match row.get_value(4)? { Value::Integer(n) => n, _ => 1 },
                created_at: match row.get_value(5)? { Value::Text(s) => s, _ => String::new() },
                updated_at: match row.get_value(6)? { Value::Text(s) => s, _ => String::new() },
            });
        }
        Ok(out)
    }

    pub async fn tpl_set_version(&self, template_id: &str, version: i64) -> anyhow::Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "UPDATE templates SET current_version = ?2, updated_at = datetime('now') WHERE template_id = ?1",
            Params::Positional(vec![Value::from(template_id), Value::Integer(version)]),
        )
        .await?;
        Ok(())
    }

    pub async fn tpl_remove(&self, template_id: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "DELETE FROM templates WHERE template_id = ?1",
            Params::Positional(vec![Value::from(template_id)]),
        )
        .await?;
        Ok(())
    }
}
