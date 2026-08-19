use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::types::{ChannelMessage, StreamEvent};

#[async_trait]
pub trait McpToolProvider: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    async fn call(&self, args: Value) -> anyhow::Result<ToolOutput>;
}

pub struct ToolOutput {
    pub text: String,
}

impl ToolOutput {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(self: Box<Self>, ctx: ChannelContext) -> anyhow::Result<()>;
}

pub struct ChannelContext {
    pub registry: ChannelRegistryHandle,
    pub zeroclaw: ZeroClawHandle,
}

impl ChannelContext {
    pub fn new(registry: ChannelRegistryHandle, zeroclaw: ZeroClawHandle) -> Self {
        Self { registry, zeroclaw }
    }
}

#[derive(Clone)]
pub struct ChannelRegistryHandle {
    tx: mpsc::Sender<(ChannelMessage, oneshot::Sender<anyhow::Result<String>>, Option<mpsc::Sender<StreamEvent>>)>,
}

impl ChannelRegistryHandle {
    pub fn new(tx: mpsc::Sender<(ChannelMessage, oneshot::Sender<anyhow::Result<String>>, Option<mpsc::Sender<StreamEvent>>)>) -> Self {
        Self { tx }
    }

    pub async fn route_incoming(&self, msg: ChannelMessage) -> anyhow::Result<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send((msg, reply_tx, None))
            .await
            .map_err(|e| anyhow::anyhow!("channel registry disconnected: {}", e))?;
        reply_rx
            .await
            .map_err(|e| anyhow::anyhow!("channel reply cancelled: {}", e))?
    }

    pub async fn route_incoming_stream(
        &self,
        msg: ChannelMessage,
        event_tx: mpsc::Sender<StreamEvent>,
    ) -> anyhow::Result<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send((msg, reply_tx, Some(event_tx)))
            .await
            .map_err(|e| anyhow::anyhow!("channel registry disconnected: {}", e))?;
        reply_rx
            .await
            .map_err(|e| anyhow::anyhow!("channel reply cancelled: {}", e))?
    }
}

#[derive(Clone)]
pub struct ZeroClawHandle {
    tx: mpsc::Sender<ZeroClawCommand>,
}

impl ZeroClawHandle {
    pub fn new(tx: mpsc::Sender<ZeroClawCommand>) -> Self {
        Self { tx }
    }

    pub async fn prompt(&self, session_id: String, prompt: String) -> anyhow::Result<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ZeroClawCommand::Prompt { session_id, prompt, reply_tx })
            .await
            .map_err(|e| anyhow::anyhow!("zeroclaw bridge disconnected: {}", e))?;
        reply_rx
            .await
            .map_err(|e| anyhow::anyhow!("zeroclaw reply cancelled: {}", e))?
    }

    pub async fn prompt_stream(
        &self,
        session_id: String,
        prompt: String,
        event_tx: mpsc::Sender<StreamEvent>,
    ) -> anyhow::Result<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ZeroClawCommand::PromptStream { session_id, prompt, event_tx, reply_tx })
            .await
            .map_err(|e| anyhow::anyhow!("zeroclaw bridge disconnected: {}", e))?;
        reply_rx
            .await
            .map_err(|e| anyhow::anyhow!("zeroclaw reply cancelled: {}", e))?
    }

    pub async fn get_session_messages(&self, session_id: String, limit: Option<usize>) -> anyhow::Result<crate::types::SessionMessagesResult> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ZeroClawCommand::GetSessionMessages { session_id, limit, reply_tx })
            .await
            .map_err(|e| anyhow::anyhow!("zeroclaw bridge disconnected: {}", e))?;
        reply_rx
            .await
            .map_err(|e| anyhow::anyhow!("zeroclaw reply cancelled: {}", e))?
    }

    pub async fn new_session(&self, session_id: String, agent_alias: String) -> anyhow::Result<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ZeroClawCommand::NewSession { session_id, agent_alias, reply_tx })
            .await
            .map_err(|e| anyhow::anyhow!("zeroclaw bridge disconnected: {}", e))?;
        reply_rx
            .await
            .map_err(|e| anyhow::anyhow!("zeroclaw reply cancelled: {}", e))?
    }

    pub async fn cancel_session(&self, session_id: String) -> anyhow::Result<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ZeroClawCommand::Cancel { session_id, reply_tx })
            .await
            .map_err(|e| anyhow::anyhow!("zeroclaw bridge disconnected: {}", e))?;
        reply_rx
            .await
            .map_err(|e| anyhow::anyhow!("zeroclaw reply cancelled: {}", e))?
    }
}

pub enum ZeroClawCommand {
    Prompt {
        session_id: String,
        prompt: String,
        reply_tx: oneshot::Sender<anyhow::Result<String>>,
    },
    NewSession {
        session_id: String,
        agent_alias: String,
        reply_tx: oneshot::Sender<anyhow::Result<String>>,
    },
    PromptStream {
        session_id: String,
        prompt: String,
        event_tx: mpsc::Sender<StreamEvent>,
        reply_tx: oneshot::Sender<anyhow::Result<String>>,
    },
    GetSessionMessages {
        session_id: String,
        limit: Option<usize>,
        reply_tx: oneshot::Sender<anyhow::Result<crate::types::SessionMessagesResult>>,
    },
    Cancel {
        session_id: String,
        reply_tx: oneshot::Sender<anyhow::Result<bool>>,
    },
}
