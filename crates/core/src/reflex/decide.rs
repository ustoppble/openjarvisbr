// crates/core/src/reflex/decide.rs
//! Juiz do reflexo, puro: fala + inventário → perguntas; respostas + limiares
//! → decisão. Sem I/O, para testar de mesa.

use std::collections::BTreeSet;

use super::eye::Inventory;
use super::questions::{Question, Questions};

pub const MAX_CANDIDATES: usize = 40;
pub const NONE: &str = "none";

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

/// Apps rodando sempre entram; instalados entram se algum token da fala (≥3
/// letras) for prefixo de alguma palavra do nome. Teto `MAX_CANDIDATES`.
pub fn app_candidates(heard: &str, inv: &Inventory) -> Vec<String> {
    let tokens: Vec<String> = normalize(heard)
        .split(' ')
        .filter(|t| t.chars().count() >= 3)
        .map(str::to_string)
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for app in &inv.running_apps {
        if seen.insert(app.name.clone()) {
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
        let name = normalize(&app.name);
        let hit = tokens.iter().any(|t| name.split(' ').any(|w| w.starts_with(t.as_str())));
        if hit && seen.insert(app.name.clone()) {
            out.push(app.name.clone());
        }
    }
    out.truncate(MAX_CANDIDATES);
    out
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
    q.insert("intent", Question::choice(
        "O que o usuário está pedindo em `state`? Escolha `none` se não for um pedido claro de ação.",
        INTENT_CRITERIA.iter().copied()));
    let apps = app_candidates(s.heard, s.inventory);
    if !apps.is_empty() {
        let mut criteria: Vec<(String, String)> = apps
            .iter()
            .map(|name| (name.clone(), format!("o aplicativo {name}")))
            .collect();
        criteria.push((NONE.into(), "nenhum destes aplicativos".into()));
        q.insert("app", Question::choice("Se for para abrir um aplicativo, qual?", criteria));
    }
    if !s.inventory.sites.is_empty() {
        let mut criteria: Vec<(String, String)> = s
            .inventory
            .sites
            .iter()
            .take(MAX_CANDIDATES)
            .map(|site| (site.name.clone(), format!("o site {} ({})", site.name, site.url)))
            .collect();
        criteria.push((NONE.into(), "nenhum destes sites".into()));
        q.insert("site", Question::choice("Se for para abrir um site, qual?", criteria));
    }
    q.insert("media", Question::choice("Se for controle de mídia, qual ação?", MEDIA_CRITERIA.iter().copied()));
    q.insert("volume", Question::choice("Se for volume, qual ação?", VOLUME_CRITERIA.iter().copied()));
    Some(q)
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
    fn candidatos_rodando_sempre_entram_e_instalados_por_prefixo() {
        let i = inv(&["Safari", "Spotify", "Zed", "Slack"], &["Finder", "Zed"]);
        let c = app_candidates("abre o spo", &i);
        assert!(c.contains(&"Finder".to_string()));
        assert!(c.contains(&"Zed".to_string()));
        assert!(c.contains(&"Spotify".to_string()));
        assert!(!c.contains(&"Slack".to_string()));
        assert!(!c.contains(&"Safari".to_string()));
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
                assert!(criteria.contains_key("Finder"));
                assert!(criteria.contains_key(NONE));
                assert!(!criteria.contains_key("Safari"));
            }
            _ => panic!("app deve ser choice"),
        }
        match &q.0["site"] {
            Question::Choice { criteria, .. } => assert!(criteria.contains_key("YouTube")),
            _ => panic!(),
        }
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
        let q = build_questions(&s).unwrap();
        assert!(!q.0.contains_key("app"));
        assert!(q.0.contains_key("volume"));
    }
}
