//! ENF (Electrical Network Frequency) module: the mains hum and its drift.
//!
//! Power grids run at a nominal 50 Hz or 60 Hz, but the real frequency wanders by tens of
//! millihertz as supply and demand change, the same way across a whole grid. Equipment
//! near a microphone leaks that hum (and its harmonics) into recordings, so the drift pattern
//! is a forensic fingerprint: it identifies the grid and, matched against grid frequency
//! logs, the time of recording.
//!
//! Method: downsample the shared 16 kHz stream to 4 kHz; every second, take the last 16 s,
//! Hann window, zero-pad to 262 144 points (0.015 Hz bins). For 50 Hz and 60 Hz, take the band
//! around each harmonic h = 1..8, divide it by its own noise, rescale it to the fundamental and
//! sum: the combined spectrum. Its peak, refined by parabolic interpolation, is the reading; its
//! excess over noise is the score. The nominal frequency is decided by vote between the two
//! sums (each harmonic capped, so one loud tone can't win). Readings that jump faster than a
//! grid can (slope rule) are rejected and kept with the reason. Runs of clear readings of at
//! least 30 s become findings. Rejection rules after libhum (github.com/RaphaelJ/libhum).

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
/// Harmonics 1..=HARMONICS of the nominal are combined (8 × 60 Hz = 480 Hz, well under 2 kHz).
pub const HARMONICS: u32 = 8;
/// Search ± this much around the nominal (in the fundamental's frequency).
const SEARCH_HZ: f64 = 0.25;
/// Grid step of the combined spectrum, in bins of the fundamental.
const GRID_STEP: f64 = 0.25;
/// One harmonic's power over its noise is capped at this (20 dB) in the score, so one strong
/// tone (a fan, a whine) can't outscore hum spread over many harmonics.
const CAP: f64 = 100.0;
/// Weight of one window in the running average of each band (about 5 s memory: the grid
/// moves at most ~2.5 mHz/s, so the average lags by ~12 mHz at worst).
const AVG_ALPHA: f64 = 0.2;
/// The current reading is searched this far either side of the averaged spectrum's peak.
const TRACK_HZ: f64 = 0.025;
/// Weight of one window in each harmonic's running power at the reading (about 20 s memory).
const HIT_ALPHA: f64 = 0.05;
/// A harmonic's weight: its running power at the reading over noise, beyond this (noise alone
/// averages 1).
const WEIGHT_FLOOR: f64 = 1.3;
/// What a band's own peak over noise × ln 2 averages with no hum: the largest of ~33 noise
/// bins (each averaging 1), about ln 33 + 0.58, plus a margin.
const NOISE_PEAK: f64 = 4.6;
/// Readings at or above this score (dB) count as clear hum. The score is the combined
/// spectrum's excess over what noise alone gives, in units of its spread (see `combined`).
pub const SNR_MIN_DB: f64 = 7.0;
/// Shortest run reported as a finding, and the longest gap bridged within a run.
const MIN_SEG_S: f64 = 30.0;
const MAX_GAP_S: f64 = 5.0;
/// Section rule: a clear reading counts only inside a run of at least this long.
pub const MIN_SECTION_S: f64 = 8.0;
/// Slope rule: a grid can't change faster than this (Hz per second). A reading further from
/// its clear neighbours than the slope allows (plus three times both errors) is rejected.
pub const MAX_SLOPE_HZ_S: f64 = 0.0025;

/// Why a reading is not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// Score under SNR_MIN_DB: no clear hum in this window.
    Weak,
    /// Clear, but the grid (50 or 60 Hz) is not decided yet.
    Undecided,
    /// Clear, but jumps faster than a grid can (slope rule).
    Slope,
    /// Clear, but not inside a run of at least MIN_SECTION_S (section rule).
    Section,
}

impl Reject {
    pub fn label(self) -> &'static str {
        match self {
            Reject::Weak => "too weak",
            Reject::Undecided => "grid not decided yet",
            Reject::Slope => "slope rule",
            Reject::Section => "section rule",
        }
    }
}

/// One reading: the grid frequency estimate at time `t` (relative seconds, window centre).
#[derive(Clone, Copy, Debug)]
pub struct EnfPoint {
    pub t: f64,
    /// The fundamental's frequency (not the harmonic's).
    pub hz: f64,
    pub nominal: f64,
    /// The harmonic that contributed most, for display.
    pub harmonic: u32,
    /// Detection score, dB (see `SNR_MIN_DB`).
    pub snr_db: f64,
    /// Estimated error of `hz` (1 sigma), Hz.
    pub err_hz: f64,
    /// Bit h-1 set: harmonic h stood 3 dB above its noise at the peak.
    pub used: u8,
    /// Clear and continuous: used in findings. False readings are shown, not used.
    pub ok: bool,
    /// Why not `ok`.
    pub reject: Option<Reject>,
}

impl EnfPoint {
    /// The harmonics that stood out, as a list ("2, 3, 5").
    pub fn used_list(&self) -> Vec<u32> {
        (1..=HARMONICS).filter(|h| self.used & (1 << (h - 1)) != 0).collect()
    }
}

/// The combined spectrum's best peak for one nominal.
#[derive(Clone, Copy, Debug)]
struct Peak {
    hz: f64,
    score_db: f64,
    /// Capped excess, for comparing nominals.
    excess: f64,
    harmonic: u32,
    used: u8,
    err_hz: f64,
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
    /// Before the grid is decided, per nominal: the run of clear readings that keep to the
    /// slope rule (last t, last hz, length). Only readings in such a run of MIN_SECTION_S vote.
    pre: [(f64, f64, f64); 2],
    nominal: Option<f64>,
    /// Per nominal (50, 60) and harmonic: the band over its noise, averaged over recent
    /// windows. Sets the harmonic's weight and where the current reading is searched.
    band_avg: [Vec<Vec<f64>>; 2],
    /// Per nominal and harmonic: running mean of the band over its noise at the reading's
    /// frequency, × ln 2 (1 for noise alone). Sets the harmonic's weight.
    hit_avg: [[f64; HARMONICS as usize]; 2],
    /// Per nominal and harmonic: running mean of the band's own peak over noise × ln 2. Starts
    /// the weights before any reading is locked.
    peak_avg: [[f64; HARMONICS as usize]; 2],
    /// Recent clear readings (on the decided grid), for the slope rule: (t, hz, err).
    recent: Vec<(f64, f64, f64)>,
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
            pre: [(f64::NEG_INFINITY, 0.0, 0.0); 2],
            nominal: None,
            band_avg: [Vec::new(), Vec::new()],
            hit_avg: [[1.0; HARMONICS as usize]; 2],
            peak_avg: [[NOISE_PEAK; HARMONICS as usize]; 2],
            recent: Vec::new(),
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

    /// Power at frequency `f`, linearly interpolated between bins.
    fn power_at(&self, f: f64) -> f64 {
        let b = Self::bin(f);
        let i = b.floor() as usize;
        if i + 1 >= self.power.len() {
            return 0.0;
        }
        let w = b - i as f64;
        self.power[i] as f64 * (1.0 - w) + self.power[i + 1] as f64 * w
    }

    /// Noise near harmonic `fc`: median power 1–3 Hz either side (×h), clear of the peak.
    fn noise(&self, fc: f64, h: f64) -> Option<f64> {
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
        Some((*med as f64).max(1e-30))
    }

    /// The combined spectrum for nominal `nom`: every harmonic's band, divided by its own noise
    /// and rescaled to the fundamental, summed. Its peak is the reading. The score is how far the
    /// sum at the peak (each harmonic capped at CAP) stands above what noise alone gives there:
    /// noise power over its median averages 1/ln 2 per harmonic, with the same spread.
    ///
    /// Each harmonic is weighted by how far its band's peak has stood above noise lately (a
    /// matched weighting): harmonics with no hum get no weight, so they don't drown the ones
    /// that have it. With no harmonic standing out yet, all weigh the same.
    fn combined(&mut self, ni: usize, nom: f64) -> Option<Peak> {
        use std::f64::consts::LN_2;
        let df = SR / NFFT as f64;
        let step = GRID_STEP * df;
        let n = (2.0 * SEARCH_HZ / step) as usize + 1;
        let f0 = nom - SEARCH_HZ;
        let mut rows: Vec<(u32, Vec<f64>)> = Vec::new();
        for h in 1..=HARMONICS {
            let hf = h as f64;
            if (nom + SEARCH_HZ + 3.0) * hf >= SR / 2.0 - 1.0 {
                break;
            }
            let Some(noise) = self.noise(nom * hf, hf) else { continue };
            let r: Vec<f64> = (0..n).map(|i| self.power_at((f0 + i as f64 * step) * hf) / noise).collect();
            rows.push((h, r));
        }
        if rows.is_empty() {
            return None;
        }
        // Running average of each band: noise averages out, the hum (moving slowly) stays.
        let avg = &mut self.band_avg[ni];
        if avg.len() != rows.len() {
            *avg = rows.iter().map(|(_, r)| r.clone()).collect();
        } else {
            for (a, (_, r)) in avg.iter_mut().zip(&rows) {
                for (x, v) in a.iter_mut().zip(r) {
                    *x += AVG_ALPHA * (v - *x);
                }
            }
        }
        let argmax = |v: &[f64]| v.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i);
        let pk = &mut self.peak_avg[ni];
        for (h, r) in &rows {
            let top = r.iter().fold(0.0f64, |m, &b| m.max(b)) * LN_2;
            let x = &mut pk[*h as usize - 1];
            *x += HIT_ALPHA * (top - *x);
        }
        let hit = &mut self.hit_avg[ni];
        let mut w: Vec<f64> = rows
            .iter()
            .map(|(h, _)| {
                let i = *h as usize - 1;
                (hit[i] - WEIGHT_FLOOR).max(0.0) + (self.peak_avg[ni][i] - NOISE_PEAK).max(0.0)
            })
            .collect();
        if w.iter().all(|&x| x == 0.0) {
            w.iter_mut().for_each(|x| *x = 1.0);
        }
        // Where the averaged combined spectrum peaks: the current reading is searched near it.
        let asum: Vec<f64> = (0..n).map(|i| avg.iter().zip(&w).map(|(a, wh)| wh * a[i]).sum()).collect();
        let c = argmax(&asum)?;
        // A maximum on the edge of the search range is the slope of something else, not a peak.
        if c == 0 || c == n - 1 {
            return None;
        }
        let half = (TRACK_HZ / step) as usize;
        let (lo, hi) = (c.saturating_sub(half).max(1), (c + half).min(n - 2));
        let mut sum = vec![0.0f64; n];
        for ((_, r), &wh) in rows.iter().zip(&w) {
            for (s, v) in sum.iter_mut().zip(r) {
                *s += wh * v;
            }
        }
        let k = lo + argmax(&sum[lo..=hi])?;
        for (h, r) in &rows {
            let x = &mut hit[*h as usize - 1];
            *x += HIT_ALPHA * (r[k] * LN_2 - *x);
        }
        // Parabolic interpolation on the log of the sum.
        let (a, b, c) = (sum[k - 1].max(1e-30).ln(), sum[k].max(1e-30).ln(), sum[k + 1].max(1e-30).ln());
        let den = a - 2.0 * b + c;
        let d = if den < 0.0 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 };
        let sw: f64 = w.iter().sum();
        let sw2: f64 = w.iter().map(|x| x * x).sum();
        let capped: f64 = rows.iter().zip(&w).map(|((_, r), wh)| wh * r[k].min(CAP)).sum();
        let z = (capped - sw / LN_2) / (sw2.sqrt() / LN_2);
        let excess = z;
        let (mut harmonic, mut top, mut used) = (1, 0.0, 0u8);
        for (h, r) in &rows {
            if r[k] > top {
                top = r[k];
                harmonic = *h;
            }
            if r[k] * LN_2 >= 2.0 {
                used |= 1 << (h - 1);
            }
        }
        // Error: a peak's frequency error is about bin / (π √(2 SNR)); SNR here is the summed
        // signal power over the summed noise.
        let snr = ((sum[k] * LN_2 - sw) / sw).max(1e-3);
        let err_hz = (df / (std::f64::consts::PI * (2.0 * snr).sqrt())).clamp(0.0005, 0.02);
        Some(Peak { hz: f0 + (k as f64 + d) * step, score_db: 10.0 * z.max(1e-3).log10(), excess, harmonic, used, err_hz })
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
        // 50 or 60 Hz: the nominal whose capped excess is larger. Hum has energy at many
        // multiples; a fan or whine is usually one strong tone, which the cap holds down.
        let mut peaks = Vec::new();
        for &n in &nominals {
            if let Some(p) = self.combined(if n == 50.0 { 0 } else { 1 }, n) {
                peaks.push((n, p));
            }
        }
        let (nom, pk) = peaks.into_iter().max_by(|a, b| a.1.excess.total_cmp(&b.1.excess))?;
        let mut p = EnfPoint {
            t: (self.total as f64 - win as f64 / 2.0) / SR,
            hz: pk.hz,
            nominal: nom,
            harmonic: pk.harmonic,
            snr_db: pk.score_db,
            err_hz: pk.err_hz,
            used: pk.used,
            ok: false,
            reject: None,
        };
        if p.snr_db < SNR_MIN_DB {
            p.reject = Some(Reject::Weak);
        }
        if self.nominal.is_none() && p.snr_db >= SNR_MIN_DB {
            let i = if p.nominal == 50.0 { 0 } else { 1 };
            let (lt, lhz, len) = self.pre[i];
            let dt = p.t - lt;
            let len = if dt <= 2.0 + 1e-6 && (p.hz - lhz).abs() <= MAX_SLOPE_HZ_S * dt + 6.0 * p.err_hz { len + dt } else { 0.0 };
            self.pre[i] = (p.t, p.hz, len);
            if len < MIN_SECTION_S {
                return Some(p);
            }
            self.votes[i] += 1;
            let (a, b) = (self.votes[i], self.votes[1 - i]);
            if a >= 10 && a >= 3 * b {
                self.nominal = Some(p.nominal);
            }
        }
        Some(p)
    }

    /// Judge a reading and add it to the current run; close the run if the hum went away.
    /// A reading counts (`ok`) if it is clear (score), on the decided grid, and keeps to the
    /// slope rule against the median of the recent clear readings. Those recent readings
    /// include rejected ones, so a track that really moved is followed again after a few
    /// seconds. Others are kept for display, flagged with why, not used.
    fn track(&mut self, p: &mut EnfPoint) -> Option<EnfSegment> {
        let mut out = None;
        if let Some(run) = &self.run {
            let last = run.pts.last().map_or(p.t, |q| q.t);
            if p.t - last > MAX_GAP_S {
                out = self.close();
            }
        }
        if p.reject.is_none() && self.nominal != Some(p.nominal) {
            p.reject = Some(Reject::Undecided);
        }
        if p.reject.is_none() {
            if self.recent.len() >= 5 {
                let med = |v: &mut Vec<f64>| {
                    v.sort_by(f64::total_cmp);
                    v[v.len() / 2]
                };
                let hz = med(&mut self.recent.iter().map(|r| r.1).collect());
                let t = med(&mut self.recent.iter().map(|r| r.0).collect());
                let err = med(&mut self.recent.iter().map(|r| r.2).collect());
                if (p.hz - hz).abs() > MAX_SLOPE_HZ_S * (p.t - t).abs() + 3.0 * (p.err_hz + err) {
                    p.reject = Some(Reject::Slope);
                }
            }
            self.recent.push((p.t, p.hz, p.err_hz));
            if self.recent.len() > 9 {
                self.recent.remove(0);
            }
        }
        p.ok = p.reject.is_none();
        if p.ok {
            self.run.get_or_insert(Run { pts: Vec::new() }).pts.push(*p);
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

/// Section rule: readings marked `ok` that are not inside a run of at least MIN_SECTION_S of
/// `ok` readings (gaps up to 2 s bridged) lose `ok` and get `Reject::Section`.
pub fn sections(pts: &[EnfPoint]) -> Vec<EnfPoint> {
    let mut out = pts.to_vec();
    let mut i = 0;
    while i < out.len() {
        if !out[i].ok {
            i += 1;
            continue;
        }
        let mut j = i;
        while j + 1 < out.len() && out[j + 1].ok && out[j + 1].t - out[j].t <= 2.0 + 1e-6 {
            j += 1;
        }
        if out[j].t - out[i].t < MIN_SECTION_S {
            for p in &mut out[i..=j] {
                p.ok = false;
                p.reject = Some(Reject::Section);
            }
        }
        i = j + 1;
    }
    out
}

/// Shortest clean run, and largest typical error, for a recording to be matched against a grid
/// log ("good enough to date"). Placeholders until tuned on real recordings and grid logs.
pub const DATE_RUN_S: f64 = 180.0;
pub const DATE_ERR_HZ: f64 = 0.010;
/// Clean readings that vary less than this (standard deviation, Hz) over a minute or more look
/// like a steady tone (a fan, a whine), not a grid, which always wanders.
const STEADY_HZ: f64 = 0.0005;

/// How good the hum is, from best to worst.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// No reading counts.
    None,
    /// Some readings count (grid and hum track), too few or too rough to match a log.
    Weak,
    /// A clean run long and exact enough to match against a grid log.
    Good,
}

impl Level {
    pub fn word(self) -> &'static str {
        match self {
            Level::None => "none",
            Level::Weak => "weak",
            Level::Good => "good",
        }
    }
}

/// The signal quality verdict and the numbers it rests on, each with what it needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Quality {
    pub level: Level,
    /// Readings that count (after the section rule), of all readings.
    pub kept: usize,
    pub total: usize,
    /// Median score of the readings that count (of all readings when none counts), dB.
    pub score_db: f64,
    /// Harmonics that stood out in the readings that count.
    pub harmonics: Vec<u32>,
    /// Longest run of readings that count, s.
    pub longest_run_s: f64,
    /// Median error of the readings that count, Hz (NaN when none counts).
    pub err_hz: f64,
    /// Readings not used, per reason, in `Reject` order.
    pub rejected: Vec<(Reject, usize)>,
    /// The clean readings barely move: possibly a steady tone, not a grid.
    pub steady: bool,
}

impl Quality {
    /// One line: the verdict in words.
    pub fn verdict(&self) -> &'static str {
        match self.level {
            Level::Good => "good enough to date",
            Level::Weak => "weak: grid and hum track only",
            Level::None if self.total == 0 => "listening",
            Level::None => "no hum",
        }
    }

    /// The numbers, each with what it needs.
    pub fn lines(&self) -> Vec<String> {
        let mmss = |s: f64| format!("{}:{:02}", (s / 60.0) as u64, (s % 60.0) as u64);
        let mut v = vec![
            format!("score {:.1} dB (need {SNR_MIN_DB:.0})", self.score_db),
            format!("readings kept {} of {}", self.kept, self.total),
            format!("longest run {} (need {})", mmss(self.longest_run_s), mmss(DATE_RUN_S)),
        ];
        if self.err_hz.is_finite() {
            v.push(format!("error ±{:.1} mHz (need <{:.0})", self.err_hz * 1000.0, DATE_ERR_HZ * 1000.0));
        }
        if !self.harmonics.is_empty() {
            let h: Vec<String> = self.harmonics.iter().map(|h| h.to_string()).collect();
            v.push(format!("harmonics {}", h.join(", ")));
        }
        for (r, n) in &self.rejected {
            v.push(format!("{n} {}", r.label()));
        }
        if self.steady {
            v.push("very steady: may be a fan or tone, not the grid".into());
        }
        v
    }
}

/// The verdict over all readings so far.
pub fn quality(pts: &[EnfPoint]) -> Quality {
    let pts = sections(pts);
    let ok: Vec<&EnfPoint> = pts.iter().filter(|p| p.ok).collect();
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
    };
    let score_db = median(if ok.is_empty() { pts.iter().map(|p| p.snr_db).collect() } else { ok.iter().map(|p| p.snr_db).collect() });
    let err_hz = median(ok.iter().map(|p| p.err_hz).collect());
    // Longest run of counted readings (gaps up to 2 s bridged), and how much it moves.
    let (mut longest, mut steady) = (0.0f64, false);
    let mut i = 0;
    while i < ok.len() {
        let mut j = i;
        while j + 1 < ok.len() && ok[j + 1].t - ok[j].t <= 2.0 + 1e-6 {
            j += 1;
        }
        let span = ok[j].t - ok[i].t;
        longest = longest.max(span);
        if span >= 60.0 {
            let n = (j - i + 1) as f64;
            let mean = ok[i..=j].iter().map(|p| p.hz).sum::<f64>() / n;
            let sd = (ok[i..=j].iter().map(|p| (p.hz - mean).powi(2)).sum::<f64>() / n).sqrt();
            steady |= sd < STEADY_HZ;
        }
        i = j + 1;
    }
    // A harmonic counts if it stood out in at least half the counted readings (noise pokes
    // over 3 dB now and then in any band).
    let harmonics = (1..=HARMONICS).filter(|h| 2 * ok.iter().filter(|p| p.used & (1 << (h - 1)) != 0).count() >= ok.len().max(1)).collect();
    let rejected = [Reject::Weak, Reject::Undecided, Reject::Slope, Reject::Section]
        .into_iter()
        .map(|r| (r, pts.iter().filter(|p| p.reject == Some(r)).count()))
        .filter(|(_, n)| *n > 0)
        .collect();
    let level = if ok.is_empty() {
        Level::None
    } else if longest >= DATE_RUN_S && err_hz <= DATE_ERR_HZ && !steady {
        Level::Good
    } else {
        Level::Weak
    };
    Quality { level, kept: ok.len(), total: pts.len(), score_db, harmonics, longest_run_s: longest, err_hz, rejected, steady }
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
             zero-padded FFT {NFFT} points ({:.4} Hz bins); harmonics 1–{HARMONICS} of 50/60 Hz, each over its \
             noise, summed (combined spectrum) + parabolic peak; clear hum = score ≥ {SNR_MIN_DB} dB; slope rule \
             {:.1} mHz/s; findings = runs ≥ {MIN_SEG_S:.0} s",
            SR / NFFT as f64,
            MAX_SLOPE_HZ_S * 1000.0
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

    /// Hum at 50 Hz drifting ±20 mHz over a minute: harmonic k+1 at `amps[k]`, plus a steady
    /// 60 Hz tone (a fan) at `fan`, in white noise. 150 s.
    fn mix(amps: [f64; 5], fan: f64, noise: f32) -> (Vec<f32>, impl Fn(f64) -> f64) {
        let sr = INPUT_SR;
        let f = |t: f64| 50.0 + 0.02 * (2.0 * std::f64::consts::PI * t / 60.0).sin();
        let mut rng = Rng::new(3);
        let mut ph = 0.0f64;
        let x = (0..(150.0 * sr) as usize)
            .map(|i| {
                let t = i as f64 / sr;
                ph += 2.0 * std::f64::consts::PI * f(t) / sr;
                let mut v = fan * (2.0 * std::f64::consts::PI * 60.0 * t).sin();
                for (k, a) in amps.iter().enumerate() {
                    v += a * ((k + 1) as f64 * ph).sin();
                }
                v as f32 + noise * rng.normal() as f32
            })
            .collect();
        (x, f)
    }

    /// Errors (Hz) of the readings that count after the section rule, sorted.
    fn ok_errors(pts: &[EnfPoint], f: impl Fn(f64) -> f64) -> Vec<f64> {
        let mut e: Vec<f64> = sections(pts)
            .iter()
            .filter(|p| p.ok)
            .map(|p| (p.hz - (0..160).map(|k| f(p.t - 8.0 + k as f64 * 0.1)).sum::<f64>() / 160.0).abs())
            .collect();
        e.sort_by(f64::total_cmp);
        e
    }

    /// No fundamental, weak 2nd, 3rd and 5th harmonics.
    const WEAK: [f64; 5] = [0.0, 0.02, 0.015, 0.0, 0.01];

    /// Weak hum deep in noise (measured: 22 readings, median 4.0 mHz, worst 9.7 mHz; the
    /// detector before this one gave 33 at 6.5 mHz median, libhum 12 at 1.6 mHz).
    #[test]
    fn weak_hum_in_noise() {
        let (x, f) = mix(WEAK, 0.0, 2.0);
        let (m, pts, _) = run(&x);
        assert_eq!(m.nominal(), Some(50.0));
        let e = ok_errors(&pts, f);
        assert!(e.len() >= 15, "readings {}", e.len());
        assert!(e[e.len() / 2] < 0.005, "median {}", e[e.len() / 2]);
        assert!(e[e.len() - 1] < 0.012, "worst {}", e[e.len() - 1]);
    }

    /// Weaker still: nothing wrong is reported (the detector before this one gave readings
    /// ~108 mHz off here).
    #[test]
    fn weaker_hum_no_wrong_readings() {
        for noise in [3.0, 5.0] {
            let (x, f) = mix(WEAK, 0.0, noise);
            let (_, pts, found) = run(&x);
            let e = ok_errors(&pts, f);
            assert!(e.iter().all(|&v| v < 0.01), "noise {noise}: {e:?}");
            assert!(found.is_empty(), "noise {noise}");
        }
    }

    /// A steady 60 Hz fan louder than any one harmonic of a 50 Hz hum: the hum's grid wins.
    #[test]
    fn fan_does_not_win_over_hum() {
        let (x, f) = mix([0.02, 0.03, 0.01, 0.015, 0.01], 0.035, 0.3);
        let (m, pts, _) = run(&x);
        assert_eq!(m.nominal(), Some(50.0));
        let e = ok_errors(&pts, f);
        assert!(e.len() > 100 && e[e.len() / 2] < 0.002, "{} readings", e.len());
    }

    /// Every reading that doesn't count says why.
    #[test]
    fn rejected_readings_say_why() {
        let (x, _) = mix(WEAK, 0.0, 2.0);
        let (_, pts, _) = run(&x);
        for p in sections(&pts) {
            assert_eq!(p.ok, p.reject.is_none(), "{p:?}");
        }
        assert!(pts.iter().all(|p| p.err_hz >= 0.0005 && p.err_hz <= 0.02));
    }

    #[test]
    fn quality_levels() {
        // Clear hum for 4 minutes: good enough to date.
        let (x, _) = hum(50.0, 240.0, 0.1);
        let q = quality(&run(&x).1);
        assert_eq!(q.level, Level::Good, "{q:?}");
        assert!(q.longest_run_s >= DATE_RUN_S && q.err_hz < DATE_ERR_HZ && !q.steady);
        assert!(q.harmonics.contains(&2), "{q:?}");
        // Weak hum: counted, but too few to date.
        let (x, _) = mix(WEAK, 0.0, 2.0);
        let q = quality(&run(&x).1);
        assert_eq!(q.level, Level::Weak, "{q:?}");
        assert!(q.kept > 0 && q.kept < q.total);
        assert!(q.lines().iter().any(|l| l.contains("too weak")));
        // Noise: no hum, and the score shows how far off it is.
        let (x, _) = mix(WEAK, 0.0, 5.0);
        let q = quality(&run(&x).1);
        assert_eq!((q.level, q.kept, q.verdict()), (Level::None, 0, "no hum"));
        assert!(q.score_db < SNR_MIN_DB);
        // Nothing yet.
        assert_eq!(quality(&[]).verdict(), "listening");
    }

    /// A steady 60 Hz tone alone reads as hum (nothing else tells it apart), but the verdict
    /// says it barely moves and does not call it good enough to date.
    #[test]
    fn steady_tone_is_flagged() {
        let (x, _) = mix([0.0; 5], 0.035, 0.3);
        let q = quality(&run(&x).1);
        assert!(q.steady, "{q:?}");
        assert_ne!(q.level, Level::Good);
    }
}

