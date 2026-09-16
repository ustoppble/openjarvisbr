//! Motor da OpenJarvisBR: conecta a sessão Live, liga o microfone à sessão e
//! a sessão ao playback, e publica tudo o que acontece como [`EngineEvent`].
//! Não escreve nada no terminal — quem consome os eventos (CLI, desktop)
//! decide como mostrar.
//!
//! O loop roda numa thread própria com runtime tokio de uma thread só: os
//! streams do `cpal` não são `Send` em todas as plataformas, e assim o
//! [`EngineHandle`] pode ser usado de qualquer runtime.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tracing::{info, warn};

use crate::audio::capture::{self, CaptureError, CaptureHandle};
use crate::audio::fx::VoiceFx;
use crate::audio::playback::{PlaybackError, Player};
use crate::live::protocol::ServerEvent;
use crate::live::session::{LiveConfig, LiveError, LiveSession};

/// Capacidade do canal entre a thread de captura e o loop async.
const CHANNEL_CAPACITY: usize = 64;
/// Capacidade do broadcast de eventos. `Level` sai 20x por segundo; um
/// consumidor que atrase mais que isso recebe `Lagged` e segue.
const EVENT_CAPACITY: usize = 512;
/// Folga depois do último áudio do modelo antes de reabrir o microfone no
/// modo caixa de som: cobre os vãos entre pacotes e a cauda do alto-falante.
const MIC_REOPEN_DELAY: Duration = Duration::from_millis(700);
/// Intervalo dos eventos `Level`.
const LEVEL_INTERVAL: Duration = Duration::from_millis(50);

/// Estado do assistente, na ordem de precedência (erro vence tudo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    Connecting,
    Listening,
    Speaking,
    Muted,
    Error,
}

/// Tudo o que o motor publica.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    /// Emitido só quando o estado muda.
    State(EngineState),
    /// Pedaço da transcrição da fala do usuário.
    UserText(String),
    /// Pedaço da transcrição da fala do modelo.
    ModelText(String),
    /// O turno do modelo acabou (fim normal ou interrupção por voz).
    TurnComplete,
    /// RMS normalizado 0..1 da janela de ~50ms: `mic` do que foi capturado
    /// (0 com mic mudo), `model` do áudio do modelo.
    Level { mic: f32, model: f32 },
    /// Reconexão em andamento (queda, goAway ou `reconnect()`).
    Reconnecting { attempt: u32 },
    /// A sessão caiu de vez. O motor segue vivo em `Error` esperando
    /// `reconnect()` ou `stop()`; o erro tipado fica em
    /// [`EngineHandle::last_error`].
    Error { kind: EngineErrorKind, message: String },
}

/// Categoria do erro, para quem consome os eventos (desktop, CLI) decidir a
/// UI da tabela "Tratamento de erros" da spec sem conhecer os tipos internos
/// de socket/áudio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineErrorKind {
    /// Chave inválida ou sem permissão (401).
    InvalidKey,
    /// Sem microfone, ou o configurado não existe mais.
    NoInputDevice,
    /// Sem saída de áudio, ou a configurada não existe mais.
    NoOutputDevice,
    /// Socket caiu e a reconexão automática esgotou as tentativas.
    Socket,
    /// Quota da API esgotada ou limite de taxa atingido (429).
    Quota,
    /// Qualquer outra falha (runtime, resampler, formato de amostra…).
    Other,
}

/// Parâmetros de uma sessão de conversa. `Debug` é manual para nunca
/// imprimir a chave.
#[derive(Clone)]
pub struct EngineConfig {
    pub api_key: String,
    pub voice: String,
    pub device_in: Option<String>,
    pub device_out: Option<String>,
    /// Mic aberto enquanto o modelo fala (interrupção por voz). Desligado por
    /// padrão para evitar eco com caixa de som.
    pub barge_in: bool,
    /// Pasta onde gravar playback.wav, mic.wav, events.log e raw.jsonl.
    pub record_dir: Option<PathBuf>,
    /// Instrução de sistema enviada no setup.
    pub system_prompt: String,
    /// Intensidade do efeito de voz (0 = desligado).
    pub fx_amount: f32,
    /// Texto pedido como turno de usuário logo após conectar (ex.: "se
    /// apresente no seu novo papel" ao trocar de perfil). `None` no início
    /// normal do app.
    pub greeting: Option<String>,
}

impl std::fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineConfig")
            .field("api_key", &"***")
            .field("voice", &self.voice)
            .field("device_in", &self.device_in)
            .field("device_out", &self.device_out)
            .field("barge_in", &self.barge_in)
            .field("record_dir", &self.record_dir)
            .field("fx_amount", &self.fx_amount)
            .field("greeting", &self.greeting)
            .finish()
    }
}

impl EngineConfig {
    fn live_config(&self) -> LiveConfig {
        let mut cfg = LiveConfig::new(self.api_key.clone(), self.voice.clone())
            .with_system_prompt(self.system_prompt.clone());
        if let Some(dir) = &self.record_dir {
            let _ = std::fs::create_dir_all(dir);
            cfg = cfg.with_raw_log(dir.join("raw.jsonl"));
        }
        if let Some(greeting) = &self.greeting {
            cfg = cfg.with_greeting(greeting.clone());
        }
        cfg
    }
}

/// Falhas ao subir o motor.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("{0}")]
    Connect(LiveError),
    #[error("{0}")]
    Capture(CaptureError),
    #[error("{0}")]
    Playback(PlaybackError),
    #[error("não foi possível iniciar o motor: {0}")]
    Runtime(String),
}

impl EngineError {
    /// Código de saída do processo, conforme a tabela de erros da spec
    /// (2 chave, 3 áudio, 4 socket, 5 quota).
    pub fn exit_code(&self) -> i32 {
        match self {
            EngineError::Connect(err) => err.exit_code(),
            EngineError::Capture(_) | EngineError::Playback(_) => 3,
            EngineError::Runtime(_) => 4,
        }
    }

    /// Categoria do erro, para a UI decidir o que mostrar.
    pub fn kind(&self) -> EngineErrorKind {
        match self {
            EngineError::Connect(err) => err.kind(),
            EngineError::Capture(err) => err.kind(),
            EngineError::Playback(err) => err.kind(),
            EngineError::Runtime(_) => EngineErrorKind::Other,
        }
    }
}

enum Command {
    Mute(bool),
    Reconnect,
    SetFxAmount(f32),
    Stop,
}

/// Controle de um motor em execução. Descartar o handle sem `stop()` também
/// encerra o motor (o canal de comandos fecha).
pub struct EngineHandle {
    commands: mpsc::UnboundedSender<Command>,
    events: broadcast::Sender<EngineEvent>,
    /// Receptor criado antes do primeiro evento: o primeiro `events()` o
    /// recebe e não perde `State(Connecting)`.
    first_events: Mutex<Option<broadcast::Receiver<EngineEvent>>>,
    last_error: Arc<Mutex<Option<LiveError>>>,
    finished: watch::Receiver<bool>,
}

impl EngineHandle {
    pub fn mute(&self, muted: bool) {
        let _ = self.commands.send(Command::Mute(muted));
    }

    /// Derruba a sessão atual e conecta de novo com a mesma config.
    pub fn reconnect(&self) {
        let _ = self.commands.send(Command::Reconnect);
    }

    /// Ajusta o efeito de voz ao vivo (0..1).
    pub fn set_fx_amount(&self, amount: f32) {
        let _ = self.commands.send(Command::SetFxAmount(amount));
    }

    /// Encerra o motor e espera microfone, gravação e sessão fecharem.
    pub async fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
        let mut finished = self.finished.clone();
        let _ = finished.wait_for(|done| *done).await;
    }

    /// Assina os eventos. O primeiro assinante recebe tudo desde o início.
    pub fn events(&self) -> broadcast::Receiver<EngineEvent> {
        self.first_events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_else(|| self.events.subscribe())
    }

    /// Erro que derrubou a sessão pela última vez, se houver.
    pub fn last_error(&self) -> Option<LiveError> {
        self.last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

pub struct Engine;

impl Engine {
    /// Conecta na Live API, abre microfone e saída e devolve o handle. Retorna
    /// erro se qualquer um dos três falhar.
    pub async fn start(config: EngineConfig) -> Result<EngineHandle, EngineError> {
        start_with(config, Backend::Real).await
    }
}

/// De onde vêm sessão e áudio: dispositivos reais, ou dublês nos testes.
enum Backend {
    Real,
    #[cfg(test)]
    Fake(fake::FakeBackend),
}

async fn start_with(config: EngineConfig, backend: Backend) -> Result<EngineHandle, EngineError> {
    let (commands, command_rx) = mpsc::unbounded_channel();
    let (events, first_rx) = broadcast::channel(EVENT_CAPACITY);
    let (ready_tx, ready_rx) = oneshot::channel();
    let (finished_tx, finished) = watch::channel(false);
    let last_error = Arc::new(Mutex::new(None));

    let thread_events = events.clone();
    let thread_error = Arc::clone(&last_error);
    std::thread::Builder::new()
        .name("jarvis-engine".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    let _ = ready_tx.send(Err(EngineError::Runtime(err.to_string())));
                    return;
                }
            };
            runtime.block_on(async move {
                let emit = Emitter(thread_events);
                match Worker::open(config, backend, command_rx, emit, thread_error).await {
                    Ok(worker) => {
                        let _ = ready_tx.send(Ok(()));
                        worker.run().await;
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                    }
                }
            });
            let _ = finished_tx.send(true);
        })
        .map_err(|err| EngineError::Runtime(err.to_string()))?;

    match ready_rx.await {
        Ok(Ok(())) => Ok(EngineHandle {
            commands,
            events,
            first_events: Mutex::new(Some(first_rx)),
            last_error,
            finished,
        }),
        Ok(Err(err)) => Err(err),
        Err(_) => Err(EngineError::Runtime("thread do motor morreu".to_string())),
    }
}

#[derive(Clone)]
struct Emitter(broadcast::Sender<EngineEvent>);

impl Emitter {
    fn send(&self, event: EngineEvent) {
        // Sem assinante não é erro: o evento só não tem quem leia.
        let _ = self.0.send(event);
    }
}

/// Sessão com o modelo: a Live API de verdade, ou um roteiro nos testes.
enum Session {
    Live(LiveSession),
    #[cfg(test)]
    Fake(fake::FakeSession),
}

impl Session {
    fn send_audio(&mut self, samples: &[i16]) {
        match self {
            Session::Live(session) => session.send_audio(samples),
            #[cfg(test)]
            Session::Fake(_) => {}
        }
    }

    async fn next_event(&mut self) -> Option<ServerEvent> {
        match self {
            Session::Live(session) => session.next_event().await,
            #[cfg(test)]
            Session::Fake(session) => session.events.recv().await,
        }
    }

    fn error(&self) -> Option<LiveError> {
        match self {
            Session::Live(session) => session.error(),
            #[cfg(test)]
            Session::Fake(_) => None,
        }
    }
}

/// Saída de áudio: alto-falante, ou nada nos testes.
enum Output {
    Device(Box<Player>),
    #[cfg(test)]
    Null,
}

impl Output {
    fn push(&self, samples: &[i16]) {
        match self {
            Output::Device(player) => player.push(samples),
            #[cfg(test)]
            Output::Null => {}
        }
    }

    fn is_playing(&self) -> bool {
        match self {
            Output::Device(player) => player.is_playing(),
            #[cfg(test)]
            Output::Null => false,
        }
    }

    fn queued(&self) -> usize {
        match self {
            Output::Device(player) => player.queued(),
            #[cfg(test)]
            Output::Null => 0,
        }
    }

    fn end_of_turn(&self) {
        match self {
            Output::Device(player) => player.end_of_turn(),
            #[cfg(test)]
            Output::Null => {}
        }
    }

    fn flush(&self) {
        match self {
            Output::Device(player) => player.flush(),
            #[cfg(test)]
            Output::Null => {}
        }
    }
}

/// Acumula quadrados das amostras de uma janela para o RMS.
#[derive(Default)]
struct Meter {
    sum_sq: f64,
    count: usize,
}

impl Meter {
    fn add(&mut self, samples: &[i16]) {
        for &s in samples {
            let v = s as f64 / 32768.0;
            self.sum_sq += v * v;
        }
        self.count += samples.len();
    }

    /// RMS 0..1 da janela, ou `None` se não chegou nada; zera a janela.
    fn take(&mut self) -> Option<f32> {
        let level =
            (self.count > 0).then(|| ((self.sum_sq / self.count as f64).sqrt() as f32).min(1.0));
        *self = Meter::default();
        level
    }
}

/// Resultado de esperar uma conexão enquanto comandos chegam.
enum Connected {
    Ok(Session),
    Failed(LiveError),
    Stopped,
}

struct Worker {
    config: EngineConfig,
    backend: Backend,
    commands: mpsc::UnboundedReceiver<Command>,
    emit: Emitter,
    last_error: Arc<Mutex<Option<LiveError>>>,
    session: Option<Session>,
    /// `None` depois que a captura fecha sozinha.
    mic: Option<mpsc::Receiver<Vec<i16>>>,
    _capture: Option<CaptureHandle>,
    output: Output,
    voice_fx: VoiceFx,
    recorder: Option<Recorder>,
    state: Option<EngineState>,
    muted: bool,
    speaking: bool,
    connecting: bool,
    failed: bool,
    last_model_audio: Option<Instant>,
    mic_meter: Meter,
    model_meter: Meter,
    model_level: f32,
}

impl Worker {
    /// Conecta e abre os dispositivos, na mesma ordem de antes: sessão,
    /// microfone, saída.
    async fn open(
        mut config: EngineConfig,
        mut backend: Backend,
        commands: mpsc::UnboundedReceiver<Command>,
        emit: Emitter,
        last_error: Arc<Mutex<Option<LiveError>>>,
    ) -> Result<Worker, EngineError> {
        emit.send(EngineEvent::State(EngineState::Connecting));
        info!("conectando à Live API");
        let session = connect(&config, &mut backend)
            .await
            .map_err(EngineError::Connect)?;
        // O `greeting` (ex.: "se apresente no novo papel") é só para a
        // primeira conexão; uma reconexão manual (`Worker::reconnect`) reusa
        // este `config` e não deve repeti-lo.
        config.greeting = None;

        let (mic, capture, output) = match &mut backend {
            Backend::Real => {
                let (std_tx, std_rx) = std_mpsc::channel::<Vec<i16>>();
                let capture = capture::start(config.device_in.as_deref(), std_tx)
                    .map_err(EngineError::Capture)?;
                let player =
                    Player::new(config.device_out.as_deref()).map_err(EngineError::Playback)?;
                (
                    bridge_capture_channel(std_rx),
                    Some(capture),
                    Output::Device(Box::new(player)),
                )
            }
            #[cfg(test)]
            Backend::Fake(fake) => (fake.take_mic(), None, Output::Null),
        };

        let recorder = config.record_dir.as_ref().and_then(|dir| match Recorder::open(dir) {
            Ok(recorder) => Some(recorder),
            Err(err) => {
                warn!(pasta = %dir.display(), erro = %err, "não foi possível abrir a gravação de diagnóstico");
                None
            }
        });

        Ok(Worker {
            voice_fx: VoiceFx::new(config.fx_amount),
            config,
            backend,
            commands,
            emit,
            last_error,
            session: Some(session),
            mic: Some(mic),
            _capture: capture,
            output,
            recorder,
            state: Some(EngineState::Connecting),
            muted: false,
            speaking: false,
            connecting: false,
            failed: false,
            last_model_audio: None,
            mic_meter: Meter::default(),
            model_meter: Meter::default(),
            model_level: 0.0,
        })
    }

    async fn run(mut self) {
        self.update_state();
        let mut ticker = tokio::time::interval(LEVEL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                chunk = next_chunk(&mut self.mic) => match chunk {
                    Some(samples) => self.on_mic(samples),
                    None => {
                        warn!("captura de áudio encerrada inesperadamente");
                        self.mic = None;
                    }
                },
                event = next_event(&mut self.session) => self.on_server_event(event),
                command = self.commands.recv() => match command {
                    Some(Command::Mute(muted)) => {
                        self.muted = muted;
                        self.update_state();
                    }
                    Some(Command::SetFxAmount(amount)) => self.voice_fx = VoiceFx::new(amount),
                    Some(Command::Reconnect) => {
                        if !self.reconnect().await {
                            break;
                        }
                    }
                    Some(Command::Stop) | None => {
                        info!("encerrando o motor");
                        break;
                    }
                },
                _ = ticker.tick() => self.on_tick(),
            }
        }

        if let Some(recorder) = self.recorder.take() {
            recorder.finish();
        }
        if let Some(capture) = &self._capture {
            capture.stop();
        }
    }

    fn on_mic(&mut self, samples: Vec<i16>) {
        // Half-duplex: sem fone, o mic capta a voz do próprio Jarvis e o
        // servidor a trata como fala do usuário. Só enviamos enquanto ele
        // está calado.
        let recently_spoke = self
            .last_model_audio
            .is_some_and(|t| t.elapsed() < MIC_REOPEN_DELAY);
        let gated = !self.config.barge_in && (self.output.is_playing() || recently_spoke);
        if self.muted {
            return;
        }
        self.mic_meter.add(&samples);
        if gated {
            return;
        }
        if let Some(session) = self.session.as_mut() {
            if let Some(r) = self.recorder.as_mut() {
                r.mic(&samples);
            }
            session.send_audio(&samples);
        }
    }

    fn on_server_event(&mut self, event: Option<ServerEvent>) {
        match event {
            Some(ServerEvent::Audio(mut samples)) => {
                self.last_model_audio = Some(Instant::now());
                if let Some(r) = self.recorder.as_mut() {
                    r.event(
                        "audio",
                        format!("{} amostras, fila={}", samples.len(), self.output.queued()),
                    );
                    r.playback(&samples);
                }
                self.model_meter.add(&samples);
                self.voice_fx.process(&mut samples);
                self.output.push(&samples);
                self.speaking = true;
                self.update_state();
            }
            Some(ServerEvent::Interrupted) => {
                if let Some(r) = self.recorder.as_mut() {
                    r.event(
                        "interrupted",
                        format!("fila descartada={}", self.output.queued()),
                    );
                }
                self.output.flush();
                self.last_model_audio = None;
                self.speaking = false;
                self.emit.send(EngineEvent::TurnComplete);
                self.update_state();
            }
            Some(ServerEvent::UserText(text)) => {
                if let Some(r) = self.recorder.as_mut() {
                    r.event("user_text", &text);
                }
                self.emit.send(EngineEvent::UserText(text));
            }
            Some(ServerEvent::ModelText(text)) => {
                if let Some(r) = self.recorder.as_mut() {
                    r.event("model_text", &text);
                }
                self.emit.send(EngineEvent::ModelText(text));
            }
            Some(ServerEvent::TurnComplete) => {
                if let Some(r) = self.recorder.as_mut() {
                    r.event("turn_complete", "");
                }
                self.output.end_of_turn();
                self.emit.send(EngineEvent::TurnComplete);
            }
            Some(ServerEvent::GoAway) => {
                if let Some(r) = self.recorder.as_mut() {
                    r.event("go_away", "reconectando");
                }
                info!("servidor pediu encerramento (goAway); reconectando");
            }
            // Execução de ferramentas chega com o card E (JRV-53 só traz o protocolo).
            Some(ServerEvent::ToolCall(_)) | Some(ServerEvent::ToolCallCancellation(_)) => {}
            Some(ServerEvent::Reconnecting(attempt)) => {
                self.connecting = true;
                self.update_state();
                self.emit.send(EngineEvent::Reconnecting { attempt });
            }
            Some(ServerEvent::Reconnected) => {
                self.connecting = false;
                self.update_state();
            }
            Some(ServerEvent::Closed) | None => {
                let err = self.session.take().and_then(|s| s.error());
                let kind = err.as_ref().map_or(EngineErrorKind::Socket, LiveError::kind);
                let message = match &err {
                    Some(err) => err.to_string(),
                    None => "conexão com a Live API encerrada".to_string(),
                };
                *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = err;
                self.fail(kind, message);
            }
        }
    }

    fn on_tick(&mut self) {
        // O áudio do modelo chega mais rápido que o tempo real; sem pacote
        // novo na janela, mantém o nível enquanto ainda há fila tocando.
        self.model_level = match self.model_meter.take() {
            Some(level) => level,
            None if self.output.is_playing() => self.model_level,
            None => 0.0,
        };
        let mic = self.mic_meter.take().unwrap_or(0.0);
        self.emit.send(EngineEvent::Level {
            mic,
            model: self.model_level,
        });

        if self.speaking
            && !self.output.is_playing()
            && self
                .last_model_audio
                .is_none_or(|t| t.elapsed() >= MIC_REOPEN_DELAY)
        {
            self.speaking = false;
            self.update_state();
        }
    }

    /// Reconexão pedida pelo handle. Retorna `false` se chegou `stop()`
    /// durante a espera.
    async fn reconnect(&mut self) -> bool {
        self.session = None;
        self.output.flush();
        self.speaking = false;
        self.failed = false;
        self.connecting = true;
        self.update_state();
        self.emit.send(EngineEvent::Reconnecting { attempt: 1 });

        let connected = {
            let connecting = connect(&self.config, &mut self.backend);
            tokio::pin!(connecting);
            loop {
                tokio::select! {
                    result = &mut connecting => break match result {
                        Ok(session) => Connected::Ok(session),
                        Err(err) => Connected::Failed(err),
                    },
                    command = self.commands.recv() => match command {
                        Some(Command::Mute(muted)) => self.muted = muted,
                        Some(Command::SetFxAmount(amount)) => self.voice_fx = VoiceFx::new(amount),
                        Some(Command::Reconnect) => {}
                        Some(Command::Stop) | None => break Connected::Stopped,
                    },
                }
            }
        };

        self.connecting = false;
        match connected {
            Connected::Ok(session) => {
                *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
                self.session = Some(session);
                self.update_state();
                true
            }
            Connected::Failed(err) => {
                let kind = err.kind();
                let message = err.to_string();
                *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(err);
                self.fail(kind, message);
                true
            }
            Connected::Stopped => false,
        }
    }

    fn fail(&mut self, kind: EngineErrorKind, message: String) {
        self.output.flush();
        self.speaking = false;
        self.connecting = false;
        self.failed = true;
        self.update_state();
        self.emit.send(EngineEvent::Error { kind, message });
    }

    /// Recalcula o estado a partir das flags e emite só se mudou.
    fn update_state(&mut self) {
        let state = if self.failed {
            EngineState::Error
        } else if self.connecting {
            EngineState::Connecting
        } else if self.speaking {
            EngineState::Speaking
        } else if self.muted {
            EngineState::Muted
        } else {
            EngineState::Listening
        };
        if self.state != Some(state) {
            self.state = Some(state);
            self.emit.send(EngineEvent::State(state));
        }
    }
}

/// Próximo chunk do microfone; sem captura, nunca.
async fn next_chunk(mic: &mut Option<mpsc::Receiver<Vec<i16>>>) -> Option<Vec<i16>> {
    match mic {
        Some(mic) => mic.recv().await,
        None => std::future::pending().await,
    }
}

/// Próximo evento da sessão; sem sessão (erro ou reconectando), nunca.
async fn next_event(session: &mut Option<Session>) -> Option<ServerEvent> {
    match session {
        Some(session) => session.next_event().await,
        None => std::future::pending().await,
    }
}

async fn connect(config: &EngineConfig, backend: &mut Backend) -> Result<Session, LiveError> {
    match backend {
        Backend::Real => LiveSession::connect(config.live_config())
            .await
            .map(Session::Live),
        #[cfg(test)]
        Backend::Fake(fake) => Ok(Session::Fake(fake.connect())),
    }
}

/// Encaminha os chunks do canal síncrono do `cpal` (thread própria do host de
/// áudio) para um canal `tokio` que o loop pode `select!`ar.
fn bridge_capture_channel(std_rx: std_mpsc::Receiver<Vec<i16>>) -> mpsc::Receiver<Vec<i16>> {
    let (tokio_tx, tokio_rx) = mpsc::channel(CHANNEL_CAPACITY);
    std::thread::spawn(move || {
        while let Ok(chunk) = std_rx.recv() {
            if tokio_tx.blocking_send(chunk).is_err() {
                break;
            }
        }
    });
    tokio_rx
}

/// Gravador de diagnóstico: o que foi pro alto-falante, o que foi pro
/// modelo e a linha do tempo dos eventos. Só existe com `record_dir`.
struct Recorder {
    playback: hound::WavWriter<io::BufWriter<std::fs::File>>,
    mic: hound::WavWriter<io::BufWriter<std::fs::File>>,
    events: io::BufWriter<std::fs::File>,
    started: Instant,
}

impl Recorder {
    fn open(dir: &std::path::Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let spec = |rate| hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let wav = |name: &str, rate| {
            hound::WavWriter::create(dir.join(name), spec(rate))
                .map_err(|e| io::Error::other(e.to_string()))
        };
        Ok(Self {
            playback: wav("playback.wav", 24_000)?,
            mic: wav("mic.wav", 16_000)?,
            events: io::BufWriter::new(std::fs::File::create(dir.join("events.log"))?),
            started: Instant::now(),
        })
    }

    fn event(&mut self, what: &str, detail: impl std::fmt::Display) {
        let ms = self.started.elapsed().as_millis();
        let _ = writeln!(self.events, "{ms:>8}ms  {what:<14} {detail}");
        // Descarrega a cada evento: se o processo morrer sem Ctrl+C (pane
        // fechado, kill), o log e os cabeçalhos WAV ainda ficam válidos.
        let _ = self.events.flush();
        let _ = self.playback.flush();
        let _ = self.mic.flush();
    }

    fn playback(&mut self, samples: &[i16]) {
        for &s in samples {
            let _ = self.playback.write_sample(s);
        }
    }

    fn mic(&mut self, samples: &[i16]) {
        for &s in samples {
            let _ = self.mic.write_sample(s);
        }
    }

    fn finish(self) {
        let _ = self.playback.finalize();
        let _ = self.mic.finalize();
        let mut events = self.events;
        let _ = events.flush();
    }
}

#[cfg(test)]
mod fake {
    use tokio::sync::mpsc;

    use crate::live::protocol::ServerEvent;

    /// Sessão roteirizada: entrega o que o teste mandar no canal.
    pub struct FakeSession {
        pub events: mpsc::Receiver<ServerEvent>,
    }

    /// Primeira conexão usa o roteiro do teste; as seguintes ficam mudas.
    pub struct FakeBackend {
        script: Option<mpsc::Receiver<ServerEvent>>,
        mic: Option<mpsc::Receiver<Vec<i16>>>,
        silent: Vec<mpsc::Sender<ServerEvent>>,
    }

    impl FakeBackend {
        pub fn new(script: mpsc::Receiver<ServerEvent>, mic: mpsc::Receiver<Vec<i16>>) -> Self {
            FakeBackend {
                script: Some(script),
                mic: Some(mic),
                silent: Vec::new(),
            }
        }

        pub fn connect(&mut self) -> FakeSession {
            let events = self.script.take().unwrap_or_else(|| {
                let (tx, rx) = mpsc::channel(1);
                self.silent.push(tx);
                rx
            });
            FakeSession { events }
        }

        pub fn take_mic(&mut self) -> mpsc::Receiver<Vec<i16>> {
            self.mic.take().expect("mic do teste já usado")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EngineConfig {
        EngineConfig {
            api_key: "chave-de-teste".to_string(),
            voice: "Puck".to_string(),
            device_in: None,
            device_out: None,
            barge_in: false,
            record_dir: None,
            system_prompt: String::new(),
            fx_amount: 0.35,
            greeting: None,
        }
    }

    async fn start_fake() -> (
        EngineHandle,
        mpsc::Sender<ServerEvent>,
        mpsc::Sender<Vec<i16>>,
    ) {
        let (script_tx, script_rx) = mpsc::channel(16);
        let (mic_tx, mic_rx) = mpsc::channel(16);
        let backend = Backend::Fake(fake::FakeBackend::new(script_rx, mic_rx));
        let handle = start_with(test_config(), backend)
            .await
            .expect("motor fake sobe");
        (handle, script_tx, mic_tx)
    }

    /// Próximo evento que não seja `Level`, com limite de tempo.
    async fn next_non_level(rx: &mut broadcast::Receiver<EngineEvent>) -> EngineEvent {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("evento dentro de 2s")
                .expect("canal aberto");
            if !matches!(event, EngineEvent::Level { .. }) {
                return event;
            }
        }
    }

    #[tokio::test]
    async fn engine_emits_session_events_in_order() {
        let (handle, script, _mic) = start_fake().await;
        let mut events = handle.events();

        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::State(EngineState::Connecting)
        );
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::State(EngineState::Listening)
        );

        script
            .send(ServerEvent::Audio(vec![8_000; 480]))
            .await
            .unwrap();
        script
            .send(ServerEvent::UserText("oi".into()))
            .await
            .unwrap();
        script
            .send(ServerEvent::ModelText("olá, você".into()))
            .await
            .unwrap();
        script.send(ServerEvent::TurnComplete).await.unwrap();

        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::State(EngineState::Speaking)
        );
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::UserText("oi".into())
        );
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::ModelText("olá, você".into())
        );
        assert_eq!(next_non_level(&mut events).await, EngineEvent::TurnComplete);
        // Sem fila tocando, volta a ouvir depois da folga do half-duplex.
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::State(EngineState::Listening)
        );

        handle.mute(true);
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::State(EngineState::Muted)
        );

        tokio::time::timeout(Duration::from_secs(2), handle.stop())
            .await
            .expect("stop termina");
    }

    #[tokio::test]
    async fn engine_emits_levels_for_mic_and_model() {
        let (handle, script, mic) = start_fake().await;
        let mut events = handle.events();

        mic.send(vec![16_384; 320]).await.unwrap();
        script
            .send(ServerEvent::Audio(vec![-16_384; 480]))
            .await
            .unwrap();

        let (mut saw_mic, mut saw_model) = (false, false);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !(saw_mic && saw_model) && Instant::now() < deadline {
            if let Ok(Ok(EngineEvent::Level { mic, model })) =
                tokio::time::timeout(Duration::from_millis(200), events.recv()).await
            {
                assert!((0.0..=1.0).contains(&mic) && (0.0..=1.0).contains(&model));
                saw_mic |= (mic - 0.5).abs() < 0.01;
                saw_model |= (model - 0.5).abs() < 0.01;
            }
        }
        assert!(saw_mic, "nível do mic não chegou");
        assert!(saw_model, "nível do modelo não chegou");

        handle.stop().await;
    }

    #[tokio::test]
    async fn engine_start_stop_finishes_thread() {
        let (handle, _script, _mic) = start_fake().await;
        tokio::time::timeout(Duration::from_secs(2), handle.stop())
            .await
            .expect("stop termina");
        assert!(*handle.finished.borrow(), "thread do motor não terminou");
        // Comandos depois do stop são ignorados, sem pânico.
        handle.mute(true);
        handle.reconnect();
    }

    #[tokio::test]
    async fn engine_reports_session_loss_and_reconnects() {
        let (handle, script, _mic) = start_fake().await;
        let mut events = handle.events();
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::State(EngineState::Connecting)
        );
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::State(EngineState::Listening)
        );

        drop(script);
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::State(EngineState::Error)
        );
        assert!(matches!(
            next_non_level(&mut events).await,
            EngineEvent::Error { .. }
        ));

        handle.reconnect();
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::State(EngineState::Connecting)
        );
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::Reconnecting { attempt: 1 }
        );
        assert_eq!(
            next_non_level(&mut events).await,
            EngineEvent::State(EngineState::Listening)
        );
        assert!(handle.last_error().is_none());

        handle.stop().await;
    }
}
