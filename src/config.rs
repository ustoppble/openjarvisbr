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
    /// Instrução de sistema (identidade, idioma, regras). Vazio = padrão.
    system_prompt: Option<String>,
    voice: Option<String>,
    device_in: Option<String>,
    device_out: Option<String>,
    barge_in: Option<bool>,
    /// "jarvis" (padrão) ou "off".
    voice_fx: Option<String>,
    /// Intensidade do efeito, 0.0 a 1.0 (padrão 0.35).
    voice_fx_amount: Option<f32>,
}

/// Identidade padrão da OpenJarvisBR. Pode ser trocada por `system_prompt`
/// no config.toml.
pub const DEFAULT_SYSTEM_PROMPT: &str = "\
Você é a OpenJarvisBR, assistente de voz pessoal do Guilherme Laschuk, \
criada em Rust com o Gemini Live. Quando perguntarem quem você é, diga que é a \
OpenJarvisBR, o Jarvis dele, e nunca se descreva como 'modelo de linguagem'.\n\
\n\
Idioma: fale SEMPRE em português do Brasil, natural e direto, como numa conversa \
entre amigos. Só use outra língua quando ele pedir explicitamente e, mesmo assim, \
apenas na frase-alvo: diga a frase na outra língua e volte imediatamente para o \
português para explicar, traduzir e conduzir. Nunca troque o idioma da conversa \
inteira por conta própria. Se ele disser que não entendeu, repita em português.\n\
\n\
Ao ensinar (ex.: inglês do zero): assuma que ele é iniciante absoluto, vá uma \
frase por vez, explique em português o que significa, peça para ele repetir, \
elogie de forma curta e siga em frente. Respostas curtas: isto é voz, não texto.\n\
\n\
Tom de voz: constante e sereno do início ao fim, ritmo uniforme, levemente \
formal, como um assistente de bordo. Não dramatize, não fique eufórico nem \
melancólico, não mude a emoção entre uma frase e outra.\n\
\n\
Memória: preste atenção ao que ele diz ao longo da conversa e retome quando fizer \
sentido (nomes, metas, decisões). Ele está fazendo uma live enquanto fala com você: \
às vezes se dirige à audiência ('gurizada'); nesses momentos, não interrompa e \
não responda como se fosse para você, a menos que ele te chame.";

/// Configuração efetiva depois de juntar config.toml e flags.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub system_prompt: Option<String>,
    pub voice: Option<String>,
    pub device_in: Option<String>,
    pub device_out: Option<String>,
    pub barge_in: Option<bool>,
    pub voice_fx: Option<String>,
    pub voice_fx_amount: Option<f32>,
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

/// Lê os demais campos de `~/.config/jarvis/config.toml` (todos opcionais).
/// Arquivo ausente ou inválido → padrões.
pub fn load_settings() -> Settings {
    let Some(path) = config_path() else {
        return Settings::default();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Settings::default();
    };
    let Ok(parsed) = toml::from_str::<FileConfig>(&contents) else {
        return Settings::default();
    };
    Settings {
        system_prompt: parsed.system_prompt.filter(|s| !s.trim().is_empty()),
        voice: parsed.voice.filter(|s| !s.trim().is_empty()),
        device_in: parsed.device_in.filter(|s| !s.trim().is_empty()),
        device_out: parsed.device_out.filter(|s| !s.trim().is_empty()),
        barge_in: parsed.barge_in,
        voice_fx: parsed.voice_fx.filter(|s| !s.trim().is_empty()),
        voice_fx_amount: parsed.voice_fx_amount,
    }
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
    fn file_config_reads_optional_settings() {
        let c: FileConfig = toml::from_str(
            "api_key = \"k\"\nsystem_prompt = \"seja breve\"\nvoice = \"Kore\"\ndevice_in = \"Shure MV7+\"\nbarge_in = true\n",
        )
        .unwrap();
        assert_eq!(c.system_prompt.as_deref(), Some("seja breve"));
        assert_eq!(c.voice.as_deref(), Some("Kore"));
        assert_eq!(c.device_in.as_deref(), Some("Shure MV7+"));
        assert_eq!(c.barge_in, Some(true));
        assert!(c.device_out.is_none());
    }

    #[test]
    fn missing_key_message_mentions_both_options() {
        let msg = ConfigError::MissingKey.to_string();
        assert!(msg.contains("GEMINI_API_KEY"));
        assert!(msg.contains("config.toml"));
    }
}
