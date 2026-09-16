//! Ferramentas de sistema: volume, mídia, agenda e lembretes.
//!
//! No macOS tudo passa pelo `osascript`. Cada ferramenta separa a validação
//! dos argumentos e a geração do AppleScript (funções puras, testáveis sem
//! executar nada) da execução. Fora do macOS a chamada devolve o erro
//! "não suportado ainda" em vez de panicar.

pub mod calendar;
pub mod media;
pub mod reminder;
pub mod volume;

use std::time::Duration;

use serde_json::Value;

use super::{Tool, ToolError};

/// Todas as ferramentas de sistema, prontas para registrar.
pub fn all() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(volume::SysVolume),
        Box::new(media::MediaControl),
        Box::new(calendar::CalendarCreate),
        Box::new(reminder::ReminderSet),
    ]
}

/// Tempo máximo de um `osascript`. Folgado porque o primeiro acesso ao
/// Calendário/Lembretes abre o pedido de permissão do macOS.
const OSASCRIPT_TIMEOUT: Duration = Duration::from_secs(30);

/// Erro padrão para sistemas sem implementação.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn unsupported(tool: &str) -> ToolError {
    ToolError::Failed(format!(
        "{tool} não suportado ainda neste sistema (só macOS por enquanto)"
    ))
}

/// Executa um AppleScript e devolve o stdout sem a quebra de linha final.
#[cfg(target_os = "macos")]
async fn run_osascript(_tool: &str, script: &str) -> Result<String, ToolError> {
    let child = tokio::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(OSASCRIPT_TIMEOUT, child)
        .await
        .map_err(|_| ToolError::Timeout)?
        .map_err(|e| ToolError::Failed(format!("não consegui rodar osascript: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(ToolError::Failed(stderr.trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

#[cfg(not(target_os = "macos"))]
async fn run_osascript(tool: &str, _script: &str) -> Result<String, ToolError> {
    let _ = OSASCRIPT_TIMEOUT;
    Err(unsupported(tool))
}

/// Literal de string AppleScript entre aspas, com `\` e `"` escapados.
fn applescript_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn args_object(args: &Value) -> Result<&serde_json::Map<String, Value>, ToolError> {
    args.as_object()
        .ok_or_else(|| ToolError::InvalidArgs("esperava um objeto JSON".into()))
}

/// Campo de texto obrigatório e não vazio.
fn required_str(args: &Value, key: &str) -> Result<String, ToolError> {
    optional_str(args, key)?.ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` é obrigatório")))
}

/// Campo de texto opcional; string vazia conta como ausente.
fn optional_str(args: &Value, key: &str) -> Result<Option<String>, ToolError> {
    match args_object(args)?.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(ToolError::InvalidArgs(format!("`{key}` deve ser texto"))),
    }
}

/// Data e hora local, sem fuso (o Calendário/Lembretes usam o fuso do Mac).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl LocalDateTime {
    /// Aceita `AAAA-MM-DDTHH:MM[:SS]` (ou com espaço no lugar do `T`).
    /// Recusa fuso (`Z`, `+03:00`) para não criar evento na hora errada.
    pub fn parse_iso(s: &str) -> Result<Self, ToolError> {
        let bad = || {
            ToolError::InvalidArgs(format!(
                "data/hora `{s}` inválida; use horário local AAAA-MM-DDTHH:MM, sem fuso"
            ))
        };
        let s = s.trim();
        let (date, time) = s.split_once(['T', ' ']).ok_or_else(bad)?;
        let num = |part: &str, len: usize| -> Result<u32, ToolError> {
            if part.len() != len || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            part.parse().map_err(|_| bad())
        };
        let d: Vec<&str> = date.split('-').collect();
        let t: Vec<&str> = time.split(':').collect();
        if d.len() != 3 || !(2..=3).contains(&t.len()) {
            return Err(bad());
        }
        let year = num(d[0], 4)?;
        let month = num(d[1], 2)?;
        let day = num(d[2], 2)?;
        let hour = num(t[0], 2)?;
        let minute = num(t[1], 2)?;
        let second = if t.len() == 3 { num(t[2], 2)? } else { 0 };
        if !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return Err(bad());
        }
        Ok(Self {
            year: year as u16,
            month: month as u8,
            day: day as u8,
            hour: hour as u8,
            minute: minute as u8,
            second: second as u8,
        })
    }

    pub fn to_iso(self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// AppleScript que monta a data na variável `var`, campo a campo, sem
    /// depender do formato de data do idioma do sistema. O dia vai para 1
    /// antes de trocar o mês para não transbordar (31 jan → fev).
    fn applescript_assign(self, var: &str) -> String {
        format!(
            "set {var} to current date\n\
             set day of {var} to 1\n\
             set year of {var} to {}\n\
             set month of {var} to {}\n\
             set day of {var} to {}\n\
             set hours of {var} to {}\n\
             set minutes of {var} to {}\n\
             set seconds of {var} to {}\n",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapa_strings_applescript() {
        assert_eq!(applescript_string(r#"a "b" \c"#), r#""a \"b\" \\c""#);
        assert_eq!(applescript_string("x\r\ny"), "\"x\ny\"");
    }

    #[test]
    fn all_tem_as_quatro_ferramentas() {
        let names: Vec<String> = all().iter().map(|t| t.spec().name).collect();
        assert_eq!(
            names,
            ["sys.volume", "media.control", "calendar.create", "reminder.set"]
        );
    }

    #[test]
    fn parse_iso_aceita_formatos_locais() {
        let dt = LocalDateTime::parse_iso("2026-09-17T15:30").unwrap();
        assert_eq!(dt.to_iso(), "2026-09-17T15:30:00");
        let dt = LocalDateTime::parse_iso("2024-02-29 08:05:09").unwrap();
        assert_eq!(dt.to_iso(), "2024-02-29T08:05:09");
    }

    #[test]
    fn parse_iso_recusa_invalidos_e_fuso() {
        for s in [
            "",
            "2026-09-17",
            "2026-13-01T10:00",
            "2026-02-29T10:00",
            "2026-04-31T10:00",
            "2026-09-17T24:00",
            "2026-09-17T10:60",
            "2026-09-17T10:00Z",
            "2026-09-17T10:00:00+03:00",
            "26-09-17T10:00",
        ] {
            assert!(
                matches!(LocalDateTime::parse_iso(s), Err(ToolError::InvalidArgs(_))),
                "deveria recusar {s:?}"
            );
        }
    }

    #[test]
    fn data_applescript_campo_a_campo() {
        let dt = LocalDateTime::parse_iso("2026-01-31T09:00").unwrap();
        let script = dt.applescript_assign("d");
        let day1 = script.find("set day of d to 1\n").unwrap();
        let month = script.find("set month of d to 1\n").unwrap();
        assert!(day1 < month);
        assert!(script.contains("set year of d to 2026\n"));
        assert!(script.contains("set day of d to 31\n"));
        assert!(script.contains("set hours of d to 9\n"));
    }

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn fora_do_macos_erro_claro() {
        let err = run_osascript("sys.volume", "").await.unwrap_err();
        assert!(err.to_string().contains("não suportado ainda"));
    }
}
