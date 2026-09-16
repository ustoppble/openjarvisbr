//! Cancelamento de eco acústico (AEC) para o modo caixa de som com barge-in.
//!
//! Sem fone, o mic capta a voz do próprio Jarvis e a Live API a trata como
//! fala do usuário: ele se interrompe sozinho. Aqui o áudio do mic (near-end,
//! 16kHz) passa por um cancelador que usa como referência (far-end) o que o
//! dispositivo de saída está tocando de fato.
//!
//! - **Referência**: o callback do `Player` copia cada amostra tocada para um
//!   ring buffer lock-free ([`reference_channel`]). Como a cópia sai do
//!   callback, e não do `push`, a fila de playback (segundos adiantados, a
//!   Live API manda mais rápido que o tempo real) não entra no atraso: sobra
//!   só a latência de saída + entrada do dispositivo, que o estimador de
//!   atraso do AEC3 acompanha.
//! - **Cancelador**: no macOS, AEC3 do WebRTC (`webrtc-audio-processing`, com
//!   high-pass e noise suppression). Nas outras plataformas o build C++ da
//!   crate exige meson/clang/objcopy que o CI Windows não tem; lá roda um
//!   NLMS de 512 taps com compensação de atraso fixa.
//! - **Gate residual**: enquanto a referência tem som (mais uma margem pela
//!   latência), um chunk cujo RMS após o AEC fique abaixo de 2× o ruído de
//!   fundo não é enviado.

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, PolynomialDegree, Resampler};

/// Taxa em que o AEC processa (a mesma do mic enviado à Live API).
pub const AEC_RATE: u32 = 16_000;
/// Amostras por frame de 10ms a [`AEC_RATE`].
pub const FRAME: usize = (AEC_RATE / 100) as usize;

/// Segundos de referência que cabem no ring buffer antes de descartar.
const REFERENCE_SECONDS: usize = 2;
const RESAMPLE_CHUNK: usize = 480;
/// RMS (0..1) acima do qual a referência conta como "o player está tocando".
const FAR_ACTIVE_RMS: f32 = 0.002;
/// Por quanto tempo depois do último frame audível da referência o gate
/// continua valendo: latência de saída + entrada (~150ms) com margem de 100ms.
const FAR_HOLD_MS: usize = 250;
/// O chunk só passa, com o player tocando, se o RMS após o AEC for maior que
/// este múltiplo do ruído de fundo.
const GATE_RATIO: f32 = 2.0;
/// Limites do ruído de fundo estimado (RMS 0..1): o piso evita que um mic
/// digitalmente mudo zere o gate; o teto evita que fala longa sem playback
/// suba o fundo até bloquear o barge-in.
const FLOOR_MIN: f32 = 0.001;
const FLOOR_MAX: f32 = 0.02;

/// Lado produtor da referência, vivo no callback de saída.
pub struct ReferenceTap {
    prod: HeapProd<i16>,
}

impl ReferenceTap {
    /// Copia uma amostra tocada. Nunca bloqueia: com o ring cheio (motor
    /// parado), a amostra é descartada.
    #[inline]
    pub fn push(&mut self, sample: i16) {
        let _ = self.prod.try_push(sample);
    }
}

/// Lado consumidor da referência: amostras mono na taxa do dispositivo.
pub struct EchoReference {
    cons: HeapCons<i16>,
    resampler: Option<Async<f32>>,
    pending: Vec<f32>,
}

/// Cria o par tap/referência para um dispositivo de saída a `device_rate`.
pub fn reference_channel(device_rate: u32) -> (ReferenceTap, EchoReference) {
    let (prod, cons) = HeapRb::<i16>::new(device_rate as usize * REFERENCE_SECONDS).split();
    let resampler = (device_rate != AEC_RATE).then(|| {
        Async::<f32>::new_poly(
            AEC_RATE as f64 / device_rate as f64,
            1.0,
            PolynomialDegree::Cubic,
            RESAMPLE_CHUNK,
            1,
            FixedAsync::Input,
        )
        .expect("resampler da referência de eco")
    });
    (
        ReferenceTap { prod },
        EchoReference {
            cons,
            resampler,
            pending: Vec::new(),
        },
    )
}

impl EchoReference {
    /// Tudo que tocou desde a última chamada, a 16kHz, em -1..1.
    pub(crate) fn drain_into(&mut self, out: &mut Vec<f32>) {
        let available = self.cons.occupied_len();
        let Some(resampler) = self.resampler.as_mut() else {
            out.extend(self.cons.pop_iter().take(available).map(to_unit));
            return;
        };
        self.pending
            .extend(self.cons.pop_iter().take(available).map(to_unit));
        loop {
            let needed = resampler.input_frames_next();
            if self.pending.len() < needed {
                break;
            }
            let input = InterleavedSlice::new(&self.pending[..needed], 1, needed).unwrap();
            let frames = resampler.output_frames_next();
            let mut buf = vec![0f32; frames];
            let mut output = InterleavedSlice::new_mut(&mut buf, 1, frames).unwrap();
            let (consumed, produced) = resampler
                .process_into_buffer(&input, &mut output, None)
                .expect("resample da referência de eco");
            self.pending.drain(..consumed);
            out.extend_from_slice(&buf[..produced]);
        }
    }
}

fn to_unit(sample: i16) -> f32 {
    sample as f32 / 32768.0
}

fn to_i16(sample: f32) -> i16 {
    (sample * 32768.0)
        .round()
        .clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

/// Cancelador de eco em frames de 10ms a 16kHz.
trait Canceller {
    /// Frame que tocou (far-end).
    fn render(&mut self, frame: &[f32]);
    /// Frame do mic (near-end), limpo no lugar.
    fn capture(&mut self, frame: &mut [f32]);
}

#[cfg(target_os = "macos")]
mod webrtc {
    use webrtc_audio_processing::Processor;
    use webrtc_audio_processing_config::{
        Config, EchoCanceller, HighPassFilter, NoiseSuppression, NoiseSuppressionLevel,
    };

    pub struct WebRtc(Processor);

    impl WebRtc {
        pub fn new() -> Option<Self> {
            let processor = match Processor::new(super::AEC_RATE) {
                Ok(p) => p,
                Err(err) => {
                    tracing::warn!(erro = ?err, "AEC3 indisponível; usando NLMS");
                    return None;
                }
            };
            processor.set_config(Config {
                echo_canceller: Some(EchoCanceller::Full {
                    stream_delay_ms: None,
                }),
                high_pass_filter: Some(HighPassFilter::default()),
                noise_suppression: Some(NoiseSuppression {
                    level: NoiseSuppressionLevel::Moderate,
                    analyze_linear_aec_output: false,
                }),
                ..Default::default()
            });
            Some(WebRtc(processor))
        }
    }

    impl super::Canceller for WebRtc {
        fn render(&mut self, frame: &[f32]) {
            if let Err(err) = self.0.analyze_render_frame([frame]) {
                tracing::debug!(erro = ?err, "AEC3: frame de referência recusado");
            }
        }

        fn capture(&mut self, frame: &mut [f32]) {
            if let Err(err) = self.0.process_capture_frame([&mut *frame]) {
                tracing::debug!(erro = ?err, "AEC3: frame do mic recusado");
            }
        }
    }
}

/// Filtro adaptativo NLMS: estima o eco como convolução da referência e
/// subtrai do mic. A referência passa por um atraso fixo antes do filtro
/// para cobrir a latência típica do dispositivo; o filtro cobre o resto.
struct Nlms {
    weights: Vec<f32>,
    /// Janela circular das últimas `taps` amostras da referência atrasada.
    history: Vec<f32>,
    pos: usize,
    /// Linha de atraso da referência (FIFO).
    delay: std::collections::VecDeque<f32>,
    delay_len: usize,
}

const NLMS_TAPS: usize = 512;
const NLMS_DELAY_MS: usize = 20;
const NLMS_MU: f32 = 0.5;
const NLMS_EPS: f32 = 1e-6;

impl Nlms {
    fn new(taps: usize, delay_samples: usize) -> Self {
        Nlms {
            weights: vec![0.0; taps],
            history: vec![0.0; taps],
            pos: 0,
            delay: std::collections::VecDeque::with_capacity(delay_samples + FRAME * 8),
            delay_len: delay_samples,
        }
    }
}

impl Canceller for Nlms {
    fn render(&mut self, frame: &[f32]) {
        self.delay.extend(frame.iter().copied());
    }

    fn capture(&mut self, frame: &mut [f32]) {
        let taps = self.weights.len();
        for sample in frame.iter_mut() {
            // Uma amostra de referência por amostra do mic, respeitando o
            // atraso; sem referência suficiente, silêncio.
            let x = if self.delay.len() > self.delay_len {
                self.delay.pop_front().unwrap_or(0.0)
            } else {
                0.0
            };
            self.pos = (self.pos + 1) % taps;
            self.history[self.pos] = x;
            let mut estimate = 0.0;
            let mut energy = NLMS_EPS;
            for (k, w) in self.weights.iter().enumerate() {
                let h = self.history[(self.pos + taps - k) % taps];
                estimate += w * h;
                energy += h * h;
            }
            let error = *sample - estimate;
            let step = NLMS_MU * error / energy;
            for (k, w) in self.weights.iter_mut().enumerate() {
                *w += step * self.history[(self.pos + taps - k) % taps];
            }
            *sample = error;
        }
        // Referência acumulada além do necessário (mic parado): descarta o
        // excesso para o atraso não crescer.
        let max = self.delay_len + FRAME * 4;
        while self.delay.len() > max {
            self.delay.pop_front();
        }
    }
}

/// Gate por energia do resíduo enquanto o player toca.
struct ResidualGate {
    floor: f32,
    /// Amostras de mic restantes em que a referência ainda conta como ativa.
    far_hold: usize,
}

impl ResidualGate {
    fn new() -> Self {
        ResidualGate {
            floor: FLOOR_MIN,
            far_hold: 0,
        }
    }

    fn far_frame(&mut self, frame: &[f32]) {
        if rms(frame) > FAR_ACTIVE_RMS {
            self.far_hold = AEC_RATE as usize * FAR_HOLD_MS / 1000;
        }
    }

    fn far_active(&self) -> bool {
        self.far_hold > 0
    }

    /// `true` se o chunk (já limpo) deve ser enviado.
    fn pass(&mut self, cleaned: &[f32]) -> bool {
        let level = rms(cleaned);
        let active = self.far_active();
        self.far_hold = self.far_hold.saturating_sub(cleaned.len());
        if !active {
            // Fundo: desce rápido, sobe devagar (fala não vira fundo).
            let alpha = if level < self.floor { 0.2 } else { 0.005 };
            self.floor = (self.floor + alpha * (level - self.floor)).clamp(FLOOR_MIN, FLOOR_MAX);
            return true;
        }
        level >= GATE_RATIO * self.floor
    }
}

/// AEC + gate residual do mic, alimentado pela referência do player.
pub struct EchoCanceller {
    canceller: Box<dyn Canceller>,
    reference: Option<EchoReference>,
    far: Vec<f32>,
    near: Vec<f32>,
    gate: ResidualGate,
}

impl EchoCanceller {
    /// Cancelador que lê a referência do player.
    pub fn new(reference: EchoReference) -> Self {
        let mut aec = Self::detached();
        aec.reference = Some(reference);
        aec
    }

    fn detached() -> Self {
        EchoCanceller {
            canceller: default_canceller(),
            reference: None,
            far: Vec::new(),
            near: Vec::new(),
            gate: ResidualGate::new(),
        }
    }

    #[cfg(test)]
    fn with_canceller(canceller: Box<dyn Canceller>) -> Self {
        EchoCanceller {
            canceller,
            ..Self::detached()
        }
    }

    /// Nome do cancelador em uso, para log.
    pub fn backend() -> &'static str {
        if cfg!(target_os = "macos") {
            "webrtc-aec3"
        } else {
            "nlms"
        }
    }

    /// Limpa um chunk do mic (PCM i16 16kHz) usando o que tocou desde a
    /// última chamada. `None`: o resíduo é só eco/fundo e não deve ser
    /// enviado. O retorno sai em frames inteiros de 10ms (pode atrasar até
    /// 9ms de amostras para o próximo chunk).
    pub fn process(&mut self, mic: &[i16]) -> Option<Vec<i16>> {
        let mut played = Vec::new();
        if let Some(reference) = self.reference.as_mut() {
            reference.drain_into(&mut played);
        }
        self.process_with(&played, mic)
    }

    /// Núcleo de [`Self::process`] com a referência explícita (16kHz, -1..1).
    fn process_with(&mut self, played: &[f32], mic: &[i16]) -> Option<Vec<i16>> {
        self.far.extend_from_slice(played);
        let whole = self.far.len() / FRAME * FRAME;
        for frame in self.far[..whole].as_chunks::<FRAME>().0 {
            self.canceller.render(frame);
            self.gate.far_frame(frame);
        }
        self.far.drain(..whole);

        self.near.extend(mic.iter().map(|&s| to_unit(s)));
        let whole = self.near.len() / FRAME * FRAME;
        let mut cleaned: Vec<f32> = self.near.drain(..whole).collect();
        for frame in cleaned.as_chunks_mut::<FRAME>().0 {
            self.canceller.capture(frame);
        }
        if cleaned.is_empty() {
            return Some(Vec::new());
        }
        self.gate
            .pass(&cleaned)
            .then(|| cleaned.into_iter().map(to_i16).collect())
    }
}

fn default_canceller() -> Box<dyn Canceller> {
    #[cfg(target_os = "macos")]
    if let Some(webrtc) = webrtc::WebRtc::new() {
        return Box::new(webrtc);
    }
    Box::new(Nlms::new(
        NLMS_TAPS,
        AEC_RATE as usize * NLMS_DELAY_MS / 1000,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHUNK: usize = 320;

    fn tone(len: usize, start: usize, freq: f32, amp: f32) -> Vec<f32> {
        (start..start + len)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / AEC_RATE as f32).sin())
            .collect()
    }

    /// Voz sintética: harmônicos com envelope silábico (~4Hz), não
    /// estacionária como um seno puro.
    fn voice(len: usize, start: usize) -> Vec<f32> {
        (start..start + len)
            .map(|i| {
                let t = i as f32 / AEC_RATE as f32;
                let env = (2.0 * std::f32::consts::PI * 4.0 * t).sin().abs();
                let f0 = 180.0 + 30.0 * (2.0 * std::f32::consts::PI * 0.7 * t).sin();
                let s: f32 = (1..6)
                    .map(|h| (2.0 * std::f32::consts::PI * f0 * h as f32 * t).sin() / h as f32)
                    .sum();
                0.2 * env * s
            })
            .collect()
    }

    fn db(signal: f32, reference: f32) -> f32 {
        20.0 * (signal.max(1e-9) / reference).log10()
    }

    fn to_pcm(samples: &[f32]) -> Vec<i16> {
        samples.iter().map(|&s| to_i16(s)).collect()
    }

    fn noise(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    }

    /// Seno de 440Hz tocado com um fundo de banda larga 30 dB abaixo (o
    /// chiado que toda saída real tem): um seno matematicamente puro é
    /// periódico e deixa o estimador de atraso do AEC3 sem pista nenhuma.
    fn played_sine(total: usize) -> Vec<f32> {
        let mut seed = 11;
        tone(total, 0, 440.0, 0.3)
            .into_iter()
            .map(|s| s + 0.01 * noise(&mut seed))
            .collect()
    }

    /// Eco do seno no mic (30ms de atraso, ganho 0.3, ruído de fundo), frame
    /// a frame direto no cancelador, sem o gate. Devolve o resíduo dos
    /// últimos 3s em dB relativo ao eco.
    fn canceller_residual_db(canceller: &mut dyn Canceller) -> f32 {
        let total = AEC_RATE as usize * 8;
        let delay = 480;
        let far = played_sine(total);
        let mut seed = 7;
        let tail = total - AEC_RATE as usize * 3;
        let (mut echo_tail, mut out_tail) = (Vec::new(), Vec::new());
        for start in (0..total).step_by(FRAME) {
            canceller.render(&far[start..start + FRAME]);
            let echo: Vec<f32> = (start..start + FRAME)
                .map(|i| {
                    if i >= delay {
                        0.3 * far[i - delay]
                    } else {
                        0.0
                    }
                })
                .collect();
            let mut mic: Vec<f32> = echo.iter().map(|e| e + 0.001 * noise(&mut seed)).collect();
            canceller.capture(&mut mic);
            if start >= tail {
                echo_tail.extend(echo);
                out_tail.extend(mic);
            }
        }
        db(rms(&out_tail), rms(&echo_tail))
    }

    #[test]
    fn default_canceller_removes_played_sine() {
        let atten = canceller_residual_db(&mut *default_canceller());
        assert!(atten < -20.0, "resíduo {atten:.1} dB");
    }

    #[test]
    fn nlms_removes_played_sine() {
        let mut nlms = Nlms::new(NLMS_TAPS, AEC_RATE as usize * NLMS_DELAY_MS / 1000);
        let atten = canceller_residual_db(&mut nlms);
        assert!(atten < -20.0, "resíduo NLMS {atten:.1} dB");
    }

    #[test]
    fn echo_only_chunks_are_not_sent_while_playing() {
        // Pipeline inteiro (AEC + gate), chunks de 20ms como o motor: depois
        // de convergir, o que sai durante o playback fica 20 dB abaixo do eco
        // (chunk descartado conta como silêncio).
        let mut aec = EchoCanceller::detached();
        let total = AEC_RATE as usize * 8;
        let far = played_sine(total);
        let echo: Vec<f32> = (0..total)
            .map(|i| if i >= 480 { 0.3 * far[i - 480] } else { 0.0 })
            .collect();
        let tail = total - AEC_RATE as usize * 3;
        let mut out = Vec::new();
        for start in (0..total).step_by(CHUNK) {
            let sent = aec.process_with(
                &far[start..start + CHUNK],
                &to_pcm(&echo[start..start + CHUNK]),
            );
            if start >= tail {
                match sent {
                    Some(clean) => out.extend(clean.iter().map(|&s| to_unit(s))),
                    None => out.extend(std::iter::repeat_n(0.0, CHUNK)),
                }
            }
        }
        let atten = db(rms(&out), rms(&echo[tail..]));
        assert!(atten < -20.0, "saída {atten:.1} dB relativa ao eco");
    }

    fn mic_without_playback_passes(aec: &mut EchoCanceller) {
        let total = AEC_RATE as usize * 4;
        let speech = voice(total, 0);
        let silence = vec![0.0; CHUNK];
        let mut out = Vec::new();
        for start in (0..total).step_by(CHUNK) {
            let clean = aec
                .process_with(&silence, &to_pcm(&speech[start..start + CHUNK]))
                .expect("sem playback o gate não fecha");
            out.extend(clean.iter().map(|&s| to_unit(s)));
        }
        let tail = total / 2;
        let change = db(rms(&out[tail..]), rms(&speech[tail..]));
        assert!(change.abs() < 3.0, "voz sem playback mudou {change:.1} dB");
    }

    #[test]
    fn default_canceller_keeps_mic_without_playback() {
        mic_without_playback_passes(&mut EchoCanceller::detached());
    }

    #[test]
    fn nlms_keeps_mic_without_playback_intact() {
        let mut aec = EchoCanceller::with_canceller(Box::new(Nlms::new(NLMS_TAPS, 320)));
        let total = AEC_RATE as usize * 2;
        let speech = to_pcm(&voice(total, 0));
        let mut out = Vec::new();
        for chunk in speech.chunks(CHUNK) {
            out.extend(aec.process_with(&[0.0; CHUNK], chunk).unwrap());
        }
        assert_eq!(out, speech, "NLMS sem referência não altera o mic");
    }

    #[test]
    fn user_voice_over_playback_is_sent() {
        // Barge-in: com o player tocando, a voz do usuário por cima passa.
        let mut aec = EchoCanceller::detached();
        let total = AEC_RATE as usize * 2;
        let start0 = AEC_RATE as usize * 4;
        let far = tone(total, start0, 440.0, 0.3);
        let speech = voice(total, 0);
        let mut sent = 0;
        for start in (0..total).step_by(CHUNK) {
            let mic: Vec<f32> = (start..start + CHUNK)
                .map(|i| speech[i] + if i >= 480 { 0.5 * far[i - 480] } else { 0.0 })
                .collect();
            if aec
                .process_with(&far[start..start + CHUNK], &to_pcm(&mic))
                .is_some()
            {
                sent += 1;
            }
        }
        let chunks = total / CHUNK;
        assert!(
            sent * 2 > chunks,
            "só {sent}/{chunks} chunks de fala passaram"
        );
    }

    #[test]
    fn gate_blocks_echo_only_chunks_while_playing() {
        let mut gate = ResidualGate::new();
        gate.far_frame(&tone(FRAME, 0, 440.0, 0.3));
        assert!(gate.far_active());
        assert!(
            !gate.pass(&vec![0.0005; CHUNK]),
            "resíduo baixo com player tocando"
        );
        assert!(gate.pass(&tone(CHUNK, 0, 300.0, 0.1)), "fala alta passa");
        // Sem referência por mais que o hold, tudo volta a passar.
        for _ in 0..20 {
            gate.pass(&vec![0.0; CHUNK]);
        }
        assert!(!gate.far_active());
        assert!(gate.pass(&vec![0.0005; CHUNK]));
    }

    #[test]
    fn reference_resamples_device_rate_to_16k() {
        let (mut tap, mut reference) = reference_channel(48_000);
        for s in tone(48_000, 0, 440.0, 0.25).iter().map(|&s| to_i16(s)) {
            tap.push(s);
        }
        let mut out = Vec::new();
        reference.drain_into(&mut out);
        assert!(out.len().abs_diff(16_000) < 800, "{} amostras", out.len());
        let level = rms(&out[1000..]);
        assert!((level - 0.25 / 2f32.sqrt()).abs() < 0.02, "rms {level}");
    }
}
