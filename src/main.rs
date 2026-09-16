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

    /// Grava playback.wav, mic.wav e events.log nesta pasta (diagnóstico)
    #[arg(long, value_name = "PASTA")]
    record: Option<std::path::PathBuf>,

    /// Ativa logs em nível debug
    #[arg(long)]
    debug: bool,
}

/// Garante uma única instância por usuário: um segundo `jarvis` ouviria o
/// mesmo microfone e falaria por cima do primeiro (duas vozes, cortes).
/// O handle do arquivo precisa viver até o fim do processo — o lock cai
/// junto com ele, inclusive em kill/pane fechado.
fn acquire_single_instance_lock() -> Result<std::fs::File, String> {
    use fs4::fs_std::FileExt;
    use std::io::{Read, Seek, Write};

    let dir = std::env::temp_dir().join("openjarvisbr");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("jarvis.lock");
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| e.to_string())?;
    if file.try_lock_exclusive().is_err() {
        let mut other = String::new();
        let _ = file.read_to_string(&mut other);
        let other = other.trim();
        return Err(if other.is_empty() {
            "já existe um Jarvis rodando. Feche o outro antes de abrir este.".to_string()
        } else {
            format!("já existe um Jarvis rodando (pid {other}). Feche o outro antes de abrir este.")
        });
    }
    let _ = file.set_len(0);
    let _ = file.seek(std::io::SeekFrom::Start(0));
    let _ = write!(file, "{}", std::process::id());
    let _ = file.flush();
    Ok(file)
}

fn main() {
    let cli = Cli::parse();
    init_tracing(cli.debug);

    let _instance_lock = match acquire_single_instance_lock() {
        Ok(lock) => lock,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(6);
        }
    };

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
        record_dir: cli.record,
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
