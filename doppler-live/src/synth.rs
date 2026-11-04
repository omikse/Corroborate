//! Synthetic test audio: Doppler passes with harmonics, optional steady hum, white noise.

use crate::fit::{model, C_SOUND};

#[derive(Clone, Debug)]
pub struct Pass {
    pub f0: f64,
    pub v: f64,
    pub d: f64,
    pub tc: f64,
    /// Amplitude of each partial, starting with the fundamental.
    pub amps: Vec<f32>,
}

/// Small deterministic RNG (xorshift64*) with Box–Muller normals.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    pub fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        ((self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    pub fn normal(&mut self) -> f64 {
        let (a, b) = (self.uniform(), self.uniform());
        (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
    }
}

/// Render `dur_s` seconds at `sr`. `hum` is (frequency, amplitude) of a steady tone.
pub fn render(sr: f64, dur_s: f64, passes: &[Pass], hum: Option<(f64, f32)>, noise: f32, seed: u64) -> Vec<f32> {
    let n = (sr * dur_s) as usize;
    let mut rng = Rng::new(seed);
    let mut x: Vec<f32> = (0..n).map(|_| noise * rng.normal() as f32).collect();
    for p in passes {
        let pp = [p.f0, p.v, p.d, p.tc];
        let mut phase = 0.0f64;
        for (i, s) in x.iter_mut().enumerate() {
            let t = i as f64 / sr;
            phase += 2.0 * std::f64::consts::PI * model(t, &pp, C_SOUND) / sr;
            // Fade with distance so the far ends drop into the noise, as real passes do.
            let tau = t - p.tc;
            let r = (p.d * p.d + p.v * p.v * tau * tau).sqrt();
            let g = (p.d / r) as f32;
            for (h, &a) in p.amps.iter().enumerate() {
                *s += g * a * ((h + 1) as f64 * phase).sin() as f32;
            }
        }
    }
    if let Some((f, a)) = hum {
        for (i, s) in x.iter_mut().enumerate() {
            *s += a * (2.0 * std::f64::consts::PI * f * i as f64 / sr).sin() as f32;
        }
    }
    x
}
