//! `calendar.create`: cria um evento no Calendário do macOS (pede confirmação).

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{applescript_string, args_object, optional_str, required_str, run_osascript, LocalDateTime};
use crate::tools::{Risk, Tool, ToolError, ToolSpec};

const NAME: &str = "calendar.create";
const DEFAULT_DURATION_MIN: u32 = 60;
const MAX_DURATION_MIN: u32 = 7 * 24 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    pub title: String,
    pub start: LocalDateTime,
    pub duration_min: u32,
    pub notes: Option<String>,
    /// Nome do calendário; `None` = primeiro calendário editável.
    pub calendar: Option<String>,
}

impl CalendarEvent {
    pub fn from_args(args: &Value) -> Result<Self, ToolError> {
        let title = required_str(args, "title")?;
        let start = LocalDateTime::parse_iso(&required_str(args, "start")?)?;
        let duration_min = match args_object(args)?.get("duration_min") {
            None | Some(Value::Null) => DEFAULT_DURATION_MIN,
            Some(v) => v
                .as_u64()
                .filter(|d| (1..=MAX_DURATION_MIN as u64).contains(d))
                .ok_or_else(|| {
                    ToolError::InvalidArgs(format!(
                        "`duration_min` deve ser um inteiro de 1 a {MAX_DURATION_MIN}"
                    ))
                })? as u32,
        };
        Ok(Self {
            title,
            start,
            duration_min,
            notes: optional_str(args, "notes")?,
            calendar: optional_str(args, "calendar")?,
        })
    }

    /// Script que cria o evento e devolve o `uid` dele.
    pub fn applescript(&self) -> String {
        let target = match &self.calendar {
            Some(name) => format!("calendar {}", applescript_string(name)),
            None => "first calendar whose writable is true".to_string(),
        };
        let mut props = format!(
            "summary:{}, start date:startDate, end date:endDate",
            applescript_string(&self.title)
        );
        if let Some(notes) = &self.notes {
            props.push_str(&format!(", description:{}", applescript_string(notes)));
        }
        format!(
            "{}set endDate to startDate + {}\n\
             tell application \"Calendar\"\n\
             \tset targetCal to {target}\n\
             \tset newEvent to make new event at end of events of targetCal with properties {{{props}}}\n\
             \treturn uid of newEvent\n\
             end tell",
            self.start.applescript_assign("startDate"),
            u64::from(self.duration_min) * 60,
        )
    }
}

pub struct CalendarCreate;

#[async_trait]
impl Tool for CalendarCreate {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: NAME.into(),
            description: "Cria um evento no Calendário do Mac.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Título do evento." },
                    "start": {
                        "type": "string",
                        "description": "Início em horário local, AAAA-MM-DDTHH:MM, sem fuso."
                    },
                    "duration_min": {
                        "type": "integer", "minimum": 1, "maximum": MAX_DURATION_MIN,
                        "description": "Duração em minutos (padrão 60)."
                    },
                    "notes": { "type": "string", "description": "Notas do evento." },
                    "calendar": {
                        "type": "string",
                        "description": "Nome do calendário; sem ele usa o primeiro editável."
                    }
                },
                "required": ["title", "start"]
            }),
            risk: Risk::Confirm,
        }
    }

    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        let event = CalendarEvent::from_args(&args)?;
        let uid = run_osascript(NAME, &event.applescript()).await?;
        Ok(json!({
            "created": true,
            "uid": uid,
            "title": event.title,
            "start": event.start.to_iso(),
            "duration_min": event.duration_min,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valida_argumentos() {
        let ev = CalendarEvent::from_args(&json!({
            "title": "Reunião", "start": "2026-09-17T15:00"
        }))
        .unwrap();
        assert_eq!(ev.duration_min, 60);
        assert_eq!(ev.notes, None);
        assert_eq!(ev.calendar, None);

        let ev = CalendarEvent::from_args(&json!({
            "title": "Call", "start": "2026-09-17T15:00", "duration_min": 30,
            "notes": "pauta", "calendar": "Trabalho"
        }))
        .unwrap();
        assert_eq!(ev.duration_min, 30);
        assert_eq!(ev.notes.as_deref(), Some("pauta"));

        for bad in [
            json!({"start": "2026-09-17T15:00"}),
            json!({"title": "  ", "start": "2026-09-17T15:00"}),
            json!({"title": "x"}),
            json!({"title": "x", "start": "amanhã às 3"}),
            json!({"title": "x", "start": "2026-09-17T15:00Z"}),
            json!({"title": "x", "start": "2026-09-17T15:00", "duration_min": 0}),
            json!({"title": "x", "start": "2026-09-17T15:00", "duration_min": 99999}),
            json!({"title": "x", "start": "2026-09-17T15:00", "duration_min": "30"}),
        ] {
            assert!(
                matches!(CalendarEvent::from_args(&bad), Err(ToolError::InvalidArgs(_))),
                "deveria recusar {bad}"
            );
        }
    }

    #[test]
    fn gera_applescript() {
        let ev = CalendarEvent::from_args(&json!({
            "title": "Dentista \"Dr. X\"", "start": "2026-09-17T15:30", "duration_min": 45,
            "notes": "levar exame"
        }))
        .unwrap();
        let script = ev.applescript();
        assert!(script.contains("set hours of startDate to 15\n"));
        assert!(script.contains("set minutes of startDate to 30\n"));
        assert!(script.contains("set endDate to startDate + 2700\n"));
        assert!(script.contains("tell application \"Calendar\""));
        assert!(script.contains("set targetCal to first calendar whose writable is true"));
        assert!(script.contains(
            r#"with properties {summary:"Dentista \"Dr. X\"", start date:startDate, end date:endDate, description:"levar exame"}"#
        ));
        assert!(script.ends_with("end tell"));

        let ev = CalendarEvent::from_args(&json!({
            "title": "x", "start": "2026-09-17T15:30", "calendar": "Casa"
        }))
        .unwrap();
        let script = ev.applescript();
        assert!(script.contains(r#"set targetCal to calendar "Casa""#));
        assert!(!script.contains("description:"));
    }

    #[test]
    fn spec_pede_confirmacao() {
        assert_eq!(CalendarCreate.spec().risk, Risk::Confirm);
    }
}
