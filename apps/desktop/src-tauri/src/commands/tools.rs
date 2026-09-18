// Comandos Tauri de ferramentas (JRV-58): confirmação vinda do overlay,
// aba Ferramentas das configurações, permissões do macOS e, em build de
// desenvolvimento, um mock que dispara `engine://tool` sem o Engine.
//
// Contrato do evento `engine://tool` (spec v3 › engine.rs eventos):
// `{ kind: "requested" | "confirm_needed" | "result", id, name, summary, ok? }`.

use std::time::Duration;

use openjarvisbr_core::mcp::{self, discovery};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::tools_config;

use crate::tool_events::TOOL_EVENT;

#[derive(Debug, Clone, Serialize)]
pub struct ToolEventPayload {
    pub kind: &'static str,
    pub id: String,
    pub name: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
}

/// Emite um evento de ferramenta no mesmo formato de `tool_events` (usado
/// pelo mock de desenvolvimento).
pub fn emit_tool_event(app: &AppHandle, payload: ToolEventPayload) {
    let _ = app.emit(TOOL_EVENT, payload);
}

/// Botão Confirmar/Negar do overlay: repassa ao Engine
/// (`EngineHandle::confirm_tool`). Em build de desenvolvimento, ids do mock
/// seguem o fluxo simulado.
#[tauri::command]
pub fn confirm_tool(app: AppHandle, id: String, approve: bool) {
    tracing::info!(id = %id, approve, "confirmação de ferramenta pelo overlay");
    #[cfg(debug_assertions)]
    if id.starts_with(MOCK_ID_PREFIX) {
        mock_after_confirm(app, id, approve);
        return;
    }
    let state = app.state::<crate::AppState>();
    let guard = state.engine.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(engine) = guard.as_ref() {
        engine.confirm_tool(&id, approve);
    }
}

/// Mostra/esconde a faixa de ferramenta: cresce a janela do overlay para
/// baixo e, com botões na tela, deixa ela receber cliques (fora disso o
/// overlay ignora o mouse para não atrapalhar o que está atrás).
#[tauri::command]
pub fn set_overlay_tool_strip(app: AppHandle, visible: bool, interactive: bool) {
    crate::window_layout::set_tool_strip(&app, visible, interactive);
}

#[derive(Debug, Serialize)]
pub struct ToolsSettingsPayload {
    pub enabled: bool,
    /// `[tools].full_access` (JRV-65).
    pub full_access: bool,
    pub mcp_servers: Vec<tools_config::McpServerInfo>,
}

/// Estado da aba Ferramentas: toggle geral e servidores MCP (URL mascarada,
/// token só como origem: nome da env ou "no Keychain").
#[tauri::command]
pub fn get_tools_settings() -> ToolsSettingsPayload {
    ToolsSettingsPayload {
        enabled: tools_config::tools_enabled(),
        full_access: openjarvisbr_core::config::load_settings().full_access,
        mcp_servers: tools_config::mcp_servers(),
    }
}

/// Toggle "Acesso total" da aba Ferramentas: grava e aplica na hora, sem
/// reiniciar o motor (JRV-65).
#[tauri::command]
pub fn set_full_access(app: AppHandle, on: bool) -> Result<(), String> {
    crate::apply_full_access(&app, on)
}

/// "Adicionar servidor": token digitado vai para o Keychain/Credential
/// Manager e o config guarda só `token_keychain`. Reinicia o motor, que só
/// carrega os servidores ao subir.
#[tauri::command]
pub async fn add_mcp_server(app: AppHandle, server: tools_config::NewMcpServer) -> Result<(), String> {
    let (config, token) = tools_config::build_server(server)?;
    if let (Some(token), Some(name)) = (&token, &config.token_keychain) {
        mcp::set_keychain_token(name, token).map_err(|err| err.to_string())?;
    }
    tools_config::upsert_server(&config)?;
    tracing::info!(server = %config.name, "servidor MCP salvo");
    crate::restart_engine(app).await;
    Ok(())
}

#[tauri::command]
pub async fn remove_mcp_server(app: AppHandle, name: String) -> Result<(), String> {
    if let Some(keychain) = tools_config::remove_server(&name)? {
        mcp::delete_keychain_token(&keychain).map_err(|err| err.to_string())?;
    }
    crate::restart_engine(app).await;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct McpTestResult {
    pub ok: bool,
    /// "N tools" no sucesso; a mensagem do core no erro (sem token/endereço).
    pub message: String,
}

const MCP_TEST_TIMEOUT: Duration = Duration::from_secs(15);

/// "Testar": conecta e chama `tools/list` com o cliente MCP do core.
#[tauri::command]
pub async fn test_mcp_server(name: String) -> McpTestResult {
    let Some(config) = tools_config::find_server(&name) else {
        return McpTestResult { ok: false, message: "servidor não encontrado no config".to_string() };
    };
    probe(config).await
}

async fn probe(config: mcp::McpServerConfig) -> McpTestResult {
    let kind = discovery::Kind::of(&config);
    let client = mcp::McpClient::new(config);
    match tokio::time::timeout(MCP_TEST_TIMEOUT, client.list_tools()).await {
        Ok(Ok(tools)) => McpTestResult { ok: true, message: format!("{} tools", tools.len()) },
        Ok(Err(err)) => McpTestResult { ok: false, message: describe_error(kind, &err) },
        Err(_) => McpTestResult { ok: false, message: "tempo esgotado".to_string() },
    }
}

/// Mensagem da aba para um erro do core; recusa no Overclock local é o app
/// fechado (sem porta nem endereço na tela).
fn describe_error(kind: Option<discovery::Kind>, err: &mcp::McpError) -> String {
    match (kind, err) {
        (Some(discovery::Kind::Overclock), mcp::McpError::Refused) => "Overclock fechado".to_string(),
        _ => err.to_string(),
    }
}

/// "Conectar Overclock" / "Conectar OverClick" (JRV-68): descobre URL e
/// token no que o Overclock grava em disco (ou no ~/.claude.json, para o
/// OverClick), guarda o token no Keychain, grava o servidor no config só com
/// `token_keychain`, reinicia o motor e devolve o teste de conexão.
#[tauri::command]
pub async fn connect_mcp_server(app: AppHandle, kind: String) -> Result<McpTestResult, String> {
    let kind = discovery::Kind::parse(&kind).ok_or_else(|| format!("servidor desconhecido: {kind}"))?;
    let existing = tools_config::find_server(kind.as_str());
    let config = tauri::async_runtime::spawn_blocking(move || discovery::connect(kind, existing.as_ref()))
        .await
        .map_err(|_| "falha interna ao descobrir o servidor".to_string())?
        .map_err(|err| err.to_string())?;
    tools_config::upsert_server(&config)?;
    tracing::info!(server = %config.name, "servidor MCP descoberto e salvo");
    let result = probe(config).await;
    crate::restart_engine(app).await;
    Ok(result)
}

/// Links externos da aba (onde pegar o token do OverClick). Só destinos
/// conhecidos, nunca uma URL arbitrária vinda da webview.
#[tauri::command]
pub fn open_tools_link(link: String) -> Result<(), String> {
    let url = match link.as_str() {
        "overclick_tokens" => "https://cloud.overclock.sh",
        other => return Err(format!("link desconhecido: {other}")),
    };
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("não foi possível abrir o navegador: {err}"))
}

#[cfg(debug_assertions)]
const MOCK_ID_PREFIX: &str = "mock-";

/// Dev: dispara um evento falso de ferramenta. `kind` ausente roda o fluxo
/// completo (requested → confirm_needed; o resultado sai ao confirmar/negar
/// no overlay). Em build de release só devolve erro.
#[tauri::command]
pub fn emit_tool_mock(app: AppHandle, kind: Option<String>) -> Result<(), String> {
    #[cfg(debug_assertions)]
    {
        crate::open_overlay_window(&app);
        run_tool_mock(&app, kind.as_deref())
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (app, kind);
        Err("emit_tool_mock só existe em build de desenvolvimento".to_string())
    }
}

#[cfg(debug_assertions)]
pub fn run_tool_mock(app: &AppHandle, kind: Option<&str>) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let id = format!("{MOCK_ID_PREFIX}{}", SEQ.fetch_add(1, Ordering::SeqCst));
    let event = |kind: &'static str, ok: Option<bool>, summary: &str| ToolEventPayload {
        kind,
        id: id.clone(),
        name: "shell.run".to_string(),
        summary: summary.to_string(),
        ok,
    };
    match kind {
        Some("requested") => emit_tool_event(app, event("requested", None, "ls na pasta atual")),
        Some("confirm_needed") => emit_tool_event(app, event("confirm_needed", None, "ls na pasta atual")),
        Some("result") => emit_tool_event(app, event("result", Some(true), "12 arquivos listados")),
        None => {
            emit_tool_event(app, event("requested", None, "ls na pasta atual"));
            emit_tool_event(app, event("confirm_needed", None, "ls na pasta atual"));
        }
        Some(other) => return Err(format!("kind desconhecido: {other}")),
    }
    Ok(())
}

#[cfg(debug_assertions)]
fn mock_after_confirm(app: AppHandle, id: String, approve: bool) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        let (ok, summary) = if approve {
            (true, "12 arquivos listados")
        } else {
            (false, "negado pelo usuário")
        };
        emit_tool_event(
            &app,
            ToolEventPayload {
                kind: "result",
                id,
                name: "shell.run".to_string(),
                summary: summary.to_string(),
                ok: Some(ok),
            },
        );
    });
}
