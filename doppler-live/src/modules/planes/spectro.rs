//! Streaming low-band spectrogram with per-bin whitening.
//!
//! Each frame is a Hann-windowed real FFT of the last `nfft` samples, taken every `hop`
//! samples. Only the bins between `fmin` and `fmax` are kept. Power in dB is whitened per bin
//! against a running median and median absolute deviation, so steady tones (mains hum, engine
//! drones that never move) sit near zero and only *changing* tones stand out. A per-frame
//! median across bins is then subtracted so broadband bursts (wind, handling noise) do not
//! light up a whole column.
//!
//! The running statistics use sign-based stochastic quantile updates: O(1) per bin and frame,
//! no history buffer, robust to a tone sitting in one bin for a while.

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

pub struct Frontend {
    pub sr: f64,
    pub nfft: usize,
    pub hop: usize,
    /// First kept FFT bin.
    pub b0: usize,
    /// Number of kept bins.
    pub nb: usize,
    /// Bin width in Hz.
    pub df: f64,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    pending: Vec<f32>,
    med: Vec<f32>,
    mad: Vec<f32>,
    level: Vec<f32>,
    tmp: Vec<f32>,
    frames: u64,
}

impl Frontend {
    pub fn new(sr: f64, nfft: usize, hop: usize, fmin: f64, fmax: f64) -> Self {
        let df = sr / nfft as f64;
        let b0 = (fmin / df).ceil() as usize;
        let b1 = ((fmax / df).floor() as usize).min(nfft / 2);
        let nb = b1 - b0 + 1;
        let window: Vec<f32> = (0..nfft)
            .map(|i| {
                let x = std::f64::consts::PI * 2.0 * i as f64 / nfft as f64;
                (0.5 - 0.5 * x.cos()) as f32
            })
            .collect();
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(nfft);
        let input = fft.make_input_vec();
        let spectrum = fft.make_output_vec();
        let scratch = fft.make_scratch_vec();
        Self {
            sr,
            nfft,
            hop,
            b0,
            nb,
            df,
            window,
            fft,
            input,
            spectrum,
            scratch,
            pending: Vec::with_capacity(nfft + hop * 8),
            med: vec![0.0; nb],
            mad: vec![3.0; nb],
            level: vec![0.0; nb],
            tmp: vec![0.0; nb],
            frames: 0,
        }
    }

    /// Frequency (Hz) of kept bin `i`, fractional bins allowed.
    pub fn bin_hz(&self, i: f64) -> f64 {
        (self.b0 as f64 + i) * self.df
    }

    /// Fractional kept-bin index of frequency `f` (may be out of range).
    pub fn hz_bin(&self, f: f64) -> f64 {
        f / self.df - self.b0 as f64
    }

    /// Frames per second.
    pub fn fps(&self) -> f64 {
        self.sr / self.hop as f64
    }

    pub fn feed(&mut self, x: &[f32]) {
        self.pending.extend_from_slice(x);
    }

    /// Compute the next whitened frame into `out` (length `nb`) if enough samples are
    /// buffered. Returns the frame's centre time in seconds since the stream start.
    pub fn next_frame(&mut self, out: &mut [f32]) -> Option<f64> {
        // A frame needs its window, and the step to the next frame must be buffered too
        // (the step can be longer than the window, as in the coarse display layer).
        if self.pending.len() < self.nfft.max(self.hop) {
            return None;
        }
        for (i, v) in self.input.iter_mut().enumerate() {
            *v = self.pending[i] * self.window[i];
        }
        self.pending.drain(..self.hop);
        self.fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .expect("fft sizes are fixed at construction");

        for i in 0..self.nb {
            let c = self.spectrum[self.b0 + i];
            self.level[i] = 10.0 * (c.norm_sqr() + 1e-12).log10();
        }

        // Fast start so the first minute converges, then slow so a tone holding one bin for
        // tens of seconds (the flat ends of a Doppler curve) is not absorbed into the median.
        let eta = if self.frames == 0 {
            self.med.copy_from_slice(&self.level);
            0.0
        } else if self.frames < 120 {
            0.3
        } else if self.frames < 240 {
            0.05
        } else {
            0.01
        };
        for i in 0..self.nb {
            let l = self.level[i];
            let m = self.med[i];
            self.med[i] = m + eta * (l - m).signum();
            let dev = (l - self.med[i]).abs();
            let s = self.mad[i];
            self.mad[i] = (s + 0.5 * eta * (dev - s).signum()).max(0.3);
            out[i] = (l - self.med[i]) / (1.4826 * self.mad[i]);
        }

        // Remove broadband columns: subtract the frame's median across bins.
        self.tmp.copy_from_slice(out);
        let mid = self.nb / 2;
        let (_, m, _) = self.tmp.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
        let m = *m;
        for v in out.iter_mut() {
            *v -= m;
        }

        let t = (self.frames as f64 * self.hop as f64 + self.nfft as f64 / 2.0) / self.sr;
        self.frames += 1;
        Some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hop_longer_than_window() {
        // 1024-point window, 4000-sample step (the viewer's full-band display layer).
        let mut fe = Frontend::new(16_000.0, 1024, 4000, 0.0, 8000.0);
        let mut out = vec![0.0; fe.nb];
        let mut frames = 0;
        for _ in 0..40 {
            fe.feed(&[0.1; 800]);
            while fe.next_frame(&mut out).is_some() {
                frames += 1;
            }
        }
        // 32 000 samples, a frame every 4000 once 4000 are buffered: 8 frames.
        assert_eq!(frames, 8);
    }
}
