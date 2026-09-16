# OpenJarvisBR v3 — braços: ferramentas locais + Overclock/OverClick via MCP

Data: 2026-09-16 · Status: aprovado pelo dono · Execução: máximo paralelismo, Opus 5

## Objetivo

O Jarvis passa a AGIR: abre apps, roda comandos, mexe em arquivos, controla
sistema e agenda, e opera o Overclock (panes, missões, handoffs) e o OverClick
(cards) por voz. Ações arriscadas pedem confirmação por voz ou botão no overlay.

## Contratos (todos os cards constroem contra isto; nada de esperar o vizinho)

```rust
// crates/core/src/tools/mod.rs
pub enum Risk { Safe, Confirm }          // Confirm = pede "confirma?" antes
pub struct ToolSpec { pub name: String, pub description: String,
                      pub parameters: serde_json::Value /* JSON Schema */, pub risk: Risk }
pub struct ToolCall { pub id: String, pub name: String, pub args: serde_json::Value }
pub struct ToolResult { pub id: String, pub output: serde_json::Value, pub error: Option<String> }
#[async_trait] pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError>;
}
pub struct Registry { .. }   // register(Box<dyn Tool>), specs(), get(name), allow-list por perfil
```

```rust
// crates/core/src/live/protocol.rs (novo)
ServerEvent::ToolCall(Vec<ToolCall>)            // de serverContent/toolCall
ServerEvent::ToolCallCancellation(Vec<String>)  // ids
SetupRequest::with_tools(&[ToolSpec])           // tools: [{functionDeclarations:[..]}]
ToolResponseRequest::new(&[ToolResult])         // {"toolResponse":{"functionResponses":[{id,name,response}]}}
LiveSession::send_tool_response(&[ToolResult])
```

```rust
// crates/core/src/engine.rs (novo)
EngineEvent::ToolRequested { call: ToolCall, risk: Risk }
EngineEvent::ToolConfirmNeeded { id: String, name: String, summary: String }
EngineEvent::ToolResult { id: String, name: String, ok: bool, summary: String }
EngineHandle::confirm_tool(id: &str, approve: bool)
// política: Safe executa direto; Confirm emite ToolConfirmNeeded e aguarda
// confirm_tool OU confirmação por voz ("sim/confirma/pode" nos 15s seguintes,
// "não/cancela" nega); timeout 20s = negado. Resultado sempre volta pro modelo.
```

```toml
# config.toml
[tools]
enabled = true
[[mcp_servers]]
name = "overclock"
url = "http://127.0.0.1:${OVERCLOCK_MCP_PORT}/mcp"     # ${VAR} expandido do ambiente
bearer_env = "OVERCLOCK_MCP_BEARER_TOKEN"              # nome da env, nunca o valor
[[mcp_servers]]
name = "overclick"
url = "https://cloud.overclock.sh/mcp"
bearer_env = "OVERCLICK_MCP_BEARER_TOKEN"
```

Perfis (JRV-52) ganham `tools: Vec<String>` (globs: "fs.*", "mcp.overclock.*").
Assistente: tudo. Parceiro de código: fs.*, shell.*, mcp.*. Professor/Terapeuta:
nenhuma. Mentor: agenda.*, mcp.overclick.*.

## Cards (onda 1 em paralelo; onda 2 integra; onda 3 valida)

| Card | Escopo (arquivos exclusivos) | Depende |
|---|---|---|
| A Protocolo de tools na Live API | crates/core/src/live/protocol.rs, session.rs, tests/ | — |
| B Framework de tools (trait, registry, política, allow-list) | crates/core/src/tools/mod.rs, registry.rs, policy.rs | — |
| C Tools locais: app/url/shell/arquivos | crates/core/src/tools/local/*.rs | contrato B |
| D Tools de sistema: volume, mídia, agenda, lembrete | crates/core/src/tools/system/*.rs | contrato B |
| M Cliente MCP (HTTP streamable + stdio) → tools `mcp.<server>.<tool>` | crates/core/src/mcp/*.rs | contrato B |
| F Desktop: overlay mostra pedido/confirmação/resultado, botões Confirmar/Negar, permissões macOS | apps/desktop/** (overlay/tools.ts, tray, settings aba Ferramentas) | contrato de eventos |
| E Engine: liga A+B+C+D+M, confirmação por voz e por handle, allow-list por perfil, system prompt de uso de ferramentas | crates/core/src/engine.rs, profiles.rs | A,B,C,D,M |
| H Roteiro v3 + build | docs/testes/v3.md, README | E,F |

Nomes de tools: `app.open`, `web.open`, `shell.run` (Confirm), `fs.read`,
`fs.write` (Confirm), `fs.list`, `sys.volume`, `media.control`,
`calendar.create` (Confirm), `reminder.set`, `mcp.<server>.<tool>` (Confirm se o
tool MCP não for de leitura: heurística por nome list/get/read/search = Safe).

## Segurança

- Tokens só por nome de env; nunca em log, evento, overlay ou commit.
- `shell.run` com timeout 30s, cwd = home, sem sudo, saída truncada a 4KB.
- `fs.*` restrito ao home do usuário; caminhos fora → erro.
- Toda ação Confirm registra no events.log do `--record`.

## Pronto quando

"Jarvis, abre o Safari" abre. "Roda ls na pasta atual" pede confirmação, executa
e lê o resultado. "Cria um card de bug no OpenJarvisBR dizendo X" cria no
OverClick após confirmar. "Abre um pane Sonnet no card JRV-60" abre no Overclock.
"O que o pane 5657 entregou?" lê o handoff e resume por voz.
