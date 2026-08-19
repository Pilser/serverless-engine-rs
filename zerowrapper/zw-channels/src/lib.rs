pub mod router;
pub mod zeroclaw_bridge;

pub use zeroclaw_bridge::ZeroClawBridge;

use tokio::sync::{mpsc, oneshot};
use zw_core::{Channel, ChannelContext, ChannelRegistryHandle, ZeroClawHandle, ChannelMessage, StreamEvent};

type RegistryMsg = (ChannelMessage, oneshot::Sender<anyhow::Result<String>>, Option<mpsc::Sender<StreamEvent>>);

pub struct ChannelRegistry {
    channels: Vec<Box<dyn Channel>>,
    ctx_tx: mpsc::Sender<RegistryMsg>,
    ctx_rx: Option<mpsc::Receiver<RegistryMsg>>,
    zeroclaw: ZeroClawBridge,
}

impl ChannelRegistry {
    pub fn new(zeroclaw: ZeroClawBridge) -> Self {
        let (tx, rx) = mpsc::channel::<RegistryMsg>(1024);
        Self {
            channels: Vec::new(),
            ctx_tx: tx,
            ctx_rx: Some(rx),
            zeroclaw,
        }
    }

    pub fn register(&mut self, channel: Box<dyn Channel>) {
        tracing::info!("Registering channel: {}", channel.name());
        self.channels.push(channel);
    }

    pub fn registry_handle(&self) -> ChannelRegistryHandle {
        ChannelRegistryHandle::new(self.ctx_tx.clone())
    }

    pub fn zeroclaw_handle(&self) -> ZeroClawHandle {
        self.zeroclaw.handle()
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let registry_handle = self.registry_handle();
        let zeroclaw_handle = self.zeroclaw_handle();

        for channel in self.channels {
            let ctx = ChannelContext::new(registry_handle.clone(), zeroclaw_handle.clone());
            tokio::spawn(async move {
                if let Err(e) = channel.start(ctx).await {
                    tracing::error!("Channel error: {}", e);
                }
            });
        }

        let mut rx = self.ctx_rx.take().expect("ctx_rx already taken");
        let zc = self.zeroclaw.clone();

        while let Some((msg, reply_tx, event_tx)) = rx.recv().await {
            let zc = zc.clone();
            tokio::spawn(async move {
                let result = router::route_incoming(zc, msg, event_tx).await;
                let _ = reply_tx.send(result);
            });
        }

        Ok(())
    }
}
