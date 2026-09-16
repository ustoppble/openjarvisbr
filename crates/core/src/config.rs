//! Carregamento de configuração: chave da API do Gemini.
//!
//! A chave nunca é logada nem impressa — apenas seu tamanho pode ser
//! reportado pelo chamador.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::mcp::McpServerConfig;

const ENV_KEY: &str = "GEMINI_API_KEY";

/// Padrão quando `overlay_style` está ausente ou inválido no config.toml.
const DEFAULT_OVERLAY_STYLE: &str = "surreal";

/// "surreal" e "orb" são os únicos valores aceitos; qualquer outro cai no
/// padrão em vez de propagar um estilo desconhecido pro overlay.
fn normalize_overlay_style(value: Option<String>) -> String {
    match value.as_deref() {
        Some("orb") => "orb".to_string(),
        _ => DEFAULT_OVERLAY_STYLE.to_string(),
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
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
    /// Como a pessoa quer ser chamada. Ausente = o Jarvis ainda não sabe o
    /// nome e pergunta na primeira conversa.
    user_name: Option<String>,
    /// Visual do overlay: "surreal" (cena 3D, padrão) ou "orb" (orb de 7
    /// pontos, mais leve).
    overlay_style: Option<String>,
    /// Servidores MCP, `[[mcp_servers]]` no config.toml.
    #[serde(default)]
    mcp_servers: Vec<McpServerConfig>,
}

/// Marcador substituído pelo nome (ou por "você", sem nome) num
/// `system_prompt` customizado do config.toml.
const NAME_PLACEHOLDER: &str = "{nome}";

/// Identidade padrão da OpenJarvisBR, personalizada com o nome salvo em
/// `config.toml` (`user_name`). Sem nome, o Jarvis não inventa: trata a
/// pessoa por "você" e pergunta o nome na primeira conversa. Pode ser
/// trocada por `system_prompt` no config.toml.
pub fn default_system_prompt(user_name: Option<&str>) -> String {
    let user_name = user_name.filter(|n| !n.trim().is_empty());
    let identity = match user_name {
        Some(name) => format!(
            "assistente de voz pessoal de {name}, \
criada em Rust com o Gemini Live. Chame-o(a) sempre de {name}."
        ),
        None => "assistente de voz pessoal, criada em Rust com o Gemini Live. \
Você ainda não sabe o nome da pessoa: na primeira conversa, pergunte como ela \
quer ser chamada e passe a usar esse nome; até lá use 'você'."
            .to_string(),
    };
    format!(
        "Você é a OpenJarvisBR, {identity} Quando perguntarem quem você é, diga que é a \
OpenJarvisBR, o Jarvis, e nunca se descreva como 'modelo de linguagem'.\n\
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
não responda como se fosse para você, a menos que ele te chame."
    )
}

/// Aplica o nome salvo (ou "você", sem nome) num `system_prompt` próprio do
/// usuário, substituindo a marcação `{nome}` quando ela existir. Sem
/// marcação, o texto volta inalterado.
pub fn apply_user_name(prompt: &str, user_name: Option<&str>) -> String {
    let name = user_name.filter(|n| !n.trim().is_empty()).unwrap_or("você");
    prompt.replace(NAME_PLACEHOLDER, name)
}

/// System prompt efetivo: o `system_prompt` customizado (com `{nome}`
/// substituído) quando existir, senão o padrão personalizado com
/// `user_name`.
pub fn effective_system_prompt(settings: &Settings) -> String {
    match &settings.system_prompt {
        Some(custom) => apply_user_name(custom, settings.user_name.as_deref()),
        None => default_system_prompt(settings.user_name.as_deref()),
    }
}

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
    pub user_name: Option<String>,
    pub overlay_style: Option<String>,
    /// Servidores MCP (`[[mcp_servers]]`).
    pub mcp_servers: Vec<McpServerConfig>,
}

/// Estilo do overlay já resolvido: `surreal` (padrão) ou `orb`.
pub fn effective_overlay_style(settings: &Settings) -> String {
    normalize_overlay_style(settings.overlay_style.clone())
}

impl FileConfig {
    fn key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| self.gemini_api_key.clone())
            .filter(|k| !k.is_empty())
    }
}

/// Forma gravada em disco: sempre o nome canônico `api_key`, nunca o legado.
#[derive(Debug, Serialize)]
struct FileConfigOut {
    api_key: Option<String>,
    system_prompt: Option<String>,
    voice: Option<String>,
    device_in: Option<String>,
    device_out: Option<String>,
    barge_in: Option<bool>,
    voice_fx: Option<String>,
    voice_fx_amount: Option<f32>,
    user_name: Option<String>,
    overlay_style: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mcp_servers: Vec<McpServerConfig>,
}

/// Campos que a janela de configurações grava. `api_key` só vem preenchido
/// quando o usuário digitou uma chave nova; caso contrário a chave existente
/// no arquivo é preservada sem nunca passar pelo chamador.
#[derive(Debug, Clone, Default)]
pub struct SaveSettings {
    pub api_key: Option<String>,
    pub system_prompt: Option<String>,
    pub voice: Option<String>,
    pub device_in: Option<String>,
    pub device_out: Option<String>,
    pub barge_in: Option<bool>,
    pub voice_fx_amount: Option<f32>,
    pub user_name: Option<String>,
    pub overlay_style: String,
}

#[derive(Debug)]
pub enum ConfigError {
    MissingKey,
    NoHome,
    ReadFile(PathBuf, std::io::Error),
    ParseFile(PathBuf, toml::de::Error),
    WriteFile(PathBuf, std::io::Error),
    SerializeFile(toml::ser::Error),
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
            ConfigError::NoHome => write!(f, "não foi possível localizar o diretório do usuário (HOME)"),
            ConfigError::ReadFile(path, err) => {
                write!(f, "não foi possível ler {}: {err}", path.display())
            }
            ConfigError::ParseFile(path, err) => {
                write!(f, "não foi possível interpretar {}: {err}", path.display())
            }
            ConfigError::WriteFile(path, err) => {
                write!(f, "não foi possível gravar {}: {err}", path.display())
            }
            ConfigError::SerializeFile(err) => {
                write!(f, "não foi possível gerar o config.toml: {err}")
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
        user_name: parsed.user_name.filter(|s| !s.trim().is_empty()),
        overlay_style: parsed.overlay_style,
        mcp_servers: parsed.mcp_servers,
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

/// Lê o `FileConfig` existente sem exigir chave; arquivo ausente ou inválido
/// vira um `FileConfig` vazio.
fn read_file_config(path: &PathBuf) -> FileConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| toml::from_str(&contents).ok())
        .unwrap_or_default()
}

/// Grava `~/.config/jarvis/config.toml` a partir de `update`. A chave
/// existente no arquivo é preservada quando `update.api_key` é `None`; nunca
/// passa pelo log. Campos não geridos pela janela de configurações (ex.:
/// `voice_fx` on/off) também são preservados.
pub fn save(update: SaveSettings) -> Result<(), ConfigError> {
    let path = config_path().ok_or(ConfigError::NoHome)?;
    let existing = read_file_config(&path);

    let api_key = update
        .api_key
        .filter(|k| !k.is_empty())
        .or_else(|| existing.key());

    let out = FileConfigOut {
        api_key,
        system_prompt: update.system_prompt.filter(|s| !s.trim().is_empty()),
        voice: update.voice.filter(|s| !s.trim().is_empty()),
        device_in: update.device_in.filter(|s| !s.trim().is_empty()),
        device_out: update.device_out.filter(|s| !s.trim().is_empty()),
        barge_in: update.barge_in,
        voice_fx: existing.voice_fx,
        voice_fx_amount: update.voice_fx_amount,
        user_name: update.user_name.filter(|s| !s.trim().is_empty()),
        overlay_style: normalize_overlay_style(Some(update.overlay_style)),
        mcp_servers: existing.mcp_servers,
    };

    let toml_str = toml::to_string_pretty(&out).map_err(ConfigError::SerializeFile)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| ConfigError::WriteFile(path.clone(), err))?;
    }
    std::fs::write(&path, toml_str).map_err(|err| ConfigError::WriteFile(path.clone(), err))?;
    Ok(())
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

    /// Isola `save`/`load_settings` num `HOME` temporário: os dois lêem o
    /// caminho do config a partir da env, então o teste não pode tocar o
    /// `~/.config/jarvis/config.toml` real.
    struct TempHome {
        original: Option<std::ffi::OsString>,
        dir: PathBuf,
    }

    impl TempHome {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("openjarvisbr-config-test-{unique}"));
            std::fs::create_dir_all(&dir).expect("cria diretório temporário");
            let original = std::env::var_os("HOME");
            std::env::set_var("HOME", &dir);
            TempHome { original, dir }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn save_preserves_existing_key_and_round_trips_settings() {
        let home = TempHome::new();

        save(SaveSettings {
            api_key: Some("chave-secreta".to_string()),
            voice: Some("Kore".to_string()),
            barge_in: Some(true),
            voice_fx_amount: Some(0.7),
            ..Default::default()
        })
        .expect("primeiro save grava a chave");

        // Segundo save sem api_key: a chave gravada antes precisa sobreviver.
        save(SaveSettings {
            api_key: None,
            voice: Some("Puck".to_string()),
            device_in: Some("Shure MV7+".to_string()),
            barge_in: Some(false),
            voice_fx_amount: Some(0.5),
            system_prompt: Some("seja breve".to_string()),
            overlay_style: "orb".to_string(),
            ..Default::default()
        })
        .expect("segundo save preserva a chave existente");

        let path = config_path().unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("chave-secreta"));

        let key = load_api_key().expect("chave preservada entre saves");
        assert_eq!(key, "chave-secreta");

        let settings = load_settings();
        assert_eq!(settings.voice.as_deref(), Some("Puck"));
        assert_eq!(settings.device_in.as_deref(), Some("Shure MV7+"));
        assert_eq!(settings.barge_in, Some(false));
        assert_eq!(settings.voice_fx_amount, Some(0.5));
        assert_eq!(settings.system_prompt.as_deref(), Some("seja breve"));
        assert_eq!(effective_overlay_style(&settings), "orb");

        drop(home);
    }

    #[test]
    fn overlay_style_falls_back_to_surreal_when_absent_or_invalid() {
        assert_eq!(normalize_overlay_style(None), "surreal");
        assert_eq!(normalize_overlay_style(Some("".to_string())), "surreal");
        assert_eq!(normalize_overlay_style(Some("bogus".to_string())), "surreal");
        assert_eq!(normalize_overlay_style(Some("orb".to_string())), "orb");
    }

    #[test]
    fn default_prompt_with_name_addresses_the_person_by_name() {
        let prompt = default_system_prompt(Some("Maria"));
        assert!(prompt.contains("assistente de voz pessoal de Maria"));
        assert!(prompt.contains("Chame-o(a) sempre de Maria"));
        assert!(!prompt.to_lowercase().contains("laschuk"));
    }

    #[test]
    fn default_prompt_without_name_asks_how_to_be_called() {
        let prompt = default_system_prompt(None);
        assert!(prompt.contains("Você ainda não sabe o nome da pessoa"));
        assert!(prompt.contains("use 'você'"));
        assert!(!prompt.contains("Maria"));
        assert!(!prompt.to_lowercase().contains("laschuk"));
    }

    #[test]
    fn apply_user_name_replaces_placeholder_or_falls_back_to_voce() {
        assert_eq!(
            apply_user_name("Olá, {nome}!", Some("Maria")),
            "Olá, Maria!"
        );
        assert_eq!(apply_user_name("Olá, {nome}!", None), "Olá, você!");
        assert_eq!(apply_user_name("sem marcação", Some("Maria")), "sem marcação");
    }

    #[test]
    fn effective_system_prompt_prefers_custom_prompt_over_default() {
        let mut settings = Settings {
            user_name: Some("Maria".to_string()),
            ..Default::default()
        };
        assert_eq!(
            effective_system_prompt(&settings),
            default_system_prompt(Some("Maria"))
        );

        settings.system_prompt = Some("Fale com {nome}.".to_string());
        assert_eq!(effective_system_prompt(&settings), "Fale com Maria.");
    }

    #[test]
    fn missing_key_message_mentions_both_options() {
        let msg = ConfigError::MissingKey.to_string();
        assert!(msg.contains("GEMINI_API_KEY"));
        assert!(msg.contains("config.toml"));
    }
}
