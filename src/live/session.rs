//! Sessão WebSocket com a Gemini Live API.
//!
//! `LiveSession::connect` abre o socket (tokio-tungstenite + rustls), envia o
//! setup e espera `setupComplete`. Depois disso uma task em background é dona
//! do socket: envia o áudio recebido por `send_audio`, entrega os eventos em
//! `next_event` e reconecta em `goAway` ou queda com backoff 1s/2s/4s (máx. 3
//! tentativas), reenviando as últimas transcrições como contexto de texto.
//!
//! A URL de conexão carrega a chave em `?key=` — ela nunca vai para log nem
//! para mensagem de erro: só o host é logado.
//!
//! `app.rs` (próximo card) é quem usa este módulo; até lá, itens ainda não
//! referenciados por fora não devem acender `dead_code`.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, info, warn};

use super::protocol::{
    self, ClientContent, ClientContentRequest, ProtocolError, RealtimeInputRequest, ServerEvent,
    SetupRequest, TextPart, Turn,
};

/// Host da Live API — único pedaço do endereço que pode ir para log.
pub const HOST: &str = "generativelanguage.googleapis.com";
const PATH: &str = "/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

/// Tempo máximo esperando `setupComplete` depois de enviar o setup.
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Quantas linhas de transcrição guardar para reenviar na reconexão.
const HISTORY_LINES: usize = 20;
/// Capacidade dos canais entre a sessão e a task do socket.
const CHANNEL_CAPACITY: usize = 256;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Sink = SplitSink<Socket, Message>;
type Stream = SplitStream<Socket>;

/// Parâmetros da sessão. `Debug` é manual para nunca imprimir a chave.
#[derive(Clone)]
pub struct LiveConfig {
    pub api_key: String,
    pub voice: String,
}

impl LiveConfig {
    pub fn new(api_key: impl Into<String>, voice: impl Into<String>) -> Self {
        LiveConfig {
            api_key: api_key.into(),
            voice: voice.into(),
        }
    }

    /// URL completa com a chave. Uso interno exclusivo do handshake.
    fn url(&self) -> String {
        format!("wss://{HOST}{PATH}?key={}", self.api_key)
    }
}

impl std::fmt::Debug for LiveConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveConfig")
            .field("api_key", &"***")
            .field("voice", &self.voice)
            .finish()
    }
}

/// Erros da sessão. Nenhuma variante carrega corpo bruto de resposta, URL ou
/// chave — mensagens de terceiros passam por `scrub` antes de entrar aqui.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LiveError {
    #[error("chave da API do Gemini inválida ou sem permissão (401)")]
    Unauthorized,
    #[error("quota da API do Gemini esgotada ou limite de taxa atingido (429)")]
    RateLimited,
    #[error("falha ao conectar em {HOST}: {0}")]
    Connect(String),
    #[error("servidor não confirmou o setup em {}s", SETUP_TIMEOUT.as_secs())]
    SetupTimeout,
    #[error("conexão fechada pelo servidor antes do setup: {0}")]
    Closed(String),
    #[error("reconexão falhou após {0} tentativas")]
    GaveUp(u32),
}

impl LiveError {
    /// Código de saída do processo, conforme a tabela de erros da spec.
    pub fn exit_code(&self) -> i32 {
        match self {
            LiveError::Unauthorized => 2,
            LiveError::RateLimited => 5,
            _ => 4,
        }
    }
}

/// Backoff da reconexão: 1s, 2s, 4s e depois desiste.
#[derive(Debug, Default, Clone)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    pub const MAX_ATTEMPTS: u32 = 3;

    pub fn new() -> Self {
        Backoff::default()
    }

    /// Espera antes da próxima tentativa, ou `None` quando esgotou.
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.attempt >= Self::MAX_ATTEMPTS {
            return None;
        }
        let delay = Duration::from_secs(1 << self.attempt);
        self.attempt += 1;
        Some(delay)
    }

    pub fn attempts(&self) -> u32 {
        self.attempt
    }
}

/// Sessão ativa. `send_audio` e `next_event` podem ser usados de pontas
/// diferentes do loop da app; o socket em si vive numa task própria.
pub struct LiveSession {
    audio_tx: mpsc::Sender<Vec<i16>>,
    events: mpsc::Receiver<ServerEvent>,
    fatal: Arc<Mutex<Option<LiveError>>>,
    task: tokio::task::JoinHandle<()>,
}

impl LiveSession {
    /// Abre o socket, envia o setup e espera `setupComplete`.
    pub async fn connect(cfg: LiveConfig) -> Result<LiveSession, LiveError> {
        let (sink, stream) = open(&cfg, None).await?;

        let (audio_tx, audio_rx) = mpsc::channel(CHANNEL_CAPACITY);
        let (event_tx, events) = mpsc::channel(CHANNEL_CAPACITY);
        let fatal = Arc::new(Mutex::new(None));

        let worker = Worker {
            cfg,
            sink,
            stream,
            audio_rx,
            event_tx,
            history: History::default(),
            fatal: Arc::clone(&fatal),
        };
        let task = tokio::spawn(worker.run());

        Ok(LiveSession {
            audio_tx,
            events,
            fatal,
            task,
        })
    }

    /// Enfileira um chunk de PCM i16 16kHz mono. Se a fila estiver cheia
    /// (ex.: durante uma reconexão) o chunk é descartado — áudio velho não
    /// serve para conversa em tempo real.
    pub fn send_audio(&self, samples: &[i16]) {
        if self.audio_tx.try_send(samples.to_vec()).is_err() {
            debug!("chunk de áudio descartado (sessão ocupada ou encerrada)");
        }
    }

    /// Próximo evento do servidor. `Closed` sinaliza fim da sessão; depois
    /// dele vem `None`. Se o fim foi por erro, `error()` diz qual.
    pub async fn next_event(&mut self) -> Option<ServerEvent> {
        self.events.recv().await
    }

    /// Erro fatal que encerrou a sessão, se houver.
    pub fn error(&self) -> Option<LiveError> {
        self.fatal.lock().ok().and_then(|guard| guard.clone())
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Últimas transcrições, agrupando fragmentos consecutivos do mesmo lado.
#[derive(Debug, Default)]
struct History {
    lines: VecDeque<(Speaker, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Speaker {
    User,
    Model,
}

impl History {
    fn push(&mut self, speaker: Speaker, fragment: &str) {
        if fragment.is_empty() {
            return;
        }
        match self.lines.back_mut() {
            Some((last, text)) if *last == speaker => text.push_str(fragment),
            _ => {
                self.lines.push_back((speaker, fragment.to_string()));
                if self.lines.len() > HISTORY_LINES {
                    self.lines.pop_front();
                }
            }
        }
    }

    /// Texto de contexto para reenviar na reconexão, ou `None` sem histórico.
    fn context(&self) -> Option<String> {
        if self.lines.is_empty() {
            return None;
        }
        let mut out =
            String::from("Contexto: a conexão caiu e foi retomada. Últimas falas da conversa:\n");
        for (speaker, text) in &self.lines {
            let who = match speaker {
                Speaker::User => "Usuário",
                Speaker::Model => "OpenJarvisBR",
            };
            out.push_str(&format!("{who}: {}\n", text.trim()));
        }
        Some(out)
    }
}

struct Worker {
    cfg: LiveConfig,
    sink: Sink,
    stream: Stream,
    audio_rx: mpsc::Receiver<Vec<i16>>,
    event_tx: mpsc::Sender<ServerEvent>,
    history: History,
    fatal: Arc<Mutex<Option<LiveError>>>,
}

enum Step {
    Continue,
    Reconnect,
    Stop,
}

impl Worker {
    async fn run(mut self) {
        loop {
            let step = tokio::select! {
                chunk = self.audio_rx.recv() => match chunk {
                    Some(samples) => self.send_audio(&samples).await,
                    None => Step::Stop,
                },
                msg = self.stream.next() => self.handle(msg).await,
            };

            match step {
                Step::Continue => {}
                Step::Stop => break,
                Step::Reconnect => {
                    if let Err(err) = self.reconnect().await {
                        warn!(host = HOST, erro = %err, "sessão encerrada");
                        if let Ok(mut guard) = self.fatal.lock() {
                            *guard = Some(err);
                        }
                        break;
                    }
                }
            }
        }
        let _ = self.sink.close().await;
        let _ = self.event_tx.send(ServerEvent::Closed).await;
    }

    async fn send_audio(&mut self, samples: &[i16]) -> Step {
        let Ok(json) = serde_json::to_string(&RealtimeInputRequest::from_pcm(samples)) else {
            return Step::Continue;
        };
        match self.sink.send(Message::text(json)).await {
            Ok(()) => Step::Continue,
            Err(err) => {
                warn!(host = HOST, erro = %scrub(&err.to_string(), &self.cfg.api_key), "falha ao enviar áudio");
                Step::Reconnect
            }
        }
    }

    async fn handle(&mut self, msg: Option<Result<Message, tungstenite::Error>>) -> Step {
        let raw = match msg {
            Some(Ok(Message::Text(text))) => text.to_string(),
            Some(Ok(Message::Binary(bytes))) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => text,
                Err(_) => return Step::Continue,
            },
            Some(Ok(Message::Close(frame))) => {
                warn!(host = HOST, motivo = %close_reason(frame.as_ref()), "servidor fechou o socket");
                return Step::Reconnect;
            }
            Some(Ok(_)) => return Step::Continue,
            Some(Err(err)) => {
                warn!(host = HOST, erro = %scrub(&err.to_string(), &self.cfg.api_key), "erro no socket");
                return Step::Reconnect;
            }
            None => {
                warn!(host = HOST, "socket encerrado");
                return Step::Reconnect;
            }
        };

        let event = match protocol::parse(&raw) {
            Ok(event) => event,
            Err(ProtocolError::NoEvent) => return Step::Continue,
            Err(err) => {
                debug!(erro = %err, "mensagem ignorada");
                return Step::Continue;
            }
        };

        let step = match &event {
            ServerEvent::UserText(text) => {
                self.history.push(Speaker::User, text);
                Step::Continue
            }
            ServerEvent::ModelText(text) => {
                self.history.push(Speaker::Model, text);
                Step::Continue
            }
            ServerEvent::GoAway => {
                info!(host = HOST, "goAway recebido, reconectando");
                Step::Reconnect
            }
            _ => Step::Continue,
        };

        if self.event_tx.send(event).await.is_err() {
            return Step::Stop;
        }
        step
    }

    async fn reconnect(&mut self) -> Result<(), LiveError> {
        let _ = self.sink.close().await;
        let context = self.history.context();
        let mut backoff = Backoff::new();

        while let Some(delay) = backoff.next_delay() {
            info!(
                host = HOST,
                tentativa = backoff.attempts(),
                espera_s = delay.as_secs(),
                "reconectando"
            );
            tokio::time::sleep(delay).await;
            match open(&self.cfg, context.as_deref()).await {
                Ok((sink, stream)) => {
                    self.sink = sink;
                    self.stream = stream;
                    // Áudio acumulado durante a queda já está velho.
                    while self.audio_rx.try_recv().is_ok() {}
                    info!(host = HOST, "reconectado");
                    return Ok(());
                }
                Err(err @ (LiveError::Unauthorized | LiveError::RateLimited)) => return Err(err),
                Err(err) => warn!(host = HOST, erro = %err, "tentativa de reconexão falhou"),
            }
        }
        Err(LiveError::GaveUp(Backoff::MAX_ATTEMPTS))
    }
}

/// Handshake + setup + `setupComplete` (+ contexto de texto, se houver).
async fn open(cfg: &LiveConfig, context: Option<&str>) -> Result<(Sink, Stream), LiveError> {
    install_crypto_provider();
    info!(host = HOST, "conectando na Live API");

    let (socket, _response) = tokio_tungstenite::connect_async(cfg.url())
        .await
        .map_err(|err| handshake_error(err, &cfg.api_key))?;
    let (mut sink, mut stream) = socket.split();

    let setup = serde_json::to_string(&SetupRequest::new(cfg.voice.clone()))
        .map_err(|err| LiveError::Connect(err.to_string()))?;
    sink.send(Message::text(setup))
        .await
        .map_err(|err| LiveError::Connect(scrub(&err.to_string(), &cfg.api_key)))?;

    tokio::time::timeout(
        SETUP_TIMEOUT,
        wait_setup_complete(&mut stream, &cfg.api_key),
    )
    .await
    .map_err(|_| LiveError::SetupTimeout)??;
    debug!(host = HOST, "setupComplete recebido");

    if let Some(text) = context {
        let request = ClientContentRequest {
            client_content: ClientContent {
                turns: vec![Turn {
                    role: "user".to_string(),
                    parts: vec![TextPart {
                        text: text.to_string(),
                    }],
                }],
                // Só contexto: o modelo não deve responder a este turno.
                turn_complete: false,
            },
        };
        let json =
            serde_json::to_string(&request).map_err(|err| LiveError::Connect(err.to_string()))?;
        sink.send(Message::text(json))
            .await
            .map_err(|err| LiveError::Connect(scrub(&err.to_string(), &cfg.api_key)))?;
        debug!(host = HOST, "contexto das últimas transcrições reenviado");
    }

    Ok((sink, stream))
}

async fn wait_setup_complete(stream: &mut Stream, api_key: &str) -> Result<(), LiveError> {
    while let Some(msg) = stream.next().await {
        let raw = match msg {
            Ok(Message::Text(text)) => text.to_string(),
            Ok(Message::Binary(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            Ok(Message::Close(frame)) => return Err(close_error(frame.as_ref())),
            Ok(_) => continue,
            Err(err) => return Err(LiveError::Connect(scrub(&err.to_string(), api_key))),
        };
        if is_setup_complete(&raw) {
            return Ok(());
        }
    }
    Err(LiveError::Closed("socket encerrado".to_string()))
}

fn is_setup_complete(raw: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(raw)
        .map(|value| value.get("setupComplete").is_some())
        .unwrap_or(false)
}

fn install_crypto_provider() {
    // Falha só quando já existe um provider instalado — o que basta.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Converte erro do handshake HTTP em erro tipado, sem corpo bruto.
fn handshake_error(err: tungstenite::Error, api_key: &str) -> LiveError {
    match err {
        tungstenite::Error::Http(response) => match response.status().as_u16() {
            401 | 403 => LiveError::Unauthorized,
            429 => LiveError::RateLimited,
            status => LiveError::Connect(format!("HTTP {status}")),
        },
        other => LiveError::Connect(scrub(&other.to_string(), api_key)),
    }
}

/// A Live API costuma recusar chave/quota fechando o socket com um motivo
/// textual depois do setup; classifica esse motivo em erro tipado.
fn close_error(frame: Option<&CloseFrame>) -> LiveError {
    let reason = close_reason(frame);
    classify_close(&reason).unwrap_or(LiveError::Closed(reason))
}

fn classify_close(reason: &str) -> Option<LiveError> {
    let lower = reason.to_lowercase();
    if ["api key", "api_key", "unauthenticated", "permission", "401"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return Some(LiveError::Unauthorized);
    }
    if ["quota", "resource_exhausted", "rate limit", "429"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return Some(LiveError::RateLimited);
    }
    None
}

/// Resumo curto do close frame (código + primeiros caracteres do motivo).
fn close_reason(frame: Option<&CloseFrame>) -> String {
    match frame {
        Some(frame) => {
            let reason: String = frame.reason.chars().take(120).collect();
            format!("{} {reason}", u16::from(frame.code))
        }
        None => "sem close frame".to_string(),
    }
}

/// Remove a chave (e qualquer URL com `key=`) de uma mensagem de terceiros.
fn scrub(message: &str, api_key: &str) -> String {
    let mut out = if api_key.is_empty() {
        message.to_string()
    } else {
        message.replace(api_key, "***")
    };
    if let Some(at) = out.find("key=") {
        let end = out[at..]
            .find(|c: char| c.is_whitespace() || c == '&' || c == '"')
            .map_or(out.len(), |offset| at + offset);
        out.replace_range(at..end, "key=***");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_one_two_four_then_gives_up() {
        let mut backoff = Backoff::new();
        assert_eq!(backoff.next_delay(), Some(Duration::from_secs(1)));
        assert_eq!(backoff.next_delay(), Some(Duration::from_secs(2)));
        assert_eq!(backoff.next_delay(), Some(Duration::from_secs(4)));
        assert_eq!(
            backoff.next_delay(),
            None,
            "a 4ª tentativa não deve existir"
        );
        assert_eq!(backoff.attempts(), Backoff::MAX_ATTEMPTS);
    }

    #[test]
    fn config_debug_never_shows_key() {
        let cfg = LiveConfig::new("segredo-de-teste", "Puck");
        let shown = format!("{cfg:?}");
        assert!(!shown.contains("segredo-de-teste"));
        assert!(shown.contains("***"));
    }

    #[test]
    fn scrub_masks_key_and_key_param() {
        let msg = "erro em wss://host/path?key=segredo&x=1 (segredo)";
        let clean = scrub(msg, "segredo");
        assert!(!clean.contains("segredo"));
        assert!(clean.contains("key=***"));
    }

    #[test]
    fn errors_never_mention_url_or_key() {
        for err in [
            LiveError::Unauthorized,
            LiveError::RateLimited,
            LiveError::Connect("x".into()),
            LiveError::GaveUp(3),
        ] {
            let text = err.to_string();
            assert!(!text.contains("key="), "{text}");
            assert!(!text.contains("wss://"), "{text}");
        }
    }

    #[test]
    fn exit_codes_follow_spec() {
        assert_eq!(LiveError::Unauthorized.exit_code(), 2);
        assert_eq!(LiveError::RateLimited.exit_code(), 5);
        assert_eq!(LiveError::GaveUp(3).exit_code(), 4);
    }

    #[test]
    fn close_reasons_become_typed_errors() {
        assert_eq!(
            classify_close("1007 API key not valid. Please pass a valid API key."),
            Some(LiveError::Unauthorized)
        );
        assert_eq!(
            classify_close("1011 You exceeded your current quota"),
            Some(LiveError::RateLimited)
        );
        assert_eq!(classify_close("1000 bye"), None);
    }

    #[test]
    fn detects_setup_complete() {
        assert!(is_setup_complete(r#"{"setupComplete": {}}"#));
        assert!(!is_setup_complete(r#"{"serverContent": {}}"#));
        assert!(!is_setup_complete("não é json"));
    }

    #[test]
    fn history_merges_fragments_and_builds_context() {
        let mut history = History::default();
        history.push(Speaker::User, "oi, ");
        history.push(Speaker::User, "quem é você");
        history.push(Speaker::Model, "eu sou a OpenJarvisBR");
        let context = history.context().unwrap();
        assert!(context.contains("Usuário: oi, quem é você"));
        assert!(context.contains("OpenJarvisBR: eu sou a OpenJarvisBR"));
    }

    #[test]
    fn history_keeps_only_last_lines() {
        let mut history = History::default();
        for i in 0..(HISTORY_LINES + 5) {
            let speaker = if i % 2 == 0 {
                Speaker::User
            } else {
                Speaker::Model
            };
            history.push(speaker, &format!("fala {i}"));
        }
        assert_eq!(history.lines.len(), HISTORY_LINES);
        let context = history.context().unwrap();
        assert!(context.contains(&format!("fala {}\n", HISTORY_LINES + 4)));
        assert!(!context.contains("fala 0\n"));
    }

    #[test]
    fn empty_history_has_no_context() {
        assert!(History::default().context().is_none());
    }
}
