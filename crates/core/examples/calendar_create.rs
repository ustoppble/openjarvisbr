//! Exemplo manual do `calendar.create`: cria um evento REAL no Calendário do Mac.
//!
//! Uso: `cargo run -p openjarvisbr-core --example calendar_create -- [INÍCIO] [TÍTULO]`
//! INÍCIO em horário local `AAAA-MM-DDTHH:MM`; sem ele, amanhã às 10:00.
//! Na primeira vez o macOS pede permissão de acesso ao Calendário.

use openjarvisbr_core::tools::system::calendar::CalendarCreate;
use openjarvisbr_core::tools::Tool;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let start = match args.next() {
        Some(start) => start,
        None => tomorrow_at_ten()?,
    };
    let title = args
        .next()
        .unwrap_or_else(|| "Teste do Jarvis (calendar.create)".to_string());

    let call_args = json!({
        "title": title,
        "start": start,
        "duration_min": 30,
        "notes": "Criado pelo example calendar_create do OpenJarvisBR. Pode apagar.",
    });
    println!("Criando evento: {call_args}");
    let result = CalendarCreate.call(call_args).await?;
    println!("Resultado: {result}");
    Ok(())
}

/// Amanhã às 10:00 no fuso local, via `date` do macOS.
fn tomorrow_at_ten() -> Result<String, Box<dyn std::error::Error>> {
    let out = std::process::Command::new("date")
        .args(["-v+1d", "+%Y-%m-%dT10:00"])
        .output()?;
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}
