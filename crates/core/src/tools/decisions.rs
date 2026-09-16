//! Memória de decisões por ferramenta: o que já não precisa perguntar.
//!
//! Três camadas, nesta ordem: ferramentas que nunca perguntam (leitura e
//! abrir app/site), as aprovadas nesta sessão (um "sim" vale até o app
//! fechar) e as liberadas para sempre (`[tools].always_allow`, gravadas
//! quando o usuário diz "sempre pode" ou "não pergunta mais").

use std::collections::BTreeSet;

use super::{policy, Risk};

/// Ferramentas locais que nunca pedem confirmação, qualquer que seja o risco
/// declarado.
pub const NEVER_ASK: &[&str] = &[
    "app.open",
    "web.open",
    "fs.read",
    "fs.list",
    "sys.volume",
    "media.control",
];

/// Frases (já normalizadas: minúsculas, sem acento) que liberam uma
/// ferramenta para sempre.
const ALWAYS_PHRASES: &[&str] = &[
    "sempre pode",
    "pode sempre",
    "sempre liberado",
    "libera sempre",
    "pode fazer sempre",
    "nao pergunta mais",
    "nao pergunte mais",
    "nao precisa perguntar",
    "nao precisa mais perguntar",
    "nao precisa confirmar",
    "nao precisa mais confirmar",
    "para de perguntar",
    "pare de perguntar",
];

/// Palavra falada → ferramenta, para "sempre pode rodar comandos" sem um
/// pedido pendente.
const TOOL_HINTS: &[(&str, &str)] = &[
    ("comando", "shell.run"),
    ("comandos", "shell.run"),
    ("terminal", "shell.run"),
    ("shell", "shell.run"),
    ("arquivo", "fs.write"),
    ("arquivos", "fs.write"),
    ("agenda", "calendar.create"),
    ("evento", "calendar.create"),
    ("eventos", "calendar.create"),
    ("lembrete", "reminder.set"),
    ("lembretes", "reminder.set"),
];

/// A ferramenta nunca pergunta: leitura local, abrir app/site, volume, mídia
/// e tools MCP de leitura (`mcp.<server>.task_list`).
pub fn never_asks(name: &str) -> bool {
    if NEVER_ASK.contains(&name) {
        return true;
    }
    name.strip_prefix("mcp.")
        .and_then(|rest| rest.split_once('.'))
        .is_some_and(|(_, tool)| policy::mcp_risk(tool) == Risk::Safe)
}

#[derive(Debug, Clone, Default)]
pub struct Decisions {
    session: BTreeSet<String>,
    always: BTreeSet<String>,
}

impl Decisions {
    /// Começa com a lista `[tools].always_allow` do config.
    pub fn new(always_allow: &[String]) -> Self {
        Self {
            session: BTreeSet::new(),
            always: clean(always_allow).into_iter().collect(),
        }
    }

    /// Pode executar sem perguntar?
    pub fn allows(&self, name: &str) -> bool {
        never_asks(name) || self.session.contains(name) || self.always.contains(name)
    }

    /// Um "sim" vale para a mesma ferramenta até o app fechar.
    pub fn approve_for_session(&mut self, name: &str) {
        self.session.insert(name.to_string());
    }

    /// Libera para sempre. `true` se a lista mudou (e precisa ser gravada).
    pub fn allow_always(&mut self, name: &str) -> bool {
        self.session.insert(name.to_string());
        self.always.insert(name.to_string())
    }

    /// Troca a lista permanente (ex.: o usuário removeu uma nas
    /// configurações). Aprovações da sessão das removidas também caem.
    pub fn set_always_allow(&mut self, list: &[String]) {
        let next: BTreeSet<String> = clean(list).into_iter().collect();
        for removed in self.always.difference(&next) {
            self.session.remove(removed);
        }
        self.always = next;
    }

    /// Lista permanente, ordenada, como vai para o config.
    pub fn always_allow(&self) -> Vec<String> {
        self.always.iter().cloned().collect()
    }
}

/// Nomes sem espaços nem vazios, sem duplicatas, na ordem original.
pub fn clean(list: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    list.iter()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty() && seen.insert(name.clone()))
        .collect()
}

/// A fala libera para sempre ("sempre pode", "não pergunta mais")?
pub fn always_allow_intent(text: &str) -> bool {
    let spoken = format!(" {} ", normalize_words(text).join(" "));
    ALWAYS_PHRASES
        .iter()
        .any(|phrase| spoken.contains(&format!(" {phrase} ")))
}

/// Ferramenta citada na fala ("sempre pode rodar comandos" → `shell.run`).
pub fn tool_hint(text: &str) -> Option<&'static str> {
    normalize_words(text).iter().find_map(|word| {
        TOOL_HINTS
            .iter()
            .find(|(hint, _)| hint == word)
            .map(|(_, tool)| *tool)
    })
}

/// Minúsculas, sem acento, quebrado em palavras alfanuméricas.
pub(crate) fn normalize_words(text: &str) -> Vec<String> {
    let folded: String = text
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'ê' | 'è' | 'ë' => 'e',
            'í' | 'î' | 'ì' | 'ï' => 'i',
            'ó' | 'ô' | 'õ' | 'ò' | 'ö' => 'o',
            'ú' | 'û' | 'ù' | 'ü' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect();
    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_tools_and_open_never_ask() {
        let decisions = Decisions::default();
        for name in [
            "app.open",
            "web.open",
            "fs.read",
            "fs.list",
            "sys.volume",
            "media.control",
            "mcp.overclick.task_list",
            "mcp.overclock.pane_read",
        ] {
            assert!(decisions.allows(name), "{name}");
        }
        for name in [
            "shell.run",
            "fs.write",
            "calendar.create",
            "mcp.overclick.task_create",
            "mcp.overclock.listener",
        ] {
            assert!(!decisions.allows(name), "{name}");
        }
    }

    #[test]
    fn session_approval_covers_the_same_tool_only() {
        let mut decisions = Decisions::default();
        assert!(!decisions.allows("shell.run"));
        decisions.approve_for_session("shell.run");
        assert!(decisions.allows("shell.run"));
        assert!(!decisions.allows("fs.write"));
        // Aprovação de sessão não vai para o config.
        assert!(decisions.always_allow().is_empty());
        // Uma sessão nova (app reaberto) começa do zero.
        assert!(!Decisions::default().allows("shell.run"));
    }

    #[test]
    fn always_allow_starts_from_config_and_can_be_revoked() {
        let mut decisions = Decisions::new(&[" shell.run ".into(), String::new()]);
        assert!(decisions.allows("shell.run"));
        assert_eq!(decisions.always_allow(), ["shell.run"]);

        assert!(decisions.allow_always("fs.write"));
        assert!(!decisions.allow_always("fs.write"));
        assert_eq!(decisions.always_allow(), ["fs.write", "shell.run"]);

        decisions.set_always_allow(&["fs.write".into()]);
        assert!(!decisions.allows("shell.run"));
        assert!(decisions.allows("fs.write"));
    }

    #[test]
    fn recognizes_always_allow_phrases() {
        for text in [
            "sempre pode",
            "Pode sempre!",
            "sempre pode rodar comandos",
            "não pergunta mais",
            "Não precisa perguntar, pode rodar",
            "tá, não pergunte mais isso",
        ] {
            assert!(always_allow_intent(text), "{text}");
        }
        for text in [
            "pode",
            "sim",
            "não",
            "sempre que eu pedir, pergunta",
            "pode rodar ls",
            "não pode",
        ] {
            assert!(!always_allow_intent(text), "{text}");
        }
    }

    #[test]
    fn tool_hints_from_speech() {
        assert_eq!(tool_hint("sempre pode rodar comandos"), Some("shell.run"));
        assert_eq!(
            tool_hint("não pergunta mais no terminal"),
            Some("shell.run")
        );
        assert_eq!(
            tool_hint("pode sempre criar evento"),
            Some("calendar.create")
        );
        assert_eq!(tool_hint("sempre pode"), None);
    }
}
