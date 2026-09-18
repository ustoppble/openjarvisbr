# Briefing — orquestrador: autonomia do Jarvis (loop de autoaprimoramento)

Papel deste pane: **orquestrador**. Pode abrir panes (`pane_spawn`) e é o dono do board:
todo trabalho vira card no OverClick (projeto do repo `bside/jarvis`, missão nova para
este objetivo) **antes** de qualquer pane executar. Regra do dono, 2026-08-19.

## Meta do dono (2026-09-18, literal)

"quero que voce entre em loop pra conseguir fazer autoaprimoramento, até conseguir ter
autonomia total pra abrir aplicativos, abrir o google chrome, navegar, abrir o safari,
navegar, saber quando usar o jev, quando não usar."

Interpretação de trabalho: um ciclo **medir → corrigir → rebuild → medir** até que um
roteiro fixo de comandos de voz passe inteiro sem intervenção. O ciclo só existe se houver
medição; começar pela medição.

## O que já existe (estado do `main`)

- `a3efcba` `66b512d` `d39d028` — memória de ações do reflexo (M1): `reflex/memory.rs`,
  `Inventory.learned`, `EyeHandle::learn/forget/forget_all`, juiz com pergunta/intent
  `learned`.
- `ae6b572` `75dbae8` — engine aprende toda ação do Gemini e age por risco (M2); aba
  Reflexo no desktop.
- `1b493e8` — `web.open` aceita `browser` ("abre o google.com no Safari" agora abre no
  Safari; apelidos chrome/edge/brave/firefox/arc/opera). **Sem card ainda** (autorizado
  pelo dono; card retroativo).
- `7de8b79` — tracing no engine: `usuário disse`, `modelo disse`, `turno concluído`
  (debug), `tool pedida pelo modelo` (info, risco + resumo mascarado), `tool concluída`
  (info, origem modelo/reflexo, ms), `tool falhou` (warn, erro). **Sem card ainda.**
  Ainda **não está no build** que roda: ver "Build" abaixo.
- Em andamento no **pane-5836**: janela do Jarvis (overlay e Configurações: mover,
  redimensionar, ocultar). Briefing: `docs/superpowers/briefs/2026-09-18-janela-do-jarvis.md`.
  Ele está editando `crates/core/src/config.rs` (campos `overlay`, `settings_window`) no
  tree compartilhado; enquanto não fechar, o crate não compila no tree. Coordenar: ou
  espera o commit dele, ou verifica em worktree (`git worktree add --detach /tmp/x HEAD`,
  `CARGO_TARGET_DIR=<repo>/target-jrv56` reaproveita cache).

## Achados do trace desta tarde (cada um vira card)

1. **Dedup só cobre reflexo→Gemini.** Quando o Gemini age primeiro e o reflexo decide
   ~400 ms depois, executa de novo (`app.open` Safari às 19:22:42.62 e 19:22:43.08).
   Com `shell.run` aprendido isso é execução dupla. Corrigir em `on_reflex`: se o modelo
   já pediu (ou concluiu) a mesma (tool, args) neste turno, não repetir.
2. **`app.open` com nome localizado falha** ("Calculadora" → `open -a` só conhece
   "Calculator"). Resolver por bundle id ou tabela de apelidos pt-BR, ou consultar o
   inventário do Olho (`Inventory.installed_apps`) por prefixo normalizado.
3. **Ação MCP de escrita aprendida a partir de transcrição ruim**: às 19:27:36 o Gemini
   executou `mcp.overclock.pane_write` com a frase "Eu quero que você mande no PN 38 não
   5834 conta 10 + 10" e o reflexo aprendeu. Política: `mcp.*` de escrita deve ser
   `Confirm` (spec) — checar se está, e considerar não aprender ações `Confirm` que foram
   aprovadas uma vez só.
4. **Sem navegação no navegador.** Só existe `web.open`. Para "navega", "volta", "pesquisa
   isso", "clica em X" precisa de família nova, ex. `browser.*` (Chrome/Safari via
   AppleScript: aba nova, URL, voltar, buscar; JS na aba para clicar por texto) e/ou
   `ui.*` via System Events (o desktop já tem Acessibilidade/Automação em
   `apps/desktop/src-tauri/src/commands/permissions.rs`). Precisa spec antes (brainstorming).
5. **Quando usar o Jev.** Hoje o reflexo pergunta ao Jev a cada fragmento de fala. Medir:
   latência por decisão (no trace: 300–700 ms), taxa de `Act` vs `Nothing`, quantas
   `Act` foram redundantes com o Gemini. Regra candidata: só perguntar quando há
   candidato (app/site/learned) com token em comum; pular quando a fala é conversa.
6. **Bundle sem `_CodeSignature`** — `codesign --verify --strict` falha. Ver "Senha".
7. **Perfil `business_mentor`** não libera `app.*`/`web.*`; o dono trocou para `assistant`.
   Considerar aviso no overlay quando o reflexo/Gemini pede tool fora do perfil.

## Senha / Keychain a cada build (pedido do dono)

O dono pediu para "pedir a senha uma única vez". **Nenhum pane pode receber, digitar ou
guardar a senha dele.** A causa real: cada build ad-hoc muda a assinatura, e o Keychain
trata como app novo, pedindo a senha para liberar a chave da API. Correção certa:
assinar com identidade estável.
- Caminho: o dono cria um certificado de assinatura de código auto-assinado no
  Keychain Access (Certificate Assistant → Create a Certificate → tipo "Code Signing",
  nome ex. `OpenJarvisBR Dev`), e o `tauri.conf.json` ganha
  `bundle.macOS.signingIdentity = "OpenJarvisBR Dev"`. Isso também gera `_CodeSignature`
  e resolve o achado 6.
- Alternativa sem cert: chave via `GEMINI_API_KEY` no ambiente ou no config.toml (o core
  já aceita), sem Keychain. Menos seguro; decisão do dono.

## Build e observação

- Instalador: `cd apps/desktop && npm run tauri build` → `target/release/bundle/dmg/OpenJarvisBR_<versão>_aarch64.dmg`
  e `.../macos/OpenJarvisBR.app`. Antes de rodar, `pkill -f openjarvisbr-desktop`.
- Rodar com trace (o `open` descarta stderr):
  `RUST_LOG="info,openjarvisbr_core=debug,openjarvisbr_core::audio=warn" target/release/bundle/macos/OpenJarvisBR.app/Contents/MacOS/openjarvisbr-desktop >> /tmp/jarvis-trace.log 2>&1 &`
- Trace da tarde: `/tmp/jarvis-trace.log` (limpar códigos ANSI com `sed -E 's/\x1b\[[0-9;]*m//g'`).
- Modo dev: porta 1420; se presa, é Vite órfão deste repo (matar e subir de novo).
- Nunca imprimir chave/senha; `call_summary` já mascara, mantenha assim.

## Roteiro de aceitação (o "loop" termina quando passa inteiro, sem intervenção)

1. "abre o Safari" → Safari na frente, uma execução só.
2. "abre o google.com no Safari" → Safari, não Chrome.
3. "abre o Chrome" / "abre o youtube no Chrome".
4. "volta" / "pesquisa overclock" na aba atual (precisa `browser.*`).
5. "abre a Calculadora" (nome pt-BR).
6. "abre a globo" duas vezes → segunda via reflexo (`origem=reflexo`), sem duplicar.
7. "diminui o som" / "coloca no máximo".
8. Conversa sem pedido ("que horas são?") → reflexo `Nothing` e, idealmente, sem chamar o Jev.

Medir cada rodada pelo trace: tempo fala→ação, origem, duplicatas, erros de tool.

## Adendo (dono, 2026-09-18): arquitetura do loop — observador + agente maior

Sugestão que o dono aprovou e quer que o orquestrador faça: **um MCP observador**
que acompanha o agente menor (o Jarvis rodando: Gemini + reflexo/Jev), quebra o que
observa em tarefas, e um **agente maior** (o orquestrador/panes) lê essas tarefas,
analisa e aprimora o Jarvis, em ciclo.

Forma concreta sugerida:
- **Fonte**: o trace já existe (`7de8b79`): `usuário disse`, `tool pedida`, `tool
  concluída`/`tool falhou` com ms e origem, decisões do reflexo com latência. O MCP
  observador lê esse log (ou um `events.jsonl` que o engine passe a gravar) e expõe
  tools como `observe_session` (resumo da última sessão: pedidos, ações, falhas,
  duplicatas, latências), `list_failures`, `propose_tasks` (uma tarefa por padrão de
  falha, com evidência `timestamp + linha`).
- **Quebra em tarefas**: cada padrão vira card no OverClick com `o_que`, `por_que`,
  `como_confirmo` (a linha do trace que deve aparecer/desaparecer).
- **Agente maior**: o orquestrador pega os cards, delega panes, rebuilda, reroda o
  roteiro de aceitação e fecha o ciclo comparando o trace antes/depois.
- Fica no repo (ex. `crates/observer-mcp` ou script em `tools/`), registrado no
  `config.toml` como MCP para o próprio Jarvis poder consultar ("o que deu errado hoje?").
