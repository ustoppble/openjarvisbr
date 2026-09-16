// Seção `[tools]` e lista `[[mcp_servers]]` do config.toml, vistas pelo
// desktop (JRV-58). O `config::save` do core regrava o arquivo só com os
// campos que conhece, então quem salva pela janela de configurações guarda
// estas seções antes e as devolve depois (`preserve` / `restore`).
//
// Tokens nunca passam por aqui: `[[mcp_servers]]` guarda só o NOME da env
// (`bearer_env`), e a URL é mascarada antes de chegar à interface.

use std::path::PathBuf;

use openjarvisbr_core::mcp::McpServerConfig;
use serde::Serialize;
use toml::{Table, Value};

fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config").join("jarvis").join("config.toml"))
}

fn read_table() -> Table {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|contents| contents.parse::<Table>().ok())
        .unwrap_or_default()
}

fn write_table(table: &Table) -> Result<(), String> {
    let path = config_path().ok_or("não foi possível localizar o diretório do usuário (HOME)")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("não foi possível gravar o config.toml: {err}"))?;
    }
    let contents = toml::to_string_pretty(table).map_err(|err| format!("não foi possível gerar o config.toml: {err}"))?;
    std::fs::write(&path, contents).map_err(|err| format!("não foi possível gravar o config.toml: {err}"))
}

/// `[tools].enabled`; ausente vale ligado (padrão da spec v3).
pub fn tools_enabled() -> bool {
    read_table()
        .get("tools")
        .and_then(|tools| tools.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// Grava só `[tools].enabled`, preservando o resto do arquivo.
pub fn set_tools_enabled(enabled: bool) -> Result<(), String> {
    let mut table = read_table();
    let tools = table
        .entry("tools")
        .or_insert_with(|| Value::Table(Table::new()));
    if !tools.is_table() {
        *tools = Value::Table(Table::new());
    }
    if let Some(tools) = tools.as_table_mut() {
        tools.insert("enabled".to_string(), Value::Boolean(enabled));
    }
    write_table(&table)
}

/// Seções que o `config::save` do core não conhece e descartaria.
pub struct Preserved {
    tools: Option<Value>,
    mcp_servers: Option<Value>,
}

pub fn preserve() -> Preserved {
    let table = read_table();
    Preserved {
        tools: table.get("tools").cloned(),
        mcp_servers: table.get("mcp_servers").cloned(),
    }
}

/// Devolve ao arquivo o que `preserve` guardou (só o que o save tirou) e
/// aplica `enabled` quando veio da janela de configurações.
pub fn restore(preserved: Preserved, enabled: Option<bool>) -> Result<(), String> {
    let mut table = read_table();
    if let Some(tools) = preserved.tools {
        table.entry("tools").or_insert(tools);
    }
    // A aba Ferramentas é a dona de [[mcp_servers]]: volta exatamente o que
    // havia antes do save do core (que reserializa e perderia campos novos).
    if let Some(servers) = preserved.mcp_servers {
        table.insert("mcp_servers".to_string(), servers);
    }
    if let Some(enabled) = enabled {
        let tools = table
            .entry("tools")
            .or_insert_with(|| Value::Table(Table::new()));
        if let Some(tools) = tools.as_table_mut() {
            tools.insert("enabled".to_string(), Value::Boolean(enabled));
        }
    }
    write_table(&table)
}

/// Servidor MCP pronto para a aba Ferramentas: nome, destino mascarado e de
/// onde vem o token — nunca o valor dele.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct McpServerInfo {
    pub name: String,
    /// URL mascarada, ou o executável do transporte stdio (sem argumentos,
    /// que podem carregar credencial).
    pub target: String,
    pub transport: &'static str,
    pub bearer_env: Option<String>,
    pub bearer_env_present: bool,
    pub token_keychain: bool,
    /// Servidor com descoberta automática (`overclock` / `overclick`).
    pub discovery: Option<&'static str>,
}

pub fn mcp_servers() -> Vec<McpServerInfo> {
    server_configs(&read_table())
        .into_iter()
        .map(|server| McpServerInfo {
            target: match (&server.url, &server.command) {
                (Some(url), _) => mask_url(url),
                (None, Some(command)) => command.clone(),
                (None, None) => String::new(),
            },
            transport: if server.url.is_some() { "http" } else { "stdio" },
            bearer_env_present: server.bearer_env.as_deref().is_some_and(env_present),
            token_keychain: server.token_keychain.is_some(),
            discovery: openjarvisbr_core::mcp::discovery::Kind::of(&server).map(|kind| kind.as_str()),
            bearer_env: server.bearer_env,
            name: server.name,
        })
        .collect()
}

/// Se a env existe e não está vazia — só o booleano, nunca o valor.
pub fn env_present(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| !value.is_empty())
}

fn server_configs(table: &Table) -> Vec<McpServerConfig> {
    let Some(servers) = table.get("mcp_servers").and_then(Value::as_array) else {
        return Vec::new();
    };
    servers
        .iter()
        .filter_map(|server| server.clone().try_into::<McpServerConfig>().ok())
        .filter(|server| !server.name.trim().is_empty())
        .collect()
}

pub fn find_server(name: &str) -> Option<McpServerConfig> {
    server_configs(&read_table()).into_iter().find(|server| server.name == name)
}

/// Formulário "Adicionar servidor". `target` começando com http(s):// vira
/// `url`; senão é um comando stdio (primeira palavra = executável).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewMcpServer {
    pub name: String,
    pub target: String,
    /// Token digitado: vai para o Keychain e o config guarda só o nome.
    pub token: Option<String>,
    /// Alternativa avançada: nome da env com o token.
    pub bearer_env: Option<String>,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn build_server(form: NewMcpServer) -> Result<(McpServerConfig, Option<String>), String> {
    let name = form.name.trim().to_string();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("nome inválido: use letras, números, - ou _".to_string());
    }
    let target = form.target.trim();
    if target.is_empty() {
        return Err("informe a URL ou o comando".to_string());
    }
    let mut server = McpServerConfig {
        name: name.clone(),
        ..Default::default()
    };
    if target.starts_with("http://") || target.starts_with("https://") {
        server.url = Some(target.to_string());
    } else {
        let mut parts = target.split_whitespace().map(str::to_string);
        server.command = parts.next();
        server.args = parts.collect();
    }
    let token = non_empty(form.token);
    server.bearer_env = non_empty(form.bearer_env);
    if token.is_some() && server.bearer_env.is_none() {
        server.token_keychain = Some(name);
    }
    Ok((server, token))
}

/// Grava (ou substitui, pelo nome) um servidor em `[[mcp_servers]]`,
/// preservando os demais e o resto do arquivo.
pub fn upsert_server(server: &McpServerConfig) -> Result<(), String> {
    let mut table = read_table();
    let value = Value::try_from(server).map_err(|err| format!("servidor inválido: {err}"))?;
    let servers = table
        .entry("mcp_servers")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !servers.is_array() {
        *servers = Value::Array(Vec::new());
    }
    if let Some(list) = servers.as_array_mut() {
        match list
            .iter_mut()
            .find(|item| item.get("name").and_then(Value::as_str) == Some(server.name.as_str()))
        {
            Some(existing) => *existing = value,
            None => list.push(value),
        }
    }
    write_table(&table)
}

/// Remove o servidor `name`; devolve se ele usava token no Keychain.
pub fn remove_server(name: &str) -> Result<Option<String>, String> {
    let mut table = read_table();
    let mut keychain = None;
    if let Some(list) = table.get_mut("mcp_servers").and_then(Value::as_array_mut) {
        list.retain(|item| {
            let matches = item.get("name").and_then(Value::as_str) == Some(name);
            if matches {
                keychain = item.get("token_keychain").and_then(Value::as_str).map(str::to_string);
            }
            !matches
        });
    }
    write_table(&table)?;
    Ok(keychain)
}

/// Esconde tudo que pode carregar credencial ou endereço numa URL:
/// usuário/senha antes do `@`, porta numérica, valores da query string e
/// fragmento. Placeholders `${VAR}` ficam (são nomes de env, não valores).
pub fn mask_url(url: &str) -> String {
    let (base, fragment) = match url.split_once('#') {
        Some((base, _)) => (base, "#***"),
        None => (url, ""),
    };
    let (base, query) = match base.split_once('?') {
        Some((base, query)) => {
            let masked: Vec<String> = query
                .split('&')
                .filter(|pair| !pair.is_empty())
                .map(|pair| match pair.split_once('=') {
                    Some((key, _)) => format!("{key}=***"),
                    None => "***".to_string(),
                })
                .collect();
            (base, format!("?{}", masked.join("&")))
        }
        None => (base, String::new()),
    };
    let base = match base.split_once("://") {
        Some((scheme, rest)) => {
            let authority_end = rest.find('/').unwrap_or(rest.len());
            let (authority, path) = rest.split_at(authority_end);
            let (userinfo, host) = match authority.rsplit_once('@') {
                Some((_, host)) => ("***@", host),
                None => ("", authority),
            };
            let host = match host.rsplit_once(':') {
                Some((name, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
                    format!("{name}:***")
                }
                _ => host.to_string(),
            };
            format!("{scheme}://{userinfo}{host}{path}")
        }
        None => base.to_string(),
    };
    format!("{base}{query}{fragment}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_url_esconde_query_userinfo_e_fragmento() {
        assert_eq!(mask_url("https://cloud.example/mcp"), "https://cloud.example/mcp");
        assert_eq!(
            mask_url("https://user:secret@cloud.example/mcp?token=abc&x=1#frag"),
            "https://***@cloud.example/mcp?token=***&x=***#***"
        );
        assert_eq!(
            mask_url("http://127.0.0.1:4321/mcp/claude?toolProfile=full"),
            "http://127.0.0.1:***/mcp/claude?toolProfile=***"
        );
        assert_eq!(
            mask_url("http://127.0.0.1:${OVERCLOCK_MCP_PORT}/mcp"),
            "http://127.0.0.1:${OVERCLOCK_MCP_PORT}/mcp"
        );
    }

    #[test]
    fn server_configs_le_servidores_validos() {
        let table: Table = "[[mcp_servers]]\nname = \"a\"\nurl = \"http://h/mcp\"\ntoken_keychain = \"a\"\n[[mcp_servers]]\nurl = \"sem-nome\"\n"
            .parse()
            .unwrap();
        let servers = server_configs(&table);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].token_keychain.as_deref(), Some("a"));
    }

    #[test]
    fn build_server_separa_url_comando_e_token() {
        let form = |target: &str, token: Option<&str>, env: Option<&str>| NewMcpServer {
            name: "srv".to_string(),
            target: target.to_string(),
            token: token.map(str::to_string),
            bearer_env: env.map(str::to_string),
        };
        let (http, token) = build_server(form("https://x/mcp", Some("t"), None)).unwrap();
        assert_eq!(http.url.as_deref(), Some("https://x/mcp"));
        assert_eq!(http.token_keychain.as_deref(), Some("srv"));
        assert_eq!(token.as_deref(), Some("t"));

        let (stdio, _) = build_server(form("node server.js --x", None, Some("ENV_T"))).unwrap();
        assert_eq!(stdio.command.as_deref(), Some("node"));
        assert_eq!(stdio.args, vec!["server.js".to_string(), "--x".to_string()]);
        assert_eq!(stdio.bearer_env.as_deref(), Some("ENV_T"));
        assert_eq!(stdio.token_keychain, None);

        assert!(build_server(NewMcpServer { name: "com espaço".into(), ..form("x", None, None) }).is_err());
    }
}
