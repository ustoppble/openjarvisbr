// OpenJarvisBR desktop: vive na bandeja, sem janela principal visível. Ao
// abrir, carrega a chave e o config do core, sobe o Engine e passa a
// reemitir seus eventos como eventos Tauri (`engine://…`) para o overlay
// (JRV-32) e a janela de configurações (JRV-33) consumirem.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod errors;
mod tool_events;
mod tools_config;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use openjarvisbr_core::config::{
    all_profiles, effective_fx_amount, effective_overlay_style, effective_system_prompt,
    effective_voice, load_api_key, load_settings, save_profile,
};
use openjarvisbr_core::engine::{Engine, EngineConfig, EngineEvent, EngineHandle, EngineState};
use openjarvisbr_core::profiles::DEFAULT_PROFILE_ID;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tokio::sync::broadcast;

use commands::settings::{get_settings, list_devices, save_settings, set_fx_amount};
use commands::always_allow::{get_always_allow, remove_always_allow};
use commands::permissions::{
    check_accessibility, get_permissions, open_privacy_pane, request_accessibility,
    request_permissions, reveal_app,
};
use commands::tools::{
    add_mcp_server, confirm_tool, connect_mcp_server, emit_tool_mock, get_tools_settings, open_tools_link,
    remove_mcp_server, set_full_access, set_overlay_tool_strip, test_mcp_server,
};
use errors::{handle_engine_error, take_pending_error};

const MUTE_SHORTCUT: &str = "CmdOrCtrl+Shift+J";
const TRAY_ID: &str = "main";

/// Estado compartilhado do app: o handle do motor (ausente sem chave) e o
/// item de menu "Mutar/Desmutar", guardado para trocar o texto sem
/// reconstruir o menu inteiro. `pub(crate)` para os comandos de
/// `commands::settings` lerem/reiniciarem o motor.
pub(crate) struct AppState {
    pub(crate) engine: Mutex<Option<EngineHandle>>,
    mute_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    /// Itens `CheckMenuItem` do submenu "Perfil", por id de perfil — para
    /// marcar o ativo sem reconstruir o menu inteiro a cada troca.
    profile_items: Mutex<Vec<(String, CheckMenuItem<tauri::Wry>)>>,
    /// Item "Ferramentas: ligadas/desligadas" (JRV-58).
    tools_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    /// Item marcável "Acesso total (sem confirmação)" (JRV-65).
    full_access_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    muted: AtomicBool,
    /// Erro esperando a janela de configurações carregar e consumir via
    /// `take_pending_error` (ver `errors.rs`).
    pub(crate) pending_error: Mutex<Option<errors::SettingsErrorPayload>>,
}

/// Texto pedido ao Jarvis logo após trocar de perfil, para ele se apresentar
/// no papel novo.
pub(crate) const PROFILE_GREETING: &str = "Apresente-se brevemente no seu novo papel.";

fn profile_menu_id(id: &str) -> String {
    format!("profile:{id}")
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

/// Texto ao passar o mouse na bandeja: tentativa de reconexão em andamento
/// ou a última mensagem de erro. `None` limpa (estado normal, sem nada a
/// dizer).
pub(crate) fn set_tray_tooltip(app: &AppHandle, text: Option<&str>) {
    let handle = app.clone();
    let text = text.map(|s| s.to_string());
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
            let _ = tray.set_tooltip(text.as_deref());
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

fn tools_label(enabled: bool) -> &'static str {
    if enabled {
        "Ferramentas: ligadas"
    } else {
        "Ferramentas: desligadas"
    }
}

/// Atualiza o texto do item de ferramentas da bandeja — chamado pelo toggle
/// do próprio menu e depois de salvar a aba Ferramentas.
pub(crate) fn set_tools_label(app: &AppHandle, enabled: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let state = handle.state::<AppState>();
        let guard = state.tools_item.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = guard.as_ref() {
            let _ = item.set_text(tools_label(enabled));
        }
    });
}

/// Liga/desliga `[tools].enabled` pela bandeja e reinicia o motor, que só
/// lê a seção ao subir.
fn toggle_tools(app: &AppHandle) {
    let next = !tools_config::tools_enabled();
    if let Err(err) = tools_config::set_tools_enabled(next) {
        tracing::warn!(erro = %err, "não foi possível salvar [tools].enabled");
        return;
    }
    set_tools_label(app, next);
    tauri::async_runtime::spawn(restart_engine(app.clone()));
}

/// Evento do overlay/configurações quando o modo acesso total muda.
pub(crate) const FULL_ACCESS_EVENT: &str = "engine://full_access";

/// Marca o item da bandeja e avisa as janelas do modo acesso total.
pub(crate) fn sync_full_access(app: &AppHandle, on: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let state = handle.state::<AppState>();
        let guard = state.full_access_item.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = guard.as_ref() {
            let _ = item.set_checked(on);
        }
    });
    let _ = app.emit(FULL_ACCESS_EVENT, serde_json::json!({ "on": on }));
}

/// Liga/desliga o acesso total: grava `[tools].full_access` e aplica no
/// motor em andamento, sem reiniciar. Sem motor, só sincroniza a interface.
pub(crate) fn apply_full_access(app: &AppHandle, on: bool) -> Result<(), String> {
    openjarvisbr_core::config::save_full_access(on).map_err(|err| err.to_string())?;
    tracing::info!(acesso_total = on, "modo acesso total alterado");
    let state = app.state::<AppState>();
    let guard = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        // O motor responde com `EngineEvent::FullAccess`, que sincroniza.
        Some(engine) => engine.set_full_access(on),
        None => sync_full_access(app, on),
    }
    Ok(())
}

fn toggle_full_access(app: &AppHandle) {
    let next = !load_settings().full_access;
    if let Err(err) = apply_full_access(app, next) {
        tracing::warn!(erro = %err, "não foi possível salvar [tools].full_access");
        sync_full_access(app, !next);
    }
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

pub(crate) fn open_settings_window(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("settings") {
            let _ = window.show();
            let _ = window.set_focus();
            return;
        }
        let _ = WebviewWindowBuilder::new(&handle, "settings", WebviewUrl::App("settings.html".into()))
            .title("OpenJarvisBR — Configurações")
            .inner_size(520.0, 640.0)
            .resizable(false)
            .center()
            .build();
    });
}

const OVERLAY_WIDTH_ORB: f64 = 420.0;
const OVERLAY_HEIGHT_ORB: f64 = 140.0;
/// A cena 3D surreal precisa de mais espaço que o orb de 7 pontos.
const OVERLAY_WIDTH_SURREAL: f64 = 520.0;
const OVERLAY_HEIGHT_SURREAL: f64 = 220.0;
const OVERLAY_TOP_MARGIN: f64 = 12.0;

/// Dimensões da janela do overlay pro `overlay_style` atual.
pub(crate) fn overlay_size() -> (f64, f64) {
    match effective_overlay_style(&load_settings()).as_str() {
        "orb" => (OVERLAY_WIDTH_ORB, OVERLAY_HEIGHT_ORB),
        _ => (OVERLAY_WIDTH_SURREAL, OVERLAY_HEIGHT_SURREAL),
    }
}

/// Topo central do monitor ativo (o que está sob o cursor, com fallback pro
/// primário), em pixels lógicos, respeitando a `work_area` do monitor para
/// não abrir atrás da menu bar/dock.
fn overlay_position(app: &AppHandle, width: f64) -> (f64, f64) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());

    let Some(monitor) = monitor else {
        return (100.0, OVERLAY_TOP_MARGIN);
    };

    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let width_px = width * scale;
    let margin_px = OVERLAY_TOP_MARGIN * scale;

    let x = area.position.x as f64 + (area.size.width as f64 - width_px) / 2.0;
    let y = area.position.y as f64 + margin_px;

    (x / scale, y / scale)
}

pub(crate) fn open_overlay_window(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if handle.get_webview_window("overlay").is_some() {
            // Overlay já existe, deixa ele gerenciar sua visibilidade via eventos
            return;
        }
        let (width, height) = overlay_size();
        let (x, y) = overlay_position(&handle, width);
        // `visible(true)` mantém a janela sempre mapeada: o show/hide real é
        // puramente CSS (overlay.ts), assim nunca chamamos a API nativa de
        // show() que tornaria a janela key window. `focusable(false)` é a
        // garantia definitiva contra roubo de foco (sobrepõe canBecomeKeyWindow
        // no macOS), independente de qualquer show/hide futuro.
        let window = WebviewWindowBuilder::new(&handle, "overlay", WebviewUrl::App("overlay.html".into()))
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .focused(false)
            .focusable(false)
            // Os botões Confirmar/Negar (JRV-58) precisam do primeiro clique
            // numa janela que nunca vira key window.
            .accept_first_mouse(true)
            .visible(true)
            .inner_size(width, height)
            .position(x, y)
            .build();

        if let Ok(window) = window {
            let _ = window.set_ignore_cursor_events(true);
        }
    });
}

/// Marca no submenu "Perfil" o item cujo id é `active_id`, desmarcando os
/// demais — chamado depois de qualquer troca de perfil, seja pelo próprio
/// menu ou pela janela de configurações, para os dois ficarem em sincronia.
pub(crate) fn sync_profile_menu(app: &AppHandle, active_id: &str) {
    let handle = app.clone();
    let active_id = active_id.to_string();
    let _ = app.run_on_main_thread(move || {
        let state = handle.state::<AppState>();
        let guard = state.profile_items.lock().unwrap_or_else(|e| e.into_inner());
        for (id, item) in guard.iter() {
            let _ = item.set_checked(*id == active_id);
        }
    });
}

/// Troca de perfil pedida no menu da bandeja: salva só o `profile` no
/// config.toml (preserva o resto), sincroniza o check do menu e reinicia o
/// motor pedindo que o Jarvis se apresente no papel novo.
fn switch_profile(app: &AppHandle, id: &str) {
    if let Err(err) = save_profile(id) {
        tracing::warn!(erro = %err, "não foi possível salvar o perfil escolhido");
        return;
    }
    sync_profile_menu(app, id);
    let app = app.clone();
    tauri::async_runtime::spawn(restart_engine_with_greeting(
        app,
        Some(PROFILE_GREETING.to_string()),
    ));
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    if let Some(profile_id) = id.strip_prefix("profile:") {
        switch_profile(app, profile_id);
        return;
    }
    match id {
        "mute" => toggle_mute(app),
        "reconnect" => do_reconnect(app),
        "tools" => toggle_tools(app),
        "full_access" => toggle_full_access(app),
        "settings" => open_settings_window(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

fn handle_engine_event(app: &AppHandle, event: EngineEvent) {
    match event {
        EngineEvent::State(state) => {
            set_tray_icon(app, state);
            // O tooltip de erro/reconexão só é limpo ao sair do estado de
            // erro; `Reconnecting` (abaixo) e `Error` (abaixo) o preenchem de
            // novo na sequência.
            if state != EngineState::Error {
                set_tray_tooltip(app, None);
            }
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
            set_tray_tooltip(app, Some(&format!("Reconectando… (tentativa {attempt})")));
            let _ = app.emit(
                "engine://state",
                serde_json::json!({ "state": "connecting", "reconnecting": true, "attempt": attempt }),
            );
        }
        EngineEvent::FullAccess(on) => sync_full_access(app, on),
        event @ (EngineEvent::ToolRequested { .. }
        | EngineEvent::ToolConfirmNeeded { .. }
        | EngineEvent::ToolResult { .. }) => tool_events::emit(app, &event),
        EngineEvent::Error { kind, message } => {
            let _ = app.emit(
                "engine://error",
                serde_json::json!({ "kind": errors::label(kind), "message": message }),
            );
            handle_engine_error(app, kind, message);
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

/// Monta a `EngineConfig` a partir da chave e do config.toml atuais — voz,
/// prompt e intensidade do efeito vêm do perfil ativo, a menos que o usuário
/// tenha um valor próprio salvo (ver `effective_*` em `config.rs`).
fn build_engine_config(api_key: String, greeting: Option<String>) -> EngineConfig {
    let settings = load_settings();
    let system_prompt = effective_system_prompt(&settings);
    let voice = effective_voice(&settings);
    let fx_amount = effective_fx_amount(&settings);
    let tools = openjarvisbr_core::config::effective_tool_globs(&settings);
    EngineConfig {
        api_key,
        voice,
        device_in: settings.device_in.clone(),
        device_out: settings.device_out.clone(),
        barge_in: settings.barge_in.unwrap_or(false),
        record_dir: None,
        system_prompt,
        fx_amount,
        greeting,
        tools,
        mcp_servers: settings.mcp_servers,
        full_access: settings.full_access,
        always_allow: settings.always_allow,
    }
}

/// Antes de subir o motor, relê o Overclock/OverClick em disco para os
/// servidores `overclock`/`overclick` do config (JRV-68): porta ou token
/// novos vão para o config e o Keychain. Sem Overclock, nada muda.
async fn refresh_discovered_mcp_servers() {
    let servers = load_settings().mcp_servers;
    let updated = tauri::async_runtime::spawn_blocking(move || {
        openjarvisbr_core::mcp::discovery::refresh(&servers)
    })
    .await
    .unwrap_or_default();
    for server in updated {
        match tools_config::upsert_server(&server) {
            Ok(()) => tracing::info!(server = %server.name, "servidor MCP atualizado pela descoberta"),
            Err(err) => tracing::warn!(server = %server.name, error = %err, "não foi possível gravar o servidor descoberto"),
        }
    }
}

/// Derruba o motor em execução (se houver) e sobe de novo com o config.toml
/// atual — chamado pela janela de configurações depois de salvar voz,
/// dispositivos, barge-in ou system prompt, que só entram em vigor no
/// próximo `Engine::start`.
pub(crate) async fn restart_engine(app: AppHandle) {
    restart_engine_with_greeting(app, None).await;
}

/// Como `restart_engine`, mas com `greeting` opcional: usado ao trocar de
/// perfil, para o Jarvis se apresentar no papel novo assim que reconectar.
pub(crate) async fn restart_engine_with_greeting(app: AppHandle, greeting: Option<String>) {
    let old = {
        let state = app.state::<AppState>();
        let mut guard = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        guard.take()
    };
    if let Some(engine) = old {
        engine.stop().await;
    }
    spawn_startup_with_greeting(app, greeting);
}

/// Carrega chave e config, sobe o motor e passa a reemitir seus eventos.
/// Sem chave (ou falha ao conectar), abre a janela de configurações vazia e
/// deixa a bandeja em erro — nunca expõe a chave em log.
fn spawn_startup(app: AppHandle) {
    spawn_startup_with_greeting(app, None);
}

/// Como `spawn_startup`, mas com `greeting` opcional (ver
/// `restart_engine_with_greeting`).
fn spawn_startup_with_greeting(app: AppHandle, greeting: Option<String>) {
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

        refresh_discovered_mcp_servers().await;
        let config = build_engine_config(api_key, greeting);

        match Engine::start(config).await {
            Ok(handle) => {
                let events = handle.events();
                {
                    let state = app.state::<AppState>();
                    *state.engine.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
                }
                open_overlay_window(&app);
                forward_events(app, events).await;
            }
            Err(err) => {
                tracing::warn!(erro = %err, "não foi possível iniciar o motor");
                open_settings_window(&app);
                set_tray_icon(&app, EngineState::Error);
                handle_engine_error(&app, err.kind(), err.to_string());
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
        // Precisa ser o primeiro plugin registrado (requisito do próprio
        // plugin). Segunda abertura: ativa a existente (mostra Settings) e
        // sai — nunca duas instâncias do motor/mic ao mesmo tempo.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_settings_window(app);
        }))
        // Arrastar o ícone do app para a lista do painel de Privacidade (JRV-58).
        .plugin(tauri_plugin_drag::init())
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
            profile_items: Mutex::new(Vec::new()),
            tools_item: Mutex::new(None),
            full_access_item: Mutex::new(None),
            muted: AtomicBool::new(false),
            pending_error: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            set_mute,
            reconnect,
            quit,
            get_settings,
            list_devices,
            save_settings,
            set_fx_amount,
            take_pending_error,
            confirm_tool,
            set_overlay_tool_strip,
            get_tools_settings,
            set_full_access,
            get_always_allow,
            remove_always_allow,
            add_mcp_server,
            remove_mcp_server,
            connect_mcp_server,
            test_mcp_server,
            open_tools_link,
            get_permissions,
            check_accessibility,
            request_accessibility,
            request_permissions,
            reveal_app,
            open_privacy_pane,
            emit_tool_mock
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            let mute_item = MenuItem::with_id(app, "mute", "Mutar", true, None::<&str>)?;
            let reconnect_item = MenuItem::with_id(app, "reconnect", "Reconectar", true, None::<&str>)?;

            let settings_at_startup = load_settings();
            let active_profile = settings_at_startup
                .profile
                .clone()
                .unwrap_or_else(|| DEFAULT_PROFILE_ID.to_string());
            let mut profile_items: Vec<(String, CheckMenuItem<tauri::Wry>)> = Vec::new();
            for profile in all_profiles(&settings_at_startup) {
                let checked = profile.id == active_profile;
                let item = CheckMenuItem::with_id(
                    app,
                    profile_menu_id(&profile.id),
                    &profile.name,
                    true,
                    checked,
                    None::<&str>,
                )?;
                profile_items.push((profile.id, item));
            }
            let profile_refs: Vec<&dyn IsMenuItem<tauri::Wry>> = profile_items
                .iter()
                .map(|(_, item)| item as &dyn IsMenuItem<tauri::Wry>)
                .collect();
            let profile_submenu = Submenu::with_items(app, "Perfil", true, &profile_refs)?;

            let tools_item = MenuItem::with_id(
                app,
                "tools",
                tools_label(tools_config::tools_enabled()),
                true,
                None::<&str>,
            )?;
            let full_access_item = CheckMenuItem::with_id(
                app,
                "full_access",
                "Acesso total (sem confirmação)",
                true,
                settings_at_startup.full_access,
                None::<&str>,
            )?;
            let settings_item = MenuItem::with_id(app, "settings", "Configurações", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &mute_item,
                    &reconnect_item,
                    &tools_item,
                    &full_access_item,
                    &profile_submenu,
                    &settings_item,
                    &quit_item,
                ],
            )?;

            {
                let state = app.state::<AppState>();
                *state.mute_item.lock().unwrap_or_else(|e| e.into_inner()) = Some(mute_item);
                *state.profile_items.lock().unwrap_or_else(|e| e.into_inner()) = profile_items;
                *state.tools_item.lock().unwrap_or_else(|e| e.into_inner()) = Some(tools_item);
                *state.full_access_item.lock().unwrap_or_else(|e| e.into_inner()) = Some(full_access_item);
            }

            let _tray = TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon_for_state(EngineState::Connecting))
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
                .build(app)?;

            app.global_shortcut().register(MUTE_SHORTCUT)?;

            // Dev: OPENJARVISBR_TOOL_MOCK=1 dispara o fluxo falso de ferramenta
            // (requested → confirm_needed) alguns segundos após abrir, para
            // validar o overlay sem o Engine emitir eventos de verdade.
            #[cfg(debug_assertions)]
            if std::env::var("OPENJARVISBR_TOOL_MOCK").is_ok_and(|v| v == "1") {
                let mock_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                    open_overlay_window(&mock_handle);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    if let Err(err) = commands::tools::run_tool_mock(&mock_handle, None) {
                        tracing::warn!(erro = %err, "mock de ferramenta falhou");
                    }
                });
            }

            spawn_startup(handle);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("erro ao rodar o app OpenJarvisBR");
}
