//! Olho do reflexo: inventário em memória do que o usuário pode pedir por
//! voz (apps instalados, apps rodando, sites configurados). Nunca faz rede.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

// TEMP até card A: `crate::config::SiteConfig` ainda não existe; este stub
// tem o mesmo shape do plano (Task 4) e deve ser apagado quando o card A
// mesclar, trocando por `use crate::config::SiteConfig;`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SiteConfig {
    pub name: String,
    pub url: String,
}

/// Um app conhecido pelo Olho: nome do bundle (sem `.app`) e se está rodando.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    pub bundle_id: Option<String>,
    pub running: bool,
}

impl AppEntry {
    pub fn installed(name: &str) -> Self {
        Self { name: name.to_string(), bundle_id: None, running: false }
    }
}

/// Inventário corrente: o que o juiz pode oferecer como opção.
#[derive(Debug, Clone)]
pub struct Inventory {
    pub installed_apps: Vec<AppEntry>,
    pub running_apps: Vec<AppEntry>,
    pub sites: Vec<SiteConfig>,
    pub refreshed_at: Instant,
}

impl Inventory {
    pub fn new(installed_apps: Vec<AppEntry>, sites: Vec<SiteConfig>) -> Self {
        Self { installed_apps, running_apps: Vec::new(), sites, refreshed_at: Instant::now() }
    }

    /// Atualiza `running` nos instalados e reconstrói `running_apps` (inclui
    /// apps rodando que não estão nas pastas varridas, ex.: Finder).
    pub fn mark_running(&mut self, running: &[String]) {
        let set: BTreeSet<&str> = running.iter().map(String::as_str).collect();
        for app in &mut self.installed_apps {
            app.running = set.contains(app.name.as_str());
        }
        self.running_apps = set
            .iter()
            .map(|name| {
                self.installed_apps
                    .iter()
                    .find(|a| a.name == *name)
                    .cloned()
                    .unwrap_or_else(|| AppEntry {
                        name: name.to_string(),
                        bundle_id: None,
                        running: true,
                    })
            })
            .collect();
        self.refreshed_at = Instant::now();
    }
}

/// "Visual Studio Code.app" → "Visual Studio Code"; qualquer outra coisa → None.
pub fn app_name_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(".app").map(str::to_string)
}

/// Varre as pastas (um nível) e devolve os `.app` sem repetição, ordenados.
pub fn scan_app_dirs(dirs: &[PathBuf]) -> Vec<AppEntry> {
    let mut names = BTreeSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            if let Some(name) = app_name_from_path(&entry.path()) {
                names.insert(name);
            }
        }
    }
    names.into_iter().map(|n| AppEntry::installed(&n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nome_do_app_vem_do_bundle_sem_extensao() {
        assert_eq!(
            app_name_from_path(Path::new("/Applications/Visual Studio Code.app")).as_deref(),
            Some("Visual Studio Code")
        );
        assert_eq!(app_name_from_path(Path::new("/Applications/README.txt")), None);
    }

    #[test]
    fn varre_pastas_ignora_nao_app_e_ordena_sem_repetir() {
        let dir = std::env::temp_dir().join(format!("eye-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for name in ["Zed.app", "Safari.app", "notas.txt", "Safari.app"] {
            let p = dir.join(name);
            if name.ends_with(".app") {
                std::fs::create_dir_all(&p).unwrap();
            } else {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(&p, b"").unwrap();
            }
        }
        let other = dir.join("sub");
        std::fs::create_dir_all(other.join("Safari.app")).unwrap();
        let apps = scan_app_dirs(&[dir.clone(), other]);
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Safari", "Zed"]);
        assert!(apps.iter().all(|a| !a.running));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inventario_marca_rodando() {
        let mut inv = Inventory::new(
            vec![AppEntry::installed("Safari"), AppEntry::installed("Zed")],
            vec![],
        );
        inv.mark_running(&["Zed".to_string(), "Finder".to_string()]);
        assert!(inv.installed_apps.iter().find(|a| a.name == "Zed").unwrap().running);
        assert!(!inv.installed_apps.iter().find(|a| a.name == "Safari").unwrap().running);
        let running: Vec<&str> = inv.running_apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(running, vec!["Finder", "Zed"]);
    }
}
