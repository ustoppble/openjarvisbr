//! Carregamento de configuração: chave da API do Gemini.
//!
//! A chave nunca é logada nem impressa — apenas seu tamanho pode ser
//! reportado pelo chamador.

use std::fmt;
use std::path::PathBuf;

use serde::Deserialize;

const ENV_KEY: &str = "GEMINI_API_KEY";

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    /// Nome documentado no README e no roteiro de testes.
    api_key: Option<String>,
    /// Nome legado, ainda aceito.
    gemini_api_key: Option<String>,
}

impl FileConfig {
    fn key(self) -> Option<String> {
        self.api_key
            .or(self.gemini_api_key)
            .filter(|k| !k.is_empty())
    }
}

#[derive(Debug)]
pub enum ConfigError {
    MissingKey,
    ReadFile(PathBuf, std::io::Error),
    ParseFile(PathBuf, toml::de::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::MissingKey => write!(
                f,
                "GEMINI_API_KEY não encontrada. Configure de uma das formas:\n\
                 \x20\x201. export GEMINI_API_KEY=sua_chave\n\
                 \x20\x202. crie ~/.config/jarvis/config.toml com:\n\
                 \x20\x20\x20\x20 api_key = \"sua_chave\""
            ),
            ConfigError::ReadFile(path, err) => {
                write!(f, "não foi possível ler {}: {err}", path.display())
            }
            ConfigError::ParseFile(path, err) => {
                write!(f, "não foi possível interpretar {}: {err}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("jarvis")
            .join("config.toml"),
    )
}

/// Carrega a chave da API a partir da env `GEMINI_API_KEY` ou, na ausência
/// dela, de `~/.config/jarvis/config.toml`.
pub fn load_api_key() -> Result<String, ConfigError> {
    if let Ok(key) = std::env::var(ENV_KEY) {
        if !key.is_empty() {
            return Ok(key);
        }
    }

    let path = config_path().ok_or(ConfigError::MissingKey)?;
    if !path.exists() {
        return Err(ConfigError::MissingKey);
    }

    let contents =
        std::fs::read_to_string(&path).map_err(|err| ConfigError::ReadFile(path.clone(), err))?;
    let parsed: FileConfig =
        toml::from_str(&contents).map_err(|err| ConfigError::ParseFile(path.clone(), err))?;

    parsed.key().ok_or(ConfigError::MissingKey)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_config_accepts_api_key_and_legacy_name() {
        let a: FileConfig = toml::from_str("api_key = \"abc\"").unwrap();
        assert_eq!(a.key().as_deref(), Some("abc"));
        let b: FileConfig = toml::from_str("gemini_api_key = \"xyz\"").unwrap();
        assert_eq!(b.key().as_deref(), Some("xyz"));
        let c: FileConfig = toml::from_str("api_key = \"\"").unwrap();
        assert_eq!(c.key(), None);
    }

    #[test]
    fn missing_key_message_mentions_both_options() {
        let msg = ConfigError::MissingKey.to_string();
        assert!(msg.contains("GEMINI_API_KEY"));
        assert!(msg.contains("config.toml"));
    }
}
