//! Quem responde às perguntas: o Jev de verdade (HTTP) ou um fake de teste.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use super::questions::{Answers, Questions};

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
