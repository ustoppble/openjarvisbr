//! Terminal da OpenJarvisBR: imprime a transcrição colorida a partir dos
//! eventos do Engine e traduz teclado (M muta, Ctrl+C encerra) em comandos.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use colored::Colorize;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tracing::{info, warn};

use openjarvisbr_core::audio::capture::{self, CaptureError};
use openjarvisbr_core::audio::playback::PlaybackError;
use openjarvisbr_core::engine::{EngineError, EngineEvent, EngineHandle};
use openjarvisbr_core::engine_tools::call_summary;

/// Intervalo de checagem de eventos de teclado (raw mode).
const KEY_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Consome os eventos até a conversa terminar (Ctrl+C ou sessão perdida) e
/// devolve o código de saída do processo.
pub async fn run(
    handle: EngineHandle,
    barge_in: bool,
    fx_amount: f32,
    record_dir: Option<PathBuf>,
) -> i32 {
    let mut events = handle.events();
    let raw_mode = RawMode::enable();
    let (mut control_rx, _control_tx) = spawn_key_thread(raw_mode.enabled);

    print_line("ouvindo — fale quando quiser (M muta o microfone, S/N responde a um pedido de confirmação, Ctrl+C encerra)");
    if !barge_in {
        print_line("modo caixa de som: espere o Jarvis terminar pra falar (--barge-in libera interrupção por voz, use com fone)".dimmed());
    }
    if fx_amount > 0.0 {
        print_line(format!("efeito de voz jarvis: {fx_amount:.2} (--fx-amount ajusta)").dimmed());
    }
    if let Some(dir) = &record_dir {
        print_line(format!("gravando diagnóstico em {}", dir.display()).dimmed());
    }

    let mut muted = false;
    let mut transcript = Transcript::default();
    // Confirmações abertas, a mais antiga primeiro: S/N responde a ela.
    let mut confirms: VecDeque<String> = VecDeque::new();
    let exit_code = loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(EngineEvent::UserText(text)) => transcript.user(&text),
                Ok(EngineEvent::ModelText(text)) => transcript.model(&text),
                Ok(EngineEvent::TurnComplete) => transcript.end_line(),
                Ok(EngineEvent::Error { message, .. }) => {
                    transcript.end_line();
                    eprint!("sessão encerrada: {message}\r\n");
                    break handle.last_error().map_or(0, |err| err.exit_code());
                }
                Ok(EngineEvent::ToolRequested { call, .. }) => {
                    transcript.end_line();
                    print_line(format!("[ferramenta] {}", call_summary(&call)).dimmed());
                }
                Ok(EngineEvent::ToolConfirmNeeded { id, summary, .. }) => {
                    transcript.end_line();
                    print_line(format!("[confirma?] {summary} — diga sim/não ou tecle S/N").yellow().bold());
                    confirms.push_back(id);
                }
                Ok(EngineEvent::ToolResult { id, name, ok, summary }) => {
                    confirms.retain(|pending| pending != &id);
                    transcript.end_line();
                    let line = format!("[{}] {name}: {summary}", if ok { "ok" } else { "falhou" });
                    print_line(if ok { line.green() } else { line.red() });
                }
                Ok(EngineEvent::State(_) | EngineEvent::Level { .. } | EngineEvent::Reconnecting { .. }) => {}
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break 0,
            },
            Some(control) = control_rx.recv() => match control {
                ControlEvent::ToggleMute => {
                    muted = !muted;
                    handle.mute(muted);
                    transcript.end_line();
                    print_line(if muted { "[mic mudo]".yellow() } else { "[mic ativo]".yellow() });
                }
                ControlEvent::Confirm(approve) => {
                    if let Some(id) = confirms.pop_front() {
                        handle.confirm_tool(&id, approve);
                    }
                }
                ControlEvent::Quit => {
                    info!("Ctrl+C recebido, encerrando");
                    break 0;
                }
            },
            _ = tokio::signal::ctrl_c() => {
                info!("Ctrl+C recebido, encerrando");
                break 0;
            }
        }
    };

    transcript.end_line();
    handle.stop().await;
    drop(raw_mode);
    exit_code
}

/// Imprime a falha de subida já formatada para o usuário (dispositivos
/// disponíveis ou dica de permissão no Mac).
pub fn report_start_error(err: &EngineError) {
    match err {
        EngineError::Connect(err) => eprintln!("erro ao conectar: {err}"),
        EngineError::Capture(err) => {
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
        }
        EngineError::Playback(err) => {
            eprintln!("erro de áudio (saída): {err}");
            if let PlaybackError::NoOutputDevice | PlaybackError::DeviceNotFound(_) = err {
                eprintln!(
                    "verifique se há um dispositivo de saída de áudio conectado e selecionado."
                );
            }
        }
        EngineError::Runtime(_) => eprintln!("{err}"),
    }
}

/// Eventos de teclado tratados pelo loop.
enum ControlEvent {
    ToggleMute,
    /// S (sim) ou N (não) para a confirmação de ferramenta mais antiga.
    Confirm(bool),
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

/// Lê eventos de teclado em uma thread própria (bloqueante) quando o modo raw
/// está ativo. O `Sender` retornado deve ser mantido vivo pelo chamador: sem
/// ele, o canal fecha e o `select!` do loop gira sem parar.
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
                            KeyCode::Char('s') | KeyCode::Char('S') => {
                                Some(ControlEvent::Confirm(true))
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') => {
                                Some(ControlEvent::Confirm(false))
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

/// Junta os pedaços de transcrição que chegam em chunks numa única linha por
/// turno: o prefixo só aparece quando o falante muda, e a linha só fecha em
/// `end_line` (turno completo, interrupção, mute, saída).
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
