//! Medidor de nível de captura: abre o microfone padrão (ou `--device-in`),
//! reamostra para 16kHz i16 mono e imprime um medidor RMS simples no
//! terminal para cada chunk de 20ms recebido. Ctrl+C encerra.
//!
//! Uso: `cargo run --example capture_meter [-- --device-in "Nome"]`

use openjarvisbr_core::audio::capture;

use std::sync::mpsc;
use std::time::Duration;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let device_in = std::env::args()
        .skip_while(|a| a != "--device-in")
        .nth(1);

    let (tx, rx) = mpsc::channel::<Vec<i16>>();

    let handle = match capture::start(device_in.as_deref(), tx) {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("não foi possível abrir o microfone: {err}");
            eprintln!("dispositivos de entrada disponíveis:");
            for name in capture::list_input_devices() {
                eprintln!("  - {name}");
            }
            std::process::exit(3);
        }
    };

    println!("capturando (Ctrl+C para sair)...");

    loop {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(chunk) => print_meter(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                eprintln!("(sem áudio nos últimos 2s)");
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    handle.stop();
}

fn print_meter(chunk: &[i16]) {
    let rms = rms_level(chunk);
    let bars = ((rms / i16::MAX as f64) * 40.0).round() as usize;
    println!("[{:<40}] rms={rms:.0}", "#".repeat(bars.min(40)));
}

fn rms_level(chunk: &[i16]) -> f64 {
    if chunk.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = chunk.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum_sq / chunk.len() as f64).sqrt()
}
