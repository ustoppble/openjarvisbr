//! Modo roteiro (`--script FILE`): manda cada linha do arquivo como fala do
//! usuário, espera o turno do modelo terminar e segue para a próxima. O mic
//! fica mudo. Serve para medir o Jarvis sem ninguém falar: o trace
//! (`RUST_LOG=…=debug`) é o resultado, e `tools/jarvis_trace_report.py` lê.
//!
//! Formato do arquivo: uma fala por linha; linhas vazias e começando com `#`
//! são ignoradas. `sleep N` espera N segundos (para ações que abrem apps).

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use openjarvisbr_core::engine::{EngineEvent, EngineHandle};
use openjarvisbr_core::engine_tools::call_summary;

/// Teto por fala (modelo + tools), não ritmo: a próxima linha só sai quando
/// o turno fechou, nenhuma tool está pendente e os eventos silenciaram.
const TURN_TIMEOUT: Duration = Duration::from_secs(25);
/// Silêncio de eventos que fecha a fala (o reflexo pode agir ~400 ms depois
/// do turno concluído; uma tool pedida no silêncio reabre a espera).
const QUIET: Duration = Duration::from_millis(900);

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

async fn wait_for_turn(
    events: &mut tokio::sync::broadcast::Receiver<EngineEvent>,
    failures: &mut usize,
) -> bool {
    // A fala fecha quando: turno concluído E zero tools pendentes E
    // QUIET sem eventos. Uma tool pedida (modelo ou reflexo) depois
    // do turno reabre a espera até o resultado dela chegar.
    let mut pending: HashSet<String> = HashSet::new();
    let mut turn_done = false;
    let done = tokio::time::timeout(TURN_TIMEOUT, async {
        loop {
            // Medidores chegam mesmo sem fala (20 Hz). Só atividade do
            // turno deve reiniciar o silêncio, senão toda fala bate o teto.
            let activity = async {
                loop {
                    match events.recv().await {
                        Ok(EngineEvent::Level { .. }) => continue,
                        event => break event,
                    }
                }
            };
            let next = if turn_done && pending.is_empty() {
                match tokio::time::timeout(QUIET, activity).await {
                    Ok(ev) => ev,
                    Err(_) => break true, // silêncio: fala fechada
                }
            } else {
                activity.await
            };
            match next {
                Ok(EngineEvent::ModelText(t)) => {
                    // A chamada de tool pode ter fechado um turno intermediário.
                    // A resposta que vem depois precisa do próprio TurnComplete.
                    turn_done = false;
                    print!("{t}");
                }
                Ok(EngineEvent::ToolRequested { call, .. }) => {
                    println!("\n  [tool] {}", call_summary(&call));
                    pending.insert(call.id);
                }
                Ok(EngineEvent::ToolResult {
                    id,
                    name,
                    ok,
                    summary,
                    ..
                }) => {
                    println!("  [{}] {name}: {summary}", if ok { "ok" } else { "ERRO" });
                    if !ok {
                        *failures += 1;
                    }
                    pending.remove(&id);
                    // Resultados do modelo geram uma nova resposta dele. Já o
                    // reflexo só publica o resultado; não envia functionResponse.
                    if !id.starts_with(openjarvisbr_core::reflex::decide::REFLEX_CALL_PREFIX)
                        && summary != "cancelada"
                    {
                        turn_done = false;
                    }
                }
                Ok(EngineEvent::ReflexActed {
                    call, latency_ms, ..
                }) => println!("  [reflexo {latency_ms} ms] {}", call.name),
                Ok(EngineEvent::ToolConfirmNeeded { id, summary, .. }) => {
                    println!("  [confirma?] {summary} (roteiro não confirma)");
                    // não vai ser respondida: não segura a fala
                    pending.remove(&id);
                }
                Ok(EngineEvent::TurnComplete) => {
                    println!();
                    turn_done = true;
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
        Ok(true) => true,
        Ok(false) => false,
        Err(_) => {
            println!(
                "  [timeout] fala não fechou em {}s (turno={turn_done}, tools pendentes={})",
                TURN_TIMEOUT.as_secs(),
                pending.len()
            );
            true
        }
    }
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
                Ok(EngineEvent::State(openjarvisbr_core::engine::EngineState::Listening)) => {
                    break true
                }
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
                if !wait_for_turn(&mut events, &mut failures).await {
                    handle.stop().await;
                    return 1;
                }
            }
        }
    }
    handle.stop().await;
    println!(
        "roteiro terminado: {} falas, {failures} tool(s) com erro",
        lines.iter().filter(|l| matches!(l, Line::Say(_))).count()
    );
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
    #[tokio::test]
    async fn nivel_continuo_nao_adia_fim_do_turno() {
        let (tx, mut events) = tokio::sync::broadcast::channel(128);
        tx.send(EngineEvent::TurnComplete).unwrap();
        let levels = tokio::spawn(async move {
            loop {
                let _ = tx.send(EngineEvent::Level {
                    mic: 0.0,
                    model: 0.0,
                });
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
        let mut failures = 0;
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_turn(&mut events, &mut failures),
        )
        .await;
        levels.abort();
        assert!(
            matches!(result, Ok(true)),
            "níveis de áudio impediram o fim do turno"
        );
        assert_eq!(failures, 0);
    }

    #[tokio::test]
    async fn tool_tardia_reabre_espera_ate_receber_resultado() {
        use openjarvisbr_core::tools::{Risk, ToolCall};

        let (tx, mut events) = tokio::sync::broadcast::channel(128);
        tx.send(EngineEvent::TurnComplete).unwrap();
        let waiting = tokio::spawn(async move {
            let mut failures = 0;
            let done = wait_for_turn(&mut events, &mut failures).await;
            (done, failures)
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        tx.send(EngineEvent::ToolRequested {
            call: ToolCall {
                id: "reflex-late-tool".into(),
                name: "app.open".into(),
                args: Default::default(),
            },
            risk: Risk::Safe,
        })
        .unwrap();
        tokio::time::sleep(Duration::from_millis(1000)).await;
        assert!(!waiting.is_finished(), "avançou com uma tool pendente");
        tx.send(EngineEvent::ToolResult {
            id: "reflex-late-tool".into(),
            name: "app.open".into(),
            ok: true,
            summary: "aberto".into(),
        })
        .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), waiting).await;
        assert!(matches!(result, Ok(Ok((true, 0)))));
    }

    #[tokio::test]
    async fn resposta_da_tool_exige_novo_fim_do_turno_do_modelo() {
        use openjarvisbr_core::tools::{Risk, ToolCall};

        let (tx, mut events) = tokio::sync::broadcast::channel(128);
        tx.send(EngineEvent::ToolRequested {
            call: ToolCall {
                id: "model-call".into(),
                name: "app.open".into(),
                args: Default::default(),
            },
            risk: Risk::Safe,
        })
        .unwrap();
        tx.send(EngineEvent::TurnComplete).unwrap();
        tx.send(EngineEvent::ToolResult {
            id: "model-call".into(),
            name: "app.open".into(),
            ok: true,
            summary: "aberto".into(),
        })
        .unwrap();
        let waiting = tokio::spawn(async move { wait_for_turn(&mut events, &mut 0).await });
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(
            !waiting.is_finished(),
            "avançou antes de o modelo encerrar a resposta da tool"
        );
        tx.send(EngineEvent::TurnComplete).unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), waiting).await,
            Ok(Ok(true))
        ));
    }

    #[tokio::test]
    async fn texto_novo_exige_novo_fim_do_turno_do_modelo() {
        let (tx, mut events) = tokio::sync::broadcast::channel(128);
        tx.send(EngineEvent::TurnComplete).unwrap();
        tx.send(EngineEvent::ModelText("Feito.".into())).unwrap();
        let waiting = tokio::spawn(async move { wait_for_turn(&mut events, &mut 0).await });
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(
            !waiting.is_finished(),
            "avançou durante a nova resposta do modelo"
        );
        tx.send(EngineEvent::TurnComplete).unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), waiting).await,
            Ok(Ok(true))
        ));
    }
}
