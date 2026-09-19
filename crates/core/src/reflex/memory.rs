//! Memória de ações do reflexo: toda chamada de tool que o modelo executou
//! com sucesso vira uma `LearnedAction`, oferecida ao juiz na próxima fala
//! parecida. Persistida em `~/.config/jarvis/reflex_memory.toml`, nunca no
//! config.toml. Spec: adendo "Memória de ações" (2026-09-18).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::tools::ToolCall;

/// Teto de ações lembradas. Ao passar, sai a de menor `count` mais antiga.
pub const MAX_ACTIONS: usize = 200;

/// Chaves de args que impedem o aprendizado (comparação sem caixa, por
/// substring, em qualquer nível do JSON).
pub const SENSITIVE_KEYS: &[&str] = &["token", "key", "password", "secret", "authorization"];

/// Uma ação que o assistente já fez para o usuário.
#[derive(Debug, Clone, PartialEq)]
pub struct LearnedAction {
    /// O que o usuário disse quando a chamada chegou.
    pub phrase: String,
    pub tool: String,
    pub args: serde_json::Value,
    /// Quantas vezes essa (tool, args) foi executada com sucesso.
    pub count: u32,
    /// Unix epoch (segundos) da última execução.
    pub last_used: u64,
}

impl LearnedAction {
    /// Reconstrói a chamada com os args gravados e o id pedido.
    pub fn to_call(&self, id: String) -> ToolCall {
        ToolCall {
            id,
            name: self.tool.clone(),
            args: self.args.clone(),
        }
    }
}

/// `true` se alguma chave (em qualquer profundidade) contém um termo sensível.
pub fn has_sensitive_args(args: &serde_json::Value) -> bool {
    match args {
        serde_json::Value::Object(map) => map.iter().any(|(k, v)| {
            let key = k.to_ascii_lowercase();
            SENSITIVE_KEYS.iter().any(|s| key.contains(s)) || has_sensitive_args(v)
        }),
        serde_json::Value::Array(items) => items.iter().any(has_sensitive_args),
        _ => false,
    }
}

/// Segundos desde a época Unix (0 se o relógio estiver antes de 1970).
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug)]
pub enum MemoryError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    Serialize(toml::ser::Error),
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryError::Io(e) => write!(f, "memória do reflexo: {e}"),
            MemoryError::Parse(e) => write!(f, "memória do reflexo inválida: {e}"),
            MemoryError::Serialize(e) => write!(f, "memória do reflexo não serializa: {e}"),
        }
    }
}

impl std::error::Error for MemoryError {}

impl From<std::io::Error> for MemoryError {
    fn from(e: std::io::Error) -> Self {
        MemoryError::Io(e)
    }
}

/// Linha do arquivo TOML: `args` vai como string JSON.
#[derive(Debug, Serialize, Deserialize)]
struct ActionRow {
    phrase: String,
    tool: String,
    args: String,
    #[serde(default = "one")]
    count: u32,
    #[serde(default)]
    last_used: u64,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MemoryFile {
    #[serde(default)]
    actions: Vec<ActionRow>,
}

/// Lista ordenada de ações aprendidas (ordem estável: o índice serve de chave).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Memory {
    actions: Vec<LearnedAction>,
}

impl Memory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_actions(actions: Vec<LearnedAction>) -> Self {
        let mut m = Self { actions };
        m.enforce_cap();
        m
    }

    pub fn actions(&self) -> &[LearnedAction] {
        &self.actions
    }

    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Aprende `call` dita por `phrase`. Mesma (tool, args) já existente:
    /// incrementa `count`, atualiza `last_used` e troca `phrase` se a nova for
    /// mais curta. Devolve `false` (e não muda nada) se `phrase` for vazia ou
    /// se os args tiverem chave sensível.
    pub fn learn(&mut self, phrase: &str, call: &ToolCall) -> bool {
        let phrase = phrase.trim();
        if phrase.is_empty() || has_sensitive_args(&call.args) {
            return false;
        }
        let now = now_secs();
        if let Some(existing) = self
            .actions
            .iter_mut()
            .find(|a| a.tool == call.name && a.args == call.args)
        {
            existing.count = existing.count.saturating_add(1);
            existing.last_used = now;
            if phrase.chars().count() < existing.phrase.chars().count() {
                existing.phrase = phrase.to_string();
            }
            return true;
        }
        self.actions.push(LearnedAction {
            phrase: phrase.to_string(),
            tool: call.name.clone(),
            args: call.args.clone(),
            count: 1,
            last_used: now,
        });
        self.enforce_cap();
        true
    }

    /// Junta o que outra instância gravou (app e CLI compartilham o mesmo
    /// arquivo; sem isto o último a salvar apagava o que o outro aprendeu).
    /// Entradas novas entram; (tool, args) repetida fica com o maior `count`,
    /// o `last_used` mais recente e a frase mais curta. Devolve quantas
    /// entraram. Teto reaplicado.
    pub fn merge_from(&mut self, other: &Memory) -> usize {
        let mut added = 0;
        for theirs in &other.actions {
            match self
                .actions
                .iter_mut()
                .find(|a| a.tool == theirs.tool && a.args == theirs.args)
            {
                Some(mine) => {
                    mine.count = mine.count.max(theirs.count);
                    mine.last_used = mine.last_used.max(theirs.last_used);
                    if !theirs.phrase.trim().is_empty()
                        && theirs.phrase.chars().count() < mine.phrase.chars().count()
                    {
                        mine.phrase = theirs.phrase.clone();
                    }
                }
                None => {
                    self.actions.push(theirs.clone());
                    added += 1;
                }
            }
        }
        self.enforce_cap();
        added
    }

    /// Esquece a ação na posição `index`. `None` se não existir.
    pub fn forget(&mut self, index: usize) -> Option<LearnedAction> {
        (index < self.actions.len()).then(|| self.actions.remove(index))
    }

    pub fn clear(&mut self) {
        self.actions.clear();
    }

    fn enforce_cap(&mut self) {
        while self.actions.len() > MAX_ACTIONS {
            let victim = self
                .actions
                .iter()
                .enumerate()
                .min_by_key(|(_, a)| (a.count, a.last_used))
                .map(|(i, _)| i);
            match victim {
                Some(i) => {
                    self.actions.remove(i);
                }
                None => break,
            }
        }
    }

    /// Lê o arquivo. Arquivo inexistente = memória vazia. Linhas com `args`
    /// que não são JSON válido são ignoradas.
    pub fn load(path: &Path) -> Result<Self, MemoryError> {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(e) => return Err(MemoryError::Io(e)),
        };
        let file: MemoryFile = toml::from_str(&raw).map_err(MemoryError::Parse)?;
        let actions = file
            .actions
            .into_iter()
            .filter_map(|row| {
                let args = serde_json::from_str(&row.args).ok()?;
                Some(LearnedAction {
                    phrase: row.phrase,
                    tool: row.tool,
                    args,
                    count: row.count,
                    last_used: row.last_used,
                })
            })
            .collect();
        Ok(Self::from_actions(actions))
    }

    /// Grava em TOML (`[[actions]]`), criando a pasta se preciso.
    pub fn save(&self, path: &Path) -> Result<(), MemoryError> {
        let file = MemoryFile {
            actions: self
                .actions
                .iter()
                .map(|a| ActionRow {
                    phrase: a.phrase.clone(),
                    tool: a.tool.clone(),
                    args: a.args.to_string(),
                    count: a.count,
                    last_used: a.last_used,
                })
                .collect(),
        };
        let text = toml::to_string(&file).map_err(MemoryError::Serialize)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)?;
        Ok(())
    }

    /// `~/.config/jarvis/reflex_memory.toml`; `None` sem `HOME`.
    pub fn default_path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join(".config")
                .join("jarvis")
                .join("reflex_memory.toml"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "m-1".into(),
            name: name.into(),
            args,
        }
    }

    #[test]
    fn aprende_acao_nova() {
        let mut m = Memory::new();
        assert!(m.learn(
            "abre a globo",
            &call("web.open", json!({"url": "https://globo.com"}))
        ));
        assert_eq!(m.len(), 1);
        let a = &m.actions()[0];
        assert_eq!(a.phrase, "abre a globo");
        assert_eq!(a.tool, "web.open");
        assert_eq!(a.args, json!({"url": "https://globo.com"}));
        assert_eq!(a.count, 1);
        assert!(a.last_used > 0);
    }

    #[test]
    fn mesma_tool_e_args_incrementa_e_guarda_frase_mais_curta() {
        let mut m = Memory::new();
        let c = call("web.open", json!({"url": "https://globo.com"}));
        assert!(m.learn("abre o site da globo por favor", &c));
        assert!(m.learn("abre a globo", &c));
        assert!(m.learn("abre o portal da globo agora", &c));
        assert_eq!(m.len(), 1);
        assert_eq!(m.actions()[0].count, 3);
        assert_eq!(m.actions()[0].phrase, "abre a globo");
        // args diferentes = ação diferente
        assert!(m.learn(
            "abre o uol",
            &call("web.open", json!({"url": "https://uol.com.br"}))
        ));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn frase_vazia_nao_aprende() {
        let mut m = Memory::new();
        assert!(!m.learn("   ", &call("web.open", json!({"url": "x"}))));
        assert!(m.is_empty());
    }

    #[test]
    fn args_sensiveis_nao_sao_aprendidos() {
        let mut m = Memory::new();
        for args in [
            json!({"token": "abc"}),
            json!({"api_key": "abc"}),
            json!({"Password": "abc"}),
            json!({"client_secret": "abc"}),
            json!({"headers": {"Authorization": "Bearer x"}}),
            json!({"items": [{"apiKey": "x"}]}),
        ] {
            assert!(
                !m.learn("faz algo", &call("http.get", args.clone())),
                "{args}"
            );
        }
        assert!(m.is_empty());
    }

    /// Regra conservadora: substring da chave ("keyboard" contém "key").
    #[test]
    fn substring_de_chave_sensivel_bloqueia() {
        let mut m = Memory::new();
        assert!(!m.learn(
            "abre",
            &call(
                "web.open",
                json!({"url": "https://x.com", "keyboard": true})
            )
        ));
        assert!(has_sensitive_args(&json!({"secretly": 1})));
        assert!(!has_sensitive_args(
            &json!({"url": "https://x.com?token=1"})
        ));
    }

    #[test]
    fn teto_descarta_menor_count_mais_antiga() {
        let mut m = Memory::new();
        for n in 0..MAX_ACTIONS {
            let c = call("web.open", json!({"url": format!("https://s{n}.com")}));
            assert!(m.learn(&format!("abre s{n}"), &c));
        }
        assert_eq!(m.len(), MAX_ACTIONS);
        // s0 fica popular; s1 é a mais antiga com count 1
        let s0 = call("web.open", json!({"url": "https://s0.com"}));
        m.learn("abre s0", &s0);
        // força ordem temporal: s1 fica mais velha que as outras
        m.actions[1].last_used = 1;
        let nova = call("web.open", json!({"url": "https://nova.com"}));
        assert!(m.learn("abre nova", &nova));
        assert_eq!(m.len(), MAX_ACTIONS);
        assert!(m
            .actions()
            .iter()
            .any(|a| a.args == json!({"url": "https://nova.com"})));
        assert!(m
            .actions()
            .iter()
            .any(|a| a.args == json!({"url": "https://s0.com"})));
        assert!(!m
            .actions()
            .iter()
            .any(|a| a.args == json!({"url": "https://s1.com"})));
    }

    #[test]
    fn merge_junta_o_que_a_outra_instancia_aprendeu() {
        let mut a = Memory::new();
        a.learn("abre o safari por favor", &call("app.open", json!({"name": "Safari"})));
        a.learn("abre o safari por favor", &call("app.open", json!({"name": "Safari"})));
        let mut b = Memory::new();
        b.learn("abre o safari", &call("app.open", json!({"name": "Safari"})));
        b.learn("abre a globo", &call("web.open", json!({"url": "https://globo.com"})));
        assert_eq!(a.merge_from(&b), 1, "só a globo é nova");
        assert_eq!(a.len(), 2);
        let safari = &a.actions()[0];
        assert_eq!(safari.count, 2, "maior count, sem somar a base comum");
        assert_eq!(safari.phrase, "abre o safari", "frase mais curta vence");
        assert_eq!(a.actions()[1].phrase, "abre a globo");
        // idempotente
        assert_eq!(a.merge_from(&b), 0);
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn merge_respeita_o_teto() {
        let mut a = Memory::new();
        for n in 0..MAX_ACTIONS {
            a.learn(&format!("a{n}"), &call("web.open", json!({"url": format!("https://a{n}.com")})));
        }
        let mut b = Memory::new();
        b.learn("b", &call("web.open", json!({"url": "https://b.com"})));
        assert_eq!(a.merge_from(&b), 1);
        assert_eq!(a.len(), MAX_ACTIONS);
    }

    #[test]
    fn esquece_por_indice_e_tudo() {
        let mut m = Memory::new();
        m.learn("a", &call("web.open", json!({"url": "https://a.com"})));
        m.learn("b", &call("web.open", json!({"url": "https://b.com"})));
        assert!(m.forget(5).is_none());
        let gone = m.forget(0).unwrap();
        assert_eq!(gone.phrase, "a");
        assert_eq!(m.len(), 1);
        assert_eq!(m.actions()[0].phrase, "b");
        m.clear();
        assert!(m.is_empty());
    }

    #[test]
    fn grava_e_le_toml_com_args_como_json_string() {
        let dir = std::env::temp_dir().join(format!("reflex-memory-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join("reflex_memory.toml");
        let mut m = Memory::new();
        m.learn(
            "abre a globo",
            &call("web.open", json!({"url": "https://globo.com"})),
        );
        m.learn(
            "abre a globo",
            &call("web.open", json!({"url": "https://globo.com"})),
        );
        m.learn(
            "volume 30",
            &call("sys.volume", json!({"action": "set", "level": 30})),
        );
        m.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[actions]]"), "{text}");
        // o `toml` escolhe aspas simples (literal) quando o JSON tem aspas duplas
        assert!(
            text.contains("args = '{\"url\":\"https://globo.com\"}'")
                || text.contains("args = \"{\\\"url\\\":\\\"https://globo.com\\\"}\""),
            "{text}"
        );
        let back = Memory::load(&path).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.actions()[0].count, 2);
        assert_eq!(
            back.actions()[1].args,
            json!({"action": "set", "level": 30})
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arquivo_inexistente_e_memoria_vazia() {
        let path =
            std::env::temp_dir().join(format!("reflex-memory-nada-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(Memory::load(&path).unwrap().is_empty());
    }

    #[test]
    fn linha_com_args_invalidos_e_ignorada_e_toml_quebrado_da_erro() {
        let dir = std::env::temp_dir().join(format!("reflex-memory-ruim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.toml");
        std::fs::write(
            &path,
            "[[actions]]\nphrase = \"a\"\ntool = \"web.open\"\nargs = \"nao é json\"\n\n\
             [[actions]]\nphrase = \"b\"\ntool = \"web.open\"\nargs = \"{\\\"url\\\":\\\"https://b.com\\\"}\"\n",
        )
        .unwrap();
        let m = Memory::load(&path).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m.actions()[0].phrase, "b");
        assert_eq!(m.actions()[0].count, 1);
        std::fs::write(&path, "isso não é toml = = =").unwrap();
        assert!(matches!(Memory::load(&path), Err(MemoryError::Parse(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn caminho_padrao_fica_em_config_jarvis() {
        let p = Memory::default_path().unwrap();
        assert!(
            p.ends_with(".config/jarvis/reflex_memory.toml"),
            "{}",
            p.display()
        );
    }

    #[test]
    fn to_call_reconstroi_a_chamada() {
        let a = LearnedAction {
            phrase: "x".into(),
            tool: "web.open".into(),
            args: json!({"url": "https://x.com"}),
            count: 1,
            last_used: 0,
        };
        let c = a.to_call("reflex-9".into());
        assert_eq!(c.id, "reflex-9");
        assert_eq!(c.name, "web.open");
        assert_eq!(c.args, json!({"url": "https://x.com"}));
    }
}
