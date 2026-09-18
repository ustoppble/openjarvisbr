//! Peças do motor para ferramentas: montagem do `Registry` com a allow-list
//! do perfil, seção de ferramentas do system prompt, leitura de "sim/não" na
//! fala do usuário e os resumos curtos que vão para eventos e overlay.
//!
//! O loop em si (esperar confirmação, executar com timeout, responder ao
//! modelo) vive em `engine.rs`; aqui fica o que dá para testar sem sessão.

use std::time::Duration;

use serde_json::Value;

use crate::mcp::{self, McpServerConfig};
use crate::tools::decisions::normalize_words;
use crate::tools::{local, system, FullAccess, Registry, ToolCall};

/// Janela em que um "sim/não" falado vale como resposta a um pedido de
/// confirmação.
pub const VOICE_CONFIRM_WINDOW: Duration = Duration::from_secs(15);
/// Sem resposta (voz ou handle) neste prazo, a chamada é negada.
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(20);
/// Limite de execução de qualquer ferramenta.
pub const EXEC_TIMEOUT: Duration = Duration::from_secs(30);
/// Uma ação `Confirm` idêntica (mesmo nome e argumentos) pedida de novo logo
/// depois de aprovada não executa outra vez.
pub const REPEAT_WINDOW: Duration = Duration::from_secs(30);
/// Ação feita pelo reflexo: uma chamada igual do modelo dentro desta janela
/// recebe sucesso sem executar de novo.
pub const REFLEX_DONE_WINDOW: Duration = Duration::from_secs(8);
/// Chamada idêntica do modelo que chega logo depois de uma fala nova ainda é
/// a repetição atrasada da fala anterior (trace 19:57:08: "abre a globo" e,
/// 50 ms depois, app.open Calculator de novo). Dentro desta folga não executa
/// nem aprende.
pub const LATE_DUP_GRACE: Duration = Duration::from_secs(2);
/// Tamanho máximo dos resumos de eventos (overlay, terminal).
const SUMMARY_MAX: usize = 160;

/// Instruções de uso de ferramentas, anexadas ao system prompt quando o
/// perfil tem alguma ferramenta liberada.
pub const TOOLS_PROMPT: &str = "Ferramentas: você pode agir no computador e nos \
sistemas do usuário com as ferramentas declaradas nesta sessão.\n\
- Só chame uma ferramenta quando o usuário pedir uma ação ou uma informação que \
dependa dela; em conversa comum, apenas converse.\n\
- Pedido de ação: chame a ferramenta direto, sem anunciar o que vai fazer, sem \
reformular o pedido e sem perguntar 'confirma?'. Quem pede confirmação, quando \
precisa, é o app: ele segura a execução, pergunta ao usuário e te envia o \
resultado. Nunca peça confirmação por conta própria.\n\
- Se o usuário disser 'sim' ou 'não' a uma confirmação do app, NÃO chame a \
ferramenta de novo: o app já executa ou cancela a chamada pendente e te envia \
o resultado.\n\
- Com o resultado em mãos, diga em uma frase curta o que aconteceu. Não releia \
o comando, não anuncie passos, não repita instruções que já deu nesta conversa \
nem a frase que acabou de dizer. Só detalhe se o usuário pedir. Nunca invente \
um resultado que a ferramenta não devolveu.\n\
- Se o resultado vier com erro, negado ou sem confirmação, diga isso em uma \
frase e não tente de novo sem o usuário pedir.\n\
- Se o usuário disser 'sempre pode' ou 'não pergunta mais', o app grava a \
permissão: responda só 'Certo, não pergunto mais.'\n\
- Nunca leia em voz alta tokens, senhas, chaves, segredos nem endereços de \
servidor, mesmo que apareçam num resultado.";

/// Anexado ao prompt de ferramentas quando a sessão abre em acesso total.
pub const FULL_ACCESS_PROMPT: &str = "Modo acesso total ligado: nenhuma ação \
pede confirmação e os arquivos podem estar em qualquer pasta do computador, \
não só no home.";

/// Uma fala repetida só descarta a partir deste número de palavras quando
/// ainda está chegando (prefixo da anterior); igual por inteiro descarta
/// sempre.
const REPEAT_PREFIX_WORDS: usize = 6;

/// A fala do modelo neste turno repete a do turno anterior? Compara
/// normalizado (sem caixa, acento nem pontuação): igual por inteiro, ou, ainda
/// chegando, um prefixo de pelo menos [`REPEAT_PREFIX_WORDS`] palavras.
pub fn is_repeat(current: &str, previous: &str) -> bool {
    let current = normalize_words(current);
    let previous = normalize_words(previous);
    if current.is_empty() || previous.is_empty() {
        return false;
    }
    current == previous
        || (current.len() >= REPEAT_PREFIX_WORDS && previous.starts_with(&current))
}

/// Resposta reconhecida numa fala durante a espera por confirmação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceAnswer {
    Approve,
    Deny,
}

const APPROVE_WORDS: &[&str] = &[
    "sim",
    "confirma",
    "confirmo",
    "confirmado",
    "pode",
    "manda",
    "isso",
    "claro",
    "positivo",
];
const DENY_WORDS: &[&str] = &[
    "nao",
    "cancela",
    "cancelar",
    "cancelado",
    "deixa",
    "negativo",
    "pare",
    "espera",
];

/// Lê "sim/confirma/pode/manda" ou "não/cancela/deixa" numa fala (que pode
/// chegar picada em fragmentos: passe o acumulado). Negação vence: "pode não"
/// nega.
pub fn voice_answer(text: &str) -> Option<VoiceAnswer> {
    let words = normalize_words(text);
    if words.iter().any(|w| DENY_WORDS.contains(&w.as_str())) {
        return Some(VoiceAnswer::Deny);
    }
    if words.iter().any(|w| APPROVE_WORDS.contains(&w.as_str())) {
        return Some(VoiceAnswer::Approve);
    }
    None
}

/// O modelo terminou a fala pedindo confirmação ("…, confirma?")?
pub fn asks_confirmation(model_turn: &str) -> bool {
    let words = normalize_words(model_turn);
    words
        .iter()
        .rev()
        .take(6)
        .any(|w| matches!(w.as_str(), "confirma" | "confirmar" | "posso" | "autoriza"))
        && model_turn.trim_end().ends_with('?')
}

/// Registro de produção: locais + sistema + MCP, filtrado pelos globs do
/// perfil. Globs vazios = nenhuma ferramenta (e nenhum MCP é contatado).
pub async fn build_registry(
    globs: &[String],
    mcp_servers: &[McpServerConfig],
    full_access: &FullAccess,
) -> Registry {
    if globs.is_empty() {
        return Registry::new();
    }
    let mut registry = Registry::new();
    for tool in local::all(full_access.clone()).into_iter().chain(system::all()) {
        registry.register(tool);
    }
    // Servidor MCP que nenhum glob do perfil alcança nem é contatado.
    let servers: Vec<McpServerConfig> = mcp_servers
        .iter()
        .filter(|server| {
            let probe = format!("mcp.{}.x", server.name);
            globs.iter().any(|glob| mcp_glob_may_match(glob, &probe))
        })
        .cloned()
        .collect();
    for tool in mcp::load_all(&servers).await {
        registry.register(tool);
    }
    registry.filter_for_profile(globs)
}

/// Um glob pode casar alguma tool deste servidor? Compara só o prefixo antes
/// do primeiro `*` com `mcp.<server>.`.
fn mcp_glob_may_match(glob: &str, probe: &str) -> bool {
    let prefix = glob.trim().split('*').next().unwrap_or_default();
    let server_prefix = &probe[..probe.len() - 1];
    server_prefix.starts_with(prefix) || prefix.starts_with(server_prefix)
}

/// Uma linha que diz o que a chamada vai fazer, para o overlay/terminal
/// ("shell.run: ls -la"). Campos com cara de segredo saem como `***`.
pub fn call_summary(call: &ToolCall) -> String {
    const MAIN_KEYS: &[&str] = &[
        "command", "app", "name", "url", "path", "title", "text", "action", "level", "query",
    ];
    let detail = match &call.args {
        Value::Object(map) if map.is_empty() => String::new(),
        Value::Object(map) => {
            let main = MAIN_KEYS
                .iter()
                .find_map(|key| map.get(*key).and_then(Value::as_str).map(|v| (key, v)));
            match main {
                Some((key, value)) if !is_secret_key(key) && map.len() == 1 => value.to_string(),
                _ => compact_args(map),
            }
        }
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let line = if detail.is_empty() {
        call.name.clone()
    } else {
        format!("{}: {detail}", call.name)
    };
    truncate(&line, SUMMARY_MAX)
}

fn compact_args(map: &serde_json::Map<String, Value>) -> String {
    map.iter()
        .map(|(key, value)| {
            let value = if is_secret_key(key) {
                "***".to_string()
            } else {
                match value {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                }
            };
            format!("{key}={value}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_lowercase();
    [
        "token", "secret", "password", "senha", "key", "bearer", "cookie", "auth",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

/// Texto de contexto enviado ao modelo quando o reflexo agiu, para ele não
/// repetir a ação nem narrar como futuro.
pub fn reflex_context_text(call: &ToolCall) -> String {
    format!(
        "[sistema] já executado agora pelo reflexo: {} ({}). Não chame de novo; \
         se for comentar, fale no passado e seja breve.",
        call.name,
        call_summary(call)
    )
}

/// Resumo de um resultado: a mensagem de erro, ou o começo da saída.
pub fn result_summary(output: &Value, error: Option<&str>) -> String {
    if let Some(error) = error {
        return truncate(error, SUMMARY_MAX);
    }
    let text = match output {
        Value::Null => "ok".to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    truncate(text.trim(), SUMMARY_MAX)
}

fn truncate(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// System prompt com a seção de ferramentas, quando há alguma liberada.
pub fn system_prompt_with_tools(prompt: &str, has_tools: bool, full_access: bool) -> String {
    if !has_tools {
        return prompt.to_string();
    }
    let tools = if full_access {
        format!("{TOOLS_PROMPT}\n{FULL_ACCESS_PROMPT}")
    } else {
        TOOLS_PROMPT.to_string()
    };
    if prompt.trim().is_empty() {
        return tools;
    }
    format!("{prompt}\n\n{tools}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn voice_answers() {
        assert_eq!(voice_answer("Sim."), Some(VoiceAnswer::Approve));
        assert_eq!(voice_answer(" pode mandar"), Some(VoiceAnswer::Approve));
        assert_eq!(voice_answer("confirma"), Some(VoiceAnswer::Approve));
        assert_eq!(voice_answer("manda ver"), Some(VoiceAnswer::Approve));
        assert_eq!(voice_answer("Não!"), Some(VoiceAnswer::Deny));
        assert_eq!(voice_answer("nao, cancela"), Some(VoiceAnswer::Deny));
        assert_eq!(voice_answer("deixa pra lá"), Some(VoiceAnswer::Deny));
        assert_eq!(voice_answer("pode não"), Some(VoiceAnswer::Deny));
        assert_eq!(voice_answer("qual é a previsão"), None);
        // "simples" não é "sim".
        assert_eq!(voice_answer("simples"), None);
        // "para" é preposição comum ("pode rodar para mim"), não negação.
        assert_eq!(
            voice_answer("pode rodar para mim"),
            Some(VoiceAnswer::Approve)
        );
    }

    #[test]
    fn detects_model_confirmation_question() {
        assert!(asks_confirmation("Vou criar o card de bug, confirma?"));
        assert!(asks_confirmation("Posso rodar o comando?"));
        assert!(!asks_confirmation(
            "Criei o card. Confirma se apareceu no quadro."
        ));
        assert!(!asks_confirmation("Quer que eu abra o Safari?"));
    }

    #[test]
    fn detects_repeated_model_speech() {
        let previous = "Pronto, rodei o ls e listei os arquivos da sua pasta.";
        assert!(is_repeat("pronto rodei o ls e listei os arquivos da sua pasta", previous));
        assert!(is_repeat("Pronto, rodei o ls e listei", previous));
        // Começo curto em comum não é repetição.
        assert!(!is_repeat("Pronto, rodei", previous));
        assert!(!is_repeat("Pronto, abri o Safari.", previous));
        assert!(!is_repeat("", previous));
        assert!(!is_repeat("Pronto.", ""));
        assert!(is_repeat("Pronto.", "pronto"));
    }

    #[test]
    fn summaries_are_short_and_mask_secrets() {
        let call = ToolCall {
            id: "1".into(),
            name: "shell.run".into(),
            args: json!({"command": "ls ~"}),
        };
        assert_eq!(call_summary(&call), "shell.run: ls ~");

        let call = ToolCall {
            id: "2".into(),
            name: "mcp.x.login".into(),
            args: json!({"user": "a", "api_token": "abc123"}),
        };
        let summary = call_summary(&call);
        assert!(summary.contains("api_token=***"), "{summary}");
        assert!(!summary.contains("abc123"));

        let long = "x".repeat(500);
        assert_eq!(
            result_summary(&json!(long), None).chars().count(),
            SUMMARY_MAX
        );
        assert_eq!(result_summary(&Value::Null, Some("negado")), "negado");
    }

    #[test]
    fn reflex_context_names_the_action_in_the_past() {
        let call = ToolCall {
            id: "reflex-1".into(),
            name: "app.open".into(),
            args: json!({"name": "Safari"}),
        };
        let text = reflex_context_text(&call);
        assert!(text.starts_with("[sistema] já executado agora pelo reflexo: app.open (app.open: Safari)"), "{text}");
        assert!(text.contains("Não chame de novo"));
        assert_eq!(REFLEX_DONE_WINDOW, Duration::from_secs(8));
    }

    #[test]
    fn prompt_section_only_with_tools() {
        assert_eq!(system_prompt_with_tools("base", false, false), "base");
        assert_eq!(system_prompt_with_tools("base", false, true), "base");
        assert!(system_prompt_with_tools("base", true, true).ends_with(FULL_ACCESS_PROMPT));
        assert!(!system_prompt_with_tools("base", true, false).contains(FULL_ACCESS_PROMPT));
        let with = system_prompt_with_tools("base", true, false);
        assert!(with.starts_with("base\n\n"));
        assert!(with.contains("Nunca peça confirmação por conta própria"));
        assert!(with.contains("Nunca invente"));
        assert!(with.contains("não pergunto mais"));
    }

    #[test]
    fn mcp_servers_outside_globs_are_skipped() {
        assert!(mcp_glob_may_match("*", "mcp.overclock.x"));
        assert!(mcp_glob_may_match("mcp.*", "mcp.overclock.x"));
        assert!(mcp_glob_may_match("mcp.overclock.*", "mcp.overclock.x"));
        assert!(mcp_glob_may_match(
            "mcp.overclock.pane_list",
            "mcp.overclock.x"
        ));
        assert!(!mcp_glob_may_match("mcp.overclick.*", "mcp.overclock.x"));
        assert!(!mcp_glob_may_match("fs.*", "mcp.overclock.x"));
    }

    #[tokio::test]
    async fn empty_globs_build_empty_registry() {
        assert!(build_registry(&[], &[], &FullAccess::default()).await.is_empty());
        let code = build_registry(&["fs.*".to_string()], &[], &FullAccess::default()).await;
        assert_eq!(code.names(), ["fs.list", "fs.read", "fs.write"]);
    }
}
