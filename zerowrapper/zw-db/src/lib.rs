pub mod audit;
pub mod calls;
pub mod cron;
pub mod filters;
pub mod hook_deliveries;
pub mod hooks;
pub mod jobs;
pub mod keys;
pub mod links;
pub mod logic;
pub mod migrations;
pub mod netguard;
pub mod rate;
pub mod recipes;
pub mod secrets;
pub mod serverless;
pub mod sessions;
pub mod session_store;
pub mod templates;
pub mod ttl;
pub mod users;
pub mod whiteboard;

pub use session_store::SessionStore;
pub use users::AccountSeed;

use std::sync::Arc;
use turso::{Connection, Database};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct DbPool {
    conn: Arc<Mutex<Connection>>,
    _db: Arc<Database>,
}

impl DbPool {
    pub async fn open(db_path: &str) -> anyhow::Result<Self> {
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let db = turso::Builder::new_local(db_path)
            .experimental_index_method(true)
            .build()
            .await?;
        let conn = db.connect()?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            _db: Arc::new(db),
        })
    }

    pub async fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute_batch(migrations::SYSTEM_SCHEMA).await?;
        for migration in migrations::SYSTEM_MIGRATIONS {
            let _ = conn.execute_batch(migration).await;
        }
        Ok(())
    }

    pub async fn migrate_session(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute_batch(migrations::SESSION_SCHEMA).await?;
        for migration in migrations::SESSION_MIGRATIONS {
            let _ = conn.execute_batch(migration).await;
        }
        Ok(())
    }

    pub async fn conn(&self) -> tokio::sync::MutexGuard<'_, Connection> {
        self.conn.lock().await
    }

    pub async fn seed_accounts(&self, accounts: &std::collections::HashMap<String, crate::users::AccountSeed>) -> anyhow::Result<()> {
        for (phone, account) in accounts {
            self.upsert_user(phone, Some(&account.role), None).await?;
            self.set_user_password(phone, &account.password).await?;
            self.set_user_role(phone, &account.role).await?;
            tracing::info!("Seeded account: {} role={}", phone, account.role);
        }
        Ok(())
    }
}
