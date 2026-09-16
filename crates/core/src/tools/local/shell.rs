//! `shell.run`: roda um comando de terminal no home, com timeout, sem sudo e
//! com a saída (stdout + stderr) truncada.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt};

use super::str_arg;
use crate::tools::{Risk, Tool, ToolError, ToolSpec};

/// Tempo máximo de um comando.
pub const SHELL_TIMEOUT: Duration = Duration::from_secs(30);
/// Bytes de saída devolvidos ao modelo; o resto é descartado.
pub const SHELL_OUTPUT_LIMIT: usize = 4096;
/// Comandos de elevação de privilégio recusados.
const ELEVATION: &[&str] = &["sudo", "su", "doas", "pkexec", "runas"];

pub struct ShellRun {
    cwd: PathBuf,
    timeout: Duration,
}

impl ShellRun {
    /// Roda com `cwd` como diretório de trabalho e timeout de 30s.
    pub fn new(cwd: PathBuf) -> Self {
        Self::with_timeout(cwd, SHELL_TIMEOUT)
    }

    pub fn with_timeout(cwd: PathBuf, timeout: Duration) -> Self {
        Self { cwd, timeout }
    }
}

#[async_trait]
impl Tool for ShellRun {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shell.run".into(),
            description: "Roda um comando de terminal (shell) na pasta home do usuário e \
                          devolve a saída (stdout e stderr juntos, até 4KB) e o código de \
                          saída. Limite de 30 segundos; comandos com sudo são recusados. \
                          Pede confirmação antes de executar."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Comando completo, ex.: \"ls -la Documentos\" ou \"git -C projetos/app status\"."
                    }
                },
                "required": ["command"]
            }),
            risk: Risk::Confirm,
        }
    }

    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let command = str_arg(&args, "command")?.trim();
        if command.is_empty() {
            return Err(ToolError::InvalidArgs("comando vazio".into()));
        }
        if asks_elevation(command) {
            return Err(ToolError::InvalidArgs(
                "não rodo comandos com sudo ou outra elevação de privilégio".into(),
            ));
        }
        let cwd = self
            .cwd
            .canonicalize()
            .map_err(|_| ToolError::Failed("pasta home não encontrada".into()))?;
        run(command, &cwd, self.timeout).await
    }
}

/// `true` se alguma palavra do comando é um comando de elevação.
fn asks_elevation(command: &str) -> bool {
    command
        .split(|c: char| c.is_whitespace() || ";|&()`$<>{}'\"".contains(c))
        .any(|word| ELEVATION.contains(&word.rsplit('/').next().unwrap_or(word)))
}

#[cfg(unix)]
fn build(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sh");
    // `exec 2>&1` junta stderr ao stdout na ordem em que saem.
    cmd.arg("-c").arg(format!("exec 2>&1\n{command}"));
    // Grupo próprio: no timeout mata o comando e os filhos dele.
    cmd.process_group(0);
    cmd
}

#[cfg(windows)]
fn build(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.arg("/C").arg(format!("{command} 2>&1"));
    cmd
}

async fn run(
    command: &str,
    cwd: &std::path::Path,
    timeout: Duration,
) -> Result<serde_json::Value, ToolError> {
    let mut cmd = build(command);
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| ToolError::Failed(format!("não consegui iniciar o shell: {e}")))?;
    let stdout = tokio::spawn(read_capped(child.stdout.take(), SHELL_OUTPUT_LIMIT));
    let stderr = tokio::spawn(read_capped(child.stderr.take(), SHELL_OUTPUT_LIMIT));

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => status.map_err(|e| ToolError::Failed(e.to_string()))?,
        Err(_) => {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                let _ = std::process::Command::new("kill")
                    .args(["-KILL", &format!("-{pid}")])
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = child.kill().await;
            stdout.abort();
            stderr.abort();
            return Err(ToolError::Timeout);
        }
    };

    // Um neto em segundo plano pode segurar o pipe aberto; não espera por ele.
    let grace = Duration::from_secs(2);
    let (mut out, mut truncated) = tokio::time::timeout(grace, stdout)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let (err, err_truncated) = tokio::time::timeout(grace, stderr)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    out.extend_from_slice(&err);
    truncated |= err_truncated;
    if out.len() > SHELL_OUTPUT_LIMIT {
        out.truncate(SHELL_OUTPUT_LIMIT);
        truncated = true;
    }

    Ok(json!({
        "exit_code": status.code(),
        "output": String::from_utf8_lossy(&out),
        "truncated": truncated,
    }))
}

/// Lê até `cap` bytes e drena o resto (para o processo não travar no pipe).
async fn read_capped<R: AsyncRead + Unpin>(reader: Option<R>, cap: usize) -> (Vec<u8>, bool) {
    let Some(mut reader) = reader else {
        return (Vec::new(), false);
    };
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = cap.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
                truncated |= n > room;
            }
        }
    }
    (kept, truncated)
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::Instant;

    use super::super::testutil::TempDir;
    use super::*;

    #[tokio::test]
    async fn roda_no_cwd_e_junta_stderr() {
        let tmp = TempDir::new();
        std::fs::write(tmp.0.join("marca.txt"), "x").unwrap();
        let tool = ShellRun::new(tmp.0.clone());
        let out = tool
            .call(json!({"command": "ls; echo erro >&2; exit 3"}))
            .await
            .unwrap();
        assert_eq!(out["exit_code"], 3);
        let text = out["output"].as_str().unwrap();
        assert!(text.contains("marca.txt"), "{text}");
        assert!(text.contains("erro"), "{text}");
        assert_eq!(out["truncated"], false);
    }

    #[tokio::test]
    async fn trunca_saida_em_4kb() {
        let tmp = TempDir::new();
        let out = ShellRun::new(tmp.0.clone())
            .call(json!({"command": "yes | head -c 100000"}))
            .await
            .unwrap();
        assert_eq!(out["output"].as_str().unwrap().len(), SHELL_OUTPUT_LIMIT);
        assert_eq!(out["truncated"], true);
        assert_eq!(out["exit_code"], 0);
    }

    #[tokio::test]
    async fn interrompe_no_timeout() {
        let tmp = TempDir::new();
        let tool = ShellRun::with_timeout(tmp.0.clone(), Duration::from_millis(300));
        let started = Instant::now();
        let res = tool.call(json!({"command": "sleep 10"})).await;
        assert!(matches!(res, Err(ToolError::Timeout)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn recusa_sudo() {
        let tmp = TempDir::new();
        let tool = ShellRun::new(tmp.0.clone());
        for bad in [
            "sudo ls",
            "echo oi && sudo rm x",
            "/usr/bin/sudo ls",
            "(su -c id)",
            "x|doas id",
        ] {
            assert!(
                matches!(tool.call(json!({"command": bad})).await, Err(ToolError::InvalidArgs(ref m)) if m.contains("sudo")),
                "deveria recusar {bad}"
            );
        }
        assert!(!asks_elevation("echo pseudo sudoers"));
        assert!(matches!(
            tool.call(json!({"command": " "})).await,
            Err(ToolError::InvalidArgs(_))
        ));
    }

    #[test]
    fn timeout_padrao_30s() {
        assert_eq!(SHELL_TIMEOUT, Duration::from_secs(30));
    }
}
