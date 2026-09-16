//! Ponto de entrada da OpenJarvisBR: parseia flags, carrega config e sobe a App.

mod app;
mod audio;
mod config;
mod live;

use clap::Parser;

use app::{App, AppConfig};

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

    /// Mantém o microfone aberto enquanto o Jarvis fala, permitindo
    /// interromper por voz. Use só com fone: com caixa de som o mic capta
    /// a própria voz dele e a conversa vira eco.
    #[arg(long)]
    barge_in: bool,

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

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("não foi possível iniciar o runtime assíncrono: {err}");
            std::process::exit(4);
        }
    };

    let mut app = App::new(AppConfig {
        api_key,
        voice: cli.voice,
        device_in: cli.device_in,
        device_out: cli.device_out,
        barge_in: cli.barge_in,
    });
    let exit_code = runtime.block_on(app.run());
    std::process::exit(exit_code);
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
