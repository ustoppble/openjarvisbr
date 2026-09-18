# OpenJarvisBR v4 — Reflexo com Jev: plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** O Jarvis executa ações seguras (abrir app, abrir site conhecido, mídia, volume) e resolve confirmações por voz em menos de 500 ms, perguntando ao Jev (TypeSafe.ai) a cada fragmento de transcrição, sem tirar o Gemini Live do papel de cérebro.

**Architecture:** Módulo novo `crates/core/src/reflex/` com quatro arquivos de responsabilidade única: `questions.rs` (tipos do protocolo TypeSafe), `judge.rs` (cliente HTTP + fake), `eye.rs` (inventário de apps e sites em memória) e `decide.rs` (função pura fala + inventário + respostas → decisão). `mod.rs` orquestra debounce, cancelamento e uma ação por turno. O engine chama `reflex.hear()` em cada `ServerEvent::UserText`, executa `Decision::Act` pelo mesmo caminho de tool `Safe`, deduplica a chamada igual que o Gemini fizer depois, e avisa o Gemini por texto que a ação já foi feita.

**Tech Stack:** Rust 2021, tokio, reqwest (já no workspace), serde/serde_json, async-trait. Desktop: Tauri 2 + TypeScript (Vite). Spec: `docs/superpowers/specs/2026-09-18-openjarvisbr-v4-reflexo-jev-design.md`.

## Global Constraints

- Regra de ouro: **Gemini pensa, Jev julga.** O reflexo só executa tools de `decisions::NEVER_ASK` restritas a `app.open`, `web.open`, `sys.volume`, `media.control`. A lista é constante no código.
- Sem chave (`typesafe_api_key` no config ou env `TYPESAFE_API_KEY`), com `[reflex].enabled = false` ou com falha de rede, o Jarvis se comporta exatamente como na v3.
- Limiares padrão: `act_threshold = 0.85`, `confirm_threshold = 0.85`. Debounce 120 ms. Timeout HTTP 800 ms. Sem retry no mesmo fragmento. Três 429/529 seguidos = pausa de 30 s.
- `web.open` pelo reflexo só recebe URLs de `[[reflex.sites]]`, nunca URL montada da fala.
- Uma `Decision::Act` por turno; destrava em `TurnComplete`.
- Dedup: ação feita pelo reflexo fica em `recently_done` por 8 s; chamada igual do Gemini nesse prazo recebe sucesso sem executar.
- Chave nunca em log, evento, overlay, `--record` ou commit. `Debug` manual mascara com `***`.
- Endpoint: `POST https://api.typesafe.ai/v1/systemone`, header `Authorization: Bearer <key>`, modelo `jev-latest`.
- Comentários e docs em português, como o resto do repo. Commits com prefixo `feat:`/`test:`/`docs:` e sufixo `[JRV]`.
- Comandos: `cargo test -p openjarvisbr-core reflex` para o módulo; `cargo test -p openjarvisbr-core` para tudo; `cargo clippy -p openjarvisbr-core -- -D warnings` antes de cada commit.

---

## Mapa de arquivos

| Arquivo | Responsabilidade | Card |
|---|---|---|
| `crates/core/src/reflex/questions.rs` | Tipos do protocolo TypeSafe: `Question`, `Questions`, `Answer`, `Answers`, `Usage`, (de)serialização exata do JSON | A |
| `crates/core/src/reflex/judge.rs` | Trait `Judge`, `JevClient` (reqwest), `FakeJudge`, `JudgeError` | A |
| `crates/core/src/config.rs` | Campos `typesafe_api_key`, seção `[reflex]`, `[[reflex.sites]]`, `load_reflex()` | A |
| `crates/core/src/reflex/eye.rs` | `Inventory`, `AppEntry`, `Site`, `Eye::start`, `EyeHandle::snapshot/refresh_now`, scanners macOS | B |
| `crates/core/examples/eye_dump.rs` | Imprime o inventário para inspeção manual | B |
| `crates/core/src/reflex/decide.rs` | `Intent`, `Situation`, `Decision`, `build_questions`, `decide`, prefiltro de candidatos | C |
| `crates/core/src/reflex/mod.rs` | `ReflexConfig`, `Reflex` (debounce, cancelamento, trava por turno), reexports | D |
| `crates/core/src/engine.rs` | Hook em `UserText`, `Decision` → ação/confirmação, `recently_done`, contexto ao Gemini, eventos `ReflexActed`/`ReflexConfirmed`, `TurnComplete` → `end_turn` | E |
| `crates/core/src/engine_tools.rs` | `REFLEX_DONE_WINDOW`, `reflex_context_text()` | E |
| `crates/cli/src/main.rs` | Subcomando `reflex` (`jarvis reflex "frase"`, `jarvis reflex --eye`) | F |
| `apps/desktop/src-tauri/src/tool_events.rs` | Emite `engine://reflex` | G |
| `apps/desktop/src/overlay/tools.ts` | Flash "⚡ reflexo · Safari · 310 ms" | G |
| `apps/desktop/settings.html`, `src/settings/main.ts`, `src-tauri/src/main.rs` | Aba Reflexo: liga/desliga, limiar, sites, chave mascarada | G |
| `docs/testes/v4.md`, `README.md` | Roteiro leigo e documentação | H |

## Ondas de execução (paralelismo máximo)

```
Onda 1 (4 panes em paralelo, sem dependência entre si):
  A  questions.rs + judge.rs + config       (Tasks 1–4)
  B  eye.rs + example                       (Tasks 5–7)
  C  decide.rs (usa os tipos do contrato)   (Tasks 8–11)
  F  CLI `jarvis reflex` (com FakeJudge)    (Task 15)
  G1 overlay flash + aba Reflexo (visual)   (Tasks 16–17)

Onda 2 (depois de A+B+C mesclados):
  D  mod.rs orquestrador                    (Tasks 12–13)
  E  engine                                 (Task 14)         ← depende de D
  G2 fiação Tauri engine://reflex + chave   (Task 18)         ← depende de E

Onda 3:
  H  docs/testes/v4.md + README             (Task 19)
```

Regra de convivência na onda 1: cada card só toca os arquivos da sua linha na
tabela. `crates/core/src/reflex/mod.rs` na onda 1 é **só** a declaração dos
submódulos (Task 0, feita pelo orquestrador antes de abrir os panes). C compila
contra os tipos de A? Não: C declara em teste os fixtures que precisa usando os
tipos de `questions.rs`. Se A ainda não mesclou, C trabalha num branch que
inclui o esqueleto de tipos da Task 1 (é pequeno e é contrato fechado).

---

### Task 0: Esqueleto do módulo (orquestrador, antes da onda 1)

**Files:**
- Create: `crates/core/src/reflex/mod.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Criar o módulo com os submódulos declarados e vazios**

```rust
// crates/core/src/reflex/mod.rs
//! Reflexo: decisões rápidas (~300 ms) com o Jev (TypeSafe.ai) na frente do
//! Gemini Live. Spec: docs/superpowers/specs/2026-09-18-openjarvisbr-v4-reflexo-jev-design.md
//!
//! Regra de ouro: Gemini pensa, Jev julga. O reflexo só executa ações que hoje
//! já não pedem confirmação e só escolhe entre opções listadas.

pub mod decide;
pub mod eye;
pub mod judge;
pub mod questions;
```

Crie os quatro arquivos vazios com um comentário de cabeçalho cada:

```bash
cd /Users/laschuk/Developer/bside/jarvis
for f in decide eye judge questions; do printf '//! (card em andamento)\n' > crates/core/src/reflex/$f.rs; done
```

- [ ] **Step 2: Exportar em lib.rs**

Em `crates/core/src/lib.rs`, adicionar `pub mod reflex;` depois de `pub mod profiles;`.

- [ ] **Step 3: Compilar e commitar**

Run: `cargo build -p openjarvisbr-core`
Expected: compila sem warnings novos.

```bash
git add crates/core/src/lib.rs crates/core/src/reflex
git commit -m "feat(reflex): esqueleto do módulo [JRV]"
```

---

## Card A — protocolo TypeSafe, cliente Jev, config

### Task 1: Tipos do protocolo (`questions.rs`)

**Files:**
- Create: `crates/core/src/reflex/questions.rs`

**Interfaces:**
- Produces: `Question`, `Questions`, `Answer`, `Answers`, `Usage`, `Questions::insert`, `Answers::choice(id) -> Option<(&str, f32)>`, `Answers::noul(id) -> Option<f32>`.

- [ ] **Step 1: Escrever o teste de serialização do pedido e parse da resposta**

```rust
// crates/core/src/reflex/questions.rs (no fim do arquivo)
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pedido_serializa_no_formato_da_typesafe() {
        let mut q = Questions::default();
        q.insert("intent", Question::choice("O que o usuário pede?", [
            ("open_app", "Abrir um aplicativo"),
            ("none", "Nenhuma ação"),
        ]));
        q.insert("approve", Question::noul("O usuário está aprovando um pedido pendente", None));
        q.insert("frustration", Question::score("Quão irritado", ["calmo", "irritado"]));
        let body = Request { state: "abre o safari".into(), model: "jev-latest".into(), questions: q };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["model"], "jev-latest");
        assert_eq!(v["state"], "abre o safari");
        assert_eq!(v["questions"]["intent"]["type"], "choice");
        assert_eq!(v["questions"]["intent"]["criteria"]["open_app"], "Abrir um aplicativo");
        assert_eq!(v["questions"]["approve"]["type"], "noul");
        assert!(v["questions"]["approve"].get("criteria").is_none());
        assert_eq!(v["questions"]["frustration"]["criteria"][1], "irritado");
    }

    #[test]
    fn resposta_da_doc_faz_parse() {
        let raw = json!({
            "model": "jev-latest",
            "answers": {
                "intent": { "type": "choice", "choice": "open_app",
                            "probabilities": { "open_app": 0.93, "none": 0.07 }, "confidence": 0.93 },
                "approve": { "type": "noul", "noul": 0.12 },
                "frustration": { "type": "score", "score": 1.035,
                                 "legend": {"0": "calmo", "1": "irritado"},
                                 "probabilities": {"0": 0.2, "1": 0.8}, "confidence": 0.8 }
            },
            "usage": { "input_tokens": 312, "output_tokens": 48 }
        });
        let a: Answers = serde_json::from_value(raw).unwrap();
        assert_eq!(a.choice("intent"), Some(("open_app", 0.93)));
        assert_eq!(a.noul("approve"), Some(0.12));
        assert_eq!(a.choice("approve"), None);
        assert_eq!(a.usage.input_tokens, 312);
        assert!(matches!(a.answers.get("frustration"), Some(Answer::Score { score, .. }) if (*score - 1.035).abs() < 1e-6));
    }

    #[test]
    fn probabilidade_de_uma_opcao() {
        let raw = json!({ "model": "jev-latest", "answers": { "app": { "type": "choice",
            "choice": "Safari", "probabilities": {"Safari": 0.6, "Spotify": 0.4}, "confidence": 0.6 } },
            "usage": { "input_tokens": 1, "output_tokens": 1 } });
        let a: Answers = serde_json::from_value(raw).unwrap();
        assert_eq!(a.probability("app", "Spotify"), Some(0.4));
        assert_eq!(a.probability("app", "Zoom"), None);
    }
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core reflex::questions`
Expected: erro de compilação (tipos não existem).

- [ ] **Step 3: Implementar os tipos**

```rust
// crates/core/src/reflex/questions.rs
//! Tipos do protocolo TypeSafe (`POST /v1/systemone`): perguntas tipadas
//! (choice, score, noul) e respostas com probabilidades.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Uma pergunta ao Jev. `criteria` muda de forma conforme o tipo.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    Choice {
        instructions: String,
        criteria: BTreeMap<String, String>,
    },
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
    Noul {
        instructions: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
}

/// O que "sim" e "não" significam num `noul`, quando vale explicar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoulCriteria {
    #[serde(rename = "true")]
    pub yes: String,
    #[serde(rename = "false")]
    pub no: String,
}

impl Question {
    pub fn choice<I, K, V>(instructions: &str, criteria: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self::Choice {
            instructions: instructions.to_string(),
            criteria: criteria.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
        }
    }

    pub fn score<I, S>(instructions: &str, levels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Score {
            instructions: instructions.to_string(),
            criteria: levels.into_iter().map(Into::into).collect(),
        }
    }

    pub fn noul(instructions: &str, criteria: Option<NoulCriteria>) -> Self {
        Self::Noul {
            instructions: instructions.to_string(),
            criteria,
        }
    }
}

/// Conjunto de perguntas de uma chamada, por id.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Questions(pub BTreeMap<String, Question>);

impl Questions {
    pub fn insert(&mut self, id: &str, question: Question) {
        self.0.insert(id.to_string(), question);
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// Corpo do pedido.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub state: String,
    pub model: String,
    pub questions: Questions,
}

/// Uma resposta do Jev.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Choice {
        choice: String,
        #[serde(default)]
        probabilities: BTreeMap<String, f32>,
        #[serde(default)]
        confidence: f32,
    },
    Score {
        score: f32,
        #[serde(default)]
        probabilities: BTreeMap<String, f32>,
        #[serde(default)]
        confidence: f32,
    },
    Noul {
        noul: f32,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Answers {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub answers: BTreeMap<String, Answer>,
    #[serde(default)]
    pub usage: Usage,
}

impl Answers {
    /// Opção escolhida e a confiança, se a resposta `id` for um `choice`.
    pub fn choice(&self, id: &str) -> Option<(&str, f32)> {
        match self.answers.get(id)? {
            Answer::Choice { choice, confidence, .. } => Some((choice.as_str(), *confidence)),
            _ => None,
        }
    }

    /// Probabilidade de uma opção específica num `choice`.
    pub fn probability(&self, id: &str, option: &str) -> Option<f32> {
        match self.answers.get(id)? {
            Answer::Choice { probabilities, .. } => probabilities.get(option).copied(),
            _ => None,
        }
    }

    pub fn noul(&self, id: &str) -> Option<f32> {
        match self.answers.get(id)? {
            Answer::Noul { noul } => Some(*noul),
            _ => None,
        }
    }
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p openjarvisbr-core reflex::questions`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/reflex/questions.rs
git commit -m "feat(reflex): tipos do protocolo TypeSafe [JRV]"
```

### Task 2: Trait `Judge`, `FakeJudge` e `JudgeError`

**Files:**
- Create: `crates/core/src/reflex/judge.rs`

**Interfaces:**
- Produces: `trait Judge { async fn ask(&self, state: &str, questions: &Questions) -> Result<Answers, JudgeError> }`, `FakeJudge::new()`, `FakeJudge::push(Answers)`, `FakeJudge::push_err(JudgeError)`, `FakeJudge::calls() -> Vec<(String, Questions)>`, `JudgeError::{Unauthorized, RateLimited, Overloaded, Timeout, Http(String), Parse(String), Disabled}`.

- [ ] **Step 1: Teste do fake (respostas na ordem, registro das chamadas)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflex::questions::{Answer, Question};

    fn answers_with_noul(id: &str, v: f32) -> Answers {
        let mut a = Answers::default();
        a.answers.insert(id.into(), Answer::Noul { noul: v });
        a
    }

    #[tokio::test]
    async fn fake_devolve_na_ordem_e_registra_chamadas() {
        let fake = FakeJudge::new();
        fake.push(answers_with_noul("approve", 0.9));
        fake.push_err(JudgeError::Timeout);
        let mut q = Questions::default();
        q.insert("approve", Question::noul("aprova?", None));

        let a = fake.ask("sim pode", &q).await.unwrap();
        assert_eq!(a.noul("approve"), Some(0.9));
        assert!(matches!(fake.ask("hã", &q).await, Err(JudgeError::Timeout)));
        // esgotou: erro Disabled, nunca pânico
        assert!(matches!(fake.ask("x", &q).await, Err(JudgeError::Disabled)));

        let calls = fake.calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0, "sim pode");
        assert_eq!(calls[0].1.len(), 1);
    }
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core reflex::judge`
Expected: erro de compilação.

- [ ] **Step 3: Implementar trait, erro e fake**

```rust
// crates/core/src/reflex/judge.rs
//! Quem responde às perguntas: o Jev de verdade (HTTP) ou um fake de teste.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use super::questions::{Answers, Questions, Request};

#[derive(Debug, Error, Clone, PartialEq)]
pub enum JudgeError {
    #[error("chave da TypeSafe inválida (401)")]
    Unauthorized,
    #[error("limite de chamadas da TypeSafe (429)")]
    RateLimited,
    #[error("TypeSafe sobrecarregada (529)")]
    Overloaded,
    #[error("TypeSafe não respondeu a tempo")]
    Timeout,
    #[error("erro HTTP da TypeSafe: {0}")]
    Http(String),
    #[error("resposta da TypeSafe inválida: {0}")]
    Parse(String),
    #[error("reflexo desligado")]
    Disabled,
}

#[async_trait]
pub trait Judge: Send + Sync {
    async fn ask(&self, state: &str, questions: &Questions) -> Result<Answers, JudgeError>;
}

/// Juiz de teste: devolve respostas programadas na ordem e guarda o que foi
/// perguntado.
#[derive(Default)]
pub struct FakeJudge {
    queue: Mutex<VecDeque<Result<Answers, JudgeError>>>,
    calls: Mutex<Vec<(String, Questions)>>,
    /// Atraso artificial por chamada (testes de debounce/cancelamento).
    pub delay: Duration,
}

impl FakeJudge {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_delay(delay: Duration) -> Self {
        Self { delay, ..Self::default() }
    }
    pub fn push(&self, answers: Answers) {
        self.queue.lock().unwrap().push_back(Ok(answers));
    }
    pub fn push_err(&self, err: JudgeError) {
        self.queue.lock().unwrap().push_back(Err(err));
    }
    pub fn calls(&self) -> Vec<(String, Questions)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Judge for FakeJudge {
    async fn ask(&self, state: &str, questions: &Questions) -> Result<Answers, JudgeError> {
        self.calls.lock().unwrap().push((state.to_string(), questions.clone()));
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.queue.lock().unwrap().pop_front().unwrap_or(Err(JudgeError::Disabled))
    }
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p openjarvisbr-core reflex::judge`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/reflex/judge.rs
git commit -m "feat(reflex): trait Judge e FakeJudge [JRV]"
```

### Task 3: `JevClient` (HTTP real)

**Files:**
- Modify: `crates/core/src/reflex/judge.rs`
- Create: `crates/core/tests/fixtures/jev_response.json`
- Create: `crates/core/tests/jev_live.rs`

**Interfaces:**
- Produces: `JevClient::new(api_key: String, model: String) -> Result<JevClient, JudgeError>`, `JevClient::with_base_url(self, url: &str) -> Self` (para testes), `pub const JEV_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone"`, `pub const JEV_TIMEOUT: Duration = Duration::from_millis(800)`.

- [ ] **Step 1: Teste de mapeamento de status e de parse, sem rede**

O crate não tem servidor HTTP fake nas deps e não vamos adicionar um. Testamos a
função pura `classify(status, body) -> Result<Answers, JudgeError>` e deixamos a
rede para o teste live ignorado.

```rust
// dentro de mod tests em judge.rs
#[test]
fn classifica_status_http() {
    let ok = include_str!("../../tests/fixtures/jev_response.json");
    let a = classify(200, ok).unwrap();
    assert_eq!(a.choice("intent"), Some(("open_app", 0.93)));
    assert!(matches!(classify(401, "{}"), Err(JudgeError::Unauthorized)));
    assert!(matches!(classify(429, "{}"), Err(JudgeError::RateLimited)));
    assert!(matches!(classify(529, "{}"), Err(JudgeError::Overloaded)));
    assert!(matches!(classify(500, "boom"), Err(JudgeError::Http(m)) if m.contains("500")));
    assert!(matches!(classify(200, "não é json"), Err(JudgeError::Parse(_))));
}

#[test]
fn debug_do_cliente_nao_vaza_chave() {
    let c = JevClient::new("segredo-123".into(), "jev-latest".into()).unwrap();
    let s = format!("{c:?}");
    assert!(!s.contains("segredo-123"));
    assert!(s.contains("***"));
}
```

Fixture `crates/core/tests/fixtures/jev_response.json`:

```json
{
  "model": "jev-latest",
  "answers": {
    "intent": { "type": "choice", "choice": "open_app",
                "probabilities": { "open_app": 0.93, "open_site": 0.03, "media": 0.02, "volume": 0.01, "none": 0.01 },
                "confidence": 0.93 },
    "app": { "type": "choice", "choice": "Safari",
             "probabilities": { "Safari": 0.97, "Spotify": 0.02, "none": 0.01 }, "confidence": 0.97 },
    "approve": { "type": "noul", "noul": 0.04 },
    "deny": { "type": "noul", "noul": 0.02 },
    "always": { "type": "noul", "noul": 0.01 }
  },
  "usage": { "input_tokens": 240, "output_tokens": 30 }
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core reflex::judge`
Expected: erro de compilação (`classify`, `JevClient`).

- [ ] **Step 3: Implementar o cliente**

Adicionar em `judge.rs`:

```rust
pub const JEV_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
pub const JEV_TIMEOUT: Duration = Duration::from_millis(800);

/// Cliente HTTP do Jev. `Debug` manual: nunca imprime a chave.
pub struct JevClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
    url: String,
}

impl std::fmt::Debug for JevClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevClient")
            .field("api_key", &"***")
            .field("model", &self.model)
            .field("url", &self.url)
            .finish()
    }
}

impl JevClient {
    pub fn new(api_key: String, model: String) -> Result<Self, JudgeError> {
        if api_key.trim().is_empty() {
            return Err(JudgeError::Disabled);
        }
        let http = reqwest::Client::builder()
            .timeout(JEV_TIMEOUT)
            .use_rustls_tls()
            .build()
            .map_err(|e| JudgeError::Http(e.to_string()))?;
        Ok(Self { http, api_key, model, url: JEV_ENDPOINT.to_string() })
    }

    pub fn with_base_url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }
}

/// Mapeia status + corpo para resultado. Puro, testável sem rede.
pub(crate) fn classify(status: u16, body: &str) -> Result<Answers, JudgeError> {
    match status {
        200 => serde_json::from_str(body).map_err(|e| JudgeError::Parse(e.to_string())),
        401 => Err(JudgeError::Unauthorized),
        429 => Err(JudgeError::RateLimited),
        529 => Err(JudgeError::Overloaded),
        other => Err(JudgeError::Http(format!("status {other}"))),
    }
}

#[async_trait]
impl Judge for JevClient {
    async fn ask(&self, state: &str, questions: &Questions) -> Result<Answers, JudgeError> {
        let body = Request {
            state: state.to_string(),
            model: self.model.clone(),
            questions: questions.clone(),
        };
        let response = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| if e.is_timeout() { JudgeError::Timeout } else { JudgeError::Http(e.to_string()) })?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|e| JudgeError::Http(e.to_string()))?;
        classify(status, &text)
    }
}
```

Verifique se `reqwest` no workspace tem a feature `json` (o `Cargo.toml` da raiz
lista `rustls-no-provider` e `http2`). Se `.json(&body)` não compilar, adicione
`"json"` às features de `reqwest` em `/Users/laschuk/Developer/bside/jarvis/Cargo.toml`
(`[workspace.dependencies] reqwest = { ..., features = [..., "json"] }`). Se
`use_rustls_tls()` não existir nessa versão, remova a linha: o MCP client em
`crates/core/src/mcp/transport.rs` já constrói um `reqwest::Client`; copie o
mesmo builder de lá.

- [ ] **Step 4: Teste live, ignorado sem chave**

```rust
// crates/core/tests/jev_live.rs
//! Uma chamada real ao Jev. Roda só com TYPESAFE_API_KEY:
//! `TYPESAFE_API_KEY=... cargo test -p openjarvisbr-core --test jev_live -- --ignored --nocapture`
use openjarvisbr_core::reflex::judge::{JevClient, Judge};
use openjarvisbr_core::reflex::questions::{Question, Questions};

#[tokio::test]
#[ignore]
async fn jev_responde_em_menos_de_um_segundo() {
    let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
    let client = JevClient::new(key, "jev-latest".into()).unwrap();
    let mut q = Questions::default();
    q.insert("intent", Question::choice("O que o usuário pede em state?", [
        ("open_app", "Abrir um aplicativo instalado"),
        ("open_site", "Abrir um site"),
        ("none", "Nenhuma ação"),
    ]));
    q.insert("app", Question::choice("Qual aplicativo, se for abrir um?", [
        ("Safari", "navegador Safari"), ("Spotify", "player Spotify"), ("none", "nenhum destes"),
    ]));
    let t0 = std::time::Instant::now();
    let a = client.ask("abre o safari pra mim", &q).await.expect("resposta");
    let ms = t0.elapsed().as_millis();
    eprintln!("latência {ms} ms · usage {:?}", a.usage);
    assert_eq!(a.choice("intent").map(|c| c.0), Some("open_app"));
    assert_eq!(a.choice("app").map(|c| c.0), Some("Safari"));
    assert!(ms < 1000, "latência {ms} ms");
}
```

- [ ] **Step 5: Rodar unitários e clippy**

Run: `cargo test -p openjarvisbr-core reflex::judge && cargo clippy -p openjarvisbr-core -- -D warnings`
Expected: 3 passed, clippy limpo.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/reflex/judge.rs crates/core/tests/fixtures/jev_response.json crates/core/tests/jev_live.rs Cargo.toml Cargo.lock
git commit -m "feat(reflex): cliente HTTP do Jev [JRV]"
```

### Task 4: Config `[reflex]` e chave

**Files:**
- Modify: `crates/core/src/config.rs`

**Interfaces:**
- Produces: `pub struct ReflexSettings { pub enabled: bool, pub api_key: Option<String>, pub model: String, pub act_threshold: f32, pub confirm_threshold: f32, pub debounce_ms: u64, pub sites: Vec<SiteConfig> }`, `pub struct SiteConfig { pub name: String, pub url: String }`, `pub const ENV_TYPESAFE_KEY: &str = "TYPESAFE_API_KEY"`, `pub fn load_reflex() -> ReflexSettings`, `pub fn save_reflex_enabled(on: bool)`, `pub fn save_typesafe_api_key(key: &str)`. `Settings` ganha campo `reflex: ReflexSettings`.

- [ ] **Step 1: Testes de parse**

Adicionar no `mod tests` existente de `config.rs`:

```rust
#[test]
fn reflex_le_secao_e_sites() {
    let raw = r#"
typesafe_api_key = "ts-xyz"
[reflex]
enabled = true
act_threshold = 0.9
[[reflex.sites]]
name = "YouTube"
url = "https://youtube.com"
"#;
    let parsed: FileConfig = toml::from_str(raw).unwrap();
    let r = reflex_from(&parsed, None);
    assert!(r.enabled);
    assert_eq!(r.api_key.as_deref(), Some("ts-xyz"));
    assert_eq!(r.model, "jev-latest");
    assert!((r.act_threshold - 0.9).abs() < 1e-6);
    assert!((r.confirm_threshold - 0.85).abs() < 1e-6);
    assert_eq!(r.debounce_ms, 120);
    assert_eq!(r.sites.len(), 1);
    assert_eq!(r.sites[0].name, "YouTube");
}

#[test]
fn reflex_sem_chave_fica_desligado_e_env_vence_arquivo() {
    let parsed: FileConfig = toml::from_str("").unwrap();
    let r = reflex_from(&parsed, None);
    assert!(!r.enabled);
    assert!(r.api_key.is_none());
    let parsed: FileConfig = toml::from_str("typesafe_api_key = \"arquivo\"").unwrap();
    let r = reflex_from(&parsed, Some("env".to_string()));
    assert_eq!(r.api_key.as_deref(), Some("env"));
    assert!(r.enabled);
}

#[test]
fn reflex_enabled_false_vence_chave() {
    let parsed: FileConfig = toml::from_str("typesafe_api_key = \"x\"\n[reflex]\nenabled = false").unwrap();
    assert!(!reflex_from(&parsed, None).enabled);
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core config::tests::reflex`
Expected: erro de compilação.

- [ ] **Step 3: Implementar**

Em `FileConfig` adicionar:

```rust
    /// Chave da TypeSafe (Jev). Também aceita a env `TYPESAFE_API_KEY`.
    typesafe_api_key: Option<String>,
    /// Seção `[reflex]`.
    reflex: Option<ReflexSection>,
```

Novos tipos, perto de `ToolsSection`:

```rust
/// `[reflex]` no config.toml.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ReflexSection {
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub act_threshold: Option<f32>,
    pub confirm_threshold: Option<f32>,
    pub debounce_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<SiteConfig>,
}

/// Um site que o reflexo pode abrir por voz (`[[reflex.sites]]`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SiteConfig {
    pub name: String,
    pub url: String,
}

/// Configuração resolvida do reflexo. `Debug` manual: nunca imprime a chave.
#[derive(Clone)]
pub struct ReflexSettings {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub model: String,
    pub act_threshold: f32,
    pub confirm_threshold: f32,
    pub debounce_ms: u64,
    pub sites: Vec<SiteConfig>,
}

impl std::fmt::Debug for ReflexSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReflexSettings")
            .field("enabled", &self.enabled)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("model", &self.model)
            .field("act_threshold", &self.act_threshold)
            .field("confirm_threshold", &self.confirm_threshold)
            .field("debounce_ms", &self.debounce_ms)
            .field("sites", &self.sites)
            .finish()
    }
}

impl Default for ReflexSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            model: "jev-latest".into(),
            act_threshold: 0.85,
            confirm_threshold: 0.85,
            debounce_ms: 120,
            sites: Vec::new(),
        }
    }
}

pub const ENV_TYPESAFE_KEY: &str = "TYPESAFE_API_KEY";

/// Resolve `[reflex]` + chave. `env_key` é o valor da env já lido (injetável
/// nos testes). Sem chave = desligado; `enabled = false` vence a chave.
fn reflex_from(parsed: &FileConfig, env_key: Option<String>) -> ReflexSettings {
    let d = ReflexSettings::default();
    let api_key = env_key
        .filter(|k| !k.is_empty())
        .or_else(|| parsed.typesafe_api_key.clone().filter(|k| !k.is_empty()));
    let s = parsed.reflex.clone().unwrap_or_default();
    ReflexSettings {
        enabled: api_key.is_some() && s.enabled.unwrap_or(true),
        api_key,
        model: s.model.filter(|m| !m.trim().is_empty()).unwrap_or(d.model),
        act_threshold: s.act_threshold.unwrap_or(d.act_threshold).clamp(0.5, 1.0),
        confirm_threshold: s.confirm_threshold.unwrap_or(d.confirm_threshold).clamp(0.5, 1.0),
        debounce_ms: s.debounce_ms.unwrap_or(d.debounce_ms),
        sites: s.sites,
    }
}

/// Lê a configuração do reflexo do config.toml e da env.
pub fn load_reflex() -> ReflexSettings {
    let env_key = std::env::var(ENV_TYPESAFE_KEY).ok();
    let Some(path) = config_path() else {
        return reflex_from(&FileConfig::default(), env_key);
    };
    reflex_from(&read_file_config(&path), env_key)
}

/// Grava `[reflex].enabled`.
pub fn save_reflex_enabled(on: bool) -> Result<(), ConfigError> {
    let path = config_path().ok_or(ConfigError::MissingKey)?;
    let mut cfg = read_file_config(&path);
    cfg.reflex.get_or_insert_with(Default::default).enabled = Some(on);
    write_file_config(&path, &cfg)
}

/// Grava `typesafe_api_key`. Vazio remove.
pub fn save_typesafe_api_key(key: &str) -> Result<(), ConfigError> {
    let path = config_path().ok_or(ConfigError::MissingKey)?;
    let mut cfg = read_file_config(&path);
    cfg.typesafe_api_key = Some(key.trim().to_string()).filter(|k| !k.is_empty());
    write_file_config(&path, &cfg)
}
```

Adicionar `pub reflex: ReflexSettings` em `Settings` e preencher em
`load_settings()` com `reflex: reflex_from(&parsed, std::env::var(ENV_TYPESAFE_KEY).ok())`
(atenção: `parsed` é movido nos campos; leia `reflex` **antes** de mover `parsed.tools`).
`Settings` deriva `Default`; `ReflexSettings` implementa `Default`, ok.

Sobre `write_file_config`: veja como `save_full_access` (linha ~449) escreve o
arquivo (`FileConfigOut` + `toml::to_string_pretty`). Se não existir um helper
genérico, extraia um `fn write_file_config(path: &PathBuf, cfg: &FileConfig) -> Result<(), ConfigError>`
a partir do que `save_full_access` faz e passe `save_full_access` a usá-lo. `FileConfigOut`
precisa ganhar os dois campos novos (`typesafe_api_key`, `reflex`) para não
apagá-los ao regravar.

- [ ] **Step 4: Rodar tudo do config**

Run: `cargo test -p openjarvisbr-core config`
Expected: todos passam, inclusive os antigos de `save_*`.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/config.rs
git commit -m "feat(reflex): config [reflex], sites e chave da TypeSafe [JRV]"
```

---

## Card B — Olho: inventário de apps e sites

### Task 5: Tipos do inventário e prefiltro de apps instalados

**Files:**
- Create: `crates/core/src/reflex/eye.rs`

**Interfaces:**
- Produces: `pub struct AppEntry { pub name: String, pub bundle_id: Option<String>, pub running: bool }`, `pub struct Inventory { pub installed_apps: Vec<AppEntry>, pub running_apps: Vec<AppEntry>, pub sites: Vec<SiteConfig>, pub refreshed_at: Instant }`, `pub fn app_name_from_path(path: &Path) -> Option<String>`, `pub fn scan_app_dirs(dirs: &[PathBuf]) -> Vec<AppEntry>`.

- [ ] **Step 1: Testes com diretórios temporários**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nome_do_app_vem_do_bundle_sem_extensao() {
        assert_eq!(app_name_from_path(Path::new("/Applications/Visual Studio Code.app")).as_deref(), Some("Visual Studio Code"));
        assert_eq!(app_name_from_path(Path::new("/Applications/README.txt")), None);
    }

    #[test]
    fn varre_pastas_ignora_nao_app_e_ordena_sem_repetir() {
        let dir = std::env::temp_dir().join(format!("eye-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for name in ["Zed.app", "Safari.app", "notas.txt", "Safari.app"] {
            let p = dir.join(name);
            if name.ends_with(".app") { std::fs::create_dir_all(&p).unwrap(); } else { std::fs::create_dir_all(&dir).unwrap(); std::fs::write(&p, b"").unwrap(); }
        }
        let other = dir.join("sub");
        std::fs::create_dir_all(other.join("Safari.app")).unwrap();
        let apps = scan_app_dirs(&[dir.clone(), other]);
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Safari", "Zed"]);
        assert!(apps.iter().all(|a| !a.running));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inventario_marca_rodando() {
        let mut inv = Inventory::new(vec![AppEntry::installed("Safari"), AppEntry::installed("Zed")], vec![]);
        inv.mark_running(&["Zed".to_string(), "Finder".to_string()]);
        assert!(inv.installed_apps.iter().find(|a| a.name == "Zed").unwrap().running);
        assert!(!inv.installed_apps.iter().find(|a| a.name == "Safari").unwrap().running);
        let running: Vec<&str> = inv.running_apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(running, vec!["Finder", "Zed"]);
    }
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core reflex::eye`
Expected: erro de compilação.

- [ ] **Step 3: Implementar**

```rust
// crates/core/src/reflex/eye.rs
//! Olho do reflexo: inventário em memória do que o usuário pode pedir por
//! voz (apps instalados, apps rodando, sites configurados). Nunca faz rede.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::config::SiteConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    pub bundle_id: Option<String>,
    pub running: bool,
}

impl AppEntry {
    pub fn installed(name: &str) -> Self {
        Self { name: name.to_string(), bundle_id: None, running: false }
    }
}

#[derive(Debug, Clone)]
pub struct Inventory {
    pub installed_apps: Vec<AppEntry>,
    pub running_apps: Vec<AppEntry>,
    pub sites: Vec<SiteConfig>,
    pub refreshed_at: Instant,
}

impl Inventory {
    pub fn new(installed_apps: Vec<AppEntry>, sites: Vec<SiteConfig>) -> Self {
        Self { installed_apps, running_apps: Vec::new(), sites, refreshed_at: Instant::now() }
    }

    /// Atualiza `running` nos instalados e reconstrói `running_apps` (inclui
    /// apps rodando que não estão nas pastas varridas, ex.: Finder).
    pub fn mark_running(&mut self, running: &[String]) {
        let set: BTreeSet<&str> = running.iter().map(String::as_str).collect();
        for app in &mut self.installed_apps {
            app.running = set.contains(app.name.as_str());
        }
        self.running_apps = set
            .iter()
            .map(|name| {
                self.installed_apps
                    .iter()
                    .find(|a| a.name == *name)
                    .cloned()
                    .unwrap_or_else(|| AppEntry { name: name.to_string(), bundle_id: None, running: true })
            })
            .collect();
        self.refreshed_at = Instant::now();
    }
}

/// "Visual Studio Code.app" → "Visual Studio Code"; qualquer outra coisa → None.
pub fn app_name_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(".app").map(str::to_string)
}

/// Varre as pastas (um nível) e devolve os `.app` sem repetição, ordenados.
pub fn scan_app_dirs(dirs: &[PathBuf]) -> Vec<AppEntry> {
    let mut names = BTreeSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            if let Some(name) = app_name_from_path(&entry.path()) {
                names.insert(name);
            }
        }
    }
    names.into_iter().map(|n| AppEntry::installed(&n)).collect()
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p openjarvisbr-core reflex::eye`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/reflex/eye.rs
git commit -m "feat(reflex): inventário de apps (tipos e varredura) [JRV]"
```

### Task 6: `Eye` com refresh em background e apps rodando (macOS)

**Files:**
- Modify: `crates/core/src/reflex/eye.rs`

**Interfaces:**
- Produces: `pub struct EyeHandle`, `Eye::start(sites: Vec<SiteConfig>) -> EyeHandle`, `Eye::start_with(dirs, sites, running_probe: Arc<dyn RunningProbe>, installed_every, running_every) -> EyeHandle`, `EyeHandle::snapshot() -> Arc<Inventory>`, `EyeHandle::refresh_now()`, `pub trait RunningProbe: Send + Sync { fn running_app_names(&self) -> Vec<String> }`, `pub struct OsRunningProbe`.

- [ ] **Step 1: Teste com probe fake e intervalos curtos**

```rust
#[cfg(test)]
mod handle_tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeProbe(Mutex<Vec<String>>);
    impl RunningProbe for FakeProbe {
        fn running_app_names(&self) -> Vec<String> { self.0.lock().unwrap().clone() }
    }

    #[tokio::test]
    async fn snapshot_e_barato_e_refresh_atualiza_rodando() {
        let probe = Arc::new(FakeProbe(Mutex::new(vec!["Finder".into()])));
        let eye = Eye::start_with(vec![], vec![], probe.clone(), Duration::from_secs(3600), Duration::from_millis(20));
        tokio::time::sleep(Duration::from_millis(60)).await;
        let a = eye.snapshot();
        assert_eq!(a.running_apps.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["Finder"]);
        *probe.0.lock().unwrap() = vec!["Finder".into(), "Safari".into()];
        eye.refresh_now();
        tokio::time::sleep(Duration::from_millis(60)).await;
        let b = eye.snapshot();
        assert_eq!(b.running_apps.len(), 2);
        // snapshot é Arc: clonar não copia o inventário
        assert!(Arc::ptr_eq(&eye.snapshot(), &eye.snapshot()));
    }
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core reflex::eye::handle_tests`
Expected: erro de compilação.

- [ ] **Step 3: Implementar**

```rust
/// Quem diz quais apps estão rodando. Trait para o teste não depender do SO.
pub trait RunningProbe: Send + Sync {
    fn running_app_names(&self) -> Vec<String>;
}

/// macOS: `osascript -e 'tell application "System Events" to get name of every process whose background only is false'`.
/// Outros SOs: lista vazia.
pub struct OsRunningProbe;

impl RunningProbe for OsRunningProbe {
    fn running_app_names(&self) -> Vec<String> {
        #[cfg(target_os = "macos")]
        {
            let out = std::process::Command::new("osascript")
                .arg("-e")
                .arg("tell application \"System Events\" to get name of every process whose background only is false")
                .output();
            match out {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .split(", ")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
                _ => Vec::new(),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Vec::new()
        }
    }
}

pub const INSTALLED_EVERY: Duration = Duration::from_secs(300);
pub const RUNNING_EVERY: Duration = Duration::from_secs(3);

/// Pastas padrão de apps por SO.
pub fn default_app_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Applications"));
        dirs.push(PathBuf::from("/System/Applications"));
        dirs.push(PathBuf::from("/System/Applications/Utilities"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join("Applications"));
        }
    }
    dirs
}

#[derive(Clone)]
pub struct EyeHandle {
    inner: Arc<RwLock<Arc<Inventory>>>,
    kick: tokio::sync::mpsc::UnboundedSender<()>,
}

impl EyeHandle {
    /// O(1): clona o `Arc` do inventário corrente.
    pub fn snapshot(&self) -> Arc<Inventory> {
        Arc::clone(&self.inner.read().unwrap_or_else(|e| e.into_inner()))
    }
    /// Pede um refresh dos rodando fora do intervalo.
    pub fn refresh_now(&self) {
        let _ = self.kick.send(());
    }
}

pub struct Eye;

impl Eye {
    pub fn start(sites: Vec<SiteConfig>) -> EyeHandle {
        Self::start_with(default_app_dirs(), sites, Arc::new(OsRunningProbe), INSTALLED_EVERY, RUNNING_EVERY)
    }

    pub fn start_with(
        dirs: Vec<PathBuf>,
        sites: Vec<SiteConfig>,
        probe: Arc<dyn RunningProbe>,
        installed_every: Duration,
        running_every: Duration,
    ) -> EyeHandle {
        let inventory = Arc::new(Inventory::new(scan_app_dirs(&dirs), sites.clone()));
        let inner = Arc::new(RwLock::new(inventory));
        let (kick, mut kicked) = tokio::sync::mpsc::unbounded_channel::<()>();
        let handle = EyeHandle { inner: Arc::clone(&inner), kick };
        tokio::spawn(async move {
            let mut last_installed = Instant::now();
            let mut tick = tokio::time::interval(running_every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    Some(()) = kicked.recv() => {}
                }
                let probe = Arc::clone(&probe);
                let running = tokio::task::spawn_blocking(move || probe.running_app_names())
                    .await
                    .unwrap_or_default();
                let mut next = (**inner.read().unwrap_or_else(|e| e.into_inner())).clone();
                if last_installed.elapsed() >= installed_every {
                    let dirs = dirs.clone();
                    next.installed_apps = tokio::task::spawn_blocking(move || scan_app_dirs(&dirs))
                        .await
                        .unwrap_or_default();
                    last_installed = Instant::now();
                }
                next.mark_running(&running);
                *inner.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(next);
            }
        });
        handle
    }
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p openjarvisbr-core reflex::eye`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/reflex/eye.rs
git commit -m "feat(reflex): Olho com refresh em background e apps rodando [JRV]"
```

### Task 7: Exemplo `eye_dump`

**Files:**
- Create: `crates/core/examples/eye_dump.rs`

- [ ] **Step 1: Escrever o exemplo**

```rust
//! Imprime o inventário do Olho: `cargo run -p openjarvisbr-core --example eye_dump`
use openjarvisbr_core::config;
use openjarvisbr_core::reflex::eye::Eye;

#[tokio::main]
async fn main() {
    let reflex = config::load_reflex();
    let eye = Eye::start(reflex.sites.clone());
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let inv = eye.snapshot();
    println!("instalados: {}", inv.installed_apps.len());
    println!("rodando ({}):", inv.running_apps.len());
    for app in &inv.running_apps {
        println!("  • {}", app.name);
    }
    println!("sites ({}):", inv.sites.len());
    for site in &inv.sites {
        println!("  • {} → {}", site.name, site.url);
    }
}
```

- [ ] **Step 2: Rodar**

Run: `cargo run -p openjarvisbr-core --example eye_dump`
Expected: lista com dezenas de instalados e os apps abertos agora (Finder incluso).

- [ ] **Step 3: Commit**

```bash
git add crates/core/examples/eye_dump.rs
git commit -m "feat(reflex): exemplo eye_dump [JRV]"
```

---

## Card C — Juiz puro (`decide.rs`)

### Task 8: Prefiltro de candidatos

**Files:**
- Create: `crates/core/src/reflex/decide.rs`

**Interfaces:**
- Produces: `pub const MAX_CANDIDATES: usize = 40`, `pub const NONE: &str = "none"`, `pub fn normalize(text: &str) -> String` (minúsculas, sem acento, só letras/dígitos/espaço), `pub fn app_candidates(heard: &str, inv: &Inventory) -> Vec<String>`.

- [ ] **Step 1: Testes**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SiteConfig;
    use crate::reflex::eye::{AppEntry, Inventory};

    fn inv(installed: &[&str], running: &[&str]) -> Inventory {
        let mut i = Inventory::new(installed.iter().map(|n| AppEntry::installed(n)).collect(), vec![
            SiteConfig { name: "YouTube".into(), url: "https://youtube.com".into() },
        ]);
        i.mark_running(&running.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        i
    }

    #[test]
    fn normaliza_acentos_e_pontuacao() {
        assert_eq!(normalize("Abre o Safári, por favor!"), "abre o safari por favor");
    }

    #[test]
    fn candidatos_rodando_sempre_entram_e_instalados_por_prefixo() {
        let i = inv(&["Safari", "Spotify", "Zed", "Slack"], &["Finder", "Zed"]);
        let c = app_candidates("abre o spo", &i);
        assert!(c.contains(&"Finder".to_string()));
        assert!(c.contains(&"Zed".to_string()));
        assert!(c.contains(&"Spotify".to_string()));
        assert!(!c.contains(&"Slack".to_string()));
        assert!(!c.contains(&"Safari".to_string()));
    }

    #[test]
    fn candidatos_respeitam_teto() {
        let names: Vec<String> = (0..100).map(|n| format!("App{n:03}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let i = inv(&refs, &[]);
        assert_eq!(app_candidates("abre o app", &i).len(), MAX_CANDIDATES);
    }

    #[test]
    fn tokens_curtos_nao_casam() {
        let i = inv(&["Zed", "Zoom"], &[]);
        assert!(app_candidates("ze", &i).is_empty());
        assert_eq!(app_candidates("zed", &i), vec!["Zed".to_string()]);
    }
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core reflex::decide`
Expected: erro de compilação.

- [ ] **Step 3: Implementar**

```rust
// crates/core/src/reflex/decide.rs
//! Juiz do reflexo, puro: fala + inventário → perguntas; respostas + limiares
//! → decisão. Sem I/O, para testar de mesa.

use std::collections::BTreeSet;

use serde_json::json;

use super::eye::Inventory;
use super::questions::{Answers, Question, Questions};
use crate::tools::ToolCall;

pub const MAX_CANDIDATES: usize = 40;
pub const NONE: &str = "none";

/// Minúsculas, sem acento, só letras/dígitos/espaço, espaços simples.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let mapped = match ch {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'a',
            'É' | 'È' | 'Ê' | 'Ë' => 'e',
            'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
            'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'o',
            'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
            'Ç' => 'c',
            c if c.is_alphanumeric() => c.to_ascii_lowercase(),
            _ => ' ',
        };
        out.push(mapped);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Apps rodando sempre entram; instalados entram se algum token da fala (≥3
/// letras) for prefixo de alguma palavra do nome. Teto `MAX_CANDIDATES`.
pub fn app_candidates(heard: &str, inv: &Inventory) -> Vec<String> {
    let tokens: Vec<String> = normalize(heard)
        .split(' ')
        .filter(|t| t.chars().count() >= 3)
        .map(str::to_string)
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for app in &inv.running_apps {
        if seen.insert(app.name.clone()) {
            out.push(app.name.clone());
        }
    }
    for app in &inv.installed_apps {
        if out.len() >= MAX_CANDIDATES {
            break;
        }
        if seen.contains(&app.name) {
            continue;
        }
        let name = normalize(&app.name);
        let hit = tokens.iter().any(|t| name.split(' ').any(|w| w.starts_with(t.as_str())));
        if hit && seen.insert(app.name.clone()) {
            out.push(app.name.clone());
        }
    }
    out.truncate(MAX_CANDIDATES);
    out
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p openjarvisbr-core reflex::decide`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/reflex/decide.rs
git commit -m "feat(reflex): prefiltro de candidatos [JRV]"
```

### Task 9: `build_questions`

**Files:**
- Modify: `crates/core/src/reflex/decide.rs`

**Interfaces:**
- Produces: `pub struct Situation<'a> { pub heard: &'a str, pub inventory: &'a Inventory, pub pending_confirm: bool, pub turn_locked: bool }`, `pub fn build_questions(s: &Situation) -> Option<Questions>`. Ids fixos: `intent`, `app`, `site`, `media`, `volume`, `approve`, `deny`, `always`.

- [ ] **Step 1: Testes**

```rust
#[test]
fn fala_vazia_nao_pergunta() {
    let i = inv(&["Safari"], &[]);
    let s = Situation { heard: "  ", inventory: &i, pending_confirm: false, turn_locked: false };
    assert!(build_questions(&s).is_none());
}

#[test]
fn turno_travado_so_pergunta_confirmacao() {
    let i = inv(&["Safari"], &[]);
    let s = Situation { heard: "sim pode", inventory: &i, pending_confirm: true, turn_locked: true };
    let q = build_questions(&s).unwrap();
    let ids: Vec<&String> = q.0.keys().collect();
    assert_eq!(ids, vec!["always", "approve", "deny"]);
}

#[test]
fn turno_travado_sem_pendente_nao_pergunta() {
    let i = inv(&["Safari"], &[]);
    let s = Situation { heard: "sim", inventory: &i, pending_confirm: false, turn_locked: true };
    assert!(build_questions(&s).is_none());
}

#[test]
fn perguntas_completas_incluem_candidatos_e_none() {
    let i = inv(&["Safari", "Spotify"], &["Finder"]);
    let s = Situation { heard: "abre o spotify", inventory: &i, pending_confirm: false, turn_locked: false };
    let q = build_questions(&s).unwrap();
    assert!(q.0.contains_key("intent"));
    assert!(q.0.contains_key("always"));
    assert!(!q.0.contains_key("approve"));
    match &q.0["app"] {
        Question::Choice { criteria, .. } => {
            assert!(criteria.contains_key("Spotify"));
            assert!(criteria.contains_key("Finder"));
            assert!(criteria.contains_key(NONE));
            assert!(!criteria.contains_key("Safari"));
        }
        _ => panic!("app deve ser choice"),
    }
    match &q.0["site"] {
        Question::Choice { criteria, .. } => assert!(criteria.contains_key("YouTube")),
        _ => panic!(),
    }
    match &q.0["intent"] {
        Question::Choice { criteria, .. } => {
            for k in ["open_app", "open_site", "media", "volume", NONE] { assert!(criteria.contains_key(k), "{k}"); }
        }
        _ => panic!(),
    }
}

#[test]
fn sem_candidato_de_app_nao_pergunta_app() {
    let i = inv(&["Safari"], &[]);
    let s = Situation { heard: "aumenta o volume", inventory: &i, pending_confirm: false, turn_locked: false };
    let q = build_questions(&s).unwrap();
    assert!(!q.0.contains_key("app"));
    assert!(q.0.contains_key("volume"));
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core reflex::decide`
Expected: erro de compilação.

- [ ] **Step 3: Implementar**

```rust
pub struct Situation<'a> {
    pub heard: &'a str,
    pub inventory: &'a Inventory,
    /// Há chamada `Confirm` esperando resposta.
    pub pending_confirm: bool,
    /// O reflexo já agiu neste turno: só confirmação interessa.
    pub turn_locked: bool,
}

const INTENT_CRITERIA: &[(&str, &str)] = &[
    ("open_app", "O usuário quer abrir ou ir para um aplicativo do computador."),
    ("open_site", "O usuário quer abrir um site ou página na internet."),
    ("media", "O usuário quer controlar a música ou o vídeo: tocar, pausar, próxima, anterior."),
    ("volume", "O usuário quer mudar o volume do computador ou silenciar."),
    (NONE, "Não é um pedido de ação no computador: conversa, pergunta, outra coisa."),
];

const MEDIA_CRITERIA: &[(&str, &str)] = &[
    ("play", "tocar / continuar"),
    ("pause", "pausar / parar"),
    ("next", "próxima faixa"),
    ("previous", "faixa anterior / voltar"),
    (NONE, "nenhuma destas"),
];

const VOLUME_CRITERIA: &[(&str, &str)] = &[
    ("up", "aumentar o volume"),
    ("down", "diminuir o volume"),
    ("mute", "silenciar / mudo"),
    ("unmute", "tirar do mudo / voltar o som"),
    ("set", "colocar o volume num valor específico"),
    (NONE, "nenhuma destas"),
];

fn confirmation_questions(q: &mut Questions) {
    q.insert("approve", Question::noul(
        "O usuário está dizendo SIM: aprovando, autorizando ou mandando seguir com o pedido pendente.", None));
    q.insert("deny", Question::noul(
        "O usuário está dizendo NÃO: recusando, cancelando ou mandando parar o pedido pendente.", None));
}

/// Perguntas para esta situação. `None` = nada a perguntar.
pub fn build_questions(s: &Situation) -> Option<Questions> {
    if s.heard.trim().is_empty() {
        return None;
    }
    let mut q = Questions::default();
    q.insert("always", Question::noul(
        "O usuário pede para o assistente nunca mais perguntar antes de fazer isso (liberar para sempre).", None));
    if s.pending_confirm {
        confirmation_questions(&mut q);
    }
    if s.turn_locked {
        return if s.pending_confirm { Some(q) } else { None };
    }
    q.insert("intent", Question::choice(
        "O que o usuário está pedindo em `state`? Escolha `none` se não for um pedido claro de ação.",
        INTENT_CRITERIA.iter().copied()));
    let apps = app_candidates(s.heard, s.inventory);
    if !apps.is_empty() {
        let mut criteria: Vec<(String, String)> = apps
            .iter()
            .map(|name| (name.clone(), format!("o aplicativo {name}")))
            .collect();
        criteria.push((NONE.into(), "nenhum destes aplicativos".into()));
        q.insert("app", Question::choice("Se for para abrir um aplicativo, qual?", criteria));
    }
    if !s.inventory.sites.is_empty() {
        let mut criteria: Vec<(String, String)> = s
            .inventory
            .sites
            .iter()
            .take(MAX_CANDIDATES)
            .map(|site| (site.name.clone(), format!("o site {} ({})", site.name, site.url)))
            .collect();
        criteria.push((NONE.into(), "nenhum destes sites".into()));
        q.insert("site", Question::choice("Se for para abrir um site, qual?", criteria));
    }
    q.insert("media", Question::choice("Se for controle de mídia, qual ação?", MEDIA_CRITERIA.iter().copied()));
    q.insert("volume", Question::choice("Se for volume, qual ação?", VOLUME_CRITERIA.iter().copied()));
    Some(q)
}
```

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p openjarvisbr-core reflex::decide`
Expected: 9 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/reflex/decide.rs
git commit -m "feat(reflex): build_questions [JRV]"
```

### Task 10: `decide` com limiares

**Files:**
- Modify: `crates/core/src/reflex/decide.rs`

**Interfaces:**
- Produces: `pub struct Thresholds { pub act: f32, pub confirm: f32 }`, `pub enum Decision { Act(ToolCall), Approve, Deny, AlwaysAllow, Nothing }`, `pub fn decide(s: &Situation, a: &Answers, t: &Thresholds) -> Decision`, `pub fn volume_level(heard: &str) -> Option<u8>`, `pub const REFLEX_CALL_PREFIX: &str = "reflex-"`.

- [ ] **Step 1: Testes de mesa**

```rust
use crate::reflex::questions::Answer;
use std::collections::BTreeMap;

fn t() -> Thresholds { Thresholds { act: 0.85, confirm: 0.85 } }

fn choice(a: &mut Answers, id: &str, pick: &str, p: f32) {
    let mut probs = BTreeMap::new();
    probs.insert(pick.to_string(), p);
    probs.insert(NONE.to_string(), 1.0 - p);
    a.answers.insert(id.into(), Answer::Choice { choice: pick.into(), probabilities: probs, confidence: p });
}
fn noul(a: &mut Answers, id: &str, v: f32) {
    a.answers.insert(id.into(), Answer::Noul { noul: v });
}

#[test]
fn abre_app_com_confianca() {
    let i = inv(&["Safari"], &[]);
    let s = Situation { heard: "abre o safari", inventory: &i, pending_confirm: false, turn_locked: false };
    let mut a = Answers::default();
    choice(&mut a, "intent", "open_app", 0.95);
    choice(&mut a, "app", "Safari", 0.97);
    match decide(&s, &a, &t()) {
        Decision::Act(call) => {
            assert_eq!(call.name, "app.open");
            assert_eq!(call.args, json!({"name": "Safari"}));
            assert!(call.id.starts_with(REFLEX_CALL_PREFIX));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn limiar_exato_age_e_abaixo_nao() {
    let i = inv(&["Safari"], &[]);
    let s = Situation { heard: "abre o safari", inventory: &i, pending_confirm: false, turn_locked: false };
    let mut a = Answers::default();
    choice(&mut a, "intent", "open_app", 0.85);
    choice(&mut a, "app", "Safari", 0.85);
    assert!(matches!(decide(&s, &a, &t()), Decision::Act(_)));
    let mut b = Answers::default();
    choice(&mut b, "intent", "open_app", 0.95);
    choice(&mut b, "app", "Safari", 0.84);
    assert!(matches!(decide(&s, &b, &t()), Decision::Nothing));
}

#[test]
fn alvo_none_nao_age() {
    let i = inv(&["Safari"], &[]);
    let s = Situation { heard: "abre aí", inventory: &i, pending_confirm: false, turn_locked: false };
    let mut a = Answers::default();
    choice(&mut a, "intent", "open_app", 0.95);
    choice(&mut a, "app", NONE, 0.9);
    assert!(matches!(decide(&s, &a, &t()), Decision::Nothing));
}

#[test]
fn site_so_da_lista_com_url_do_config() {
    let i = inv(&[], &[]);
    let s = Situation { heard: "abre o youtube", inventory: &i, pending_confirm: false, turn_locked: false };
    let mut a = Answers::default();
    choice(&mut a, "intent", "open_site", 0.95);
    choice(&mut a, "site", "YouTube", 0.95);
    match decide(&s, &a, &t()) {
        Decision::Act(call) => {
            assert_eq!(call.name, "web.open");
            assert_eq!(call.args, json!({"url": "https://youtube.com"}));
        }
        other => panic!("{other:?}"),
    }
    let mut b = Answers::default();
    choice(&mut b, "intent", "open_site", 0.95);
    choice(&mut b, "site", "Globo", 0.95); // não está no config
    assert!(matches!(decide(&s, &b, &t()), Decision::Nothing));
}

#[test]
fn midia_e_volume() {
    let i = inv(&[], &[]);
    let s = Situation { heard: "próxima música", inventory: &i, pending_confirm: false, turn_locked: false };
    let mut a = Answers::default();
    choice(&mut a, "intent", "media", 0.95);
    choice(&mut a, "media", "next", 0.95);
    assert!(matches!(decide(&s, &a, &t()), Decision::Act(c) if c.name == "media.control" && c.args == json!({"action": "next"})));

    let s2 = Situation { heard: "coloca o volume em 30", inventory: &i, pending_confirm: false, turn_locked: false };
    let mut b = Answers::default();
    choice(&mut b, "intent", "volume", 0.95);
    choice(&mut b, "volume", "set", 0.95);
    assert!(matches!(decide(&s2, &b, &t()), Decision::Act(c) if c.args == json!({"action": "set", "level": 30})));

    let s3 = Situation { heard: "aumenta o volume", inventory: &i, pending_confirm: false, turn_locked: false };
    let mut c = Answers::default();
    choice(&mut c, "intent", "volume", 0.95);
    choice(&mut c, "volume", "up", 0.95);
    assert!(matches!(decide(&s3, &c, &t()), Decision::Act(c) if c.args == json!({"action": "up"})));

    // set sem número na fala: nada
    let s4 = Situation { heard: "coloca o volume", inventory: &i, pending_confirm: false, turn_locked: false };
    assert!(matches!(decide(&s4, &b, &t()), Decision::Nothing));
}

#[test]
fn confirmacao_por_voz() {
    let i = inv(&[], &[]);
    let s = Situation { heard: "pode ir", inventory: &i, pending_confirm: true, turn_locked: false };
    let mut a = Answers::default();
    noul(&mut a, "approve", 0.9); noul(&mut a, "deny", 0.1); noul(&mut a, "always", 0.0);
    assert!(matches!(decide(&s, &a, &t()), Decision::Approve));
    let mut b = Answers::default();
    noul(&mut b, "approve", 0.1); noul(&mut b, "deny", 0.92); noul(&mut b, "always", 0.0);
    assert!(matches!(decide(&s, &b, &t()), Decision::Deny));
    // empate: nada
    let mut c = Answers::default();
    noul(&mut c, "approve", 0.9); noul(&mut c, "deny", 0.6);
    assert!(matches!(decide(&s, &c, &t()), Decision::Nothing));
    // sem pendente, approve alto é ignorado
    let s2 = Situation { heard: "pode ir", inventory: &i, pending_confirm: false, turn_locked: false };
    assert!(matches!(decide(&s2, &a, &t()), Decision::Nothing));
}

#[test]
fn sempre_pode_vence_o_resto() {
    let i = inv(&["Safari"], &[]);
    let s = Situation { heard: "sempre pode abrir", inventory: &i, pending_confirm: true, turn_locked: false };
    let mut a = Answers::default();
    noul(&mut a, "always", 0.9); noul(&mut a, "approve", 0.9); noul(&mut a, "deny", 0.0);
    choice(&mut a, "intent", "open_app", 0.99);
    choice(&mut a, "app", "Safari", 0.99);
    assert!(matches!(decide(&s, &a, &t()), Decision::AlwaysAllow));
}

#[test]
fn turno_travado_nunca_age() {
    let i = inv(&["Safari"], &[]);
    let s = Situation { heard: "abre o safari", inventory: &i, pending_confirm: false, turn_locked: true };
    let mut a = Answers::default();
    choice(&mut a, "intent", "open_app", 0.99);
    choice(&mut a, "app", "Safari", 0.99);
    assert!(matches!(decide(&s, &a, &t()), Decision::Nothing));
}

#[test]
fn extrai_nivel_de_volume() {
    assert_eq!(volume_level("coloca o volume em 30"), Some(30));
    assert_eq!(volume_level("volume 100 por cento"), Some(100));
    assert_eq!(volume_level("volume em 250"), None);
    assert_eq!(volume_level("aumenta o volume"), None);
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core reflex::decide`
Expected: erro de compilação.

- [ ] **Step 3: Implementar**

```rust
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    pub act: f32,
    pub confirm: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Act(ToolCall),
    Approve,
    Deny,
    AlwaysAllow,
    Nothing,
}

pub const REFLEX_CALL_PREFIX: &str = "reflex-";

fn new_call(name: &str, args: serde_json::Value) -> ToolCall {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    ToolCall { id: format!("{REFLEX_CALL_PREFIX}{n}"), name: name.to_string(), args }
}

/// Primeiro inteiro 0..=100 da fala.
pub fn volume_level(heard: &str) -> Option<u8> {
    normalize(heard)
        .split(' ')
        .filter_map(|t| t.parse::<u32>().ok())
        .find(|n| *n <= 100)
        .map(|n| n as u8)
}

/// Escolha `id` com opção ≠ none e probabilidade ≥ limiar.
fn confident_pick<'a>(a: &'a Answers, id: &str, min: f32) -> Option<&'a str> {
    let (pick, _) = a.choice(id)?;
    if pick == NONE {
        return None;
    }
    let p = a.probability(id, pick).unwrap_or(0.0);
    (p >= min).then_some(pick)
}

pub fn decide(s: &Situation, a: &Answers, t: &Thresholds) -> Decision {
    if a.noul("always").unwrap_or(0.0) >= t.confirm {
        return Decision::AlwaysAllow;
    }
    if s.pending_confirm {
        let approve = a.noul("approve").unwrap_or(0.0);
        let deny = a.noul("deny").unwrap_or(0.0);
        if approve >= t.confirm && deny < 0.5 {
            return Decision::Approve;
        }
        if deny >= t.confirm && approve < 0.5 {
            return Decision::Deny;
        }
    }
    if s.turn_locked {
        return Decision::Nothing;
    }
    let Some(intent) = confident_pick(a, "intent", t.act) else {
        return Decision::Nothing;
    };
    match intent {
        "open_app" => match confident_pick(a, "app", t.act) {
            Some(name) => Decision::Act(new_call("app.open", json!({ "name": name }))),
            None => Decision::Nothing,
        },
        "open_site" => {
            let Some(name) = confident_pick(a, "site", t.act) else { return Decision::Nothing };
            match s.inventory.sites.iter().find(|site| site.name == name) {
                Some(site) => Decision::Act(new_call("web.open", json!({ "url": site.url }))),
                None => Decision::Nothing,
            }
        }
        "media" => match confident_pick(a, "media", t.act) {
            Some(action) => Decision::Act(new_call("media.control", json!({ "action": action }))),
            None => Decision::Nothing,
        },
        "volume" => match confident_pick(a, "volume", t.act) {
            Some("set") => match volume_level(s.heard) {
                Some(level) => Decision::Act(new_call("sys.volume", json!({ "action": "set", "level": level }))),
                None => Decision::Nothing,
            },
            Some(action) => Decision::Act(new_call("sys.volume", json!({ "action": action }))),
            None => Decision::Nothing,
        },
        _ => Decision::Nothing,
    }
}
```

Atenção: `sys.volume` hoje aceita `get|set|mute|unmute`. `up`/`down` **não
existem**. O card E (Task 14) resolve isso lendo o volume atual e somando ±10;
alternativa mais limpa é o card C **também** adicionar `up`/`down` em
`crates/core/src/tools/system/volume.rs` (`VolumeAction::Up`/`Down` = get + set
±10, com teste). Decisão: **C adiciona `up`/`down` em `volume.rs`** nesta task,
para `Decision::Act` ser executável direto. Atualize o `enum` do schema em
`spec()` para `["get","set","up","down","mute","unmute"]` e adicione um teste
`from_args` para `up`. Confira o AppleScript existente de `set` e reaproveite:
`set volume output volume ((output volume of (get volume settings)) + 10)`.

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p openjarvisbr-core reflex::decide && cargo test -p openjarvisbr-core tools::system::volume`
Expected: 18 passed em decide; volume passa.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/reflex/decide.rs crates/core/src/tools/system/volume.rs
git commit -m "feat(reflex): decide com limiares; sys.volume ganha up/down [JRV]"
```

### Task 11: Guarda de segurança das tools do reflexo

**Files:**
- Modify: `crates/core/src/reflex/decide.rs`

**Interfaces:**
- Produces: `pub const REFLEX_TOOLS: &[&str] = &["app.open", "web.open", "sys.volume", "media.control"]`, `pub fn is_reflex_tool(name: &str) -> bool`.

- [ ] **Step 1: Teste**

```rust
#[test]
fn so_tools_seguras_saem_do_reflexo() {
    for name in REFLEX_TOOLS {
        assert!(crate::tools::decisions::never_asks(name), "{name} precisa estar em NEVER_ASK");
        assert!(is_reflex_tool(name));
    }
    assert!(!is_reflex_tool("shell.run"));
    assert!(!is_reflex_tool("fs.read")); // NEVER_ASK, mas fora do reflexo
}
```

- [ ] **Step 2: Implementar**

```rust
/// Tools que o reflexo pode disparar. Constante: não é configurável.
pub const REFLEX_TOOLS: &[&str] = &["app.open", "web.open", "sys.volume", "media.control"];

pub fn is_reflex_tool(name: &str) -> bool {
    REFLEX_TOOLS.contains(&name)
}
```

E no fim de `decide`, embrulhe o retorno: se for `Decision::Act(call)` e
`!is_reflex_tool(&call.name)`, devolva `Decision::Nothing` (defesa em
profundidade; hoje todo caminho já produz só essas quatro).

- [ ] **Step 3: Rodar, clippy, commit**

Run: `cargo test -p openjarvisbr-core reflex && cargo clippy -p openjarvisbr-core -- -D warnings`

```bash
git add crates/core/src/reflex/decide.rs
git commit -m "feat(reflex): lista fixa de tools do reflexo [JRV]"
```

---

## Card D — Orquestrador `Reflex` (onda 2)

### Task 12: `ReflexConfig` e `Reflex::hear` com debounce e uma ação por turno

**Files:**
- Modify: `crates/core/src/reflex/mod.rs`

**Interfaces:**
- Consumes: `Judge`, `EyeHandle`, `build_questions`, `decide`, `Thresholds`, `Decision`, `ReflexSettings`.
- Produces: `pub struct Reflex`, `Reflex::new(settings: ReflexSettings, judge: Arc<dyn Judge>, eye: EyeHandle) -> (Reflex, mpsc::UnboundedReceiver<Outcome>)`, `Reflex::hear(&self, heard: String, pending_confirm: bool)`, `Reflex::end_turn(&self)`, `pub struct Outcome { pub decision: Decision, pub latency_ms: u32, pub confidence: f32 }`.

- [ ] **Step 1: Testes**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ReflexSettings, SiteConfig};
    use crate::reflex::decide::NONE;
    use crate::reflex::eye::{AppEntry, Eye, RunningProbe};
    use crate::reflex::judge::{FakeJudge, JudgeError};
    use crate::reflex::questions::{Answer, Answers};
    use std::collections::BTreeMap;
    use std::time::Duration;

    struct NoProbe;
    impl RunningProbe for NoProbe { fn running_app_names(&self) -> Vec<String> { vec![] } }

    fn eye() -> EyeHandle {
        let dir = std::env::temp_dir().join(format!("reflex-eye-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Safari.app")).unwrap();
        Eye::start_with(vec![dir], vec![SiteConfig { name: "YouTube".into(), url: "https://youtube.com".into() }],
            Arc::new(NoProbe), Duration::from_secs(3600), Duration::from_secs(3600))
    }

    fn settings() -> ReflexSettings {
        ReflexSettings { enabled: true, api_key: Some("x".into()), debounce_ms: 30, ..Default::default() }
    }

    fn open_safari() -> Answers {
        let mut a = Answers::default();
        for (id, pick) in [("intent", "open_app"), ("app", "Safari")] {
            let mut p = BTreeMap::new(); p.insert(pick.to_string(), 0.95); p.insert(NONE.to_string(), 0.05);
            a.answers.insert(id.into(), Answer::Choice { choice: pick.into(), probabilities: p, confidence: 0.95 });
        }
        a
    }

    #[tokio::test]
    async fn debounce_junta_fragmentos_e_age_uma_vez_por_turno() {
        let judge = Arc::new(FakeJudge::new());
        judge.push(open_safari());
        judge.push(open_safari());
        let (reflex, mut rx) = Reflex::new(settings(), judge.clone(), eye());
        reflex.hear("abre o".into(), false);
        tokio::time::sleep(Duration::from_millis(5)).await;
        reflex.hear("abre o safari".into(), false);
        let out = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await.unwrap().unwrap();
        assert!(matches!(out.decision, Decision::Act(ref c) if c.name == "app.open"));
        // só uma pergunta (a primeira foi engolida pelo debounce)
        assert_eq!(judge.calls().len(), 1);
        assert_eq!(judge.calls()[0].0, "abre o safari");
        // turno travado: novo fragmento não gera outra ação
        reflex.hear("abre o safari de novo".into(), false);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(rx.try_recv().is_err());
        // destrava
        reflex.end_turn();
        reflex.hear("abre o safari".into(), false);
        let out2 = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await.unwrap().unwrap();
        assert!(matches!(out2.decision, Decision::Act(_)));
    }

    #[tokio::test]
    async fn erro_do_juiz_nao_emite_nada() {
        let judge = Arc::new(FakeJudge::new());
        judge.push_err(JudgeError::Timeout);
        let (reflex, mut rx) = Reflex::new(settings(), judge, eye());
        reflex.hear("abre o safari".into(), false);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn desligado_nao_pergunta() {
        let judge = Arc::new(FakeJudge::new());
        let mut s = settings(); s.enabled = false;
        let (reflex, _rx) = Reflex::new(s, judge.clone(), eye());
        reflex.hear("abre o safari".into(), false);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(judge.calls().is_empty());
    }

    #[tokio::test]
    async fn fragmento_novo_cancela_pergunta_lenta_em_voo() {
        let judge = Arc::new(FakeJudge::with_delay(Duration::from_millis(150)));
        judge.push(open_safari());
        judge.push(open_safari());
        let (reflex, mut rx) = Reflex::new(settings(), judge.clone(), eye());
        reflex.hear("abre".into(), false);
        tokio::time::sleep(Duration::from_millis(60)).await; // debounce passou, pergunta em voo há ~30ms (< 200ms)
        reflex.hear("abre o safari".into(), false);
        let out = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.unwrap().unwrap();
        assert!(matches!(out.decision, Decision::Act(_)));
        // a primeira foi cancelada: só um Outcome
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(rx.try_recv().is_err());
    }
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core reflex::tests`
Expected: erro de compilação.

- [ ] **Step 3: Implementar**

Substituir o conteúdo de `mod.rs` (mantendo o cabeçalho e os `pub mod`):

```rust
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

pub use crate::config::ReflexSettings;
pub use decide::{Decision, Thresholds};
pub use eye::EyeHandle;
pub use judge::Judge;

use decide::{build_questions, decide, Situation};
use judge::JudgeError;

/// Uma decisão pronta para o engine, com telemetria.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub decision: Decision,
    pub latency_ms: u32,
    pub confidence: f32,
}

/// Pergunta em voo há menos que isto é cancelada quando chega fragmento novo.
const CANCEL_IF_YOUNGER_THAN: Duration = Duration::from_millis(200);
/// Erros seguidos de cota/sobrecarga até pausar.
const BACKOFF_AFTER: u32 = 3;
const BACKOFF_FOR: Duration = Duration::from_secs(30);

struct InFlight {
    started: Instant,
    task: JoinHandle<()>,
}

pub struct Reflex {
    settings: ReflexSettings,
    judge: Arc<dyn Judge>,
    eye: EyeHandle,
    tx: mpsc::UnboundedSender<Outcome>,
    /// Já agiu neste turno.
    locked: Arc<AtomicBool>,
    /// Desligado de vez (401) até reiniciar.
    dead: Arc<AtomicBool>,
    quota_errors: Arc<AtomicU32>,
    paused_until: Arc<Mutex<Option<Instant>>>,
    /// Debounce pendente e/ou pergunta em voo.
    pending: Arc<Mutex<Option<InFlight>>>,
}

impl Reflex {
    pub fn new(settings: ReflexSettings, judge: Arc<dyn Judge>, eye: EyeHandle) -> (Self, mpsc::UnboundedReceiver<Outcome>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let reflex = Self {
            settings,
            judge,
            eye,
            tx,
            locked: Arc::new(AtomicBool::new(false)),
            dead: Arc::new(AtomicBool::new(false)),
            quota_errors: Arc::new(AtomicU32::new(0)),
            paused_until: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(None)),
        };
        (reflex, rx)
    }

    pub fn enabled(&self) -> bool {
        self.settings.enabled && !self.dead.load(Ordering::Relaxed)
    }

    /// Fragmento novo da fala do usuário (acumulada no turno).
    pub fn hear(&self, heard: String, pending_confirm: bool) {
        if !self.enabled() {
            return;
        }
        if let Some(until) = *self.paused_until.lock().unwrap() {
            if Instant::now() < until {
                return;
            }
        }
        {
            let mut slot = self.pending.lock().unwrap();
            if let Some(prev) = slot.take() {
                if prev.started.elapsed() < CANCEL_IF_YOUNGER_THAN {
                    prev.task.abort();
                }
                // mais velha que isso: deixa terminar (o resultado ainda vale)
            }
        }
        let judge = Arc::clone(&self.judge);
        let eye = self.eye.clone();
        let tx = self.tx.clone();
        let locked = Arc::clone(&self.locked);
        let dead = Arc::clone(&self.dead);
        let quota = Arc::clone(&self.quota_errors);
        let paused = Arc::clone(&self.paused_until);
        let thresholds = Thresholds { act: self.settings.act_threshold, confirm: self.settings.confirm_threshold };
        let debounce = Duration::from_millis(self.settings.debounce_ms);
        let task = tokio::spawn(async move {
            tokio::time::sleep(debounce).await;
            let inventory = eye.snapshot();
            let turn_locked = locked.load(Ordering::Relaxed);
            let situation = Situation { heard: &heard, inventory: &inventory, pending_confirm, turn_locked };
            let Some(questions) = build_questions(&situation) else { return };
            let t0 = Instant::now();
            match judge.ask(&heard, &questions).await {
                Ok(answers) => {
                    quota.store(0, Ordering::Relaxed);
                    let latency_ms = t0.elapsed().as_millis() as u32;
                    let decision = decide(&situation, &answers, &thresholds);
                    let confidence = answers.choice("intent").map(|c| c.1).unwrap_or(0.0);
                    debug!(?decision, latency_ms, "reflexo");
                    match &decision {
                        Decision::Nothing => {}
                        Decision::Act(_) => {
                            // corrida: outro fragmento pode ter agido no meio
                            if locked.swap(true, Ordering::SeqCst) {
                                return;
                            }
                            let _ = tx.send(Outcome { decision, latency_ms, confidence });
                        }
                        _ => {
                            let _ = tx.send(Outcome { decision, latency_ms, confidence });
                        }
                    }
                }
                Err(JudgeError::Unauthorized) => {
                    warn!("chave da TypeSafe inválida; reflexo desligado até reiniciar");
                    dead.store(true, Ordering::Relaxed);
                }
                Err(err @ (JudgeError::RateLimited | JudgeError::Overloaded)) => {
                    let n = quota.fetch_add(1, Ordering::Relaxed) + 1;
                    if n >= BACKOFF_AFTER {
                        info!(%err, "reflexo pausado por {}s", BACKOFF_FOR.as_secs());
                        *paused.lock().unwrap() = Some(Instant::now() + BACKOFF_FOR);
                        quota.store(0, Ordering::Relaxed);
                    }
                }
                Err(err) => debug!(%err, "reflexo pulou esta rodada"),
            }
        });
        *self.pending.lock().unwrap() = Some(InFlight { started: Instant::now() + debounce, task });
    }

    /// Fim do turno do modelo: o reflexo pode agir de novo.
    pub fn end_turn(&self) {
        self.locked.store(false, Ordering::SeqCst);
    }
}
```

Nota sobre `started: Instant::now() + debounce`: a idade da pergunta em voo é
contada a partir do fim do debounce, que é quando o HTTP começa. Durante o
debounce a idade é "negativa" e `elapsed()` devolve zero, então sempre cancela,
que é o comportamento desejado.

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test -p openjarvisbr-core reflex`
Expected: todos passam (questions, judge, eye, decide, mod).

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/reflex/mod.rs
git commit -m "feat(reflex): orquestrador com debounce, cancelamento e trava por turno [JRV]"
```

### Task 13: Fábrica `Reflex::from_settings`

**Files:**
- Modify: `crates/core/src/reflex/mod.rs`

**Interfaces:**
- Produces: `Reflex::from_settings(settings: ReflexSettings) -> Option<(Reflex, mpsc::UnboundedReceiver<Outcome>)>` (None se desligado ou sem chave; cria `JevClient` e `Eye::start`).

- [ ] **Step 1: Teste**

```rust
#[tokio::test]
async fn from_settings_sem_chave_da_none() {
    assert!(Reflex::from_settings(ReflexSettings::default()).is_none());
    let s = ReflexSettings { enabled: true, api_key: Some("k".into()), ..Default::default() };
    assert!(Reflex::from_settings(s).is_some());
}
```

- [ ] **Step 2: Implementar**

```rust
impl Reflex {
    /// Monta o reflexo real a partir do config. `None` = desligado.
    pub fn from_settings(settings: ReflexSettings) -> Option<(Self, mpsc::UnboundedReceiver<Outcome>)> {
        if !settings.enabled {
            return None;
        }
        let key = settings.api_key.clone()?;
        let client = judge::JevClient::new(key, settings.model.clone()).ok()?;
        let eye = eye::Eye::start(settings.sites.clone());
        Some(Self::new(settings, Arc::new(client), eye))
    }
}
```

- [ ] **Step 3: Rodar, clippy, commit**

Run: `cargo test -p openjarvisbr-core reflex && cargo clippy -p openjarvisbr-core -- -D warnings`

```bash
git add crates/core/src/reflex/mod.rs
git commit -m "feat(reflex): Reflex::from_settings [JRV]"
```

---

## Card E — Engine (onda 2, depois de D)

### Task 14: Integração no engine

**Files:**
- Modify: `crates/core/src/engine.rs`
- Modify: `crates/core/src/engine_tools.rs`

**Interfaces:**
- Consumes: `Reflex`, `Outcome`, `Decision`, `ReflexSettings`, `FakeJudge`, `EyeHandle`.
- Produces: `EngineConfig.reflex: ReflexSettings`; `EngineEvent::ReflexActed { call: ToolCall, latency_ms: u32, confidence: f32 }`, `EngineEvent::ReflexConfirmed { approve: bool, confidence: f32 }`; `engine_tools::REFLEX_DONE_WINDOW: Duration = 8s`; `engine_tools::reflex_context_text(call: &ToolCall) -> String`; `fake::FakeBackend::with_reflex(self, judge: Arc<dyn Judge>, eye: EyeHandle) -> Self`.

- [ ] **Step 1: Testes no `mod tests` de engine.rs**

Localize como o `FakeBackend` registra tools (`fake.registry`) e como os testes
existentes de tool funcionam (busque `fn tool_safe_executes` ou parecido na
seção de testes). Adicione:

```rust
#[tokio::test]
async fn reflexo_abre_app_antes_do_gemini_e_deduplica_a_chamada_dele() {
    use crate::reflex::eye::{Eye, RunningProbe};
    use crate::reflex::judge::FakeJudge;
    use crate::reflex::questions::{Answer, Answers};
    use std::collections::BTreeMap;

    struct NoProbe;
    impl RunningProbe for NoProbe { fn running_app_names(&self) -> Vec<String> { vec![] } }

    // tool fake `app.open` que só registra a chamada
    #[derive(Clone, Default)]
    struct SpyOpen(Arc<Mutex<Vec<serde_json::Value>>>);
    #[async_trait::async_trait]
    impl crate::tools::Tool for SpyOpen {
        fn spec(&self) -> ToolSpec {
            ToolSpec { name: "app.open".into(), description: "spy".into(), parameters: serde_json::json!({"type":"object"}), risk: Risk::Safe }
        }
        async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
            self.0.lock().unwrap().push(args);
            Ok(serde_json::json!({"status": "aberto"}))
        }
    }

    let dir = std::env::temp_dir().join(format!("engine-reflex-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Safari.app")).unwrap();
    let eye = Eye::start_with(vec![dir], vec![], Arc::new(NoProbe), Duration::from_secs(3600), Duration::from_secs(3600));
    let judge = Arc::new(FakeJudge::new());
    let mut answers = Answers::default();
    for (id, pick) in [("intent", "open_app"), ("app", "Safari")] {
        let mut p = BTreeMap::new(); p.insert(pick.to_string(), 0.96); p.insert("none".to_string(), 0.04);
        answers.answers.insert(id.into(), Answer::Choice { choice: pick.into(), probabilities: p, confidence: 0.96 });
    }
    judge.push(answers);

    let spy = SpyOpen::default();
    let (script_tx, script_rx) = mpsc::channel(16);
    let (_mic_tx, mic_rx) = mpsc::channel(16);
    let mut fake = fake::FakeBackend::new(script_rx, mic_rx);
    fake.registry.register(Box::new(spy.clone()));
    let fake = fake.with_reflex(judge, eye);
    let mut config = test_config();
    config.tools = vec!["*".into()];
    config.reflex = crate::config::ReflexSettings { enabled: true, api_key: Some("k".into()), debounce_ms: 10, ..Default::default() };
    let handle = start_with(config, Backend::Fake(fake)).await.unwrap();
    let mut events = handle.events();

    script_tx.send(ServerEvent::UserText("abre o safari".into())).await.unwrap();

    // espera ReflexActed
    let acted = loop {
        match next_non_level(&mut events).await {
            EngineEvent::ReflexActed { call, .. } => break call,
            EngineEvent::Error { message, .. } => panic!("{message}"),
            _ => {}
        }
    };
    assert_eq!(acted.name, "app.open");
    // a tool rodou uma vez
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(spy.0.lock().unwrap().len(), 1);

    // Gemini chama a mesma ação em seguida: deduplicada, tool NÃO roda de novo
    script_tx.send(ServerEvent::ToolCall(vec![ToolCall { id: "g1".into(), name: "app.open".into(), args: serde_json::json!({"name": "Safari"}) }])).await.unwrap();
    let result = loop {
        match next_non_level(&mut events).await {
            EngineEvent::ToolResult { id, ok, .. } if id == "g1" => break ok,
            _ => {}
        }
    };
    assert!(result);
    assert_eq!(spy.0.lock().unwrap().len(), 1, "dedup: não executa de novo");
    handle.stop().await;
}

#[tokio::test]
async fn reflexo_aprova_pendente_por_voz() {
    use crate::reflex::eye::{Eye, RunningProbe};
    use crate::reflex::judge::FakeJudge;
    use crate::reflex::questions::{Answer, Answers};
    struct NoProbe;
    impl RunningProbe for NoProbe { fn running_app_names(&self) -> Vec<String> { vec![] } }

    // tool Confirm fake
    #[derive(Clone, Default)]
    struct SpyShell(Arc<Mutex<u32>>);
    #[async_trait::async_trait]
    impl crate::tools::Tool for SpyShell {
        fn spec(&self) -> ToolSpec {
            ToolSpec { name: "shell.run".into(), description: "spy".into(), parameters: serde_json::json!({"type":"object"}), risk: Risk::Confirm }
        }
        async fn call(&self, _args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
            *self.0.lock().unwrap() += 1;
            Ok(serde_json::json!({"status": "ok"}))
        }
    }

    let eye = Eye::start_with(vec![], vec![], Arc::new(NoProbe), Duration::from_secs(3600), Duration::from_secs(3600));
    let judge = Arc::new(FakeJudge::new());
    let mut a = Answers::default();
    a.answers.insert("approve".into(), Answer::Noul { noul: 0.93 });
    a.answers.insert("deny".into(), Answer::Noul { noul: 0.03 });
    a.answers.insert("always".into(), Answer::Noul { noul: 0.01 });
    judge.push(a);

    let spy = SpyShell::default();
    let (script_tx, script_rx) = mpsc::channel(16);
    let (_mic_tx, mic_rx) = mpsc::channel(16);
    let mut fake = fake::FakeBackend::new(script_rx, mic_rx);
    fake.registry.register(Box::new(spy.clone()));
    let fake = fake.with_reflex(judge, eye);
    let mut config = test_config();
    config.tools = vec!["*".into()];
    config.reflex = crate::config::ReflexSettings { enabled: true, api_key: Some("k".into()), debounce_ms: 10, ..Default::default() };
    let handle = start_with(config, Backend::Fake(fake)).await.unwrap();
    let mut events = handle.events();

    script_tx.send(ServerEvent::ToolCall(vec![ToolCall { id: "c1".into(), name: "shell.run".into(), args: serde_json::json!({"command": "ls"}) }])).await.unwrap();
    loop { if let EngineEvent::ToolConfirmNeeded { .. } = next_non_level(&mut events).await { break } }

    // frase fora das listas fixas de engine_tools
    script_tx.send(ServerEvent::UserText("bora, toca ficha".into())).await.unwrap();
    loop {
        match next_non_level(&mut events).await {
            EngineEvent::ReflexConfirmed { approve: true, .. } => break,
            EngineEvent::ToolResult { id, .. } if id == "c1" => panic!("resolveu sem o reflexo"),
            _ => {}
        }
    }
    loop { if let EngineEvent::ToolResult { id, ok: true, .. } = next_non_level(&mut events).await { if id == "c1" { break } } }
    assert_eq!(*spy.0.lock().unwrap(), 1);
    handle.stop().await;
}
```

Se `fake.registry` não for público, torne `pub(crate)` ou adicione um método
`FakeBackend::register(&mut self, tool: Box<dyn Tool>)`. Se o teste existente de
tools já tiver um helper para isso, use o helper.

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test -p openjarvisbr-core engine::tests::reflexo`
Expected: erro de compilação (`with_reflex`, `ReflexActed`, `config.reflex`).

- [ ] **Step 3: `engine_tools.rs`**

```rust
/// Ação feita pelo reflexo: uma chamada igual do modelo dentro desta janela
/// recebe sucesso sem executar de novo.
pub const REFLEX_DONE_WINDOW: Duration = Duration::from_secs(8);

/// Texto de contexto enviado ao modelo quando o reflexo agiu, para ele não
/// repetir a ação nem narrar como futuro.
pub fn reflex_context_text(call: &ToolCall) -> String {
    format!(
        "[sistema] já executado agora pelo reflexo: {} ({}). Não chame de novo; \
         se for comentar, fale no passado e seja breve.",
        call.name,
        call_summary(call)
    )
}
```

- [ ] **Step 4: `engine.rs`**

1. `EngineConfig` ganha `pub reflex: crate::config::ReflexSettings` (e o `Debug`
   manual imprime `.field("reflex", &self.reflex)`, que já mascara a chave).
   Atualize `test_config()` com `reflex: Default::default()`.
2. `EngineEvent` ganha:

```rust
    /// O reflexo (Jev) executou uma ação segura antes de o modelo responder.
    ReflexActed { call: ToolCall, latency_ms: u32, confidence: f32 },
    /// O reflexo resolveu uma confirmação pendente por voz.
    ReflexConfirmed { approve: bool, confidence: f32 },
```

3. `Worker` ganha campos:

```rust
    reflex: Option<crate::reflex::Reflex>,
    reflex_rx: mpsc::UnboundedReceiver<crate::reflex::Outcome>,
    /// Ações feitas pelo reflexo (nome, args, quando), para deduplicar.
    recently_done: Vec<(String, serde_json::Value, Instant, serde_json::Value)>,
```

   Em `Worker::open`, depois de montar o `registry`:

```rust
        let (reflex, reflex_rx) = match &backend {
            Backend::Fake(fake) if fake.reflex.is_some() => {
                let (judge, eye) = fake.reflex.clone().unwrap();
                let (r, rx) = crate::reflex::Reflex::new(config.reflex.clone(), judge, eye);
                (Some(r), rx)
            }
            Backend::Fake(_) => { let (_tx, rx) = mpsc::unbounded_channel(); (None, rx) }
            Backend::Real => match crate::reflex::Reflex::from_settings(config.reflex.clone()) {
                Some((r, rx)) => { info!("reflexo ligado"); (Some(r), rx) }
                None => { let (_tx, rx) = mpsc::unbounded_channel(); (None, rx) }
            },
        };
```

   Guarde `_tx`? Não: um `UnboundedReceiver` cujo sender caiu devolve `None` em
   `recv()`, e no `select!` isso vira loop quente. Use o padrão
   `Some(out) = self.reflex_rx.recv()` **só se** `reflex.is_some()`: mais simples é
   manter um sender vivo no `Worker` (`_reflex_keepalive: mpsc::UnboundedSender<Outcome>`)
   no caso `None`. Faça isso.

4. `FakeBackend` (módulo `fake` dentro de engine.rs) ganha
   `pub reflex: Option<(Arc<dyn Judge>, EyeHandle)>` e
   `pub fn with_reflex(mut self, judge: Arc<dyn Judge>, eye: EyeHandle) -> Self { self.reflex = Some((judge, eye)); self }`.

5. No `select!` do `run`:

```rust
                Some(outcome) = self.reflex_rx.recv() => self.on_reflex(outcome),
```

6. Em `ServerEvent::UserText`, **antes** de `hear_always_allow`:

```rust
                if let Some(reflex) = &self.reflex {
                    reflex.hear(self.user_heard.clone(), !self.confirms.is_empty());
                }
```

7. Em `ServerEvent::TurnComplete` (onde já chama `end_model_turn`):

```rust
                if let Some(reflex) = &self.reflex { reflex.end_turn(); }
```

8. Novo método:

```rust
    /// Decisão do reflexo: age, confirma ou libera, sempre pelos caminhos que
    /// já existem para o modelo.
    fn on_reflex(&mut self, outcome: crate::reflex::Outcome) {
        use crate::reflex::Decision;
        match outcome.decision {
            Decision::Act(call) => {
                if !crate::reflex::decide::is_reflex_tool(&call.name) || self.registry.get(&call.name).is_none() {
                    warn!(ferramenta = %call.name, "reflexo pediu ferramenta fora da lista; ignorado");
                    return;
                }
                info!(ferramenta = %call.name, ms = outcome.latency_ms, "reflexo agiu");
                if let Some(r) = self.recorder.as_mut() {
                    r.event("reflex_act", format!("{} {} {}ms", call.id, engine_tools::call_summary(&call), outcome.latency_ms));
                }
                self.recently_done.retain(|(_, _, at, _)| at.elapsed() < engine_tools::REFLEX_DONE_WINDOW);
                self.recently_done.push((call.name.clone(), call.args.clone(), Instant::now(), serde_json::json!({"status": "executada pelo reflexo"})));
                self.emit.send(EngineEvent::ReflexActed { call: call.clone(), latency_ms: outcome.latency_ms, confidence: outcome.confidence });
                self.emit.send(EngineEvent::ToolRequested { call: call.clone(), risk: Risk::Safe });
                if let Some(session) = self.session.as_ref() {
                    session.send_text(&engine_tools::reflex_context_text(&call));
                }
                self.execute(call);
            }
            Decision::Approve | Decision::Deny => {
                let approve = matches!(outcome.decision, Decision::Approve);
                let ids: Vec<String> = self.confirms.iter().map(|p| p.call.id.clone()).collect();
                if ids.is_empty() { return; }
                self.emit.send(EngineEvent::ReflexConfirmed { approve, confidence: outcome.confidence });
                for id in ids {
                    self.resolve_confirm(&id, approve, "voz, reflexo");
                }
                self.user_heard.clear();
            }
            Decision::AlwaysAllow => {
                // reaproveita o caminho de voz: força a checagem com a fala atual
                if !self.hear_always_allow() {
                    // a lista fixa não reconheceu a frase; libera as pendentes mesmo assim
                    let pending: Vec<(String, String)> = self.confirms.iter().map(|p| (p.call.id.clone(), p.call.name.clone())).collect();
                    let mut changed = false;
                    for (_, name) in &pending { changed |= self.decisions.allow_always(name); }
                    if changed { self.persist_always_allow(); }
                    for (id, _) in pending { self.resolve_confirm(&id, true, "voz, reflexo sempre pode"); }
                }
            }
            Decision::Nothing => {}
        }
    }
```

   `session.send_text(..)`: veja como `greeting` é enviado como turno de texto
   no início (`EngineConfig.greeting`); há um método na `Session`/`LiveSession`
   para mandar texto de cliente (`send_client_text` ou parecido). Use o mesmo
   método. Se ele encerra o turno do usuário (`turnComplete: true`), tudo bem:
   o modelo responde curto.

9. Dedup em `on_tool_call`, logo depois de resolver `spec` e antes de calcular
   `risk`:

```rust
        if let Some(pos) = self.recently_done.iter().position(|(name, args, at, _)| {
            name == &call.name && args == &call.args && at.elapsed() < engine_tools::REFLEX_DONE_WINDOW
        }) {
            let output = self.recently_done[pos].3.clone();
            info!(ferramenta = %call.name, "já executada pelo reflexo; não repete");
            if let Some(r) = self.recorder.as_mut() { r.event("tool_dedup_reflex", &call.id); }
            self.emit.send(EngineEvent::ToolRequested { call: call.clone(), risk: Risk::Safe });
            self.respond(&call.name, ToolResult::ok(&call.id, serde_json::json!({
                "status": "já executada",
                "nota": "o reflexo acabou de executar exatamente esta ação; use este resultado e não chame de novo",
                "resultado": output,
            })));
            return;
        }
```

10. Em `on_tool_finished`, se `done.result.id` começa com
    `crate::reflex::decide::REFLEX_CALL_PREFIX`: não enviar `send_tool_response`
    ao Gemini (ele não pediu essa chamada; um `functionResponse` com id
    desconhecido é erro de protocolo). Emitir só o `EngineEvent::ToolResult`.
    Se falhou (`error.is_some()`), remover a entrada de `recently_done` com o
    mesmo nome+args para o Gemini poder tentar. Implementação: separe
    `respond` em `respond` (atual) e `emit_result` (só evento + recorder), e
    escolha conforme o prefixo.

- [ ] **Step 5: Rodar todos os testes do core**

Run: `cargo test -p openjarvisbr-core`
Expected: todos passam, inclusive os dois novos.

- [ ] **Step 6: Clippy e commit**

Run: `cargo clippy -p openjarvisbr-core -- -D warnings`

```bash
git add crates/core/src/engine.rs crates/core/src/engine_tools.rs
git commit -m "feat(reflex): engine age pelo reflexo, deduplica e confirma por voz [JRV]"
```

---

## Card F — CLI `jarvis reflex` (onda 1)

### Task 15: Subcomando de diagnóstico

**Files:**
- Modify: `crates/cli/src/main.rs`

**Interfaces:**
- Consumes: `config::load_reflex`, `Eye::start`, `JevClient`, `build_questions`, `decide`, `Thresholds`.

- [ ] **Step 1: Adicionar subcomando ao clap**

O `Cli` hoje é só flags. Adicione um `#[command(subcommand)] command: Option<Sub>`
e:

```rust
#[derive(clap::Subcommand, Debug)]
enum Sub {
    /// Diagnóstico do reflexo (Jev): pergunta com uma frase ou lista o inventário
    Reflex {
        /// Frase como se fosse a transcrição do usuário
        phrase: Option<String>,
        /// Só imprime o inventário do Olho (apps rodando, instalados, sites)
        #[arg(long)]
        eye: bool,
        /// Simula confirmação pendente (testa "sim"/"não")
        #[arg(long)]
        pending: bool,
    },
}
```

No começo de `main`, antes do lock de instância única (o diagnóstico não abre
microfone):

```rust
    if let Some(Sub::Reflex { phrase, eye, pending }) = cli.command {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        std::process::exit(runtime.block_on(reflex_diagnostic(phrase, eye, pending)));
    }
```

- [ ] **Step 2: Implementar `reflex_diagnostic`**

```rust
async fn reflex_diagnostic(phrase: Option<String>, only_eye: bool, pending: bool) -> i32 {
    use openjarvisbr_core::reflex::decide::{build_questions, decide, Situation, Thresholds};
    use openjarvisbr_core::reflex::eye::Eye;
    use openjarvisbr_core::reflex::judge::{JevClient, Judge};

    let settings = config::load_reflex();
    let eye = Eye::start(settings.sites.clone());
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let inv = eye.snapshot();
    if only_eye || phrase.is_none() {
        println!("apps rodando ({}): {}", inv.running_apps.len(), inv.running_apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", "));
        println!("apps instalados: {}", inv.installed_apps.len());
        println!("sites ({}): {}", inv.sites.len(), inv.sites.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "));
        println!("reflexo: {}", if settings.enabled { "ligado" } else { "desligado (sem chave ou enabled = false)" });
        return 0;
    }
    let phrase = phrase.unwrap();
    let situation = Situation { heard: &phrase, inventory: &inv, pending_confirm: pending, turn_locked: false };
    let Some(questions) = build_questions(&situation) else {
        println!("nada a perguntar para essa frase");
        return 0;
    };
    println!("perguntas ({}): {}", questions.len(), questions.0.keys().cloned().collect::<Vec<_>>().join(", "));
    if !settings.enabled {
        eprintln!("reflexo desligado: configure typesafe_api_key no config.toml ou TYPESAFE_API_KEY");
        return 2;
    }
    let client = match JevClient::new(settings.api_key.clone().unwrap_or_default(), settings.model.clone()) {
        Ok(c) => c,
        Err(err) => { eprintln!("{err}"); return 2; }
    };
    let t0 = std::time::Instant::now();
    let answers = match client.ask(&phrase, &questions).await {
        Ok(a) => a,
        Err(err) => { eprintln!("erro do Jev: {err}"); return 3; }
    };
    let ms = t0.elapsed().as_millis();
    for (id, _) in &questions.0 {
        if let Some((pick, conf)) = answers.choice(id) {
            println!("  {id:<8} → {pick} ({conf:.2})");
        } else if let Some(v) = answers.noul(id) {
            println!("  {id:<8} → {v:.2}");
        }
    }
    let thresholds = Thresholds { act: settings.act_threshold, confirm: settings.confirm_threshold };
    println!("decisão: {:?}", decide(&situation, &answers, &thresholds));
    println!("latência: {ms} ms · tokens {}+{}", answers.usage.input_tokens, answers.usage.output_tokens);
    0
}
```

- [ ] **Step 3: Compilar e rodar sem chave**

Run: `cargo run -p openjarvisbr-cli -- reflex --eye`
Expected: imprime apps rodando, instalados, sites e "reflexo: desligado" (se
não houver chave). Confira o nome do pacote do CLI em `crates/cli/Cargo.toml`.

Run: `cargo run -p openjarvisbr-cli -- reflex "abre o safari"`
Expected: lista as perguntas e sai com código 2 avisando que falta a chave.

- [ ] **Step 4: Commit**

```bash
git add crates/cli/src/main.rs
git commit -m "feat(reflex): jarvis reflex para diagnosticar perguntas, decisão e latência [JRV]"
```

---

## Card G — Desktop (G1 na onda 1, G2 na onda 2)

### Task 16: Flash "⚡ reflexo" no overlay (G1)

**Files:**
- Modify: `apps/desktop/src/overlay/tools.ts`
- Modify: `apps/desktop/overlay.html` (CSS)

**Interfaces:**
- Consumes (contrato, emitido na Task 18): evento Tauri `engine://reflex` com payload `{ kind: "acted" | "confirmed", name?: string, summary?: string, latency_ms?: number, approve?: boolean }`.

- [ ] **Step 1: Tipos e listener**

Em `tools.ts` adicionar ao lado de `ToolEventPayload`:

```ts
export interface ReflexEventPayload {
    kind: "acted" | "confirmed";
    name?: string;
    summary?: string;
    latency_ms?: number;
    approve?: boolean;
}
```

Em `initialize()`, além do listener existente:

```ts
        this.unlistenReflex = await listen<ReflexEventPayload>("engine://reflex", (event) => this.handleReflex(event.payload));
```

Novo método (mostra por 2,5 s sem botões, tom `reflex`):

```ts
    private handleReflex(payload: ReflexEventPayload) {
        const text = payload.kind === "acted"
            ? `⚡ reflexo · ${payload.summary ?? payload.name ?? ""} · ${payload.latency_ms ?? 0} ms`
            : `⚡ reflexo · ${payload.approve ? "aprovado" : "negado"} por voz`;
        this.host.show();
        this.container.classList.add("tool-active");
        this.strip.dataset.mode = "result";
        this.strip.dataset.tone = "reflex";
        this.textEl.textContent = text;
        this.confirmButton.hidden = true;
        this.denyButton.hidden = true;
        this.mode = "result";
        this.schedule(() => this.reset(), 2500);
    }
```

Use os nomes reais dos helpers já existentes na classe para agendar o
auto-hide e resetar (`schedule`/`reset` são os nomes prováveis; confira no
arquivo e reutilize). Em `dispose()`, chame `unlistenReflex` também.

- [ ] **Step 2: CSS em overlay.html**

Ao lado de `.tool-strip[data-tone="ok"]`:

```css
        .tool-strip[data-tone="reflex"] { border-color: rgba(250, 204, 21, 0.6); }
        .tool-strip[data-tone="reflex"] .tool-text { color: #facc15; }
```

- [ ] **Step 3: Verificar tipos**

Run: `cd apps/desktop && npx tsc --noEmit -p tsconfig.json`
Expected: sem erros.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src/overlay/tools.ts apps/desktop/overlay.html
git commit -m "feat(reflex): overlay mostra flash do reflexo [JRV]"
```

### Task 17: Aba Reflexo nas Configurações (G1, parte visual)

**Files:**
- Modify: `apps/desktop/settings.html`
- Modify: `apps/desktop/src/settings/main.ts`

**Interfaces:**
- Consumes (contrato, Task 18): comandos Tauri `get_reflex_settings() -> { enabled: boolean, has_key: boolean, act_threshold: number, sites: {name,url}[] }`, `set_reflex_enabled(on: boolean)`, `set_typesafe_api_key(key: string)`, `save_reflex_sites(sites: {name,url}[])`.

- [ ] **Step 1: HTML**

Adicionar a aba ao lado de `data-tab="tools"`:

```html
        <button type="button" role="tab" data-tab="reflex">Reflexo</button>
```

E a seção, depois de `<section id="tab-tools">…</section>`:

```html
      <section id="tab-reflex" role="tabpanel">
        <h2>Reflexo (Jev)</h2>
        <p class="hint">Ações seguras em ~300 ms: abrir app, abrir site da lista, mídia e volume,
          além de "sim/não" por voz. A transcrição do que você fala e os nomes dos apps são
          enviados à TypeSafe.ai enquanto o reflexo estiver ligado.</p>
        <label class="row"><input type="checkbox" id="reflex-enabled"> Reflexo ligado</label>
        <label class="row">Chave da TypeSafe
          <input type="password" id="reflex-key" placeholder="cole a chave (fica mascarada)" autocomplete="off">
          <span id="reflex-key-status" class="hint"></span>
        </label>
        <label class="row">Confiança mínima para agir
          <input type="range" id="reflex-threshold" min="0.5" max="1" step="0.01">
          <span id="reflex-threshold-value"></span>
        </label>
        <h3>Sites que o reflexo pode abrir</h3>
        <ul id="reflex-sites"></ul>
        <div class="row">
          <input id="reflex-site-name" placeholder="Nome (ex.: YouTube)">
          <input id="reflex-site-url" placeholder="https://…">
          <button type="button" id="reflex-site-add">Adicionar</button>
        </div>
        <button type="button" id="reflex-save">Salvar</button>
      </section>
```

Siga as classes e a estrutura da aba Ferramentas já existente para o visual
ficar igual. Em `selectTab`, adicione `document.body.classList.toggle("tab-reflex", tab === "reflex")`
se o CSS das abas depender de classe no body.

- [ ] **Step 2: TypeScript**

Em `main.ts`:

```ts
interface ReflexSettingsPayload {
  enabled: boolean;
  has_key: boolean;
  act_threshold: number;
  sites: { name: string; url: string }[];
}

let reflexSites: { name: string; url: string }[] = [];

function renderReflexSites() {
  const list = document.getElementById("reflex-sites") as HTMLUListElement;
  list.innerHTML = "";
  reflexSites.forEach((site, i) => {
    const li = document.createElement("li");
    li.textContent = `${site.name} → ${site.url} `;
    const rm = document.createElement("button");
    rm.type = "button"; rm.textContent = "remover";
    rm.addEventListener("click", () => { reflexSites.splice(i, 1); renderReflexSites(); });
    li.appendChild(rm);
    list.appendChild(li);
  });
}

async function loadReflex() {
  const r = await invoke<ReflexSettingsPayload>("get_reflex_settings");
  (document.getElementById("reflex-enabled") as HTMLInputElement).checked = r.enabled;
  (document.getElementById("reflex-key-status") as HTMLElement).textContent = r.has_key ? "chave salva (***)" : "sem chave";
  const slider = document.getElementById("reflex-threshold") as HTMLInputElement;
  slider.value = String(r.act_threshold);
  (document.getElementById("reflex-threshold-value") as HTMLElement).textContent = r.act_threshold.toFixed(2);
  reflexSites = r.sites;
  renderReflexSites();
}

document.getElementById("reflex-threshold")?.addEventListener("input", (e) => {
  (document.getElementById("reflex-threshold-value") as HTMLElement).textContent = Number((e.target as HTMLInputElement).value).toFixed(2);
});

document.getElementById("reflex-site-add")?.addEventListener("click", () => {
  const name = (document.getElementById("reflex-site-name") as HTMLInputElement).value.trim();
  const url = (document.getElementById("reflex-site-url") as HTMLInputElement).value.trim();
  if (!name || !/^https?:\/\//.test(url)) { setStatus("informe nome e URL começando com http(s)://", "error"); return; }
  reflexSites.push({ name, url });
  (document.getElementById("reflex-site-name") as HTMLInputElement).value = "";
  (document.getElementById("reflex-site-url") as HTMLInputElement).value = "";
  renderReflexSites();
});

document.getElementById("reflex-save")?.addEventListener("click", async () => {
  try {
    const key = (document.getElementById("reflex-key") as HTMLInputElement).value;
    if (key.trim()) {
      await invoke("set_typesafe_api_key", { key });
      (document.getElementById("reflex-key") as HTMLInputElement).value = "";
    }
    await invoke("save_reflex_sites", { sites: reflexSites, actThreshold: Number((document.getElementById("reflex-threshold") as HTMLInputElement).value) });
    await invoke("set_reflex_enabled", { on: (document.getElementById("reflex-enabled") as HTMLInputElement).checked });
    await loadReflex();
    setStatus("reflexo salvo. Reinicie a conversa para aplicar.", "ok");
  } catch (err) {
    setStatus(String(err), "error");
  }
});

void loadReflex().catch(() => { /* comandos chegam na Task 18 */ });
```

Reaproveite `setStatus` e `invoke` já importados no arquivo.

- [ ] **Step 3: Tipos**

Run: `cd apps/desktop && npx tsc --noEmit -p tsconfig.json`
Expected: sem erros.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/settings.html apps/desktop/src/settings/main.ts
git commit -m "feat(reflex): aba Reflexo nas configurações (visual) [JRV]"
```

### Task 18: Fiação Tauri (G2, depois de E)

**Files:**
- Modify: `apps/desktop/src-tauri/src/tool_events.rs`
- Modify: `apps/desktop/src-tauri/src/main.rs`
- Modify: `crates/core/src/config.rs` (só `save_reflex_sites`)

- [ ] **Step 1: Evento `engine://reflex`**

Em `tool_events.rs`, ao lado de `emit`:

```rust
pub(crate) const REFLEX_EVENT: &str = "engine://reflex";

#[derive(serde::Serialize, Clone)]
struct ReflexPayload<'a> {
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approve: Option<bool>,
}

pub(crate) fn emit_reflex(app: &AppHandle, event: &EngineEvent) {
    let payload = match event {
        EngineEvent::ReflexActed { call, latency_ms, .. } => ReflexPayload {
            kind: "acted",
            name: Some(&call.name),
            summary: Some(openjarvisbr_core::engine_tools::call_summary(call)),
            latency_ms: Some(*latency_ms),
            approve: None,
        },
        EngineEvent::ReflexConfirmed { approve, .. } => ReflexPayload {
            kind: "confirmed", name: None, summary: None, latency_ms: None, approve: Some(*approve),
        },
        _ => return,
    };
    let _ = app.emit(REFLEX_EVENT, payload);
}
```

Em `main.rs`, no `match` de eventos (perto da linha 400):

```rust
        event @ (EngineEvent::ReflexActed { .. } | EngineEvent::ReflexConfirmed { .. }) => tool_events::emit_reflex(app, &event),
```

- [ ] **Step 2: `EngineConfig.reflex` no desktop**

Onde o desktop monta `EngineConfig` (busque `always_allow: settings.always_allow` em `main.rs`), adicione `reflex: settings.reflex.clone()`.

- [ ] **Step 3: Comandos Tauri**

```rust
#[derive(serde::Serialize)]
struct ReflexSettingsPayload {
    enabled: bool,
    has_key: bool,
    act_threshold: f32,
    sites: Vec<openjarvisbr_core::config::SiteConfig>,
}

#[tauri::command]
fn get_reflex_settings() -> ReflexSettingsPayload {
    let r = openjarvisbr_core::config::load_reflex();
    ReflexSettingsPayload { enabled: r.enabled, has_key: r.api_key.is_some(), act_threshold: r.act_threshold, sites: r.sites }
}

#[tauri::command]
fn set_reflex_enabled(on: bool) -> Result<(), String> {
    openjarvisbr_core::config::save_reflex_enabled(on).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_typesafe_api_key(key: String) -> Result<(), String> {
    openjarvisbr_core::config::save_typesafe_api_key(&key).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_reflex_sites(sites: Vec<openjarvisbr_core::config::SiteConfig>, act_threshold: f32) -> Result<(), String> {
    openjarvisbr_core::config::save_reflex_sites(&sites, act_threshold).map_err(|e| e.to_string())
}
```

Registre os quatro no `generate_handler![...]`. Em `config.rs`:

```rust
/// Grava `[[reflex.sites]]` e `[reflex].act_threshold`.
pub fn save_reflex_sites(sites: &[SiteConfig], act_threshold: f32) -> Result<(), ConfigError> {
    let path = config_path().ok_or(ConfigError::MissingKey)?;
    let mut cfg = read_file_config(&path);
    let section = cfg.reflex.get_or_insert_with(Default::default);
    section.sites = sites.to_vec();
    section.act_threshold = Some(act_threshold.clamp(0.5, 1.0));
    write_file_config(&path, &cfg)
}
```

- [ ] **Step 4: Build do desktop**

Run: `cd apps/desktop && npm run tauri build -- --debug` (ou o comando de build documentado no README)
Expected: compila. Abra Configurações › Reflexo, cole uma chave, salve, e veja "chave salva (***)".

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src-tauri/src/tool_events.rs apps/desktop/src-tauri/src/main.rs crates/core/src/config.rs
git commit -m "feat(reflex): desktop emite engine://reflex e salva config do reflexo [JRV]"
```

---

## Card H — Roteiro de testes e README (onda 3)

### Task 19: `docs/testes/v4.md` e README

**Files:**
- Create: `docs/testes/v4.md`
- Modify: `README.md`

- [ ] **Step 1: Roteiro leigo**

Siga o formato de `docs/testes/v3.md` (mesma introdução, "Antes de começar",
atividades numeradas com frase entre crases e resultado esperado). Atividades:

1. **Reflexo desligado se comporta como a v3.** Sem chave, `Abre o Safari` abre pelo Gemini, sem flash amarelo no overlay.
2. **Ligar o reflexo.** Configurações › Reflexo: colar chave, marcar ligado, salvar; status mostra "chave salva (***)". Reiniciar a conversa.
3. **Abrir app na velocidade do reflexo.** `Abre o Spotify`: o app abre antes de o Jarvis terminar de falar; overlay mostra "⚡ reflexo · Spotify · NNN ms" com NNN abaixo de 600; o Jarvis não abre de novo nem diz "vou abrir".
4. **Mídia.** Com música tocando, `próxima música` pula na hora.
5. **Volume.** `aumenta o volume` sobe; `coloca o volume em 20` vai para 20.
6. **Site da lista.** Adicionar YouTube na aba; `abre o youtube` abre. `abre o site da globo` (fora da lista) NÃO abre pelo reflexo (pode abrir pelo Gemini, mais devagar).
7. **Confirmação por voz fora das listas.** Pedir `roda ls na pasta atual`; quando perguntar, responder `bora, toca ficha`: executa, overlay mostra "⚡ reflexo · aprovado por voz". Repetir com `deixa pra lá`: nega.
8. **Diagnóstico.** No terminal, `jarvis reflex "abre o safari"` imprime as probabilidades, a decisão e a latência.
9. **Chave errada.** Colocar uma chave inválida: nada quebra, o Jarvis segue pelo Gemini, e o log tem "chave da TypeSafe inválida".

- [ ] **Step 2: README**

Seção "Reflexo (Jev)" curta: o que é, como ligar (chave, `[reflex]`, sites),
aviso de privacidade (transcrição e nomes de apps vão para a TypeSafe.ai), e o
comando `jarvis reflex`.

- [ ] **Step 3: Commit**

```bash
git add docs/testes/v4.md README.md
git commit -m "docs: roteiro de testes v4 e README do reflexo [JRV]"
```

---

## Self-review (feito ao escrever)

- **Cobertura do spec:** contratos (Tasks 1, 2, 5, 6, 8–12), perguntas ao Jev (9), regras de decisão e limiares (10), tools fixas (11), latência/debounce/cancelamento/trava (12), config e chave (4), dedup + contexto ao Gemini + confirmação por voz + eventos (14), CLI (15), overlay e aba (16–18), degradação por 401/429/529/timeout (12, 3), segurança de `web.open` só por lista (10), roteiro e README (19), teste live opcional (3).
- **Lacuna resolvida:** `sys.volume` não tinha `up`/`down`; a Task 10 adiciona em `volume.rs` dentro do card C (arquivo passa a ser exclusivo de C na onda 1).
- **Nomes consistentes:** `Situation`, `Thresholds`, `Decision`, `Outcome`, `EyeHandle::snapshot`, `FakeJudge::push/push_err/calls`, `Answers::choice/probability/noul`, `REFLEX_CALL_PREFIX`, `is_reflex_tool`, `ReflexSettings`, `SiteConfig`, `engine://reflex`.
- **Ponto de atenção para E:** o id `reflex-N` nunca vai ao Gemini como `functionResponse` (item 10 da Task 14). Se `Session` não tiver `send_text`, o executor cria um a partir do caminho do `greeting`.
