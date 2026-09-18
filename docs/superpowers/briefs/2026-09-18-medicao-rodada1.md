# Medição — rodada 1 (primeira rodada autônoma)

Primeira vez que o roteiro de aceitação rodou **sem o dono falar nada**.

- **Como**: worktree limpo do HEAD `534b2be` (`git worktree add --detach /tmp/jrv-loop HEAD`),
  build `cargo build --release --bin jarvis` com `CARGO_TARGET_DIR=.../target-jrv56`,
  `jarvis --script docs/testes/roteiro-autonomia.txt` (mic mudo, entrada por texto — JRV-81).
- **Quando**: 2026-09-18 19:51:28–19:52:08 UTC. Exit 0, 12 turnos.
- **Trace**: `docs/testes/rodada1-trace.log` (cópia de `/tmp/jrv-rodada1.log`).
- **Relatório**: `python3 tools/jarvis_trace_report.py docs/testes/rodada1-trace.log`.
- O app da bandeja foi parado durante a rodada (dois Jarvis no mesmo microfone se ouvem
  e se respondem) e devolvido ao ar depois; o volume foi restaurado a 100.

## Placar contra a régua (duplicatas=0, falhas=0, recusas=0)

**2 / 0 / 1** — não passa.

| # | Comando | Resultado |
|---|---------|-----------|
| 1 | abre o Safari | OK — uma execução só, `origem=reflexo`, dedup do JRV-83 pegou a chamada do modelo |
| 2 | abre o google.com no Safari | Abriu no Safari certo, mas `web.open` executou **2×** (modelo sozinho) |
| 3 | abre o Chrome | OK — `origem=reflexo` |
| 4 | abre o youtube no Chrome | Certo, mas `web.open` executou **2×** (modelo sozinho) |
| 5 | volta | **Recusa**: "Não consigo navegar pelas páginas" |
| 6 | pesquisa overclock | Abriu busca em aba nova em vez de pesquisar na aba atual |
| 7 | abre a Calculadora | OK — o modelo resolveu "Calculadora" → `app.open: Calculator` sozinho |
| 8 | abre a globo (2×) | A segunda saiu com `origem=modelo`; o roteiro pede `origem=reflexo` |
| 9 | diminui o som / volume 30 | OK — `origem=reflexo`, dedup pegou a chamada do modelo |
| 10 | que horas são? | OK — reflexo `Nothing`, não agiu |

## Números

- Turnos: 12 · Tools: 14 (modelo 10, reflexo 4) · Falhas: 0
- Duplicatas: 2 — as duas `web.open` chamada duas vezes **pelo próprio modelo** no mesmo turno
- Jev: 11 chamadas, 4 `Act` (36%), média 370 ms, máx 455 ms → 7 chamadas sem ação
- Transcrição → primeira tool: média 737 ms, máx 1404 ms

## Comparação com a medição 1 (voz, 19:37)

| | Medição 1 (voz) | Rodada 1 (texto, autônoma) |
|---|---|---|
| Turnos | 7 | 12 |
| Duplicatas | 2 | 2 |
| Falhas | 0 | 0 |
| Jev `Act` | 1/8 (12%) | 4/11 (36%) |
| Recusas | 2 | 1 |

## O que esta rodada descobriu sobre a própria régua

Dois defeitos na medição, registrados como cards antes de qualquer conclusão:

- **JRV-94** — o relatório contou `recusas: 0` com uma recusa dentro. A frase do modelo veio
  partida em dois chunks ("Não consigo navegar" + "pelas páginas") e o detector só casa
  frase inteira numa linha. Com isso, o roteiro "passaria" com recusa dentro.
- **JRV-95** — o `--script` manda a próxima linha ~1,5 s depois da anterior sem esperar o
  turno fechar, e as tools escorrem para o turno seguinte: a `web.open` da busca do turno 6
  aparece dentro do turno 7 ("abre a Calculadora").

Enquanto esses dois não fecharem, o placar acima é uma aproximação, não um veredito.

## O que a rodada mandou para o board

- **JRV-92** (duplicata modelo→modelo) — causa direta das 2 duplicatas.
- **JRV-96** (implementar `browser.*`) — causa direta da recusa e do item 6.
- **JRV-87** (gate do Jev) — as 7 chamadas sem ação.
- **JRV-93** (`ui.*`) — da sessão de voz: "digita ali 4728" → "não consigo digitar na tela".
