//! Quem responde às perguntas: o Jev de verdade (HTTP) ou um fake de teste.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use thiserror::Error;

use super::questions::{Answers, Questions, Request};

pub const JEV_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
pub const JEV_TIMEOUT: Duration = Duration::from_millis(800);

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
        Self {
            delay,
            ..Self::default()
        }
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
        self.calls
            .lock()
            .unwrap()
            .push((state.to_string(), questions.clone()));
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(JudgeError::Disabled))
    }
}

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
        // Mesmo provider rustls (ring) da sessão Live e do MCP; falha só se
        // já houver um instalado.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .connect_timeout(JEV_TIMEOUT)
            .timeout(JEV_TIMEOUT)
            .build()
            .map_err(|e| JudgeError::Http(e.to_string()))?;
        Ok(Self {
            http,
            api_key,
            model,
            url: JEV_ENDPOINT.to_string(),
        })
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
        // `reqwest` no workspace não tem a feature `json`: serializa à mão,
        // como o transporte MCP.
        let payload = serde_json::to_vec(&body).map_err(|e| JudgeError::Parse(e.to_string()))?;
        let mut auth = HeaderValue::from_str(&format!("Bearer {}", self.api_key))
            .map_err(|_| JudgeError::Unauthorized)?;
        auth.set_sensitive(true);
        let response = self
            .http
            .post(&self.url)
            .header(AUTHORIZATION, auth)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(payload)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    JudgeError::Timeout
                } else {
                    JudgeError::Http(e.to_string())
                }
            })?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .await
            .map_err(|e| JudgeError::Http(e.to_string()))?;
        classify(status, &text)
    }
}

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

    #[test]
    fn classifica_status_http() {
        let ok = include_str!("../../tests/fixtures/jev_response.json");
        let a = classify(200, ok).unwrap();
        assert_eq!(a.choice("intent"), Some(("open_app", 0.93)));
        assert!(matches!(classify(401, "{}"), Err(JudgeError::Unauthorized)));
        assert!(matches!(classify(429, "{}"), Err(JudgeError::RateLimited)));
        assert!(matches!(classify(529, "{}"), Err(JudgeError::Overloaded)));
        assert!(matches!(classify(500, "boom"), Err(JudgeError::Http(m)) if m.contains("500")));
        assert!(matches!(
            classify(200, "não é json"),
            Err(JudgeError::Parse(_))
        ));
    }

    #[test]
    fn debug_do_cliente_nao_vaza_chave() {
        let c = JevClient::new("segredo-123".into(), "jev-latest".into()).unwrap();
        let s = format!("{c:?}");
        assert!(!s.contains("segredo-123"));
        assert!(s.contains("***"));
    }
}
