use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use serde_json::Value;
use tokio::sync::mpsc;
use zw_core::{ZeroClawHandle, ZeroClawCommand, StreamEvent, SessionMessagesResult};
use zw_db::DbPool;

pub struct ZeroClawBridge {
    socket_path: String,
    cmd_tx: tokio::sync::mpsc::Sender<ZeroClawCommand>,
    cmd_rx: Option<tokio::sync::mpsc::Receiver<ZeroClawCommand>>,
    db: DbPool,
    whiteboard: Option<zw_whiteboard::WhiteboardHandle>,
}

impl Clone for ZeroClawBridge {
    fn clone(&self) -> Self {
        Self {
            socket_path: self.socket_path.clone(),
            cmd_tx: self.cmd_tx.clone(),
            cmd_rx: None,
            db: self.db.clone(),
            whiteboard: self.whiteboard.clone(),
        }
    }
}

struct ZeroClawConnection {
    writer: OwnedWriteHalf,
    reader: BufReader<OwnedReadHalf>,
    next_id: u64,
}

impl ZeroClawBridge {
    pub fn new(socket_path: &str, db: DbPool) -> Self {
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(64);
        Self {
            socket_path: socket_path.to_string(),
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            db,
            whiteboard: None,
        }
    }

    pub fn set_whiteboard(&mut self, handle: zw_whiteboard::WhiteboardHandle) {
        self.whiteboard = Some(handle);
    }

    pub fn handle(&self) -> ZeroClawHandle {
        ZeroClawHandle::new(self.cmd_tx.clone())
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let socket_path = self.socket_path.clone();
        let mut cmd_rx = self.cmd_rx.take().expect("cmd_rx already taken");

        // Process commands from the handle — connect per command (resilient to daemon restart)
        while let Some(cmd) = cmd_rx.recv().await {
            let socket_path = socket_path.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::exec_command(socket_path, cmd).await {
                    tracing::error!("ZeroClaw command failed: {:#}", e);
                }
            });
        }
        Ok(())
    }

    async fn exec_command(socket_path: String, cmd: ZeroClawCommand) -> anyhow::Result<()> {
        let mut conn = Self::connect_to(&socket_path).await?;
        match cmd {
            ZeroClawCommand::Prompt { session_id, prompt, reply_tx } => {
                let result = conn.prompt(&session_id, &prompt).await;
                let _ = reply_tx.send(result);
            }
            ZeroClawCommand::PromptStream { session_id, prompt, event_tx, reply_tx } => {
                let result = conn.prompt_stream(&session_id, &prompt, &event_tx).await;
                let _ = reply_tx.send(result);
            }
            ZeroClawCommand::NewSession { session_id, agent_alias, reply_tx } => {
                let result = conn.new_session(&session_id, &agent_alias).await.map(|s| s);
                let _ = reply_tx.send(result);
            }
            ZeroClawCommand::GetSessionMessages { session_id, limit, reply_tx } => {
                let result = conn.get_session_messages(&session_id, limit).await;
                let _ = reply_tx.send(result);
            }
            ZeroClawCommand::Cancel { session_id, reply_tx } => {
                let result = conn.cancel_session(&session_id).await;
                let _ = reply_tx.send(result);
            }
        }
        Ok(())
    }

    async fn connect_to(socket_path: &str) -> anyhow::Result<ZeroClawConnection> {
        let stream = UnixStream::connect(socket_path)
            .await
            .map_err(|e| anyhow::anyhow!("connect to ZeroClaw socket {}: {}", socket_path, e))?;
        let (reader, writer) = stream.into_split();
        let mut conn = ZeroClawConnection {
            writer,
            reader: BufReader::new(reader),
            next_id: 0,
        };
        conn.send_request("initialize", serde_json::json!({"protocol_version": 1})).await?;
        Ok(conn)
    }

    async fn connect(&self) -> anyhow::Result<ZeroClawConnection> {
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| anyhow::anyhow!("connect to ZeroClaw socket {}: {}", self.socket_path, e))?;
        let (reader, writer) = stream.into_split();
        let mut conn = ZeroClawConnection {
            writer,
            reader: BufReader::new(reader),
            next_id: 0,
        };
        conn.send_request("initialize", serde_json::json!({"protocol_version": 1})).await?;
        Ok(conn)
    }

    async fn format_prompt(&self, _user_id: &str, channel: &str, role: &str, attachments: &[zw_core::Attachment], prompt: &str) -> String {
        // Roles arrive as a comma-separated list (from the engine profiles
        // table); present them to the agent as an array. No owner/customer key.
        let roles: Vec<&str> = role.split(',').map(|r| r.trim()).filter(|r| !r.is_empty()).collect();
        let mut envelope = serde_json::json!({
            "message": prompt,
            "channel": channel,
            "roles": roles,
        });
        if !attachments.is_empty() {
            let attach: Vec<serde_json::Value> = attachments.iter().map(|a| {
                let mut entry = serde_json::json!({
                    "filename": a.filename,
                    "mime_type": a.mime_type,
                });
                if !a.path.is_empty() {
                    entry["path"] = serde_json::json!(a.path);
                }
                if !a.data.is_empty() {
                    entry["data"] = serde_json::json!(a.data);
                }
                entry
            }).collect();
            envelope["attachments"] = serde_json::json!(attach);
        }
        envelope.to_string()
    }

    pub async fn process_prompt(&self, session_id: &str, user_id: &str, agent_alias: &str, channel: &str, role: &str, attachments: &[zw_core::Attachment], prompt: &str) -> anyhow::Result<String> {
        let mut conn = self.connect().await?;
        let actual_session_id = conn.new_session(session_id, agent_alias).await?;
        if let Some(wb) = &self.whiteboard {
            if let Err(e) = wb.ensure(&actual_session_id).await {
                tracing::warn!("whiteboard ensure failed: {}", e);
            }
        }
        if let Err(e) = self.db.upsert_session(&actual_session_id, user_id, agent_alias, None, None, "web").await {
            tracing::warn!("Failed to save session to DB: {}", e);
        }
        let structured_prompt = self.format_prompt(user_id, channel, role, attachments, prompt).await;
        conn.prompt(&actual_session_id, &structured_prompt).await
    }

    pub async fn process_prompt_stream(
        &self,
        session_id: &str,
        user_id: &str,
        agent_alias: &str,
        channel: &str,
        role: &str,
        attachments: &[zw_core::Attachment],
        prompt: &str,
        event_tx: mpsc::Sender<StreamEvent>,
    ) -> anyhow::Result<String> {
        let mut conn = self.connect().await?;
        let actual_session_id = conn.new_session(session_id, agent_alias).await?;

        let _ = event_tx.send(StreamEvent::SessionCreated { session_id: actual_session_id.clone() }).await;

        if let Some(wb) = &self.whiteboard {
            if let Err(e) = wb.ensure(&actual_session_id).await {
                tracing::warn!("whiteboard ensure failed: {}", e);
            }
        }

        if let Err(e) = self.db.upsert_session(&actual_session_id, user_id, agent_alias, None, None, "web").await {
            tracing::warn!("Failed to save session to DB: {}", e);
        }
        let structured_prompt = self.format_prompt(user_id, channel, role, attachments, prompt).await;
        conn.prompt_stream(&actual_session_id, &structured_prompt, &event_tx).await
    }

    pub async fn cancel_session(&self, session_id: &str) -> anyhow::Result<bool> {
        let mut conn = self.connect().await?;
        conn.cancel_session(session_id).await
    }

    pub async fn get_session_messages(
        &self,
        session_id: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<SessionMessagesResult> {
        let mut conn = self.connect().await?;
        conn.get_session_messages(session_id, limit).await
    }
}

impl ZeroClawConnection {
    async fn new_session(&mut self, session_id: &str, agent_alias: &str) -> anyhow::Result<String> {
        let params = serde_json::json!({
            "agent_alias": agent_alias,
            "session_id": session_id,
        });
        let result = self.send_request("session/new", params).await?;
        let daemon_session_id = result.get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or(session_id)
            .to_string();
        Ok(daemon_session_id)
    }

    async fn prompt(&mut self, session_id: &str, prompt: &str) -> anyhow::Result<String> {
        let params = serde_json::json!({
            "session_id": session_id,
            "prompt": prompt,
        });

        self.next_id += 1;
        let req_id = self.next_id;
        let raw = serde_json::to_string(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": "session/prompt",
            "params": params,
        }))?;
        let mut line = raw;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;

        let mut full_text = String::new();
        loop {
            let mut buf = String::new();
            let n = self.reader.read_line(&mut buf).await?;
            if n == 0 {
                anyhow::bail!("ZeroClaw connection closed");
            }
            let trimmed = buf.trim();

            if let Ok(resp) = serde_json::from_str::<Value>(trimmed) {
                if resp.get("id").and_then(|id| id.as_u64()) == Some(req_id) {
                    if let Some(result) = resp.get("result") {
                        if let Some(content) = result.get("content").and_then(|c| c.as_str()) {
                            if !content.is_empty() {
                                full_text = content.to_string();
                            }
                        }
                    }
                    if let Some(err) = resp.get("error") {
                        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                        let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown");
                        anyhow::bail!("ZeroClaw error ({}): {}", code, message);
                    }
                    return Ok(full_text);
                }

                if resp.get("method").and_then(|m| m.as_str()) == Some("session/update") {
                    if let Some(params) = resp.get("params") {
                        if let Some(text) = params.get("text").and_then(|t| t.as_str()) {
                            full_text.push_str(text);
                        }
                        if let Some(event_type) = params.get("type").and_then(|t| t.as_str()) {
                            if event_type == "turn_complete" {
                                if let Some(content) = params.get("content").and_then(|c| c.as_str()) {
                                    if !content.is_empty() {
                                        full_text = content.to_string();
                                    }
                                }
                                return Ok(full_text);
                            }
                        }
                    }
                }
            }
        }
    }

    async fn prompt_stream(
        &mut self,
        session_id: &str,
        prompt: &str,
        event_tx: &mpsc::Sender<StreamEvent>,
    ) -> anyhow::Result<String> {
        let params = serde_json::json!({
            "session_id": session_id,
            "prompt": prompt,
        });

        self.next_id += 1;
        let req_id = self.next_id;
        let raw = serde_json::to_string(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": "session/prompt",
            "params": params,
        }))?;
        let mut line = raw;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;

        let mut full_text = String::new();
        loop {
            let mut buf = String::new();
            let n = self.reader.read_line(&mut buf).await?;
            if n == 0 {
                anyhow::bail!("ZeroClaw connection closed");
            }
            let trimmed = buf.trim();

            if let Ok(resp) = serde_json::from_str::<Value>(trimmed) {
                if resp.get("id").and_then(|id| id.as_u64()) == Some(req_id) {
                    if let Some(err) = resp.get("error") {
                        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                        let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown");
                        let _ = event_tx.send(StreamEvent::Error { message: format!("ZeroClaw error ({}): {}", code, message) }).await;
                        anyhow::bail!("ZeroClaw error ({}): {}", code, message);
                    }
                    if let Some(result) = resp.get("result") {
                        if let Some(content) = result.get("content").and_then(|c| c.as_str()) {
                            if !content.is_empty() {
                                full_text = content.to_string();
                                let _ = event_tx.send(StreamEvent::Done { content: content.to_string() }).await;
                            }
                        }
                    }
                    let _ = event_tx.send(StreamEvent::ChunkReset).await;
                    return Ok(full_text);
                }

                if resp.get("method").and_then(|m| m.as_str()) == Some("session/update") {
                    if let Some(params) = resp.get("params") {
                        let event_type = params.get("type").and_then(|t| t.as_str()).unwrap_or("");

                        match event_type {
                            "agent_thought_chunk" => {
                                if let Some(text) = params.get("text").and_then(|t| t.as_str()) {
                                    let _ = event_tx.send(StreamEvent::Thinking { content: text.to_string() }).await;
                                }
                            }
                            "agent_message_chunk" => {
                                if let Some(text) = params.get("text").and_then(|t| t.as_str()) {
                                    full_text.push_str(text);
                                    let _ = event_tx.send(StreamEvent::Chunk { content: text.to_string() }).await;
                                }
                            }
                            "tool_call" => {
                                let tool_call_id = params.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                                let args = params.get("raw_input").cloned().unwrap_or(Value::Null);
                                let _ = event_tx.send(StreamEvent::ToolCall { tool_call_id, name, args }).await;
                            }
                            "tool_result" => {
                                let tool_call_id = params.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                                let output = params.get("raw_output").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let _ = event_tx.send(StreamEvent::ToolResult { tool_call_id, name, output }).await;
                            }
                            "turn_complete" => {
                                let content = params.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                if !content.is_empty() {
                                    full_text = content.to_string();
                                }
                                let _ = event_tx.send(StreamEvent::ChunkReset).await;
                                let _ = event_tx.send(StreamEvent::Done { content: full_text.clone() }).await;
                                return Ok(full_text);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    async fn cancel_session(&mut self, session_id: &str) -> anyhow::Result<bool> {
        let params = serde_json::json!({
            "session_id": session_id,
        });
        let result = self.send_request("session/cancel", params).await?;
        Ok(result.get("cancelled").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    async fn get_session_messages(&mut self, session_id: &str, limit: Option<usize>) -> anyhow::Result<SessionMessagesResult> {
        let params = serde_json::json!({
            "session_id": session_id,
            "limit": limit.unwrap_or(100),
        });
        let result = self.send_request("session/messages", params).await?;
        let messages_result: SessionMessagesResult = serde_json::from_value(result)
            .map_err(|e| anyhow::anyhow!("failed to parse session messages: {}", e))?;
        Ok(messages_result)
    }

    async fn send_request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.next_id += 1;
        let req_id = self.next_id;
        let raw = serde_json::to_string(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": method,
            "params": params,
        }))?;
        let mut line = raw;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;

        loop {
            let mut buf = String::new();
            let n = self.reader.read_line(&mut buf).await?;
            if n == 0 {
                anyhow::bail!("ZeroClaw connection closed while waiting for {}", method);
            }
            let trimmed = buf.trim();

            if let Ok(resp) = serde_json::from_str::<Value>(trimmed) {
                if let Some(id) = resp.get("id").and_then(|id| id.as_u64()) {
                    if id == req_id {
                        if let Some(err) = resp.get("error") {
                            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                            let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown");
                            anyhow::bail!("ZeroClaw {} error ({}): {}", method, code, message);
                        }
                        if let Some(result) = resp.get("result") {
                            return Ok(result.clone());
                        }
                        return Ok(serde_json::json!({}));
                    }
                }
            }
        }
    }
}
