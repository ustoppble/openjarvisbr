//! Olho do reflexo: inventário em memória do que o usuário pode pedir por
//! voz (apps instalados, apps rodando, sites configurados). Nunca faz rede.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

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

/// Quem diz quais apps estão rodando. Trait para o teste não depender do SO.
pub trait RunningProbe: Send + Sync {
    fn running_app_names(&self) -> Vec<String>;
}

/// macOS: `osascript -e 'tell application "System Events" to get name of every process whose background only is false'`.
/// Outros SOs: lista vazia.
pub struct OsRunningProbe;

impl RunningProbe for OsRunningProbe {
    fn running_app_names(&self) -> Vec<String> {
        #[cfg(target_os = "macos")]
        {
            let out = std::process::Command::new("osascript")
                .arg("-e")
                .arg("tell application \"System Events\" to get name of every process whose background only is false")
                .output();
            match out {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .split(", ")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
                _ => Vec::new(),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Vec::new()
        }
    }
}

pub const INSTALLED_EVERY: Duration = Duration::from_secs(300);
pub const RUNNING_EVERY: Duration = Duration::from_secs(3);

/// Pastas padrão de apps por SO.
pub fn default_app_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Applications"));
        dirs.push(PathBuf::from("/System/Applications"));
        dirs.push(PathBuf::from("/System/Applications/Utilities"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join("Applications"));
        }
    }
    dirs
}

/// Acesso barato ao inventário corrente; `Clone` para espalhar pelo engine.
#[derive(Clone)]
pub struct EyeHandle {
    inner: Arc<RwLock<Arc<Inventory>>>,
    kick: tokio::sync::mpsc::UnboundedSender<()>,
}

impl EyeHandle {
    /// O(1): clona o `Arc` do inventário corrente.
    pub fn snapshot(&self) -> Arc<Inventory> {
        Arc::clone(&self.inner.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// Pede um refresh dos rodando fora do intervalo.
    pub fn refresh_now(&self) {
        let _ = self.kick.send(());
    }
}

/// Fábrica do Olho: varre uma vez na largada e mantém uma task em background
/// atualizando os rodando (`running_every`) e os instalados (`installed_every`).
pub struct Eye;

impl Eye {
    pub fn start(sites: Vec<SiteConfig>) -> EyeHandle {
        Self::start_with(
            default_app_dirs(),
            sites,
            Arc::new(OsRunningProbe),
            INSTALLED_EVERY,
            RUNNING_EVERY,
        )
    }

    pub fn start_with(
        dirs: Vec<PathBuf>,
        sites: Vec<SiteConfig>,
        probe: Arc<dyn RunningProbe>,
        installed_every: Duration,
        running_every: Duration,
    ) -> EyeHandle {
        let inventory = Arc::new(Inventory::new(scan_app_dirs(&dirs), sites.clone()));
        let inner = Arc::new(RwLock::new(inventory));
        let (kick, mut kicked) = tokio::sync::mpsc::unbounded_channel::<()>();
        let handle = EyeHandle { inner: Arc::clone(&inner), kick };
        tokio::spawn(async move {
            let mut last_installed = Instant::now();
            let mut tick = tokio::time::interval(running_every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    Some(()) = kicked.recv() => {}
                }
                let probe = Arc::clone(&probe);
                let running = tokio::task::spawn_blocking(move || probe.running_app_names())
                    .await
                    .unwrap_or_default();
                let mut next = (**inner.read().unwrap_or_else(|e| e.into_inner())).clone();
                if last_installed.elapsed() >= installed_every {
                    let dirs = dirs.clone();
                    next.installed_apps = tokio::task::spawn_blocking(move || scan_app_dirs(&dirs))
                        .await
                        .unwrap_or_default();
                    last_installed = Instant::now();
                }
                next.mark_running(&running);
                *inner.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(next);
            }
        });
        handle
    }
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

#[cfg(test)]
mod handle_tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeProbe(Mutex<Vec<String>>);
    impl RunningProbe for FakeProbe {
        fn running_app_names(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    #[tokio::test]
    async fn snapshot_e_barato_e_refresh_atualiza_rodando() {
        let probe = Arc::new(FakeProbe(Mutex::new(vec!["Finder".into()])));
        let eye = Eye::start_with(
            vec![],
            vec![],
            probe.clone(),
            Duration::from_secs(3600),
            Duration::from_millis(20),
        );
        tokio::time::sleep(Duration::from_millis(60)).await;
        let a = eye.snapshot();
        assert_eq!(
            a.running_apps.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            vec!["Finder"]
        );
        *probe.0.lock().unwrap() = vec!["Finder".into(), "Safari".into()];
        eye.refresh_now();
        tokio::time::sleep(Duration::from_millis(60)).await;
        let b = eye.snapshot();
        assert_eq!(b.running_apps.len(), 2);
        // snapshot é Arc: clonar não copia o inventário
        assert!(Arc::ptr_eq(&eye.snapshot(), &eye.snapshot()));
    }
}
