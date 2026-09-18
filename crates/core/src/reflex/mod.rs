//! Reflexo: decisões rápidas (~300 ms) com o Jev (TypeSafe.ai) na frente do
//! Gemini Live. Spec: docs/superpowers/specs/2026-09-18-openjarvisbr-v4-reflexo-jev-design.md
//!
//! Regra de ouro: Gemini pensa, Jev julga. O reflexo só executa ações que hoje
//! já não pedem confirmação e só escolhe entre opções listadas.

pub mod decide;
pub mod eye;
pub mod judge;
pub mod questions;

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
use questions::Answers;

/// Uma decisão pronta para o engine, com telemetria.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub decision: Decision,
    pub latency_ms: u32,
    pub confidence: f32,
}

/// Pergunta em voo há menos que isto é cancelada quando chega fragmento novo.
const CANCEL_IF_YOUNGER_THAN: Duration = Duration::from_millis(200);
/// Erros seguidos de cota/sobrecarga (429/529) até pausar.
const BACKOFF_AFTER: u32 = 3;
const BACKOFF_FOR: Duration = Duration::from_secs(30);

/// Debounce pendente ou pergunta em voo. `started` é o instante em que o
/// HTTP começa (fim do debounce); durante o debounce `elapsed()` é zero.
struct InFlight {
    started: Instant,
    task: JoinHandle<()>,
}

/// Orquestrador do reflexo: junta fragmentos (debounce), pergunta ao juiz,
/// emite no máximo uma `Decision::Act` por turno e recua em erro de cota.
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
    pending: Mutex<Option<InFlight>>,
}

impl std::fmt::Debug for Reflex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reflex")
            .field("settings", &self.settings)
            .field("locked", &self.locked.load(Ordering::Relaxed))
            .field("dead", &self.dead.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Reflex {
    pub fn new(
        settings: ReflexSettings,
        judge: Arc<dyn Judge>,
        eye: EyeHandle,
    ) -> (Self, mpsc::UnboundedReceiver<Outcome>) {
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
            pending: Mutex::new(None),
        };
        (reflex, rx)
    }

    /// Monta o reflexo real a partir do config. `None` = desligado ou sem chave.
    pub fn from_settings(settings: ReflexSettings) -> Option<(Self, mpsc::UnboundedReceiver<Outcome>)> {
        if !settings.enabled {
            return None;
        }
        let key = settings.api_key.clone()?;
        let client = judge::JevClient::new(key, settings.model.clone()).ok()?;
        let eye = eye::Eye::start(settings.sites.clone());
        Some(Self::new(settings, Arc::new(client), eye))
    }

    /// Ligado no config e não desligado por chave inválida.
    pub fn enabled(&self) -> bool {
        self.settings.enabled && !self.dead.load(Ordering::Relaxed)
    }

    /// O Olho, para o engine pedir refresh (ex.: depois de abrir um app).
    pub fn eye(&self) -> &EyeHandle {
        &self.eye
    }

    /// Fragmento novo da fala do usuário (acumulada no turno).
    pub fn hear(&self, heard: String, pending_confirm: bool) {
        if !self.enabled() {
            return;
        }
        if let Some(until) = *self.paused_until.lock().unwrap_or_else(|e| e.into_inner()) {
            if Instant::now() < until {
                return;
            }
        }
        {
            let mut slot = self.pending.lock().unwrap_or_else(|e| e.into_inner());
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
        let thresholds = Thresholds {
            act: self.settings.act_threshold,
            confirm: self.settings.confirm_threshold,
        };
        let debounce = Duration::from_millis(self.settings.debounce_ms);
        let task = tokio::spawn(async move {
            tokio::time::sleep(debounce).await;
            let inventory = eye.snapshot();
            let turn_locked = locked.load(Ordering::Relaxed);
            let situation = Situation {
                heard: &heard,
                inventory: &inventory,
                pending_confirm,
                turn_locked,
            };
            let Some(questions) = build_questions(&situation) else {
                return;
            };
            let t0 = Instant::now();
            match judge.ask(&heard, &questions).await {
                Ok(answers) => {
                    quota.store(0, Ordering::Relaxed);
                    let latency_ms = t0.elapsed().as_millis() as u32;
                    let decision = decide(&situation, &answers, &thresholds);
                    let confidence = confidence_of(&decision, &answers);
                    debug!(?decision, latency_ms, "reflexo");
                    if matches!(decision, Decision::Nothing) {
                        return;
                    }
                    // corrida: outro fragmento pode ter agido no meio
                    if matches!(decision, Decision::Act(_)) && locked.swap(true, Ordering::SeqCst) {
                        return;
                    }
                    let _ = tx.send(Outcome {
                        decision,
                        latency_ms,
                        confidence,
                    });
                }
                Err(JudgeError::Unauthorized) => {
                    warn!("chave da TypeSafe inválida; reflexo desligado até reiniciar");
                    dead.store(true, Ordering::Relaxed);
                }
                Err(err @ (JudgeError::RateLimited | JudgeError::Overloaded)) => {
                    let n = quota.fetch_add(1, Ordering::Relaxed) + 1;
                    if n >= BACKOFF_AFTER {
                        info!(%err, "reflexo pausado por {}s", BACKOFF_FOR.as_secs());
                        *paused.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(Instant::now() + BACKOFF_FOR);
                        quota.store(0, Ordering::Relaxed);
                    }
                }
                Err(err) => debug!(%err, "reflexo pulou esta rodada"),
            }
        });
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(InFlight {
            started: Instant::now() + debounce,
            task,
        });
    }

    /// Fim do turno do modelo: o reflexo pode agir de novo.
    pub fn end_turn(&self) {
        self.locked.store(false, Ordering::SeqCst);
    }
}

/// Confiança que sustenta a decisão, para telemetria.
fn confidence_of(decision: &Decision, answers: &Answers) -> f32 {
    let value = match decision {
        Decision::Act(_) => answers.choice("intent").map(|c| c.1),
        Decision::Approve => answers.noul("approve"),
        Decision::Deny => answers.noul("deny"),
        Decision::AlwaysAllow => answers.noul("always"),
        Decision::Nothing => None,
    };
    value.unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ReflexSettings, SiteConfig};
    use crate::reflex::decide::NONE;
    use crate::reflex::eye::{Eye, RunningProbe};
    use crate::reflex::judge::{FakeJudge, JudgeError};
    use crate::reflex::questions::{Answer, Answers};
    use std::collections::BTreeMap;
    use std::time::Duration;

    struct NoProbe;
    impl RunningProbe for NoProbe {
        fn running_app_names(&self) -> Vec<String> {
            vec![]
        }
    }

    fn eye() -> EyeHandle {
        let dir = std::env::temp_dir().join(format!("reflex-eye-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Safari.app")).unwrap();
        Eye::start_with(
            vec![dir],
            vec![SiteConfig { name: "YouTube".into(), url: "https://youtube.com".into() }],
            Arc::new(NoProbe),
            Duration::from_secs(3600),
            Duration::from_secs(3600),
        )
    }

    fn settings() -> ReflexSettings {
        ReflexSettings { enabled: true, api_key: Some("x".into()), debounce_ms: 30, ..Default::default() }
    }

    fn open_safari() -> Answers {
        let mut a = Answers::default();
        for (id, pick) in [("intent", "open_app"), ("app", "Safari")] {
            let mut p = BTreeMap::new();
            p.insert(pick.to_string(), 0.95);
            p.insert(NONE.to_string(), 0.05);
            a.answers.insert(
                id.into(),
                Answer::Choice { choice: pick.into(), probabilities: p, confidence: 0.95 },
            );
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
        let mut s = settings();
        s.enabled = false;
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
        // debounce passou, pergunta em voo há ~30 ms (< 200 ms): deve ser cancelada
        tokio::time::sleep(Duration::from_millis(60)).await;
        reflex.hear("abre o safari".into(), false);
        let out = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.unwrap().unwrap();
        assert!(matches!(out.decision, Decision::Act(_)));
        // a primeira foi cancelada: só um Outcome
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn from_settings_sem_chave_da_none() {
        assert!(Reflex::from_settings(ReflexSettings::default()).is_none());
        // ligado, mas sem chave: continua desligado
        let sem_chave = ReflexSettings { enabled: true, ..Default::default() };
        assert!(Reflex::from_settings(sem_chave).is_none());
        // chave presente, mas [reflex].enabled = false: desligado
        let desligado = ReflexSettings { api_key: Some("k".into()), ..Default::default() };
        assert!(Reflex::from_settings(desligado).is_none());
        let s = ReflexSettings { enabled: true, api_key: Some("segredo-xyz".into()), ..Default::default() };
        let (reflex, _rx) = Reflex::from_settings(s).unwrap();
        assert!(reflex.enabled());
        assert!(!format!("{reflex:?}").contains("segredo-xyz"), "Debug não pode vazar a chave");
    }
}
