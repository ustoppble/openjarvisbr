//! Modo roteiro (`--script FILE`): manda cada linha do arquivo como fala do
//! usuário, espera o turno do modelo terminar e segue para a próxima. O mic
//! fica mudo. Serve para medir o Jarvis sem ninguém falar: o trace
//! (`RUST_LOG=…=debug`) é o resultado, e `tools/jarvis_trace_report.py` lê.
//!
//! Formato do arquivo: uma fala por linha; linhas vazias e começando com `#`
//! são ignoradas. `sleep N` espera N segundos (para ações que abrem apps).

use std::path::Path;
use std::time::Duration;

use openjarvisbr_core::engine::{EngineEvent, EngineHandle};
use openjarvisbr_core::engine_tools::call_summary;

/// Tempo máximo por turno (modelo + tools) antes de seguir em frente.
const TURN_TIMEOUT: Duration = Duration::from_secs(25);
/// Folga depois de cada turno para o reflexo/tools terminarem e o trace assentar.
const SETTLE: Duration = Duration::from_millis(1500);

pub enum Line {
    Say(String),
    Sleep(Duration),
}

/// Lê o roteiro: falas, `sleep N`, sem comentários e vazios.
pub fn parse_script(text: &str) -> Vec<Line> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.strip_prefix("sleep ") {
            Some(secs) => Line::Sleep(Duration::from_secs_f64(secs.trim().parse().unwrap_or(1.0))),
            None => Line::Say(l.to_string()),
        })
        .collect()
}

pub async fn run(handle: EngineHandle, path: &Path) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("não consegui ler o roteiro {}: {err}", path.display());
            return 2;
        }
    };
    let lines = parse_script(&text);
    let mut events = handle.events();
    handle.mute(true);
    // espera a conexão
    let connected = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match events.recv().await {
                Ok(EngineEvent::State(openjarvisbr_core::engine::EngineState::Listening)) => break true,
                Ok(EngineEvent::Error { message, .. }) => {
                    eprintln!("sessão encerrada: {message}");
                    break false;
                }
                Ok(_) => {}
                Err(_) => break false,
            }
        }
    })
    .await
    .unwrap_or(false);
    if !connected {
        eprintln!("não conectou a tempo; roteiro abortado");
        handle.stop().await;
        return 1;
    }
    let mut failures = 0;
    for (n, line) in lines.iter().enumerate() {
        match line {
            Line::Sleep(d) => tokio::time::sleep(*d).await,
            Line::Say(say) => {
                println!("[{}] > {say}", n + 1);
                handle.send_user_text(say);
                let done = tokio::time::timeout(TURN_TIMEOUT, async {
                    loop {
                        match events.recv().await {
                            Ok(EngineEvent::ModelText(t)) => print!("{t}"),
                            Ok(EngineEvent::ToolRequested { call, .. }) => println!("\n  [tool] {}", call_summary(&call)),
                            Ok(EngineEvent::ToolResult { name, ok, summary, .. }) => {
                                println!("  [{}] {name}: {summary}", if ok { "ok" } else { "ERRO" });
                                if !ok {
                                    failures += 1;
                                }
                            }
                            Ok(EngineEvent::ReflexActed { call, latency_ms, .. }) => println!("  [reflexo {latency_ms} ms] {}", call.name),
                            Ok(EngineEvent::ToolConfirmNeeded { summary, .. }) => println!("  [confirma?] {summary} (roteiro não confirma)"),
                            Ok(EngineEvent::TurnComplete) => {
                                println!();
                                break true;
                            }
                            Ok(EngineEvent::Error { message, .. }) => {
                                eprintln!("sessão encerrada: {message}");
                                break false;
                            }
                            Ok(_) => {}
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                            Err(_) => break false,
                        }
                    }
                })
                .await;
                match done {
                    Ok(true) => {}
                    Ok(false) => {
                        handle.stop().await;
                        return 1;
                    }
                    Err(_) => println!("  [timeout] turno não terminou em {}s", TURN_TIMEOUT.as_secs()),
                }
                tokio::time::sleep(SETTLE).await;
            }
        }
    }
    handle.stop().await;
    println!("roteiro terminado: {} falas, {failures} tool(s) com erro", lines.iter().filter(|l| matches!(l, Line::Say(_))).count());
    i32::from(failures > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roteiro_ignora_comentarios_e_entende_sleep() {
        let lines = parse_script("# teste\n\nabre o Safari\nsleep 2.5\n  abre a globo  \n");
        assert_eq!(lines.len(), 3);
        assert!(matches!(&lines[0], Line::Say(s) if s == "abre o Safari"));
        assert!(matches!(&lines[1], Line::Sleep(d) if *d == Duration::from_millis(2500)));
        assert!(matches!(&lines[2], Line::Say(s) if s == "abre a globo"));
    }
}
