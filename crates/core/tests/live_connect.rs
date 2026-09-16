//! Teste de integração com a Gemini Live API real. Ignorado por padrão:
//! `GEMINI_API_KEY=... cargo test -- --ignored live_connect`.
//!
//! O crate só tem binário, então o módulo `live` é incluído pelo caminho.

#[path = "../src/live/mod.rs"]
mod live;

use std::time::{Duration, Instant};

use live::session::{LiveConfig, LiveError, LiveSession};

#[tokio::test]
#[ignore = "precisa de rede e GEMINI_API_KEY"]
async fn live_connect() {
    let Some(key) = std::env::var("GEMINI_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
    else {
        panic!("GEMINI_API_KEY não definida");
    };

    let started = Instant::now();
    let result = LiveSession::connect(LiveConfig::new(key, "Puck")).await;
    let elapsed = started.elapsed();

    let session = result.unwrap_or_else(|err| panic!("setup recusado: {err}"));
    assert!(
        elapsed < Duration::from_secs(3),
        "setupComplete demorou {elapsed:?}"
    );
    drop(session);
}

#[tokio::test]
#[ignore = "precisa de rede"]
async fn live_connect_invalid_key_is_unauthorized() {
    let result = LiveSession::connect(LiveConfig::new("chave-invalida-de-teste", "Puck")).await;
    match result {
        Err(err) => {
            assert_eq!(err, LiveError::Unauthorized, "veio {err}");
            assert!(!err.to_string().contains("chave-invalida-de-teste"));
        }
        Ok(_) => panic!("chave inválida não deveria conectar"),
    }
}
