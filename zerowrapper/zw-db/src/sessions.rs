use crate::DbPool;
use turso::params::Params;
use turso::Value;

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub user_id: String,
    pub agent_alias: String,
    pub account: Option<String>,
    pub chat_id: Option<String>,
    pub channel: String,
    pub created_at: String,
    pub last_active: String,
}

impl DbPool {
    pub async fn upsert_session(
        &self,
        id: &str,
        user_id: &str,
        agent_alias: &str,
        account: Option<&str>,
        chat_id: Option<&str>,
        channel: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "INSERT OR IGNORE INTO sessions (id, user_id, agent_alias, account, chat_id, channel) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            Params::Positional(vec![
                Value::from(id),
                Value::from(user_id),
                Value::from(agent_alias),
                account.map(Value::from).unwrap_or(Value::Null),
                chat_id.map(Value::from).unwrap_or(Value::Null),
                Value::from(channel),
            ]),
        ).await?;
        Ok(())
    }

    pub async fn touch_session(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn().await;
        conn.execute(
            "UPDATE sessions SET last_active = datetime('now') WHERE id = ?1",
            Params::Positional(vec![Value::from(id)]),
        ).await?;
        Ok(())
    }

    pub async fn get_session(&self, id: &str) -> anyhow::Result<Option<SessionRow>> {
        let conn = self.conn().await;
        let mut rows = conn.query(
            "SELECT id, user_id, agent_alias, account, chat_id, channel, created_at, last_active FROM sessions WHERE id = ?1",
            Params::Positional(vec![Value::from(id)]),
        ).await?;
        if let Some(row) = rows.next().await? {
            Ok(Some(SessionRow {
                id: match row.get_value(0)? { Value::Text(s) => s, _ => return Err(anyhow::anyhow!("invalid session id")) },
                user_id: match row.get_value(1)? { Value::Text(s) => s, _ => return Err(anyhow::anyhow!("invalid user_id")) },
                agent_alias: match row.get_value(2)? { Value::Text(s) => s, _ => return Err(anyhow::anyhow!("invalid agent_alias")) },
                account: match row.get_value(3)? { Value::Text(s) => Some(s), _ => None },
                chat_id: match row.get_value(4)? { Value::Text(s) => Some(s), _ => None },
                channel: match row.get_value(5)? { Value::Text(s) => s, _ => String::from("web") },
                created_at: match row.get_value(6)? { Value::Text(s) => s, _ => String::new() },
                last_active: match row.get_value(7)? { Value::Text(s) => s, _ => String::new() },
            }))
        } else {
            Ok(None)
        }
    }
}
