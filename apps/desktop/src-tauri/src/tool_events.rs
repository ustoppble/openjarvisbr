//! Ponte dos eventos de ferramenta do Engine para o overlay: `engine://tool`
//! com `{ kind: "requested" | "confirm_needed" | "result", id, name, summary,
//! ok? }` (contrato da spec v3, consumido pelo JRV-58).

use openjarvisbr_core::engine::EngineEvent;
use openjarvisbr_core::engine_tools::call_summary;
use tauri::{AppHandle, Emitter};

pub const TOOL_EVENT: &str = "engine://tool";

/// Payload do evento, ou `None` se não for evento de ferramenta.
pub fn payload(event: &EngineEvent) -> Option<serde_json::Value> {
    let value = match event {
        EngineEvent::ToolRequested { call, .. } => serde_json::json!({
            "kind": "requested",
            "id": call.id,
            "name": call.name,
            "summary": call_summary(call),
        }),
        EngineEvent::ToolConfirmNeeded { id, name, summary } => serde_json::json!({
            "kind": "confirm_needed",
            "id": id,
            "name": name,
            "summary": summary,
        }),
        EngineEvent::ToolResult {
            id,
            name,
            ok,
            summary,
        } => serde_json::json!({
            "kind": "result",
            "id": id,
            "name": name,
            "summary": summary,
            "ok": ok,
        }),
        _ => return None,
    };
    Some(value)
}

pub fn emit(app: &AppHandle, event: &EngineEvent) {
    if let Some(payload) = payload(event) {
        let _ = app.emit(TOOL_EVENT, payload);
    }
}
