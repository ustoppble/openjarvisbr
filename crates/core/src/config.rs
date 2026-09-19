//! Carregamento de configuração: chave da API do Gemini.
//!
//! A chave nunca é logada nem impressa — apenas seu tamanho pode ser
//! reportado pelo chamador.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::mcp::McpServerConfig;
use crate::profiles::{self, Profile};

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
    /// Id do perfil ativo (um dos embutidos ou um de `profiles`). Ausente =
    /// perfil padrão (`profiles::DEFAULT_PROFILE_ID`).
    profile: Option<String>,
    /// Perfis próprios do usuário, `[[profiles]]` no config.toml. Um
    /// customizado com o mesmo id de um embutido o sobrescreve.
    #[serde(default)]
    profiles: Vec<Profile>,
    /// Seção `[tools]`.
    tools: Option<ToolsSection>,
    /// Chave da TypeSafe (Jev). Também aceita a env `TYPESAFE_API_KEY`.
    typesafe_api_key: Option<String>,
    /// Chave do OpenRouter, que serve o Jev em `/api/alpha/decisions`. Usada
    /// só quando não há chave da TypeSafe. Também aceita a env
    /// `OPENROUTER_API_KEY`.
    openrouter_api_key: Option<String>,
    /// Seção `[reflex]`.
    reflex: Option<ReflexSection>,
    /// Seção `[overlay]`: posição e tamanho da janela do overlay.
    overlay: Option<OverlaySection>,
    /// Seção `[settings_window]`: tamanho e posição da janela de
    /// configurações.
    settings_window: Option<WindowGeometry>,
}

/// Escalas do overlay aceitas em `[overlay].scale`, com o fator aplicado ao
/// tamanho da janela e ao zoom da página.
pub const OVERLAY_SCALES: [(&str, f64); 3] = [("small", 0.8), ("medium", 1.0), ("large", 1.25)];

/// Padrão quando `[overlay].scale` está ausente ou inválido.
const DEFAULT_OVERLAY_SCALE: &str = "medium";

/// `[overlay]` no config.toml. `x`/`y` em pixels lógicos; ausentes = topo
/// central do monitor ativo.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct OverlaySection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// "small", "medium" (padrão) ou "large".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<String>,
}

/// Tamanho e posição de uma janela, em pixels lógicos.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct WindowGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Nome da escala já resolvido: qualquer valor fora de `OVERLAY_SCALES` cai
/// no padrão.
pub fn normalize_overlay_scale(value: Option<&str>) -> &'static str {
    OVERLAY_SCALES
        .iter()
        .find(|(name, _)| Some(*name) == value)
        .map(|(name, _)| *name)
        .unwrap_or(DEFAULT_OVERLAY_SCALE)
}

/// Fator da escala `name` (1.0 para desconhecida).
pub fn overlay_scale_factor(name: &str) -> f64 {
    OVERLAY_SCALES
        .iter()
        .find(|(scale, _)| *scale == name)
        .map(|(_, factor)| *factor)
        .unwrap_or(1.0)
}

/// Quem serve o Jev: a TypeSafe direto ou o OpenRouter (mesmo protocolo,
/// endpoint e namespace de modelo diferentes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflexProvider {
    TypeSafe,
    OpenRouter,
}

impl ReflexProvider {
    pub fn endpoint(self) -> &'static str {
        match self {
            Self::TypeSafe => "https://api.typesafe.ai/v1/systemone",
            Self::OpenRouter => "https://openrouter.ai/api/alpha/decisions",
        }
    }
    pub fn default_model(self) -> &'static str {
        match self {
            Self::TypeSafe => "jev-latest",
            Self::OpenRouter => "typesafe/jev-1.13",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::TypeSafe => "TypeSafe",
            Self::OpenRouter => "OpenRouter",
        }
    }
}

/// `[tools]` no config.toml.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ToolsSection {
    /// Liga/desliga todas as ferramentas. Ausente = ligadas.
    pub enabled: Option<bool>,
    /// Modo acesso total: nada pede confirmação e `fs.*` sai do home.
    /// Ausente = desligado.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_access: Option<bool>,
    /// Ferramentas liberadas para sempre ("sempre pode"): não pedem mais
    /// confirmação. Ausente = nenhuma.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always_allow: Vec<String>,
}

/// `[reflex]` no config.toml.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ReflexSection {
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub act_threshold: Option<f32>,
    pub confirm_threshold: Option<f32>,
    pub debounce_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<SiteConfig>,
}

/// Um site que o reflexo pode abrir por voz (`[[reflex.sites]]`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SiteConfig {
    pub name: String,
    pub url: String,
}

/// Configuração resolvida do reflexo. `Debug` manual: nunca imprime a chave.
#[derive(Clone)]
pub struct ReflexSettings {
    pub enabled: bool,
    pub api_key: Option<String>,
    /// Quem atende com essa chave.
    pub provider: ReflexProvider,
    /// URL do endpoint de decisões do provedor.
    pub endpoint: String,
    pub model: String,
    pub act_threshold: f32,
    pub confirm_threshold: f32,
    pub debounce_ms: u64,
    pub sites: Vec<SiteConfig>,
}

impl fmt::Debug for ReflexSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReflexSettings")
            .field("enabled", &self.enabled)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("act_threshold", &self.act_threshold)
            .field("confirm_threshold", &self.confirm_threshold)
            .field("debounce_ms", &self.debounce_ms)
            .field("sites", &self.sites)
            .finish()
    }
}

impl Default for ReflexSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            provider: ReflexProvider::TypeSafe,
            endpoint: ReflexProvider::TypeSafe.endpoint().into(),
            model: ReflexProvider::TypeSafe.default_model().into(),
            act_threshold: 0.85,
            confirm_threshold: 0.85,
            debounce_ms: 120,
            sites: Vec::new(),
        }
    }
}

/// Env que fornece a chave da TypeSafe (vence a do arquivo).
pub const ENV_TYPESAFE_KEY: &str = "TYPESAFE_API_KEY";
/// Env que fornece a chave do OpenRouter (vence a do arquivo).
pub const ENV_OPENROUTER_KEY: &str = "OPENROUTER_API_KEY";

/// Chaves já lidas do ambiente, injetáveis nos testes.
#[derive(Debug, Default, Clone)]
pub struct ReflexEnv {
    pub typesafe: Option<String>,
    pub openrouter: Option<String>,
}

impl ReflexEnv {
    fn from_process() -> Self {
        Self {
            typesafe: std::env::var(ENV_TYPESAFE_KEY).ok(),
            openrouter: std::env::var(ENV_OPENROUTER_KEY).ok(),
        }
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|k| !k.trim().is_empty())
}

/// Resolve `[reflex]` + chave + provedor. A TypeSafe vence quando tem chave
/// (env ou arquivo); senão o OpenRouter. Sem chave = desligado;
/// `enabled = false` vence a chave. O modelo padrão segue o provedor, porque
/// os dois usam namespaces diferentes (`jev-latest` vs `typesafe/jev-1.13`).
fn reflex_from(parsed: &FileConfig, env: ReflexEnv) -> ReflexSettings {
    let d = ReflexSettings::default();
    let typesafe = non_empty(env.typesafe).or_else(|| non_empty(parsed.typesafe_api_key.clone()));
    let openrouter = non_empty(env.openrouter).or_else(|| non_empty(parsed.openrouter_api_key.clone()));
    let (provider, api_key) = match (typesafe, openrouter) {
        (Some(k), _) => (ReflexProvider::TypeSafe, Some(k)),
        (None, Some(k)) => (ReflexProvider::OpenRouter, Some(k)),
        (None, None) => (ReflexProvider::TypeSafe, None),
    };
    let s = parsed.reflex.clone().unwrap_or_default();
    ReflexSettings {
        enabled: api_key.is_some() && s.enabled.unwrap_or(true),
        api_key,
        provider,
        endpoint: provider.endpoint().to_string(),
        model: s
            .model
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| provider.default_model().to_string()),
        act_threshold: s
            .act_threshold
            .unwrap_or(d.act_threshold)
            .clamp(0.5, 1.0),
        confirm_threshold: s
            .confirm_threshold
            .unwrap_or(d.confirm_threshold)
            .clamp(0.5, 1.0),
        debounce_ms: s.debounce_ms.unwrap_or(d.debounce_ms),
        sites: s.sites,
    }
}

/// Lê a configuração do reflexo do config.toml e da env.
pub fn load_reflex() -> ReflexSettings {
    let env = ReflexEnv::from_process();
    let Some(path) = config_path() else {
        return reflex_from(&FileConfig::default(), env);
    };
    reflex_from(&read_file_config(&path), env)
}

/// Globs de ferramentas para o motor: nenhum com `[tools].enabled = false`,
/// senão a allow-list do perfil ativo.
pub fn effective_tool_globs(settings: &Settings) -> Vec<String> {
    if !settings.tools_enabled {
        return Vec::new();
    }
    effective_profile(settings).tools
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
não responda como se fosse para você, a menos que ele te chame.\n\
\n\
Ações: quando uma ferramenta der certo (abrir app ou site, volume, mídia, \
navegar), responda apenas 'Feito.' Não narre o que executou, não repita o pedido, \
não confirme antes de agir quando a ferramenta não pede confirmação. Chame cada \
ferramenta uma vez só: se o resultado disser 'já executada', não chame de novo e \
responda 'Feito.' Se falhar, diga o erro em uma frase curta, sem pedir desculpas."
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
/// substituído) quando existir, senão o prompt do perfil ativo.
pub fn effective_system_prompt(settings: &Settings) -> String {
    match &settings.system_prompt {
        Some(custom) => apply_user_name(custom, settings.user_name.as_deref()),
        None => effective_profile(settings).system_prompt,
    }
}

/// Perfil ativo já resolvido: o id salvo em `profile` (ou o padrão), contra
/// os perfis customizados e depois os embutidos. Um id desconhecido cai no
/// padrão (aviso em log, nunca falha).
pub fn effective_profile(settings: &Settings) -> Profile {
    let id = settings
        .profile
        .as_deref()
        .unwrap_or(profiles::DEFAULT_PROFILE_ID);
    profiles::resolve_profile(id, settings.user_name.as_deref(), &settings.custom_profiles)
}

/// Voz efetiva: a customizada em `voice` quando existir, senão a do perfil
/// ativo.
pub fn effective_voice(settings: &Settings) -> String {
    settings
        .voice
        .clone()
        .unwrap_or_else(|| effective_profile(settings).voice)
}

/// Intensidade do efeito efetiva: `voice_fx = "off"` sempre desliga; senão o
/// `voice_fx_amount` customizado quando existir, senão o do perfil ativo.
pub fn effective_fx_amount(settings: &Settings) -> f32 {
    if settings.voice_fx.as_deref() == Some("off") {
        return 0.0;
    }
    settings
        .voice_fx_amount
        .unwrap_or_else(|| effective_profile(settings).fx_amount)
}

/// Embutidos + customizados, prontos para listar na UI (menu, select,
/// `--list-profiles`).
pub fn all_profiles(settings: &Settings) -> Vec<Profile> {
    profiles::all_profiles(settings.user_name.as_deref(), &settings.custom_profiles)
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
    /// Id do perfil ativo. Ausente = padrão (`profiles::DEFAULT_PROFILE_ID`).
    pub profile: Option<String>,
    /// Perfis próprios do usuário (`[[profiles]]`).
    pub custom_profiles: Vec<Profile>,
    /// `[tools].enabled` (padrão ligado).
    pub tools_enabled: bool,
    /// `[tools].full_access` (padrão desligado).
    pub full_access: bool,
    /// `[tools].always_allow` (JRV-66).
    pub always_allow: Vec<String>,
    /// `[reflex]` + `typesafe_api_key` já resolvidos.
    pub reflex: ReflexSettings,
    /// `[overlay]` como está no arquivo (escala via `effective_overlay_scale`).
    pub overlay: OverlaySection,
    /// `[settings_window]`, quando a janela já foi movida/redimensionada.
    pub settings_window: Option<WindowGeometry>,
}

/// Escala do overlay já resolvida: "small", "medium" (padrão) ou "large".
pub fn effective_overlay_scale(settings: &Settings) -> &'static str {
    normalize_overlay_scale(settings.overlay.scale.as_deref())
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
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    profiles: Vec<Profile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<ToolsSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    typesafe_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    openrouter_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reflex: Option<ReflexSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overlay: Option<OverlaySection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings_window: Option<WindowGeometry>,
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
    /// Id do perfil a ativar. Vazio preserva o perfil já salvo.
    pub profile: String,
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
    let reflex = reflex_from(&parsed, ReflexEnv::from_process());
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
        profile: parsed.profile.filter(|s| !s.trim().is_empty()),
        custom_profiles: parsed.profiles,
        tools_enabled: parsed.tools.as_ref().and_then(|t| t.enabled).unwrap_or(true),
        full_access: parsed
            .tools
            .as_ref()
            .and_then(|t| t.full_access)
            .unwrap_or(false),
        always_allow: parsed
            .tools
            .map(|t| crate::tools::decisions::clean(&t.always_allow))
            .unwrap_or_default(),
        reflex,
        overlay: parsed.overlay.unwrap_or_default(),
        settings_window: parsed.settings_window,
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
        profile: Some(update.profile)
            .filter(|s| !s.trim().is_empty())
            .or(existing.profile),
        profiles: existing.profiles,
        tools: existing.tools,
        typesafe_api_key: existing.typesafe_api_key,
        openrouter_api_key: existing.openrouter_api_key,
        reflex: existing.reflex,
        overlay: existing.overlay,
        settings_window: existing.settings_window,
    };

    write_file_config(&path, &out)
}

/// Troca só o perfil ativo no config.toml, preservando literalmente todo o
/// resto do arquivo (voz, dispositivos, system prompt, perfis customizados
/// etc.) — usado pelo menu da bandeja, que não reconstrói o formulário
/// inteiro da janela de configurações.
pub fn save_profile(id: &str) -> Result<(), ConfigError> {
    let path = config_path().ok_or(ConfigError::NoHome)?;
    let existing = read_file_config(&path);
    let out = FileConfigOut {
        api_key: existing.key(),
        system_prompt: existing.system_prompt.clone(),
        voice: existing.voice.clone(),
        device_in: existing.device_in.clone(),
        device_out: existing.device_out.clone(),
        barge_in: existing.barge_in,
        voice_fx: existing.voice_fx.clone(),
        voice_fx_amount: existing.voice_fx_amount,
        user_name: existing.user_name.clone(),
        overlay_style: normalize_overlay_style(existing.overlay_style.clone()),
        mcp_servers: existing.mcp_servers.clone(),
        profile: Some(id.to_string()),
        profiles: existing.profiles.clone(),
        tools: existing.tools.clone(),
        typesafe_api_key: existing.typesafe_api_key.clone(),
        openrouter_api_key: existing.openrouter_api_key.clone(),
        reflex: existing.reflex.clone(),
        overlay: existing.overlay.clone(),
        settings_window: existing.settings_window,
    };
    write_file_config(&path, &out)
}

/// Grava só `[tools].full_access`, preservando literalmente o resto do
/// arquivo (inclusive seções que o core não modela).
pub fn save_full_access(on: bool) -> Result<(), ConfigError> {
    save_tools_value("full_access", toml::Value::Boolean(on))
}

/// Grava só `[tools].always_allow` (JRV-66), preservando o resto do arquivo.
pub fn save_always_allow(names: &[String]) -> Result<(), ConfigError> {
    let names = crate::tools::decisions::clean(names)
        .into_iter()
        .map(toml::Value::String)
        .collect();
    save_tools_value("always_allow", toml::Value::Array(names))
}

/// Troca uma chave de `[tools]` preservando literalmente o resto do arquivo.
fn save_tools_value(key: &str, value: toml::Value) -> Result<(), ConfigError> {
    save_section_value("tools", key, value)
}

/// Grava só `[reflex].enabled`, preservando o resto do arquivo.
pub fn save_reflex_enabled(on: bool) -> Result<(), ConfigError> {
    save_section_value("reflex", "enabled", toml::Value::Boolean(on))
}

/// Grava `typesafe_api_key` na raiz do config.toml. Vazio remove a chave.
/// A chave nunca passa pelo log.
pub fn save_typesafe_api_key(key: &str) -> Result<(), ConfigError> {
    save_root_key("typesafe_api_key", key)
}

/// Grava `openrouter_api_key` na raiz do config.toml. Vazio remove a chave.
pub fn save_openrouter_api_key(key: &str) -> Result<(), ConfigError> {
    save_root_key("openrouter_api_key", key)
}

fn save_root_key(name: &'static str, key: &str) -> Result<(), ConfigError> {
    let key = key.trim();
    edit_table(|table| {
        if key.is_empty() {
            table.remove(name);
        } else {
            table.insert(name.to_string(), toml::Value::String(key.to_string()));
        }
    })
}

/// Grava `[[reflex.sites]]` e `[reflex].act_threshold` (limitado a 0.5..=1.0),
/// preservando o resto do arquivo. Lista vazia remove os sites.
pub fn save_reflex_sites(sites: &[SiteConfig], act_threshold: f32) -> Result<(), ConfigError> {
    let sites: toml::Value = toml::Value::try_from(sites.to_vec()).map_err(ConfigError::SerializeFile)?;
    let threshold = act_threshold.clamp(0.5, 1.0);
    edit_table(|table| {
        let entry = table
            .entry("reflex")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if !entry.is_table() {
            *entry = toml::Value::Table(toml::Table::new());
        }
        if let Some(section) = entry.as_table_mut() {
            match &sites {
                toml::Value::Array(items) if items.is_empty() => {
                    section.remove("sites");
                }
                _ => {
                    section.insert("sites".to_string(), sites.clone());
                }
            }
            section.insert(
                "act_threshold".to_string(),
                toml::Value::Float(f64::from(threshold)),
            );
        }
    })
}

/// Grava `[overlay].x`/`y` (pixels lógicos), preservando o resto do arquivo.
pub fn save_overlay_position(x: f64, y: f64) -> Result<(), ConfigError> {
    edit_section("overlay", |section| {
        section.insert("x".to_string(), toml::Value::Float(x.round()));
        section.insert("y".to_string(), toml::Value::Float(y.round()));
    })
}

/// Tira `[overlay].x`/`y`: o overlay volta a abrir no topo central.
pub fn clear_overlay_position() -> Result<(), ConfigError> {
    edit_section("overlay", |section| {
        section.remove("x");
        section.remove("y");
    })
}

/// Grava `[overlay].scale` (valor inválido vira o padrão), preservando o
/// resto do arquivo.
pub fn save_overlay_scale(scale: &str) -> Result<(), ConfigError> {
    let scale = normalize_overlay_scale(Some(scale));
    save_section_value("overlay", "scale", toml::Value::String(scale.to_string()))
}

/// Grava `[settings_window]`, preservando o resto do arquivo.
pub fn save_settings_window(geometry: WindowGeometry) -> Result<(), ConfigError> {
    let value = toml::Value::try_from(geometry).map_err(ConfigError::SerializeFile)?;
    edit_table(|table| {
        table.insert("settings_window".to_string(), value);
    })
}

/// Troca uma chave de uma seção (`[section]`) preservando literalmente o
/// resto do arquivo (inclusive seções que o core não modela).
fn save_section_value(section: &str, key: &str, value: toml::Value) -> Result<(), ConfigError> {
    edit_section(section, |entry| {
        entry.insert(key.to_string(), value);
    })
}

/// Aplica `edit` na tabela `[section]` (criada se faltar), preservando o
/// resto do arquivo.
fn edit_section(section: &str, edit: impl FnOnce(&mut toml::Table)) -> Result<(), ConfigError> {
    edit_table(|table| {
        let entry = table
            .entry(section)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if !entry.is_table() {
            *entry = toml::Value::Table(toml::Table::new());
        }
        if let Some(entry) = entry.as_table_mut() {
            edit(entry);
        }
    })
}

/// Lê o config.toml como tabela crua, aplica `edit` e regrava.
fn edit_table(edit: impl FnOnce(&mut toml::Table)) -> Result<(), ConfigError> {
    let path = config_path().ok_or(ConfigError::NoHome)?;
    let mut table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|contents| contents.parse::<toml::Table>().ok())
        .unwrap_or_default();
    edit(&mut table);
    let contents = toml::to_string_pretty(&table).map_err(ConfigError::SerializeFile)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| ConfigError::WriteFile(path.clone(), err))?;
    }
    std::fs::write(&path, contents).map_err(|err| ConfigError::WriteFile(path.clone(), err))
}

fn write_file_config(path: &PathBuf, out: &FileConfigOut) -> Result<(), ConfigError> {
    let toml_str = toml::to_string_pretty(out).map_err(ConfigError::SerializeFile)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| ConfigError::WriteFile(path.clone(), err))?;
    }
    std::fs::write(path, toml_str).map_err(|err| ConfigError::WriteFile(path.clone(), err))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn prompt_padrao_manda_responder_so_feito_depois_de_agir() {
        for name in [None, Some("Laschuk")] {
            let p = super::default_system_prompt(name);
            assert!(p.contains("responda apenas 'Feito.'"), "{p}");
            assert!(p.contains("Chame cada ferramenta uma vez só"), "{p}");
        }
        // todos os perfis embutidos herdam a regra
        for profile in crate::profiles::builtin_profiles(Some("Laschuk")) {
            assert!(profile.system_prompt.contains("responda apenas 'Feito.'"), "{}", profile.id);
        }
    }

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
    fn tools_section_and_profile_globs() {
        let on: FileConfig = toml::from_str("api_key = \"k\"").unwrap();
        assert!(on.tools.is_none());
        let off: FileConfig = toml::from_str("[tools]\nenabled = false\n").unwrap();
        assert_eq!(off.tools.unwrap().enabled, Some(false));

        let mut settings = Settings {
            tools_enabled: true,
            ..Settings::default()
        };
        // A lista de cada perfil é testada em profiles.rs; aqui só a ligação
        // config → perfil ativo.
        let assistant = profiles::resolve_profile(profiles::DEFAULT_PROFILE_ID, None, &[]).tools;
        assert!(assistant.iter().any(|glob| glob == "*"));
        assert_eq!(effective_tool_globs(&settings), assistant);
        settings.profile = Some("therapist".to_string());
        assert!(effective_tool_globs(&settings).is_empty());
        settings.profile = Some("pair_programmer".to_string());
        settings.tools_enabled = false;
        assert!(effective_tool_globs(&settings).is_empty());
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

    /// `HOME` é global no processo; `cargo test` roda os testes em threads
    /// paralelas, então mais de um `TempHome` vivo ao mesmo tempo faz um
    /// pisar no `HOME` do outro. Este mutex serializa os testes que precisam
    /// de um `HOME` isolado.
    static TEMP_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Isola `save`/`load_settings` num `HOME` temporário: os dois lêem o
    /// caminho do config a partir da env, então o teste não pode tocar o
    /// `~/.config/jarvis/config.toml` real.
    struct TempHome {
        _guard: std::sync::MutexGuard<'static, ()>,
        original: Option<std::ffi::OsString>,
        dir: PathBuf,
    }

    impl TempHome {
        fn new() -> Self {
            let guard = TEMP_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("openjarvisbr-config-test-{unique}"));
            std::fs::create_dir_all(&dir).expect("cria diretório temporário");
            let original = std::env::var_os("HOME");
            std::env::set_var("HOME", &dir);
            TempHome {
                _guard: guard,
                original,
                dir,
            }
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
    fn full_access_defaults_off_and_round_trips_preserving_the_file() {
        let _home = TempHome::new();
        assert!(!load_settings().full_access);
        let path = config_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "api_key = \"k\"\n[tools]\nenabled = false\n[[mcp_servers]]\nname = \"a\"\nurl = \"http://h/mcp\"\n",
        )
        .unwrap();
        assert!(!load_settings().full_access);

        save_full_access(true).unwrap();
        let settings = load_settings();
        assert!(settings.full_access);
        assert!(!settings.tools_enabled);
        assert_eq!(settings.mcp_servers.len(), 1);
        assert_eq!(load_api_key().unwrap(), "k");

        // `save` da janela de configurações preserva a seção [tools].
        save(SaveSettings::default()).unwrap();
        assert!(load_settings().full_access);
        save_full_access(false).unwrap();
        assert!(!load_settings().full_access);
    }

    #[test]
    fn always_allow_is_persisted_and_survives_the_settings_save() {
        let _home = TempHome::new();
        assert!(load_settings().always_allow.is_empty());
        let path = config_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "api_key = \"k\"\n[tools]\nfull_access = true\n").unwrap();

        save_always_allow(&["shell.run".into(), "shell.run".into(), " fs.write".into()]).unwrap();
        let settings = load_settings();
        assert_eq!(settings.always_allow, ["shell.run", "fs.write"]);
        assert!(settings.full_access);
        assert_eq!(load_api_key().unwrap(), "k");

        // "Reabrir o app": a janela de configurações regrava o arquivo.
        save(SaveSettings::default()).unwrap();
        assert_eq!(load_settings().always_allow, ["shell.run", "fs.write"]);

        save_always_allow(&["fs.write".into()]).unwrap();
        assert_eq!(load_settings().always_allow, ["fs.write"]);
        save_always_allow(&[]).unwrap();
        assert!(load_settings().always_allow.is_empty());
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
    fn load_settings_reads_profile_id_and_custom_profiles() {
        let home = TempHome::new();
        save(SaveSettings {
            api_key: Some("k".to_string()),
            profile: "pair_programmer".to_string(),
            ..Default::default()
        })
        .unwrap();

        let path = config_path().unwrap();
        let mut contents = std::fs::read_to_string(&path).unwrap();
        contents.push_str(
            "\n[[profiles]]\nid = \"meu_perfil\"\nname = \"Meu Perfil\"\nsystem_prompt = \"seja você mesmo\"\n",
        );
        std::fs::write(&path, contents).unwrap();

        let settings = load_settings();
        assert_eq!(settings.profile.as_deref(), Some("pair_programmer"));
        assert_eq!(settings.custom_profiles.len(), 1);
        assert_eq!(settings.custom_profiles[0].id, "meu_perfil");

        let profile = effective_profile(&settings);
        assert_eq!(profile.id, "pair_programmer");

        drop(home);
    }

    #[test]
    fn custom_profile_with_builtin_id_overrides_it() {
        let home = TempHome::new();
        let settings = Settings {
            profile: Some("assistant".to_string()),
            custom_profiles: vec![Profile {
                id: "assistant".to_string(),
                name: "Assistente custom".to_string(),
                description: String::new(),
                system_prompt: "prompt próprio".to_string(),
                voice: "Kore".to_string(),
                fx_amount: 0.1,
                tools: Vec::new(),
            }],
            ..Default::default()
        };
        let profile = effective_profile(&settings);
        assert_eq!(profile.name, "Assistente custom");
        assert_eq!(profile.system_prompt, "prompt próprio");
        assert_eq!(effective_voice(&settings), "Kore");
        assert_eq!(effective_fx_amount(&settings), 0.1);
        drop(home);
    }

    #[test]
    fn unknown_profile_id_falls_back_to_default() {
        let settings = Settings {
            profile: Some("nao-existe".to_string()),
            ..Default::default()
        };
        let profile = effective_profile(&settings);
        assert_eq!(profile.id, profiles::DEFAULT_PROFILE_ID);
    }

    #[test]
    fn save_profile_switches_active_profile_and_preserves_the_rest() {
        let home = TempHome::new();
        save(SaveSettings {
            api_key: Some("chave-secreta".to_string()),
            voice: Some("Kore".to_string()),
            system_prompt: Some("meu prompt".to_string()),
            ..Default::default()
        })
        .unwrap();

        save_profile("english_teacher").expect("troca o perfil");

        let settings = load_settings();
        assert_eq!(settings.profile.as_deref(), Some("english_teacher"));
        assert_eq!(settings.voice.as_deref(), Some("Kore"));
        assert_eq!(settings.system_prompt.as_deref(), Some("meu prompt"));
        assert_eq!(load_api_key().unwrap(), "chave-secreta");

        drop(home);
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

    #[test]
    fn save_reflex_sites_grava_lista_e_limiar_preservando_o_resto() {
        let home = TempHome::new();
        let path = config_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "typesafe_api_key = \"k\"\n[reflex]\nenabled = true\n[outra]\nx = 1\n",
        )
        .unwrap();

        let sites = vec![SiteConfig {
            name: "YouTube".into(),
            url: "https://youtube.com".into(),
        }];
        save_reflex_sites(&sites, 1.7).expect("grava sites");

        let r = load_reflex();
        assert!(r.enabled, "enabled preservado");
        assert_eq!(r.sites, sites);
        assert!((r.act_threshold - 1.0).abs() < f32::EPSILON, "limiar limitado a 1.0");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[outra]"), "seção alheia sobrevive");

        // Lista vazia remove os sites, sem tocar no resto.
        save_reflex_sites(&[], 0.9).expect("remove sites");
        let r = load_reflex();
        assert!(r.sites.is_empty());
        assert!((r.act_threshold - 0.9).abs() < 1e-6);
        drop(home);
    }

    #[test]
    fn reflex_le_secao_e_sites() {
        let raw = r#"
typesafe_api_key = "ts-xyz"
[reflex]
enabled = true
act_threshold = 0.9
[[reflex.sites]]
name = "YouTube"
url = "https://youtube.com"
"#;
        let parsed: FileConfig = toml::from_str(raw).unwrap();
        let r = reflex_from(&parsed, ReflexEnv::default());
        assert!(r.enabled);
        assert_eq!(r.api_key.as_deref(), Some("ts-xyz"));
        assert_eq!(r.model, "jev-latest");
        assert!((r.act_threshold - 0.9).abs() < 1e-6);
        assert!((r.confirm_threshold - 0.85).abs() < 1e-6);
        assert_eq!(r.debounce_ms, 120);
        assert_eq!(r.sites.len(), 1);
        assert_eq!(r.sites[0].name, "YouTube");
    }

    #[test]
    fn reflex_openrouter_quando_so_ha_chave_dele() {
        let parsed: FileConfig = toml::from_str("openrouter_api_key = \"or-xyz\"").unwrap();
        let r = reflex_from(&parsed, ReflexEnv::default());
        assert!(r.enabled);
        assert_eq!(r.provider, ReflexProvider::OpenRouter);
        assert_eq!(r.endpoint, "https://openrouter.ai/api/alpha/decisions");
        assert_eq!(r.model, "typesafe/jev-1.13");
        assert_eq!(r.api_key.as_deref(), Some("or-xyz"));
        // env do OpenRouter vence o arquivo
        let r = reflex_from(&parsed, ReflexEnv { openrouter: Some("or-env".into()), ..Default::default() });
        assert_eq!(r.api_key.as_deref(), Some("or-env"));
    }

    #[test]
    fn reflex_typesafe_vence_openrouter_e_modelo_explicito_vence_padrao() {
        let parsed: FileConfig =
            toml::from_str("typesafe_api_key = \"ts\"\nopenrouter_api_key = \"or\"").unwrap();
        let r = reflex_from(&parsed, ReflexEnv::default());
        assert_eq!(r.provider, ReflexProvider::TypeSafe);
        assert_eq!(r.model, "jev-latest");
        let parsed: FileConfig =
            toml::from_str("openrouter_api_key = \"or\"\n[reflex]\nmodel = \"typesafe/jev-1.13-20260917\"").unwrap();
        let r = reflex_from(&parsed, ReflexEnv::default());
        assert_eq!(r.model, "typesafe/jev-1.13-20260917");
    }

    #[test]
    fn reflex_sem_chave_fica_desligado_e_env_vence_arquivo() {
        let parsed: FileConfig = toml::from_str("").unwrap();
        let r = reflex_from(&parsed, ReflexEnv::default());
        assert!(!r.enabled);
        assert!(r.api_key.is_none());
        let parsed: FileConfig = toml::from_str("typesafe_api_key = \"arquivo\"").unwrap();
        let r = reflex_from(&parsed, ReflexEnv { typesafe: Some("env".to_string()), ..Default::default() });
        assert_eq!(r.api_key.as_deref(), Some("env"));
        assert!(r.enabled);
    }

    #[test]
    fn reflex_enabled_false_vence_chave() {
        let parsed: FileConfig =
            toml::from_str("typesafe_api_key = \"x\"\n[reflex]\nenabled = false").unwrap();
        assert!(!reflex_from(&parsed, ReflexEnv::default()).enabled);
    }

    #[test]
    fn reflex_debug_nao_vaza_chave() {
        let parsed: FileConfig = toml::from_str("typesafe_api_key = \"segredo-456\"").unwrap();
        let s = format!("{:?}", reflex_from(&parsed, ReflexEnv::default()));
        assert!(!s.contains("segredo-456"));
        assert!(s.contains("***"));
    }

    #[test]
    fn escala_do_overlay_normaliza_e_da_o_fator() {
        assert_eq!(normalize_overlay_scale(None), "medium");
        assert_eq!(normalize_overlay_scale(Some("gigante")), "medium");
        assert_eq!(normalize_overlay_scale(Some("small")), "small");
        assert_eq!(overlay_scale_factor("small"), 0.8);
        assert_eq!(overlay_scale_factor("large"), 1.25);
        assert_eq!(overlay_scale_factor("?"), 1.0);
    }

    #[test]
    fn overlay_e_janela_de_configuracoes_sobrevivem_aos_saves_e_preservam_o_resto() {
        let _home = TempHome::new();
        let settings = load_settings();
        assert_eq!(effective_overlay_scale(&settings), "medium");
        assert_eq!(settings.overlay, OverlaySection::default());
        assert!(settings.settings_window.is_none());

        let path = config_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "api_key = \"k\"\n[tools]\nfull_access = true\n[reflex]\nenabled = false\nact_threshold = 0.9\n",
        )
        .unwrap();

        save_overlay_position(812.4, 40.0).unwrap();
        save_overlay_scale("large").unwrap();
        let geometry = WindowGeometry { x: 10.0, y: 20.0, width: 600.0, height: 700.0 };
        save_settings_window(geometry).unwrap();

        let settings = load_settings();
        assert_eq!(settings.overlay.x, Some(812.0));
        assert_eq!(settings.overlay.y, Some(40.0));
        assert_eq!(effective_overlay_scale(&settings), "large");
        assert_eq!(settings.settings_window, Some(geometry));
        assert!(settings.full_access);
        assert_eq!(load_api_key().unwrap(), "k");

        // A janela de configurações e o menu de perfil regravam o arquivo
        // inteiro: [overlay] e [settings_window] têm que sobreviver.
        save(SaveSettings::default()).unwrap();
        save_profile("pair_programmer").unwrap();
        let settings = load_settings();
        assert_eq!(settings.overlay.x, Some(812.0));
        assert_eq!(effective_overlay_scale(&settings), "large");
        assert_eq!(settings.settings_window, Some(geometry));

        save_overlay_scale("gigante").unwrap();
        clear_overlay_position().unwrap();
        let settings = load_settings();
        assert_eq!(settings.overlay, OverlaySection { x: None, y: None, scale: Some("medium".into()) });
        assert!(settings.full_access);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[reflex]"));
        assert!(raw.contains("act_threshold = 0.9"));
    }
}
