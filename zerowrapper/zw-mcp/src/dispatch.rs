use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::RwLock;
use tracing::{debug, error, warn};

pub type McpProviders = Arc<RwLock<Vec<Box<dyn zw_core::McpToolProvider>>>>;

fn mcp_log(method: &str, msg: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/zw-mcp.log")
    {
        use std::io::Write;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{}] {} — {}", ts, method, msg);
    }
}

use crate::protocol::*;

/// Handle a single JSON-RPC MCP request. Returns `Some(response)` when a reply
/// is expected, or `None` for notifications (e.g. `notifications/initialized`).
pub async fn handle_request(providers: &McpProviders, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
    let method = req.method.as_str();
    let params = req.params.clone().unwrap_or_else(|| serde_json::json!({}));
    let req_id = req.id;

    match method {
        "initialize" => {
            mcp_log("initialize", &format!("received from id={:?}", req_id));
            let result = serde_json::json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "zeroRapper",
                    "version": "0.1.0"
                }
            });
            Some(JsonRpcResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id: req_id,
                result: Some(result),
                error: None,
            })
        }
        "notifications/initialized" => {
            mcp_log("initialized", "client initialized");
            debug!("MCP client initialized");
            None
        }
        "tools/list" => {
            mcp_log("tools/list", "received");
            let lock = providers.read().await;
            let tools: Vec<McpToolDef> = lock.iter().map(|p| McpToolDef {
                name: p.name().to_string(),
                description: Some(p.description().to_string()),
                input_schema: p.input_schema(),
            }).collect();
            let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
            mcp_log("tools/list", &format!("returning {} tool(s): {:?}", tool_names.len(), tool_names));
            drop(lock);
            let result = serde_json::json!({ "tools": tools });
            Some(JsonRpcResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id: req_id,
                result: Some(result),
                error: None,
            })
        }
        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            mcp_log("tools/call", &format!("name={}", tool_name));
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| serde_json::json!({}));

            let lock = providers.read().await;
            let provider = lock.iter().find(|p| p.name() == tool_name);
            match provider {
                Some(provider) => {
                    match provider.call(arguments).await {
                        Ok(output) => {
                            let result = serde_json::json!({
                                "content": [{
                                    "type": "text",
                                    "text": output.text
                                }]
                            });
                            drop(lock);
                            Some(JsonRpcResponse {
                                jsonrpc: JSONRPC_VERSION.to_string(),
                                id: req_id,
                                result: Some(result),
                                error: None,
                            })
                        }
                        Err(e) => {
                            drop(lock);
                            let err_msg = format!("{}", e);
                            error!("MCP tool call error: {}", err_msg);
                            Some(JsonRpcResponse {
                                jsonrpc: JSONRPC_VERSION.to_string(),
                                id: req_id,
                                result: Some(serde_json::json!({
                                    "isError": true,
                                    "content": [{
                                        "type": "text",
                                        "text": err_msg
                                    }]
                                })),
                                error: None,
                            })
                        }
                    }
                }
                None => {
                    drop(lock);
                    Some(JsonRpcResponse {
                        jsonrpc: JSONRPC_VERSION.to_string(),
                        id: req_id,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32601,
                            message: format!("Method not found: tools/call for '{}'", tool_name),
                            data: None,
                        }),
                    })
                }
            }
        }
        _ => {
            Some(JsonRpcResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id: req_id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", method),
                    data: None,
                }),
            })
        }
    }
}

pub async fn run_stdio_loop(providers: McpProviders) -> anyhow::Result<()> {
    mcp_log("startup", "MCP stdio loop started, waiting for requests...");
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                warn!("Failed to parse MCP request: {}", e);
                let resp = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: None,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                        data: None,
                    }),
                };
                write_response(&resp).await?;
                continue;
            }
        };

        if let Some(resp) = handle_request(&providers, req).await {
            write_response(&resp).await?;
        }
    }

    debug!("MCP stdin closed, shutting down");
    Ok(())
}

async fn write_response(resp: &JsonRpcResponse) -> anyhow::Result<()> {
    let mut line = serde_json::to_string(resp)?;
    line.push('\n');
    let mut stdout = tokio::io::stdout();
    stdout.write_all(line.as_bytes()).await?;
    stdout.flush().await?;
    Ok(())
}
