//! Motor da OpenJarvisBR: conecta a sessão Live, liga o microfone à sessão e
//! a sessão ao playback, e publica tudo o que acontece como [`EngineEvent`].
//! Não escreve nada no terminal — quem consome os eventos (CLI, desktop)
//! decide como mostrar.
//!
//! O loop roda numa thread própria com runtime tokio de uma thread só: os
//! streams do `cpal` não são `Send` em todas as plataformas, e assim o
//! [`EngineHandle`] pode ser usado de qualquer runtime.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tracing::{debug, info, warn};

use crate::audio::aec::EchoCanceller;
use crate::audio::capture::{self, CaptureError, CaptureHandle};
use crate::audio::fx::VoiceFx;
use crate::audio::playback::{PlaybackError, Player};
use crate::engine_tools::{self, VoiceAnswer};
use crate::live::protocol::ServerEvent;
use crate::live::session::{LiveConfig, LiveError, LiveSession};
use crate::mcp::McpServerConfig;
use crate::config;
use crate::tools::decisions::{self, Decisions};
use crate::tools::{FullAccess, Policy, Registry, Risk, ToolCall, ToolError, ToolResult, ToolSpec};

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
    /// Modo acesso total: emitido ao subir (se ligado) e a cada mudança por
    /// [`EngineHandle::set_full_access`].
    FullAccess(bool),
    /// O modelo pediu uma ferramenta (já com o risco efetivo da política).
    ToolRequested { call: ToolCall, risk: Risk },
    /// Chamada `Confirm` segurada: aprovar com
    /// [`EngineHandle::confirm_tool`] ou por voz ("sim"/"não").
    ToolConfirmNeeded {
        id: String,
        name: String,
        summary: String,
    },
    /// Fim de uma chamada: executada (`ok`), com erro, negada, sem
    /// confirmação a tempo ou cancelada pelo servidor.
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        summary: String,
    },
    /// O reflexo (Jev) executou uma ação segura antes de o modelo responder.
    ReflexActed {
        call: ToolCall,
        latency_ms: u32,
        confidence: f32,
    },
    /// O reflexo resolveu uma confirmação pendente por voz.
    ReflexConfirmed { approve: bool, confidence: f32 },
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
    /// Allow-list de ferramentas do perfil (globs: `fs.*`, `mcp.overclock.*`,
    /// `*`). Vazia = sem ferramentas (e sem seção de ferramentas no prompt).
    pub tools: Vec<String>,
    /// Servidores MCP cujas tools entram no registro (se algum glob alcança).
    pub mcp_servers: Vec<McpServerConfig>,
    /// Acesso total: nada pede confirmação e `fs.*` sai do home. Muda ao vivo
    /// com [`EngineHandle::set_full_access`].
    pub full_access: bool,
    /// `[tools].always_allow`: ferramentas que nunca pedem confirmação.
    /// Muda ao vivo com [`EngineHandle::set_always_allow`].
    pub always_allow: Vec<String>,
    /// Reflexo (Jev): desligado ou sem chave, o motor se comporta como na v3.
    pub reflex: config::ReflexSettings,
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
            .field("tools", &self.tools)
            .field("full_access", &self.full_access)
            .field("always_allow", &self.always_allow)
            // O `Debug` de `ReflexSettings` já mascara a chave.
            .field("reflex", &self.reflex)
            .field(
                "mcp_servers",
                &self.mcp_servers.iter().map(|s| &s.name).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl EngineConfig {
    fn live_config(&self, tools: &[ToolSpec]) -> LiveConfig {
        let prompt = engine_tools::system_prompt_with_tools(
            &self.system_prompt,
            !tools.is_empty(),
            self.full_access,
        );
        let mut cfg = LiveConfig::new(self.api_key.clone(), self.voice.clone())
            .with_system_prompt(prompt)
            .with_tools(tools.to_vec());
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
    /// Texto digitado como fala do usuário (modo texto / roteiro).
    UserText(String),
    Reconnect,
    SetFxAmount(f32),
    SetFullAccess(bool),
    SetAlwaysAllow(Vec<String>),
    ConfirmTool { id: String, approve: bool },
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

    /// Manda `text` como se o usuário tivesse falado: passa pelo reflexo,
    /// pela confirmação por voz e vira um turno de usuário na Live API
    /// (`clientContent`, `turnComplete`). Vazio é ignorado.
    pub fn send_user_text(&self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let _ = self.commands.send(Command::UserText(text.to_string()));
    }

    /// Ajusta o efeito de voz ao vivo (0..1).
    pub fn set_fx_amount(&self, amount: f32) {
        let _ = self.commands.send(Command::SetFxAmount(amount));
    }

    /// Liga/desliga o modo acesso total sem reconectar: vale a partir da
    /// próxima chamada de ferramenta e é anunciado com
    /// [`EngineEvent::FullAccess`].
    pub fn set_full_access(&self, on: bool) {
        let _ = self.commands.send(Command::SetFullAccess(on));
    }

    /// Troca a lista "sempre permitido" (ex.: removida nas configurações) sem
    /// reconectar. Quem chama já gravou o config.
    pub fn set_always_allow(&self, names: Vec<String>) {
        let _ = self.commands.send(Command::SetAlwaysAllow(names));
    }

    /// Responde a um [`EngineEvent::ToolConfirmNeeded`]. Id desconhecido (já
    /// resolvido por voz, expirado ou cancelado) é ignorado.
    pub fn confirm_tool(&self, id: &str, approve: bool) {
        let _ = self.commands.send(Command::ConfirmTool {
            id: id.to_string(),
            approve,
        });
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
    /// Live API e ferramentas de verdade, mas o "microfone" é um canal
    /// (fala sintetizada) e a saída é muda — validação ponta a ponta.
    #[cfg(test)]
    LiveMic(Option<mpsc::Receiver<Vec<i16>>>),
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

    fn send_tool_response(&self, results: &[ToolResult]) {
        match self {
            Session::Live(session) => session.send_tool_response(results),
            #[cfg(test)]
            Session::Fake(session) => {
                let _ = session.responses.send(results.to_vec());
            }
        }
    }

    /// Texto enviado como turno de usuário (mesmo caminho do `greeting`):
    /// contexto do reflexo para o modelo.
    fn send_text(&self, text: &str) {
        match self {
            Session::Live(session) => session.send_text(text),
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

/// Chamada `Confirm` esperando resposta.
struct PendingConfirm {
    call: ToolCall,
    asked: Instant,
    /// Fala do usuário acumulada desde o pedido (a transcrição chega picada).
    heard: String,
}

/// Prazos da confirmação; os testes encurtam.
#[derive(Debug, Clone, Copy)]
struct ToolTimeouts {
    voice_window: Duration,
    confirm: Duration,
    exec: Duration,
}

impl Default for ToolTimeouts {
    fn default() -> Self {
        ToolTimeouts {
            voice_window: engine_tools::VOICE_CONFIRM_WINDOW,
            confirm: engine_tools::CONFIRM_TIMEOUT,
            exec: engine_tools::EXEC_TIMEOUT,
        }
    }
}

/// Resultado de uma execução, devolvido pela task ao loop. Leva a chamada
/// inteira: uma chamada do modelo que deu certo vira ação aprendida.
struct Finished {
    call: ToolCall,
    result: ToolResult,
    /// Quanto a execução levou (sem contar espera por confirmação).
    elapsed: Duration,
}

/// Ação feita pelo reflexo há pouco: uma chamada igual do modelo dentro de
/// [`engine_tools::REFLEX_DONE_WINDOW`] recebe `output` sem executar de novo.
struct ReflexDone {
    call: ToolCall,
    at: Instant,
    output: serde_json::Value,
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
    /// Cancelamento de eco do mic; só existe com `barge_in`.
    aec: Option<EchoCanceller>,
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
    /// Ferramentas liberadas pelo perfil.
    registry: Registry,
    tool_specs: Vec<ToolSpec>,
    policy: Policy,
    full_access: FullAccess,
    timeouts: ToolTimeouts,
    confirms: Vec<PendingConfirm>,
    /// Ações `Confirm` aprovadas há pouco (nome, argumentos, quando): o modelo
    /// às vezes chama de novo ao ouvir o "sim".
    approved: Vec<(String, serde_json::Value, Instant)>,
    /// Fala do modelo no turno corrente (para ver se terminou em "confirma?").
    model_turn: String,
    /// Fala do turno anterior do modelo, para descartar repetição literal.
    previous_model_turn: String,
    /// O turno corrente repete o anterior: áudio e texto dele são descartados.
    turn_repeated: bool,
    /// O que já não pede confirmação: leitura, aprovadas na sessão e
    /// `always_allow`.
    decisions: Decisions,
    /// Última ferramenta que pediu confirmação, alvo de um "sempre pode"
    /// dito depois.
    last_confirm_tool: Option<String>,
    /// Fala do usuário desde o fim do último turno do modelo (chega picada).
    user_heard: String,
    /// Quando o modelo perguntou "confirma?" sem chamada pendente, e o que o
    /// usuário disse desde então.
    question: Option<(Instant, String)>,
    /// "Sim" dito à pergunta do modelo antes de a chamada chegar: aprova a
    /// próxima chamada `Confirm` dentro da janela de voz.
    early_approval: Option<Instant>,
    /// Execuções em andamento, por id (para cancelar).
    running: HashMap<String, (String, tokio::task::AbortHandle)>,
    finished_tx: mpsc::UnboundedSender<Finished>,
    finished_rx: mpsc::UnboundedReceiver<Finished>,
    /// Reflexo (Jev); `None` desligado, sem chave ou sem dublê nos testes.
    reflex: Option<crate::reflex::Reflex>,
    reflex_rx: mpsc::UnboundedReceiver<crate::reflex::Outcome>,
    /// Sender vivo quando o reflexo está desligado: um receiver órfão
    /// devolveria `None` direto e viraria loop quente no `select!`.
    _reflex_keepalive: Option<mpsc::UnboundedSender<crate::reflex::Outcome>>,
    /// Ações feitas pelo reflexo, para deduplicar a chamada igual do modelo.
    recently_done: Vec<ReflexDone>,
    /// Chamadas do modelo ainda não terminadas → o que o usuário tinha dito
    /// quando chegaram. Sem erro no fim, a ação fica aprendida com essa frase.
    pending_learn: HashMap<String, String>,
    /// Chamadas que o modelo pediu nesta fala do usuário (zerada quando uma
    /// fala nova começa; vale `REFLEX_DONE_WINDOW`). O reflexo não repete o
    /// que o modelo já pediu, e o modelo não repete a si mesmo.
    model_calls: Vec<(ToolCall, Instant)>,
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
        let full_access = FullAccess::new(config.full_access);
        if config.full_access {
            emit.send(EngineEvent::FullAccess(true));
        }
        let (registry, timeouts) = match &backend {
            Backend::Real => (
                engine_tools::build_registry(&config.tools, &config.mcp_servers, &full_access)
                    .await,
                ToolTimeouts::default(),
            ),
            #[cfg(test)]
            Backend::Fake(fake) => (fake.registry.filter_for_profile(&config.tools), fake.timeouts),
            #[cfg(test)]
            Backend::LiveMic(_) => (
                engine_tools::build_registry(&config.tools, &config.mcp_servers, &full_access)
                    .await,
                ToolTimeouts::default(),
            ),
        };
        let tool_specs = registry.specs();
        info!(ferramentas = tool_specs.len(), "conectando à Live API");
        let session = connect(&config, &tool_specs, &mut backend)
            .await
            .map_err(EngineError::Connect)?;
        // O `greeting` (ex.: "se apresente no novo papel") é só para a
        // primeira conexão; uma reconexão manual (`Worker::reconnect`) reusa
        // este `config` e não deve repeti-lo.
        config.greeting = None;

        let (mic, capture, output, aec) = match &mut backend {
            Backend::Real => {
                let (std_tx, std_rx) = std_mpsc::channel::<Vec<i16>>();
                let capture = capture::start(config.device_in.as_deref(), std_tx)
                    .map_err(EngineError::Capture)?;
                // Barge-in com caixa de som: o mic passa pelo cancelamento de
                // eco com o que o player toca como referência.
                let (player, aec) = if config.barge_in {
                    let (player, reference) =
                        Player::with_echo_reference(config.device_out.as_deref())
                            .map_err(EngineError::Playback)?;
                    info!(cancelador = EchoCanceller::backend(), "cancelamento de eco ligado");
                    (player, Some(EchoCanceller::new(reference)))
                } else {
                    let player = Player::new(config.device_out.as_deref())
                        .map_err(EngineError::Playback)?;
                    (player, None)
                };
                (
                    bridge_capture_channel(std_rx),
                    Some(capture),
                    Output::Device(Box::new(player)),
                    aec,
                )
            }
            #[cfg(test)]
            Backend::Fake(fake) => (fake.take_mic(), None, Output::Null, None),
            #[cfg(test)]
            Backend::LiveMic(mic) => (mic.take().expect("mic já usado"), None, Output::Null, None),
        };

        let recorder = config.record_dir.as_ref().and_then(|dir| match Recorder::open(dir) {
            Ok(recorder) => Some(recorder),
            Err(err) => {
                warn!(pasta = %dir.display(), erro = %err, "não foi possível abrir a gravação de diagnóstico");
                None
            }
        });

        let (reflex, reflex_rx, reflex_keepalive) = match &backend {
            #[cfg(test)]
            Backend::Fake(fake) if fake.reflex.is_some() => {
                let (judge, eye) = fake.reflex.clone().expect("reflexo do teste");
                let (reflex, rx) = crate::reflex::Reflex::new(config.reflex.clone(), judge, eye);
                (Some(reflex), rx, None)
            }
            Backend::Real => match crate::reflex::Reflex::from_settings(config.reflex.clone()) {
                Some((reflex, rx)) => {
                    info!("reflexo ligado");
                    (Some(reflex), rx, None)
                }
                None => {
                    let (tx, rx) = mpsc::unbounded_channel();
                    (None, rx, Some(tx))
                }
            },
            #[cfg(test)]
            Backend::Fake(_) | Backend::LiveMic(_) => {
                let (tx, rx) = mpsc::unbounded_channel();
                (None, rx, Some(tx))
            }
        };

        let (finished_tx, finished_rx) = mpsc::unbounded_channel();
        Ok(Worker {
            reflex,
            reflex_rx,
            _reflex_keepalive: reflex_keepalive,
            recently_done: Vec::new(),
            pending_learn: HashMap::new(),
            model_calls: Vec::new(),
            registry,
            tool_specs,
            policy: Policy::with_full_access(full_access.clone()),
            full_access,
            timeouts,
            confirms: Vec::new(),
            approved: Vec::new(),
            model_turn: String::new(),
            previous_model_turn: String::new(),
            turn_repeated: false,
            decisions: Decisions::new(&config.always_allow),
            last_confirm_tool: None,
            user_heard: String::new(),
            question: None,
            early_approval: None,
            running: HashMap::new(),
            finished_tx,
            finished_rx,
            voice_fx: VoiceFx::new(config.fx_amount),
            config,
            backend,
            commands,
            emit,
            last_error,
            session: Some(session),
            mic: Some(mic),
            _capture: capture,
            aec,
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
                Some(done) = self.finished_rx.recv() => self.on_tool_finished(done),
                Some(outcome) = self.reflex_rx.recv() => self.on_reflex(outcome),
                command = self.commands.recv() => match command {
                    Some(Command::Mute(muted)) => {
                        self.muted = muted;
                        self.update_state();
                    }
                    Some(Command::ConfirmTool { id, approve }) => self.resolve_confirm(&id, approve, "botão"),
                    Some(Command::UserText(text)) => self.on_typed_text(text),
                    Some(Command::SetFxAmount(amount)) => self.voice_fx = VoiceFx::new(amount),
                    Some(Command::SetFullAccess(on)) => self.set_full_access(on),
                    Some(Command::SetAlwaysAllow(names)) => self.set_always_allow(names),
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

        self.reset_tools();
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
        // Com barge-in, o AEC consome a referência a cada chunk (mesmo mudo,
        // para ela não acumular) e o gate residual descarta o que é só eco.
        let cleaned = self.aec.as_mut().map(|aec| aec.process(&samples));
        if self.muted {
            return;
        }
        self.mic_meter.add(&samples);
        if gated {
            return;
        }
        let samples = match cleaned {
            Some(Some(cleaned)) => cleaned,
            Some(None) => return,
            None => samples,
        };
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
                if self.turn_repeated {
                    // Fala repetida: não toca.
                    return;
                }
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
                self.end_model_turn();
                if let Some(reflex) = &self.reflex {
                    reflex.end_turn();
                }
                self.emit.send(EngineEvent::TurnComplete);
                self.update_state();
            }
            Some(ServerEvent::UserText(text)) => self.on_user_text(text, "voz"),
            Some(ServerEvent::ModelText(text)) => {
                debug!(texto = %text, "modelo disse");
                if let Some(r) = self.recorder.as_mut() {
                    r.event("model_text", &text);
                }
                self.model_turn.push_str(&text);
                if self.turn_repeated || self.check_repeat() {
                    return;
                }
                self.emit.send(EngineEvent::ModelText(text));
            }
            Some(ServerEvent::TurnComplete) => {
                debug!("turno do modelo concluído");
                if let Some(r) = self.recorder.as_mut() {
                    r.event("turn_complete", "");
                }
                self.output.end_of_turn();
                self.end_model_turn();
                if let Some(reflex) = &self.reflex {
                    reflex.end_turn();
                }
                self.emit.send(EngineEvent::TurnComplete);
            }
            Some(ServerEvent::GoAway) => {
                if let Some(r) = self.recorder.as_mut() {
                    r.event("go_away", "reconectando");
                }
                info!("servidor pediu encerramento (goAway); reconectando");
            }
            Some(ServerEvent::ToolCall(calls)) => {
                for call in calls {
                    self.on_tool_call(call);
                }
            }
            Some(ServerEvent::ToolCallCancellation(ids)) => self.cancel_tools(&ids),
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
                self.reset_tools();
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

    /// Troca o modo acesso total. A próxima reconexão já leva o prompt certo.
    fn set_full_access(&mut self, on: bool) {
        info!(acesso_total = on, "modo acesso total alterado");
        self.full_access.set(on);
        self.config.full_access = on;
        if let Some(r) = self.recorder.as_mut() {
            r.event("full_access", on.to_string());
        }
        self.emit.send(EngineEvent::FullAccess(on));
    }

    /// Lista "sempre permitido" trocada de fora (configurações).
    fn set_always_allow(&mut self, names: Vec<String>) {
        info!(ferramentas = ?names, "lista sempre permitido alterada");
        self.decisions.set_always_allow(&names);
    }

    /// A fala do modelo neste turno repete a do anterior? Marca o turno,
    /// descarta o áudio na fila e avisa no log.
    fn check_repeat(&mut self) -> bool {
        if !engine_tools::is_repeat(&self.model_turn, &self.previous_model_turn) {
            return false;
        }
        warn!(fala = %self.model_turn.trim(), "modelo repetiu a fala anterior; descartada");
        if let Some(r) = self.recorder.as_mut() {
            r.event("model_repeat", self.model_turn.trim());
        }
        self.turn_repeated = true;
        self.output.flush();
        self.last_model_audio = None;
        self.speaking = false;
        self.update_state();
        true
    }

    /// "Sempre pode" / "não pergunta mais": libera para sempre as pendentes
    /// (aprovando-as), a ferramenta citada ("rodar comandos") ou a última que
    /// pediu confirmação. `true` se a fala foi consumida.
    fn hear_always_allow(&mut self) -> bool {
        if !decisions::always_allow_intent(&self.user_heard) {
            return false;
        }
        let pending: Vec<(String, String)> = self
            .confirms
            .iter()
            .map(|p| (p.call.id.clone(), p.call.name.clone()))
            .collect();
        let mut names: Vec<String> = pending.iter().map(|(_, name)| name.clone()).collect();
        if names.is_empty() {
            let target = decisions::tool_hint(&self.user_heard)
                .map(str::to_string)
                .or_else(|| self.last_confirm_tool.clone());
            names.extend(target);
        }
        self.user_heard.clear();
        if names.is_empty() {
            return false;
        }
        let mut changed = false;
        for name in &names {
            changed |= self.decisions.allow_always(name);
        }
        info!(ferramentas = ?names, "liberadas para sempre por voz");
        if let Some(r) = self.recorder.as_mut() {
            r.event("always_allow", names.join(","));
        }
        if changed {
            self.persist_always_allow();
        }
        if self.question.take().is_some() {
            self.early_approval = Some(Instant::now());
        }
        for (id, _) in pending {
            self.resolve_confirm(&id, true, "voz, sempre pode");
        }
        true
    }

    /// Grava `[tools].always_allow` no config (só no backend real: os testes
    /// não mexem no config do usuário).
    fn persist_always_allow(&self) {
        if !matches!(self.backend, Backend::Real) {
            return;
        }
        if let Err(err) = config::save_always_allow(&self.decisions.always_allow()) {
            warn!(erro = %err, "não foi possível gravar [tools].always_allow");
        }
    }

    /// Fala do usuário (transcrição da Live API ou texto digitado): alimenta
    /// o reflexo e a confirmação por voz, grava e publica.
    fn on_user_text(&mut self, text: String, via: &'static str) {
        debug!(texto = %text, via, "usuário disse");
        if let Some(r) = self.recorder.as_mut() {
            r.event("user_text", &text);
        }
        if self.user_heard.is_empty() {
            // fala nova: o que o modelo pediu na anterior não conta mais
            self.model_calls.clear();
        }
        self.user_heard.push_str(&text);
        if let Some(reflex) = &self.reflex {
            reflex.hear(self.user_heard.clone(), !self.confirms.is_empty());
        }
        if !self.hear_always_allow() {
            self.hear_confirmation(&text);
        }
        self.emit.send(EngineEvent::UserText(text));
    }

    /// Texto digitado (modo texto): mesmo caminho da transcrição e, em
    /// seguida, um turno de usuário para o modelo responder.
    fn on_typed_text(&mut self, text: String) {
        self.on_user_text(text.clone(), "texto");
        if let Some(session) = self.session.as_ref() {
            session.send_text(&text);
        }
    }

    /// O modelo já pediu esta mesma (tool, args) nesta fala, dentro da janela?
    fn model_already_called(&self, call: &ToolCall) -> bool {
        self.model_calls.iter().any(|(done, at)| {
            done.id != call.id
                && done.name == call.name
                && done.args == call.args
                && at.elapsed() < engine_tools::REFLEX_DONE_WINDOW
        })
    }

    /// Aplica a política: `Safe` executa já; `Confirm` segura e pergunta.
    fn on_tool_call(&mut self, call: ToolCall) {
        let summary = engine_tools::call_summary(&call);
        let Some(spec) = self.registry.get(&call.name).map(|tool| tool.spec()) else {
            // Fora da allow-list do perfil (ou inexistente): recusa na hora.
            warn!(ferramenta = %call.name, "ferramenta não liberada neste perfil");
            self.emit.send(EngineEvent::ToolRequested {
                call: call.clone(),
                risk: Risk::Confirm,
            });
            let error = ToolError::NotAllowed(call.name.clone()).to_string();
            self.respond(&call.name, ToolResult::err(&call.id, error));
            return;
        };
        if let Some(done) = self.recently_done.iter().find(|done| {
            done.call.name == call.name
                && done.call.args == call.args
                && done.at.elapsed() < engine_tools::REFLEX_DONE_WINDOW
        }) {
            let output = done.output.clone();
            info!(ferramenta = %call.name, "já executada pelo reflexo; não repete");
            if let Some(r) = self.recorder.as_mut() {
                r.event("tool_dedup_reflex", &call.id);
            }
            self.emit.send(EngineEvent::ToolRequested {
                call: call.clone(),
                risk: Risk::Safe,
            });
            let output = serde_json::json!({
                "status": "já executada",
                "nota": "o reflexo acabou de executar exatamente esta ação; \
                         use este resultado e não chame de novo",
                "resultado": output,
            });
            self.respond(&call.name, ToolResult::ok(&call.id, output));
            return;
        }
        self.pending_learn
            .insert(call.id.clone(), self.user_heard.clone());
        self.model_calls
            .retain(|(_, at)| at.elapsed() < engine_tools::REFLEX_DONE_WINDOW);
        self.model_calls.push((call.clone(), Instant::now()));
        // O reflexo já pediu confirmação desta mesma ação: a chamada do modelo
        // assume o lugar dela (com a fala já ouvida), para um "sim" só rodar
        // uma vez e a resposta ir ao modelo.
        let inherited = self.take_reflex_confirm_like(&call);
        let mut risk = self.policy.risk(&spec);
        if risk == Risk::Confirm
            && self.decisions.allows(&call.name)
            && !self.recently_approved(&call)
        {
            // Já aprovada nesta sessão, liberada para sempre ou só leitura.
            risk = Risk::Safe;
        }
        info!(id = %call.id, ferramenta = %call.name, risco = ?risk, pedido = %summary, "tool pedida pelo modelo");
        if let Some(r) = self.recorder.as_mut() {
            r.event("tool_call", format!("{} [{risk:?}] {summary}", call.id));
        }
        self.emit.send(EngineEvent::ToolRequested {
            call: call.clone(),
            risk,
        });
        match risk {
            Risk::Safe => self.execute(call),
            Risk::Confirm if self.recently_approved(&call) => {
                info!(ferramenta = %call.name, "repetição de ação já aprovada, não executa de novo");
                self.pending_learn.remove(&call.id);
                if let Some(r) = self.recorder.as_mut() {
                    r.event("tool_repeat", &call.id);
                }
                let output = serde_json::json!({
                    "status": "já executada",
                    "nota": "esta mesma ação acabou de ser confirmada e executada; \
                             use o resultado da chamada anterior e não chame de novo",
                });
                self.respond(&call.name, ToolResult::ok(&call.id, output));
            }
            Risk::Confirm => {
                info!(ferramenta = %call.name, "aguardando confirmação");
                self.last_confirm_tool = Some(call.name.clone());
                let id = call.id.clone();
                self.emit.send(EngineEvent::ToolConfirmNeeded {
                    id: id.clone(),
                    name: call.name.clone(),
                    summary,
                });
                let (asked, heard) = inherited.unwrap_or_else(|| (Instant::now(), String::new()));
                self.confirms.push(PendingConfirm { call, asked, heard });
                let window = self.timeouts.voice_window;
                if self
                    .early_approval
                    .take()
                    .is_some_and(|at| at.elapsed() < window)
                {
                    self.resolve_confirm(&id, true, "voz, antes da chamada");
                }
            }
        }
    }

    /// Há confirmação pendente do reflexo com o mesmo nome e argumentos?
    /// Tira-a da fila (evento "cancelada", como em `cancel_tools`) e devolve
    /// quando foi pedida e o que já se ouviu, para a chamada do modelo herdar.
    fn take_reflex_confirm_like(&mut self, call: &ToolCall) -> Option<(Instant, String)> {
        let at = self.confirms.iter().position(|p| {
            p.call.id.starts_with(crate::reflex::decide::REFLEX_CALL_PREFIX)
                && p.call.name == call.name
                && p.call.args == call.args
        })?;
        let pending = self.confirms.remove(at);
        info!(ferramenta = %call.name, "chamada do modelo assume a confirmação pedida pelo reflexo");
        if let Some(r) = self.recorder.as_mut() {
            r.event("tool_cancel", format!("{} substituída por {}", pending.call.id, call.id));
        }
        self.emit.send(EngineEvent::ToolResult {
            id: pending.call.id,
            name: pending.call.name,
            ok: false,
            summary: "cancelada".to_string(),
        });
        Some((pending.asked, pending.heard))
    }

    /// Decisão do reflexo: age, confirma ou libera, sempre pelos caminhos que
    /// já existem para o modelo.
    fn on_reflex(&mut self, outcome: crate::reflex::Outcome) {
        use crate::reflex::decide::Decision;
        match outcome.decision {
            Decision::Act(call) => {
                let Some(spec) = self.registry.get(&call.name).map(|tool| tool.spec()) else {
                    warn!(ferramenta = %call.name, "reflexo pediu ferramenta fora do perfil; ignorado");
                    return;
                };
                // O modelo chegou primeiro com a mesma ação nesta fala: o
                // reflexo não repete (JRV-83).
                if self.model_already_called(&call) {
                    info!(ferramenta = %call.name, ms = outcome.latency_ms, "modelo já pediu esta ação; reflexo não repete");
                    if let Some(r) = self.recorder.as_mut() {
                        r.event("tool_dedup_model", format!("{} (reflexo)", engine_tools::call_summary(&call)));
                    }
                    return;
                }
                // Regra de risco, a mesma do modelo: `Safe` (ou já liberada)
                // executa já; `Confirm` pergunta na hora, sem esperar o Gemini.
                if self.policy.risk(&spec) == Risk::Confirm && !self.decisions.allows(&call.name) {
                    self.reflex_confirm(call, outcome.latency_ms);
                    return;
                }
                info!(ferramenta = %call.name, ms = outcome.latency_ms, "reflexo agiu");
                if let Some(r) = self.recorder.as_mut() {
                    r.event(
                        "reflex_act",
                        format!(
                            "{} {} {}ms",
                            call.id,
                            engine_tools::call_summary(&call),
                            outcome.latency_ms
                        ),
                    );
                }
                self.recently_done
                    .retain(|done| done.at.elapsed() < engine_tools::REFLEX_DONE_WINDOW);
                self.recently_done.push(ReflexDone {
                    call: call.clone(),
                    at: Instant::now(),
                    output: serde_json::json!({"status": "executada pelo reflexo"}),
                });
                self.emit.send(EngineEvent::ReflexActed {
                    call: call.clone(),
                    latency_ms: outcome.latency_ms,
                    confidence: outcome.confidence,
                });
                self.emit.send(EngineEvent::ToolRequested {
                    call: call.clone(),
                    risk: Risk::Safe,
                });
                if let Some(session) = self.session.as_ref() {
                    session.send_text(&engine_tools::reflex_context_text(&call));
                }
                self.execute(call);
            }
            Decision::Approve | Decision::Deny => {
                let approve = matches!(outcome.decision, Decision::Approve);
                let ids: Vec<String> = self.confirms.iter().map(|p| p.call.id.clone()).collect();
                if ids.is_empty() {
                    return;
                }
                self.emit.send(EngineEvent::ReflexConfirmed {
                    approve,
                    confidence: outcome.confidence,
                });
                for id in ids {
                    self.resolve_confirm(&id, approve, "voz, reflexo");
                }
                self.user_heard.clear();
            }
            Decision::AlwaysAllow => {
                // Reaproveita o caminho de voz com a fala atual; se a lista
                // fixa não reconheceu a frase, libera as pendentes mesmo assim.
                if !self.hear_always_allow() {
                    let pending: Vec<(String, String)> = self
                        .confirms
                        .iter()
                        .map(|p| (p.call.id.clone(), p.call.name.clone()))
                        .collect();
                    if pending.is_empty() {
                        return;
                    }
                    self.emit.send(EngineEvent::ReflexConfirmed {
                        approve: true,
                        confidence: outcome.confidence,
                    });
                    let mut changed = false;
                    for (_, name) in &pending {
                        changed |= self.decisions.allow_always(name);
                    }
                    if changed {
                        self.persist_always_allow();
                    }
                    for (id, _) in pending {
                        self.resolve_confirm(&id, true, "voz, reflexo sempre pode");
                    }
                    self.user_heard.clear();
                }
            }
            Decision::Nothing => {}
        }
    }

    /// Ação `Confirm` escolhida pelo reflexo: não executa; pede confirmação
    /// já, pelo mesmo caminho da chamada do modelo. "Sim" por voz ou botão
    /// executa; a chamada igual do modelo, se vier, assume o pedido.
    fn reflex_confirm(&mut self, call: ToolCall, latency_ms: u32) {
        let summary = engine_tools::call_summary(&call);
        info!(ferramenta = %call.name, ms = latency_ms, "reflexo pediu confirmação");
        if let Some(r) = self.recorder.as_mut() {
            r.event(
                "reflex_confirm",
                format!("{} {summary} {latency_ms}ms", call.id),
            );
        }
        self.last_confirm_tool = Some(call.name.clone());
        self.emit.send(EngineEvent::ToolRequested {
            call: call.clone(),
            risk: Risk::Confirm,
        });
        self.emit.send(EngineEvent::ToolConfirmNeeded {
            id: call.id.clone(),
            name: call.name.clone(),
            summary,
        });
        self.confirms.push(PendingConfirm {
            call,
            asked: Instant::now(),
            heard: String::new(),
        });
    }

    /// Fim de um turno do modelo: se ele terminou perguntando "confirma?"
    /// sem chamada pendente, a próxima resposta do usuário fica valendo.
    fn end_model_turn(&mut self) {
        let turn = std::mem::take(&mut self.model_turn);
        self.user_heard.clear();
        if !self.turn_repeated
            && engine_tools::is_repeat(&turn, &self.previous_model_turn)
        {
            warn!(fala = %turn.trim(), "modelo repetiu a fala anterior; descartada");
            self.output.flush();
            self.last_model_audio = None;
            self.speaking = false;
        }
        self.turn_repeated = false;
        if !turn.trim().is_empty() {
            self.previous_model_turn = turn.clone();
        }
        if self.confirms.is_empty() && engine_tools::asks_confirmation(&turn) {
            self.question = Some((Instant::now(), String::new()));
        }
    }

    /// Fala do usuário enquanto há confirmação pendente: "sim" aprova, "não"
    /// nega — vale para todas as pendentes dentro da janela de voz.
    fn hear_confirmation(&mut self, text: &str) {
        if self.confirms.is_empty() {
            let window = self.timeouts.voice_window;
            if let Some((at, heard)) = self.question.as_mut() {
                if at.elapsed() > window {
                    self.question = None;
                } else {
                    heard.push_str(text);
                    match engine_tools::voice_answer(heard) {
                        Some(VoiceAnswer::Approve) => {
                            self.early_approval = Some(Instant::now());
                            self.question = None;
                        }
                        Some(VoiceAnswer::Deny) => {
                            self.early_approval = None;
                            self.question = None;
                        }
                        None => {}
                    }
                }
            }
            return;
        }
        let window = self.timeouts.voice_window;
        let mut decided = Vec::new();
        for pending in &mut self.confirms {
            if pending.asked.elapsed() > window {
                continue;
            }
            // Os fragmentos já trazem os próprios espaços.
            pending.heard.push_str(text);
            if let Some(answer) = engine_tools::voice_answer(&pending.heard) {
                decided.push((pending.call.id.clone(), answer == VoiceAnswer::Approve));
            }
        }
        for (id, approve) in decided {
            self.resolve_confirm(&id, approve, "voz");
        }
    }

    fn resolve_confirm(&mut self, id: &str, approve: bool, via: &str) {
        let Some(at) = self.confirms.iter().position(|p| p.call.id == id) else {
            return;
        };
        let pending = self.confirms.remove(at);
        if let Some(r) = self.recorder.as_mut() {
            let verdict = if approve { "aprovada" } else { "negada" };
            r.event("tool_confirm", format!("{id} {verdict} por {via}"));
        }
        info!(ferramenta = %pending.call.name, aprovada = approve, via, "confirmação");
        if approve {
            self.decisions.approve_for_session(&pending.call.name);
            self.approved
                .retain(|(_, _, at)| at.elapsed() < engine_tools::REPEAT_WINDOW);
            self.approved.push((
                pending.call.name.clone(),
                pending.call.args.clone(),
                Instant::now(),
            ));
            if pending
                .call
                .id
                .starts_with(crate::reflex::decide::REFLEX_CALL_PREFIX)
            {
                // Ação do reflexo aprovada: só agora o modelo fica sabendo, e
                // a chamada igual dele passa a ser deduplicada.
                self.recently_done
                    .retain(|done| done.at.elapsed() < engine_tools::REFLEX_DONE_WINDOW);
                self.recently_done.push(ReflexDone {
                    call: pending.call.clone(),
                    at: Instant::now(),
                    output: serde_json::json!({"status": "executada pelo reflexo"}),
                });
                if let Some(session) = self.session.as_ref() {
                    session.send_text(&engine_tools::reflex_context_text(&pending.call));
                }
            }
            self.execute(pending.call);
        } else {
            self.pending_learn.remove(&pending.call.id);
            let error = format!(
                "{}: o usuário não autorizou, nada foi executado",
                ToolError::Denied
            );
            self.respond(&pending.call.name, ToolResult::err(&pending.call.id, error));
        }
    }

    fn recently_approved(&self, call: &ToolCall) -> bool {
        self.approved.iter().any(|(name, args, at)| {
            name == &call.name && args == &call.args && at.elapsed() < engine_tools::REPEAT_WINDOW
        })
    }

    /// Confirmações sem resposta no prazo viram negação.
    fn expire_confirms(&mut self) {
        let limit = self.timeouts.confirm;
        let expired: Vec<ToolCall> = {
            let (old, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.confirms)
                .into_iter()
                .partition(|p| p.asked.elapsed() >= limit);
            self.confirms = keep;
            old.into_iter().map(|p| p.call).collect()
        };
        for call in expired {
            self.pending_learn.remove(&call.id);
            if let Some(r) = self.recorder.as_mut() {
                r.event("tool_confirm", format!("{} expirada", call.id));
            }
            let error = format!(
                "sem confirmação em {}s: nada foi executado",
                limit.as_secs().max(1)
            );
            self.respond(&call.name, ToolResult::err(&call.id, error));
        }
    }

    /// Executa numa task própria, com timeout; o resultado volta pelo canal.
    fn execute(&mut self, call: ToolCall) {
        let registry = self.registry.clone();
        let limit = self.timeouts.exec;
        let done = self.finished_tx.clone();
        let id = call.id.clone();
        let name = call.name.clone();
        let task = tokio::spawn(async move {
            let t0 = Instant::now();
            let result = match tokio::time::timeout(limit, registry.call(&call)).await {
                Ok(result) => result,
                Err(_) => ToolResult::err(
                    &call.id,
                    format!("{} ({}s): a ação não terminou", ToolError::Timeout, limit.as_secs()),
                ),
            };
            let _ = done.send(Finished { call, result, elapsed: t0.elapsed() });
        });
        self.running.insert(id, (name, task.abort_handle()));
    }

    fn on_tool_finished(&mut self, done: Finished) {
        // Cancelada no meio do caminho: o servidor não quer mais a resposta.
        if self.running.remove(&done.result.id).is_none() {
            self.pending_learn.remove(&done.result.id);
            debug!(id = %done.result.id, ferramenta = %done.call.name, "tool terminou depois de cancelada");
            return;
        }
        let origem = if done.result.id.starts_with(crate::reflex::decide::REFLEX_CALL_PREFIX) {
            "reflexo"
        } else {
            "modelo"
        };
        let ms = done.elapsed.as_millis() as u64;
        match done.result.error.as_deref() {
            Some(err) => warn!(
                id = %done.result.id, ferramenta = %done.call.name, origem, ms, erro = %err,
                "tool falhou"
            ),
            None => info!(
                id = %done.result.id, ferramenta = %done.call.name, origem, ms,
                resultado = %engine_tools::result_summary(&done.result.output, None),
                "tool concluída"
            ),
        }
        if !done
            .result
            .id
            .starts_with(crate::reflex::decide::REFLEX_CALL_PREFIX)
        {
            let phrase = self.pending_learn.remove(&done.result.id);
            if done.result.error.is_none() {
                self.learn(phrase.as_deref().unwrap_or_default(), &done.call);
            }
            self.respond(&done.call.name, done.result);
            return;
        }
        // Chamada do reflexo: o modelo não a pediu, e um `functionResponse`
        // com id desconhecido é erro de protocolo. Só o evento sai. Falhou?
        // Sai da dedup para o modelo poder tentar; deu certo? A dedup passa a
        // devolver o resultado real.
        let at = self
            .recently_done
            .iter()
            .position(|d| d.call.id == done.result.id);
        match (at, done.result.error.is_some()) {
            (Some(at), true) => {
                self.recently_done.remove(at);
            }
            (Some(at), false) => self.recently_done[at].output = done.result.output.clone(),
            (None, _) => {}
        }
        self.emit_result(&done.call.name, done.result);
    }

    /// Ação do modelo que deu certo vira memória do reflexo, com a frase que
    /// o usuário disse. Sem frase (o modelo agiu sozinho) não há o que
    /// reconhecer depois: não aprende.
    fn learn(&mut self, phrase: &str, call: &ToolCall) {
        let phrase = phrase.trim();
        if phrase.is_empty() {
            return;
        }
        let Some(reflex) = self.reflex.as_ref() else {
            return;
        };
        if reflex.eye().learn(phrase, call) {
            info!(ferramenta = %call.name, frase = phrase, "reflexo aprendeu a ação");
            if let Some(r) = self.recorder.as_mut() {
                r.event(
                    "reflex_learn",
                    format!("{} \"{phrase}\"", engine_tools::call_summary(call)),
                );
            }
        }
    }

    /// Devolve o resultado ao modelo e publica o evento.
    fn respond(&mut self, name: &str, result: ToolResult) {
        if let Some(session) = self.session.as_ref() {
            session.send_tool_response(std::slice::from_ref(&result));
        }
        self.emit_result(name, result);
    }

    /// Só publica o evento (e grava): para chamadas que o modelo não pediu.
    fn emit_result(&mut self, name: &str, result: ToolResult) {
        let ok = result.error.is_none();
        let summary = engine_tools::result_summary(&result.output, result.error.as_deref());
        if let Some(r) = self.recorder.as_mut() {
            r.event("tool_result", format!("{} ok={ok} {summary}", result.id));
        }
        self.emit.send(EngineEvent::ToolResult {
            id: result.id,
            name: name.to_string(),
            ok,
            summary,
        });
    }

    /// `toolCallCancellation`: descarta pendentes e aborta execuções, sem
    /// responder ao modelo.
    fn cancel_tools(&mut self, ids: &[String]) {
        for id in ids {
            self.pending_learn.remove(id);
            let name = if let Some(at) = self.confirms.iter().position(|p| &p.call.id == id) {
                Some(self.confirms.remove(at).call.name)
            } else {
                self.running.remove(id).map(|(name, task)| {
                    task.abort();
                    name
                })
            };
            let Some(name) = name else { continue };
            if let Some(r) = self.recorder.as_mut() {
                r.event("tool_cancel", id);
            }
            self.emit.send(EngineEvent::ToolResult {
                id: id.clone(),
                name,
                ok: false,
                summary: "cancelada".to_string(),
            });
        }
    }

    /// Sessão perdida ou motor parando: nada pendente sobrevive.
    fn reset_tools(&mut self) {
        self.question = None;
        self.early_approval = None;
        self.model_turn.clear();
        self.turn_repeated = false;
        self.user_heard.clear();
        self.recently_done.clear();
        self.pending_learn.clear();
        if let Some(reflex) = &self.reflex {
            reflex.end_turn();
        }
        let ids: Vec<String> = self
            .confirms
            .iter()
            .map(|p| p.call.id.clone())
            .chain(self.running.keys().cloned())
            .collect();
        self.cancel_tools(&ids);
    }

    fn on_tick(&mut self) {
        if !self.confirms.is_empty() {
            self.expire_confirms();
        }
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
        self.reset_tools();
        self.session = None;
        self.output.flush();
        self.speaking = false;
        self.failed = false;
        self.connecting = true;
        self.update_state();
        self.emit.send(EngineEvent::Reconnecting { attempt: 1 });

        let connected = {
            let connecting = connect(&self.config, &self.tool_specs, &mut self.backend);
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
                        Some(Command::SetFullAccess(on)) => {
                            self.full_access.set(on);
                            self.emit.send(EngineEvent::FullAccess(on));
                        }
                        Some(Command::SetAlwaysAllow(names)) => {
                            self.decisions.set_always_allow(&names);
                        }
                        // Sem sessão ainda: texto digitado durante a conexão é descartado.
                        Some(Command::Reconnect)
                        | Some(Command::ConfirmTool { .. })
                        | Some(Command::UserText(_)) => {}
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

async fn connect(
    config: &EngineConfig,
    tools: &[ToolSpec],
    backend: &mut Backend,
) -> Result<Session, LiveError> {
    match backend {
        Backend::Real => LiveSession::connect(config.live_config(tools))
            .await
            .map(Session::Live),
        #[cfg(test)]
        Backend::Fake(fake) => Ok(Session::Fake(fake.connect())),
        #[cfg(test)]
        Backend::LiveMic(_) => LiveSession::connect(config.live_config(tools))
            .await
            .map(Session::Live),
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
    use std::sync::Arc;

    use tokio::sync::mpsc;

    use super::ToolTimeouts;
    use crate::live::protocol::ServerEvent;
    use crate::reflex::eye::EyeHandle;
    use crate::reflex::judge::Judge;
    use crate::tools::{Registry, ToolResult};

    /// Sessão roteirizada: entrega o que o teste mandar no canal e repassa
    /// as respostas de ferramenta que o motor devolveria ao modelo.
    pub struct FakeSession {
        pub events: mpsc::Receiver<ServerEvent>,
        pub responses: mpsc::UnboundedSender<Vec<ToolResult>>,
    }

    /// Primeira conexão usa o roteiro do teste; as seguintes ficam mudas.
    pub struct FakeBackend {
        script: Option<mpsc::Receiver<ServerEvent>>,
        mic: Option<mpsc::Receiver<Vec<i16>>>,
        silent: Vec<mpsc::Sender<ServerEvent>>,
        pub registry: Registry,
        pub timeouts: ToolTimeouts,
        responses: mpsc::UnboundedSender<Vec<ToolResult>>,
        /// Juiz e Olho dublês: com eles o motor liga o reflexo.
        pub reflex: Option<(Arc<dyn Judge>, EyeHandle)>,
    }

    impl FakeBackend {
        pub fn new(script: mpsc::Receiver<ServerEvent>, mic: mpsc::Receiver<Vec<i16>>) -> Self {
            FakeBackend {
                script: Some(script),
                mic: Some(mic),
                silent: Vec::new(),
                registry: Registry::new(),
                timeouts: ToolTimeouts::default(),
                responses: mpsc::unbounded_channel().0,
                reflex: None,
            }
        }

        pub fn with_reflex(mut self, judge: Arc<dyn Judge>, eye: EyeHandle) -> Self {
            self.reflex = Some((judge, eye));
            self
        }

        pub fn with_tools(
            mut self,
            registry: Registry,
            timeouts: ToolTimeouts,
            responses: mpsc::UnboundedSender<Vec<ToolResult>>,
        ) -> Self {
            self.registry = registry;
            self.timeouts = timeouts;
            self.responses = responses;
            self
        }

        pub fn connect(&mut self) -> FakeSession {
            let events = self.script.take().unwrap_or_else(|| {
                let (tx, rx) = mpsc::channel(1);
                self.silent.push(tx);
                rx
            });
            FakeSession {
                events,
                responses: self.responses.clone(),
            }
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
            tools: Vec::new(),
            mcp_servers: Vec::new(),
            full_access: false,
            always_allow: Vec::new(),
            reflex: Default::default(),
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

    /// Olho de teste: não sonda processos.
    struct NoProbe;
    impl crate::reflex::eye::RunningProbe for NoProbe {
        fn running_app_names(&self) -> Vec<String> {
            vec![]
        }
    }

    fn reflex_settings() -> crate::config::ReflexSettings {
        crate::config::ReflexSettings {
            enabled: true,
            api_key: Some("k".into()),
            debounce_ms: 10,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn texto_digitado_vira_fala_do_usuario_e_passa_pelo_reflexo() {
        use crate::reflex::eye::Eye;
        use crate::reflex::judge::FakeJudge;
        use crate::reflex::questions::{Answer, Answers};
        use std::collections::BTreeMap;

        #[derive(Clone, Default)]
        struct SpyOpen(Arc<Mutex<Vec<serde_json::Value>>>);
        #[async_trait::async_trait]
        impl crate::tools::Tool for SpyOpen {
            fn spec(&self) -> ToolSpec {
                ToolSpec {
                    name: "app.open".into(),
                    description: "spy".into(),
                    parameters: serde_json::json!({"type": "object"}),
                    risk: Risk::Safe,
                }
            }
            async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
                self.0.lock().unwrap().push(args);
                Ok(serde_json::json!({"status": "aberto"}))
            }
        }

        let dir = std::env::temp_dir().join(format!("engine-typed-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Safari.app")).unwrap();
        let eye = Eye::start_with(vec![dir], vec![], Arc::new(NoProbe), Duration::from_secs(3600), Duration::from_secs(3600));
        let judge = Arc::new(FakeJudge::new());
        let mut answers = Answers::default();
        for (id, pick) in [("intent", "open_app"), ("app", "Safari")] {
            let mut p = BTreeMap::new();
            p.insert(pick.to_string(), 0.96);
            p.insert("none".to_string(), 0.04);
            answers.answers.insert(id.into(), Answer::Choice { choice: pick.into(), probabilities: p, confidence: 0.96 });
        }
        judge.push(answers);

        let spy = SpyOpen::default();
        let (_script_tx, script_rx) = mpsc::channel(16);
        let (_mic_tx, mic_rx) = mpsc::channel(16);
        let mut fake = fake::FakeBackend::new(script_rx, mic_rx);
        fake.registry.register(Box::new(spy.clone()));
        let fake = fake.with_reflex(judge.clone(), eye);
        let mut config = test_config();
        config.tools = vec!["*".into()];
        config.reflex = reflex_settings();
        let handle = start_with(config, Backend::Fake(fake)).await.unwrap();
        let mut events = handle.events();

        // vazio é ignorado; texto vira evento UserText e chega ao juiz
        handle.send_user_text("   ");
        handle.send_user_text("  abre o safari  ");
        let heard = loop {
            match next_non_level(&mut events).await {
                EngineEvent::UserText(text) => break text,
                EngineEvent::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        };
        assert_eq!(heard, "abre o safari");
        let acted = loop {
            match next_non_level(&mut events).await {
                EngineEvent::ReflexActed { call, .. } => break call,
                EngineEvent::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        };
        assert_eq!(acted.name, "app.open");
        loop {
            if let EngineEvent::ToolResult { id, ok, .. } = next_non_level(&mut events).await {
                if id == acted.id {
                    assert!(ok);
                    break;
                }
            }
        }
        assert_eq!(spy.0.lock().unwrap().len(), 1);
        assert_eq!(judge.calls().len(), 1, "o juiz foi consultado uma vez, com o texto digitado");
        assert_eq!(judge.calls()[0].0, "abre o safari");
        handle.stop().await;
    }

    #[tokio::test]
    async fn reflexo_abre_app_antes_do_gemini_e_deduplica_a_chamada_dele() {
        use crate::reflex::eye::Eye;
        use crate::reflex::judge::FakeJudge;
        use crate::reflex::questions::{Answer, Answers};
        use std::collections::BTreeMap;

        // Tool fake `app.open` que só registra a chamada.
        #[derive(Clone, Default)]
        struct SpyOpen(Arc<Mutex<Vec<serde_json::Value>>>);
        #[async_trait::async_trait]
        impl crate::tools::Tool for SpyOpen {
            fn spec(&self) -> ToolSpec {
                ToolSpec {
                    name: "app.open".into(),
                    description: "spy".into(),
                    parameters: serde_json::json!({"type": "object"}),
                    risk: Risk::Safe,
                }
            }
            async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
                self.0.lock().unwrap().push(args);
                Ok(serde_json::json!({"status": "aberto"}))
            }
        }

        let dir = std::env::temp_dir().join(format!("engine-reflex-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Safari.app")).unwrap();
        let eye = Eye::start_with(
            vec![dir],
            vec![],
            Arc::new(NoProbe),
            Duration::from_secs(3600),
            Duration::from_secs(3600),
        );
        let judge = Arc::new(FakeJudge::new());
        let mut answers = Answers::default();
        for (id, pick) in [("intent", "open_app"), ("app", "Safari")] {
            let mut p = BTreeMap::new();
            p.insert(pick.to_string(), 0.96);
            p.insert("none".to_string(), 0.04);
            answers.answers.insert(
                id.into(),
                Answer::Choice {
                    choice: pick.into(),
                    probabilities: p,
                    confidence: 0.96,
                },
            );
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
        config.reflex = reflex_settings();
        let handle = start_with(config, Backend::Fake(fake)).await.unwrap();
        let mut events = handle.events();

        script_tx
            .send(ServerEvent::UserText("abre o safari".into()))
            .await
            .unwrap();

        let acted = loop {
            match next_non_level(&mut events).await {
                EngineEvent::ReflexActed { call, .. } => break call,
                EngineEvent::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        };
        assert_eq!(acted.name, "app.open");
        assert!(acted.id.starts_with(crate::reflex::decide::REFLEX_CALL_PREFIX));
        // A tool rodou uma vez e o resultado saiu como evento.
        loop {
            if let EngineEvent::ToolResult { id, ok, .. } = next_non_level(&mut events).await {
                if id == acted.id {
                    assert!(ok);
                    break;
                }
            }
        }
        assert_eq!(spy.0.lock().unwrap().len(), 1);

        // O Gemini chama a mesma ação em seguida: deduplicada, a tool NÃO
        // roda de novo e ele recebe sucesso.
        script_tx
            .send(ServerEvent::ToolCall(vec![ToolCall {
                id: "g1".into(),
                name: "app.open".into(),
                args: serde_json::json!({"name": "Safari"}),
            }]))
            .await
            .unwrap();
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
        use crate::reflex::eye::Eye;
        use crate::reflex::judge::FakeJudge;
        use crate::reflex::questions::{Answer, Answers};

        // Tool `Confirm` fake que conta execuções.
        #[derive(Clone, Default)]
        struct SpyShell(Arc<Mutex<u32>>);
        #[async_trait::async_trait]
        impl crate::tools::Tool for SpyShell {
            fn spec(&self) -> ToolSpec {
                ToolSpec {
                    name: "shell.run".into(),
                    description: "spy".into(),
                    parameters: serde_json::json!({"type": "object"}),
                    risk: Risk::Confirm,
                }
            }
            async fn call(&self, _args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
                *self.0.lock().unwrap() += 1;
                Ok(serde_json::json!({"status": "ok"}))
            }
        }

        let eye = Eye::start_with(
            vec![],
            vec![],
            Arc::new(NoProbe),
            Duration::from_secs(3600),
            Duration::from_secs(3600),
        );
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
        config.reflex = reflex_settings();
        let handle = start_with(config, Backend::Fake(fake)).await.unwrap();
        let mut events = handle.events();

        script_tx
            .send(ServerEvent::ToolCall(vec![ToolCall {
                id: "c1".into(),
                name: "shell.run".into(),
                args: serde_json::json!({"command": "ls"}),
            }]))
            .await
            .unwrap();
        loop {
            if let EngineEvent::ToolConfirmNeeded { .. } = next_non_level(&mut events).await {
                break;
            }
        }

        // Frase fora das listas fixas de engine_tools: só o reflexo entende.
        script_tx
            .send(ServerEvent::UserText("bora, toca ficha".into()))
            .await
            .unwrap();
        loop {
            match next_non_level(&mut events).await {
                EngineEvent::ReflexConfirmed { approve: true, .. } => break,
                EngineEvent::ToolResult { id, .. } if id == "c1" => {
                    panic!("resolveu sem o reflexo")
                }
                _ => {}
            }
        }
        loop {
            if let EngineEvent::ToolResult { id, ok: true, .. } = next_non_level(&mut events).await {
                if id == "c1" {
                    break;
                }
            }
        }
        assert_eq!(*spy.0.lock().unwrap(), 1);
        handle.stop().await;
    }

    /// Tool espiã com nome e risco à escolha: registra os argumentos.
    #[derive(Clone)]
    struct SpyTool {
        name: &'static str,
        risk: Risk,
        calls: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    impl SpyTool {
        fn new(name: &'static str, risk: Risk) -> Self {
            SpyTool { name, risk, calls: Arc::default() }
        }
        fn count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    #[async_trait::async_trait]
    impl crate::tools::Tool for SpyTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.name.into(),
                description: "spy".into(),
                parameters: serde_json::json!({"type": "object"}),
                risk: self.risk,
            }
        }
        async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
            self.calls.lock().unwrap().push(args);
            Ok(serde_json::json!({"status": "ok"}))
        }
    }

    /// Olho vazio (sem apps, sem sites) só com a memória de ações.
    fn empty_eye() -> crate::reflex::EyeHandle {
        crate::reflex::eye::Eye::start_with(
            vec![],
            vec![],
            Arc::new(NoProbe),
            Duration::from_secs(3600),
            Duration::from_secs(3600),
        )
    }

    /// Juiz fake confiante: `intent = learned` e a ação aprendida `key`.
    fn learned_answers(key: &str) -> crate::reflex::questions::Answers {
        use crate::reflex::questions::{Answer, Answers};
        use std::collections::BTreeMap;
        let mut answers = Answers::default();
        for (id, pick) in [("intent", "learned"), ("learned", key)] {
            let mut p = BTreeMap::new();
            p.insert(pick.to_string(), 0.96);
            p.insert("none".to_string(), 0.04);
            answers.answers.insert(
                id.into(),
                Answer::Choice {
                    choice: pick.into(),
                    probabilities: p,
                    confidence: 0.96,
                },
            );
        }
        answers
    }

    async fn start_reflex_engine(
        spy: &SpyTool,
        judge: Arc<crate::reflex::judge::FakeJudge>,
        eye: crate::reflex::EyeHandle,
    ) -> (EngineHandle, broadcast::Receiver<EngineEvent>, mpsc::Sender<ServerEvent>) {
        let (script_tx, script_rx) = mpsc::channel(16);
        let (_mic_tx, mic_rx) = mpsc::channel(16);
        let mut fake = fake::FakeBackend::new(script_rx, mic_rx);
        fake.registry.register(Box::new(spy.clone()));
        let fake = fake.with_reflex(judge, eye);
        let mut config = test_config();
        config.tools = vec!["*".into()];
        config.reflex = reflex_settings();
        let handle = start_with(config, Backend::Fake(fake)).await.unwrap();
        let events = handle.events();
        (handle, events, script_tx)
    }

    async fn wait_tool_result(
        events: &mut broadcast::Receiver<EngineEvent>,
        wanted: &str,
    ) -> (bool, String) {
        loop {
            if let EngineEvent::ToolResult { id, ok, summary, .. } = next_non_level(events).await {
                if id == wanted {
                    return (ok, summary);
                }
            }
        }
    }

    #[tokio::test]
    async fn reflexo_aprende_acao_que_o_gemini_executou() {
        use crate::reflex::judge::FakeJudge;

        let eye = empty_eye();
        let judge = Arc::new(FakeJudge::new()); // sem respostas: o reflexo não age
        let spy = SpyTool::new("web.open", Risk::Safe);
        let (handle, mut events, script_tx) = start_reflex_engine(&spy, judge, eye.clone()).await;

        script_tx
            .send(ServerEvent::UserText("abre a globo".into()))
            .await
            .unwrap();
        let call = ToolCall {
            id: "g1".into(),
            name: "web.open".into(),
            args: serde_json::json!({"url": "https://globo.com"}),
        };
        script_tx
            .send(ServerEvent::ToolCall(vec![call.clone()]))
            .await
            .unwrap();
        assert!(wait_tool_result(&mut events, "g1").await.0);
        assert_eq!(spy.count(), 1);

        let learned = eye.snapshot().learned.clone();
        assert_eq!(learned.len(), 1, "a ação do Gemini ficou aprendida");
        assert_eq!(learned[0].tool, "web.open");
        assert_eq!(learned[0].args, call.args);
        assert_eq!(learned[0].phrase, "abre a globo");
        assert_eq!(learned[0].count, 1);
        handle.stop().await;
    }

    #[tokio::test]
    async fn reflexo_repete_acao_aprendida_sem_o_gemini_e_deduplica() {
        use crate::reflex::judge::FakeJudge;

        let eye = empty_eye();
        let call = ToolCall {
            id: "g0".into(),
            name: "web.open".into(),
            args: serde_json::json!({"url": "https://globo.com"}),
        };
        assert!(eye.learn("abre a globo", &call));
        let judge = Arc::new(FakeJudge::new());
        judge.push(learned_answers("l0"));
        let spy = SpyTool::new("web.open", Risk::Safe);
        let (handle, mut events, script_tx) = start_reflex_engine(&spy, judge, eye).await;

        script_tx
            .send(ServerEvent::UserText("abre a globo".into()))
            .await
            .unwrap();
        let acted = loop {
            match next_non_level(&mut events).await {
                EngineEvent::ReflexActed { call, .. } => break call,
                EngineEvent::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        };
        assert_eq!(acted.name, "web.open");
        assert_eq!(acted.args, call.args, "args gravados na memória");
        assert!(acted.id.starts_with(crate::reflex::decide::REFLEX_CALL_PREFIX));
        assert!(wait_tool_result(&mut events, &acted.id).await.0);
        assert_eq!(spy.count(), 1);

        // O Gemini chama a mesma ação: deduplicada, recebe sucesso sem rodar.
        script_tx
            .send(ServerEvent::ToolCall(vec![ToolCall { id: "g1".into(), ..call }]))
            .await
            .unwrap();
        assert!(wait_tool_result(&mut events, "g1").await.0);
        assert_eq!(spy.count(), 1, "dedup: não executa de novo");
        handle.stop().await;
    }

    /// JRV-83 — trace 19:37:24: "Abrir o Safari" → modelo app.open 48 ms →
    /// reflexo agiu 344 ms depois → app.open de novo. O reflexo não pode
    /// repetir o que o modelo já pediu nesta fala.
    #[tokio::test]
    async fn reflexo_nao_repete_acao_que_o_gemini_ja_pediu_nesta_fala() {
        use crate::reflex::judge::{FakeJudge, JudgeError};

        let eye = empty_eye();
        let call = ToolCall {
            id: "g0".into(),
            name: "web.open".into(),
            args: serde_json::json!({"url": "https://globo.com"}),
        };
        assert!(eye.learn("abre a globo", &call));
        let judge = Arc::new(FakeJudge::with_delay(Duration::from_millis(120)));
        judge.push(learned_answers("l0"));
        judge.push_err(JudgeError::Timeout);
        let spy = SpyTool::new("web.open", Risk::Safe);
        let (handle, mut events, script_tx) = start_reflex_engine(&spy, judge, eye).await;

        // transcrição e chamada do modelo chegam juntas; o juiz demora 120 ms
        script_tx.send(ServerEvent::UserText("abre a globo".into())).await.unwrap();
        script_tx
            .send(ServerEvent::ToolCall(vec![ToolCall { id: "g1".into(), ..call.clone() }]))
            .await
            .unwrap();
        assert!(wait_tool_result(&mut events, "g1").await.0);
        assert_eq!(spy.count(), 1);
        // o reflexo decide depois: não age, e nada executa de novo
        tokio::time::sleep(Duration::from_millis(400)).await;
        while let Ok(ev) = events.try_recv() {
            assert!(!matches!(ev, EngineEvent::ReflexActed { .. }), "reflexo repetiu a ação do modelo");
        }
        assert_eq!(spy.count(), 1, "dedup modelo→reflexo");

        // fala nova, mesma ação: aí o reflexo pode agir de novo
        handle.stop().await;
    }

    #[tokio::test]
    async fn reflexo_acao_aprendida_confirm_pede_confirmacao_e_so_roda_com_sim() {
        use crate::reflex::judge::FakeJudge;

        let eye = empty_eye();
        let call = ToolCall {
            id: "g0".into(),
            name: "shell.run".into(),
            args: serde_json::json!({"command": "ls"}),
        };
        assert!(eye.learn("lista os arquivos", &call));
        let judge = Arc::new(FakeJudge::new());
        judge.push(learned_answers("l0"));
        let spy = SpyTool::new("shell.run", Risk::Confirm);
        let (handle, mut events, script_tx) = start_reflex_engine(&spy, judge, eye).await;

        script_tx
            .send(ServerEvent::UserText("lista os arquivos".into()))
            .await
            .unwrap();
        let pending = loop {
            match next_non_level(&mut events).await {
                EngineEvent::ToolConfirmNeeded { id, name, .. } => break (id, name),
                EngineEvent::ReflexActed { .. } => panic!("agiu sem confirmar"),
                EngineEvent::Error { message, .. } => panic!("{message}"),
                _ => {}
            }
        };
        assert!(pending.0.starts_with(crate::reflex::decide::REFLEX_CALL_PREFIX));
        assert_eq!(pending.1, "shell.run");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(spy.count(), 0, "não roda antes do sim");

        script_tx
            .send(ServerEvent::UserText("sim".into()))
            .await
            .unwrap();
        assert!(wait_tool_result(&mut events, &pending.0).await.0);
        assert_eq!(spy.count(), 1);
        handle.stop().await;
    }

    #[tokio::test]
    async fn chamada_igual_do_gemini_assume_confirmacao_pedida_pelo_reflexo() {
        use crate::reflex::judge::FakeJudge;

        let eye = empty_eye();
        let call = ToolCall {
            id: "g0".into(),
            name: "shell.run".into(),
            args: serde_json::json!({"command": "ls"}),
        };
        assert!(eye.learn("lista os arquivos", &call));
        let judge = Arc::new(FakeJudge::new());
        judge.push(learned_answers("l0"));
        let spy = SpyTool::new("shell.run", Risk::Confirm);
        let (handle, mut events, script_tx) = start_reflex_engine(&spy, judge, eye).await;

        script_tx
            .send(ServerEvent::UserText("lista os arquivos".into()))
            .await
            .unwrap();
        let reflex_id = loop {
            if let EngineEvent::ToolConfirmNeeded { id, .. } = next_non_level(&mut events).await {
                break id;
            }
        };
        // O Gemini pede a mesma ação: o pedido do reflexo sai ("cancelada") e
        // o dele entra no lugar, sem segundo "confirma?" para o usuário.
        script_tx
            .send(ServerEvent::ToolCall(vec![ToolCall { id: "g1".into(), ..call }]))
            .await
            .unwrap();
        let mut cancelled = false;
        loop {
            match next_non_level(&mut events).await {
                EngineEvent::ToolResult { id, ok: false, summary, .. } if id == reflex_id => {
                    assert_eq!(summary, "cancelada");
                    cancelled = true;
                }
                EngineEvent::ToolConfirmNeeded { id, .. } if id == "g1" => break,
                _ => {}
            }
        }
        assert!(cancelled);
        script_tx
            .send(ServerEvent::UserText("sim".into()))
            .await
            .unwrap();
        assert!(wait_tool_result(&mut events, "g1").await.0);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(spy.count(), 1, "um sim, uma execução");
        handle.stop().await;
    }

    mod tool_flow {
        use super::*;
        use async_trait::async_trait;
        use serde_json::{json, Value};

        use crate::tools::Tool;

        /// Ferramenta de teste: devolve os argumentos.
        struct Echo {
            name: &'static str,
            risk: Risk,
        }

        #[async_trait]
        impl Tool for Echo {
            fn spec(&self) -> ToolSpec {
                ToolSpec {
                    name: self.name.to_string(),
                    description: "eco".to_string(),
                    parameters: json!({"type": "object"}),
                    risk: self.risk,
                }
            }

            async fn call(&self, args: Value) -> Result<Value, ToolError> {
                Ok(args)
            }
        }

        struct Harness {
            handle: EngineHandle,
            events: broadcast::Receiver<EngineEvent>,
            script: mpsc::Sender<ServerEvent>,
            responses: mpsc::UnboundedReceiver<Vec<ToolResult>>,
            _mic: mpsc::Sender<Vec<i16>>,
        }

        async fn start_tools(globs: &[&str], confirm: Duration) -> Harness {
            start_tools_with(globs, confirm, false).await
        }

        async fn start_tools_with(globs: &[&str], confirm: Duration, full_access: bool) -> Harness {
            let mut registry = Registry::new();
            registry.register(Box::new(Echo {
                name: "app.open",
                risk: Risk::Safe,
            }));
            registry.register(Box::new(Echo {
                name: "shell.run",
                risk: Risk::Confirm,
            }));
            registry.register(Box::new(Echo {
                name: "fs.write",
                risk: Risk::Confirm,
            }));
            let timeouts = ToolTimeouts {
                voice_window: Duration::from_secs(5),
                confirm,
                exec: Duration::from_secs(5),
            };
            let (script_tx, script_rx) = mpsc::channel(16);
            let (mic_tx, mic_rx) = mpsc::channel(16);
            let (resp_tx, responses) = mpsc::unbounded_channel();
            let backend = Backend::Fake(
                fake::FakeBackend::new(script_rx, mic_rx).with_tools(registry, timeouts, resp_tx),
            );
            let mut config = test_config();
            config.tools = globs.iter().map(|g| g.to_string()).collect();
            config.full_access = full_access;
            let handle = start_with(config, backend).await.expect("motor fake sobe");
            let events = handle.events();
            Harness {
                handle,
                events,
                script: script_tx,
                responses,
                _mic: mic_tx,
            }
        }

        fn call(id: &str, name: &str, args: Value) -> ServerEvent {
            ServerEvent::ToolCall(vec![ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                args,
            }])
        }

        /// Próximo evento de ferramenta, ignorando o resto.
        async fn next_tool_event(rx: &mut broadcast::Receiver<EngineEvent>) -> EngineEvent {
            loop {
                let event = next_non_level(rx).await;
                if matches!(
                    event,
                    EngineEvent::ToolRequested { .. }
                        | EngineEvent::ToolConfirmNeeded { .. }
                        | EngineEvent::ToolResult { .. }
                ) {
                    return event;
                }
            }
        }

        async fn next_response(rx: &mut mpsc::UnboundedReceiver<Vec<ToolResult>>) -> ToolResult {
            let mut batch = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .expect("resposta dentro de 2s")
                .expect("canal aberto");
            assert_eq!(batch.len(), 1);
            batch.remove(0)
        }

        #[tokio::test]
        async fn safe_tool_executes_and_responds() {
            let mut h = start_tools(&["*"], Duration::from_secs(5)).await;
            h.script
                .send(call("c1", "app.open", json!({"name": "Safari"})))
                .await
                .unwrap();

            assert_eq!(
                next_tool_event(&mut h.events).await,
                EngineEvent::ToolRequested {
                    call: ToolCall {
                        id: "c1".into(),
                        name: "app.open".into(),
                        args: json!({"name": "Safari"}),
                    },
                    risk: Risk::Safe,
                }
            );
            let result = next_response(&mut h.responses).await;
            assert_eq!(result, ToolResult::ok("c1", json!({"name": "Safari"})));
            match next_tool_event(&mut h.events).await {
                EngineEvent::ToolResult { id, name, ok, .. } => {
                    assert_eq!((id.as_str(), name.as_str(), ok), ("c1", "app.open", true));
                }
                other => panic!("esperava ToolResult, veio {other:?}"),
            }
            h.handle.stop().await;
        }

        #[tokio::test]
        async fn full_access_skips_confirmation_and_toggles_live() {
            let mut h = start_tools_with(&["*"], Duration::from_secs(5), true).await;
            h.script
                .send(call("f1", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();
            assert!(matches!(
                next_tool_event(&mut h.events).await,
                EngineEvent::ToolRequested { risk: Risk::Safe, .. }
            ));
            let result = next_response(&mut h.responses).await;
            assert_eq!(result, ToolResult::ok("f1", json!({"command": "ls"})));
            assert!(matches!(
                next_tool_event(&mut h.events).await,
                EngineEvent::ToolResult { ok: true, .. }
            ));

            h.handle.set_full_access(false);
            loop {
                if next_non_level(&mut h.events).await == EngineEvent::FullAccess(false) {
                    break;
                }
            }
            h.script
                .send(call("f2", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();
            assert!(matches!(
                next_tool_event(&mut h.events).await,
                EngineEvent::ToolRequested { risk: Risk::Confirm, .. }
            ));
            assert!(matches!(
                next_tool_event(&mut h.events).await,
                EngineEvent::ToolConfirmNeeded { .. }
            ));
            h.handle.stop().await;
        }

        #[tokio::test]
        async fn confirm_without_answer_is_denied_by_timeout() {
            let mut h = start_tools(&["*"], Duration::from_millis(300)).await;
            h.script
                .send(call("c2", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();

            assert!(matches!(
                next_tool_event(&mut h.events).await,
                EngineEvent::ToolRequested { risk: Risk::Confirm, .. }
            ));
            assert_eq!(
                next_tool_event(&mut h.events).await,
                EngineEvent::ToolConfirmNeeded {
                    id: "c2".into(),
                    name: "shell.run".into(),
                    summary: "shell.run: ls".into(),
                }
            );
            let result = next_response(&mut h.responses).await;
            assert_eq!(result.id, "c2");
            let error = result.error.expect("negado vira erro");
            assert!(error.contains("sem confirmação"), "{error}");
            assert!(matches!(
                next_tool_event(&mut h.events).await,
                EngineEvent::ToolResult { ok: false, .. }
            ));
            h.handle.stop().await;
        }

        #[tokio::test]
        async fn voice_yes_approves_confirm() {
            let mut h = start_tools(&["*"], Duration::from_secs(5)).await;
            h.script
                .send(call("c3", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();
            assert!(matches!(
                next_tool_event(&mut h.events).await,
                EngineEvent::ToolRequested { .. }
            ));
            assert!(matches!(
                next_tool_event(&mut h.events).await,
                EngineEvent::ToolConfirmNeeded { .. }
            ));
            // Transcrição picada: "Si" + "m." só forma "sim" junta.
            h.script.send(ServerEvent::UserText("Si".into())).await.unwrap();
            h.script.send(ServerEvent::UserText("m.".into())).await.unwrap();

            let result = next_response(&mut h.responses).await;
            assert_eq!(result, ToolResult::ok("c3", json!({"command": "ls"})));

            // O modelo chama de novo a mesma ação ao ouvir o "sim": não
            // executa nem pergunta outra vez.
            h.script
                .send(call("c3b", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();
            let repeat = next_response(&mut h.responses).await;
            assert_eq!(repeat.id, "c3b");
            assert_eq!(repeat.output["status"], "já executada");
            h.handle.stop().await;
        }

        #[tokio::test]
        async fn yes_to_models_question_approves_the_call_that_follows() {
            let mut h = start_tools(&["*"], Duration::from_secs(5)).await;
            h.script
                .send(ServerEvent::ModelText("Vou criar o card, confirma?".into()))
                .await
                .unwrap();
            h.script.send(ServerEvent::TurnComplete).await.unwrap();
            h.script.send(ServerEvent::UserText(" Sim.".into())).await.unwrap();
            h.script
                .send(call("c8", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();
            let result = next_response(&mut h.responses).await;
            assert_eq!(result, ToolResult::ok("c8", json!({"command": "ls"})));

            // Sem pergunta do modelo, "sim" solto não aprova nada adiante
            // (outra ferramenta: shell.run já está aprovada na sessão).
            h.script.send(ServerEvent::UserText(" sim".into())).await.unwrap();
            h.script
                .send(call("c9", "fs.write", json!({"path": "a.txt"})))
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(300), h.responses.recv())
                    .await
                    .is_err(),
                "c9 precisa esperar confirmação"
            );
            h.handle.stop().await;
        }

        #[tokio::test]
        async fn voice_no_and_handle_resolve_confirms() {
            let mut h = start_tools(&["*"], Duration::from_secs(5)).await;
            h.script
                .send(call("c4", "shell.run", json!({"command": "rm x"})))
                .await
                .unwrap();
            h.script.send(ServerEvent::UserText("não, cancela".into())).await.unwrap();
            let denied = next_response(&mut h.responses).await;
            assert!(denied.error.unwrap().contains("negado"));

            h.script
                .send(call("c5", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();
            loop {
                if let EngineEvent::ToolConfirmNeeded { id, .. } =
                    next_tool_event(&mut h.events).await
                {
                    if id == "c5" {
                        break;
                    }
                }
            }
            h.handle.confirm_tool("c5", true);
            let ok = next_response(&mut h.responses).await;
            assert_eq!(ok.id, "c5");
            assert!(ok.error.is_none());
            h.handle.stop().await;
        }

        /// Espera o `ToolConfirmNeeded` de `id`; falha se `id` vier sem pedir.
        async fn asks(h: &mut Harness, id: &str) -> bool {
            loop {
                match next_tool_event(&mut h.events).await {
                    EngineEvent::ToolConfirmNeeded { id: asked, .. } if asked == id => return true,
                    EngineEvent::ToolResult { id: done, .. } if done == id => return false,
                    _ => {}
                }
            }
        }

        #[tokio::test]
        async fn approved_tool_is_not_asked_again_in_the_session() {
            let mut h = start_tools(&["*"], Duration::from_secs(5)).await;
            h.script
                .send(call("s1", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();
            assert!(asks(&mut h, "s1").await);
            h.script.send(ServerEvent::UserText("sim".into())).await.unwrap();
            assert_eq!(
                next_response(&mut h.responses).await,
                ToolResult::ok("s1", json!({"command": "ls"}))
            );

            // "roda ls" de novo (outro comando, fora da janela de repetição):
            // executa sem perguntar.
            h.script
                .send(call("s2", "shell.run", json!({"command": "ls -la"})))
                .await
                .unwrap();
            assert!(!asks(&mut h, "s2").await);
            assert_eq!(
                next_response(&mut h.responses).await,
                ToolResult::ok("s2", json!({"command": "ls -la"}))
            );

            // Outra ferramenta arriscada ainda pergunta.
            h.script
                .send(call("s3", "fs.write", json!({"path": "a.txt"})))
                .await
                .unwrap();
            assert!(asks(&mut h, "s3").await);
            h.handle.stop().await;
        }

        #[tokio::test]
        async fn always_allow_by_voice_approves_pending_and_later_calls() {
            let mut h = start_tools(&["*"], Duration::from_secs(5)).await;
            h.script
                .send(call("a1", "fs.write", json!({"path": "a.txt"})))
                .await
                .unwrap();
            assert!(asks(&mut h, "a1").await);
            // "não pergunta mais" não pode virar negação.
            h.script
                .send(ServerEvent::UserText("pode, não pergunta mais".into()))
                .await
                .unwrap();
            assert_eq!(
                next_response(&mut h.responses).await,
                ToolResult::ok("a1", json!({"path": "a.txt"}))
            );

            // Sem pedido pendente, "sempre pode rodar comandos" libera shell.run.
            h.script
                .send(ServerEvent::UserText("sempre pode rodar comandos".into()))
                .await
                .unwrap();
            h.script
                .send(call("a2", "shell.run", json!({"command": "pwd"})))
                .await
                .unwrap();
            assert!(!asks(&mut h, "a2").await);

            // Removida nas configurações: volta a perguntar.
            h.handle.set_always_allow(vec!["fs.write".into()]);
            // Comando e roteiro chegam por canais diferentes.
            tokio::time::sleep(Duration::from_millis(50)).await;
            h.script
                .send(call("a3", "shell.run", json!({"command": "whoami"})))
                .await
                .unwrap();
            assert!(asks(&mut h, "a3").await);
            h.handle.stop().await;
        }

        #[tokio::test]
        async fn always_allow_from_config_never_asks() {
            let (script_tx, script_rx) = mpsc::channel(16);
            let (_mic_tx, mic_rx) = mpsc::channel(16);
            let (resp_tx, mut responses) = mpsc::unbounded_channel();
            let mut registry = Registry::new();
            registry.register(Box::new(Echo {
                name: "shell.run",
                risk: Risk::Confirm,
            }));
            let timeouts = ToolTimeouts {
                voice_window: Duration::from_secs(5),
                confirm: Duration::from_secs(5),
                exec: Duration::from_secs(5),
            };
            let backend = Backend::Fake(
                fake::FakeBackend::new(script_rx, mic_rx).with_tools(registry, timeouts, resp_tx),
            );
            let mut config = test_config();
            config.tools = vec!["*".into()];
            config.always_allow = vec!["shell.run".into()];
            let handle = start_with(config, backend).await.expect("motor fake sobe");
            script_tx
                .send(call("p1", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();
            assert_eq!(
                next_response(&mut responses).await,
                ToolResult::ok("p1", json!({"command": "ls"}))
            );
            handle.stop().await;
        }

        #[tokio::test]
        async fn repeated_model_speech_is_dropped() {
            let mut h = start_tools(&["*"], Duration::from_secs(5)).await;
            let turn = async |h: &mut Harness, text: &str| -> Vec<String> {
                h.script
                    .send(ServerEvent::ModelText(text.into()))
                    .await
                    .unwrap();
                h.script.send(ServerEvent::TurnComplete).await.unwrap();
                let mut texts = Vec::new();
                loop {
                    match next_non_level(&mut h.events).await {
                        EngineEvent::ModelText(t) => texts.push(t),
                        EngineEvent::TurnComplete => return texts,
                        _ => {}
                    }
                }
            };
            assert_eq!(turn(&mut h, "Pronto, listei.").await, ["Pronto, listei."]);
            assert!(turn(&mut h, "pronto listei").await.is_empty());
            assert_eq!(turn(&mut h, "Abri o Safari.").await, ["Abri o Safari."]);
            h.handle.stop().await;
        }

        #[tokio::test]
        async fn tool_outside_profile_is_refused_and_cancellation_drops_pending() {
            let mut h = start_tools(&["app.*"], Duration::from_secs(5)).await;
            h.script
                .send(call("c6", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();
            let refused = next_response(&mut h.responses).await;
            assert!(refused.error.unwrap().contains("não permitida"));

            let mut h = start_tools(&["*"], Duration::from_secs(5)).await;
            h.script
                .send(call("c7", "shell.run", json!({"command": "ls"})))
                .await
                .unwrap();
            h.script
                .send(ServerEvent::ToolCallCancellation(vec!["c7".into()]))
                .await
                .unwrap();
            loop {
                if let EngineEvent::ToolResult { id, ok, summary, .. } =
                    next_tool_event(&mut h.events).await
                {
                    assert_eq!((id.as_str(), ok, summary.as_str()), ("c7", false, "cancelada"));
                    break;
                }
            }
            // "sim" depois do cancelamento não executa nada.
            h.script.send(ServerEvent::UserText("sim".into())).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(300), h.responses.recv())
                    .await
                    .is_err(),
                "cancelada não pode responder ao modelo"
            );
            h.handle.stop().await;
        }
    }

    /// Ponta a ponta com a Live API real, as ferramentas reais e os MCPs do
    /// ambiente: fala sintetizada pelo `say` do macOS entra como microfone.
    /// `JRV_LIVE_SAY="abre o Safari" cargo test -p openjarvisbr-core --lib
    /// live_tool_flow -- --ignored --nocapture`. `JRV_LIVE_ANSWER` (padrão
    /// "sim") é dito quando surge um pedido de confirmação. Várias falas na
    /// mesma sessão separadas por `|` ("roda ls|roda ls de novo");
    /// `JRV_LIVE_EXPECT_CONFIRMS` confere quantas vezes o app perguntou.
    #[tokio::test]
    #[ignore = "usa a Live API, o say do macOS e os MCPs reais"]
    async fn live_tool_flow() {
        let said = std::env::var("JRV_LIVE_SAY").unwrap_or_else(|_| "abre o Safari".into());
        let mut utterances: std::collections::VecDeque<String> =
            said.split('|').map(|u| u.trim().to_string()).collect();
        let utterance = utterances.pop_front().unwrap_or_default();
        let answer = std::env::var("JRV_LIVE_ANSWER").unwrap_or_else(|_| "sim".into());
        let settings = crate::config::load_settings();
        let mut config = test_config();
        config.api_key = crate::config::load_api_key().expect("chave da API");
        config.system_prompt = crate::config::effective_system_prompt(&settings);
        config.tools = vec!["*".into()];
        config.mcp_servers = vec![
            McpServerConfig {
                name: "overclock".into(),
                url: Some("http://127.0.0.1:${OVERCLOCK_MCP_PORT}/mcp".into()),
                bearer_env: Some("OVERCLOCK_MCP_BEARER_TOKEN".into()),
                ..Default::default()
            },
            McpServerConfig {
                name: "overclick".into(),
                url: Some("https://cloud.overclock.sh/mcp".into()),
                bearer_env: Some("OVERCLICK_MCP_BEARER_TOKEN".into()),
                ..Default::default()
            },
        ];

        let (mic_tx, mic_rx) = mpsc::channel(64);
        let handle = start_with(config, Backend::LiveMic(Some(mic_rx)))
            .await
            .expect("motor sobe");
        let mut events = handle.events();

        let speak = |text: String, mic: mpsc::Sender<Vec<i16>>| async move {
            let pcm = synthesize(&text);
            println!(">> fala: {text}");
            // Ritmo de tempo real (20ms por bloco) e 2s de silêncio no fim
            // para o VAD do servidor fechar o turno.
            let silence = vec![0i16; 16_000 * 2];
            for chunk in pcm.chunks(320).chain(silence.chunks(320)) {
                if mic.send(chunk.to_vec()).await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        // Silêncio contínuo entre falas, como um microfone real.
        let idle_mic = mic_tx.clone();
        let idle = tokio::spawn(async move {
            loop {
                if idle_mic.send(vec![0i16; 320]).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        tokio::time::sleep(Duration::from_secs(1)).await;
        idle.abort();
        speak(utterance, mic_tx.clone()).await;
        let idle_mic = mic_tx.clone();
        let mut idle = tokio::spawn(async move {
            loop {
                if idle_mic.send(vec![0i16; 320]).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });

        let deadline = Instant::now() + Duration::from_secs(90);
        let mut results = Vec::new();
        let mut results_before = 0;
        let mut confirms = 0;
        let mut asked = false;
        let mut last_activity = Instant::now();
        let mut model_turn = String::new();
        while Instant::now() < deadline {
            // Terminou quando algo executou e a conversa ficou quieta.
            if results.len() > results_before
                && !asked
                && last_activity.elapsed() > Duration::from_secs(8)
            {
                let Some(next) = utterances.pop_front() else {
                    break;
                };
                idle.abort();
                speak(next, mic_tx.clone()).await;
                let idle_mic = mic_tx.clone();
                idle = tokio::spawn(async move {
                    loop {
                        if idle_mic.send(vec![0i16; 320]).await.is_err() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                });
                results_before = results.len();
                last_activity = Instant::now();
                continue;
            }
            let Ok(Ok(event)) =
                tokio::time::timeout(Duration::from_millis(500), events.recv()).await
            else {
                continue;
            };
            if !matches!(event, EngineEvent::Level { .. } | EngineEvent::State(_)) {
                last_activity = Instant::now();
            }
            match &event {
                EngineEvent::Level { .. } | EngineEvent::State(_) => continue,
                EngineEvent::UserText(t) => println!("   usuário: {t}"),
                EngineEvent::ModelText(t) => {
                    model_turn.push_str(t);
                    println!("   jarvis: {t}");
                }
                EngineEvent::TurnComplete => {
                    println!("   [turno]");
                    asked |= engine_tools::asks_confirmation(&std::mem::take(&mut model_turn));
                    if asked {
                        // O modelo terminou de perguntar: responde por voz.
                        asked = false;
                        idle.abort();
                        tokio::time::sleep(Duration::from_millis(900)).await;
                        speak(answer.clone(), mic_tx.clone()).await;
                        let idle_mic = mic_tx.clone();
                        idle = tokio::spawn(async move {
                            loop {
                                if idle_mic.send(vec![0i16; 320]).await.is_err() {
                                    break;
                                }
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                        });
                    }
                }
                EngineEvent::ToolRequested { call, risk } => {
                    println!("   [requested] {} {:?}", engine_tools::call_summary(call), risk)
                }
                EngineEvent::ToolConfirmNeeded { summary, .. } => {
                    println!("   [confirm_needed] {summary}");
                    confirms += 1;
                    asked = true;
                }
                EngineEvent::ToolResult { name, ok, summary, .. } => {
                    println!("   [result] {name} ok={ok}: {summary}");
                    results.push(*ok);
                }
                other => println!("   {other:?}"),
            }
        }
        idle.abort();
        handle.stop().await;
        println!(">> pedidos de confirmação: {confirms}");
        assert!(results.contains(&true), "nenhuma ferramenta executou com sucesso");
        if let Ok(expected) = std::env::var("JRV_LIVE_EXPECT_CONFIRMS") {
            assert_eq!(confirms.to_string(), expected, "pedidos de confirmação");
        }
    }

    /// Texto → PCM 16kHz mono pelo `say` do macOS.
    fn synthesize(text: &str) -> Vec<i16> {
        let path = std::env::temp_dir().join(format!("jrv59-say-{}.wav", std::process::id()));
        let status = std::process::Command::new("say")
            .args(["-v", "Luciana", "--data-format=LEI16@16000", "-o"])
            .arg(&path)
            .arg(text)
            .status()
            .expect("say");
        assert!(status.success());
        let reader = hound::WavReader::open(&path).expect("wav do say");
        reader.into_samples::<i16>().map(|s| s.unwrap()).collect()
    }
}
