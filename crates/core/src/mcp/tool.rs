//! Adaptador: cada tool remota vira uma `Tool` do contrato, chamada
//! `mcp.<server>.<tool>`, com o JSON Schema original do servidor.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::client::{McpClient, RemoteTool};
use crate::tools::policy::mcp_risk;
use crate::tools::{Tool, ToolError, ToolSpec};

pub struct McpTool {
    client: Arc<McpClient>,
    remote: RemoteTool,
}

impl McpTool {
    pub fn new(client: Arc<McpClient>, remote: RemoteTool) -> Self {
        Self { client, remote }
    }
}

/// `mcp.<server>.<tool>`.
pub fn tool_name(server: &str, tool: &str) -> String {
    format!("mcp.{server}.{tool}")
}

/// Resultado de `tools/call` como valor para o modelo: `structuredContent`
/// quando houver; senão o texto do `content` (JSON quando o texto for JSON).
/// `isError` vira erro com o texto devolvido pelo servidor.
fn to_output(result: Value) -> Result<Value, ToolError> {
    let text: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    let text = text.join("\n");
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(ToolError::Failed(if text.is_empty() {
            "o servidor MCP devolveu erro".to_string()
        } else {
            text
        }));
    }
    if let Some(structured) = result.get("structuredContent") {
        return Ok(structured.clone());
    }
    if text.is_empty() {
        return Ok(result.get("content").cloned().unwrap_or(Value::Null));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: tool_name(self.client.name(), &self.remote.name),
            description: self.remote.description.clone(),
            parameters: self.remote.input_schema.clone(),
            // Mesma heurística da política: leitura = Safe, resto = Confirm.
            risk: mcp_risk(&self.remote.name),
        }
    }

    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        let result = self
            .client
            .call_tool(&self.remote.name, args)
            .await
            .map_err(|err| ToolError::Failed(format!("{}: {err}", self.client.name())))?;
        to_output(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpServerConfig;
    use crate::tools::Risk;
    use serde_json::json;

    #[test]
    fn output_prefers_structured_then_json_text() {
        let structured =
            json!({"structuredContent": {"a": 1}, "content": [{"type": "text", "text": "x"}]});
        assert_eq!(to_output(structured).unwrap(), json!({"a": 1}));
        let json_text = json!({"content": [{"type": "text", "text": "{\"b\":2}"}]});
        assert_eq!(to_output(json_text).unwrap(), json!({"b": 2}));
        let plain = json!({"content": [{"type": "text", "text": "oi"}]});
        assert_eq!(to_output(plain).unwrap(), json!("oi"));
        let err = json!({"isError": true, "content": [{"type": "text", "text": "falhou"}]});
        assert_eq!(
            to_output(err).unwrap_err().to_string(),
            "falha na execução: falhou"
        );
    }

    /// Servidor stdio falso em `sh`: responde initialize, ignora a
    /// notificação, lista uma tool e responde o call.
    #[cfg(unix)]
    #[tokio::test]
    async fn stdio_server_end_to_end() {
        let script = r#"
read l; printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/message","params":{}}'
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"0"}}}'
read l
read l; printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo_get","description":"eco","inputSchema":{"type":"object","properties":{"x":{"type":"string"}}}}]}}'
read l; printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"{\"eco\":\"ok\"}"}]}}'
read l
"#;
        let config = McpServerConfig {
            name: "fake".to_string(),
            command: Some("sh".to_string()),
            args: vec!["-c".to_string(), script.to_string()],
            ..Default::default()
        };
        let tools = crate::mcp::load_all(&[config]).await;
        assert_eq!(tools.len(), 1);
        let spec = tools[0].spec();
        assert_eq!(spec.name, "mcp.fake.echo_get");
        assert_eq!(spec.risk, Risk::Safe);
        assert_eq!(spec.parameters["properties"]["x"]["type"], json!("string"));
        let out = tools[0].call(json!({"x": "ok"})).await.unwrap();
        assert_eq!(out, json!({"eco": "ok"}));
    }
}
