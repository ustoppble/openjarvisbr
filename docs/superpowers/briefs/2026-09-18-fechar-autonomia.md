# Briefing — fechar a autonomia do Jarvis (o que falta)

Repo: /Users/laschuk/Developer/bside/jarvis, branch main. Um pane só, nada em paralelo
(o dono mandou fechar a orquestração paralela: muita colisão no tree compartilhado).
Regras: card no OverClick antes de cada item (projeto do repo, missão "Autonomia do
Jarvis"), commit por pathspec com o ID do card, nunca `git add -A`. Nunca pedir,
digitar ou guardar senha; nunca imprimir chave, token ou endereço. Use
`CARGO_TARGET_DIR=/Users/laschuk/Developer/bside/jarvis/target-5834` (cache pronto).
Verifique em worktree limpo (`git worktree add --detach /tmp/x HEAD`) quando o tree
compartilhado tiver sujeira de outros (hoje: `crates/core/src/reflex/mod.rs` modificado
por outro pane, não commitado — não toque, não descarte; avise o dono).

## Critério de pronto (a régua)

`jarvis --script docs/testes/roteiro-autonomia.txt` roda inteiro sem ninguém falar e
`tools/jarvis_trace_report.py <log> --aceitacao` passa: duplicatas=0, falhas=0,
recusas=0; "abre o Safari", "abre o google.com no Safari", "abre o Chrome", "abre o
youtube no Chrome", "volta", "pesquisa overclock", "abre a Calculadora" executados;
o segundo "abre a globo" com origem=reflexo; "que horas são?" sem chamar o Jev.
Anexar a medição final em docs/superpowers/briefs/ (medições anteriores estão lá).

## Estado (2026-09-18, fim do dia)

- main verde: `cargo test -p openjarvisbr-core -p openjarvisbr-cli -- --test-threads=1`
  258 ok, sem travar; clippy -D warnings ok.
- Working tree: `crates/cli/src/script.rs` tem o JRV-95 pronto (espera turno concluído +
  zero tools pendentes + 900 ms de silêncio; 25 s vira teto), compila, testes do CLI ok,
  clippy ok. **Falta o teste real e o commit.** Rode um roteiro de 3 linhas
  ("abre a Calculadora", "abre a globo", "que horas são?") com
  `RUST_LOG="info,openjarvisbr_core=debug,openjarvisbr_core::audio=warn"` e confira no
  relatório que nenhuma tool aparece no turno seguinte ao que a pediu. Commit `[JRV-95]`.
- Como rodar: `cargo run --release --bin jarvis -- --script <arquivo> 2>> <log>`; depois
  `python3 tools/jarvis_trace_report.py <log> --since <ISO> --aceitacao`.
- O app de desktop em /Applications pode estar aberto ouvindo o mic: feche
  (`pkill -f openjarvisbr-desktop`) antes de uma rodada de medição, senão ele responde ao
  áudio do CLI.

## Estado dos itens (atualizado 2026-09-19 pelo pane-5834)

- [x] 1 JRV-95 runner espera a fala fechar — 4140b7c (pane-5882).
- [x] 2 medição do main — 35eb5e8 [JRV-106]: recusas 0, falhas 0, duplicatas 6 (modelo
      repetindo a própria chamada; causa = contador de falas zerado no TurnComplete
      após cada tool call). Correção em curso: **JRV-107** (pane-5882, engine.rs).
- [ ] 3 JRV-85 não aprender ação Confirm — pane-5882, depois do JRV-107.
- [x] 4 gate do Jev — já existia (fe986d3, JRV-87/104): sem candidato app/site/aprendido
      e sem confirmação pendente, não consulta; `skipped_judge` no resumo.
- [x] 5 "só diz Feito." — e1ac890 (prompt base; todos os perfis herdam).
- [x] 6 memória compartilhada app+CLI — c80e00e (merge antes de gravar).
- [ ] 7 rodada final 0/0/0 com o instalador assinado — depois do JRV-107 e do JRV-85.

## Itens que faltam, nesta ordem (texto original)

1. **JRV-95** (acima): testar e commitar.
2. **Rodada de medição com o main atual.** Ninguém mediu depois dos commits de fechamento
   (browser.* JRV-96, app.open pt-BR JRV-84, retentativa do Jev). Gere a medição, compare
   com docs/superpowers/briefs/*medicao* e liste o que ainda falha.
3. **JRV-85 — não aprender ação de risco Confirm.** `Worker::learn` em
   crates/core/src/engine.rs aprende toda chamada bem-sucedida, inclusive shell.run e
   mcp.* de escrita (foi assim que `mcp.overclock.pane_write` entrou na memória a partir de
   uma transcrição ruim). Regra: só aprende se o risco efetivo foi Safe (política Safe ou
   já liberada "para sempre"). Teste primeiro.
4. **Gate do Jev** ("saber quando usar o Jev"). Hoje o reflexo consulta o Jev a cada
   fragmento de fala; nas rodadas, 60–90% deram `Nothing`. Regra: só consultar quando
   `build_questions` tem candidato real (app, site, ação aprendida) ou há confirmação
   pendente; senão pular e logar `skipped_judge`. Arquivos: crates/core/src/reflex/
   (decide.rs / mod.rs — atenção ao mod.rs sujo). O relatório já conta "Jev gasto sem
   ação"; a meta é derrubar essa taxa sem perder nenhum Act do roteiro.
5. **Regra "só diz feito".** Pedido literal do dono: "toda vez que ele executa algo, não
   precisa dizer que executou o comando; só diz feito". Entra no system prompt padrão
   (`default_system_prompt` em crates/core/src/config.rs) e nos perfis em
   crates/core/src/profiles.rs: depois de uma tool bem-sucedida, responder apenas
   "Feito." (sem repetir o que foi feito); em erro, dizer o erro em uma frase.
6. **Duas instâncias, uma memória.** App e CLI gravam o mesmo
   ~/.config/jarvis/reflex_memory.toml; último a salvar vence. Mínimo: reler o arquivo
   antes de gravar e mesclar por (tool, args). Card novo.
7. **Rodada final** com o instalador assinado (`cd apps/desktop && npm run tauri build`,
   `.app` em target/release/bundle/macos) instalado em /Applications, régua 0/0/0.
   Handoff com a medição.

## Contexto que economiza tempo

- Trace: `usuário disse`, `tool pedida pelo modelo`, `tool concluída` (origem
  modelo/reflexo, ms), `tool falhou`, `reflexo decision=…`, `reflexo agiu`,
  `reflexo aprendeu a ação`, `modelo já pediu esta ação; reflexo não repete`,
  `modelo repetiu a mesma chamada nesta fala`, `frase ambígua para a chamada; não aprende`.
- Dedup: `model_calls` (fala corrente, janela 8 s, folga 2 s ao virar a fala) e
  `recently_done` (reflexo→modelo). Aprendizado: só com `utterances_since_turn == 1`.
- Perfil do dono no config: `assistant` (libera app.*/web.*). `business_mentor` não libera.
- Briefings anteriores: docs/superpowers/briefs/2026-09-18-autonomia-jarvis.md (achados
  1–8 e arquitetura do observador), 2026-09-18-janela-do-jarvis.md.
