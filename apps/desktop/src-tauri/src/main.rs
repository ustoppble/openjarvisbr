// OpenJarvisBR desktop: vive na bandeja, sem janela principal visível. Ao
// abrir, carrega a chave e o config do core, sobe o Engine e passa a
// reemitir seus eventos como eventos Tauri (`engine://…`) para o overlay
// (JRV-32) e a janela de configurações (JRV-33) consumirem.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use openjarvisbr_core::config::{load_api_key, load_settings, DEFAULT_SYSTEM_PROMPT};
use openjarvisbr_core::engine::{Engine, EngineConfig, EngineEvent, EngineHandle, EngineState};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tokio::sync::broadcast;

const MUTE_SHORTCUT: &str = "CmdOrCtrl+Shift+J";
const TRAY_ID: &str = "main";

/// Estado compartilhado do app: o handle do motor (ausente sem chave) e o
/// item de menu "Mutar/Desmutar", guardado para trocar o texto sem
/// reconstruir o menu inteiro.
struct AppState {
    engine: Mutex<Option<EngineHandle>>,
    mute_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    muted: AtomicBool,
}

fn icon_for_state(state: EngineState) -> tauri::image::Image<'static> {
    match state {
        EngineState::Connecting => tauri::include_image!("../icons/tray-connecting.png"),
        EngineState::Listening => tauri::include_image!("../icons/tray-listening.png"),
        EngineState::Speaking => tauri::include_image!("../icons/tray-speaking.png"),
        EngineState::Muted => tauri::include_image!("../icons/tray-muted.png"),
        EngineState::Error => tauri::include_image!("../icons/tray-error.png"),
    }
}

fn state_label(state: EngineState) -> &'static str {
    match state {
        EngineState::Connecting => "connecting",
        EngineState::Listening => "listening",
        EngineState::Speaking => "speaking",
        EngineState::Muted => "muted",
        EngineState::Error => "error",
    }
}

/// Tray e menu só podem ser mexidos na thread principal do AppKit; chamado a
/// partir da task async que reemite eventos do motor, `set_icon` direto
/// duplicava o ícone na bandeja em vez de atualizar o existente.
fn set_tray_icon(app: &AppHandle, state: EngineState) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
            let _ = tray.set_icon(Some(icon_for_state(state)));
        }
    });
}

fn set_mute_label(app: &AppHandle, muted: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let state = handle.state::<AppState>();
        let guard = state.mute_item.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = guard.as_ref() {
            let _ = item.set_text(if muted { "Desmutar" } else { "Mutar" });
        }
    });
}

fn apply_mute(app: &AppHandle, muted: bool) {
    let state = app.state::<AppState>();
    state.muted.store(muted, Ordering::SeqCst);
    if let Some(engine) = state.engine.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        engine.mute(muted);
    }
    set_mute_label(app, muted);
}

fn toggle_mute(app: &AppHandle) {
    let state = app.state::<AppState>();
    let next = !state.muted.load(Ordering::SeqCst);
    apply_mute(app, next);
}

/// Reconecta a sessão em andamento ou, sem motor (chave ausente/inválida na
/// última tentativa), tenta subir de novo — cobre o caso de o usuário salvar
/// a chave no config.toml e clicar "Reconectar" sem reabrir o app.
fn do_reconnect(app: &AppHandle) {
    let state = app.state::<AppState>();
    let has_engine = state.engine.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    if has_engine {
        if let Some(engine) = state.engine.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            engine.reconnect();
        }
    } else {
        spawn_startup(app.clone());
    }
}

fn open_settings_window(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("settings") {
            let _ = window.show();
            let _ = window.set_focus();
            return;
        }
        let _ = WebviewWindowBuilder::new(&handle, "settings", WebviewUrl::App("settings.html".into()))
            .title("OpenJarvisBR — Configurações")
            .inner_size(420.0, 220.0)
            .resizable(false)
            .center()
            .build();
    });
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "mute" => toggle_mute(app),
        "reconnect" => do_reconnect(app),
        "settings" => open_settings_window(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

fn handle_engine_event(app: &AppHandle, event: EngineEvent) {
    match event {
        EngineEvent::State(state) => {
            set_tray_icon(app, state);
            let _ = app.emit("engine://state", serde_json::json!({ "state": state_label(state) }));
        }
        EngineEvent::UserText(text) => {
            let _ = app.emit("engine://text", serde_json::json!({ "from": "user", "text": text }));
        }
        EngineEvent::ModelText(text) => {
            let _ = app.emit("engine://text", serde_json::json!({ "from": "model", "text": text }));
        }
        EngineEvent::TurnComplete => {
            let _ = app.emit("engine://text", serde_json::json!({ "from": "turn_complete" }));
        }
        EngineEvent::Level { mic, model } => {
            let _ = app.emit("engine://level", serde_json::json!({ "mic": mic, "model": model }));
        }
        EngineEvent::Reconnecting { attempt } => {
            let _ = app.emit(
                "engine://state",
                serde_json::json!({ "state": "connecting", "reconnecting": true, "attempt": attempt }),
            );
        }
        EngineEvent::Error(message) => {
            let _ = app.emit("engine://error", message);
        }
    }
}

async fn forward_events(app: AppHandle, mut events: broadcast::Receiver<EngineEvent>) {
    loop {
        match events.recv().await {
            Ok(event) => handle_engine_event(&app, event),
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

/// Carrega chave e config, sobe o motor e passa a reemitir seus eventos.
/// Sem chave (ou falha ao conectar), abre a janela de configurações vazia e
/// deixa a bandeja em erro — nunca expõe a chave em log.
fn spawn_startup(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let api_key = match load_api_key() {
            Ok(key) => key,
            Err(err) => {
                tracing::warn!(erro = %err, "sem chave configurada");
                open_settings_window(&app);
                set_tray_icon(&app, EngineState::Error);
                return;
            }
        };

        let settings = load_settings();
        let config = EngineConfig {
            api_key,
            voice: settings.voice.unwrap_or_else(|| "Puck".to_string()),
            device_in: settings.device_in,
            device_out: settings.device_out,
            barge_in: settings.barge_in.unwrap_or(false),
            record_dir: None,
            system_prompt: settings
                .system_prompt
                .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string()),
            fx_amount: settings.voice_fx_amount.unwrap_or(0.35),
        };

        match Engine::start(config).await {
            Ok(handle) => {
                let events = handle.events();
                {
                    let state = app.state::<AppState>();
                    *state.engine.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
                }
                forward_events(app, events).await;
            }
            Err(err) => {
                tracing::warn!(erro = %err, "não foi possível iniciar o motor");
                open_settings_window(&app);
                set_tray_icon(&app, EngineState::Error);
            }
        }
    });
}

#[tauri::command]
fn set_mute(app: AppHandle, muted: bool) {
    apply_mute(&app, muted);
}

#[tauri::command]
fn reconnect(app: AppHandle) {
    do_reconnect(&app);
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

fn main() {
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_mute(app);
                    }
                })
                .build(),
        )
        .manage(AppState {
            engine: Mutex::new(None),
            mute_item: Mutex::new(None),
            muted: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![set_mute, reconnect, quit])
        .setup(|app| {
            let handle = app.handle().clone();

            let mute_item = MenuItem::with_id(app, "mute", "Mutar", true, None::<&str>)?;
            let reconnect_item = MenuItem::with_id(app, "reconnect", "Reconectar", true, None::<&str>)?;
            let settings_item = MenuItem::with_id(app, "settings", "Configurações", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&mute_item, &reconnect_item, &settings_item, &quit_item],
            )?;

            {
                let state = app.state::<AppState>();
                *state.mute_item.lock().unwrap_or_else(|e| e.into_inner()) = Some(mute_item);
            }

            let _tray = TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon_for_state(EngineState::Connecting))
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
                .build(app)?;

            app.global_shortcut().register(MUTE_SHORTCUT)?;

            spawn_startup(handle);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("erro ao rodar o app OpenJarvisBR");
}
