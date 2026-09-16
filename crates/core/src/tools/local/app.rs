//! `app.open`: abre um aplicativo pelo nome.

use async_trait::async_trait;
use serde_json::json;

use super::str_arg;
use crate::tools::{Risk, Tool, ToolError, ToolSpec};

pub struct AppOpen;

#[async_trait]
impl Tool for AppOpen {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "app.open".into(),
            description: "Abre um aplicativo instalado no computador pelo nome \
                          (ex.: \"Safari\", \"Spotify\", \"Visual Studio Code\")."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Nome do aplicativo como aparece no sistema, ex.: \"Safari\"."
                    }
                },
                "required": ["name"]
            }),
            risk: Risk::Safe,
        }
    }

    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let name = validate_name(str_arg(&args, "name")?)?;
        let status = open_command(name)?
            .status()
            .await
            .map_err(|e| ToolError::Failed(format!("não consegui abrir \"{name}\": {e}")))?;
        if !status.success() {
            return Err(ToolError::Failed(format!(
                "não encontrei o aplicativo \"{name}\""
            )));
        }
        Ok(json!({ "opened": name }))
    }
}

fn validate_name(name: &str) -> Result<&str, ToolError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ToolError::InvalidArgs("nome do aplicativo vazio".into()));
    }
    if name.starts_with('-') || name.contains(['\n', '\r', '\0']) {
        return Err(ToolError::InvalidArgs(format!(
            "nome de aplicativo inválido: {name}"
        )));
    }
    Ok(name)
}

#[cfg(target_os = "macos")]
fn open_command(name: &str) -> Result<tokio::process::Command, ToolError> {
    let mut cmd = tokio::process::Command::new("open");
    cmd.arg("-a").arg(name);
    Ok(cmd)
}

#[cfg(windows)]
fn open_command(name: &str) -> Result<tokio::process::Command, ToolError> {
    if name.contains(['&', '|', '<', '>', '^', '"']) {
        return Err(ToolError::InvalidArgs(format!(
            "nome de aplicativo inválido: {name}"
        )));
    }
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.args(["/C", "start", ""]).arg(name);
    Ok(cmd)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn open_command(name: &str) -> Result<tokio::process::Command, ToolError> {
    let mut cmd = tokio::process::Command::new("gtk-launch");
    cmd.arg(name);
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recusa_nome_vazio_ou_flag() {
        for bad in [
            json!({}),
            json!({"name": "  "}),
            json!({"name": "-n Safari"}),
        ] {
            assert!(matches!(
                AppOpen.call(bad).await,
                Err(ToolError::InvalidArgs(_))
            ));
        }
    }
}
