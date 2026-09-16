//! Fila de reprodução de PCM i16 24kHz mono via `cpal`.
//!
//! `Player::push` enfileira amostras; `flush` descarta tudo que ainda não
//! tocou (usado na interrupção). Quando o dispositivo de saída não aceita
//! 24kHz nativamente, as amostras são resampleadas com `rubato` antes de
//! entrar na fila.

// `Player` ainda não é chamado por `app.rs`/`session.rs` (cards futuros); sem
// isso o clippy marcaria toda a API como código morto.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, PolynomialDegree, Resampler};

/// Taxa de amostragem em que o app produz o áudio do modelo.
const SOURCE_RATE: u32 = 24_000;
const SOURCE_CHANNELS: usize = 1;
const RESAMPLE_CHUNK_FRAMES: usize = 480;

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

/// Fila compartilhada com o callback de áudio.
///
/// `primed` implementa um jitter buffer: o callback só começa a drenar
/// quando há pelo menos `prebuffer` amostras acumuladas (ou quando `drain`
/// foi pedido no fim do turno), e volta a esperar quando a fila esvazia.
/// Sem isso, cada vão entre pacotes de rede virava um estalo de silêncio
/// no meio da frase.
struct Queue {
    samples: VecDeque<i16>,
    primed: bool,
    drain: bool,
    prebuffer: usize,
}

impl Queue {
    fn new(prebuffer: usize) -> Self {
        Self {
            samples: VecDeque::new(),
            primed: false,
            drain: false,
            prebuffer,
        }
    }

    fn clear(&mut self) {
        self.samples.clear();
        self.primed = false;
        self.drain = false;
    }

    /// Próxima amostra a tocar, ou silêncio enquanto o buffer enche.
    fn next_sample(&mut self) -> i16 {
        if !self.primed {
            if self.samples.len() >= self.prebuffer || (self.drain && !self.samples.is_empty()) {
                self.primed = true;
            } else {
                return 0;
            }
        }
        match self.samples.pop_front() {
            Some(sample) => sample,
            None => {
                self.primed = false;
                self.drain = false;
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

/// Reprodutor de PCM i16 24kHz mono. `push` enfileira, `flush` descarta a
/// fila (interrupção).
pub struct Player {
    queue: Arc<Mutex<Queue>>,
    resampling: Option<Mutex<Resampling>>,
    device_channels: usize,
    _stream: cpal::Stream,
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

        let target_rate: cpal::SampleRate = SOURCE_RATE;
        let supported = device
            .supported_output_configs()?
            .find(|range| range.contains_rate(target_rate));

        let (config, needs_resample) = match supported {
            Some(range) => (range.with_sample_rate(target_rate), false),
            None => (device.default_output_config()?, true),
        };

        let device_channels = config.channels() as usize;
        let device_rate = config.sample_rate();
        let sample_format = config.sample_format();
        let stream_config: StreamConfig = config.into();

        // ~200ms de áudio antes de começar a tocar cada resposta.
        let prebuffer = device_rate as usize * device_channels / 5;
        let queue: Arc<Mutex<Queue>> = Arc::new(Mutex::new(Queue::new(prebuffer)));
        let stream = build_stream(&device, &stream_config, sample_format, queue.clone())?;
        stream.play()?;

        let resampling = if needs_resample {
            let ratio = device_rate as f64 / SOURCE_RATE as f64;
            let engine = Async::<f32>::new_poly(
                ratio,
                1.0,
                PolynomialDegree::Cubic,
                RESAMPLE_CHUNK_FRAMES,
                SOURCE_CHANNELS,
                FixedAsync::Input,
            )?;
            Some(Mutex::new(Resampling {
                engine,
                pending: Vec::new(),
            }))
        } else {
            None
        };

        Ok(Player {
            queue,
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
        let state = &mut *guard;
        state.pending.extend(samples.iter().map(|&s| s as f32));

        loop {
            let needed = state.engine.input_frames_next();
            if state.pending.len() < needed {
                break;
            }

            let input =
                InterleavedSlice::new(&state.pending[..needed], SOURCE_CHANNELS, needed).unwrap();
            let out_frames = state.engine.output_frames_next();
            let mut out_buf = vec![0f32; out_frames];
            let mut output =
                InterleavedSlice::new_mut(&mut out_buf, SOURCE_CHANNELS, out_frames).unwrap();

            let (consumed, produced) = state
                .engine
                .process_into_buffer(&input, &mut output, None)
                .expect("resample de playback");

            state.pending.drain(..consumed);
            self.enqueue_f32(&out_buf[..produced]);
        }
    }

    fn enqueue_i16(&self, samples: &[i16]) {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        for &sample in samples {
            for _ in 0..self.device_channels {
                queue.samples.push_back(sample);
            }
        }
    }

    fn enqueue_f32(&self, samples: &[f32]) {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        for &sample in samples {
            let clamped = sample.round().clamp(i16::MIN as f32, i16::MAX as f32);
            let value = clamped as i16;
            for _ in 0..self.device_channels {
                queue.samples.push_back(value);
            }
        }
    }

    /// Descarta toda amostra ainda não tocada (interrupção).
    /// `true` enquanto ainda há amostras na fila esperando pra tocar.
    pub fn is_playing(&self) -> bool {
        !self
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .samples
            .is_empty()
    }

    /// O modelo terminou o turno: toca o que restou na fila mesmo que seja
    /// menor que o prebuffer.
    pub fn end_of_turn(&self) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain = true;
    }

    pub fn flush(&self) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
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
    queue: Arc<Mutex<Queue>>,
) -> Result<cpal::Stream, PlaybackError> {
    let stream = match sample_format {
        SampleFormat::I16 => build_typed_stream::<i16>(device, config, queue)?,
        SampleFormat::U16 => build_typed_stream::<u16>(device, config, queue)?,
        SampleFormat::F32 => build_typed_stream::<f32>(device, config, queue)?,
        other => {
            tracing::warn!(formato = ?other, "formato de amostra não testado, tentando f32");
            build_typed_stream::<f32>(device, config, queue)?
        }
    };
    Ok(stream)
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    queue: Arc<Mutex<Queue>>,
) -> Result<cpal::Stream, PlaybackError>
where
    T: SizedSample + FromSample<i16> + Send + 'static,
{
    let stream = device.build_output_stream(
        *config,
        move |data: &mut [T], _info: &cpal::OutputCallbackInfo| {
            let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
            for sample in data.iter_mut() {
                *sample = T::from_sample(q.next_sample());
            }
        },
        |err| tracing::error!(error = %err, "erro no stream de playback"),
        None,
    )?;
    Ok(stream)
}
