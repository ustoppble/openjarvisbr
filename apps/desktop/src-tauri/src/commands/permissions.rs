// Permissões do macOS na aba Ferramentas (JRV-58): Acessibilidade (checagem
// e prompt nativos via AXIsProcessTrusted*), Automação (um AppleScript
// inofensivo por app-alvo, que faz o macOS pedir e listar o OpenJarvisBR no
// painel), ícone do app para arrastar até a lista do painel e o painel de
// Privacidade aberto com a janela de configurações fixa no topo.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

const APP_ICON_PNG: &[u8] = include_bytes!("../../icons/128x128.png");

/// Status de uma permissão, como a interface mostra.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Grant {
    Concedida,
    Pendente,
    Negada,
    NaoInstalado,
    Erro,
}

#[cfg(target_os = "macos")]
mod ax {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct Opaque {
        _private: [u8; 0],
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        pub fn AXIsProcessTrusted() -> bool;
        pub fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        pub static kAXTrustedCheckOptionPrompt: *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub static kCFBooleanTrue: *const c_void;
        pub static kCFTypeDictionaryKeyCallBacks: Opaque;
        pub static kCFTypeDictionaryValueCallBacks: Opaque;
        pub fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            count: isize,
            key_callbacks: *const Opaque,
            value_callbacks: *const Opaque,
        ) -> *const c_void;
        pub fn CFRelease(cf: *const c_void);
    }

    pub fn trusted() -> bool {
        // SAFETY: função C sem argumentos, sem estado compartilhado.
        unsafe { AXIsProcessTrusted() }
    }

    /// Checa e, sem permissão, faz o macOS mostrar o aviso que leva ao
    /// painel e já coloca o app na lista de Acessibilidade.
    pub fn trusted_with_prompt() -> bool {
        // SAFETY: dicionário CF de 1 par criado com as callbacks padrão de
        // CFType e liberado logo após o uso; chaves/valores são constantes
        // globais do sistema.
        unsafe {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );
            let trusted = AXIsProcessTrustedWithOptions(options);
            if !options.is_null() {
                CFRelease(options);
            }
            trusted
        }
    }
}

fn accessibility_grant() -> Grant {
    #[cfg(target_os = "macos")]
    {
        if ax::trusted() {
            Grant::Concedida
        } else {
            Grant::Pendente
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Grant::Concedida
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AutomationTarget {
    pub id: &'static str,
    pub label: &'static str,
    pub status: Grant,
}

#[derive(Debug, Serialize)]
pub struct PermissionsPayload {
    pub macos: bool,
    pub accessibility: Grant,
    /// Último resultado de "Pedir permissões agora"; `None` = nunca pedido
    /// nesta sessão (checar Automação sem pedir exigiria o próprio prompt).
    pub automation: Option<Vec<AutomationTarget>>,
    pub app_path: String,
    pub is_bundle: bool,
    pub icon_data_url: String,
}

static LAST_AUTOMATION: std::sync::Mutex<Option<Vec<AutomationTarget>>> = std::sync::Mutex::new(None);

/// Bundle `.app` do processo em execução; em dev (sem bundle), o binário.
fn app_path() -> (PathBuf, bool) {
    let exe = std::env::current_exe().unwrap_or_default();
    for ancestor in exe.ancestors() {
        if ancestor.extension().is_some_and(|ext| ext == "app") {
            return (ancestor.to_path_buf(), true);
        }
    }
    (exe, false)
}

/// Estado das permissões + o que a aba precisa para o ícone arrastável.
#[tauri::command]
pub fn get_permissions() -> PermissionsPayload {
    let (path, is_bundle) = app_path();
    PermissionsPayload {
        macos: cfg!(target_os = "macos"),
        accessibility: accessibility_grant(),
        automation: LAST_AUTOMATION.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        app_path: path.to_string_lossy().into_owned(),
        is_bundle,
        icon_data_url: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(APP_ICON_PNG)
        ),
    }
}

/// Barato o bastante para a aba chamar a cada 3s.
#[tauri::command]
pub fn check_accessibility() -> Grant {
    accessibility_grant()
}

/// Mostra o aviso nativo de Acessibilidade (lista o app no painel).
#[tauri::command]
pub fn request_accessibility() -> Grant {
    #[cfg(target_os = "macos")]
    {
        if ax::trusted_with_prompt() {
            Grant::Concedida
        } else {
            Grant::Pendente
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Grant::Concedida
    }
}

struct Target {
    id: &'static str,
    label: &'static str,
    script: &'static str,
    /// Caminhos em que o app precisa existir; vazio = sempre presente.
    installed_at: &'static [&'static str],
}

const TARGETS: &[Target] = &[
    Target {
        id: "system_events",
        label: "System Events",
        script: "tell application \"System Events\" to get name of first process",
        installed_at: &[],
    },
    Target {
        id: "calendar",
        label: "Calendário",
        script: "tell application \"Calendar\" to get name",
        installed_at: &[],
    },
    Target {
        id: "reminders",
        label: "Lembretes",
        script: "tell application \"Reminders\" to get name",
        installed_at: &[],
    },
    Target {
        id: "music",
        label: "Música",
        script: "tell application \"Music\" to get player state",
        installed_at: &["/System/Applications/Music.app"],
    },
    Target {
        id: "spotify",
        label: "Spotify",
        script: "tell application \"Spotify\" to get player state",
        installed_at: &["/Applications/Spotify.app", "~/Applications/Spotify.app"],
    },
];

fn is_installed(target: &Target) -> bool {
    if target.installed_at.is_empty() {
        return true;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    target.installed_at.iter().any(|path| match path.strip_prefix("~/") {
        Some(rest) => home.join(rest).exists(),
        None => PathBuf::from(path).exists(),
    })
}

/// Traduz a saída do `osascript`: -1743 = negado pelo usuário; -1744 = o
/// macOS ainda precisa perguntar; tempo esgotado = aviso aberto esperando.
fn classify(success: bool, stderr: &str) -> Grant {
    if success {
        Grant::Concedida
    } else if stderr.contains("-1743") {
        Grant::Negada
    } else if stderr.contains("-1744") {
        Grant::Pendente
    } else {
        Grant::Erro
    }
}

/// Espera do aviso de Automação: o usuário precisa de tempo para ler e clicar.
const AUTOMATION_PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

async fn run_target(target: &Target) -> Grant {
    if !is_installed(target) {
        return Grant::NaoInstalado;
    }
    let child = tokio::process::Command::new("osascript")
        .arg("-e")
        .arg(target.script)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(AUTOMATION_PROMPT_TIMEOUT, child).await {
        Ok(Ok(output)) => classify(output.status.success(), &String::from_utf8_lossy(&output.stderr)),
        Ok(Err(_)) => Grant::Erro,
        Err(_) => Grant::Pendente,
    }
}

/// "Pedir permissões agora": prompt de Acessibilidade e um AppleScript
/// inofensivo por alvo, em sequência (um aviso do macOS por vez). Cada
/// resultado sai também em `tools://automation` para a aba atualizar ao vivo.
#[tauri::command]
pub async fn request_permissions(app: AppHandle) -> Vec<AutomationTarget> {
    let _ = request_accessibility();
    let mut results = Vec::new();
    for target in TARGETS {
        let _ = app.emit(
            "tools://automation",
            AutomationTarget { id: target.id, label: target.label, status: Grant::Pendente },
        );
        let status = if cfg!(target_os = "macos") { run_target(target).await } else { Grant::Concedida };
        let result = AutomationTarget { id: target.id, label: target.label, status };
        let _ = app.emit("tools://automation", result.clone());
        results.push(result);
    }
    *LAST_AUTOMATION.lock().unwrap_or_else(|e| e.into_inner()) = Some(results.clone());
    results
}

/// Revela o app no Finder (`open -R`), para arrastar à mão se preciso.
#[tauri::command]
pub fn reveal_app() -> Result<(), String> {
    let (path, _) = app_path();
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("não foi possível abrir o Finder: {err}"))
}

static PINNING: AtomicBool = AtomicBool::new(false);

/// Deixa a janela de configurações no topo enquanto o app Ajustes do Sistema
/// estiver aberto (até 10 min), para arrastar o ícone de uma para a outra.
fn pin_settings_while_panel_open(app: &AppHandle) {
    let Some(window) = app.get_webview_window("settings") else {
        return;
    };
    let _ = window.set_always_on_top(true);
    if PINNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        let mut seen = false;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let running = system_settings_running().await;
            seen |= running;
            let gave_up = !seen && started.elapsed() > Duration::from_secs(15);
            if (seen && !running) || gave_up || started.elapsed() > Duration::from_secs(600) {
                break;
            }
        }
        let _ = window.set_always_on_top(false);
        PINNING.store(false, Ordering::SeqCst);
    });
}

async fn system_settings_running() -> bool {
    for name in ["System Settings", "System Preferences"] {
        let found = tokio::process::Command::new("pgrep")
            .arg("-x")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .status()
            .await
            .is_ok_and(|status| status.success());
        if found {
            return true;
        }
    }
    false
}

/// Abre o painel de Privacidade pedido. Só os painéis do guia — nunca uma
/// URL arbitrária vinda da webview.
#[tauri::command]
pub fn open_privacy_pane(app: AppHandle, pane: String) -> Result<(), String> {
    let anchor = match pane.as_str() {
        "automation" => "Privacy_Automation",
        "accessibility" => "Privacy_Accessibility",
        other => return Err(format!("painel desconhecido: {other}")),
    };
    if !cfg!(target_os = "macos") {
        return Err("permissões do sistema só existem no macOS".to_string());
    }
    std::process::Command::new("open")
        .arg(format!("x-apple.systempreferences:com.apple.preference.security?{anchor}"))
        .spawn()
        .map_err(|err| format!("não foi possível abrir os Ajustes do Sistema: {err}"))?;
    pin_settings_while_panel_open(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_le_codigos_do_apple_events() {
        assert_eq!(classify(true, ""), Grant::Concedida);
        assert_eq!(
            classify(false, "execution error: Not authorized to send Apple events to Calendar. (-1743)"),
            Grant::Negada
        );
        assert_eq!(classify(false, "(-1744)"), Grant::Pendente);
        assert_eq!(classify(false, "(-600)"), Grant::Erro);
    }
}
