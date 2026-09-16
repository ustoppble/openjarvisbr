//! Captura, resample e playback de áudio via `cpal`.
//!
//! Vazio neste scaffold; implementado nos próximos cards:
//! `capture.rs` (mic → PCM i16 16kHz mono), `playback.rs` (fila de PCM i16
//! 24kHz → saída, com descarte em interrupção) e `resample.rs` (rubato,
//! f32 qualquer taxa → i16 taxa alvo).

pub mod playback;
