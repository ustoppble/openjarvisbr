# Medições — rodadas 2 a 6 (loop autônomo)

Continuação de `2026-09-18-medicao-rodada1.md`. Todas as rodadas foram feitas sem o dono
falar nada: `jarvis --script docs/testes/roteiro-autonomia.txt`, medidas com
`tools/jarvis_trace_report.py` e checadas com `--aceitacao` (exit 0 = roteiro passou).

## Placar por rodada

| | R1 | R2 | R3 | R4 | R5 | R6 |
|---|---|---|---|---|---|---|
| Recusas ("não consigo") | 1 | 2 | 0 | 0 | 0 | 0 |
| Duplicatas | 2 | 0 | 0 | 0 | 1 | 2 |
| Falhas de tool | 0 | 0 | 1 | 0 | 0 | 0 |
| Jev: Act | 36% | 36% | 75% | 75% | — | — |
| Jev gasto sem ação | — | 7 turnos / 2423 ms | 1 / 328 ms | 1 / 0 ms | — | — |
| Quem age (modelo × reflexo) | 10×4 | — | 5×6 | 8×3 | — | — |
| Item 6 (fala repetida via reflexo) | falha | falha | falha | falha | — | falha |

As duplicatas oscilam entre 0 e 2 sem mudança de código: dependem de o Gemini repetir a
chamada e de o Jev responder a tempo. Rodada só é confiável com `REFLEXO MUDO: 0`.

## O que cada rodada ensinou

- **R2** — a régua mentia: o relatório imprimiu "recusas: 0" com duas recusas dentro. O
  filtro comparava `"não consigo"` em texto sensível a maiúscula e o modelo escreve
  `"Não consigo"`. Corrigido em `924a484` (JRV-94). **Sem isso o loop teria parado aqui,
  declarando sucesso com o "navegar" quebrado.**
- **R3** — `browser.back` (463 ms) e `browser.search` (293 ms) funcionaram: as duas recusas
  que travavam a missão morreram. Sobrou uma falha: `browser.search` exigia navegador em
  FOCO, e Safari/Chrome estavam abertos atrás da Calculadora.
- **R4** — falhas a zero (o foco virou cascata), mas o reflexo sumiu do 4º turno em diante:
  oito `reflexo pulou esta rodada err=TypeSafe não respondeu a tempo`. O resumo não dizia
  nada disso — parecia só "o modelo agiu mais". Métrica `REFLEXO MUDO` criada em `61b1806`;
  o comportamento virou JRV-104.
- **R5 e R6** — com a fala julgada agora visível no log, ficou claro que o texto chega
  limpo (nada de fragmentos grudados) e que o `hear()` não descarta. Mas das 12 falas do
  roteiro só 8 chegam ao Jev.

## Causa do item 6, confirmada

`crates/core/src/reflex/mod.rs:213-224`:

```rust
let Some(questions) = build_questions(&situation) else {
    if !heard.trim().is_empty() && !pending_confirm && !turn_locked {
        record_decision(&stats, &Decision::Nothing, ..., true, &heard);
    }
    return;   // turno travado → sai sem log e sem contar
};
```

Com `turn_locked` verdadeiro — o reflexo já agiu neste turno e `locked` só cai em
`end_turn()` — a fala morre ali: sem decisão, sem log, sem incrementar `total`.

Prova: rodando **só** as duas "abre a globo" isoladas, com 3 s entre elas e o turno
fechando no meio, as duas saem por `origem=reflexo` (411 ms e 354 ms) e o item 6 passa. No
roteiro completo, com falas a 1,5 s e o turno ainda travado, a segunda é engolida e quem
age é o Gemini. Card: JRV-105.

Junto vai uma pergunta de produto: **turno travado deve engolir um pedido novo do dono?**
Quem fala duas coisas seguidas não espera que a segunda seja ignorada. A trava talvez deva
cair quando chega fala nova, não só quando o modelo termina de falar.

## O padrão da tarde

Os quatro bugs mais difíceis de hoje eram o mesmo bug: **código que falha sem deixar
rastro.** A régua cega a maiúscula, o reflexo sumindo por timeout, a fala descartada no
`hear`, a decisão morta pelo turno travado. Em cada caso, consertar a observabilidade
primeiro fez o bug real aparecer em minutos — e em dois casos o diagnóstico anterior
(regra de decisão, depois timeout do Jev) estava errado justamente por falta de rastro.
