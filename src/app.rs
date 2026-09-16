//! Estado da aplicação e loop principal da OpenJarvisBR: conecta a sessão
//! Live, liga o microfone à sessão e a sessão ao playback, imprime
//! transcrições no terminal e trata mute (tecla M) e encerramento (Ctrl+C).

use std::io::{self, Write};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use colored::Colorize;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::audio::capture::{self, CaptureError, CaptureHandle};
use crate::audio::playback::{PlaybackError, Player};
use crate::live::protocol::ServerEvent;
use crate::live::session::{LiveConfig, LiveSession};

/// Capacidade dos canais internos entre threads de I/O e o loop async.
const CHANNEL_CAPACITY: usize = 64;
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

        let mut muted = false;
        let exit_code = loop {
            tokio::select! {
                chunk = audio_rx.recv() => {
                    match chunk {
                        Some(samples) => {
                            if !muted {
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
                            player.push(&samples);
                        }
                        Some(ServerEvent::Interrupted) => {
                            player.flush();
                            self.state = State::Listening;
                        }
                        Some(ServerEvent::UserText(text)) => print_user(&text),
                        Some(ServerEvent::ModelText(text)) => print_model(&text),
                        Some(ServerEvent::GoAway) => {
                            info!("servidor pediu encerramento (goAway); reconectando");
                        }
                        Some(ServerEvent::Closed) | None => {
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

fn print_user(text: &str) {
    if text.trim().is_empty() {
        return;
    }
    print_line(format!("{} {text}", "Você:".bright_cyan().bold()));
}

fn print_model(text: &str) {
    if text.trim().is_empty() {
        return;
    }
    print_line(format!("{} {text}", "OpenJarvisBR:".bright_green().bold()));
}

/// `print!` com `\r\n` explícito: em modo raw o terminal não traduz `\n`
/// sozinho, então `println!` puro deixaria a saída "em escada".
fn print_line(line: impl std::fmt::Display) {
    print!("{line}\r\n");
    let _ = io::stdout().flush();
}
