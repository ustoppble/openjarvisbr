//! Cliente MCP (Model Context Protocol): conecta nos servidores de
//! `[[mcp_servers]]` do config.toml — por HTTP streamable ou stdio — e expõe
//! cada tool remota como uma `Tool` do contrato, com nome
//! `mcp.<server>.<tool>` e o JSON Schema original.
//!
//! Tokens só entram por nome de env (`bearer_env`) ou pelo nome da entrada no
//! Keychain/Credential Manager (`token_keychain`): o valor é lido na hora da
//! requisição e nunca aparece em log, erro, config ou resultado.

pub mod client;
pub mod discovery;
pub mod tool;
pub mod transport;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tools::Tool;

pub use client::McpClient;
pub use tool::McpTool;

/// Tempo máximo para conectar e listar as tools de um servidor no
/// carregamento; passou disso, o servidor é ignorado com aviso.
const LOAD_TIMEOUT: Duration = Duration::from_secs(15);

/// Um servidor de `[[mcp_servers]]`. Exatamente um de `url` (HTTP streamable)
/// ou `command` (stdio) deve estar preenchido.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    /// Endpoint HTTP streamable; `${VAR}` é expandido do ambiente.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Executável do transporte stdio; `${VAR}` é expandido do ambiente.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Variáveis extras para o processo stdio (valores com `${VAR}`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    /// NOME da env com o bearer token (nunca o valor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_env: Option<String>,
    /// Nome da entrada no Keychain (macOS) / Credential Manager (Windows)
    /// com o bearer token, gravada pela aba Ferramentas (JRV-58). Usado só
    /// Tem prioridade sobre `bearer_env` (JRV-68).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_keychain: Option<String>,
    /// Descoberta automática (`"overclock"` / `"overclick"`): no start do
    /// Engine e no botão Conectar, URL e token são relidos do Overclock
    /// (ver `discovery`). Ausente, vale pelo nome do servidor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<String>,
}

/// Serviço das entradas de token MCP no Keychain/Credential Manager.
pub const KEYCHAIN_SERVICE: &str = "openjarvisbr-mcp";

fn keychain_entry(name: &str) -> Result<keyring::Entry, McpError> {
    keyring::Entry::new(KEYCHAIN_SERVICE, name)
        .map_err(|_| McpError::Keychain(name.to_string()))
}

/// Token guardado no Keychain para `name`; ausente ou ilegível é erro (só
/// com o nome, nunca o valor).
pub fn keychain_token(name: &str) -> Result<String, McpError> {
    keychain_entry(name)?
        .get_password()
        .ok()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| McpError::Keychain(name.to_string()))
}

/// Token do servidor, na ordem Keychain → `bearer_env` → erro. `keychain` e
/// `env` são as consultas (injetáveis nos testes); devolve o token e a
/// origem (nome da entrada ou da env), nunca loga o valor.
pub fn resolve_token(
    server_name: &str,
    token_keychain: Option<&str>,
    bearer_env: Option<&str>,
    keychain: impl Fn(&str) -> Option<String>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<(String, String)>, McpError> {
    if token_keychain.is_none() && bearer_env.is_none() {
        return Ok(None);
    }
    let from_keychain = token_keychain
        .and_then(|name| keychain(name).filter(|t| !t.is_empty()).map(|t| (t, name.to_string())));
    if let Some(found) = from_keychain {
        return Ok(Some(found));
    }
    let from_env = bearer_env
        .and_then(|name| env(name).filter(|t| !t.is_empty()).map(|t| (t, name.to_string())));
    if let Some(found) = from_env {
        return Ok(Some(found));
    }
    Err(match (token_keychain, bearer_env) {
        (Some(_), Some(env)) => McpError::NoToken {
            server: server_name.to_string(),
            env: env.to_string(),
        },
        (None, Some(env)) => McpError::MissingEnv(env.to_string()),
        _ => McpError::Keychain(token_keychain.unwrap_or(server_name).to_string()),
    })
}

/// Grava (ou troca) o token de `name` no Keychain.
pub fn set_keychain_token(name: &str, token: &str) -> Result<(), McpError> {
    keychain_entry(name)?
        .set_password(token)
        .map_err(|_| McpError::Keychain(name.to_string()))
}

/// Apaga o token de `name`; entrada inexistente não é erro.
pub fn delete_keychain_token(name: &str) -> Result<(), McpError> {
    match keychain_entry(name)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(McpError::Keychain(name.to_string())),
    }
}

/// Falhas do cliente MCP. Mensagens nunca carregam token nem endereço.
#[derive(Debug, Error)]
pub enum McpError {
    #[error("servidor MCP '{0}' sem url nem command")]
    NoTransport(String),
    #[error("variável de ambiente ausente: {0}")]
    MissingEnv(String),
    #[error("token ausente no Keychain: {0}")]
    Keychain(String),
    #[error("sem token para '{server}': nem no Keychain nem na env {env} — use Conectar na aba Ferramentas")]
    NoToken { server: String, env: String },
    #[error("{0}")]
    Discovery(String),
    #[error("conexão recusada")]
    Refused,
    #[error("não autorizado (HTTP {0})")]
    Unauthorized(u16),
    #[error("HTTP {0}")]
    Http(u16),
    #[error("falha de transporte: {0}")]
    Transport(String),
    #[error("resposta inválida: {0}")]
    Protocol(String),
    #[error("erro JSON-RPC {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("tempo esgotado")]
    Timeout,
}

impl McpError {
    /// Erros em que vale derrubar a conexão e tentar de novo uma vez.
    pub(crate) fn is_retryable(&self) -> bool {
        matches!(
            self,
            McpError::Transport(_) | McpError::Http(404) | McpError::Timeout
        )
    }
}

/// Expande `${VAR}` com o ambiente. Variável ausente é erro (com o nome, sem
/// valor); `$` solto é mantido literal.
pub fn expand_env(input: &str) -> Result<String, McpError> {
    expand_with(input, |name| std::env::var(name).ok())
}

fn expand_with(input: &str, lookup: impl Fn(&str) -> Option<String>) -> Result<String, McpError> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let name = &after[..end];
        let value = lookup(name).ok_or_else(|| McpError::MissingEnv(name.to_string()))?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Conecta em todos os servidores e devolve suas tools. Servidor fora do ar,
/// sem env ou com erro vira um aviso e é pulado — nunca derruba o resto.
pub async fn load_all(servers: &[McpServerConfig]) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    for server in servers {
        let client = Arc::new(McpClient::new(server.clone()));
        match tokio::time::timeout(LOAD_TIMEOUT, client.list_tools()).await {
            Ok(Ok(remote)) => {
                tracing::info!(server = %server.name, tools = remote.len(), "MCP conectado");
                tools.extend(
                    remote
                        .into_iter()
                        .map(|info| Box::new(McpTool::new(client.clone(), info)) as Box<dyn Tool>),
                );
            }
            Ok(Err(err)) => {
                tracing::warn!(server = %server.name, error = %err, "MCP indisponível, seguindo sem ele");
            }
            Err(_) => {
                tracing::warn!(server = %server.name, "MCP não respondeu a tempo, seguindo sem ele");
            }
        }
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(name: &str) -> Option<String> {
        match name {
            "PORT" => Some("4242".to_string()),
            _ => None,
        }
    }

    #[test]
    fn expands_vars_in_url() {
        assert_eq!(
            expand_with("http://127.0.0.1:${PORT}/mcp", lookup).unwrap(),
            "http://127.0.0.1:4242/mcp"
        );
        assert_eq!(expand_with("sem vars $X", lookup).unwrap(), "sem vars $X");
        assert_eq!(
            expand_with("aberto ${PORT", lookup).unwrap(),
            "aberto ${PORT"
        );
    }

    #[test]
    fn token_vem_do_keychain_antes_da_env() {
        let keychain = |name: &str| (name == "overclock").then(|| "do-keychain".to_string());
        let env = |name: &str| (name == "ENV_T").then(|| "da-env".to_string());
        let none = |_: &str| None;
        let got = resolve_token("overclock", Some("overclock"), Some("ENV_T"), keychain, env).unwrap();
        assert_eq!(got, Some(("do-keychain".to_string(), "overclock".to_string())));
        let got = resolve_token("x", Some("vazio"), Some("ENV_T"), keychain, env).unwrap();
        assert_eq!(got, Some(("da-env".to_string(), "ENV_T".to_string())));
        assert_eq!(resolve_token("x", None, None, none, none).unwrap(), None);
        let err = resolve_token("x", Some("vazio"), Some("ENV_T"), none, none).unwrap_err();
        assert_eq!(
            err.to_string(),
            "sem token para 'x': nem no Keychain nem na env ENV_T — use Conectar na aba Ferramentas"
        );
        let err = resolve_token("x", None, Some("ENV_T"), keychain, none).unwrap_err();
        assert_eq!(err.to_string(), "variável de ambiente ausente: ENV_T");
        let err = resolve_token("x", Some("k"), None, none, env).unwrap_err();
        assert_eq!(err.to_string(), "token ausente no Keychain: k");
    }

    #[test]
    fn missing_var_reports_only_the_name() {
        let err = expand_with("${NAO_EXISTE}", lookup).unwrap_err();
        assert_eq!(err.to_string(), "variável de ambiente ausente: NAO_EXISTE");
    }

    #[test]
    fn parses_mcp_servers_from_toml() {
        #[derive(Deserialize)]
        struct File {
            mcp_servers: Vec<McpServerConfig>,
        }
        let file: File = toml::from_str(
            r#"
[[mcp_servers]]
name = "overclock"
url = "http://127.0.0.1:${OVERCLOCK_MCP_PORT}/mcp"
bearer_env = "OVERCLOCK_MCP_BEARER_TOKEN"

[[mcp_servers]]
name = "local"
command = "node"
args = ["server.js"]
"#,
        )
        .unwrap();
        assert_eq!(file.mcp_servers.len(), 2);
        assert_eq!(
            file.mcp_servers[0].bearer_env.as_deref(),
            Some("OVERCLOCK_MCP_BEARER_TOKEN")
        );
        assert_eq!(file.mcp_servers[1].command.as_deref(), Some("node"));
        assert_eq!(file.mcp_servers[1].args, vec!["server.js".to_string()]);
    }

    #[tokio::test]
    async fn load_all_skips_unreachable_server() {
        let servers = vec![
            McpServerConfig {
                name: "sem_transporte".to_string(),
                ..Default::default()
            },
            McpServerConfig {
                name: "fora".to_string(),
                url: Some("http://127.0.0.1:9/mcp".to_string()),
                ..Default::default()
            },
        ];
        assert!(load_all(&servers).await.is_empty());
    }

    /// Keychain real: `cargo test -p openjarvisbr-core keychain -- --ignored`.
    #[test]
    #[ignore = "grava e apaga uma entrada de teste no Keychain do usuário"]
    fn keychain_roundtrip() {
        let name = "openjarvisbr-teste-roundtrip";
        set_keychain_token(name, "valor-de-teste").unwrap();
        assert_eq!(keychain_token(name).unwrap(), "valor-de-teste");
        delete_keychain_token(name).unwrap();
        assert!(keychain_token(name).is_err());
        delete_keychain_token(name).unwrap();
    }

    /// Servidores reais: `cargo test -p openjarvisbr-core mcp -- --ignored`.
    /// Precisa de OVERCLOCK_MCP_PORT/OVERCLOCK_MCP_BEARER_TOKEN (Overclock
    /// local) e OVERCLICK_MCP_BEARER_TOKEN (OverClick).
    async fn call_live(server: McpServerConfig, tool: &str) {
        let name = server.name.clone();
        let tools = load_all(&[server]).await;
        let names: Vec<String> = tools.iter().map(|t| t.spec().name).collect();
        println!("{name}: {} tools", names.len());
        let wanted = format!("mcp.{name}.{tool}");
        let found = tools
            .iter()
            .find(|t| t.spec().name == wanted)
            .unwrap_or_else(|| panic!("{wanted} não listada"));
        let output = found.call(serde_json::json!({})).await.expect("tools/call");
        assert!(!output.is_null());
        println!("{wanted}: ok ({} bytes)", output.to_string().len());
    }

    #[tokio::test]
    #[ignore = "precisa do Overclock local e de OVERCLOCK_MCP_BEARER_TOKEN"]
    async fn mcp_live_overclock_pane_list() {
        call_live(
            McpServerConfig {
                name: "overclock".to_string(),
                url: Some("http://127.0.0.1:${OVERCLOCK_MCP_PORT}/mcp".to_string()),
                bearer_env: Some("OVERCLOCK_MCP_BEARER_TOKEN".to_string()),
                ..Default::default()
            },
            "pane_list",
        )
        .await;
    }

    #[tokio::test]
    #[ignore = "precisa de rede e de OVERCLICK_MCP_BEARER_TOKEN"]
    async fn mcp_live_overclick_project_list() {
        call_live(
            McpServerConfig {
                name: "overclick".to_string(),
                url: Some("https://cloud.overclock.sh/mcp".to_string()),
                bearer_env: Some("OVERCLICK_MCP_BEARER_TOKEN".to_string()),
                ..Default::default()
            },
            "project_list",
        )
        .await;
    }
}
