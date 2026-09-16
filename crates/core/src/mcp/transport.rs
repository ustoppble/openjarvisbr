//! Transportes JSON-RPC 2.0 do MCP: HTTP streamable (POST com resposta JSON
//! ou `text/event-stream`) e stdio (uma mensagem JSON por linha).

use std::process::Stdio;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::{expand_env, McpError, McpServerConfig};

/// Tempo máximo de uma requisição (inclui ler a resposta inteira).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SESSION_HEADER: &str = "mcp-session-id";
const PROTOCOL_HEADER: &str = "mcp-protocol-version";

pub enum Transport {
    Http(HttpTransport),
    Stdio(Box<StdioTransport>),
}

impl Transport {
    /// Abre o transporte descrito em `config` (sem handshake MCP).
    pub fn open(config: &McpServerConfig) -> Result<Self, McpError> {
        if let Some(url) = &config.url {
            return Ok(Transport::Http(HttpTransport::new(
                config.name.clone(),
                expand_env(url)?,
                config.bearer_env.clone(),
                config.token_keychain.clone(),
            )?));
        }
        if let Some(command) = &config.command {
            return Ok(Transport::Stdio(Box::new(StdioTransport::spawn(
                command, config,
            )?)));
        }
        Err(McpError::NoTransport(config.name.clone()))
    }

    /// Envia a requisição `message` (com `id`) e devolve a resposta JSON-RPC
    /// de mesmo id.
    pub async fn request(&mut self, message: &Value, id: u64) -> Result<Value, McpError> {
        match self {
            Transport::Http(http) => http.request(message, id).await,
            Transport::Stdio(stdio) => stdio.request(message, id).await,
        }
    }

    /// Envia uma notificação (sem resposta esperada).
    pub async fn notify(&mut self, message: &Value) -> Result<(), McpError> {
        match self {
            Transport::Http(http) => http.post(message).await.map(|_| ()),
            Transport::Stdio(stdio) => stdio.write(message).await,
        }
    }

    /// Protocolo negociado no `initialize`, repassado nos headers HTTP.
    pub fn set_protocol_version(&mut self, version: &str) {
        if let Transport::Http(http) = self {
            http.protocol_version = Some(version.to_string());
        }
    }
}

pub struct HttpTransport {
    client: reqwest::Client,
    name: String,
    url: String,
    bearer_env: Option<String>,
    token_keychain: Option<String>,
    session_id: Option<String>,
    protocol_version: Option<String>,
}

impl HttpTransport {
    fn new(
        name: String,
        url: String,
        bearer_env: Option<String>,
        token_keychain: Option<String>,
    ) -> Result<Self, McpError> {
        // Mesmo provider rustls (ring) da sessão Live; falha só se já houver um.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| McpError::Transport(describe(err)))?;
        Ok(Self {
            client,
            name,
            url,
            bearer_env,
            token_keychain,
            session_id: None,
            protocol_version: None,
        })
    }

    fn headers(&self) -> Result<HeaderMap, McpError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        let token = super::resolve_token(
            "",
            self.token_keychain.as_deref(),
            self.bearer_env.as_deref(),
            |name| super::keychain_token(name).ok(),
            |name| std::env::var(name).ok(),
        )
        .map_err(|err| match err {
            McpError::NoToken { env, .. } => McpError::NoToken { server: self.name.clone(), env },
            other => other,
        })?;
        if let Some((token, source)) = token {
            let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| McpError::Protocol(format!("token inválido em {source}")))?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        if let Some(session) = &self.session_id {
            if let Ok(value) = HeaderValue::from_str(session) {
                headers.insert(SESSION_HEADER, value);
            }
        }
        if let Some(version) = &self.protocol_version {
            if let Ok(value) = HeaderValue::from_str(version) {
                headers.insert(PROTOCOL_HEADER, value);
            }
        }
        Ok(headers)
    }

    async fn post(&mut self, message: &Value) -> Result<reqwest::Response, McpError> {
        let response = self
            .client
            .post(&self.url)
            .headers(self.headers()?)
            .body(message.to_string())
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status().as_u16();
        match status {
            401 | 403 => return Err(McpError::Unauthorized(status)),
            200..=299 => {}
            _ => return Err(McpError::Http(status)),
        }
        if let Some(session) = response
            .headers()
            .get(SESSION_HEADER)
            .and_then(|v| v.to_str().ok())
        {
            self.session_id = Some(session.to_string());
        }
        Ok(response)
    }

    async fn request(&mut self, message: &Value, id: u64) -> Result<Value, McpError> {
        let response = self.post(message).await?;
        let is_sse = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.starts_with("text/event-stream"));
        let body = response.bytes().await.map_err(transport_error)?;
        let body = String::from_utf8_lossy(&body);
        let messages = if is_sse {
            parse_sse(&body)
        } else {
            match serde_json::from_str::<Value>(&body) {
                Ok(Value::Array(batch)) => batch,
                Ok(single) => vec![single],
                Err(err) => return Err(McpError::Protocol(format!("JSON inválido: {err}"))),
            }
        };
        messages
            .into_iter()
            .find(|m| matches_id(m, id))
            .ok_or_else(|| McpError::Protocol(format!("sem resposta para o id {id}")))
    }
}

/// Mensagens JSON dos eventos de um corpo `text/event-stream`. Linhas
/// `data:` do mesmo evento são concatenadas; eventos não-JSON são ignorados.
pub(crate) fn parse_sse(body: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut data = String::new();
    let mut flush = |data: &mut String| {
        if !data.is_empty() {
            if let Ok(value) = serde_json::from_str::<Value>(data) {
                messages.push(value);
            }
            data.clear();
        }
    };
    for line in body.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            flush(&mut data);
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    flush(&mut data);
    messages
}

fn matches_id(message: &Value, id: u64) -> bool {
    message.get("id").and_then(Value::as_u64) == Some(id)
        && (message.get("result").is_some() || message.get("error").is_some())
}

/// Erro do reqwest sem a URL (que tem endereço) e sem headers.
fn transport_error(err: reqwest::Error) -> McpError {
    if err.is_timeout() {
        McpError::Timeout
    } else if is_refused(&err) {
        McpError::Refused
    } else {
        McpError::Transport(describe(err))
    }
}

/// Conexão recusada (nada escutando na porta — ex.: Overclock fechado).
fn is_refused(err: &reqwest::Error) -> bool {
    let mut source = std::error::Error::source(err);
    while let Some(inner) = source {
        if let Some(io) = inner.downcast_ref::<std::io::Error>() {
            if io.kind() == std::io::ErrorKind::ConnectionRefused {
                return true;
            }
        }
        source = inner.source();
    }
    false
}

fn describe(err: reqwest::Error) -> String {
    if err.is_connect() {
        "falha ao conectar".to_string()
    } else {
        err.without_url().to_string()
    }
}

pub struct StdioTransport {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

impl StdioTransport {
    fn spawn(command: &str, config: &McpServerConfig) -> Result<Self, McpError> {
        let mut cmd = Command::new(expand_env(command)?);
        for arg in &config.args {
            cmd.arg(expand_env(arg)?);
        }
        for (key, value) in &config.env {
            cmd.env(key, expand_env(value)?);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| McpError::Transport(format!("não foi possível iniciar: {err}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Transport("stdin indisponível".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Transport("stdout indisponível".to_string()))?;
        Ok(Self {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
        })
    }

    async fn write(&mut self, message: &Value) -> Result<(), McpError> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|err| McpError::Transport(err.to_string()))?;
        self.stdin
            .flush()
            .await
            .map_err(|err| McpError::Transport(err.to_string()))
    }

    async fn request(&mut self, message: &Value, id: u64) -> Result<Value, McpError> {
        self.write(message).await?;
        let read = async {
            loop {
                let line = self
                    .stdout
                    .next_line()
                    .await
                    .map_err(|err| McpError::Transport(err.to_string()))?
                    .ok_or_else(|| McpError::Transport("processo encerrou".to_string()))?;
                // Notificações, logs e pedidos do servidor são ignorados.
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if matches_id(&value, id) {
                        return Ok(value);
                    }
                }
            }
        };
        tokio::time::timeout(REQUEST_TIMEOUT, read)
            .await
            .map_err(|_| McpError::Timeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_sse_events() {
        let body = "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\r\n\r\n: ping\n\ndata: nao-json\n\ndata: {\"id\":2,\ndata: \"result\":{\"ok\":true}}\n";
        let messages = parse_sse(body);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["id"], json!(1));
        assert_eq!(messages[1]["result"]["ok"], json!(true));
    }

    #[test]
    fn matches_only_responses_with_same_id() {
        assert!(matches_id(&json!({"id": 3, "result": {}}), 3));
        assert!(matches_id(&json!({"id": 3, "error": {}}), 3));
        assert!(!matches_id(&json!({"id": 4, "result": {}}), 3));
        assert!(!matches_id(&json!({"id": 3, "method": "ping"}), 3));
    }
}
