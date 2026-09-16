//! Resample de áudio via `rubato`: converte PCM f32 mono em qualquer taxa de
//! amostragem para PCM i16 mono na taxa alvo. Usado pela captura (mic →
//! 16kHz) e, futuramente, pelo playback (24kHz do modelo → saída local).
//!
//! `capture.rs` já chama esta função; o módulo inteiro só não é alcançável
//! a partir de `main` ainda porque `app.rs` não abriu a sessão Live (entrega
//! separada).
#![allow(dead_code)]

use rubato::audioadapter_buffers::owned::InterleavedOwned;
use rubato::{Async, FixedAsync, PolynomialDegree, Resampler};

/// Tamanho de chunk de entrada usado internamente pelo resampler quando as
/// taxas diferem. Apenas um alvo de eficiência; não afeta a corretude.
const CHUNK_SIZE_HINT: usize = 1024;

/// Converte `input` (mono, f32, taxa `from_hz`) para PCM i16 mono na taxa
/// `to_hz`. Retorna vazio se `input` estiver vazio ou alguma taxa for zero.
///
/// Cada chamada cria e descarta seu próprio resampler (sem estado entre
/// chamadas), como pede a interface da spec. Isso é apropriado para o uso
/// atual — resample por chunk de captura — mas introduz o delay de partida
/// do resampler a cada chamada; não é adequado para resample contínuo de um
/// stream longo sem reagrupar os chunks primeiro.
pub fn to_i16(input: &[f32], from_hz: u32, to_hz: u32) -> Vec<i16> {
    if input.is_empty() || from_hz == 0 || to_hz == 0 {
        return Vec::new();
    }

    if from_hz == to_hz {
        return input.iter().map(|&s| f32_to_i16(s)).collect();
    }

    let ratio = to_hz as f64 / from_hz as f64;
    let chunk_size = CHUNK_SIZE_HINT.min(input.len());

    let mut resampler = match Async::<f32>::new_poly(
        ratio,
        1.0,
        PolynomialDegree::Septic,
        chunk_size,
        1,
        FixedAsync::Input,
    ) {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(%err, "não foi possível criar o resampler");
            return Vec::new();
        }
    };

    let buffer_in = match InterleavedOwned::new_from(input.to_vec(), 1, input.len()) {
        Ok(b) => b,
        Err(err) => {
            tracing::warn!(%err, "buffer de entrada inválido para resample");
            return Vec::new();
        }
    };

    let output = match resampler.process_all(&buffer_in, input.len(), None) {
        Ok(o) => o,
        Err(err) => {
            tracing::warn!(%err, "falha ao reamostrar áudio");
            return Vec::new();
        }
    };

    output.take_data().into_iter().map(f32_to_i16).collect()
}

/// Converte uma amostra f32 (faixa esperada `-1.0..=1.0`) para i16, saturando
/// fora da faixa e tratando NaN como silêncio.
fn f32_to_i16(sample: f32) -> i16 {
    if sample.is_nan() {
        return 0;
    }
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(len: usize, freq_hz: f32, sample_rate: f32) -> Vec<f32> {
        (0..len)
            .map(|i| (2.0 * std::f32::consts::PI * freq_hz * i as f32 / sample_rate).sin())
            .collect()
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(to_i16(&[], 44_100, 16_000).is_empty());
    }

    #[test]
    fn zero_rate_returns_empty() {
        assert!(to_i16(&[0.1, 0.2], 0, 16_000).is_empty());
        assert!(to_i16(&[0.1, 0.2], 44_100, 0).is_empty());
    }

    #[test]
    fn same_rate_is_passthrough() {
        let input = [0.5_f32, -0.5, 0.0, 1.0, -1.0];
        let out = to_i16(&input, 16_000, 16_000);
        assert_eq!(out.len(), input.len());
        assert_eq!(out[2], 0);
        assert_eq!(out[3], i16::MAX);
        assert_eq!(out[4], -i16::MAX);
    }

    #[test]
    fn resample_44100_to_16000_has_expected_length_and_no_nan() {
        let input = sine(44_100, 440.0, 44_100.0);
        let out = to_i16(&input, 44_100, 16_000);

        let expected = 16_000;
        let tolerance = expected / 20; // 5%
        assert!(
            out.len().abs_diff(expected) <= tolerance,
            "len={} expected~={}",
            out.len(),
            expected
        );
        assert!(out.iter().all(|s| !(*s as f32).is_nan()));
        assert!(out.iter().any(|&s| s != 0));
    }

    #[test]
    fn resample_48000_to_16000_has_expected_length_and_no_nan() {
        let input = sine(48_000, 440.0, 48_000.0);
        let out = to_i16(&input, 48_000, 16_000);

        let expected = 16_000;
        let tolerance = expected / 20; // 5%
        assert!(
            out.len().abs_diff(expected) <= tolerance,
            "len={} expected~={}",
            out.len(),
            expected
        );
        assert!(out.iter().all(|s| !(*s as f32).is_nan()));
        assert!(out.iter().any(|&s| s != 0));
    }

    #[test]
    fn f32_to_i16_clamps_out_of_range_and_handles_nan() {
        assert_eq!(f32_to_i16(2.0), i16::MAX);
        assert_eq!(f32_to_i16(-2.0), -i16::MAX);
        assert_eq!(f32_to_i16(f32::NAN), 0);
    }
}
