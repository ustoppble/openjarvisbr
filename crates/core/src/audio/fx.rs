//! Efeito de voz "Jarvis": timbre levemente metálico/mecânico aplicado ao
//! PCM 24kHz mono do modelo antes do playback.
//!
//! Receita (toda em dose baixa, controlada por `amount` 0..1):
//! - comb filter curto com feedback → ressonância metálica ("voz de capacete")
//! - modulação em anel a ~55Hz misturada de leve → textura mecânica
//! - passa-alta suave em ~120Hz → tira o "corpo" e aproxima de alto-falante
//!
//! `amount = 0` é identidade bit a bit. O ganho é normalizado para nunca
//! ultrapassar o nível de entrada.

// Os examples incluem este módulo por `#[path]` sem usá-lo.
#![allow(dead_code)]

const RATE: f32 = 24_000.0;
/// Atraso do comb em amostras (~5,2ms → ressonância em ~190Hz e harmônicos).
const COMB_DELAY: usize = 125;
const RING_HZ: f32 = 55.0;
const HIGHPASS_HZ: f32 = 120.0;

pub struct VoiceFx {
    amount: f32,
    comb: Vec<f32>,
    comb_pos: usize,
    ring_phase: f32,
    hp_prev_in: f32,
    hp_prev_out: f32,
}

impl VoiceFx {
    pub fn new(amount: f32) -> Self {
        Self {
            amount: amount.clamp(0.0, 1.0),
            comb: vec![0.0; COMB_DELAY],
            comb_pos: 0,
            ring_phase: 0.0,
            hp_prev_in: 0.0,
            hp_prev_out: 0.0,
        }
    }

    pub fn amount(&self) -> f32 {
        self.amount
    }

    /// Processa um chunk no lugar.
    pub fn process(&mut self, samples: &mut [i16]) {
        if self.amount <= 0.0 {
            return;
        }
        let a = self.amount;
        let feedback = 0.45 * a;
        let ring_mix = 0.22 * a;
        let hp_alpha = {
            let rc = 1.0 / (2.0 * std::f32::consts::PI * HIGHPASS_HZ);
            let dt = 1.0 / RATE;
            rc / (rc + dt)
        };
        let ring_step = 2.0 * std::f32::consts::PI * RING_HZ / RATE;
        // Compensação de ganho: o comb com feedback f amplifica até 1/(1-f).
        let makeup = 1.0 - feedback;

        for s in samples.iter_mut() {
            let x = *s as f32;

            // passa-alta de 1 polo, misturado pelo amount
            let hp = hp_alpha * (self.hp_prev_out + x - self.hp_prev_in);
            self.hp_prev_in = x;
            self.hp_prev_out = hp;
            let x = x + (hp - x) * a * 0.6;

            // comb com feedback
            let delayed = self.comb[self.comb_pos];
            let y = x + delayed * feedback;
            self.comb[self.comb_pos] = y;
            self.comb_pos = (self.comb_pos + 1) % COMB_DELAY;
            let y = y * makeup;

            // modulação em anel misturada de leve
            let ring = y * self.ring_phase.sin();
            self.ring_phase += ring_step;
            if self.ring_phase > 2.0 * std::f32::consts::PI {
                self.ring_phase -= 2.0 * std::f32::consts::PI;
            }
            let y = y * (1.0 - ring_mix) + ring * ring_mix;

            *s = y.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize, hz: f32, amp: f32) -> Vec<i16> {
        (0..n)
            .map(|i| (amp * (2.0 * std::f32::consts::PI * hz * i as f32 / RATE).sin()) as i16)
            .collect()
    }

    #[test]
    fn amount_zero_is_identity() {
        let original = sine(4800, 220.0, 8000.0);
        let mut buf = original.clone();
        VoiceFx::new(0.0).process(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn full_amount_changes_signal_without_clipping() {
        let original = sine(24_000, 220.0, 12_000.0);
        let mut buf = original.clone();
        VoiceFx::new(1.0).process(&mut buf);
        assert_ne!(buf, original);
        let peak = buf.iter().map(|s| (*s as i32).abs()).max().unwrap();
        assert!(peak < 32_000, "estourou: pico {peak}");
        let rms = |v: &[i16]| {
            (v.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
        };
        let ratio = rms(&buf) / rms(&original);
        assert!(ratio > 0.4 && ratio < 1.6, "nível mudou demais: {ratio:.2}");
    }

    #[test]
    fn silence_stays_silent() {
        let mut buf = vec![0i16; 2400];
        VoiceFx::new(0.7).process(&mut buf);
        assert!(buf.iter().all(|&s| s == 0));
    }
}
