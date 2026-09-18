//! `web.open`: abre um endereço no navegador padrão ou, se o usuário pediu,
//! num navegador específico (`browser`).

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
            description: "Abre um site no navegador. Aceita endereço completo \
                          (https://...) ou só o domínio (ex.: \"github.com\"). Sem \
                          `browser` usa o navegador padrão do sistema."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Endereço a abrir, ex.: \"https://overclock.sh\" ou \"youtube.com\"."
                    },
                    "browser": {
                        "type": "string",
                        "description": "Navegador pedido pelo usuário, ex.: \"Safari\", \"Chrome\", \
                                        \"Firefox\". Só preencha se ele nomeou um; senão omita."
                    }
                },
                "required": ["url"]
            }),
            risk: Risk::Safe,
        }
    }

    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let url = normalize_url(str_arg(&args, "url")?)?;
        let browser = normalize_browser(args.get("browser").and_then(|v| v.as_str()))?;
        let status = open_command(&url, browser.as_deref())
            .status()
            .await
            .map_err(|e| ToolError::Failed(format!("não consegui abrir o navegador: {e}")))?;
        if !status.success() {
            return Err(match &browser {
                Some(name) => ToolError::Failed(format!("não achei o navegador \"{name}\"")),
                None => ToolError::Failed(format!("o navegador recusou {url}")),
            });
        }
        Ok(match browser {
            Some(name) => json!({ "opened": url, "browser": name }),
            None => json!({ "opened": url }),
        })
    }
}

/// Só http(s). Sem esquema, assume https.
pub(super) fn normalize_url(raw: &str) -> Result<String, ToolError> {
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

/// Apelidos que o usuário fala → nome do app como o SO conhece.
const BROWSER_ALIASES: &[(&str, &str)] = &[
    ("safari", "Safari"),
    ("chrome", "Google Chrome"),
    ("google chrome", "Google Chrome"),
    ("firefox", "Firefox"),
    ("edge", "Microsoft Edge"),
    ("microsoft edge", "Microsoft Edge"),
    ("arc", "Arc"),
    ("brave", "Brave Browser"),
    ("brave browser", "Brave Browser"),
    ("opera", "Opera"),
];

/// `None`/vazio = navegador padrão. Recusa o que pareça flag ou tenha
/// caractere de controle; apelido conhecido vira o nome oficial.
pub(super) fn normalize_browser(raw: Option<&str>) -> Result<Option<String>, ToolError> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if raw.starts_with('-') || raw.chars().any(char::is_control) {
        return Err(ToolError::InvalidArgs(format!("navegador inválido: {raw:?}")));
    }
    let lower = raw.to_lowercase();
    let name = BROWSER_ALIASES
        .iter()
        .find(|(alias, _)| *alias == lower)
        .map(|(_, official)| official.to_string())
        .unwrap_or_else(|| raw.to_string());
    Ok(Some(name))
}

#[cfg(target_os = "macos")]
fn open_command(url: &str, browser: Option<&str>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("open");
    if let Some(app) = browser {
        cmd.arg("-a").arg(app);
    }
    cmd.arg(url);
    cmd
}

#[cfg(windows)]
fn open_command(url: &str, browser: Option<&str>) -> tokio::process::Command {
    match browser {
        // `start "" <app> <url>`: o app precisa estar no PATH ou nos App Paths.
        Some(app) => {
            let mut cmd = tokio::process::Command::new("cmd");
            cmd.args(["/C", "start", ""]).arg(app).arg(url);
            cmd
        }
        None => {
            // `start` via cmd quebra em `&`; o handler de protocolo recebe a URL crua.
            let mut cmd = tokio::process::Command::new("rundll32");
            cmd.arg("url.dll,FileProtocolHandler").arg(url);
            cmd
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn open_command(url: &str, browser: Option<&str>) -> tokio::process::Command {
    match browser {
        Some(app) => {
            let mut cmd = tokio::process::Command::new(app.to_lowercase().replace(' ', "-"));
            cmd.arg(url);
            cmd
        }
        None => {
            let mut cmd = tokio::process::Command::new("xdg-open");
            cmd.arg(url);
            cmd
        }
    }
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

    #[test]
    fn erro_de_url_nao_repete_a_entrada() {
        for raw in ["nao_repetir_este_valor", "a nao_repetir_este_valor.com"] {
            let error = normalize_url(raw).unwrap_err().to_string();
            assert!(!error.contains(raw), "o erro refletiu a URL recebida");
        }
    }

    #[test]
    fn normaliza_navegador_com_apelidos_e_recusa_flags() {
        assert_eq!(normalize_browser(None).unwrap(), None);
        assert_eq!(normalize_browser(Some("  ")).unwrap(), None);
        assert_eq!(normalize_browser(Some("Safari")).unwrap().as_deref(), Some("Safari"));
        assert_eq!(normalize_browser(Some("safari")).unwrap().as_deref(), Some("Safari"));
        assert_eq!(normalize_browser(Some("chrome")).unwrap().as_deref(), Some("Google Chrome"));
        assert_eq!(normalize_browser(Some("Google Chrome")).unwrap().as_deref(), Some("Google Chrome"));
        assert_eq!(normalize_browser(Some("edge")).unwrap().as_deref(), Some("Microsoft Edge"));
        assert_eq!(normalize_browser(Some("Brave")).unwrap().as_deref(), Some("Brave Browser"));
        // nome desconhecido passa como veio (o SO decide)
        assert_eq!(normalize_browser(Some("Orion")).unwrap().as_deref(), Some("Orion"));
        for bad in ["-a", "--args", "Sa\nfari", "x\u{7}"] {
            assert!(normalize_browser(Some(bad)).is_err(), "deveria recusar {bad:?}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn comando_do_mac_usa_open_a_quando_ha_navegador() {
        let args = |cmd: tokio::process::Command| -> Vec<String> {
            cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect()
        };
        assert_eq!(args(open_command("https://google.com", None)), vec!["https://google.com"]);
        assert_eq!(
            args(open_command("https://google.com", Some("Safari"))),
            vec!["-a", "Safari", "https://google.com"]
        );
    }
}
