# OpenJarvisBR v2 — janela flutuante estilo Siri (degrau 2: interface)

Data: 2026-09-16
Status: aprovado pelo dono em conversa

## Objetivo

Tirar o OpenJarvisBR do terminal. Um app de desktop, Mac e Windows, que fica na
barra de menu sempre conectado e ouvindo; uma janela flutuante surge quando
alguém fala e some no silêncio; uma janela de configurações edita voz,
dispositivos, efeito, barge-in, system prompt e chave.

O binário de terminal `jarvis` continua existindo para debug.

## Fora de escopo (v2)

- Visão de tela, ferramentas, integração Overclock, memória entre execuções,
  voz clonada
- Instalador assinado / notarizado (build local basta)
- Wake word

## Decisões

| Decisão | Escolha | Motivo |
|---|---|---|
| Framework | Tauri 2 | Rust por baixo, reaproveita todo o core, Mac + Windows |
| Layout do repo | workspace Cargo: `crates/core` (lib) + `crates/cli` (bin `jarvis`) + `apps/desktop` (Tauri) | separa domínio de interface |
| UI | HTML/CSS/TS com Vite, sem framework pesado; Svelte se a complexidade pedir | leveza, janela pequena |
| Ativação | sempre conectado; janela aparece ao falar | decisão do dono |
| Janela | sem borda, transparente, sempre por cima, topo central, some 4s após silêncio | estilo Siri |
| Atalho global | mutar/desmutar (padrão Cmd+Shift+J / Ctrl+Shift+J) | única ação que precisa de atalho |
| Config | mesmo `~/.config/jarvis/config.toml` de hoje | uma fonte de verdade |
| Estilo visual | definido por card próprio (pesquisa de referências) | não travar a engenharia no visual |

## Arquitetura

```
crates/core/          openjarvisbr-core (lib)
  src/audio/…         (igual a hoje)
  src/live/…          (igual a hoje)
  src/config.rs       + save()
  src/engine.rs       ex-app.rs sem terminal: Engine::start(cfg) -> (EngineHandle, EventStream)
crates/cli/           bin jarvis: usa Engine + terminal (comportamento atual)
apps/desktop/         Tauri 2
  src-tauri/          comandos, tray, atalho global, janelas
  src/                UI: overlay/ e settings/
```

### Engine (core)

- `Engine::start(EngineConfig) -> Result<EngineHandle>`
- `EngineHandle`: `mute(bool)`, `reconnect()`, `set_fx_amount(f32)`, `stop()`,
  `events() -> broadcast::Receiver<EngineEvent>`
- `EngineEvent`: `State(Idle|Connecting|Listening|Speaking|Muted|Error(String))`,
  `UserText(String)`, `ModelText(String)`, `TurnComplete`, `Level{mic: f32, model: f32}`
  (RMS a cada 50ms), `Reconnecting{attempt}`
- Trocar voz, dispositivos ou system prompt = `stop()` + `start()` com config nova.

### Desktop (Tauri)

- Ao abrir: carrega config; sem chave → abre Settings com aviso. Com chave →
  `Engine::start`, ícone na bandeja.
- Overlay: janela `overlay`, `decorations:false`, `transparent:true`,
  `alwaysOnTop:true`, `skipTaskbar:true`, 420×140, topo central do monitor
  ativo. Escondida por padrão. Mostra em `Listening` com nível de mic acima
  de limiar, em `Speaking`, em `UserText`/`ModelText`. Esconde 4s após
  `TurnComplete` sem novo evento. Nunca rouba foco.
- Overlay mostra: onda reagindo a `Level`, última fala do usuário e do
  modelo (uma linha cada, elipse), ponto de estado.
- Tray: ícone por estado; menu: Mutar/Desmutar, Reconectar, Configurações,
  Sair.
- Atalho global: mutar/desmutar; feedback no ícone e no overlay.
- Settings: janela normal 520×640 com: chave (campo senha, nunca exibida),
  voz (lista das 8), microfone e saída (listas vindas do core), intensidade
  do efeito (slider 0–1, aplica ao vivo), barge-in (toggle), system prompt
  (textarea com botão "restaurar padrão"). Salvar grava o config.toml e
  reinicia o engine se precisar.
- Comandos Tauri: `get_settings`, `save_settings`, `list_devices`,
  `set_mute`, `set_fx_amount`, `reconnect`, `quit`. Eventos Tauri:
  `engine://state`, `engine://text`, `engine://level`, `engine://error`.

## Tratamento de erros

| Situação | Comportamento |
|---|---|
| Sem chave | Settings abre com aviso; tray em estado erro |
| Chave inválida (401) | Settings abre com mensagem no campo da chave |
| Sem mic / sem saída | Settings abre na aba de dispositivos com a lista |
| Socket cai | tray "reconectando", overlay mostra 1 linha; após 3 falhas, erro no tray e botão Reconectar |
| Quota (429) | mensagem no overlay e no tray; não tenta em loop |
| Segunda instância | a nova ativa a existente e sai (plugin single-instance do Tauri) |

Nenhum log, evento ou tela exibe a chave.

## Testes

- Core: testes atuais continuam; novos para `Engine` (start/stop sem rede
  via injeção de sessão fake, eventos de nível).
- Desktop: roteiro manual em `docs/testes/v2.md`: abrir sem chave, salvar
  chave, overlay aparecer ao falar e sumir, atalho de mute, trocar voz sem
  fechar o app, derrubar rede e ver reconectar, abrir segunda instância,
  build no Windows.

## Critério de pronto

- `cargo build` do workspace e `npm run tauri build` limpos; clippy e
  testes verdes.
- App na barra de menu, overlay surge ao falar e some no silêncio, mute pelo
  atalho, configurações persistem, build Windows abre e conversa.
