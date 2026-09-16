//! Estado da aplicação e loop principal da OpenJarvisBR: conecta a sessão
//! Live, liga o microfone à sessão e a sessão ao playback, imprime
//! transcrições no terminal e trata mute (tecla M) e encerramento (Ctrl+C).

use std::io::{self, Write};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use colored::Colorize;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::audio::capture::{self, CaptureError, CaptureHandle};
use crate::audio::playback::{PlaybackError, Player};
use crate::live::protocol::ServerEvent;
use crate::live::session::{LiveConfig, LiveSession};

/// Gravador de diagnóstico: o que foi pro alto-falante, o que foi pro
/// modelo e a linha do tempo dos eventos. Só existe com `--record`.
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

/// Capacidade dos canais internos entre threads de I/O e o loop async.
const CHANNEL_CAPACITY: usize = 64;
/// Folga depois do último áudio do modelo antes de reabrir o microfone no
/// modo caixa de som: cobre os vãos entre pacotes e a cauda do alto-falante.
const MIC_REOPEN_DELAY: Duration = Duration::from_millis(700);
/// Intervalo de checagem de eventos de teclado (raw mode).
const KEY_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Estados possíveis do assistente durante uma sessão.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Connecting,
    Listening,
    Speaking,
    Error,
}

/// Parâmetros necessários para abrir uma sessão de conversa.
pub struct AppConfig {
    pub api_key: String,
    pub voice: String,
    pub device_in: Option<String>,
    pub device_out: Option<String>,
    /// Mic aberto enquanto o modelo fala (interrupção por voz). Desligado por
    /// padrão para evitar eco com caixa de som.
    pub barge_in: bool,
    /// Pasta onde gravar playback.wav, mic.wav e events.log (diagnóstico).
    pub record_dir: Option<std::path::PathBuf>,
}

/// Aplicação principal: mantém o estado atual da sessão.
pub struct App {
    config: AppConfig,
    state: State,
}

impl App {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            state: State::Idle,
        }
    }

    /// Executa o loop principal: conecta, liga microfone/playback à sessão e
    /// só retorna quando a conversa termina (Ctrl+C, sessão fechada ou erro
    /// fatal). Retorna o código de saída do processo, conforme a tabela de
    /// erros da spec (2 chave, 3 áudio, 4 socket, 5 quota).
    pub async fn run(&mut self) -> i32 {
        self.state = State::Connecting;
        info!("conectando à Live API");
        let cfg = LiveConfig::new(self.config.api_key.clone(), self.config.voice.clone());
        let mut session = match LiveSession::connect(cfg).await {
            Ok(session) => session,
            Err(err) => {
                self.state = State::Error;
                eprintln!("erro ao conectar: {err}");
                return err.exit_code();
            }
        };

        let (std_tx, std_rx) = std_mpsc::channel::<Vec<i16>>();
        let capture_handle = match start_capture(self.config.device_in.as_deref(), std_tx) {
            Ok(handle) => handle,
            Err(code) => {
                self.state = State::Error;
                return code;
            }
        };

        let player = match Player::new(self.config.device_out.as_deref()) {
            Ok(player) => player,
            Err(err) => {
                self.state = State::Error;
                report_playback_error(&err);
                return 3;
            }
        };

        let raw_mode = RawMode::enable();
        let mut audio_rx = bridge_capture_channel(std_rx);
        let (mut control_rx, _control_tx) = spawn_key_thread(raw_mode.enabled);

        self.state = State::Listening;
        print_line("ouvindo — fale quando quiser (M muta o microfone, Ctrl+C encerra)");
        if !self.config.barge_in {
            print_line("modo caixa de som: espere o Jarvis terminar pra falar (--barge-in libera interrupção por voz, use com fone)".dimmed());
        }

        let mut muted = false;
        let mut transcript = Transcript::default();
        let mut last_model_audio: Option<Instant> = None;
        let mut recorder = match &self.config.record_dir {
            Some(dir) => match Recorder::open(dir) {
                Ok(r) => {
                    print_line(format!("gravando diagnóstico em {}", dir.display()).dimmed());
                    Some(r)
                }
                Err(err) => {
                    eprintln!("não foi possível gravar em {}: {err}", dir.display());
                    None
                }
            },
            None => None,
        };
        let exit_code = loop {
            tokio::select! {
                chunk = audio_rx.recv() => {
                    match chunk {
                        Some(samples) => {
                            // Half-duplex: sem fone, o mic capta a voz do
                            // próprio Jarvis e o servidor a trata como fala
                            // do usuário. Só enviamos enquanto ele está calado.
                            let recently_spoke = last_model_audio
                                .is_some_and(|t| t.elapsed() < MIC_REOPEN_DELAY);
                            let gated = !self.config.barge_in
                                && (player.is_playing() || recently_spoke);
                            if !muted && !gated {
                                if let Some(r) = recorder.as_mut() {
                                    r.mic(&samples);
                                }
                                session.send_audio(&samples);
                            }
                        }
                        None => {
                            warn!("captura de áudio encerrada inesperadamente");
                        }
                    }
                }
                event = session.next_event() => {
                    match event {
                        Some(ServerEvent::Audio(samples)) => {
                            self.state = State::Speaking;
                            last_model_audio = Some(Instant::now());
                            if let Some(r) = recorder.as_mut() {
                                r.event("audio", format!("{} amostras, fila={}", samples.len(), player.queued()));
                                r.playback(&samples);
                            }
                            player.push(&samples);
                        }
                        Some(ServerEvent::Interrupted) => {
                            if let Some(r) = recorder.as_mut() {
                                r.event("interrupted", format!("fila descartada={}", player.queued()));
                            }
                            player.flush();
                            transcript.end_line();
                            self.state = State::Listening;
                        }
                        Some(ServerEvent::UserText(text)) => {
                            if let Some(r) = recorder.as_mut() {
                                r.event("user_text", &text);
                            }
                            transcript.user(&text);
                        }
                        Some(ServerEvent::ModelText(text)) => {
                            if let Some(r) = recorder.as_mut() {
                                r.event("model_text", &text);
                            }
                            transcript.model(&text);
                        }
                        Some(ServerEvent::TurnComplete) => {
                            if let Some(r) = recorder.as_mut() {
                                r.event("turn_complete", "");
                            }
                            player.end_of_turn();
                            transcript.end_line();
                        }
                        Some(ServerEvent::GoAway) => {
                            if let Some(r) = recorder.as_mut() {
                                r.event("go_away", "reconectando");
                            }
                            info!("servidor pediu encerramento (goAway); reconectando");
                        }
                        Some(ServerEvent::Closed) | None => {
                            transcript.end_line();
                            break match session.error() {
                                Some(err) => {
                                    self.state = State::Error;
                                    eprintln!("sessão encerrada: {err}");
                                    err.exit_code()
                                }
                                None => 0,
                            };
                        }
                    }
                }
                Some(control) = control_rx.recv() => {
                    match control {
                        ControlEvent::ToggleMute => {
                            muted = !muted;
                            transcript.end_line();
                            print_line(if muted { "[mic mudo]".yellow() } else { "[mic ativo]".yellow() });
                        }
                        ControlEvent::Quit => {
                            info!("Ctrl+C recebido, encerrando");
                            break 0;
                        }
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("Ctrl+C recebido, encerrando");
                    break 0;
                }
            }
        };

        if let Some(r) = recorder.take() {
            r.finish();
        }
        capture_handle.stop();
        drop(raw_mode);
        exit_code
    }
}

/// Eventos de teclado tratados pelo loop principal.
enum ControlEvent {
    ToggleMute,
    Quit,
}

/// Guarda o modo raw do terminal e o desativa ao sair de escopo. Quando o
/// terminal não é interativo (sem TTY), a ativação falha e a app segue sem
/// leitura de teclado — Ctrl+C continua funcionando via `tokio::signal`.
struct RawMode {
    enabled: bool,
}

impl RawMode {
    fn enable() -> Self {
        match terminal::enable_raw_mode() {
            Ok(()) => RawMode { enabled: true },
            Err(err) => {
                warn!(
                    erro = %err,
                    "não foi possível ativar o modo raw do terminal; tecla M indisponível (Ctrl+C do sistema ainda funciona)"
                );
                RawMode { enabled: false }
            }
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if self.enabled {
            let _ = terminal::disable_raw_mode();
        }
    }
}

/// Abre o microfone e reporta erro de áudio (dispositivos disponíveis ou
/// dica de permissão no Mac) já formatado para o usuário.
fn start_capture(
    device_in: Option<&str>,
    tx: std_mpsc::Sender<Vec<i16>>,
) -> Result<CaptureHandle, i32> {
    capture::start(device_in, tx).map_err(|err| {
        eprintln!("erro de áudio (microfone): {err}");
        match err {
            CaptureError::NoInputDevice | CaptureError::DeviceNotFound(_) => {
                eprintln!("dispositivos de entrada disponíveis:");
                for name in capture::list_input_devices() {
                    eprintln!("  - {name}");
                }
            }
            _ => eprintln!(
                "se a permissão de microfone foi negada, confira: Ajustes › Privacidade e Segurança › Microfone"
            ),
        }
        3
    })
}

fn report_playback_error(err: &PlaybackError) {
    eprintln!("erro de áudio (saída): {err}");
    if let PlaybackError::NoOutputDevice | PlaybackError::DeviceNotFound(_) = err {
        eprintln!("verifique se há um dispositivo de saída de áudio conectado e selecionado.");
    }
}

/// Encaminha os chunks do canal síncrono do `cpal` (thread própria do host de
/// áudio) para um canal `tokio` que o loop principal pode `select!`ar.
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

/// Lê eventos de teclado em uma thread própria (bloqueante) quando o modo raw
/// está ativo. O `Sender` retornado deve ser mantido vivo pelo chamador: sem
/// ele, o canal fecha e o `select!` do loop principal gira sem parar.
fn spawn_key_thread(enabled: bool) -> (mpsc::Receiver<ControlEvent>, mpsc::Sender<ControlEvent>) {
    let (tx, rx) = mpsc::channel(8);
    if enabled {
        let thread_tx = tx.clone();
        std::thread::spawn(move || loop {
            match event::poll(KEY_POLL_INTERVAL) {
                Ok(true) => {
                    if let Ok(Event::Key(key)) = event::read() {
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }
                        let control = match key.code {
                            KeyCode::Char('m') | KeyCode::Char('M') => {
                                Some(ControlEvent::ToggleMute)
                            }
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                Some(ControlEvent::Quit)
                            }
                            _ => None,
                        };
                        if let Some(control) = control {
                            if thread_tx.blocking_send(control).is_err() {
                                break;
                            }
                        }
                    }
                }
                Ok(false) => {}
                Err(_) => break,
            }
        });
    }
    (rx, tx)
}

/// Quem está com a palavra na linha corrente do terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Speaker {
    User,
    Model,
}

/// Junta os pedaços de transcrição que a Live API manda em chunks numa
/// única linha por turno: o prefixo só aparece quando o falante muda, e a
/// linha só fecha em `end_line` (turno completo, interrupção, mute, saída).
#[derive(Default)]
struct Transcript {
    current: Option<Speaker>,
}

impl Transcript {
    fn user(&mut self, text: &str) {
        self.append(Speaker::User, text);
    }

    fn model(&mut self, text: &str) {
        self.append(Speaker::Model, text);
    }

    fn append(&mut self, speaker: Speaker, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        if self.current != Some(speaker) {
            self.end_line();
            let prefix = match speaker {
                Speaker::User => "Você:".bright_cyan().bold(),
                Speaker::Model => "OpenJarvisBR:".bright_green().bold(),
            };
            print!("{prefix} ");
            self.current = Some(speaker);
        }
        print!("{text}");
        let _ = io::stdout().flush();
    }

    fn end_line(&mut self) {
        if self.current.take().is_some() {
            print!("\r\n");
            let _ = io::stdout().flush();
        }
    }
}

/// `print!` com `\r\n` explícito: em modo raw o terminal não traduz `\n`
/// sozinho, então `println!` puro deixaria a saída "em escada".
fn print_line(line: impl std::fmt::Display) {
    print!("{line}\r\n");
    let _ = io::stdout().flush();
}
