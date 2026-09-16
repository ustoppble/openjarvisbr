# Plano de Execução — OpenJarvisBR v2 Backend + Engenharia

**Data:** 2026-09-16  
**Responsável:** Claude (effort low) + pane-5644 (UI/Design)  
**Sequência:** JRV-29 → JRV-30 → JRV-31 → JRV-32 → JRV-33 → JRV-34 → JRV-35

---

## JRV-29 — Workspace Cargo + Refactor Core Lib

**O quê:** Reorganizar repositório em workspace Cargo com 3 crates:
- `crates/core/` — openjarvisbr-core (lib)
- `crates/cli/` — bin `jarvis` (CLI atual)
- `apps/desktop/` — Tauri 2 (novo)

**Definição de Pronto:**
- `cargo build` funciona limpo (todos os crates)
- CLI `jarvis` funciona igual a hoje (sem mudança de comportamento)
- `clippy` e testes passam
- Arquivo `Cargo.toml` na raiz + toml individual por crate

**Estimativa:** 2–3 horas (refactor estrutural)

**Próximas:** JRV-30 depende disso

---

## JRV-30 — Engine (Core Lib Refactor)

**O quê:** Extrair lógica da CLI atual em uma API pública `Engine`:

```rust
pub struct Engine { /* privado */ }
pub struct EngineHandle { /* handle público */ }

impl Engine {
    pub fn start(cfg: EngineConfig) -> Result<EngineHandle>;
}

impl EngineHandle {
    pub fn mute(&self, muted: bool);
    pub fn reconnect(&self);
    pub fn set_fx_amount(&self, amount: f32);
    pub fn stop(&self);
    pub fn events(&self) -> broadcast::Receiver<EngineEvent>;
}

pub enum EngineEvent {
    State(EngineState),
    UserText(String),
    ModelText(String),
    TurnComplete,
    Level { mic: f32, model: f32 },
    Reconnecting { attempt: u32 },
}

pub enum EngineState {
    Idle,
    Connecting,
    Listening,
    Speaking,
    Muted,
    Error(String),
}
```

**Definição de Pronto:**
- API acima compila e testa
- CLI `jarvis` usa Engine (sem mudança externa)
- `Engine::start` roda com config.toml
- EventStream broadca eventos a cada 50ms (Level)
- `cargo test` passa (fixtures dos eventos)

**Estimativa:** 3–4 horas (refactor + broadcast wiring)

**Próximas:** JRV-31 consome Engine via Tauri

---

## JRV-31 — Tauri Scaffold + Tray + Atalho Global

**O quê:** Setup inicial do Tauri 2 + menu na bandeja + atalho Cmd+Shift+J

**Componentes:**
1. `apps/desktop/src-tauri/` — Rust backend Tauri
   - main.rs: setup, tray menu, atalho global
   - Comandos Tauri: `get_settings`, `save_settings`, `list_devices`, `set_mute`, `set_fx_amount`, `reconnect`, `quit`
   - Eventos: `engine://state`, `engine://text`, `engine://level`, `engine://error`
2. `apps/desktop/src/` — Frontend placeholder (HTML/CSS/TS)
   - Vite config básico
   - index.html vazio
   - Espaço pronto pra overlay + settings

**Definição de Pronto:**
- `npm run tauri build` compila sem erro
- App na bandeja (ícone + menu básico)
- Atalho Cmd+Shift+J togla mute
- Abrir Settings janela (vazio ainda)
- Engine inicia ao abrir app
- Clippy passa

**Estimativa:** 3–4 horas (boilerplate Tauri + wiring commands)

**Próximas:** JRV-32 cria overlay window; JRV-33 cria settings

---

## JRV-32 — Overlay Window (Container Vazio)

**O quê:** Janela flutuante 420×140 transparente, sempre por cima. **Sem UI gráfica** (será feito por pane-5644).

**Especificação Tauri:**
```rust
webview_builder
    .label("overlay")
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .inner_size(420.0, 140.0)
    .position(/* topo central do monitor ativo */)
    .create()
```

**Frontend (apps/desktop/src/overlay/):**
- `overlay.html` — container vazio com `#app` div
- `overlay.css` — reset + vars de cor (da spec JRV-28)
- `overlay.ts` — event listener pra engine events (state, text, level)
  - Recebe eventos Tauri
  - Log no console (debug)
  - Ready pra integrar UI gráfica

**Comportamento:**
- Escondida por padrão (visibility hidden)
- Mostra em `Listening` + mic > limiar
- Mostra em `Speaking`
- Mostra em `UserText`/`ModelText`
- Esconde 4s após `TurnComplete`
- Nunca rouba foco

**Definição de Pronto:**
- Janela aparece 420×140, topo central
- Eventos chegam no console (teste com Engine rodando)
- Comportamento de show/hide por estado está funcionando
- CSS vars prontas pra UI (--color-primary, --color-accent, etc.)

**Estimativa:** 2–3 horas (window setup + event wiring)

**Próximas:** pane-5644 integra UI gráfica aqui; JRV-33 paralelo

---

## JRV-33 — Settings Window

**O quê:** Janela normal 520×640 com formulários de configuração.

**Campos:**
1. **Chave** — input type=password (nunca exibida)
2. **Voz** — select com 8 vozes (vindas do core)
3. **Microfone** — select com devices (list_devices do core)
4. **Saída** — select com devices (playback)
5. **Intensidade do Efeito** — slider 0–1 (set_fx_amount ao vivo)
6. **Barge-in** — toggle
7. **System Prompt** — textarea + botão "restaurar padrão"
8. **Salvar** — button (grava config.toml, reinicia Engine)

**Frontend (apps/desktop/src/settings/):**
- `settings.html` — formulário estruturado
- `settings.css` — layout 2-column, responsive
- `settings.ts` — handlers pra get_settings, save_settings, list_devices

**Comportamento:**
- Abre ao iniciar sem chave (com aviso "Configure chave")
- Aberta pelo menu tray "Configurações"
- Salvar → grava ~/.config/jarvis/config.toml
- Se mudou chave/voz/device → reinicia Engine
- Se mudou fx_amount → aplica ao vivo (sem reiniciar)

**Definição de Pronto:**
- Janela abre e carrega valores atuais
- Campos preenchem corretamente
- Salvar funciona (arquivo gravado)
- list_devices retorna devices reais
- Clippy passa

**Estimativa:** 2–3 horas (formulários + wiring)

**Próximas:** JRV-34 paralelo (erros + single instance)

---

## JRV-34 — Error Handling + Single Instance

**O quê:** Tratamento de erros + evitar múltiplas instâncias

**Situações de Erro:**
1. **Sem chave** → Settings abre com aviso
2. **Chave inválida (401)** → Msg no campo da chave
3. **Sem mic/saída** → Settings abre na aba de devices
4. **Socket cai** → Tray em "Reconectando"; overlay mostra 1 linha; após 3 falhas → erro no tray + botão Reconectar
5. **Quota (429)** → Msg no overlay + no tray; não tenta em loop

**Single Instance:**
- Usar plugin `tauri-plugin-single-instance`
- Segunda instância → ativa a existente (traz ao foco)

**Definição de Pronto:**
- Clippy passa
- Cada erro tratado conforme spec
- Segunda instância detectada e ativa a primeira
- Testes de erro (fixtures com sessão fake)

**Estimativa:** 2 horas

**Próximas:** JRV-35 (testes + Windows build)

---

## JRV-35 — Roteiro de Testes + Build Windows

**O quê:** QA manual + compilação no Windows

**Roteiro de Testes (docs/testes/v2.md):**
1. [ ] Abrir sem chave → Settings com aviso
2. [ ] Salvar chave, abrir novamente → Engine conecta
3. [ ] Falar → Overlay aparece, onda reage
4. [ ] Silêncio 4s → Overlay some
5. [ ] Atalho Cmd+Shift+J → Muta, ícone muda
6. [ ] Mudar voz em Settings → Engine reinicia
7. [ ] Derrubar rede → Tray "Reconectando"
8. [ ] 3 falhas → Botão Reconectar aparece
9. [ ] Abrir segunda instância → Primeira se ativa
10. [ ] Build Windows → .exe funciona

**Build Windows:**
- `npm run tauri build -- --target x86_64-pc-windows-msvc`
- Testar no Windows (máquina virtual ou real)
- Installer criado

**Definição de Pronto:**
- Checklist de testes 100% verde
- Build Windows sem erro
- clippy + cargo test verdes
- Documentação de testes pronta

**Estimativa:** 3–4 horas (QA manual + build)

---

## Dependências & Paralelismo

```
JRV-29 (Workspace)
  ↓
JRV-30 (Engine)
  ↓
JRV-31 (Tauri + Tray + Atalho)
  ├→ JRV-32 (Overlay Window) ←← pane-5644 integra UI gráfica aqui
  └→ JRV-33 (Settings)
       ↓
    JRV-34 (Erros + Single Instance)
       ↓
    JRV-35 (Testes + Windows Build)
```

**Paralelismo:**
- JRV-32 e JRV-33 podem rodar em paralelo (após JRV-31)
- JRV-34 pode começar após JRV-32 (depende de overlay setup)
- pane-5644 trabalha em paralelo em JRV-32 (UI gráfica na janela)

---

## Tempo Total Estimado

| Card | Estimativa |
|------|-----------|
| JRV-29 | 2–3h |
| JRV-30 | 3–4h |
| JRV-31 | 3–4h |
| JRV-32 | 2–3h |
| JRV-33 | 2–3h |
| JRV-34 | 2h |
| JRV-35 | 3–4h |
| **Total** | **18–24h** |

Com paralelismo (JRV-32/33 simultâneas + pane-5644 em paralelo): **12–16h wallclock**

---

## Convenções Git

Cada card:
- Nova branch: `jrv-NN-descricao`
- Commit único: `[JRV-NN] Descrição`
- `git add` por pathspec (não `git add -A`)
- Push antes de task_deliver

---

## Próximos Passos

1. **Agora:** Criar cards no OverClick (task_create para cada JRV-29 a JRV-35)
2. **Laschuk:** Escolher entre as 7 direções de design (ou deitar pro surreal de pane-5644)
3. **Execução:** Claude começa JRV-29; pane-5644 executa UI gráfica em paralelo

---

**Status:** Plano pronto. Aguardando aprovação + criação das cards.
