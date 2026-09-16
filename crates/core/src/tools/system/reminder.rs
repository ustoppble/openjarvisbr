//! `reminder.set`: cria um lembrete com data e hora no app Lembretes do macOS.

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{applescript_string, optional_str, required_str, run_osascript, LocalDateTime};
use crate::tools::{Risk, Tool, ToolError, ToolSpec};

const NAME: &str = "reminder.set";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub title: String,
    pub due: LocalDateTime,
    pub notes: Option<String>,
    /// Nome da lista; `None` = lista padrão.
    pub list: Option<String>,
}

impl Reminder {
    pub fn from_args(args: &Value) -> Result<Self, ToolError> {
        Ok(Self {
            title: required_str(args, "title")?,
            due: LocalDateTime::parse_iso(&required_str(args, "due")?)?,
            notes: optional_str(args, "notes")?,
            list: optional_str(args, "list")?,
        })
    }

    /// Script que cria o lembrete (com alerta na hora) e devolve o `id` dele.
    pub fn applescript(&self) -> String {
        let target = match &self.list {
            Some(name) => format!("list {}", applescript_string(name)),
            None => "default list".to_string(),
        };
        let mut props = format!(
            "name:{}, due date:dueDate, remind me date:dueDate",
            applescript_string(&self.title)
        );
        if let Some(notes) = &self.notes {
            props.push_str(&format!(", body:{}", applescript_string(notes)));
        }
        format!(
            "{}tell application \"Reminders\"\n\
             \tset targetList to {target}\n\
             \tset newReminder to make new reminder at end of reminders of targetList with properties {{{props}}}\n\
             \treturn id of newReminder\n\
             end tell",
            self.due.applescript_assign("dueDate"),
        )
    }
}

pub struct ReminderSet;

#[async_trait]
impl Tool for ReminderSet {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: NAME.into(),
            description: "Cria um lembrete com data e hora no app Lembretes do Mac.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "O que lembrar." },
                    "due": {
                        "type": "string",
                        "description": "Quando avisar, em horário local AAAA-MM-DDTHH:MM, sem fuso."
                    },
                    "notes": { "type": "string", "description": "Detalhes do lembrete." },
                    "list": {
                        "type": "string",
                        "description": "Nome da lista; sem ele usa a lista padrão."
                    }
                },
                "required": ["title", "due"]
            }),
            risk: Risk::Safe,
        }
    }

    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        let reminder = Reminder::from_args(&args)?;
        let id = run_osascript(NAME, &reminder.applescript()).await?;
        Ok(json!({
            "created": true,
            "id": id,
            "title": reminder.title,
            "due": reminder.due.to_iso(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valida_argumentos() {
        let r = Reminder::from_args(&json!({"title": "Tomar remédio", "due": "2026-09-17 08:00"}))
            .unwrap();
        assert_eq!(r.due.to_iso(), "2026-09-17T08:00:00");
        assert_eq!(r.list, None);
        for bad in [
            json!({"due": "2026-09-17T08:00"}),
            json!({"title": "x"}),
            json!({"title": "x", "due": "08:00"}),
            json!({"title": "x", "due": "2026-09-17T08:00", "notes": 3}),
            json!(null),
        ] {
            assert!(
                matches!(Reminder::from_args(&bad), Err(ToolError::InvalidArgs(_))),
                "deveria recusar {bad}"
            );
        }
    }

    #[test]
    fn gera_applescript() {
        let r = Reminder::from_args(&json!({
            "title": "Ligar pro \\ banco", "due": "2026-12-31T23:59", "notes": "boleto"
        }))
        .unwrap();
        let script = r.applescript();
        assert!(script.contains("set month of dueDate to 12\n"));
        assert!(script.contains("set day of dueDate to 31\n"));
        assert!(script.contains("tell application \"Reminders\""));
        assert!(script.contains("set targetList to default list"));
        assert!(script.contains(
            r#"with properties {name:"Ligar pro \\ banco", due date:dueDate, remind me date:dueDate, body:"boleto"}"#
        ));

        let r = Reminder::from_args(&json!({"title": "x", "due": "2026-12-31T23:59", "list": "Compras"}))
            .unwrap();
        assert!(r.applescript().contains(r#"set targetList to list "Compras""#));
    }
}
