//! Captura, resample e playback de áudio via `cpal`.
//!
//! `capture.rs` (mic → PCM i16 16kHz mono), `resample.rs` (rubato, f32
//! qualquer taxa → i16 taxa alvo) e `playback.rs` (fila de PCM i16 24kHz →
//! saída, com descarte em interrupção). `aec.rs` cancela o eco do que está
//! tocando no mic quando o barge-in está ligado.

pub mod aec;
pub mod capture;
pub mod resample;

pub mod fx;
pub mod playback;
