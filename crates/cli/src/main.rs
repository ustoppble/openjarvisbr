//! Ponto de entrada da OpenJarvisBR: parseia flags, carrega config, sobe o
//! Engine e mostra no terminal o que ele publica.

mod term;

use clap::Parser;

use openjarvisbr_core::config;
use openjarvisbr_core::engine::{Engine, EngineConfig};

/// Assistente de voz OpenJarvisBR — conversa contínua com o gemini-3.8-live.
#[derive(Parser, Debug)]
#[command(name = "jarvis", version, about)]
struct Cli {
    /// Voz usada pelo modelo (padrão: config.toml ou Puck)
    #[arg(long)]
    voice: Option<String>,

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

    /// Intensidade do efeito de voz "Jarvis", 0.0 (desliga) a 1.0
    #[arg(long, value_name = "0..1")]
    fx_amount: Option<f32>,

    /// Grava playback.wav, mic.wav e events.log nesta pasta (diagnóstico)
    #[arg(long, value_name = "PASTA")]
    record: Option<std::path::PathBuf>,

    /// Perfil de personalidade a usar (padrão: config.toml ou "assistant")
    #[arg(long, value_name = "ID")]
    profile: Option<String>,

    /// Acesso total: nenhuma ferramenta pede confirmação e os arquivos podem
    /// estar fora do home. Também liga com [tools].full_access no config.toml
    #[arg(long)]
    full_access: bool,

    /// Lista os perfis disponíveis (embutidos + os do config.toml) e sai
    #[arg(long)]
    list_profiles: bool,

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

    if cli.list_profiles {
        let settings = config::load_settings();
        for profile in config::all_profiles(&settings) {
            println!("{:<18} {:<24} {}", profile.id, profile.name, profile.description);
        }
        return;
    }

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
        voz = ?cli.voice,
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

    // Flag na linha de comando vence o config.toml, que vence o perfil ativo.
    let mut settings = config::load_settings();
    if let Some(id) = cli.profile.clone() {
        settings.profile = Some(id);
    }
    let system_prompt = config::effective_system_prompt(&settings);
    let engine_config = EngineConfig {
        api_key,
        voice: cli.voice.unwrap_or_else(|| config::effective_voice(&settings)),
        device_in: cli.device_in.or(settings.device_in.clone()),
        device_out: cli.device_out.or(settings.device_out.clone()),
        barge_in: cli.barge_in || settings.barge_in.unwrap_or(false),
        record_dir: cli.record,
        fx_amount: cli.fx_amount.unwrap_or_else(|| config::effective_fx_amount(&settings)),
        system_prompt,
        greeting: None,
        tools: config::effective_tool_globs(&settings),
        mcp_servers: settings.mcp_servers.clone(),
        full_access: cli.full_access || settings.full_access,
    };
    if engine_config.full_access {
        eprintln!("⚠ acesso total ligado: o Jarvis executa comandos e mexe em arquivos sem perguntar");
    }
    let barge_in = engine_config.barge_in;
    let fx_amount = engine_config.fx_amount.clamp(0.0, 1.0);
    let record_dir = engine_config.record_dir.clone();

    let handle = match runtime.block_on(Engine::start(engine_config)) {
        Ok(handle) => handle,
        Err(err) => {
            term::report_start_error(&err);
            std::process::exit(err.exit_code());
        }
    };
    let exit_code = runtime.block_on(term::run(handle, barge_in, fx_amount, record_dir));
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
