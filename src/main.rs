//! Ponto de entrada da OpenJarvisBR: parseia flags, carrega config e sobe a App.

mod app;
mod audio;
mod config;
mod live;

use clap::Parser;

use app::App;

/// Assistente de voz OpenJarvisBR — conversa contínua com o gemini-3.8-live.
#[derive(Parser, Debug)]
#[command(name = "jarvis", version, about)]
struct Cli {
    /// Voz usada pelo modelo
    #[arg(long, default_value = "Puck")]
    voice: String,

    /// Dispositivo de entrada de áudio (microfone)
    #[arg(long)]
    device_in: Option<String>,

    /// Dispositivo de saída de áudio (alto-falante)
    #[arg(long)]
    device_out: Option<String>,

    /// Ativa logs em nível debug
    #[arg(long)]
    debug: bool,
}

fn main() {
    let cli = Cli::parse();
    init_tracing(cli.debug);

    let api_key = match config::load_api_key() {
        Ok(key) => key,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };

    tracing::debug!(
        voz = %cli.voice,
        device_in = ?cli.device_in,
        device_out = ?cli.device_out,
        "flags carregadas"
    );

    let mut app = App::new();
    app.run(api_key.len());
}

fn init_tracing(debug: bool) {
    let level = if debug { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
        )
        .init();
}
