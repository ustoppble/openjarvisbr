# OpenJarvisBR v4 — Reflexo: decisões em ~300 ms com o Jev (fase 1)

Data: 2026-09-18 · Status: rascunho para revisão do dono · Execução: máximo paralelismo

## Objetivo

O Jarvis ganha um **reflexo**: uma camada na frente do Gemini Live que decide,
a cada fragmento de fala do usuário, se aquilo é uma ação segura e fechada e a
executa antes de o Gemini terminar de ouvir. O Gemini continua sendo o cérebro:
conversa, planeja, monta argumentos e propõe tools. O reflexo só escolhe entre
opções que já estão listadas e só dispara o que hoje já não pede confirmação.

Fase 1 cobre: abrir app, abrir site conhecido, controlar mídia, volume, e
confirmação por voz (sim / não / sempre pode). Clicar em elementos da tela fica
para a fase 2, quando o "olho" de acessibilidade existir.

Modelo: **Jev** (`jev-latest`) da TypeSafe.ai, `POST https://api.typesafe.ai/v1/systemone`.
Ele recebe um `state` e perguntas tipadas (`choice`, `score`, `noul`) e devolve
probabilidades. Não gera texto, não vê tela, não extrai valores livres.

## Regra de ouro

**Gemini pensa, Jev julga.** O reflexo nunca executa nada que hoje pede
confirmação, nunca inventa alvo fora da lista, e nunca age com confiança abaixo
do limiar. Em dúvida, fica quieto e deixa o fluxo atual seguir. O Jarvis sem
chave da TypeSafe funciona exatamente como hoje.

## Arquitetura

```
fala → Gemini Live transcreve (fragmentos) ──────────────────────► Gemini decide (como hoje)
                     │                                                     ▲
                     ▼                                                     │ "já feito" (dedup)
              Reflexo (novo)                                               │
   ┌──────────────────────────────────┐                                    │
   │ Olho: inventário em memória      │  apps instalados, apps rodando,    │
   │       (atualiza em background)   │  sites conhecidos, tools seguras   │
   ├──────────────────────────────────┤                                    │
   │ Juiz: monta perguntas ao Jev,    │  state = fala acumulada do turno   │
   │       aplica limiares            │  + candidatos filtrados do Olho    │
   ├──────────────────────────────────┤                                    │
   │ Mão: executa a tool segura já    │  reaproveita Registry/Tool de v3   │
   │      registrada, marca "feito"   ├────────────────────────────────────┘
   └──────────────────────────────────┘
```

Tudo mora em `crates/core/src/reflex/`. O engine só ganha um ponto de chamada
no evento `ServerEvent::UserText` e uma checagem de dedup em `on_tool_call`.

## Contratos (todos os cards constroem contra isto)

```rust
// crates/core/src/reflex/mod.rs
pub struct ReflexConfig {
    pub enabled: bool,                  // padrão: true se houver chave
    pub api_key: Option<String>,        // de config.toml `typesafe_api_key` ou env TYPESAFE_API_KEY
    pub model: String,                  // "jev-latest"
    pub act_threshold: f32,             // 0.85: executa sozinho
    pub confirm_threshold: f32,         // 0.85: aprova/nega pendente por voz
    pub debounce_ms: u64,               // 120: espera fragmento seguinte antes de perguntar
    pub sites: Vec<Site>,               // [[reflex.sites]] name, url
}

// crates/core/src/reflex/judge.rs — cliente HTTP + trait para testes
#[async_trait] pub trait Judge: Send + Sync {
    async fn ask(&self, state: &str, questions: &Questions) -> Result<Answers, JudgeError>;
}
pub struct JevClient { .. }             // reqwest, Authorization: Bearer <key>, timeout 800 ms,
                                        // 429/529 = desiste desta rodada (sem retry: o próximo fragmento pergunta de novo)
pub struct FakeJudge { .. }             // respostas programadas, para engine e decide tests

// crates/core/src/reflex/questions.rs — tipos do protocolo TypeSafe
pub enum Question { Choice { instructions, criteria: BTreeMap<String,String> },
                    Score  { instructions, criteria: Vec<String> },
                    Noul   { instructions, criteria: Option<(String,String)> } }
pub struct Questions(BTreeMap<String, Question>);
pub enum Answer { Choice { choice: String, probabilities: BTreeMap<String,f32>, confidence: f32 },
                  Score  { score: f32, confidence: f32 },
                  Noul   { noul: f32 } }
pub struct Answers { pub answers: BTreeMap<String, Answer>, pub usage: Usage }

// crates/core/src/reflex/eye.rs — inventário, sem rede
pub struct Inventory { pub installed_apps: Vec<AppEntry>, pub running_apps: Vec<AppEntry>,
                       pub sites: Vec<Site>, pub refreshed_at: Instant }
pub struct AppEntry { pub name: String, pub bundle_id: Option<String>, pub running: bool }
pub struct Eye { .. }                   // Eye::start(config) -> EyeHandle; snapshot() é O(1), clone barato
impl EyeHandle { pub fn snapshot(&self) -> Arc<Inventory>; pub fn refresh_now(&self); }
// fontes macOS: /Applications, ~/Applications, /System/Applications (instalados);
// `lsappinfo list` ou osascript System Events (rodando). Windows/Linux: só instalados, best-effort.
// refresh: instalados a cada 5 min; rodando a cada 3 s; nunca bloqueia o snapshot.

// crates/core/src/reflex/decide.rs — puro, sem I/O: fácil de testar
pub enum Intent { OpenApp, OpenSite, Media, Volume, None }
pub struct Situation<'a> { pub heard: &'a str, pub inventory: &'a Inventory,
                           pub pending_confirm: bool, pub turn_locked: bool }
pub fn build_questions(s: &Situation) -> Option<Questions>;   // None = nada a perguntar (ex.: fala vazia)
pub fn decide(s: &Situation, a: &Answers, cfg: &ReflexConfig) -> Decision;
pub enum Decision {
    Act(ToolCall),                      // tool segura + args prontos
    Approve, Deny, AlwaysAllow,         // confirmação por voz
    Nothing,
}
// Prefiltro de candidatos (para o Choice não explodir): apps rodando sempre entram;
// instalados entram se algum token da fala (≥3 letras, sem acento) for prefixo do nome;
// teto 40 opções + "nenhum". Sites: todos os configurados (teto 40).

// crates/core/src/reflex/mod.rs — orquestra
pub struct Reflex { .. }
impl Reflex {
    pub fn new(cfg: ReflexConfig, judge: Arc<dyn Judge>, eye: EyeHandle) -> Self;
    /// Chamado a cada fragmento do usuário. Debounce interno; cancela pergunta em voo
    /// se chegar fragmento novo. Devolve pela `tx` no máximo UMA Decision::Act por turno.
    pub fn hear(&self, heard: String, pending_confirm: bool);
    pub fn end_turn(&self);             // destrava para o próximo turno
    pub fn decisions(&self) -> mpsc::Receiver<Decision>;
}
```

```rust
// crates/core/src/engine.rs — integração (card E)
EngineEvent::ReflexActed { call: ToolCall, latency_ms: u32, confidence: f32 }
EngineEvent::ReflexConfirmed { approve: bool, confidence: f32 }
// ServerEvent::UserText → reflex.hear(user_heard.clone(), !confirms.is_empty())
// Decision::Act → executa via Registry (mesmo caminho de Safe), registra em `recently_done`
//   (nome+args normalizados, 8 s), envia ao Gemini texto de contexto:
//   "[sistema] já executado agora: abriu Safari" — para ele não repetir nem narrar como futuro.
// on_tool_call: se a chamada bater em `recently_done`, responde ao Gemini com o
//   resultado gravado sem executar de novo.
// Decision::Approve/Deny → resolve_confirm(id, approve, "voz, reflexo");
// Decision::AlwaysAllow → mesmo caminho de hear_always_allow.
// Listas fixas de decisions.rs continuam como caminho rápido e como fallback sem chave.
// TurnComplete → reflex.end_turn().
```

```toml
# config.toml
typesafe_api_key = "..."        # ou env TYPESAFE_API_KEY; nunca em log/evento/overlay
[reflex]
enabled = true
act_threshold = 0.85
confirm_threshold = 0.85
[[reflex.sites]]
name = "YouTube"
url = "https://youtube.com"
[[reflex.sites]]
name = "OverClick"
url = "https://cloud.overclock.sh"
```

## Perguntas ao Jev (fase 1)

Uma chamada por rodada, todas em paralelo do lado deles:

| id | tipo | instrução (resumo) | usa |
|---|---|---|---|
| `intent` | choice | O que o usuário pede em `heard`: open_app / open_site / media / volume / none | roteia |
| `app` | choice | Qual app da lista, ou `none` | só se intent=open_app |
| `site` | choice | Qual site da lista, ou `none` | só se intent=open_site |
| `media` | choice | play / pause / next / previous / none | só se intent=media |
| `volume` | choice | up / down / mute / unmute / set / none | set → número por regex local |
| `approve` | noul | O usuário está aprovando um pedido pendente | só com confirmação pendente |
| `deny` | noul | O usuário está recusando um pedido pendente | idem |
| `always` | noul | O usuário pede para nunca mais perguntar isso | sempre |

Regras de decisão: `Act` só se `intent.confidence ≥ act_threshold` **e** a
pergunta do alvo tem `choice ≠ none` com probabilidade ≥ act_threshold.
`Approve` se `approve ≥ confirm_threshold` e `deny < 0.5`; `Deny` simétrico.
Empate ou dúvida = `Nothing`. `always` acima do limiar vira `AlwaysAllow`
independente do resto.

## Latência (orçamento por fragmento)

| etapa | alvo |
|---|---|
| snapshot do Olho | < 1 ms (Arc clone) |
| montar perguntas + prefiltro | < 2 ms |
| Jev ida e volta | ~300 ms (timeout 800) |
| executar tool segura | 20 a 150 ms (`open -a`, osascript) |
| **total até a ação** | **< 500 ms após o fragmento decisivo** |

Fragmentos chegam a cada ~200 a 400 ms. Debounce de 120 ms evita perguntar em
fragmento de meia palavra. Uma pergunta em voo é cancelada quando chega
fragmento novo, salvo se já passou de 200 ms (aí deixa terminar). Uma vez que
o turno agiu, o reflexo trava até `TurnComplete`.

## Cards (onda 1 em paralelo; onda 2 integra; onda 3 valida)

| Card | Escopo (arquivos exclusivos) | Depende |
|---|---|---|
| A Cliente Jev + tipos do protocolo + FakeJudge + config | `crates/core/src/reflex/judge.rs`, `questions.rs`, `config.rs` (campos novos) | — |
| B Olho: inventário de apps e sites, refresh em background | `crates/core/src/reflex/eye.rs`, `examples/eye_dump.rs` | — |
| C Juiz puro: `build_questions`, prefiltro, `decide`, limiares, testes de mesa | `crates/core/src/reflex/decide.rs` | contrato A (tipos) |
| D Orquestrador `Reflex`: debounce, cancelamento, trava por turno | `crates/core/src/reflex/mod.rs` | contratos A, B, C |
| E Engine: hook em `UserText`, executa `Act`, dedup `recently_done`, contexto ao Gemini, confirmação por voz via reflexo, eventos novos | `crates/core/src/engine.rs`, `engine_tools.rs`, `tests/` | A, B, C, D |
| F CLI: `jarvis reflex "abre o spotify"` imprime perguntas, respostas, decisão e latência; `jarvis reflex --eye` lista o inventário | `crates/cli/src/main.rs` (subcomando novo) | A, B, C |
| G Desktop: overlay mostra flash "⚡ reflexo · Safari · 310 ms"; Settings ganha aba Reflexo (liga/desliga, limiar, sites, campo da chave mascarado) | `apps/desktop/src/overlay/tools.ts`, `settings/*`, `src-tauri` (comandos de config) | contrato de eventos E |
| H Roteiro de testes v4 + README + build | `docs/testes/v4.md`, `README.md` | E, F, G |

Onda 1: A, B, C, F começam juntos (C usa os tipos do contrato, não espera A
compilar). Onda 2: D e E. G começa na onda 1 pela parte visual e fecha na onda 2
quando os eventos existirem. Onda 3: H.

## Erros e degradação

- Sem chave, `enabled = false`, ou falha de rede: reflexo desliga silencioso,
  Jarvis segue como hoje. Aviso único no log, nunca por voz.
- 401: log "chave da TypeSafe inválida" e desliga até reiniciar.
- 429/529: pula a rodada. Três seguidas = pausa de 30 s.
- Timeout (800 ms): pula a rodada. Não há retry dentro do mesmo fragmento.
- Tool falhou após `Act`: emite `ToolResult` com erro como hoje e **não** grava
  em `recently_done`, para o Gemini poder tentar.
- Ação executada e depois o Gemini propõe a mesma: dedup responde sucesso sem
  repetir. Ação diferente: segue normal.

## Segurança e privacidade

- A transcrição do turno e nomes de apps/sites saem para a TypeSafe. Isso vai
  escrito no README e na aba Reflexo. Sem chave, nada sai.
- Chave só por config/env; nunca em log, evento, overlay, `--record` ou commit.
- Reflexo só chama tools em `decisions::NEVER_ASK` (`app.open`, `web.open`,
  `sys.volume`, `media.control`). Lista é fixa no código, não configurável.
- `web.open` só recebe URLs de `[[reflex.sites]]`, nunca URL montada da fala.

## Testes

- `decide.rs`: tabela de casos fala → perguntas esperadas → respostas fake →
  decisão. Cobre limiar exato, `none`, empate approve/deny, fala vazia, mais de
  40 candidatos.
- `mod.rs`: debounce, cancelamento, uma ação por turno, destrava em `end_turn`.
- `engine`: com `FakeJudge` e backend fake, "abre o safari" executa antes do
  Gemini e a chamada posterior do Gemini é deduplicada; "sim" resolve pendente.
- `judge.rs`: parse da resposta real (fixture JSON da doc), 401/429/timeout.
- Live opcional (`tests/jev_live.rs`, ignorado sem `TYPESAFE_API_KEY`): uma
  chamada real mede latência e valida o formato.

## Pronto quando

Com chave configurada: "Jarvis, abre o Spotify" abre em menos de meio segundo,
antes de o Gemini responder, e o Gemini não abre de novo nem fala como se fosse
abrir. "Próxima música" pula. "Aumenta o volume" aumenta. Com um `shell.run`
pendente, "pode ir" aprova e "deixa pra lá" nega, sem estar nas listas fixas.
Sem chave, tudo funciona como na v3. `jarvis reflex "abre o safari"` mostra
probabilidades e latência.

## Fora da fase 1

Clique em elementos (olho de acessibilidade), digitar texto, cálculos, qualquer
ação de risco `Confirm`, roteamento de custo Gemini vs local, janelas e abas.

## Adendo 2026-09-18 — Memória de ações (fase 1.5, pedido do dono: "aprender tudo")

Substitui a lista manual de sites por aprendizado geral. O reflexo passa a
lembrar **toda ação que o Gemini executou com sucesso** e a oferecê-la ao Jev
como opção na próxima fala parecida.

**Captura.** Em `on_tool_finished`, quando a chamada veio do modelo (id sem
prefixo `reflex-`) e terminou sem erro, o engine grava
`LearnedAction { phrase, tool, args, count, last_used }` onde `phrase` é
`user_heard` no momento em que a chamada chegou (guardar em `PendingLearn` por
id ao receber o `ToolCall`). Mesma (tool, args) já existente: incrementa `count`
e atualiza `phrase` se a nova for mais curta. Teto 200 ações; ao passar,
descarta a de menor `count` mais antiga.

**Persistência.** Arquivo próprio `~/.config/jarvis/reflex_memory.toml`
(`[[actions]]`), nunca o config.toml. Args são gravados como JSON string. Ações
com args que contenham chaves sensíveis (`token`, `key`, `password`, `secret`,
`authorization`) **não são aprendidas**. Tools `fs.write` e `shell.run` são
aprendidas, mas ver regra de risco abaixo.

**Olho.** `Inventory.learned: Vec<LearnedAction>`; `EyeHandle::learn(action)`
atualiza o snapshot na hora e `Eye::start` carrega o arquivo no boot.

**Juiz.** Nova pergunta `learned` (choice) só quando há candidatas: opções são
as ações aprendidas cujo `phrase` normalizado compartilha ≥1 token (≥3 letras)
com a fala, top 20 por `count`, mais `none`. Texto da opção:
`"{tool}: {call_summary}"` com a frase original entre aspas. Chave da opção:
índice estável (`l0`, `l1`, …) mapeado de volta pelo juiz. `intent` ganha a
opção `learned` ("repetir uma ação que o assistente já fez antes para o
usuário"). Decisão: `intent = learned` confiante **e** `learned ≠ none`
confiante → `Decision::Act(call)` com os args gravados e id `reflex-N`.

**Risco (regra que não muda).** No engine, `on_reflex` deixa de exigir
`is_reflex_tool`. Passa a valer: tool existe no registry **e** política diz
`Safe` para ela (`policy.risk == Safe` ou `decisions.allows(name)`). Se for
`Confirm`, o engine **não executa**: emite `ToolConfirmNeeded` na hora com a
chamada aprendida (mesmo caminho de hoje, só que sem esperar o Gemini) e o
resto segue igual: "sim" por voz ou botão, "sempre pode" libera. Tools
`mcp.*` de escrita seguem `Confirm`.

**Dedup.** Igual a hoje: ação feita pelo reflexo entra em `recently_done`; o
Gemini que chamar a mesma recebe sucesso sem repetir.

**Sites.** `[[reflex.sites]]` continua aceito, mas a aba deixa de ser
formulário: vira lista somente leitura "O que o reflexo já aprendeu" com
botão "esquecer" por linha e "esquecer tudo". A pergunta `site` do juiz
continua existindo para quem tiver lista manual.

**Testes.** memória: aprende, incrementa, teto, ignora args sensíveis, carrega
e grava. juiz: candidatas por token, `intent = learned` com alvo → `Act` com os
args gravados; `none` → `Nothing`. engine: Gemini executa `web.open` de
"abre a globo" → fica aprendido; próxima fala "abre a globo" com juiz fake
escolhendo `l0` → executa sem o Gemini e deduplica; ação `Confirm` aprendida →
`ToolConfirmNeeded` imediato, sem executar.

**Fora.** Aprender a partir de ações negadas ou com erro; sincronizar memória
entre máquinas; UI de edição de args.
