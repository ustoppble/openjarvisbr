//! Exemplo manual do `Player`: toca um tom de 1s e depois um tom de 5s que é
//! interrompido por `flush()` após 1s (o som deve parar em ~1s).

use std::f32::consts::PI;
use std::thread::sleep;
use std::time::Duration;

use openjarvisbr_core::audio::playback::Player;

const SAMPLE_RATE: u32 = 24_000;

/// Gera um tom senoidal em PCM i16 mono na taxa de amostragem do player.
fn sine_wave(freq_hz: f32, duration_secs: f32) -> Vec<i16> {
    let n = (SAMPLE_RATE as f32 * duration_secs) as usize;
    let amplitude = i16::MAX as f32 * 0.2;
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            (amplitude * (2.0 * PI * freq_hz * t).sin()) as i16
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let player = Player::new(None)?;

    println!("Tocando tom de 1s...");
    player.push(&sine_wave(440.0, 1.0));
    sleep(Duration::from_millis(1_200));

    println!("Tocando tom de 5s, com flush após 1s (o som deve parar em ~1s)...");
    player.push(&sine_wave(440.0, 5.0));
    sleep(Duration::from_secs(1));
    player.flush();
    println!("flush() chamado.");
    sleep(Duration::from_millis(500));

    Ok(())
}
