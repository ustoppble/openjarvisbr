// Lista "Sempre permitido" da aba Ferramentas (JRV-66): lê e remove nomes de
// `[tools].always_allow`. A remoção vale na hora no motor em execução.

use openjarvisbr_core::config;
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn get_always_allow() -> Vec<String> {
    config::load_settings().always_allow
}

#[tauri::command]
pub fn remove_always_allow(app: AppHandle, name: String) -> Result<(), String> {
    let list: Vec<String> = config::load_settings()
        .always_allow
        .into_iter()
        .filter(|tool| tool != &name)
        .collect();
    config::save_always_allow(&list).map_err(|err| err.to_string())?;
    tracing::info!(ferramenta = %name, "removida de sempre permitido");
    let state = app.state::<crate::AppState>();
    let guard = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(engine) = guard.as_ref() {
        engine.set_always_allow(list);
    }
    Ok(())
}
