use crate::hook_deliveries::SrvDelivery;
use crate::keys::SrvApiKey;
use crate::whiteboard::PublicSnap;
use crate::DbPool;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use turso::params::Params;
use turso::Value;

#[derive(Clone)]
pub struct SessionStore {
    system: DbPool,
    root: Arc<PathBuf>,
    open: Arc<Mutex<HashMap<String, Arc<DbPool>>>>,
}

impl SessionStore {
    pub fn new(root: PathBuf, system: DbPool) -> Self {
        Self {
            system,
            root: Arc::new(root),
            open: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn system(&self) -> &DbPool {
        &self.system
    }

    fn session_path(&self, sid: &str) -> PathBuf {
        self.root.join("sessions").join(sid).join(format!("{sid}.db"))
    }

    pub async fn session(&self, sid: &str) -> anyhow::Result<Arc<DbPool>> {
        let mut open = self.open.lock().await;
        if let Some(pool) = open.get(sid) {
            return Ok(pool.clone());
        }
        let path = self.session_path(sid).to_str().unwrap().to_string();
        let pool = Arc::new(DbPool::open(&path).await?);
        pool.migrate_session().await?;
        open.insert(sid.to_string(), pool.clone());
        Ok(pool)
    }

    pub async fn discard(&self, sid: &str) {
        self.open.lock().await.remove(sid);
    }

    pub async fn session_id_for_board(&self, board_id: &str) -> anyhow::Result<String> {
        let conn = self.system.conn().await;
        let mut rows = conn
            .query(
                "SELECT session_id FROM srv_board_index WHERE board_id = ?1",
                Params::Positional(vec![Value::from(board_id)]),
            )
            .await?;
        match rows.next().await? {
            Some(row) => match row.get_value(0)? {
                Value::Text(s) => Ok(s),
                _ => Err(anyhow::anyhow!("invalid session_id in srv_board_index")),
            },
            None => anyhow::bail!("board {board_id} not found"),
        }
    }

    pub async fn board_pool(&self, board_id: &str) -> anyhow::Result<Arc<DbPool>> {
        let sid = self.session_id_for_board(board_id).await?;
        self.session(&sid).await
    }

    pub async fn register_board(&self, sid: &str, board_id: &str, owner_key: &str) -> anyhow::Result<()> {
        let conn = self.system.conn().await;
        conn.execute(
            "INSERT INTO srv_board_index (board_id, session_id, owner_key) VALUES (?1, ?2, ?3)",
            Params::Positional(vec![Value::from(board_id), Value::from(sid), Value::from(owner_key)]),
        )
        .await?;
        Ok(())
    }

    pub async fn unregister_board(&self, board_id: &str) -> anyhow::Result<()> {
        let conn = self.system.conn().await;
        conn.execute(
            "DELETE FROM srv_board_index WHERE board_id = ?1",
            Params::Positional(vec![Value::from(board_id)]),
        )
        .await?;
        Ok(())
    }

    pub async fn all_session_ids(&self) -> anyhow::Result<Vec<String>> {
        let dir = self.root.join("sessions");
        let mut out = Vec::new();
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => return Ok(out),
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                out.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        out.sort();
        Ok(out)
    }

    async fn ensure_board_owned(&self, board_id: &str, owner: &str) -> anyhow::Result<()> {
        let pool = self.board_pool(board_id).await?;
        if pool.srv_app_owned(board_id, owner).await?.is_none() {
            anyhow::bail!("board {board_id} is not owned by {owner}");
        }
        Ok(())
    }

    pub async fn create_board(
        &self,
        sid: &str,
        title: &str,
        schema_json: Option<&str>,
        public_reads: bool,
        unique_key: Option<&str>,
    ) -> anyhow::Result<String> {
        let pool = self.session(sid).await?;
        let board_id = pool.srv_app_create(sid, title, schema_json, public_reads, unique_key).await?;
        self.register_board(sid, &board_id, sid).await?;
        Ok(board_id)
    }

    pub async fn delete_board(&self, sid: &str, board_id: &str) -> anyhow::Result<()> {
        let pool = self.session(sid).await?;
        pool.srv_app_delete(board_id, sid).await?;
        self.unregister_board(board_id).await?;
        Ok(())
    }

    pub async fn key_issue(&self, board_id: &str, owner: &str, role: &str, writer: Option<&str>, scope: Option<&str>) -> anyhow::Result<(String, String)> {
        self.ensure_board_owned(board_id, owner).await?;
        self.system.srv_key_issue(board_id, owner, role, writer, scope).await
    }

    pub async fn key_list(&self, board_id: &str, owner: &str) -> anyhow::Result<Vec<SrvApiKey>> {
        self.ensure_board_owned(board_id, owner).await?;
        self.system.srv_key_list(board_id, owner).await
    }

    pub async fn key_revoke(&self, board_id: &str, owner: &str, bucket: &str) -> anyhow::Result<bool> {
        self.ensure_board_owned(board_id, owner).await?;
        self.system.srv_key_revoke(board_id, owner, bucket).await
    }

    pub async fn set_page_public(&self, sid: &str, page_no: i64, public: bool) -> anyhow::Result<Option<String>> {
        let pool = self.session(sid).await?;
        let slug = pool.wb_set_public(sid, page_no, public).await?;
        let conn = self.system.conn().await;
        match &slug {
            Some(slug) => {
                conn.execute(
                    "INSERT OR REPLACE INTO wb_public_index (slug, session_id, page_no) VALUES (?1, ?2, ?3)",
                    Params::Positional(vec![Value::from(slug.clone()), Value::from(sid), Value::Integer(page_no)]),
                )
                .await?;
            }
            None => {
                conn.execute(
                    "DELETE FROM wb_public_index WHERE session_id = ?1 AND page_no = ?2",
                    Params::Positional(vec![Value::from(sid), Value::Integer(page_no)]),
                )
                .await?;
            }
        }
        Ok(slug)
    }

    pub async fn public_snapshot(&self, slug: &str) -> anyhow::Result<Option<PublicSnap>> {
        let sid = {
            let conn = self.system.conn().await;
            let mut rows = conn
                .query(
                    "SELECT session_id FROM wb_public_index WHERE slug = ?1",
                    Params::Positional(vec![Value::from(slug)]),
                )
                .await?;
            match rows.next().await? {
                Some(row) => match row.get_value(0)? {
                    Value::Text(s) => s,
                    _ => return Err(anyhow::anyhow!("invalid session_id in wb_public_index")),
                },
                None => return Ok(None),
            }
        };
        let pool = self.session(&sid).await?;
        pool.wb_public_snapshot(slug).await
    }

    pub async fn delete_public_by_page(&self, sid: &str, page_no: i64) -> anyhow::Result<()> {
        let pool = self.session(sid).await?;
        pool.wb_delete_public_by_page(sid, page_no).await?;
        let conn = self.system.conn().await;
        conn.execute(
            "DELETE FROM wb_public_index WHERE session_id = ?1 AND page_no = ?2",
            Params::Positional(vec![Value::from(sid), Value::Integer(page_no)]),
        )
        .await?;
        Ok(())
    }

    pub async fn hook_urls(&self, board_id: &str) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let pool = self.board_pool(board_id).await?;
        pool.srv_hook_urls(board_id).await
    }

    pub async fn hook_enqueue(&self, board_id: &str, url: &str, event: &str) -> anyhow::Result<i64> {
        self.system.srv_hook_enqueue(board_id, url, event).await
    }

    pub async fn hook_reschedule(&self, board_id: &str, owner: &str, url: &str) -> anyhow::Result<i64> {
        self.ensure_board_owned(board_id, owner).await?;
        self.system.srv_hook_reschedule(board_id, owner, url).await
    }

    pub async fn hook_deliveries(&self, board_id: &str, owner: &str, url: Option<&str>) -> anyhow::Result<Vec<SrvDelivery>> {
        self.ensure_board_owned(board_id, owner).await?;
        self.system.srv_hook_deliveries(board_id, owner, url).await
    }

    pub async fn ttl_sweep_all(&self) -> anyhow::Result<u64> {
        let mut total = 0u64;
        for sid in self.all_session_ids().await? {
            let pool = self.session(&sid).await?;
            total += pool.srv_ttl_sweep().await?;
        }
        Ok(total)
    }
}
