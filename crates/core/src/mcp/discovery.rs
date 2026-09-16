//! Descoberta automática do Overclock e do OverClick (JRV-68): o app aberto
//! pelo Finder não herda as envs dos panes, então lê o que o Overclock grava
//! em disco. Cada pane escreve `~/.overclock-app/mcp-pane-<id>.json` com os
//! servidores `overclock` (HTTP local) e `overclick` (nuvem) e o
//! `Authorization` de cada um; o OverClick também pode vir de
//! `~/.claude.json` (`mcpServers.overclick.headers.Authorization`).
//!
//! O token vai para o Keychain e o config guarda só `token_keychain`. Nada
//! aqui loga, imprime ou devolve em erro o token ou a porta.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{keychain_token, set_keychain_token, McpError, McpServerConfig};

/// Pasta onde o Overclock grava os arquivos de pane.
const PANE_DIR: &str = ".overclock-app";
const PANE_PREFIX: &str = "mcp-pane-";
/// Caminho do endpoint do Overclock para clientes Claude.
const OVERCLOCK_PATH: &str = "/mcp/claude";
/// Só o necessário: perfil completo e um pane "chamador" próprio do Jarvis
/// (os demais parâmetros do pane — modelo, missão, cwd — não se aplicam).
const OVERCLOCK_QUERY: &str = "toolProfile=full&callerPaneId=openjarvisbr";
pub const OVERCLICK_URL: &str = "https://cloud.overclock.sh/mcp";

/// Qual servidor descobrir. Vem de `discovery` no config ou, sem ele, do
/// nome do servidor (`overclock` / `overclick`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Overclock,
    Overclick,
}

impl Kind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "overclock" => Some(Kind::Overclock),
            "overclick" => Some(Kind::Overclick),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Overclock => "overclock",
            Kind::Overclick => "overclick",
        }
    }

    /// Descoberta aplicável a `server`: `discovery` explícito vence; sem ele,
    /// só servidores chamados `overclock`/`overclick`.
    pub fn of(server: &McpServerConfig) -> Option<Self> {
        match &server.discovery {
            Some(value) => Kind::parse(value),
            None => Kind::parse(&server.name),
        }
    }
}

/// URL e token achados em disco. `Debug` nunca mostra o token.
#[derive(Clone, PartialEq)]
pub struct Discovered {
    pub url: String,
    pub token: String,
}

impl fmt::Debug for Discovered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Discovered")
            .field("url", &"***")
            .field("token", &"***")
            .finish()
    }
}

/// URL base do Overclock a partir da URL de um pane: esquema, host e porta
/// + `/mcp/claude` + só `toolProfile=full&callerPaneId=openjarvisbr`.
pub fn overclock_base_url(pane_url: &str) -> Option<String> {
    let (scheme, rest) = pane_url.trim().split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some(format!("{scheme}://{authority}{OVERCLOCK_PATH}?{OVERCLOCK_QUERY}"))
}

/// O `mcp-pane-*.json` modificado por último em `dir`.
pub fn newest_pane_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(PANE_PREFIX) && name.ends_with(".json")
        })
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// `Bearer xyz` → `xyz`; vazio vira `None`.
fn bearer_token(authorization: &str) -> Option<String> {
    let value = authorization.trim();
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .unwrap_or(value)
        .trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn server_entry<'a>(json: &'a Value, name: &str) -> Option<&'a Value> {
    json.get("mcpServers")?.get(name)
}

fn authorization(entry: &Value) -> Option<String> {
    bearer_token(entry.get("headers")?.get("Authorization")?.as_str()?)
}

/// Extrai URL e token de `kind` do JSON de um arquivo de pane (ou do
/// `~/.claude.json`, que tem a mesma forma em `mcpServers`).
pub fn from_json(json: &Value, kind: Kind) -> Option<Discovered> {
    let entry = server_entry(json, kind.as_str())?;
    let token = authorization(entry)?;
    let url = match kind {
        Kind::Overclock => overclock_base_url(entry.get("url")?.as_str()?)?,
        Kind::Overclick => entry
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| url.starts_with("https://"))
            .unwrap_or(OVERCLICK_URL)
            .to_string(),
    };
    Some(Discovered { url, token })
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Procura `kind` a partir de `home`: arquivo de pane mais recente e, para
/// o OverClick, `~/.claude.json` como alternativa.
pub fn discover_in(home: &Path, kind: Kind) -> Result<Discovered, McpError> {
    let from_pane = newest_pane_file(&home.join(PANE_DIR))
        .and_then(|path| read_json(&path))
        .and_then(|json| from_json(&json, kind));
    let found = match (from_pane, kind) {
        (Some(found), _) => Some(found),
        (None, Kind::Overclick) => {
            read_json(&home.join(".claude.json")).and_then(|json| from_json(&json, kind))
        }
        (None, Kind::Overclock) => None,
    };
    found.ok_or_else(|| {
        McpError::Discovery(match kind {
            Kind::Overclock => {
                "Overclock não encontrado: abra o Overclock com ao menos um pane".to_string()
            }
            Kind::Overclick => {
                "token do OverClick não encontrado no Overclock nem no ~/.claude.json".to_string()
            }
        })
    })
}

fn home_dir() -> Result<PathBuf, McpError> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| McpError::Discovery("HOME ausente".to_string()))
}

pub fn discover(kind: Kind) -> Result<Discovered, McpError> {
    discover_in(&home_dir()?, kind)
}

/// Servidor já descoberto: URL concreta, token no Keychain (conta = nome do
/// servidor) e `bearer_env` de `existing` preservado como alternativa.
/// Grava no Keychain só se o token mudou.
pub fn apply(
    kind: Kind,
    existing: Option<&McpServerConfig>,
    found: &Discovered,
) -> Result<McpServerConfig, McpError> {
    let mut server = existing.cloned().unwrap_or_else(|| McpServerConfig {
        name: kind.as_str().to_string(),
        ..Default::default()
    });
    let account = server.name.clone();
    if keychain_token(&account).ok().as_deref() != Some(found.token.as_str()) {
        set_keychain_token(&account, &found.token)?;
    }
    server.url = Some(found.url.clone());
    server.command = None;
    server.token_keychain = Some(account);
    server.discovery = Some(kind.as_str().to_string());
    Ok(server)
}

/// Botão "Conectar Overclock/OverClick": descobre e devolve o servidor
/// pronto para gravar no config.
pub fn connect(kind: Kind, existing: Option<&McpServerConfig>) -> Result<McpServerConfig, McpError> {
    apply(kind, existing, &discover(kind)?)
}

/// Start do Engine: para cada servidor com descoberta aplicável, relê o
/// Overclock e devolve só os que mudaram (URL/porta ou token), já com o
/// token atualizado no Keychain. Sem arquivo ou sem token, o servidor fica
/// como está — as envs continuam valendo.
pub fn refresh(servers: &[McpServerConfig]) -> Vec<McpServerConfig> {
    let Ok(home) = home_dir() else {
        return Vec::new();
    };
    servers
        .iter()
        .filter_map(|server| {
            let kind = Kind::of(server)?;
            let found = discover_in(&home, kind).ok()?;
            let current_token = server
                .token_keychain
                .as_deref()
                .and_then(|name| keychain_token(name).ok());
            let unchanged = server.url.as_deref() == Some(found.url.as_str())
                && current_token.as_deref() == Some(found.token.as_str());
            if unchanged {
                return None;
            }
            match apply(kind, Some(server), &found) {
                Ok(updated) => Some(updated),
                Err(err) => {
                    tracing::warn!(server = %server.name, error = %err, "descoberta MCP não gravou o token");
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "openjarvisbr-discovery-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn base_url_descarta_parametros_do_pane() {
        assert_eq!(
            overclock_base_url(
                "http://127.0.0.1:4321/mcp/claude?toolProfile=pane&model=x&workspacePath=/a&callerPaneId=pane-9"
            )
            .as_deref(),
            Some("http://127.0.0.1:4321/mcp/claude?toolProfile=full&callerPaneId=openjarvisbr")
        );
        assert_eq!(
            overclock_base_url("http://localhost:9/mcp").as_deref(),
            Some("http://localhost:9/mcp/claude?toolProfile=full&callerPaneId=openjarvisbr")
        );
        assert_eq!(overclock_base_url("ftp://h/mcp"), None);
        assert_eq!(overclock_base_url("http://u:p@h/mcp"), None);
        assert_eq!(overclock_base_url("sem esquema"), None);
    }

    #[test]
    fn extrai_overclock_e_overclick_do_json_do_pane() {
        let json: Value = serde_json::json!({
            "mcpServers": {
                "overclock": {
                    "type": "http",
                    "url": "http://127.0.0.1:1234/mcp/claude?toolProfile=x&callerPaneId=pane-1",
                    "headers": { "Authorization": "Bearer tok-a" }
                },
                "overclick": {
                    "type": "http",
                    "url": "https://cloud.overclock.sh/mcp",
                    "headers": { "Authorization": "Bearer tok-b" }
                }
            }
        });
        let oc = from_json(&json, Kind::Overclock).unwrap();
        assert_eq!(oc.token, "tok-a");
        assert!(oc.url.ends_with("/mcp/claude?toolProfile=full&callerPaneId=openjarvisbr"));
        let ok = from_json(&json, Kind::Overclick).unwrap();
        assert_eq!((ok.url.as_str(), ok.token.as_str()), (OVERCLICK_URL, "tok-b"));
        assert_eq!(format!("{oc:?}"), "Discovered { url: \"***\", token: \"***\" }");
    }

    #[test]
    fn escolhe_o_arquivo_de_pane_mais_recente() {
        let dir = temp_dir("newest");
        let now = SystemTime::now();
        for (name, age) in [
            ("mcp-pane-pane-1.json", 30),
            ("mcp-pane-pane-2.json", 5),
            ("mcp-pane-pane-3.json", 60),
            ("outro.json", 0),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, "{}").unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(now - Duration::from_secs(age)).unwrap();
        }
        assert_eq!(newest_pane_file(&dir), Some(dir.join("mcp-pane-pane-2.json")));
        assert_eq!(newest_pane_file(&dir.join("nao-existe")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overclick_cai_para_o_claude_json() {
        let home = temp_dir("fallback");
        std::fs::create_dir_all(home.join(PANE_DIR)).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            r#"{"mcpServers":{"overclick":{"headers":{"Authorization":"Bearer tok-c"}}}}"#,
        )
        .unwrap();
        let found = discover_in(&home, Kind::Overclick).unwrap();
        assert_eq!((found.url.as_str(), found.token.as_str()), (OVERCLICK_URL, "tok-c"));
        let err = discover_in(&home, Kind::Overclock).unwrap_err().to_string();
        assert!(err.contains("Overclock não encontrado"), "{err}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn kind_vem_do_campo_ou_do_nome() {
        let server = |name: &str, discovery: Option<&str>| McpServerConfig {
            name: name.to_string(),
            discovery: discovery.map(str::to_string),
            ..Default::default()
        };
        assert_eq!(Kind::of(&server("overclock", None)), Some(Kind::Overclock));
        assert_eq!(Kind::of(&server("oc", Some("overclick"))), Some(Kind::Overclick));
        assert_eq!(Kind::of(&server("outro", None)), None);
    }
}
