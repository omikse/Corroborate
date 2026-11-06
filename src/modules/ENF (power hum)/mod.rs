//! ENF (Electrical Network Frequency) module: the mains hum and its drift.
//!
//! Power grids run at a nominal 50 Hz or 60 Hz, but the real frequency wanders by tens of
//! millihertz as supply and demand change, the same way across a whole grid. Equipment
//! near a microphone leaks that hum (and its harmonics) into recordings, so the drift pattern
//! is a forensic fingerprint: it identifies the grid and, matched against grid frequency
//! logs, the time of recording.
//!
//! Method: downsample the shared 16 kHz stream to 4 kHz; every second, take the last 16 s,
//! Hann window, zero-pad to 262 144 points (0.015 Hz bins), and find the strongest peak near
//! each harmonic h = 1..4 of 50 Hz and 60 Hz. The harmonic with the best SNR (signal-to-noise
//! ratio) gives the estimate f = peak / h, refined by parabolic interpolation. The nominal
//! frequency is decided by vote once the hum is clear. Runs of clear readings of at least
//! 30 s become findings.

use crate::core::NodeSpec;
use crate::ingest::Resampler;
use crate::module::{Finding, Module, INPUT_SR};
use crate::timebase::{fmt_utc, json_escape, Anchor};
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

crate::builtin_manifest!();

/// The node's ports: the one definition, used by the workbench and by `Module::spec`.
/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
}

const SR: f64 = 4000.0;
const WIN_S: f64 = 16.0;
const HOP_S: f64 = 1.0;
const NFFT: usize = 262_144;
const HARMONICS: u32 = 4;
/// Search ± this much around each harmonic (scaled by h).
const SEARCH_HZ: f64 = 0.25;
/// Readings at or above this SNR (dB) count as clear hum.
const SNR_MIN_DB: f64 = 10.0;
/// Shortest run reported as a finding, and the longest gap bridged within a run.
const MIN_SEG_S: f64 = 30.0;
const MAX_GAP_S: f64 = 5.0;
/// A reading further than this from the recent median is an outlier (real grids drift
/// slowly: tens of millihertz over minutes).
const MAX_JUMP_HZ: f64 = 0.03;

/// One reading: the grid frequency estimate at time `t` (relative seconds, window centre).
#[derive(Clone, Copy, Debug)]
pub struct EnfPoint {
    pub t: f64,
    pub hz: f64,
    pub nominal: f64,
    pub harmonic: u32,
    pub snr_db: f64,
    /// Clear and continuous: used in findings. False readings are shown, not used.
    pub ok: bool,
}

/// A run of clear hum readings.
#[derive(Clone, Debug)]
pub struct EnfSegment {
    pub id: u64,
    pub nominal: f64,
    /// Harmonic used most often during the run.
    pub harmonic: u32,
    pub t_start: f64,
    pub t_end: f64,
    pub mean_hz: f64,
    pub min_hz: f64,
    pub max_hz: f64,
    pub mean_snr_db: f64,
    pub n_points: usize,
}

impl EnfSegment {
    pub fn to_json(&self, anchor: Option<&Anchor>) -> String {
        let mut s = format!(
            "{{\"id\":{},\"nominal_hz\":{:.0},\"harmonic\":{},\"t_start\":{:.2},\"t_end\":{:.2},\"mean_hz\":{:.4},\
             \"min_hz\":{:.4},\"max_hz\":{:.4},\"mean_snr_db\":{:.1},\"n_points\":{}",
            self.id,
            self.nominal,
            self.harmonic,
            self.t_start,
            self.t_end,
            self.mean_hz,
            self.min_hz,
            self.max_hz,
            self.mean_snr_db,
            self.n_points
        );
        match anchor {
            Some(a) => s.push_str(&format!(
                ",\"utc_start\":\"{}\",\"utc_end\":\"{}\",\"utc_source\":\"{}\"",
                fmt_utc(a.utc(self.t_start)),
                fmt_utc(a.utc(self.t_end)),
                json_escape(&a.source)
            )),
            None => s.push_str(",\"utc_start\":null,\"utc_source\":\"none (relative time only)\""),
        }
        s.push('}');
        s
    }
}

struct Run {
    pts: Vec<EnfPoint>,
}

pub struct Enf {
    rs: Resampler,
    buf: Vec<f32>,
    /// Last WIN_S of 4 kHz audio, ring.
    ring: Vec<f32>,
    head: usize,
    total: u64,
    next_at: u64,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: Vec<f32>,
    votes: [u32; 2],
    nominal: Option<f64>,
    /// Harmonic used by the current run.
    harmonic: Option<u32>,
    /// Readings not yet collected by `take_points`.
    new_points: Vec<EnfPoint>,
    run: Option<Run>,
    next_id: u64,
}

impl Default for Enf {
    fn default() -> Self {
        Self::new()
    }
}

impl Enf {
    pub fn new() -> Self {
        let win = (WIN_S * SR) as usize;
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(NFFT);
        let window = (0..win)
            .map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / win as f64).cos()) as f32)
            .collect();
        Self {
            rs: Resampler::new(INPUT_SR, SR),
            buf: Vec::with_capacity(1024),
            ring: vec![0.0; win],
            head: 0,
            total: 0,
            next_at: win as u64,
            window,
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            power: vec![0.0; NFFT / 2 + 1],
            fft,
            votes: [0, 0],
            nominal: None,
            harmonic: None,
            new_points: Vec::new(),
            run: None,
            next_id: 1,
        }
    }

    /// The grid's nominal frequency once decided (50 or 60 Hz).
    pub fn nominal(&self) -> Option<f64> {
        self.nominal
    }

    /// Readings since the last call (for live display).
    pub fn take_points(&mut self) -> Vec<EnfPoint> {
        std::mem::take(&mut self.new_points)
    }

    fn bin(f: f64) -> f64 {
        f * NFFT as f64 / SR
    }

    /// Peak near `fc`: (interpolated frequency, SNR dB).
    fn peak(&self, fc: f64, h: f64) -> Option<(f64, f64)> {
        let lo = Self::bin(fc - SEARCH_HZ * h).floor().max(1.0) as usize;
        let hi = (Self::bin(fc + SEARCH_HZ * h).ceil() as usize).min(self.power.len() - 2);
        let (mut k, mut best) = (lo, 0.0f32);
        for i in lo..=hi {
            if self.power[i] > best {
                best = self.power[i];
                k = i;
            }
        }
        // A maximum on the edge of the search range is the slope of something else, not a peak.
        if best <= 0.0 || k == lo || k == hi {
            return None;
        }
        // Noise: median power 1–3 Hz either side (×h), clear of the peak.
        let mut noise: Vec<f32> = Vec::new();
        for (a, b) in [(fc - 3.0 * h, fc - 1.0 * h), (fc + 1.0 * h, fc + 3.0 * h)] {
            let (ia, ib) = (Self::bin(a).max(1.0) as usize, (Self::bin(b) as usize).min(self.power.len() - 1));
            noise.extend(self.power[ia..ib].iter().step_by(4).copied());
        }
        if noise.is_empty() {
            return None;
        }
        let m = noise.len() / 2;
        let (_, med, _) = noise.select_nth_unstable_by(m, |a, b| a.total_cmp(b));
        let snr_db = 10.0 * ((best / med.max(1e-30)) as f64).log10();
        // Parabolic interpolation on log power.
        let (a, b, c) = (
            (self.power[k - 1].max(1e-30) as f64).ln(),
            (best as f64).ln(),
            (self.power[k + 1].max(1e-30) as f64).ln(),
        );
        let den = a - 2.0 * b + c;
        let d = if den < 0.0 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 };
        Some(((k as f64 + d) * SR / NFFT as f64, snr_db))
    }

    fn analyse(&mut self) -> Option<EnfPoint> {
        let win = self.ring.len();
        for i in 0..win {
            self.input[i] = self.ring[(self.head + i) % win] * self.window[i];
        }
        for v in &mut self.input[win..] {
            *v = 0.0;
        }
        self.fft.process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch).ok()?;
        for (p, c) in self.power.iter_mut().zip(&self.spectrum) {
            *p = c.norm_sqr();
        }
        let nominals: Vec<f64> = match self.nominal {
            Some(n) => vec![n],
            None => vec![50.0, 60.0],
        };
        let mut best: Option<EnfPoint> = None;
        let mut current: Option<EnfPoint> = None;
        for &nom in &nominals {
            for h in 1..=HARMONICS {
                let fc = nom * h as f64;
                if let Some((f, snr)) = self.peak(fc, h as f64) {
                    let p = EnfPoint { t: 0.0, hz: f / h as f64, nominal: nom, harmonic: h, snr_db: snr, ok: false };
                    if best.is_none_or(|b| snr > b.snr_db) {
                        best = Some(p);
                    }
                    if self.harmonic == Some(h) && self.nominal == Some(nom) {
                        current = Some(p);
                    }
                }
            }
        }
        // Stay on the harmonic in use unless another is clearly (3 dB) better: switching
        // harmonics mid-run adds jumps that are not in the grid.
        let mut p = match (current, best) {
            (Some(c), Some(b)) if c.snr_db + 3.0 >= b.snr_db => c,
            (_, b) => b?,
        };
        p.t = (self.total as f64 - win as f64 / 2.0) / SR;
        if self.nominal.is_none() && p.snr_db >= SNR_MIN_DB {
            let i = if p.nominal == 50.0 { 0 } else { 1 };
            self.votes[i] += 1;
            let (a, b) = (self.votes[i], self.votes[1 - i]);
            if a >= 10 && a >= 3 * b {
                self.nominal = Some(p.nominal);
            }
        }
        Some(p)
    }

    /// Judge a reading and add it to the current run; close the run if the hum went away.
    /// A reading counts (`ok`) if it is clear (SNR) and continuous: within MAX_JUMP_HZ of the
    /// median of the run's last readings. Others are kept for display, flagged, not used.
    fn track(&mut self, p: &mut EnfPoint) -> Option<EnfSegment> {
        let clear = p.snr_db >= SNR_MIN_DB && self.nominal == Some(p.nominal);
        let mut out = None;
        if let Some(run) = &self.run {
            let last = run.pts.last().map_or(p.t, |q| q.t);
            if p.t - last > MAX_GAP_S {
                out = self.close();
            }
        }
        let continuous = match &self.run {
            Some(run) if run.pts.len() >= 5 => {
                let mut tail: Vec<f64> = run.pts.iter().rev().take(9).map(|q| q.hz).collect();
                tail.sort_by(f64::total_cmp);
                (p.hz - tail[tail.len() / 2]).abs() <= MAX_JUMP_HZ
            }
            _ => true,
        };
        p.ok = clear && continuous;
        if p.ok {
            self.run.get_or_insert(Run { pts: Vec::new() }).pts.push(*p);
            self.harmonic = Some(p.harmonic);
        }
        out
    }

    fn close(&mut self) -> Option<EnfSegment> {
        let run = self.run.take()?;
        let (a, b) = (run.pts.first()?.t, run.pts.last()?.t);
        if b - a < MIN_SEG_S {
            return None;
        }
        let n = run.pts.len() as f64;
        let mut hcount = [0usize; HARMONICS as usize + 1];
        for p in &run.pts {
            hcount[p.harmonic as usize] += 1;
        }
        let harmonic = (1..=HARMONICS).max_by_key(|&h| hcount[h as usize]).unwrap_or(1);
        let seg = EnfSegment {
            id: self.next_id,
            nominal: run.pts[0].nominal,
            harmonic,
            t_start: a,
            t_end: b,
            mean_hz: run.pts.iter().map(|p| p.hz).sum::<f64>() / n,
            min_hz: run.pts.iter().map(|p| p.hz).fold(f64::INFINITY, f64::min),
            max_hz: run.pts.iter().map(|p| p.hz).fold(f64::NEG_INFINITY, f64::max),
            mean_snr_db: run.pts.iter().map(|p| p.snr_db).sum::<f64>() / n,
            n_points: run.pts.len(),
        };
        self.next_id += 1;
        Some(seg)
    }
}

impl Module for Enf {
    fn name(&self) -> &'static str {
        manifest().id
    }

    fn spec(&self) -> NodeSpec {
        spec()
    }

    fn describe(&self) -> String {
        format!(
            "enf: 16 kHz → {SR:.0} Hz (FIR lowpass 1.3 kHz); every {HOP_S:.0} s a {WIN_S:.0} s Hann window, \
             zero-padded FFT {NFFT} points ({:.4} Hz bins) + parabolic peak; harmonics 1–{HARMONICS} of 50/60 Hz, \
             best SNR wins; clear hum = SNR ≥ {SNR_MIN_DB} dB; findings = runs ≥ {MIN_SEG_S:.0} s",
            SR / NFFT as f64
        )
    }

    fn push(&mut self, x: &[f32]) -> Vec<Finding> {
        self.buf.clear();
        self.rs.process(x, &mut self.buf);
        let mut out = Vec::new();
        let win = self.ring.len();
        let hop = (HOP_S * SR) as u64;
        for i in 0..self.buf.len() {
            self.ring[self.head] = self.buf[i];
            self.head = (self.head + 1) % win;
            self.total += 1;
            if self.total >= self.next_at {
                self.next_at += hop;
                if let Some(mut p) = self.analyse() {
                    if let Some(s) = self.track(&mut p) {
                        out.push(Finding::Enf(s));
                    }
                    self.new_points.push(p);
                }
            }
        }
        out
    }

    fn flush(&mut self) -> Vec<Finding> {
        self.close().map(Finding::Enf).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Rng;

    /// Hum at `nominal` drifting ±20 mHz over a minute, with 2nd and 3rd harmonics, in noise.
    fn hum(nominal: f64, dur: f64, noise: f32) -> (Vec<f32>, impl Fn(f64) -> f64) {
        let sr = INPUT_SR;
        let f = move |t: f64| nominal + 0.02 * (2.0 * std::f64::consts::PI * t / 60.0).sin();
        let mut rng = Rng::new(3);
        let mut ph = 0.0f64;
        let x = (0..(dur * sr) as usize)
            .map(|i| {
                let t = i as f64 / sr;
                ph += 2.0 * std::f64::consts::PI * f(t) / sr;
                (0.02 * ph.sin() + 0.03 * (2.0 * ph).sin() + 0.01 * (3.0 * ph).sin()) as f32 + noise * rng.normal() as f32
            })
            .collect();
        (x, f)
    }

    fn run(x: &[f32]) -> (Enf, Vec<EnfPoint>, Vec<Finding>) {
        let mut m = Enf::new();
        let mut pts = Vec::new();
        let mut found = Vec::new();
        for c in x.chunks(800) {
            found.extend(m.push(c));
            pts.extend(m.take_points());
        }
        found.extend(m.flush());
        (m, pts, found)
    }

    #[test]
    fn tracks_50hz_drift() {
        let (x, f) = hum(50.0, 150.0, 0.1);
        let (m, pts, found) = run(&x);
        assert_eq!(m.nominal(), Some(50.0));
        // Compare with the true frequency averaged over each 16 s window.
        let mut errs: Vec<f64> = pts
            .iter()
            .filter(|p| p.ok)
            .map(|p| {
                let mean = (0..160).map(|k| f(p.t - 8.0 + k as f64 * 0.1)).sum::<f64>() / 160.0;
                (p.hz - mean).abs()
            })
            .collect();
        assert!(errs.len() > 100, "readings {}", errs.len());
        errs.sort_by(f64::total_cmp);
        let median = errs[errs.len() / 2];
        assert!(median < 0.003, "median error {median} Hz");
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn finds_60hz_grid() {
        let (x, _) = hum(60.0, 90.0, 0.1);
        let (m, _, found) = run(&x);
        assert_eq!(m.nominal(), Some(60.0));
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn noise_has_no_hum() {
        let mut rng = Rng::new(9);
        let x: Vec<f32> = (0..(90.0 * INPUT_SR) as usize).map(|_| 0.1 * rng.normal() as f32).collect();
        let (m, _, found) = run(&x);
        assert_eq!(m.nominal(), None);
        assert!(found.is_empty());
    }
}
