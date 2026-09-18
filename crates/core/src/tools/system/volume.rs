//! `sys.volume`: lê, ajusta (0–100, ou ±10 com up/down) e silencia o volume de saída.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{args_object, required_str, run_osascript};
use crate::tools::{Risk, Tool, ToolError, ToolSpec};

const NAME: &str = "sys.volume";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeAction {
    Get,
    Set(u8),
    /// Soma 10 ao volume atual (o macOS satura em 100).
    Up,
    /// Tira 10 do volume atual (o macOS satura em 0).
    Down,
    Mute,
    Unmute,
}

impl VolumeAction {
    pub fn from_args(args: &Value) -> Result<Self, ToolError> {
        let action = required_str(args, "action")?;
        match action.as_str() {
            "get" => Ok(Self::Get),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "mute" => Ok(Self::Mute),
            "unmute" => Ok(Self::Unmute),
            "set" => {
                let level = args_object(args)?
                    .get("level")
                    .ok_or_else(|| ToolError::InvalidArgs("`level` é obrigatório em set".into()))?;
                let level = level
                    .as_f64()
                    .filter(|l| (0.0..=100.0).contains(l))
                    .ok_or_else(|| {
                        ToolError::InvalidArgs("`level` deve ser um número de 0 a 100".into())
                    })?;
                Ok(Self::Set(level.round() as u8))
            }
            other => Err(ToolError::InvalidArgs(format!(
                "`action` desconhecida `{other}`; use get, set, up, down, mute ou unmute"
            ))),
        }
    }

    pub fn applescript(self) -> String {
        match self {
            Self::Get => "set s to get volume settings\n\
                          return (output volume of s as text) & \",\" & (output muted of s as text)"
                .to_string(),
            Self::Set(level) => format!("set volume output volume {level}"),
            Self::Up => "set volume output volume ((output volume of (get volume settings)) + 10)"
                .to_string(),
            Self::Down => "set volume output volume ((output volume of (get volume settings)) - 10)"
                .to_string(),
            Self::Mute => "set volume with output muted".to_string(),
            Self::Unmute => "set volume without output muted".to_string(),
        }
    }
}

/// Interpreta a saída `nível,mudo` do script de `get`.
fn parse_get_output(out: &str) -> Result<Value, ToolError> {
    let (level, muted) = out
        .trim()
        .split_once(',')
        .ok_or_else(|| ToolError::Failed(format!("saída inesperada do osascript: {out}")))?;
    // Sem dispositivo de saída com volume, o macOS devolve "missing value".
    let level = level.trim().parse::<u8>().ok();
    Ok(json!({ "level": level, "muted": muted.trim() == "true" }))
}

pub struct SysVolume;

#[async_trait]
impl Tool for SysVolume {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: NAME.into(),
            description: "Lê ou ajusta o volume de saída do computador (0 a 100, ou 10 pontos para cima/baixo) e liga/desliga o mudo."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "set", "up", "down", "mute", "unmute"],
                        "description": "get lê o volume; set ajusta para `level`; up/down sobem ou descem 10 pontos; mute/unmute silenciam ou reativam o som."
                    },
                    "level": {
                        "type": "integer", "minimum": 0, "maximum": 100,
                        "description": "Volume de 0 a 100, obrigatório em set."
                    }
                },
                "required": ["action"]
            }),
            risk: Risk::Safe,
        }
    }

    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        let action = VolumeAction::from_args(&args)?;
        let out = run_osascript(NAME, &action.applescript()).await?;
        match action {
            VolumeAction::Get => parse_get_output(&out),
            VolumeAction::Set(level) => Ok(json!({ "level": level })),
            VolumeAction::Up | VolumeAction::Down => {
                // Devolve o nível resultante para o modelo saber onde ficou.
                let out = run_osascript(NAME, &VolumeAction::Get.applescript()).await?;
                parse_get_output(&out)
            }
            VolumeAction::Mute => Ok(json!({ "muted": true })),
            VolumeAction::Unmute => Ok(json!({ "muted": false })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valida_acoes() {
        assert_eq!(VolumeAction::from_args(&json!({"action": "get"})).unwrap(), VolumeAction::Get);
        assert_eq!(
            VolumeAction::from_args(&json!({"action": "set", "level": 40})).unwrap(),
            VolumeAction::Set(40)
        );
        assert_eq!(
            VolumeAction::from_args(&json!({"action": "set", "level": 0})).unwrap(),
            VolumeAction::Set(0)
        );
        assert_eq!(
            VolumeAction::from_args(&json!({"action": "mute"})).unwrap(),
            VolumeAction::Mute
        );
        for bad in [
            json!({}),
            json!("get"),
            json!({"action": "louder"}),
            json!({"action": "set"}),
            json!({"action": "set", "level": 101}),
            json!({"action": "set", "level": -1}),
            json!({"action": "set", "level": "50"}),
        ] {
            assert!(
                matches!(VolumeAction::from_args(&bad), Err(ToolError::InvalidArgs(_))),
                "deveria recusar {bad}"
            );
        }
    }

    #[test]
    fn gera_applescript() {
        assert_eq!(VolumeAction::Set(35).applescript(), "set volume output volume 35");
        assert_eq!(VolumeAction::Mute.applescript(), "set volume with output muted");
        assert_eq!(VolumeAction::Unmute.applescript(), "set volume without output muted");
        let get = VolumeAction::Get.applescript();
        assert!(get.contains("get volume settings"));
        assert!(get.contains("output muted of s"));
    }

    #[test]
    fn interpreta_saida_do_get() {
        assert_eq!(parse_get_output("42,false\n").unwrap(), json!({"level": 42, "muted": false}));
        assert_eq!(
            parse_get_output("missing value,true").unwrap(),
            json!({"level": null, "muted": true})
        );
        assert!(parse_get_output("lixo").is_err());
    }

    #[test]
    fn spec_segura() {
        let spec = SysVolume.spec();
        assert_eq!(spec.name, "sys.volume");
        assert_eq!(spec.risk, Risk::Safe);
    }

    #[test]
    fn aceita_up_e_down() {
        assert_eq!(VolumeAction::from_args(&json!({"action": "up"})).unwrap(), VolumeAction::Up);
        assert_eq!(
            VolumeAction::from_args(&json!({"action": "down"})).unwrap(),
            VolumeAction::Down
        );
        assert_eq!(
            VolumeAction::Up.applescript(),
            "set volume output volume ((output volume of (get volume settings)) + 10)"
        );
        assert_eq!(
            VolumeAction::Down.applescript(),
            "set volume output volume ((output volume of (get volume settings)) - 10)"
        );
        let spec = SysVolume.spec();
        let actions = spec.parameters["properties"]["action"]["enum"].clone();
        assert_eq!(actions, json!(["get", "set", "up", "down", "mute", "unmute"]));
    }
}
