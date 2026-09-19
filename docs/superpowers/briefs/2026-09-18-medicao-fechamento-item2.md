# Medição de fechamento — item 2 (JRV-106)

Commit medido: `4140b7c46984491365483dae18e8dce2f7bb936e` (main com JRV-95).
Rodada: 19/09/2026, 02:16:54–02:18:27 UTC (18/09 à noite no horário local).
Doze falas, entrada por texto e microfone mudo. CLI release saiu 0; a régua saiu 1.

A build foi isolada em `/tmp/jrv5882-autonomia`, usando `target-5834`, sem a alteração
alheia de `crates/core/src/reflex/mod.rs`. Foi usado `--device-in 'MacBook Pro Microphone'`
porque o dispositivo configurado estava desconectado. Nenhum login ou senha foi solicitado.

Trace sanitizado: `docs/testes/autonomia-jrv106.trace.txt`. Os endereços estão mascarados
com identificadores estáveis para não confundir URLs diferentes na contagem de duplicatas.
O tracing deste CLI sai em stdout, junto da apresentação: o coletor preservou as linhas
reais do trace, retirando apenas prefixos da apresentação. Redirecionar só stderr não coleta
esta rodada. O stdout sanitizado original permanece em `/tmp/jrv5882-medicao-item2.log.stdout`.

Para reproduzir a análise, da raiz do repositório:

```sh
python3 tools/jarvis_trace_report.py docs/testes/autonomia-jrv106.trace.txt
python3 tools/jarvis_trace_report.py docs/testes/autonomia-jrv106.trace.txt --aceitacao
```

## Comparação

| Medida | Rodada 1 | Rodada 6 | Após JRV-95 |
|---|---:|---:|---:|
| Duplicatas | 2 | 2 | 6 |
| Falhas de tool | 0 | 0 | 0 |
| Recusas | 1 | 0 | 0 |
| Segunda Globo pelo reflexo | não | não | não |
| Falas medidas | 12 | — | 12 |

Fontes anteriores: `2026-09-18-medicao-rodada1.md` e
`2026-09-18-medicao-rodadas-2-a-6.md`. As rodadas antigas tinham a espera incorreta do
runner; não se deve atribuir toda diferença numérica a uma regressão de execução.

## O que ainda impede o pronto

1. Seis grupos de ações repetidas pelo próprio modelo: Google no Safari (2×), Chrome
   (3×), YouTube no Chrome (5×), Calculadora (4×), primeira Globo (5×), segunda Globo
   (10×). Corresponde ao bug já registrado no JRV-91; não foi corrigido nesta medição.
2. A segunda Globo saiu pelo modelo. O Jev respondeu `Nothing` nas duas falas da Globo,
   embora a ação tenha sido aprendida. A falha persiste com os turnos agora separados.
3. As respostas de sucesso ainda repetem a ação; o item 5 do brief trata isso.
4. O relatório contabiliza `skipped_judge=true` como uma chamada ao Jev de 0 ms. O trace
   tem 11 consultas reais, 1 consulta pulada e 6 decisões Act; a pergunta de horas não
   consultou o Jev. São 5 turnos com consulta sem Act, somando 1808 ms, e não 6 consultas
   inúteis. O item 4 deve conferir também a precisão dessa contagem.

Safari, Google no Safari, Chrome, YouTube no Chrome, voltar, pesquisar e Calculadora
executaram; voltar e pesquisar saíram pelo reflexo. As duas ações de volume também
executaram. Não houve erro de ferramenta, recusa, tool pendente ou timeout do runner.
A régua completa **não passou**.

# Relatório do trace do Jarvis

## Resumo

- Turnos com fala do usuário: 12
- Tools executadas: 34 modelo=30, reflexo=4
- Falhas de tool: 0
- Duplicatas (mesma ação executada mais de uma vez no turno): 6
- Chamadas ao Jev: 12; Act: 6 (50%); latência média 369 ms, máx 757 ms
- Jev gasto sem ação: 6 de 12 turnos (1808 ms jogados fora)
  - "abre o ***#d4c9d902 no Safari" — 1 chamada(s), 396 ms, nenhuma ação
  - "abre o youtube no Chrome" — 1 chamada(s), 361 ms, nenhuma ação
  - "abre a Calculadora" — 1 chamada(s), 289 ms, nenhuma ação
  - "abre a globo" — 1 chamada(s), 366 ms, nenhuma ação
  - "abre a globo" — 1 chamada(s), 396 ms, nenhuma ação
  - "que horas são?" — 1 chamada(s), 0 ms, nenhuma ação
- Transcrição→primeira tool: média 687 ms, máx 984 ms (a transcrição chega junto com a chamada; o fim da fala real não está no trace)
- Respostas 'não é possível/não consigo': 0

## Turnos

### 1. 02:16:54.234  abre o Safari
- tool `app.open: Safari` origem=modelo 43 ms ok
- reflexo Act app.open (Jev 566 ms)
- aprendeu: app.open ← "abre o Safari"
- modelo: "O Safari foi aberto com sucesso."

### 2. 02:17:00.988  abre o ***#d4c9d902 no Safari
- tool `web.open: browser=Safari, url=***#05046f26` origem=modelo 45 ms ok
- tool `web.open: browser=Safari, url=***#05046f26` origem=modelo 43 ms ok
- reflexo Nothing (Jev 396 ms)
- **DUPLICATA**: `web.open` {"browser":"Safari","opened":"***#05046f26"} executada 2x (modelo sozinho)
- aprendeu: web.open ← "abre o ***#d4c9d902 no Safari"
- modelo: "O Google já foi aberto no Safari."

### 3. 02:17:08.438  abre o Chrome
- tool `app.open: Google Chrome` origem=modelo 43 ms ok
- tool `app.open: Google Chrome` origem=modelo 38 ms ok
- tool `app.open: Google Chrome` origem=modelo 38 ms ok
- reflexo Act app.open (Jev 757 ms)
- **DUPLICATA**: `app.open` {"opened":"Google Chrome"} executada 3x (modelo sozinho)
- aprendeu: app.open ← "abre o Chrome"
- modelo: "O Google Chrome foi aberto."

### 4. 02:17:16.142  abre o youtube no Chrome
- tool `web.open: browser=Chrome, url=***#1467931a` origem=modelo 6710 ms ok
- tool `web.open: browser=Chrome, url=***#1467931a` origem=modelo 42 ms ok
- tool `web.open: browser=Chrome, url=***#4dc3a769` origem=modelo 49 ms ok
- tool `web.open: browser=Chrome, url=***#1467931a` origem=modelo 43 ms ok
- tool `web.open: browser=Chrome, url=***#1467931a` origem=modelo 43 ms ok
- reflexo Nothing (Jev 361 ms)
- **DUPLICATA**: `web.open` {"browser":"Google Chrome","opened":"***#1467931a"} executada 5x (modelo sozinho)
- aprendeu: web.open ← "abre o youtube no Chrome"
- modelo: "O YouTube foi aberto no Chrome."

### 5. 02:17:32.137  volta
- tool `browser.back` origem=reflexo 233 ms ok
- reflexo Act browser.back (Jev 349 ms)
- reflexo agiu browser.back (Jev 349 ms)
- dedup: Gemini pediu `browser.back` já feita pelo reflexo
- modelo: "A página anterior foi carregada.A página anterior foi carregada."

### 6. 02:17:39.405  pesquisa overclock
- tool `browser.search` origem=reflexo 219 ms ok
- reflexo Act browser.search (Jev 289 ms)
- reflexo agiu browser.search (Jev 289 ms)
- modelo: "A pesquisa por overclock foi realizada."

### 7. 02:17:44.486  abre a Calculadora
- tool `app.open: Calculator` origem=modelo 29 ms ok
- tool `app.open: Calculator` origem=modelo 42 ms ok
- tool `app.open: Calculator` origem=modelo 41 ms ok
- tool `app.open: Calculator` origem=modelo 41 ms ok
- reflexo Nothing (Jev 289 ms)
- **DUPLICATA**: `app.open` {"opened":"Calculator"} executada 4x (modelo sozinho)
- aprendeu: app.open ← "abre a Calculadora"
- modelo: "A Calculadora foi aberta."

### 8. 02:17:50.685  abre a globo
- tool `web.open: ***#f33bcabf` origem=modelo 39 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 47 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 42 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 42 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 47 ms ok
- reflexo Nothing (Jev 366 ms)
- **DUPLICATA**: `web.open` {"opened":"***#f33bcabf"} executada 5x (modelo sozinho)
- aprendeu: web.open ← "abre a globo"
- modelo: "O site da Globo foi aberto."

### 9. 02:17:59.586  abre a globo
- tool `web.open: ***#f33bcabf` origem=modelo 54 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 37 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 41 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 44 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 46 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 44 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 51 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 44 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 40 ms ok
- tool `web.open: ***#f33bcabf` origem=modelo 41 ms ok
- reflexo Nothing (Jev 396 ms)
- **DUPLICATA**: `web.open` {"opened":"***#f33bcabf"} executada 10x (modelo sozinho)
- aprendeu: web.open ← "abre a globo"
- modelo: "O site da Globo foi aberto novamente."

### 10. 02:18:09.135  diminui o som
- tool `sys.volume` origem=reflexo 287 ms ok
- reflexo Act sys.volume (Jev 341 ms)
- reflexo agiu sys.volume (Jev 341 ms)
- dedup: Gemini pediu `sys.volume` já feita pelo reflexo
- modelo: "O volume foi diminuído.O volume foi diminuído."

### 11. 02:18:15.320  coloca o volume em 30
- tool `sys.volume` origem=reflexo 127 ms ok
- reflexo Act sys.volume (Jev 326 ms)
- reflexo agiu sys.volume (Jev 326 ms)
- dedup: Gemini pediu `sys.volume` já feita pelo reflexo
- modelo: "O volume foi ajustado para trinta.O volume foi ajustado para trinta."

### 12. 02:18:22.780  que horas são?
- reflexo Nothing (Jev 0 ms)
- modelo: "São duas horas e dezoito minutos."

## Duplicatas

- `web.open` {"browser":"Safari","opened":"***#05046f26"} executada 2x (modelo sozinho)
- `app.open` {"opened":"Google Chrome"} executada 3x (modelo sozinho)
- `web.open` {"browser":"Google Chrome","opened":"***#1467931a"} executada 5x (modelo sozinho)
- `app.open` {"opened":"Calculator"} executada 4x (modelo sozinho)
- `web.open` {"opened":"***#f33bcabf"} executada 5x (modelo sozinho)
- `web.open` {"opened":"***#f33bcabf"} executada 10x (modelo sozinho)
