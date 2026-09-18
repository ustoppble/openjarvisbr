use std::path::Path;
use std::process::Command;

use openjarvisbr_core::mcp::{McpClient, McpServerConfig};
use serde_json::json;

#[test]
fn python_observer_contract() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("raiz do workspace");
    let output = Command::new("python3")
        .args(["tools/observer/test_server.py", "-q"])
        .current_dir(repository)
        .output()
        .expect("python3 executa a suíte do observador");

    assert!(
        output.status.success(),
        "suíte Python falhou:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn jarvis_mcp_client_calls_the_observer() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("raiz do workspace");
    let server = repository.join("tools/observer/server.py");
    let trace = repository.join("docs/testes/rodada1-trace.log");
    let client = McpClient::new(McpServerConfig {
        name: "observer".to_string(),
        command: Some("python3".to_string()),
        args: vec![server.to_string_lossy().into_owned()],
        ..Default::default()
    });

    let tools = client.list_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, ["observe_session", "list_failures", "propose_tasks"]);

    let result = client
        .call_tool(
            "observe_session",
            json!({"trace_path": trace.to_string_lossy()}),
        )
        .await
        .unwrap();
    assert_eq!(result["structuredContent"]["turns"], json!(12));
    assert_eq!(result["structuredContent"]["duplicates"], json!(2));
    assert_eq!(result["structuredContent"]["refusals"], json!(1));
}
