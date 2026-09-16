//! Teste de integração com a Gemini Live API real. Ignorado por padrão:
//! `GEMINI_API_KEY=... cargo test -- --ignored live_connect`.

use std::time::{Duration, Instant};

use openjarvisbr_core::live::session::{LiveConfig, LiveError, LiveSession};
use openjarvisbr_core::tools::{Risk, ToolSpec};

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
#[ignore = "precisa de rede e GEMINI_API_KEY"]
async fn live_connect_with_tool() {
    let Some(key) = std::env::var("GEMINI_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
    else {
        panic!("GEMINI_API_KEY não definida");
    };

    // Nome com ponto e JSON Schema completo (additionalProperties): o
    // servidor tem que aceitar o setup assim, é o formato das tools da v3.
    let tool = ToolSpec {
        name: "clock.now".to_string(),
        description: "Retorna a hora atual local.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"timezone": {"type": "string", "description": "Fuso IANA"}},
            "required": ["timezone"],
            "additionalProperties": false
        }),
        risk: Risk::Safe,
    };
    let result = LiveSession::connect(LiveConfig::new(key, "Puck").with_tools(vec![tool])).await;
    let session = result.unwrap_or_else(|err| panic!("setup com tool recusado: {err}"));
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
