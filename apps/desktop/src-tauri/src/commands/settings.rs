// Comandos Tauri da janela de configurações (JRV-33). A chave nunca sai
// deste processo em texto — `get_settings` devolve só se ela existe e
// quantos caracteres tem, nunca o valor.

use openjarvisbr_core::audio::{capture::list_input_devices, playback::list_output_devices};
use openjarvisbr_core::config::{
    load_api_key, load_settings, save, SaveSettings, DEFAULT_SYSTEM_PROMPT,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::AppState;

#[derive(Debug, Serialize)]
pub struct SettingsPayload {
    pub has_api_key: bool,
    pub api_key_chars: usize,
    pub voice: String,
    pub device_in: Option<String>,
    pub device_out: Option<String>,
    pub barge_in: bool,
    pub voice_fx_amount: f32,
    pub system_prompt: String,
    pub default_system_prompt: String,
}

/// Estado atual do config.toml, pronto para preencher o formulário. A chave
/// nunca aparece aqui — só se existe e seu tamanho, para o texto "chave
/// configurada (N caracteres)".
#[tauri::command]
pub fn get_settings() -> SettingsPayload {
    let key = load_api_key().ok();
    let settings = load_settings();
    SettingsPayload {
        has_api_key: key.is_some(),
        api_key_chars: key.map(|k| k.chars().count()).unwrap_or(0),
        voice: settings.voice.unwrap_or_else(|| "Puck".to_string()),
        device_in: settings.device_in,
        device_out: settings.device_out,
        barge_in: settings.barge_in.unwrap_or(false),
        voice_fx_amount: settings.voice_fx_amount.unwrap_or(0.35),
        system_prompt: settings
            .system_prompt
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string()),
        default_system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
    }
}

#[derive(Debug, Serialize)]
pub struct DevicesPayload {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

/// Dispositivos de entrada e saída disponíveis no host, via `cpal` no core.
#[tauri::command]
pub fn list_devices() -> DevicesPayload {
    DevicesPayload {
        input: list_input_devices(),
        output: list_output_devices(),
    }
}

#[derive(Debug, Deserialize)]
pub struct SaveSettingsPayload {
    /// `Some` só quando o usuário digitou uma chave nova; ausente preserva a
    /// existente no config.toml.
    pub api_key: Option<String>,
    pub voice: String,
    pub device_in: Option<String>,
    pub device_out: Option<String>,
    pub barge_in: bool,
    pub voice_fx_amount: f32,
    pub system_prompt: String,
}

/// Ajusta a intensidade do efeito de voz na sessão em andamento, sem
/// persistir — o slider chama isto a cada movimento; o valor final é
/// persistido quando `save_settings` roda.
#[tauri::command]
pub fn set_fx_amount(app: AppHandle, amount: f32) {
    let state = app.state::<AppState>();
    let guard = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(engine) = guard.as_ref() {
        engine.set_fx_amount(amount);
    }
}

/// Grava o config.toml e reinicia o motor quando voz, dispositivos, barge-in
/// ou o system prompt mudaram — os únicos campos presos à `EngineConfig` no
/// momento em que a sessão sobe (ver `Engine::start`).
#[tauri::command]
pub async fn save_settings(app: AppHandle, payload: SaveSettingsPayload) -> Result<(), String> {
    let before = load_settings();
    let needs_restart = before.voice.as_deref().unwrap_or("Puck") != payload.voice
        || before.device_in.as_deref() != payload.device_in.as_deref()
        || before.device_out.as_deref() != payload.device_out.as_deref()
        || before.barge_in.unwrap_or(false) != payload.barge_in
        || before
            .system_prompt
            .as_deref()
            .unwrap_or(DEFAULT_SYSTEM_PROMPT)
            != payload.system_prompt;

    save(SaveSettings {
        api_key: payload.api_key.clone(),
        system_prompt: Some(payload.system_prompt.clone()),
        voice: Some(payload.voice.clone()),
        device_in: payload.device_in.clone(),
        device_out: payload.device_out.clone(),
        barge_in: Some(payload.barge_in),
        voice_fx_amount: Some(payload.voice_fx_amount),
    })
    .map_err(|err| err.to_string())?;

    if needs_restart || payload.api_key.as_deref().is_some_and(|k| !k.is_empty()) {
        crate::restart_engine(app).await;
    } else {
        set_fx_amount(app, payload.voice_fx_amount);
    }

    Ok(())
}
