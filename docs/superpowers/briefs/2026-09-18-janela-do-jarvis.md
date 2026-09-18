# Briefing — janela do Jarvis: mover, redimensionar, minimizar

Pedido do dono (2026-09-18): "não consigo minimizar, não consigo diminuir nem aumentar o
tamanho, não consigo mover. Essas coisas são muito necessárias."

Regra do workspace: criar o card no OverClick (projeto do repo `bside/jarvis`) e
`task_claim` antes de começar.

## Contexto técnico (apps/desktop/src-tauri)

- `tauri.conf.json`: a janela `main` é 1x1 invisível, só âncora da bandeja. Não é ela.
- **Overlay** (`src/main.rs`, `WebviewWindowBuilder::new(.., "overlay", ..)`):
  `decorations(false)`, `resizable(false)`, `focusable(false)`, `always_on_top(true)`,
  `skip_taskbar(true)` e, depois de criada, `set_ignore_cursor_events(true)`. Ou seja,
  hoje não dá para mover, redimensionar nem minimizar por projeto. Tamanho fixo por
  estilo: 420x140 (orb) / 520x220 (surreal); posição top-center (`OVERLAY_TOP_MARGIN`).
- **Configurações** (`src/main.rs`, label `settings`): tem decorations (move e minimiza),
  mas `resizable(false)` e `inner_size(520, 640)` fixos.
- Restrição de produto (comentários no código): o overlay **nunca pode virar key window**
  (não roubar foco de quem está digitando) e os botões Confirmar/Negar precisam de
  `accept_first_mouse(true)`. Preservar isso.

## Deliverable

1. Configurações: `resizable(true)` com `min_inner_size`; lembrar tamanho e posição.
2. Overlay:
   - **mover**: região de arraste (`data-tauri-drag-region` em `overlay.html`) com
     `set_ignore_cursor_events(false)` enquanto o mouse estiver sobre o overlay, ou um
     "modo mover" no menu da bandeja;
   - **tamanho**: escala pequeno/médio/grande no menu da bandeja e nas Configurações;
   - **ocultar/minimizar**: item "Ocultar overlay" no menu + atalho global, e botão de
     fechar no próprio overlay.
3. Persistir posição e escala em `~/.config/jarvis/config.toml` (seção `[overlay]`) sem
   tocar `[reflex]`/`[tools]`: seguir o padrão `save_tools_value` de `crates/core/src/config.rs`,
   que preserva o resto do arquivo.
4. Validar no app real: `cd apps/desktop && npm run tauri dev` (porta 1420; se estiver
   presa, é Vite órfão deste repo: matar e subir de novo). Roteiro leigo em `docs/testes/v2.md`.
5. Commits pequenos `feat(desktop): … [JRV]`, `git add` por caminho.

## Achados pendentes de card (mesma sessão, não incluídos acima)

- `1b493e8` `web.open` ganhou `browser` (autorizado pelo dono, card retroativo).
- Dedup só cobre reflexo→Gemini; o inverso repete a ação ~400 ms depois
  (`app.open` Safari às 19:22:42 e 19:22:43 no trace).
- `app.open` falha com nome localizado ("Calculadora" vs "Calculator").
- Desktop passa `record_dir: None`: sem `events.log`, não há trace de `user_text`,
  `tool_call`, `tool_result` fora do CLI.
- Bundle `.app` sai sem `_CodeSignature`; `codesign --verify --strict` falha.
