//! Ponto de entrada da OpenJarvisBR: parseia flags, carrega config, sobe o
//! Engine e mostra no terminal o que ele publica.

mod script;
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

    /// Modo roteiro: manda cada linha do arquivo como fala do usuário (mic
    /// mudo), espera o turno terminar e sai. Para medir sem falar; combine
    /// com RUST_LOG=openjarvisbr_core=debug e tools/jarvis_trace_report.py
    #[arg(long, value_name = "ARQUIVO")]
    script: Option<std::path::PathBuf>,

    /// Ativa logs em nível debug
    #[arg(long)]
    debug: bool,

    #[command(subcommand)]
    command: Option<Sub>,
}

#[derive(clap::Subcommand, Debug)]
enum Sub {
    /// Diagnóstico do reflexo (Jev): pergunta com uma frase ou lista o inventário
    Reflex {
        /// Frase como se fosse a transcrição do usuário
        phrase: Option<String>,
        /// Só imprime o inventário do Olho (apps rodando, instalados, sites)
        #[arg(long)]
        eye: bool,
        /// Simula confirmação pendente (testa "sim"/"não")
        #[arg(long)]
        pending: bool,
    },
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

    // Diagnóstico do reflexo não abre microfone nem Gemini: roda antes do
    // lock de instância única e da exigência de chave do Gemini.
    if let Some(Sub::Reflex { phrase, eye, pending }) = cli.command {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        std::process::exit(runtime.block_on(reflex_diagnostic(phrase, eye, pending)));
    }

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
        always_allow: settings.always_allow.clone(),
        reflex: settings.reflex.clone(),
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
    let exit_code = match cli.script.as_deref() {
        Some(path) => runtime.block_on(script::run(handle, path)),
        None => runtime.block_on(term::run(handle, barge_in, fx_amount, record_dir)),
    };
    std::process::exit(exit_code);
}

/// `jarvis reflex [frase] [--eye] [--pending]`: mostra o inventário do Olho,
/// as perguntas que o reflexo faria ao Jev para a frase, as respostas com
/// confiança, a decisão e a latência. Códigos: 0 ok, 2 sem chave/desligado,
/// 3 erro do Jev. A chave nunca é impressa.
async fn reflex_diagnostic(phrase: Option<String>, only_eye: bool, pending: bool) -> i32 {
    use openjarvisbr_core::reflex::decide::{build_questions, decide, Situation, Thresholds};
    use openjarvisbr_core::reflex::eye::{Eye, SiteConfig as EyeSite};
    use openjarvisbr_core::reflex::judge::{JevClient, Judge};

    let settings = config::load_reflex();
    // config e eye têm cada um o seu `SiteConfig` (mesmos campos); converte aqui.
    let sites: Vec<EyeSite> = settings
        .sites
        .iter()
        .map(|s| EyeSite { name: s.name.clone(), url: s.url.clone() })
        .collect();
    let eye = Eye::start(sites);
    // Dá tempo ao primeiro scan (apps instalados + rodando) terminar.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let inv = eye.snapshot();
    if only_eye || phrase.is_none() {
        let running: Vec<&str> = inv.running_apps.iter().map(|a| a.name.as_str()).collect();
        let sites: Vec<&str> = inv.sites.iter().map(|s| s.name.as_str()).collect();
        println!("apps rodando ({}): {}", running.len(), running.join(", "));
        println!("apps instalados: {}", inv.installed_apps.len());
        println!("sites ({}): {}", sites.len(), sites.join(", "));
        println!(
            "reflexo: {}",
            if settings.enabled { "ligado" } else { "desligado (sem chave ou enabled = false)" }
        );
        return 0;
    }
    let phrase = phrase.unwrap_or_default();
    let situation = Situation {
        heard: &phrase,
        inventory: &inv,
        pending_confirm: pending,
        turn_locked: false,
    };
    let Some(questions) = build_questions(&situation) else {
        println!("decisão: Nothing");
        println!("Jev não consultado: nenhum candidato na fala");
        println!("latência: 0 ms");
        println!(
            "resumo da sessão: total 1 · Act 0 · Nothing 1 · latência média 0 ms · perguntas puladas 1"
        );
        return 0;
    };
    let ids: Vec<&str> = questions.0.keys().map(|k| k.as_str()).collect();
    println!("perguntas ({}): {}", questions.len(), ids.join(", "));
    if !settings.enabled {
        eprintln!(
            "reflexo desligado: configure typesafe_api_key ou openrouter_api_key no config.toml \
             (ou as envs TYPESAFE_API_KEY / OPENROUTER_API_KEY)"
        );
        return 2;
    }
    println!("provedor: {} · modelo: {}", settings.provider.label(), settings.model);
    let client = match JevClient::new(settings.api_key.clone().unwrap_or_default(), settings.model.clone()) {
        Ok(client) => client.with_base_url(&settings.endpoint),
        Err(err) => {
            eprintln!("{err}");
            return 2;
        }
    };
    let t0 = std::time::Instant::now();
    let answers = match client.ask(&phrase, &questions).await {
        Ok(answers) => answers,
        Err(err) => {
            eprintln!("erro do Jev: {err}");
            return 3;
        }
    };
    let ms = t0.elapsed().as_millis();
    for id in questions.0.keys() {
        if let Some((pick, conf)) = answers.choice(id) {
            println!("  {id:<8} → {pick} ({conf:.2})");
        } else if let Some(v) = answers.noul(id) {
            println!("  {id:<8} → {v:.2}");
        }
    }
    let thresholds = Thresholds {
        act: settings.act_threshold,
        confirm: settings.confirm_threshold,
    };
    println!("decisão: {:?}", decide(&situation, &answers, &thresholds));
    println!(
        "latência: {ms} ms · tokens {}+{}",
        answers.usage.input_tokens, answers.usage.output_tokens
    );
    0
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
