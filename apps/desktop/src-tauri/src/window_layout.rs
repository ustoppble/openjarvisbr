// Posição, tamanho e visibilidade das janelas (JRV-80). O overlay pode ser
// arrastado, trocar de escala (pequeno/médio/grande) e ser ocultado; a janela
// de configurações redimensiona e lembra onde estava. O que persiste vai para
// `[overlay]` e `[settings_window]` do config.toml.
//
// O overlay continua sem nunca virar key window (`focusable(false)`) e ignora
// o mouse por padrão. Ele só passa a receber eventos enquanto o cursor está
// sobre o card à mostra — medido aqui por polling, porque uma janela que
// ignora o mouse não recebe `mouseenter` — ou enquanto a faixa de ferramenta
// espera Confirmar/Negar.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use openjarvisbr_core::config::{
    clear_overlay_position, effective_overlay_scale, effective_overlay_style, load_settings,
    normalize_overlay_scale, overlay_scale_factor, save_overlay_position, save_overlay_scale,
    save_settings_window, Settings, WindowGeometry, OVERLAY_SCALES,
};
use tauri::menu::{CheckMenuItem, IsMenuItem, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, Window, WindowEvent,
};
use tauri_plugin_global_shortcut::Shortcut;

pub(crate) const OVERLAY_LABEL: &str = "overlay";
pub(crate) const SETTINGS_LABEL: &str = "settings";

/// Oculta/mostra o overlay de qualquer app.
pub(crate) const OVERLAY_SHORTCUT: &str = "CmdOrCtrl+Shift+Alt+J";

const OVERLAY_WIDTH_ORB: f64 = 420.0;
const OVERLAY_HEIGHT_ORB: f64 = 140.0;
/// A cena 3D surreal precisa de mais espaço que o orb de 7 pontos.
const OVERLAY_WIDTH_SURREAL: f64 = 520.0;
const OVERLAY_HEIGHT_SURREAL: f64 = 220.0;
const OVERLAY_TOP_MARGIN: f64 = 12.0;
/// Altura extra da faixa "Quer que eu execute…?" abaixo da cena.
const TOOL_STRIP_HEIGHT: f64 = 58.0;

const SETTINGS_WIDTH: f64 = 520.0;
const SETTINGS_HEIGHT: f64 = 640.0;
const SETTINGS_MIN_WIDTH: f64 = 420.0;
const SETTINGS_MIN_HEIGHT: f64 = 420.0;

const HOVER_POLL: Duration = Duration::from_millis(80);
/// Arrastar e redimensionar disparam dezenas de eventos: grava só quando
/// param.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

const HOVER_EVENT: &str = "overlay://hover";
const HIDDEN_EVENT: &str = "overlay://hidden";
const SCALE_EVENT: &str = "overlay://scale";

const MENU_HIDE: &str = "overlay_hide";
const MENU_RESET: &str = "overlay_reset";
const MENU_SCALE_PREFIX: &str = "overlay_scale:";

#[derive(Default)]
pub(crate) struct WindowLayoutState {
    /// A página do overlay está com o card à mostra (classe `visible`).
    shown: AtomicBool,
    strip_visible: AtomicBool,
    /// Faixa de ferramenta com Confirmar/Negar na tela.
    strip_interactive: AtomicBool,
    /// Cursor sobre o overlay, medido pelo polling.
    hover: AtomicBool,
    polling: AtomicBool,
    /// Ocultado pelo usuário (bandeja, atalho ou botão ×). Vale só na sessão.
    hidden: AtomicBool,
    /// Arraste começado pelo usuário: só então `Moved` vira posição salva
    /// (a posição padrão acompanha o monitor ativo e não deve ser fixada).
    dragging: AtomicBool,
    overlay_save_seq: AtomicU64,
    settings_save_seq: AtomicU64,
    hide_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    scale_items: Mutex<Vec<(&'static str, CheckMenuItem<tauri::Wry>)>>,
}

fn layout(app: &AppHandle) -> tauri::State<'_, WindowLayoutState> {
    app.state::<WindowLayoutState>()
}

/// Tamanho da janela do overlay para o estilo, a escala e a faixa de
/// ferramenta atuais.
fn overlay_window_size(settings: &Settings, strip_visible: bool) -> (f64, f64) {
    let (width, height) = match effective_overlay_style(settings).as_str() {
        "orb" => (OVERLAY_WIDTH_ORB, OVERLAY_HEIGHT_ORB),
        _ => (OVERLAY_WIDTH_SURREAL, OVERLAY_HEIGHT_SURREAL),
    };
    let height = if strip_visible { height + TOOL_STRIP_HEIGHT } else { height };
    let scale = overlay_scale_factor(effective_overlay_scale(settings));
    (width * scale, height * scale)
}

/// Topo central do monitor ativo (o que está sob o cursor, com fallback pro
/// primário), em pixels lógicos, respeitando a `work_area` do monitor para
/// não abrir atrás da menu bar/dock.
fn default_overlay_position(app: &AppHandle, width: f64) -> (f64, f64) {
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

/// Uma janela salva em (`x`, `y`) com largura `width` ainda tem o topo à
/// vista em algum monitor? Monitor desligado ou resolução trocada fazem a
/// posição salva cair fora da tela.
fn visible_on_some_monitor(app: &AppHandle, x: f64, y: f64, width: f64) -> bool {
    let (probe_x, probe_y) = (x + width / 2.0, y + 20.0);
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .any(|monitor| {
            let scale = monitor.scale_factor();
            let pos = monitor.position().to_logical::<f64>(scale);
            let size = monitor.size().to_logical::<f64>(scale);
            probe_x >= pos.x
                && probe_x < pos.x + size.width
                && probe_y >= pos.y
                && probe_y < pos.y + size.height
        })
}

pub(crate) fn open_overlay_window(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if handle.get_webview_window(OVERLAY_LABEL).is_some() {
            // Overlay já existe, deixa ele gerenciar sua visibilidade via eventos
            return;
        }
        let settings = load_settings();
        let (width, height) = overlay_window_size(&settings, false);
        let saved = settings
            .overlay
            .x
            .zip(settings.overlay.y)
            .filter(|&(x, y)| visible_on_some_monitor(&handle, x, y, width));
        let (x, y) = saved.unwrap_or_else(|| default_overlay_position(&handle, width));
        // `visible(true)` mantém a janela sempre mapeada: o show/hide real é
        // puramente CSS (overlay.ts), assim nunca chamamos a API nativa de
        // show() que tornaria a janela key window. `focusable(false)` é a
        // garantia definitiva contra roubo de foco (sobrepõe canBecomeKeyWindow
        // no macOS), independente de qualquer show/hide futuro.
        let window = WebviewWindowBuilder::new(&handle, OVERLAY_LABEL, WebviewUrl::App("overlay.html".into()))
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .focused(false)
            .focusable(false)
            // Os botões Confirmar/Negar (JRV-58), o × e o arraste precisam do
            // primeiro clique numa janela que nunca vira key window.
            .accept_first_mouse(true)
            .visible(true)
            .inner_size(width, height)
            .position(x, y)
            .build();

        if let Ok(window) = window {
            // A escala aumenta a janela e dá zoom na página: o layout em CSS
            // continua o mesmo em qualquer tamanho.
            let _ = window.set_zoom(overlay_scale_factor(effective_overlay_scale(&settings)));
            let _ = window.set_ignore_cursor_events(true);
        }
    });
}

/// O overlay recebe o mouse com Confirmar/Negar na tela ou com o cursor
/// sobre o card à mostra; fora disso os cliques passam para o que está atrás.
fn apply_cursor_mode(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(OVERLAY_LABEL) else {
            return;
        };
        let state = layout(&handle);
        let interactive = state.strip_interactive.load(Ordering::SeqCst)
            || (state.shown.load(Ordering::SeqCst) && state.hover.load(Ordering::SeqCst));
        let _ = window.set_ignore_cursor_events(!interactive);
    });
}

/// Faixa de ferramenta (JRV-58): cresce a janela para baixo e, com botões na
/// tela, deixa ela receber cliques.
pub(crate) fn set_tool_strip(app: &AppHandle, visible: bool, interactive: bool) {
    let state = layout(app);
    state.strip_visible.store(visible, Ordering::SeqCst);
    state.strip_interactive.store(visible && interactive, Ordering::SeqCst);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(OVERLAY_LABEL) {
            let (width, height) = overlay_window_size(&load_settings(), visible);
            let _ = window.set_size(LogicalSize::new(width, height));
        }
    });
    apply_cursor_mode(app);
}

fn set_hover(app: &AppHandle, inside: bool) {
    let state = layout(app);
    if state.hover.swap(inside, Ordering::SeqCst) == inside {
        return;
    }
    if !inside {
        state.dragging.store(false, Ordering::SeqCst);
    }
    let _ = app.emit(HOVER_EVENT, serde_json::json!({ "inside": inside }));
    apply_cursor_mode(app);
}

fn cursor_over_overlay(app: &AppHandle) -> Option<bool> {
    let window = app.get_webview_window(OVERLAY_LABEL)?;
    let cursor = app.cursor_position().ok()?;
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    // No macOS o tao entrega o cursor em pontos × escala do monitor principal
    // e a janela em pontos × escala do monitor dela: com um Retina ao lado de
    // um monitor 1x, só os pontos batem. No Windows os dois já são pixels do
    // mesmo espaço.
    let (cursor_scale, window_scale) = if cfg!(target_os = "macos") {
        let primary = app.primary_monitor().ok().flatten()?.scale_factor();
        (primary, window.scale_factor().ok()?)
    } else {
        (1.0, 1.0)
    };
    let (x, y) = (cursor.x / cursor_scale, cursor.y / cursor_scale);
    let (left, top) = (pos.x as f64 / window_scale, pos.y as f64 / window_scale);
    let (width, height) = (size.width as f64 / window_scale, size.height as f64 / window_scale);
    Some(x >= left && x < left + width && y >= top && y < top + height)
}

/// Enquanto o card está à mostra, confere onde está o cursor a cada 80 ms.
fn start_hover_poll(app: &AppHandle) {
    if layout(app).polling.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while layout(&app).shown.load(Ordering::SeqCst) {
            let inside = cursor_over_overlay(&app).unwrap_or(false);
            set_hover(&app, inside);
            tokio::time::sleep(HOVER_POLL).await;
        }
        layout(&app).polling.store(false, Ordering::SeqCst);
        set_hover(&app, false);
        // O card pode ter voltado entre a última volta e o `polling = false`.
        if layout(&app).shown.load(Ordering::SeqCst) {
            start_hover_poll(&app);
        }
    });
}

/// A página do overlay avisa quando o card aparece e some.
#[tauri::command]
pub fn overlay_shown(app: AppHandle, shown: bool) {
    layout(&app).shown.store(shown, Ordering::SeqCst);
    if shown {
        start_hover_poll(&app);
    } else {
        set_hover(&app, false);
    }
    apply_cursor_mode(&app);
}

/// Mouse pressionado no card: arrasta a janela. Comando síncrono roda na
/// thread principal, ainda com o evento do clique corrente.
#[tauri::command]
pub fn overlay_start_drag(app: AppHandle) {
    let Some(window) = app.get_webview_window(OVERLAY_LABEL) else {
        return;
    };
    layout(&app).dragging.store(true, Ordering::SeqCst);
    tracing::info!("arraste do overlay começou");
    if let Err(err) = window.start_dragging() {
        tracing::warn!(erro = %err, "não foi possível arrastar o overlay");
    }
}

#[derive(serde::Serialize)]
pub struct OverlayLayoutPayload {
    hidden: bool,
    scale: &'static str,
}

/// Estado inicial para a página do overlay e o select das configurações.
#[tauri::command]
pub fn get_overlay_layout(app: AppHandle) -> OverlayLayoutPayload {
    OverlayLayoutPayload {
        hidden: layout(&app).hidden.load(Ordering::SeqCst),
        scale: effective_overlay_scale(&load_settings()),
    }
}

/// Botão × do overlay.
#[tauri::command]
pub fn hide_overlay(app: AppHandle) {
    set_overlay_hidden(&app, true);
}

/// Select "Tamanho do overlay" das configurações: grava e aplica na hora.
#[tauri::command]
pub fn set_overlay_scale(app: AppHandle, scale: String) -> Result<(), String> {
    apply_overlay_scale(&app, &scale)
}

pub(crate) fn set_overlay_hidden(app: &AppHandle, hidden: bool) {
    layout(app).hidden.store(hidden, Ordering::SeqCst);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let state = layout(&handle);
        let guard = state.hide_item.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = guard.as_ref() {
            let _ = item.set_text(if hidden { "Mostrar overlay" } else { "Ocultar overlay" });
        }
    });
    let _ = app.emit(HIDDEN_EVENT, serde_json::json!({ "hidden": hidden }));
}

pub(crate) fn toggle_overlay_hidden(app: &AppHandle) {
    let hidden = layout(app).hidden.load(Ordering::SeqCst);
    set_overlay_hidden(app, !hidden);
}

pub(crate) fn is_overlay_shortcut(shortcut: &Shortcut) -> bool {
    OVERLAY_SHORTCUT
        .parse::<Shortcut>()
        .is_ok_and(|overlay| overlay == *shortcut)
}

/// Troca a escala: grava, marca no menu, avisa as configurações e redimensiona
/// o overlay em torno do centro, com o topo parado.
fn apply_overlay_scale(app: &AppHandle, scale: &str) -> Result<(), String> {
    let scale = normalize_overlay_scale(Some(scale));
    save_overlay_scale(scale).map_err(|err| err.to_string())?;
    tracing::info!(escala = scale, "escala do overlay alterada");

    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        {
            let state = layout(&handle);
            let guard = state.scale_items.lock().unwrap_or_else(|e| e.into_inner());
            for (name, item) in guard.iter() {
                let _ = item.set_checked(*name == scale);
            }
        }
        let Some(window) = handle.get_webview_window(OVERLAY_LABEL) else {
            return;
        };
        let state = layout(&handle);
        state.dragging.store(false, Ordering::SeqCst);
        let settings = load_settings();
        let (width, height) = overlay_window_size(&settings, state.strip_visible.load(Ordering::SeqCst));
        if let (Ok(pos), Ok(size), Ok(factor)) =
            (window.outer_position(), window.outer_size(), window.scale_factor())
        {
            let pos = pos.to_logical::<f64>(factor);
            let old_width = size.to_logical::<f64>(factor).width;
            let x = pos.x + (old_width - width) / 2.0;
            let _ = window.set_position(LogicalPosition::new(x, pos.y));
            if settings.overlay.x.is_some() {
                if let Err(err) = save_overlay_position(x, pos.y) {
                    tracing::warn!(erro = %err, "não foi possível salvar a posição do overlay");
                }
            }
        }
        let _ = window.set_size(LogicalSize::new(width, height));
        let _ = window.set_zoom(overlay_scale_factor(scale));
    });
    let _ = app.emit(SCALE_EVENT, serde_json::json!({ "scale": scale }));
    Ok(())
}

/// Esquece a posição arrastada e volta o overlay ao topo central.
fn reset_overlay_position(app: &AppHandle) {
    if let Err(err) = clear_overlay_position() {
        tracing::warn!(erro = %err, "não foi possível limpar a posição do overlay");
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(OVERLAY_LABEL) else {
            return;
        };
        layout(&handle).dragging.store(false, Ordering::SeqCst);
        let (width, _) = overlay_window_size(&load_settings(), false);
        let (x, y) = default_overlay_position(&handle, width);
        let _ = window.set_position(LogicalPosition::new(x, y));
    });
}

/// Itens do overlay no menu da bandeja: "Ocultar overlay" e o submenu com
/// tamanho e posição padrão.
pub(crate) fn build_tray_items(
    app: &tauri::App,
) -> tauri::Result<(MenuItem<tauri::Wry>, Submenu<tauri::Wry>)> {
    let hide_item = MenuItem::with_id(app, MENU_HIDE, "Ocultar overlay", true, Some(OVERLAY_SHORTCUT))?;
    let current = effective_overlay_scale(&load_settings());
    let mut scale_items = Vec::new();
    for (name, _) in OVERLAY_SCALES {
        let label = match name {
            "small" => "Pequeno",
            "large" => "Grande",
            _ => "Médio",
        };
        let item = CheckMenuItem::with_id(
            app,
            format!("{MENU_SCALE_PREFIX}{name}"),
            label,
            true,
            name == current,
            None::<&str>,
        )?;
        scale_items.push((name, item));
    }
    let separator = PredefinedMenuItem::separator(app)?;
    let reset_item = MenuItem::with_id(app, MENU_RESET, "Voltar à posição padrão", true, None::<&str>)?;
    let mut items: Vec<&dyn IsMenuItem<tauri::Wry>> = scale_items
        .iter()
        .map(|(_, item)| item as &dyn IsMenuItem<tauri::Wry>)
        .collect();
    items.push(&separator);
    items.push(&reset_item);
    let submenu = Submenu::with_items(app, "Tamanho e posição do overlay", true, &items)?;

    let state = app.state::<WindowLayoutState>();
    *state.hide_item.lock().unwrap_or_else(|e| e.into_inner()) = Some(hide_item.clone());
    *state.scale_items.lock().unwrap_or_else(|e| e.into_inner()) = scale_items;
    Ok((hide_item, submenu))
}

/// Trata os itens do overlay no menu da bandeja; `false` = não era dele.
pub(crate) fn handle_menu_event(app: &AppHandle, id: &str) -> bool {
    if let Some(scale) = id.strip_prefix(MENU_SCALE_PREFIX) {
        if let Err(err) = apply_overlay_scale(app, scale) {
            tracing::warn!(erro = %err, "não foi possível salvar a escala do overlay");
        }
        return true;
    }
    match id {
        MENU_HIDE => toggle_overlay_hidden(app),
        MENU_RESET => reset_overlay_position(app),
        _ => return false,
    }
    true
}

pub(crate) fn open_settings_window(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(SETTINGS_LABEL) {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
            return;
        }
        let saved = load_settings()
            .settings_window
            .filter(|g| visible_on_some_monitor(&handle, g.x, g.y, g.width));
        let builder = WebviewWindowBuilder::new(&handle, SETTINGS_LABEL, WebviewUrl::App("settings.html".into()))
            .title("OpenJarvisBR — Configurações")
            .resizable(true)
            .min_inner_size(SETTINGS_MIN_WIDTH, SETTINGS_MIN_HEIGHT);
        let builder = match saved {
            Some(g) => builder
                .inner_size(g.width.max(SETTINGS_MIN_WIDTH), g.height.max(SETTINGS_MIN_HEIGHT))
                .position(g.x, g.y),
            None => builder.inner_size(SETTINGS_WIDTH, SETTINGS_HEIGHT).center(),
        };
        let _ = builder.build();
    });
}

fn save_overlay_now(app: &AppHandle) {
    let Some(window) = app.get_webview_window(OVERLAY_LABEL) else {
        return;
    };
    let (Ok(pos), Ok(factor)) = (window.outer_position(), window.scale_factor()) else {
        return;
    };
    let pos = pos.to_logical::<f64>(factor);
    match save_overlay_position(pos.x, pos.y) {
        Ok(()) => tracing::info!("posição do overlay salva"),
        Err(err) => tracing::warn!(erro = %err, "não foi possível salvar a posição do overlay"),
    }
}

fn settings_geometry(window: &WebviewWindow) -> Option<WindowGeometry> {
    // Minimizada ou em tela cheia não é o tamanho que a pessoa escolheu (e no
    // Windows a minimizada fica em -32000).
    let skip = window.is_minimized().unwrap_or(true)
        || window.is_maximized().unwrap_or(false)
        || window.is_fullscreen().unwrap_or(false);
    if skip {
        return None;
    }
    let factor = window.scale_factor().ok()?;
    let pos = window.outer_position().ok()?.to_logical::<f64>(factor);
    let size = window.inner_size().ok()?.to_logical::<f64>(factor);
    Some(WindowGeometry {
        x: pos.x.round(),
        y: pos.y.round(),
        width: size.width.round(),
        height: size.height.round(),
    })
}

fn save_settings_now(app: &AppHandle) {
    let Some(geometry) = app
        .get_webview_window(SETTINGS_LABEL)
        .and_then(|window| settings_geometry(&window))
    else {
        return;
    };
    if let Err(err) = save_settings_window(geometry) {
        tracing::warn!(erro = %err, "não foi possível salvar o tamanho da janela de configurações");
    }
}

/// Roda `save` quando `seq` fica parado por `SAVE_DEBOUNCE`.
fn debounce(app: &AppHandle, seq: fn(&WindowLayoutState) -> &AtomicU64, save: fn(&AppHandle)) {
    let ticket = seq(&layout(app)).fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SAVE_DEBOUNCE).await;
        if seq(&layout(&app)).load(Ordering::SeqCst) == ticket {
            save(&app);
        }
    });
}

/// `Builder::on_window_event`: grava posição do overlay depois de um arraste
/// e tamanho/posição da janela de configurações.
pub(crate) fn on_window_event(window: &Window, event: &WindowEvent) {
    let app = window.app_handle();
    match (window.label(), event) {
        (OVERLAY_LABEL, WindowEvent::Moved(_)) => {
            if layout(app).dragging.load(Ordering::SeqCst) {
                debounce(app, |state| &state.overlay_save_seq, save_overlay_now);
            }
        }
        (SETTINGS_LABEL, WindowEvent::Moved(_) | WindowEvent::Resized(_)) => {
            debounce(app, |state| &state.settings_save_seq, save_settings_now);
        }
        _ => {}
    }
}
