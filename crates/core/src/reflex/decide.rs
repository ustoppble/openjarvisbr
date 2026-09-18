// crates/core/src/reflex/decide.rs
//! Juiz do reflexo, puro: fala + inventário → perguntas; respostas + limiares
//! → decisão. Sem I/O, para testar de mesa.

use std::collections::BTreeSet;

use serde_json::json;

use super::eye::Inventory;
use super::memory::LearnedAction;
use super::questions::{Answers, Question, Questions};
use crate::engine_tools::call_summary;
use crate::tools::ToolCall;

pub const MAX_CANDIDATES: usize = 40;
/// Teto de ações aprendidas oferecidas por pergunta.
pub const MAX_LEARNED: usize = 20;
pub const NONE: &str = "none";
/// Prefixo das chaves da pergunta `learned` (`l0`, `l1`, …): índice em `Inventory.learned`.
pub const LEARNED_KEY_PREFIX: &str = "l";

/// Minúsculas, sem acento, só letras/dígitos/espaço, espaços simples.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let mapped = match ch {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'a',
            'É' | 'È' | 'Ê' | 'Ë' => 'e',
            'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
            'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'o',
            'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
            'Ç' => 'c',
            c if c.is_alphanumeric() => c.to_ascii_lowercase(),
            _ => ' ',
        };
        out.push(mapped);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn name_matches(tokens: &[String], name: &str) -> bool {
    let normalized = normalize(name);
    tokens.iter().any(|token| {
        normalized
            .split(' ')
            .any(|word| word.starts_with(token.as_str()))
    })
}

/// Apps rodando têm prioridade sobre os demais instalados, mas só entram se
/// algum token da fala (≥3 letras) for prefixo de uma palavra do nome. Teto
/// `MAX_CANDIDATES`.
pub fn app_candidates(heard: &str, inv: &Inventory) -> Vec<String> {
    let tokens: Vec<String> = normalize(heard)
        .split(' ')
        .filter(|t| t.chars().count() >= 3)
        .map(str::to_string)
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for app in &inv.running_apps {
        if name_matches(&tokens, &app.name) && seen.insert(app.name.clone()) {
            out.push(app.name.clone());
        }
    }
    for app in &inv.installed_apps {
        if out.len() >= MAX_CANDIDATES {
            break;
        }
        if seen.contains(&app.name) {
            continue;
        }
        if name_matches(&tokens, &app.name) && seen.insert(app.name.clone()) {
            out.push(app.name.clone());
        }
    }
    out.truncate(MAX_CANDIDATES);
    out
}

/// Sites cujo nome compartilha um token/prefixo (≥3 letras) com a fala.
pub fn site_candidates<'a>(heard: &str, inv: &'a Inventory) -> Vec<&'a crate::config::SiteConfig> {
    let tokens = tokens(heard);
    inv.sites
        .iter()
        .filter(|site| name_matches(&tokens, &site.name))
        .take(MAX_CANDIDATES)
        .collect()
}

/// Tokens da fala com ≥3 letras, normalizados.
fn tokens(text: &str) -> Vec<String> {
    normalize(text)
        .split(' ')
        .filter(|t| t.chars().count() >= 3)
        .map(str::to_string)
        .collect()
}

/// Palavras de ligação não identificam uma ação aprendida; sem este filtro,
/// qualquer frase com "que" reaproveitaria ações sem relação com o pedido.
fn learned_tokens(text: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "que", "com", "sem", "por", "para", "pra", "pro", "uma", "uns", "umas", "dos", "das",
        "nos", "nas", "isso", "esse", "essa", "estes", "estas", "aqui", "agora", "favor", "hora",
        "horas", "sao",
    ];
    tokens(text)
        .into_iter()
        .filter(|token| !STOPWORDS.contains(&token.as_str()))
        .collect()
}

/// Ações aprendidas cuja frase compartilha ≥1 token (≥3 letras) com a fala,
/// as `MAX_LEARNED` de maior `count`. O `usize` é o índice em `inv.learned`
/// (estável: vira a chave `l{i}` da pergunta).
pub fn learned_candidates<'a>(heard: &str, inv: &'a Inventory) -> Vec<(usize, &'a LearnedAction)> {
    let heard_tokens: BTreeSet<String> = learned_tokens(heard).into_iter().collect();
    if heard_tokens.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(usize, &LearnedAction)> = inv
        .learned
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            learned_tokens(&a.phrase)
                .iter()
                .any(|t| heard_tokens.contains(t))
        })
        .collect();
    out.sort_by_key(|(_, a)| std::cmp::Reverse(a.count));
    out.truncate(MAX_LEARNED);
    out
}

/// Chave da opção `learned` para o índice `i`.
pub fn learned_key(i: usize) -> String {
    format!("{LEARNED_KEY_PREFIX}{i}")
}

/// Índice de uma chave `l{i}`; `None` para `none` ou chave estranha.
pub fn learned_index(key: &str) -> Option<usize> {
    key.strip_prefix(LEARNED_KEY_PREFIX)?.parse().ok()
}

/// Texto da opção: `tool: resumo ("frase original")`.
pub fn learned_option_text(action: &LearnedAction) -> String {
    let summary = call_summary(&action.to_call(String::new()));
    format!("{summary} (\"{}\")", action.phrase)
}

pub struct Situation<'a> {
    pub heard: &'a str,
    pub inventory: &'a Inventory,
    /// Há chamada `Confirm` esperando resposta.
    pub pending_confirm: bool,
    /// O reflexo já agiu neste turno: só confirmação interessa.
    pub turn_locked: bool,
}

const INTENT_CRITERIA: &[(&str, &str)] = &[
    ("open_app", "O usuário quer abrir ou ir para um aplicativo do computador."),
    ("open_site", "O usuário quer abrir um site ou página na internet."),
    ("media", "O usuário quer controlar a música ou o vídeo: tocar, pausar, próxima, anterior."),
    ("volume", "O usuário quer mudar o volume do computador ou silenciar."),
    ("learned", "O usuário quer repetir uma ação que o assistente já fez antes para ele (ver a pergunta `learned`)."),
    (NONE, "Não é um pedido de ação no computador: conversa, pergunta, outra coisa."),
];

const MEDIA_CRITERIA: &[(&str, &str)] = &[
    ("play", "tocar / continuar"),
    ("pause", "pausar / parar"),
    ("next", "próxima faixa"),
    ("previous", "faixa anterior / voltar"),
    (NONE, "nenhuma destas"),
];

const VOLUME_CRITERIA: &[(&str, &str)] = &[
    ("up", "aumentar o volume"),
    ("down", "diminuir o volume"),
    ("mute", "silenciar / mudo"),
    ("unmute", "tirar do mudo / voltar o som"),
    ("set", "colocar o volume num valor específico"),
    (NONE, "nenhuma destas"),
];

fn confirmation_questions(q: &mut Questions) {
    q.insert("approve", Question::noul(
        "O usuário está dizendo SIM: aprovando, autorizando ou mandando seguir com o pedido pendente.", None));
    q.insert("deny", Question::noul(
        "O usuário está dizendo NÃO: recusando, cancelando ou mandando parar o pedido pendente.", None));
}

/// Perguntas para esta situação. `None` = nada a perguntar.
pub fn build_questions(s: &Situation) -> Option<Questions> {
    if s.heard.trim().is_empty() {
        return None;
    }
    let mut q = Questions::default();
    q.insert("always", Question::noul(
        "O usuário pede para o assistente nunca mais perguntar antes de fazer isso (liberar para sempre).", None));
    if s.pending_confirm {
        confirmation_questions(&mut q);
    }
    if s.turn_locked {
        return if s.pending_confirm { Some(q) } else { None };
    }
    let apps = app_candidates(s.heard, s.inventory);
    let sites = site_candidates(s.heard, s.inventory);
    let learned = learned_candidates(s.heard, s.inventory);
    if !s.pending_confirm && apps.is_empty() && sites.is_empty() && learned.is_empty() {
        return None;
    }
    q.insert("intent", Question::choice(
        "O que o usuário está pedindo em `state`? Escolha `none` se não for um pedido claro de ação.",
        INTENT_CRITERIA.iter().copied()));
    if !apps.is_empty() {
        let mut criteria: Vec<(String, String)> = apps
            .iter()
            .map(|name| (name.clone(), format!("o aplicativo {name}")))
            .collect();
        criteria.push((NONE.into(), "nenhum destes aplicativos".into()));
        q.insert("app", Question::choice("Se for para abrir um aplicativo, qual?", criteria));
    }
    if !sites.is_empty() {
        let mut criteria: Vec<(String, String)> = sites
            .iter()
            .map(|site| (site.name.clone(), format!("o site {} ({})", site.name, site.url)))
            .collect();
        criteria.push((NONE.into(), "nenhum destes sites".into()));
        q.insert("site", Question::choice("Se for para abrir um site, qual?", criteria));
    }
    if !learned.is_empty() {
        let mut criteria: Vec<(String, String)> = learned
            .iter()
            .map(|(i, action)| (learned_key(*i), learned_option_text(action)))
            .collect();
        criteria.push((NONE.into(), "nenhuma destas ações".into()));
        q.insert("learned", Question::choice(
            "Se for para repetir uma ação que o assistente já fez antes, qual?", criteria));
    }
    q.insert("media", Question::choice("Se for controle de mídia, qual ação?", MEDIA_CRITERIA.iter().copied()));
    q.insert("volume", Question::choice("Se for volume, qual ação?", VOLUME_CRITERIA.iter().copied()));
    Some(q)
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub act: f32,
    pub confirm: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Act(ToolCall),
    Approve,
    Deny,
    AlwaysAllow,
    Nothing,
}

pub const REFLEX_CALL_PREFIX: &str = "reflex-";

fn next_call_id() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{REFLEX_CALL_PREFIX}{n}")
}

fn new_call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall { id: next_call_id(), name: name.to_string(), args }
}

/// Primeiro inteiro 0..=100 da fala.
pub fn volume_level(heard: &str) -> Option<u8> {
    normalize(heard)
        .split(' ')
        .filter_map(|t| t.parse::<u32>().ok())
        .find(|n| *n <= 100)
        .map(|n| n as u8)
}

/// Escolha `id` com opção ≠ none e probabilidade ≥ limiar.
fn confident_pick<'a>(a: &'a Answers, id: &str, min: f32) -> Option<&'a str> {
    let (pick, _) = a.choice(id)?;
    if pick == NONE {
        return None;
    }
    let p = a.probability(id, pick).unwrap_or(0.0);
    (p >= min).then_some(pick)
}

/// Tools nativas do reflexo (as perguntas fixas app/site/media/volume).
/// Constante: não é configurável. Ações aprendidas podem sair com qualquer
/// tool: quem barra é o engine, pela política de risco (`Safe` executa,
/// `Confirm` vira `ToolConfirmNeeded`).
pub const REFLEX_TOOLS: &[&str] = &["app.open", "web.open", "sys.volume", "media.control"];

pub fn is_reflex_tool(name: &str) -> bool {
    REFLEX_TOOLS.contains(&name)
}

pub fn decide(s: &Situation, a: &Answers, t: &Thresholds) -> Decision {
    if a.noul("always").unwrap_or(0.0) >= t.confirm {
        return Decision::AlwaysAllow;
    }
    if s.pending_confirm {
        let approve = a.noul("approve").unwrap_or(0.0);
        let deny = a.noul("deny").unwrap_or(0.0);
        if approve >= t.confirm && deny < 0.5 {
            return Decision::Approve;
        }
        if deny >= t.confirm && approve < 0.5 {
            return Decision::Deny;
        }
    }
    if s.turn_locked {
        return Decision::Nothing;
    }
    // A pergunta `learned` já aponta para uma chamada completa e a política
    // de risco ainda será aplicada pelo engine. Não descarte essa escolha só
    // porque a pergunta genérica `intent` divergiu ou ficou abaixo do limiar.
    if let Some(action) = confident_pick(a, "learned", t.act)
        .and_then(learned_index)
        .and_then(|i| s.inventory.learned.get(i))
    {
        return Decision::Act(action.to_call(next_call_id()));
    }
    let Some(intent) = confident_pick(a, "intent", t.act) else {
        return Decision::Nothing;
    };
    match intent {
        "open_app" => match confident_pick(a, "app", t.act) {
            Some(name) => Decision::Act(new_call("app.open", json!({ "name": name }))),
            None => Decision::Nothing,
        },
        "open_site" => {
            let Some(name) = confident_pick(a, "site", t.act) else { return Decision::Nothing };
            match s.inventory.sites.iter().find(|site| site.name == name) {
                Some(site) => Decision::Act(new_call("web.open", json!({ "url": site.url }))),
                None => Decision::Nothing,
            }
        }
        "media" => match confident_pick(a, "media", t.act) {
            Some(action) => Decision::Act(new_call("media.control", json!({ "action": action }))),
            None => Decision::Nothing,
        },
        "volume" => match confident_pick(a, "volume", t.act) {
            Some("set") => match volume_level(s.heard) {
                Some(level) => Decision::Act(new_call("sys.volume", json!({ "action": "set", "level": level }))),
                None => Decision::Nothing,
            },
            Some(action) => Decision::Act(new_call("sys.volume", json!({ "action": action }))),
            None => Decision::Nothing,
        },
        "learned" => {
            let Some(key) = confident_pick(a, "learned", t.act) else { return Decision::Nothing };
            match learned_index(key).and_then(|i| s.inventory.learned.get(i)) {
                Some(action) => Decision::Act(action.to_call(next_call_id())),
                None => Decision::Nothing,
            }
        }
        _ => Decision::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SiteConfig;
    use crate::reflex::eye::{AppEntry, Inventory};

    fn inv(installed: &[&str], running: &[&str]) -> Inventory {
        let mut i = Inventory::new(installed.iter().map(|n| AppEntry::installed(n)).collect(), vec![
            SiteConfig { name: "YouTube".into(), url: "https://youtube.com".into() },
        ]);
        i.mark_running(&running.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        i
    }

    #[test]
    fn normaliza_acentos_e_pontuacao() {
        assert_eq!(normalize("Abre o Safári, por favor!"), "abre o safari por favor");
    }

    #[test]
    fn candidatos_de_app_exigem_token_da_fala_e_respeitam_estado_rodando() {
        let i = inv(&["Safari", "Spotify", "Zed", "Slack"], &["Finder", "Zed"]);
        let c = app_candidates("abre o spo", &i);
        assert_eq!(c, vec!["Spotify".to_string()]);

        let running = app_candidates("volta pro finder", &i);
        assert_eq!(running, vec!["Finder".to_string()]);
    }

    #[test]
    fn candidatos_respeitam_teto() {
        let names: Vec<String> = (0..100).map(|n| format!("App{n:03}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let i = inv(&refs, &[]);
        assert_eq!(app_candidates("abre o app", &i).len(), MAX_CANDIDATES);
    }

    #[test]
    fn tokens_curtos_nao_casam() {
        let i = inv(&["Zed", "Zoom"], &[]);
        assert!(app_candidates("ze", &i).is_empty());
        assert_eq!(app_candidates("zed", &i), vec!["Zed".to_string()]);
    }

    #[test]
    fn fala_vazia_nao_pergunta() {
        let i = inv(&["Safari"], &[]);
        let s = Situation { heard: "  ", inventory: &i, pending_confirm: false, turn_locked: false };
        assert!(build_questions(&s).is_none());
    }

    #[test]
    fn turno_travado_so_pergunta_confirmacao() {
        let i = inv(&["Safari"], &[]);
        let s = Situation { heard: "sim pode", inventory: &i, pending_confirm: true, turn_locked: true };
        let q = build_questions(&s).unwrap();
        let ids: Vec<&String> = q.0.keys().collect();
        assert_eq!(ids, vec!["always", "approve", "deny"]);
    }

    #[test]
    fn turno_travado_sem_pendente_nao_pergunta() {
        let i = inv(&["Safari"], &[]);
        let s = Situation { heard: "sim", inventory: &i, pending_confirm: false, turn_locked: true };
        assert!(build_questions(&s).is_none());
    }

    #[test]
    fn perguntas_completas_incluem_candidatos_e_none() {
        let i = inv(&["Safari", "Spotify"], &["Finder"]);
        let s = Situation { heard: "abre o spotify", inventory: &i, pending_confirm: false, turn_locked: false };
        let q = build_questions(&s).unwrap();
        assert!(q.0.contains_key("intent"));
        assert!(q.0.contains_key("always"));
        assert!(!q.0.contains_key("approve"));
        match &q.0["app"] {
            Question::Choice { criteria, .. } => {
                assert!(criteria.contains_key("Spotify"));
                assert!(criteria.contains_key(NONE));
                assert!(!criteria.contains_key("Finder"));
                assert!(!criteria.contains_key("Safari"));
            }
            _ => panic!("app deve ser choice"),
        }
        assert!(!q.0.contains_key("site"));
        match &q.0["intent"] {
            Question::Choice { criteria, .. } => {
                for k in ["open_app", "open_site", "media", "volume", NONE] { assert!(criteria.contains_key(k), "{k}"); }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn sem_candidato_de_app_nao_pergunta_app() {
        let i = inv(&["Safari"], &[]);
        let s = Situation { heard: "aumenta o volume", inventory: &i, pending_confirm: false, turn_locked: false };
        assert!(build_questions(&s).is_none());
    }

    #[test]
    fn site_so_vira_candidato_quando_a_fala_menciona_o_nome() {
        let i = inv(&[], &[]);
        let conversa = Situation {
            heard: "que horas são?",
            inventory: &i,
            pending_confirm: false,
            turn_locked: false,
        };
        assert!(build_questions(&conversa).is_none());

        let pedido = Situation {
            heard: "abre o youtube",
            inventory: &i,
            pending_confirm: false,
            turn_locked: false,
        };
        assert!(build_questions(&pedido).unwrap().0.contains_key("site"));
    }

    use crate::reflex::questions::Answer;
    use std::collections::BTreeMap;

    fn t() -> Thresholds { Thresholds { act: 0.85, confirm: 0.85 } }

    fn choice(a: &mut Answers, id: &str, pick: &str, p: f32) {
        let mut probs = BTreeMap::new();
        probs.insert(pick.to_string(), p);
        probs.insert(NONE.to_string(), 1.0 - p);
        a.answers.insert(id.into(), Answer::Choice { choice: pick.into(), probabilities: probs, confidence: p });
    }
    fn noul(a: &mut Answers, id: &str, v: f32) {
        a.answers.insert(id.into(), Answer::Noul { noul: v });
    }

    #[test]
    fn abre_app_com_confianca() {
        let i = inv(&["Safari"], &[]);
        let s = Situation { heard: "abre o safari", inventory: &i, pending_confirm: false, turn_locked: false };
        let mut a = Answers::default();
        choice(&mut a, "intent", "open_app", 0.95);
        choice(&mut a, "app", "Safari", 0.97);
        match decide(&s, &a, &t()) {
            Decision::Act(call) => {
                assert_eq!(call.name, "app.open");
                assert_eq!(call.args, json!({"name": "Safari"}));
                assert!(call.id.starts_with(REFLEX_CALL_PREFIX));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn limiar_exato_age_e_abaixo_nao() {
        let i = inv(&["Safari"], &[]);
        let s = Situation { heard: "abre o safari", inventory: &i, pending_confirm: false, turn_locked: false };
        let mut a = Answers::default();
        choice(&mut a, "intent", "open_app", 0.85);
        choice(&mut a, "app", "Safari", 0.85);
        assert!(matches!(decide(&s, &a, &t()), Decision::Act(_)));
        let mut b = Answers::default();
        choice(&mut b, "intent", "open_app", 0.95);
        choice(&mut b, "app", "Safari", 0.84);
        assert!(matches!(decide(&s, &b, &t()), Decision::Nothing));
    }

    #[test]
    fn alvo_none_nao_age() {
        let i = inv(&["Safari"], &[]);
        let s = Situation { heard: "abre aí", inventory: &i, pending_confirm: false, turn_locked: false };
        let mut a = Answers::default();
        choice(&mut a, "intent", "open_app", 0.95);
        choice(&mut a, "app", NONE, 0.9);
        assert!(matches!(decide(&s, &a, &t()), Decision::Nothing));
    }

    #[test]
    fn site_so_da_lista_com_url_do_config() {
        let i = inv(&[], &[]);
        let s = Situation { heard: "abre o youtube", inventory: &i, pending_confirm: false, turn_locked: false };
        let mut a = Answers::default();
        choice(&mut a, "intent", "open_site", 0.95);
        choice(&mut a, "site", "YouTube", 0.95);
        match decide(&s, &a, &t()) {
            Decision::Act(call) => {
                assert_eq!(call.name, "web.open");
                assert_eq!(call.args, json!({"url": "https://youtube.com"}));
            }
            other => panic!("{other:?}"),
        }
        let mut b = Answers::default();
        choice(&mut b, "intent", "open_site", 0.95);
        choice(&mut b, "site", "Globo", 0.95); // não está no config
        assert!(matches!(decide(&s, &b, &t()), Decision::Nothing));
    }

    #[test]
    fn midia_e_volume() {
        let i = inv(&[], &[]);
        let s = Situation { heard: "próxima música", inventory: &i, pending_confirm: false, turn_locked: false };
        let mut a = Answers::default();
        choice(&mut a, "intent", "media", 0.95);
        choice(&mut a, "media", "next", 0.95);
        assert!(matches!(decide(&s, &a, &t()), Decision::Act(c) if c.name == "media.control" && c.args == json!({"action": "next"})));

        let s2 = Situation { heard: "coloca o volume em 30", inventory: &i, pending_confirm: false, turn_locked: false };
        let mut b = Answers::default();
        choice(&mut b, "intent", "volume", 0.95);
        choice(&mut b, "volume", "set", 0.95);
        assert!(matches!(decide(&s2, &b, &t()), Decision::Act(c) if c.args == json!({"action": "set", "level": 30})));

        let s3 = Situation { heard: "aumenta o volume", inventory: &i, pending_confirm: false, turn_locked: false };
        let mut c = Answers::default();
        choice(&mut c, "intent", "volume", 0.95);
        choice(&mut c, "volume", "up", 0.95);
        assert!(matches!(decide(&s3, &c, &t()), Decision::Act(c) if c.args == json!({"action": "up"})));

        // set sem número na fala: nada
        let s4 = Situation { heard: "coloca o volume", inventory: &i, pending_confirm: false, turn_locked: false };
        assert!(matches!(decide(&s4, &b, &t()), Decision::Nothing));
    }

    #[test]
    fn confirmacao_por_voz() {
        let i = inv(&[], &[]);
        let s = Situation { heard: "pode ir", inventory: &i, pending_confirm: true, turn_locked: false };
        let mut a = Answers::default();
        noul(&mut a, "approve", 0.9); noul(&mut a, "deny", 0.1); noul(&mut a, "always", 0.0);
        assert!(matches!(decide(&s, &a, &t()), Decision::Approve));
        let mut b = Answers::default();
        noul(&mut b, "approve", 0.1); noul(&mut b, "deny", 0.92); noul(&mut b, "always", 0.0);
        assert!(matches!(decide(&s, &b, &t()), Decision::Deny));
        // empate: nada
        let mut c = Answers::default();
        noul(&mut c, "approve", 0.9); noul(&mut c, "deny", 0.6);
        assert!(matches!(decide(&s, &c, &t()), Decision::Nothing));
        // sem pendente, approve alto é ignorado
        let s2 = Situation { heard: "pode ir", inventory: &i, pending_confirm: false, turn_locked: false };
        assert!(matches!(decide(&s2, &a, &t()), Decision::Nothing));
    }

    #[test]
    fn sempre_pode_vence_o_resto() {
        let i = inv(&["Safari"], &[]);
        let s = Situation { heard: "sempre pode abrir", inventory: &i, pending_confirm: true, turn_locked: false };
        let mut a = Answers::default();
        noul(&mut a, "always", 0.9); noul(&mut a, "approve", 0.9); noul(&mut a, "deny", 0.0);
        choice(&mut a, "intent", "open_app", 0.99);
        choice(&mut a, "app", "Safari", 0.99);
        assert!(matches!(decide(&s, &a, &t()), Decision::AlwaysAllow));
    }

    #[test]
    fn turno_travado_nunca_age() {
        let i = inv(&["Safari"], &[]);
        let s = Situation { heard: "abre o safari", inventory: &i, pending_confirm: false, turn_locked: true };
        let mut a = Answers::default();
        choice(&mut a, "intent", "open_app", 0.99);
        choice(&mut a, "app", "Safari", 0.99);
        assert!(matches!(decide(&s, &a, &t()), Decision::Nothing));
    }

    #[test]
    fn extrai_nivel_de_volume() {
        assert_eq!(volume_level("coloca o volume em 30"), Some(30));
        assert_eq!(volume_level("volume 100 por cento"), Some(100));
        assert_eq!(volume_level("volume em 250"), None);
        assert_eq!(volume_level("aumenta o volume"), None);
    }

    #[test]
    fn so_tools_seguras_saem_do_reflexo() {
        for name in REFLEX_TOOLS {
            assert!(crate::tools::decisions::never_asks(name), "{name} precisa estar em NEVER_ASK");
            assert!(is_reflex_tool(name));
        }
        assert!(!is_reflex_tool("shell.run"));
        assert!(!is_reflex_tool("fs.read")); // NEVER_ASK, mas fora do reflexo
    }

    use crate::reflex::memory::LearnedAction;

    fn learned(phrase: &str, tool: &str, args: serde_json::Value, count: u32) -> LearnedAction {
        LearnedAction { phrase: phrase.into(), tool: tool.into(), args, count, last_used: 0 }
    }

    fn inv_learned(actions: Vec<LearnedAction>) -> Inventory {
        Inventory::new(vec![], vec![]).with_learned(actions)
    }

    #[test]
    fn candidatas_aprendidas_por_token_em_comum_top_por_count() {
        let mut actions: Vec<LearnedAction> = (0..30)
            .map(|n| learned(&format!("abre o site {n}"), "web.open", json!({"url": format!("https://s{n}.com")}), n))
            .collect();
        actions.push(learned("toca a playlist de foco", "media.control", json!({"action": "play"}), 99));
        actions.push(learned("ab", "web.open", json!({"url": "https://curto.com"}), 5)); // token curto
        let i = inv_learned(actions);
        let c = learned_candidates("abre a globo", &i);
        assert_eq!(c.len(), MAX_LEARNED);
        // ordenadas por count desc: a primeira é "abre o site 29" (índice 29)
        assert_eq!(c[0].0, 29);
        assert_eq!(c[0].1.count, 29);
        assert!(c.iter().all(|(_, a)| a.tool == "web.open"));
        assert!(c.iter().all(|(i, _)| *i < 30));
        // acento e caixa não atrapalham
        let c2 = learned_candidates("TOCA a Playlíst", &i);
        assert_eq!(c2.len(), 1);
        assert_eq!(c2[0].0, 30);
        // fala só com tokens curtos: nada
        assert!(learned_candidates("ab o", &i).is_empty());
        // sem memória: nada
        assert!(learned_candidates("abre a globo", &inv_learned(vec![])).is_empty());
    }

    #[test]
    fn palavras_de_conversa_nao_criam_candidata_aprendida() {
        let i = inv_learned(vec![learned(
            "confere as horas e diz se são certas",
            "media.control",
            json!({"action": "play"}),
            1,
        )]);

        assert!(learned_candidates("que horas são?", &i).is_empty());
    }

    #[test]
    fn pergunta_learned_so_com_candidatas_e_com_chaves_estaveis() {
        let i = inv_learned(vec![
            learned("toca a playlist", "media.control", json!({"action": "play"}), 1),
            learned("abre a globo", "web.open", json!({"url": "https://globo.com"}), 3),
        ]);
        let s = Situation { heard: "abre a globo aí", inventory: &i, pending_confirm: false, turn_locked: false };
        let q = build_questions(&s).unwrap();
        match &q.0["learned"] {
            Question::Choice { criteria, .. } => {
                assert_eq!(criteria.len(), 2);
                assert!(criteria.contains_key(NONE));
                assert!(!criteria.contains_key("l0"));
                assert_eq!(criteria["l1"], "web.open: https://globo.com (\"abre a globo\")");
            }
            _ => panic!("learned deve ser choice"),
        }
        match &q.0["intent"] {
            Question::Choice { criteria, .. } => assert!(criteria.contains_key("learned")),
            _ => panic!(),
        }
        // sem candidata: sem pergunta
        let s2 = Situation { heard: "aumenta o volume", inventory: &i, pending_confirm: false, turn_locked: false };
        assert!(build_questions(&s2).is_none());
    }

    #[test]
    fn intent_learned_com_alvo_age_com_os_args_gravados() {
        let i = inv_learned(vec![
            learned("toca a playlist", "media.control", json!({"action": "play"}), 1),
            learned("roda o build", "shell.run", json!({"command": "cargo build"}), 3),
        ]);
        let s = Situation { heard: "roda o build", inventory: &i, pending_confirm: false, turn_locked: false };
        let mut a = Answers::default();
        choice(&mut a, "intent", "learned", 0.95);
        choice(&mut a, "learned", "l1", 0.9);
        match decide(&s, &a, &t()) {
            Decision::Act(call) => {
                // tool fora de REFLEX_TOOLS sai daqui: o engine aplica a política de risco
                assert_eq!(call.name, "shell.run");
                assert_eq!(call.args, json!({"command": "cargo build"}));
                assert!(call.id.starts_with(REFLEX_CALL_PREFIX));
            }
            other => panic!("{other:?}"),
        }
        // none: nada
        let mut b = Answers::default();
        choice(&mut b, "intent", "learned", 0.95);
        choice(&mut b, "learned", NONE, 0.9);
        assert!(matches!(decide(&s, &b, &t()), Decision::Nothing));
        // alvo abaixo do limiar: nada
        let mut c = Answers::default();
        choice(&mut c, "intent", "learned", 0.95);
        choice(&mut c, "learned", "l1", 0.5);
        assert!(matches!(decide(&s, &c, &t()), Decision::Nothing));
        // chave que não existe mais (esquecida): nada
        let mut d = Answers::default();
        choice(&mut d, "intent", "learned", 0.95);
        choice(&mut d, "learned", "l7", 0.95);
        assert!(matches!(decide(&s, &d, &t()), Decision::Nothing));
        // intent learned sem resposta learned: nada
        let mut e = Answers::default();
        choice(&mut e, "intent", "learned", 0.95);
        assert!(matches!(decide(&s, &e, &t()), Decision::Nothing));
    }

    #[test]
    fn learned_confiante_vence_intent_divergente_sem_alvo_acionavel() {
        let i = inv_learned(vec![learned(
            "abre a globo",
            "web.open",
            json!({"site": "globo"}),
            3,
        )]);
        let s = Situation { heard: "abre a globo", inventory: &i, pending_confirm: false, turn_locked: false };
        let mut a = Answers::default();
        choice(&mut a, "intent", "open_app", 0.66);
        choice(&mut a, "app", "Google Chrome", 0.49);
        choice(&mut a, "learned", "l0", 0.97);

        assert!(matches!(
            decide(&s, &a, &t()),
            Decision::Act(call)
                if call.name == "web.open" && call.args == json!({"site": "globo"})
        ));
    }

    #[test]
    fn chaves_learned_vao_e_voltam() {
        assert_eq!(learned_key(7), "l7");
        assert_eq!(learned_index("l7"), Some(7));
        assert_eq!(learned_index(NONE), None);
        assert_eq!(learned_index("lx"), None);
        assert_eq!(learned_index("Safari"), None);
    }
}
