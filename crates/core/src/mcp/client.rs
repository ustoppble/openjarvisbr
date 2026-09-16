//! Sessão MCP de um servidor: handshake `initialize` +
//! `notifications/initialized`, `tools/list` e `tools/call`, com reconexão
//! simples (derruba a conexão e refaz o handshake uma vez).

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::transport::Transport;
use super::{McpError, McpServerConfig};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Tool como o servidor a descreve em `tools/list`.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct McpClient {
    config: McpServerConfig,
    transport: Mutex<Option<Transport>>,
    next_id: AtomicU64,
}

impl McpClient {
    /// Não conecta ainda: a conexão abre na primeira requisição.
    pub fn new(config: McpServerConfig) -> Self {
        Self {
            config,
            transport: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Todas as tools do servidor (segue `nextCursor`).
    pub async fn list_tools(&self) -> Result<Vec<RemoteTool>, McpError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let result = self.rpc("tools/list", params).await?;
            let page = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| McpError::Protocol("tools/list sem 'tools'".to_string()))?;
            tools.extend(page.iter().filter_map(parse_tool));
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            if cursor.is_none() {
                return Ok(tools);
            }
        }
    }

    /// `tools/call` e devolve o `result` cru (com `content`, `isError` etc.).
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        let arguments = if arguments.is_null() {
            json!({})
        } else {
            arguments
        };
        self.rpc(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let mut guard = self.transport.lock().await;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let result = match guard.as_mut() {
                Some(transport) => self.send(transport, method, params.clone()).await,
                None => match self.connect().await {
                    Ok(mut transport) => {
                        let result = self.send(&mut transport, method, params.clone()).await;
                        *guard = Some(transport);
                        result
                    }
                    Err(err) => Err(err),
                },
            };
            match result {
                Err(err) if err.is_retryable() => {
                    // Conexão/sessão perdida: próxima tentativa refaz o handshake.
                    *guard = None;
                    // Um tools/call pode já ter executado; só repete quando a
                    // sessão expirou (404), que garante que não rodou.
                    let resend = method != "tools/call" || matches!(err, McpError::Http(404));
                    if attempt >= 2 || !resend {
                        return Err(err);
                    }
                    tracing::debug!(server = %self.config.name, error = %err, "MCP reconectando");
                }
                other => return other,
            }
        }
    }

    async fn connect(&self) -> Result<Transport, McpError> {
        let mut transport = Transport::open(&self.config)?;
        let result = self
            .send(
                &mut transport,
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "openjarvisbr", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        let version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(PROTOCOL_VERSION);
        transport.set_protocol_version(version);
        transport
            .notify(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await?;
        Ok(transport)
    }

    async fn send(
        &self,
        transport: &mut Transport,
        method: &str,
        params: Value,
    ) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let response = transport.request(&message, id).await?;
        if let Some(error) = response.get("error") {
            return Err(McpError::Rpc {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("sem mensagem")
                    .to_string(),
            });
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| McpError::Protocol(format!("{method} sem 'result'")))
    }
}

fn parse_tool(value: &Value) -> Option<RemoteTool> {
    Some(RemoteTool {
        name: value.get("name")?.as_str()?.to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        input_schema: value
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
    })
}
