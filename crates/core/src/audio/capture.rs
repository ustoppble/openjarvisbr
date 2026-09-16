//! Captura de áudio do microfone via `cpal`: converte cada callback do host
//! para PCM i16 mono na taxa alvo da Live API e emite chunks de 20ms no
//! canal `mpsc` consumido pela sessão.
//!
//! Ainda não é chamado por `app.rs` (isso é entrega separada, quando o
//! loop principal da sessão Live for implementado) — por ora só o
//! `examples/capture_meter.rs` exercita este módulo.
#![allow(dead_code)]

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, SampleFormat, StreamConfig};

use super::resample;

/// Taxa de amostragem exigida pela Live API para áudio de entrada.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;
const CHUNK_MS: u32 = 20;
/// Frames por chunk enviado no canal — 20ms a `TARGET_SAMPLE_RATE`, mono.
pub const CHUNK_FRAMES: usize = (TARGET_SAMPLE_RATE * CHUNK_MS / 1000) as usize;

/// Mantém o stream de captura vivo. Descartar o handle (`Drop`) para o cpal
/// para a captura; chame [`CaptureHandle::stop`] para parar explicitamente
/// antes disso.
pub struct CaptureHandle {
    stream: cpal::Stream,
}

impl CaptureHandle {
    /// Pausa a captura. O stream é encerrado de fato quando o handle é
    /// descartado.
    pub fn stop(&self) {
        if let Err(err) = self.stream.pause() {
            tracing::warn!(%err, "falha ao pausar a captura");
        }
    }
}

/// Erros de abertura/configuração do dispositivo de entrada. O chamador é
/// responsável por decidir a saída do processo (a spec pede código 3 quando
/// não há microfone, listando os dispositivos disponíveis via
/// [`list_input_devices`]).
#[derive(Debug)]
pub enum CaptureError {
    NoInputDevice,
    DeviceNotFound(String),
    Config(cpal::Error),
    UnsupportedSampleFormat(SampleFormat),
    BuildStream(cpal::Error),
    Play(cpal::Error),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::NoInputDevice => {
                write!(f, "nenhum dispositivo de entrada de áudio encontrado")
            }
            CaptureError::DeviceNotFound(name) => {
                write!(f, "dispositivo de entrada '{name}' não encontrado")
            }
            CaptureError::Config(err) => {
                write!(f, "não foi possível ler a configuração do microfone: {err}")
            }
            CaptureError::UnsupportedSampleFormat(fmt) => {
                write!(f, "formato de amostra não suportado: {fmt:?}")
            }
            CaptureError::BuildStream(err) => {
                write!(f, "não foi possível abrir o stream de captura: {err}")
            }
            CaptureError::Play(err) => write!(f, "não foi possível iniciar a captura: {err}"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Lista os nomes dos dispositivos de entrada disponíveis no host padrão.
/// Usado para o diagnóstico exibido quando não há microfone disponível.
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.map(|d| d.to_string()).collect())
        .unwrap_or_default()
}

fn find_device(host: &Host, device_name: Option<&str>) -> Result<Device, CaptureError> {
    match device_name {
        Some(name) => host
            .input_devices()
            .map_err(CaptureError::Config)?
            .find(|d| d.to_string() == name)
            .ok_or_else(|| CaptureError::DeviceNotFound(name.to_string())),
        None => host
            .default_input_device()
            .ok_or(CaptureError::NoInputDevice),
    }
}

/// Abre o dispositivo de entrada (padrão, ou por nome) e começa a captura,
/// convertendo cada callback do host para PCM i16 mono a
/// [`TARGET_SAMPLE_RATE`] e emitindo chunks de 20ms (`CHUNK_FRAMES` frames)
/// em `tx`. O stream roda em segundo plano até o [`CaptureHandle`] retornado
/// ser descartado ou pausado.
pub fn start(
    device_name: Option<&str>,
    tx: Sender<Vec<i16>>,
) -> Result<CaptureHandle, CaptureError> {
    let host = cpal::default_host();
    let device = find_device(&host, device_name)?;

    let supported = device
        .default_input_config()
        .map_err(CaptureError::Config)?;
    let sample_format = supported.sample_format();
    let input_rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let config: StreamConfig = supported.into();

    let pending = Arc::new(Mutex::new(Vec::<i16>::new()));

    let stream = match sample_format {
        SampleFormat::F32 => {
            let pending = Arc::clone(&pending);
            let tx = tx.clone();
            device.build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    handle_callback(data, channels, input_rate, &pending, &tx);
                },
                capture_error_callback,
                None,
            )
        }
        SampleFormat::I16 => {
            let pending = Arc::clone(&pending);
            let tx = tx.clone();
            device.build_input_stream(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let floats: Vec<f32> = data
                        .iter()
                        .map(|&s| s as f32 / i16::MAX as f32)
                        .collect();
                    handle_callback(&floats, channels, input_rate, &pending, &tx);
                },
                capture_error_callback,
                None,
            )
        }
        other => return Err(CaptureError::UnsupportedSampleFormat(other)),
    }
    .map_err(CaptureError::BuildStream)?;

    stream.play().map_err(CaptureError::Play)?;

    Ok(CaptureHandle { stream })
}

fn capture_error_callback(err: cpal::Error) {
    tracing::warn!(%err, "erro no stream de captura");
}

/// Processa um callback do host: reduz a mono, reamostra para a taxa alvo e
/// envia chunks completos de 20ms pelo canal, mantendo a sobra entre
/// chamadas em `pending`.
fn handle_callback(
    data: &[f32],
    channels: usize,
    input_rate: u32,
    pending: &Arc<Mutex<Vec<i16>>>,
    tx: &Sender<Vec<i16>>,
) {
    let mono = downmix_to_mono(data, channels);
    let resampled = resample::to_i16(&mono, input_rate, TARGET_SAMPLE_RATE);

    let mut pending = match pending.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    pending.extend_from_slice(&resampled);

    while pending.len() >= CHUNK_FRAMES {
        let chunk: Vec<i16> = pending.drain(..CHUNK_FRAMES).collect();
        if tx.send(chunk).is_err() {
            // O receptor fechou o canal (sessão encerrada); nada mais a fazer
            // até o CaptureHandle ser descartado pelo chamador.
            break;
        }
    }
}

fn downmix_to_mono(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_frames_is_20ms_at_target_rate() {
        assert_eq!(CHUNK_FRAMES, 320);
    }

    #[test]
    fn downmix_stereo_averages_channels() {
        let data = [1.0_f32, -1.0, 0.5, 0.5];
        let mono = downmix_to_mono(&data, 2);
        assert_eq!(mono, vec![0.0, 0.5]);
    }

    #[test]
    fn downmix_mono_is_noop() {
        let data = [0.1_f32, 0.2, 0.3];
        assert_eq!(downmix_to_mono(&data, 1), data.to_vec());
    }

    #[test]
    fn handle_callback_emits_full_chunks_and_keeps_remainder() {
        let pending = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel();

        // 500ms de silêncio a 16kHz já na taxa alvo: 8000 frames == 25 chunks
        // de 320 frames, sem sobra.
        let data = vec![0.0_f32; 8_000];
        handle_callback(&data, 1, TARGET_SAMPLE_RATE, &pending, &tx);

        let mut received = 0;
        while let Ok(chunk) = rx.try_recv() {
            assert_eq!(chunk.len(), CHUNK_FRAMES);
            received += 1;
        }
        assert_eq!(received, 25);
        assert!(pending.lock().unwrap().is_empty());
    }
}
