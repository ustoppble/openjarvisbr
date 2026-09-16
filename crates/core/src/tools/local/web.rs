//! `web.open`: abre um endereço no navegador padrão.

use async_trait::async_trait;
use serde_json::json;

use super::str_arg;
use crate::tools::{Risk, Tool, ToolError, ToolSpec};

pub struct WebOpen;

#[async_trait]
impl Tool for WebOpen {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web.open".into(),
            description: "Abre um site no navegador padrão. Aceita endereço completo \
                          (https://...) ou só o domínio (ex.: \"github.com\")."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Endereço a abrir, ex.: \"https://overclock.sh\" ou \"youtube.com\"."
                    }
                },
                "required": ["url"]
            }),
            risk: Risk::Safe,
        }
    }

    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let url = normalize_url(str_arg(&args, "url")?)?;
        let status = open_command(&url)
            .status()
            .await
            .map_err(|e| ToolError::Failed(format!("não consegui abrir o navegador: {e}")))?;
        if !status.success() {
            return Err(ToolError::Failed(format!("o navegador recusou {url}")));
        }
        Ok(json!({ "opened": url }))
    }
}

/// Só http(s). Sem esquema, assume https.
fn normalize_url(raw: &str) -> Result<String, ToolError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ToolError::InvalidArgs(format!("endereço inválido: {raw}")));
    }
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(raw.to_string());
    }
    if raw.contains("://") || raw.starts_with('-') || !raw.contains('.') {
        return Err(ToolError::InvalidArgs(format!(
            "só abro endereços http ou https: {raw}"
        )));
    }
    Ok(format!("https://{raw}"))
}

#[cfg(target_os = "macos")]
fn open_command(url: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("open");
    cmd.arg(url);
    cmd
}

#[cfg(windows)]
fn open_command(url: &str) -> tokio::process::Command {
    // `start` via cmd quebra em `&`; o handler de protocolo recebe a URL crua.
    let mut cmd = tokio::process::Command::new("rundll32");
    cmd.arg("url.dll,FileProtocolHandler").arg(url);
    cmd
}

#[cfg(not(any(target_os = "macos", windows)))]
fn open_command(url: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("xdg-open");
    cmd.arg(url);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_urls() {
        assert_eq!(normalize_url("github.com").unwrap(), "https://github.com");
        assert_eq!(
            normalize_url(" https://overclock.sh/a?b=1&c=2 ").unwrap(),
            "https://overclock.sh/a?b=1&c=2"
        );
        assert_eq!(normalize_url("HTTP://x.com").unwrap(), "HTTP://x.com");
        for bad in [
            "",
            "file:///etc/passwd",
            "javascript://x",
            "-a Calculator",
            "semponto",
            "a b.com",
        ] {
            assert!(normalize_url(bad).is_err(), "deveria recusar {bad}");
        }
    }
}
