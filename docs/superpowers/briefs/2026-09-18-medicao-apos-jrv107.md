# Medição após a atribuição persistente da fala — JRV-107

Base: `a939c2a`, mais a alteração de `crates/core/src/engine.rs` entregue neste mesmo commit.
Worktree isolado `/tmp/jrv5882-autonomia`; cache `target-5834`; CLI release.
Roteiro completo, microfone mudo, `--device-in 'MacBook Pro Microphone'`.
Rodada em 19/09/2026, 02:57:33–02:58:49 UTC. CLI: exit 0.

## Resultado

| Medida | JRV-106 | JRV-107 |
|---|---:|---:|
| Falas | 12 | 12 |
| Ferramentas executadas | 34 | 11 |
| Grupos de duplicatas | 6 | 0 |
| Falhas de tool | 0 | 0 |
| Recusas | 0 | 0 |
| Segunda Globo pelo reflexo | não | não |

O guard agora continua ativo depois do TurnComplete intermediário da Live API.
A linha 31 do trace registra: `modelo repetiu a mesma chamada nesta fala; não executa de novo`.
As linhas 63, 89 e 115 registram outros descartes. A linha 163 prova que a pergunta de
horas pulou o Jev (`skipped_judge=true`, 0 ms).

A aceitação ainda sai 1: falta a segunda Globo pelo reflexo. O Jev decidiu Nothing nas
duas falas, mesmo com o aprendizado visível. O modelo também continua narrando a ação;
nesta sessão a regra de responder apenas Feito não foi observada. Na busca, houve trecho
de resposta verbal que chegou na fala seguinte, embora as ferramentas tenham ficado
nas falas corretas. Esses pontos permanecem para a rodada final; não são escondidos pelo
placar 0/0/0 de duplicatas/falhas/recusas.

Validação de código sobre o main integrado: 272 testes passaram; 9 testes optativos
foram ignorados pelo próprio projeto. Clippy com `-D warnings`, build release e
`git diff --check` passaram. Três novas regressões falharam antes da correção e passaram
depois: repetição entre turnos, preservação da frase atribuída por ModelText e
ambiguidade de duas falas ainda sem resposta. Os testes existentes de full_access,
transcrição junto da tool e repetição atrasada continuaram passando.

Trace sanitizado: `docs/testes/autonomia-jrv107.trace.txt`. Coleta e mascaramento como no
JRV-106; o stdout original sanitizado está em `/tmp/jrv5882-jrv107-medicao.log.stdout`.

```sh
python3 tools/jarvis_trace_report.py docs/testes/autonomia-jrv107.trace.txt
python3 tools/jarvis_trace_report.py docs/testes/autonomia-jrv107.trace.txt --aceitacao
```

O relatório ainda conta a consulta pulada como uma chamada de 0 ms; são 11 consultas
reais e uma consulta pulada, não 12 consultas reais.

# Relatório do trace do Jarvis

## Resumo

- Turnos com fala do usuário: 12
- Tools executadas: 11 modelo=7, reflexo=4
- Falhas de tool: 0
- Duplicatas (mesma ação executada mais de uma vez no turno): 0
- Chamadas ao Jev: 12; Act: 6 (50%); latência média 375 ms, máx 729 ms
- Jev gasto sem ação: 6 de 12 turnos (1821 ms jogados fora)
  - "abre o ***#d4c9d902 no Safari" — 1 chamada(s), 641 ms, nenhuma ação
  - "abre o youtube no Chrome" — 1 chamada(s), 291 ms, nenhuma ação
  - "abre a Calculadora" — 1 chamada(s), 300 ms, nenhuma ação
  - "abre a globo" — 1 chamada(s), 282 ms, nenhuma ação
  - "abre a globo" — 1 chamada(s), 307 ms, nenhuma ação
  - "que horas são?" — 1 chamada(s), 0 ms, nenhuma ação
- Transcrição→primeira tool: média 1557 ms, máx 10022 ms (a transcrição chega junto com a chamada; o fim da fala real não está no trace)
- Respostas 'não é possível/não consigo': 0

## Turnos

### 1. 02:57:33.469  abre o Safari
- tool `app.open: Safari` origem=modelo 53 ms ok
- reflexo Act app.open (Jev 592 ms)
- aprendeu: app.open ← "abre o Safari"
- modelo: "O Safari foi aberto com sucesso."

### 2. 02:57:39.924  abre o ***#d4c9d902 no Safari
- tool `browser.goto: browser=Safari, url=***#05046f26` origem=modelo 202 ms ok
- reflexo Nothing (Jev 641 ms)
- aprendeu: browser.goto ← "abre o ***#d4c9d902 no Safari"
- modelo: "O Google já está aberto no Safari."

### 3. 02:57:47.376  abre o Chrome
- tool `app.open` origem=reflexo 9574 ms ok
- reflexo Act app.open (Jev 325 ms)
- reflexo agiu app.open (Jev 325 ms)
- dedup: Gemini pediu `app.open` já feita pelo reflexo
- modelo: "O Google Chrome foi aberto.O Google Chrome foi aberto."

### 4. 02:58:00.302  abre o youtube no Chrome
- tool `browser.goto: browser=Chrome, url=***#1467931a` origem=modelo 157 ms ok
- reflexo Nothing (Jev 291 ms)
- aprendeu: browser.goto ← "abre o youtube no Chrome"
- modelo: "O YouTube foi aberto no Chrome."

### 5. 02:58:07.422  volta
- tool `browser.back` origem=reflexo 321 ms ok
- reflexo Act browser.back (Jev 393 ms)
- reflexo agiu browser.back (Jev 393 ms)
- modelo: "A página anterior foi acessada."

### 6. 02:58:12.122  pesquisa overclock
- tool `browser.search: overclock` origem=modelo 291 ms ok
- reflexo Act browser.search (Jev 729 ms)
- aprendeu: browser.search ← "pesquisa overclock"

### 7. 02:58:14.707  abre a Calculadora
- tool `app.open: Calculator` origem=modelo 42 ms ok
- reflexo Nothing (Jev 300 ms)
- aprendeu: app.open ← "abre a Calculadora"
- modelo: "A pesquisa por A calculadora está aberta."

### 8. 02:58:19.020  abre a globo
- tool `web.open: ***#f33bcabf` origem=modelo 50 ms ok
- reflexo Nothing (Jev 282 ms)
- aprendeu: web.open ← "abre a globo"
- modelo: "A Globo está aberta no navegador."

### 9. 02:58:26.522  abre a globo
- tool `web.open: ***#f33bcabf` origem=modelo 44 ms ok
- reflexo Nothing (Jev 307 ms)
- aprendeu: web.open ← "abre a globo"
- modelo: "A Globo está aberta no navegador."

### 10. 02:58:31.884  diminui o som
- tool `sys.volume` origem=reflexo 330 ms ok
- reflexo Act sys.volume (Jev 301 ms)
- reflexo agiu sys.volume (Jev 301 ms)
- modelo: "O volume foi diminuído."

### 11. 02:58:35.919  coloca o volume em 30
- tool `sys.volume` origem=reflexo 178 ms ok
- reflexo Act sys.volume (Jev 340 ms)
- reflexo agiu sys.volume (Jev 340 ms)
- dedup: Gemini pediu `sys.volume` já feita pelo reflexo
- modelo: "O volume foi definido para 30.O volume foi definido para 30."

### 12. 02:58:43.842  que horas são?
- reflexo Nothing (Jev 0 ms)
- modelo: "São duas horas e cinquenta e oito minutos."
