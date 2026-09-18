// crates/core/src/reflex/decide.rs
//! Juiz do reflexo, puro: fala + inventário → perguntas; respostas + limiares
//! → decisão. Sem I/O, para testar de mesa.

use std::collections::BTreeSet;

use super::eye::Inventory;

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
}
