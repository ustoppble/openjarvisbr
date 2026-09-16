//! `media.control`: tocar, pausar, próxima e anterior no Music ou Spotify.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{optional_str, required_str, run_osascript};
use crate::tools::{Risk, Tool, ToolError, ToolSpec};

const NAME: &str = "media.control";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
}

impl MediaAction {
    /// Comando AppleScript; o Music e o Spotify usam os mesmos termos.
    fn command(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Toggle => "playpause",
            Self::Next => "next track",
            Self::Previous => "previous track",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Player {
    /// O player que está tocando; senão o que estiver aberto (Spotify, depois Music).
    Auto,
    Music,
    Spotify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaRequest {
    pub action: MediaAction,
    pub player: Player,
}

impl MediaRequest {
    pub fn from_args(args: &Value) -> Result<Self, ToolError> {
        let action = match required_str(args, "action")?.as_str() {
            "play" => MediaAction::Play,
            "pause" => MediaAction::Pause,
            "toggle" => MediaAction::Toggle,
            "next" => MediaAction::Next,
            "previous" | "prev" => MediaAction::Previous,
            other => {
                return Err(ToolError::InvalidArgs(format!(
                    "`action` desconhecida `{other}`; use play, pause, toggle, next ou previous"
                )))
            }
        };
        let player = match optional_str(args, "app")?.map(|s| s.to_lowercase()).as_deref() {
            None | Some("auto") => Player::Auto,
            Some("music") => Player::Music,
            Some("spotify") => Player::Spotify,
            Some(other) => {
                return Err(ToolError::InvalidArgs(format!(
                    "`app` desconhecido `{other}`; use auto, music ou spotify"
                )))
            }
        };
        Ok(Self { action, player })
    }

    /// Script que devolve o nome do player usado, ou `none` se nenhum está
    /// aberto. O `tell` vai dentro de `run script` para compilar só em tempo de
    /// execução: um `tell application "Spotify"` literal faria o macOS
    /// perguntar onde está o app quando ele não está instalado.
    pub fn applescript(self) -> String {
        let candidates = match self.player {
            Player::Auto => r#"{"Spotify", "Music"}"#,
            Player::Music => r#"{"Music"}"#,
            Player::Spotify => r#"{"Spotify"}"#,
        };
        format!(
            r#"set candidates to {candidates}
set openApps to {{}}
repeat with appName in candidates
	if application (appName as text) is running then set end of openApps to (appName as text)
end repeat
if openApps is {{}} then return "none"
set chosen to item 1 of openApps
repeat with appName in openApps
	try
		if (run script "tell application \"" & appName & "\" to (player state as text)") is "playing" then
			set chosen to (appName as text)
			exit repeat
		end if
	end try
end repeat
run script "tell application \"" & chosen & "\" to {command}"
return chosen"#,
            command = self.action.command(),
        )
    }
}

pub struct MediaControl;

#[async_trait]
impl Tool for MediaControl {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: NAME.into(),
            description: "Controla a música tocando no Music ou no Spotify: tocar, pausar, alternar, próxima e anterior."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["play", "pause", "toggle", "next", "previous"]
                    },
                    "app": {
                        "type": "string",
                        "enum": ["auto", "music", "spotify"],
                        "description": "Player alvo; auto (padrão) usa o que estiver tocando ou aberto."
                    }
                },
                "required": ["action"]
            }),
            risk: Risk::Safe,
        }
    }

    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        let req = MediaRequest::from_args(&args)?;
        let out = run_osascript(NAME, &req.applescript()).await?;
        if out.trim() == "none" {
            return Err(ToolError::Failed(
                "nenhum player aberto (abra o Music ou o Spotify)".into(),
            ));
        }
        Ok(json!({ "app": out.trim(), "action": req.action.command() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valida_argumentos() {
        let req = MediaRequest::from_args(&json!({"action": "next"})).unwrap();
        assert_eq!(req, MediaRequest { action: MediaAction::Next, player: Player::Auto });
        let req = MediaRequest::from_args(&json!({"action": "prev", "app": "Spotify"})).unwrap();
        assert_eq!(req, MediaRequest { action: MediaAction::Previous, player: Player::Spotify });
        let req = MediaRequest::from_args(&json!({"action": "pause", "app": "music"})).unwrap();
        assert_eq!(req.player, Player::Music);
        for bad in [
            json!({}),
            json!({"action": "stop"}),
            json!({"action": 1}),
            json!({"action": "play", "app": "vlc"}),
        ] {
            assert!(
                matches!(MediaRequest::from_args(&bad), Err(ToolError::InvalidArgs(_))),
                "deveria recusar {bad}"
            );
        }
    }

    #[test]
    fn gera_applescript_por_acao_e_player() {
        let script = MediaRequest { action: MediaAction::Toggle, player: Player::Auto }.applescript();
        assert!(script.starts_with(r#"set candidates to {"Spotify", "Music"}"#));
        assert!(script.contains(r#"to playpause""#));
        assert!(script.contains(r#"return "none""#));
        // Nenhum tell literal: o app só é resolvido em tempo de execução.
        assert!(!script.contains(r#"tell application "Spotify""#));

        let script = MediaRequest { action: MediaAction::Next, player: Player::Music }.applescript();
        assert!(script.starts_with(r#"set candidates to {"Music"}"#));
        assert!(script.contains(r#"to next track""#));

        let script = MediaRequest { action: MediaAction::Previous, player: Player::Spotify }.applescript();
        assert!(script.contains(r#"to previous track""#));
    }
}
