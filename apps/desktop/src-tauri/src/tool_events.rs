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

/// Evento do reflexo para o overlay: `engine://reflex` com
/// `{ kind: "acted" | "confirmed", name?, summary?, latency_ms?, approve? }`
/// (contrato da Task 18, consumido por `overlay/tools.ts`).
pub(crate) const REFLEX_EVENT: &str = "engine://reflex";

#[derive(serde::Serialize, Clone)]
pub(crate) struct ReflexPayload<'a> {
    pub(crate) kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) latency_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) approve: Option<bool>,
}

/// Emite `engine://reflex`. A confiança do Jev não vai ao overlay.
pub(crate) fn emit_reflex(app: &AppHandle, event: &EngineEvent) {
    let payload = match event {
        EngineEvent::ReflexActed {
            call, latency_ms, ..
        } => ReflexPayload {
            kind: "acted",
            name: Some(&call.name),
            summary: Some(call_summary(call)),
            latency_ms: Some(*latency_ms),
            approve: None,
        },
        EngineEvent::ReflexConfirmed { approve, .. } => ReflexPayload {
            kind: "confirmed",
            name: None,
            summary: None,
            latency_ms: None,
            approve: Some(*approve),
        },
        _ => return,
    };
    let _ = app.emit(REFLEX_EVENT, payload);
}
