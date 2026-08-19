use std::time::Duration;
use tokio::time;
use zw_db::SessionStore;

pub fn spawn_ttl_sweeper(store: SessionStore, every: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            time::sleep(every).await;
            match store.ttl_sweep_all().await {
                Ok(deleted) => {
                    if deleted > 0 {
                        tracing::info!("ttl sweeper: deleted {deleted} expired records");
                    }
                }
                Err(e) => tracing::warn!("ttl sweeper: {e}"),
            }
        }
    })
}