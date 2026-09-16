// Tratamento de erros do motor (JRV-34): decide qual janela abrir e qual
// mensagem mostrar a partir do `EngineErrorKind` tipado do core, conforme a
// tabela "Tratamento de erros" da spec. Nunca loga ou emite a chave — só a
// mensagem já sem segredo que o core devolve.

use openjarvisbr_core::engine::EngineErrorKind;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{open_settings_window, set_tray_tooltip, AppState};

/// Subconjunto de [`EngineErrorKind`] que a janela de configurações sabe
/// exibir: os outros (`Socket`, `Quota`, `Other`) só aparecem no tray/overlay.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsErrorKind {
    InvalidKey,
    NoInputDevice,
    NoOutputDevice,
}

/// Erro pendente para a janela de configurações mostrar assim que carregar
/// (ou, se já estiver aberta, recebido ao vivo pelo evento
/// `engine://settings-error`).
#[derive(Debug, Clone, Serialize)]
pub struct SettingsErrorPayload {
    pub kind: SettingsErrorKind,
    pub message: String,
}

/// Rótulo curto do erro para o payload de `engine://error` (tray/overlay).
pub fn label(kind: EngineErrorKind) -> &'static str {
    match kind {
        EngineErrorKind::InvalidKey => "invalid_key",
        EngineErrorKind::NoInputDevice => "no_input_device",
        EngineErrorKind::NoOutputDevice => "no_output_device",
        EngineErrorKind::Socket => "socket",
        EngineErrorKind::Quota => "quota",
        EngineErrorKind::Other => "other",
    }
}

/// Aplica a tabela de erros: mensagem no tray sempre, e para chave inválida
/// ou dispositivo ausente, abre Settings com a mensagem no lugar certo — na
/// janela já aberta via evento, e numa recém-criada via `take_pending_error`.
pub fn handle_engine_error(app: &AppHandle, kind: EngineErrorKind, message: String) {
    set_tray_tooltip(app, Some(&message));

    let settings_kind = match kind {
        EngineErrorKind::InvalidKey => Some(SettingsErrorKind::InvalidKey),
        EngineErrorKind::NoInputDevice => Some(SettingsErrorKind::NoInputDevice),
        EngineErrorKind::NoOutputDevice => Some(SettingsErrorKind::NoOutputDevice),
        EngineErrorKind::Socket | EngineErrorKind::Quota | EngineErrorKind::Other => None,
    };
    let Some(settings_kind) = settings_kind else {
        return;
    };

    let payload = SettingsErrorPayload {
        kind: settings_kind,
        message,
    };
    {
        let state = app.state::<AppState>();
        *state.pending_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(payload.clone());
    }
    open_settings_window(app);
    let _ = app.emit("engine://settings-error", payload);
}

/// Consumido pela janela de configurações ao carregar: devolve o erro
/// pendente (se houver) e o limpa, para não reaparecer numa reabertura
/// futura sem um novo erro.
#[tauri::command]
pub fn take_pending_error(app: AppHandle) -> Option<SettingsErrorPayload> {
    let state = app.state::<AppState>();
    let mut guard = state.pending_error.lock().unwrap_or_else(|e| e.into_inner());
    guard.take()
}
