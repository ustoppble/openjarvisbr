# OpenJarvisBR — mensuração de custo (debriefing)

Período: 16/09/2026 15:52 → 17/09/2026 14:32 (≈ 9h de trabalho no dia 16 + ≈ 1h30 no dia 17)
Fonte A: transcripts de todas as 33 sessões Claude abertas na pasta do projeto (orquestrador + panes).
Fonte B: custo por card registrado no OverClick (só cards cujo worker mediu tokens).
Fonte C: 2 sessões Codex (gpt-6-astra).

## Volume
| Modelo | Turnos | Saída (tokens) | Cache lido (tokens) |
|---|---|---|---|
| Sonnet 5 | 2.354 | 1,43 M | 383 M |
| Fable 5.1 (orquestrador) | 964 | 0,87 M | 425 M |
| Opus 5 | 867 | 0,87 M | 93 M |
| Haiku 4.5 | 683 | 0,41 M | 68 M |
| Codex gpt-6-astra | — | 0,13 M | 14 M (entrada quase toda em cache) |

## Custo
| Visão | Valor | Observação |
|---|---|---|
| Equivalente em preço de lista da API (Claude) | ≈ US$ 1.215 | Fable 5.1 ≈ 785 (dominado por leitura de cache do contexto longo do orquestrador), Opus 5 ≈ 263, Sonnet 5 ≈ 156, Haiku ≈ 11. Premissas por MTok: Opus/Fable 15/75, cache 1,5/18,75; Sonnet 3/15, cache 0,3/3,75; Haiku 1/5, cache 0,1/1,25 |
| Registrado no OverClick (37 cards com medição) | ≈ US$ 208 | Tarifação do próprio OverClick; cards entregues pelo orquestrador ficaram com uso estimado |
| Codex (preço de lista aproximado) | ≈ US$ 5–10 | 2 sessões, 14,8 M de entrada (95% em cache) |
| **Desembolso real** | **assinaturas** | Contas Claude (OAuth) e ChatGPT: custo marginal ≈ 0 além dos planos; o que se consome é cota das janelas de 5h/semana |

## Produção
- 70 cards (JRV-1 a JRV-70), 3 missões (degraus 1, 2 e 3), ~70 panes abertos.
- Entregas: CLI de voz, app Tauri Mac/Windows, overlay 3D, perfis, ferramentas locais e de sistema, MCP Overclock/OverClick, CI Windows, releases v0.2.0 e v0.2.1, código privado + repo público de downloads.
- Retrabalho relevante: cards duplicados por um pane worker (JRV-37/38/44/45), 4 iterações de bug de áudio até achar a causa real (parser descartando 28% dos chunks), builds de demonstração refeitos por conflito de porta/pasta compartilhada.

## Lições
1. Orquestrador com contexto longo custa mais que os workers somados: a leitura de cache do Fable representou ~65% do preço de lista. Sessões de orquestração longas pedem compactação ou troca de pane por degrau.
2. Fronteira de arquivos por card + contratos na spec permitiu 6 Opus em paralelo com um só conflito trivial.
3. Nunca deixar o humano falar direto com um pane worker: vira orquestrador paralelo e duplica o quadro.
4. Demonstração para o dono deve rodar de worktree própria e com porta própria, nunca da árvore compartilhada.
