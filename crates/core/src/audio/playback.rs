//! Fila de reprodução de PCM i16 24kHz mono via `cpal`.
//!
//! `Player::push` enfileira amostras; `flush` descarta tudo que ainda não
//! tocou (usado na interrupção). Quando o dispositivo de saída não aceita
//! 24kHz nativamente, as amostras são resampleadas com `rubato` antes de
//! entrar na fila.

// `Player` ainda não é chamado por `app.rs`/`session.rs` (cards futuros); sem
// isso o clippy marcaria toda a API como código morto.
#![allow(dead_code)]

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, PolynomialDegree, Resampler};

/// Taxa de amostragem em que o app produz o áudio do modelo.
const SOURCE_RATE: u32 = 24_000;
const SOURCE_CHANNELS: usize = 1;
const RESAMPLE_CHUNK_FRAMES: usize = 480;
/// Capacidade do ring buffer em segundos de áudio do dispositivo.
const RING_SECONDS: usize = 60;

#[derive(Debug)]
pub enum PlaybackError {
    NoOutputDevice,
    DeviceNotFound(String),
    Cpal(cpal::Error),
    Resampler(rubato::ResamplerConstructionError),
}

impl fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaybackError::NoOutputDevice => write!(f, "nenhum dispositivo de saída de áudio"),
            PlaybackError::DeviceNotFound(name) => {
                write!(f, "dispositivo de saída '{name}' não encontrado")
            }
            PlaybackError::Cpal(err) => write!(f, "erro de áudio: {err}"),
            PlaybackError::Resampler(err) => write!(f, "erro ao montar o resampler: {err}"),
        }
    }
}

impl std::error::Error for PlaybackError {}

impl From<cpal::Error> for PlaybackError {
    fn from(err: cpal::Error) -> Self {
        PlaybackError::Cpal(err)
    }
}

impl From<rubato::ResamplerConstructionError> for PlaybackError {
    fn from(err: rubato::ResamplerConstructionError) -> Self {
        PlaybackError::Resampler(err)
    }
}

/// Estado compartilhado entre produtor (loop da app) e o callback de áudio.
///
/// O callback roda numa thread de tempo real do CoreAudio/WASAPI e NUNCA
/// pode bloquear: por isso as amostras viajam por um ring buffer SPSC
/// lock-free (`ringbuf`) e os sinais de controle são atômicos. Um `Mutex`
/// aqui, disputado com o `push`, virava silêncio no dispositivo a cada
/// espera — palavras engolidas que nenhum log de fila enxerga.
struct Shared {
    /// Fim de turno: toca o resto mesmo abaixo do prebuffer.
    drain: AtomicBool,
    /// Incrementado no `flush`; o callback descarta tudo ao ver mudar.
    generation: AtomicU32,
    /// Amostras (já expandidas por canal) antes de começar a tocar.
    prebuffer: usize,
}

/// Lado do consumidor, vivo só dentro do callback.
struct Sink {
    cons: HeapCons<i16>,
    shared: Arc<Shared>,
    primed: bool,
    seen_generation: u32,
}

impl Sink {
    fn fill<T: SizedSample + FromSample<i16>>(&mut self, data: &mut [T]) {
        let generation = self.shared.generation.load(Ordering::Acquire);
        if generation != self.seen_generation {
            let pending = self.cons.occupied_len();
            self.cons.skip(pending);
            self.primed = false;
            self.seen_generation = generation;
        }
        for slot in data.iter_mut() {
            *slot = T::from_sample(self.next_sample());
        }
    }

    /// Próxima amostra a tocar, ou silêncio enquanto o buffer enche.
    fn next_sample(&mut self) -> i16 {
        if !self.primed {
            let available = self.cons.occupied_len();
            let drain = self.shared.drain.load(Ordering::Acquire);
            if available >= self.shared.prebuffer || (drain && available > 0) {
                self.primed = true;
            } else {
                return 0;
            }
        }
        match self.cons.try_pop() {
            Some(sample) => sample,
            None => {
                self.primed = false;
                self.shared.drain.store(false, Ordering::Release);
                0
            }
        }
    }
}

/// Estado do resampler quando o dispositivo não aceita 24kHz nativamente.
struct Resampling {
    engine: Async<f32>,
    pending: Vec<f32>,
}

impl Resampling {
    fn new(device_rate: u32) -> Result<Self, rubato::ResamplerConstructionError> {
        let ratio = device_rate as f64 / SOURCE_RATE as f64;
        let engine = Async::<f32>::new_poly(
            ratio,
            1.0,
            PolynomialDegree::Cubic,
            RESAMPLE_CHUNK_FRAMES,
            SOURCE_CHANNELS,
            FixedAsync::Input,
        )?;
        Ok(Self {
            engine,
            pending: Vec::new(),
        })
    }

    /// Alimenta amostras 24kHz e devolve o que já pôde ser convertido
    /// para a taxa do dispositivo (mono, f32 na escala i16).
    fn feed(&mut self, samples: &[i16]) -> Vec<f32> {
        self.pending.extend(samples.iter().map(|&s| s as f32));
        let mut out = Vec::new();
        loop {
            let needed = self.engine.input_frames_next();
            if self.pending.len() < needed {
                break;
            }
            let input =
                InterleavedSlice::new(&self.pending[..needed], SOURCE_CHANNELS, needed).unwrap();
            let out_frames = self.engine.output_frames_next();
            let mut out_buf = vec![0f32; out_frames];
            let mut output =
                InterleavedSlice::new_mut(&mut out_buf, SOURCE_CHANNELS, out_frames).unwrap();
            let (consumed, produced) = self
                .engine
                .process_into_buffer(&input, &mut output, None)
                .expect("resample de playback");
            self.pending.drain(..consumed);
            out.extend_from_slice(&out_buf[..produced]);
        }
        out
    }
}

/// Reprodutor de PCM i16 24kHz mono. `push` enfileira, `flush` descarta a
/// fila (interrupção).
pub struct Player {
    prod: Mutex<HeapProd<i16>>,
    shared: Arc<Shared>,
    resampling: Option<Mutex<Resampling>>,
    device_channels: usize,
    _stream: cpal::Stream,
}

/// Lista os nomes dos dispositivos de saída disponíveis no host padrão.
/// Espelha [`crate::audio::capture::list_input_devices`] para a janela de
/// configurações.
pub fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.map(|d| d.to_string()).collect())
        .unwrap_or_default()
}

impl Player {
    /// Abre o dispositivo de saída padrão, ou o indicado por `device_name`,
    /// e prepara o reprodutor. Resampleia se o dispositivo não aceitar
    /// 24kHz.
    pub fn new(device_name: Option<&str>) -> Result<Player, PlaybackError> {
        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => host
                .output_devices()?
                .find(|d| d.to_string() == name)
                .ok_or_else(|| PlaybackError::DeviceNotFound(name.to_string()))?,
            None => host
                .default_output_device()
                .ok_or(PlaybackError::NoOutputDevice)?,
        };

        // Sempre a configuração nativa do dispositivo (taxa, canais, formato).
        // Forçar 24kHz no CoreAudio abria a saída numa combinação não nativa
        // (ex.: 4 canais) e o resultado soava picado; o resample por software
        // com rubato é previsível em qualquer máquina.
        let config = device.default_output_config()?;
        let device_channels = config.channels() as usize;
        let device_rate = config.sample_rate();
        let sample_format = config.sample_format();
        let needs_resample = device_rate != SOURCE_RATE;
        tracing::info!(
            dispositivo = %device.description().map(|d| d.to_string()).unwrap_or_default(),
            taxa = device_rate,
            canais = device_channels,
            formato = ?sample_format,
            resample = needs_resample,
            "playback aberto na configuração nativa"
        );
        let stream_config: StreamConfig = config.into();

        // ~200ms de áudio antes de começar a tocar cada resposta.
        let prebuffer = device_rate as usize * device_channels / 5;
        let shared = Arc::new(Shared {
            drain: AtomicBool::new(false),
            generation: AtomicU32::new(0),
            prebuffer,
        });
        let (prod, cons) =
            HeapRb::<i16>::new(device_rate as usize * device_channels * RING_SECONDS).split();
        let sink = Sink {
            cons,
            shared: shared.clone(),
            primed: false,
            seen_generation: 0,
        };
        let stream = build_stream(&device, &stream_config, sample_format, sink)?;
        stream.play()?;

        let resampling = if needs_resample {
            Some(Mutex::new(Resampling::new(device_rate)?))
        } else {
            None
        };

        Ok(Player {
            prod: Mutex::new(prod),
            shared,
            resampling,
            device_channels,
            _stream: stream,
        })
    }

    /// Enfileira amostras PCM i16 24kHz mono para reprodução.
    pub fn push(&self, samples: &[i16]) {
        match &self.resampling {
            Some(resampling) => self.push_resampled(resampling, samples),
            None => self.enqueue_i16(samples),
        }
    }

    fn push_resampled(&self, resampling: &Mutex<Resampling>, samples: &[i16]) {
        let mut guard = resampling.lock().unwrap_or_else(|e| e.into_inner());
        let out = guard.feed(samples);
        drop(guard);
        self.enqueue_f32(&out);
    }

    fn enqueue_i16(&self, samples: &[i16]) {
        let expanded: Vec<i16> = samples
            .iter()
            .flat_map(|&s| std::iter::repeat_n(s, self.device_channels))
            .collect();
        self.enqueue_expanded(&expanded);
    }

    fn enqueue_f32(&self, samples: &[f32]) {
        let expanded: Vec<i16> = samples
            .iter()
            .map(|&s| s.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16)
            .flat_map(|s| std::iter::repeat_n(s, self.device_channels))
            .collect();
        self.enqueue_expanded(&expanded);
    }

    fn enqueue_expanded(&self, expanded: &[i16]) {
        let mut prod = self.prod.lock().unwrap_or_else(|e| e.into_inner());
        let written = prod.push_slice(expanded);
        if written < expanded.len() {
            tracing::warn!(
                perdidas = expanded.len() - written,
                "ring buffer de playback cheio; amostras descartadas"
            );
        }
    }

    /// `true` enquanto ainda há amostras na fila esperando pra tocar.
    pub fn is_playing(&self) -> bool {
        self.queued() > 0
    }

    /// Amostras (já expandidas por canal) ainda na fila.
    pub fn queued(&self) -> usize {
        self.prod
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .occupied_len()
    }

    /// O modelo terminou o turno: toca o que restou na fila mesmo que seja
    /// menor que o prebuffer.
    pub fn end_of_turn(&self) {
        self.shared.drain.store(true, Ordering::Release);
    }

    /// Descarta toda amostra ainda não tocada (interrupção). O descarte em
    /// si acontece no callback, ao notar a nova geração.
    pub fn flush(&self) {
        self.shared.drain.store(false, Ordering::Release);
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(resampling) = &self.resampling {
            resampling
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending
                .clear();
        }
    }
}

fn build_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    sink: Sink,
) -> Result<cpal::Stream, PlaybackError> {
    let stream = match sample_format {
        SampleFormat::I16 => build_typed_stream::<i16>(device, config, sink)?,
        SampleFormat::U16 => build_typed_stream::<u16>(device, config, sink)?,
        SampleFormat::F32 => build_typed_stream::<f32>(device, config, sink)?,
        other => {
            tracing::warn!(formato = ?other, "formato de amostra não testado, tentando f32");
            build_typed_stream::<f32>(device, config, sink)?
        }
    };
    Ok(stream)
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut sink: Sink,
) -> Result<cpal::Stream, PlaybackError>
where
    T: SizedSample + FromSample<i16> + Send + 'static,
{
    let stream = device.build_output_stream(
        *config,
        move |data: &mut [T], _info: &cpal::OutputCallbackInfo| sink.fill(data),
        |err| tracing::error!(error = %err, "erro no stream de playback"),
        None,
    )?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink(prebuffer: usize, cap: usize) -> (HeapProd<i16>, Sink, Arc<Shared>) {
        let shared = Arc::new(Shared {
            drain: AtomicBool::new(false),
            generation: AtomicU32::new(0),
            prebuffer,
        });
        let (prod, cons) = HeapRb::<i16>::new(cap).split();
        let sink = Sink {
            cons,
            shared: shared.clone(),
            primed: false,
            seen_generation: 0,
        };
        (prod, sink, shared)
    }

    #[test]
    fn waits_for_prebuffer_then_plays_and_underrun_reprimes() {
        let (mut prod, mut sink, _shared) = sink(4, 64);
        prod.push_slice(&[1, 2, 3]);
        let mut out = [0i16; 3];
        sink.fill(&mut out);
        assert_eq!(out, [0, 0, 0], "abaixo do prebuffer toca silêncio");
        prod.push_slice(&[4]);
        let mut out = [0i16; 6];
        sink.fill(&mut out);
        assert_eq!(out, [1, 2, 3, 4, 0, 0]);
        assert!(!sink.primed, "esvaziou: volta a esperar o prebuffer");
    }

    #[test]
    fn drain_plays_tail_below_prebuffer() {
        let (mut prod, mut sink, shared) = sink(100, 64);
        prod.push_slice(&[7, 8]);
        shared.drain.store(true, Ordering::Release);
        let mut out = [0i16; 3];
        sink.fill(&mut out);
        assert_eq!(out, [7, 8, 0]);
        assert!(!shared.drain.load(Ordering::Acquire), "drain consumido");
    }

    #[test]
    fn resample_24k_to_48k_keeps_a_sine_continuous() {
        // Seno de 440Hz a 24kHz, entregue em chunks com os tamanhos reais
        // que a Live API manda. Na saída a 48kHz não pode haver salto maior
        // que o de um seno contínuo (amplitude 10000 → ~576 por amostra).
        let mut rs = Resampling::new(48_000).unwrap();
        let sizes = [3840usize, 5760, 4800, 3840, 7680, 1920, 5760, 4800];
        let mut t = 0usize;
        let mut out = Vec::new();
        for &n in &sizes {
            let chunk: Vec<i16> = (0..n)
                .map(|i| {
                    let x = (t + i) as f32 / 24_000.0;
                    (10_000.0 * (2.0 * std::f32::consts::PI * 440.0 * x).sin()) as i16
                })
                .collect();
            t += n;
            out.extend(rs.feed(&chunk));
        }
        let total_in: usize = sizes.iter().sum();
        assert!(out.len() > total_in * 2 - 4000, "produziu {} de ~{}", out.len(), total_in * 2);
        let max_jump = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0f32, f32::max);
        assert!(max_jump < 900.0, "descontinuidade no resample: salto {max_jump}");
    }

    #[test]
    fn flush_generation_discards_pending() {
        let (mut prod, mut sink, shared) = sink(1, 64);
        prod.push_slice(&[1, 2, 3, 4]);
        shared.generation.fetch_add(1, Ordering::AcqRel);
        prod.push_slice(&[9]);
        // A geração nova descarta tudo que havia antes do fill, inclusive o 9
        // que chegou antes do callback rodar — comportamento aceito: o flush
        // é uma interrupção e o próximo turno recomeça do zero.
        let mut out = [0i16; 2];
        sink.fill(&mut out);
        assert_eq!(out, [0, 0]);
    }
}
