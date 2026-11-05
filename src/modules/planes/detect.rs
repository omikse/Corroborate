//! Streaming detector: whitened frames → rolling window → Viterbi ridges → Doppler fit → gate.
//!
//! Every `step_s` the last `window_s` of frames is searched. Up to `max_paths` ridges are
//! peeled off (each found path is masked before looking for the next). Along each ridge, runs
//! of bright frames long enough to hold a pass are fitted with the straight-pass model and
//! kept only if the fit is good and physical. The same pass found again in a later window, or
//! on one of its harmonics, is reported once.

use crate::modules::planes::fit::{self, model};
use crate::modules::planes::ridge::{subbin, Viterbi};
use crate::modules::planes::spectro::Frontend;

#[derive(Clone, Debug)]
pub struct Config {
    pub sr: f64,
    pub nfft: usize,
    pub hop: usize,
    pub fmin: f64,
    pub fmax: f64,
    pub window_s: f64,
    pub step_s: f64,
    pub warmup_s: f64,
    /// Max ridge jump per frame (bins) and penalty per bin jumped.
    pub max_jump: usize,
    pub lambda: f32,
    pub max_paths: usize,
    /// Smoothed ridge brightness (z) a run must stay above, and gaps it may bridge.
    pub run_z: f32,
    pub gap_s: f64,
    pub min_dur_s: f64,
    /// Seconds of ridge required on each side of the closest approach.
    pub min_side_s: f64,
    /// Mean ridge z over the run (the SNR gate).
    pub snr_min: f64,
    /// Max RMS misfit (Hz) between ridge and fitted curve.
    pub rms_max: f64,
    /// Points within this many Hz of the curve count as following it.
    pub inlier_hz: f64,
    /// Seconds a candidate is held before it is reported, so a better fit can replace it.
    pub hold_s: f64,
    /// Fraction of frames within ±(d/v + 2 s) of closest approach that must be on the curve.
    pub cpa_cover: f64,
    /// Minimum d/v (s): shorter transitions are unresolvable at this frame rate.
    pub min_transition_s: f64,
    pub v_min: f64,
    pub v_max: f64,
    pub d_min: f64,
    pub d_max: f64,
    pub c: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sr: 4000.0,
            nfft: 8192,
            hop: 1000,
            fmin: 40.0,
            fmax: 450.0,
            window_s: 160.0,
            step_s: 5.0,
            warmup_s: 60.0,
            max_jump: 4,
            lambda: 0.3,
            max_paths: 3,
            run_z: 1.8,
            gap_s: 3.0,
            min_dur_s: 30.0,
            min_side_s: 10.0,
            snr_min: 2.0,
            rms_max: 1.0,
            inlier_hz: 1.5,
            hold_s: 10.0,
            cpa_cover: 0.4,
            min_transition_s: 4.0,
            v_min: 25.0,
            v_max: 150.0,
            d_min: 30.0,
            d_max: 15_000.0,
            c: fit::C_SOUND,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Event {
    pub id: u64,
    /// Closest-approach time, start and end of the fitted ridge (s since stream start).
    pub t_cpa: f64,
    pub t_start: f64,
    pub t_end: f64,
    pub f0: f64,
    pub f_pre: f64,
    pub f_post: f64,
    pub speed: f64,
    pub dist: f64,
    pub snr: f64,
    pub rms: f64,
    /// How many of the 0.5×, 2×, 3× partials of the fitted curve are visible.
    pub harmonics: u32,
    pub n_points: usize,
    /// Ridge points (t, f) used in the fit.
    pub points: Vec<(f64, f64)>,
}

impl Event {
    /// Confidence grade. Provisional: set from one real recording, where every curve
    /// confirmed by eye had snr ≥ 2.8 and rms ≤ 0.5 Hz, and the unconvincing ones were
    /// weaker and noisier.
    pub fn grade(&self) -> &'static str {
        if self.snr >= 2.8 && self.rms <= 0.5 {
            "high"
        } else if (self.snr >= 2.3 && self.rms <= 0.5) || (self.snr >= 2.8 && self.rms <= 0.8) {
            "medium"
        } else {
            "low"
        }
    }

    /// One JSON object, no trailing newline. Times are relative (seconds from the first
    /// sample); with an anchor, the closest approach is also given in UTC (Coordinated
    /// Universal Time) with the anchor's source.
    pub fn to_json(&self, anchor: Option<&crate::timebase::Anchor>) -> String {
        let mut s = format!(
            "{{\"id\":{},\"grade\":\"{}\",\"t_cpa\":{:.2},\"t_start\":{:.2},\"t_end\":{:.2},\"f0_hz\":{:.2},\
             \"f_pre_hz\":{:.2},\"f_post_hz\":{:.2},\"speed_mps\":{:.1},\"cpa_dist_m\":{:.0},\
             \"snr_z\":{:.2},\"rms_hz\":{:.3},\"harmonics\":{},\"n_points\":{}",
            self.id,
            self.grade(),
            self.t_cpa,
            self.t_start,
            self.t_end,
            self.f0,
            self.f_pre,
            self.f_post,
            self.speed,
            self.dist,
            self.snr,
            self.rms,
            self.harmonics,
            self.n_points
        );
        match anchor {
            Some(a) => s.push_str(&format!(
                ",\"utc_cpa\":\"{}\",\"utc_source\":\"{}\"",
                crate::timebase::fmt_utc(a.utc(self.t_cpa)),
                crate::timebase::json_escape(&a.source)
            )),
            None => s.push_str(",\"utc_cpa\":null,\"utc_source\":\"none (relative time only)\""),
        }
        s.push('}');
        s
    }
}

pub struct Detector {
    pub cfg: Config,
    fe: Frontend,
    nb: usize,
    wframes: usize,
    step_frames: usize,
    ring: Vec<f32>,
    times: Vec<f64>,
    head: usize,
    filled: usize,
    since: usize,
    frame: Vec<f32>,
    win: Vec<f32>,
    work: Vec<f32>,
    wtimes: Vec<f64>,
    vit: Viterbi,
    path: Vec<usize>,
    /// Candidates held for `hold_s` so a better version of the same pass can replace them,
    /// with the window-end time they were first seen.
    pending: Vec<(Event, f64)>,
    /// Recently reported passes, to suppress repeats, harmonics and tails.
    recent: Vec<Event>,
    next_id: u64,
    /// Bright ridge runs (≥ 5 s) from the latest search, as (t, Hz), for display.
    ridges: Vec<Vec<(f64, f64)>>,
}

impl Detector {
    pub fn new(cfg: Config) -> Self {
        let fe = Frontend::new(cfg.sr, cfg.nfft, cfg.hop, cfg.fmin, cfg.fmax);
        let nb = fe.nb;
        let wframes = (cfg.window_s * fe.fps()).round() as usize;
        let step_frames = ((cfg.step_s * fe.fps()).round() as usize).max(1);
        Self {
            fe,
            nb,
            wframes,
            step_frames,
            ring: vec![0.0; wframes * nb],
            times: vec![0.0; wframes],
            head: 0,
            filled: 0,
            since: 0,
            frame: vec![0.0; nb],
            win: vec![0.0; wframes * nb],
            work: vec![0.0; wframes * nb],
            wtimes: vec![0.0; wframes],
            vit: Viterbi::new(wframes, nb),
            path: Vec::with_capacity(wframes),
            pending: Vec::new(),
            recent: Vec::new(),
            next_id: 1,
            ridges: Vec::new(),
            cfg,
        }
    }

    /// Ridge runs found by the most recent search (for display).
    pub fn ridges(&self) -> &[Vec<(f64, f64)>] {
        &self.ridges
    }

    /// Frequency of each bin in a frame handed to `push_with`.
    pub fn bin_freqs(&self) -> Vec<f64> {
        (0..self.nb).map(|i| self.fe.bin_hz(i as f64)).collect()
    }

    /// Candidates being held before they are reported (for display).
    pub fn pending(&self) -> impl Iterator<Item = &Event> {
        self.pending.iter().map(|(e, _)| e)
    }

    /// Feed samples (mono, `cfg.sr`). Returns any newly confirmed passes.
    pub fn push(&mut self, x: &[f32]) -> Vec<Event> {
        self.push_with(x, |_, _| {})
    }

    /// As `push`, also handing each whitened frame (time, z per bin) to `on_frame`.
    pub fn push_with(&mut self, x: &[f32], mut on_frame: impl FnMut(f64, &[f32])) -> Vec<Event> {
        self.fe.feed(x);
        let mut out = Vec::new();
        while let Some(t) = self.fe.next_frame(&mut self.frame) {
            on_frame(t, &self.frame);
            let h = self.head;
            self.ring[h * self.nb..(h + 1) * self.nb].copy_from_slice(&self.frame);
            self.times[h] = t;
            self.head = (h + 1) % self.wframes;
            self.filled = (self.filled + 1).min(self.wframes);
            self.since += 1;
            if self.since >= self.step_frames && t >= self.cfg.warmup_s {
                self.since = 0;
                out.extend(self.analyze());
            }
        }
        out
    }

    /// Search the current window once more (end of a file).
    pub fn flush(&mut self) -> Vec<Event> {
        if self.filled == 0 {
            return Vec::new();
        }
        let mut out = self.analyze();
        out.extend(self.release(f64::INFINITY, true));
        out
    }

    fn analyze(&mut self) -> Vec<Event> {
        let (nb, t) = (self.nb, self.filled);
        let start = (self.head + self.wframes - t) % self.wframes;
        for k in 0..t {
            let src = (start + k) % self.wframes;
            self.win[k * nb..(k + 1) * nb].copy_from_slice(&self.ring[src * nb..(src + 1) * nb]);
            self.wtimes[k] = self.times[src];
        }
        self.work[..t * nb].copy_from_slice(&self.win[..t * nb]);

        let fps = self.fe.fps();
        let smooth_n = ((5.0 * fps) as usize) | 1;
        let gap = (self.cfg.gap_s * fps) as usize;
        let min_len = (self.cfg.min_dur_s * fps) as usize;
        let mut found = Vec::new();
        self.ridges.clear();

        for _ in 0..self.cfg.max_paths {
            self.vit.run(&self.work[..t * nb], t, nb, self.cfg.max_jump, self.cfg.lambda, &mut self.path);
            let pz: Vec<f32> = (0..t).map(|k| self.win[k * nb + self.path[k]]).collect();
            let sm = moving_avg(&pz, smooth_n);

            for (a, b) in runs(&sm, self.cfg.run_z, gap) {
                if (b - a) as f64 >= 5.0 * fps {
                    let r = (a..b)
                        .map(|k| (self.wtimes[k], self.fe.bin_hz(self.path[k] as f64)))
                        .collect();
                    self.ridges.push(r);
                }
                if debug() {
                    eprintln!(
                        "[dbg] win end {:.0}s run {:.0}-{:.0}s len {:.0}s",
                        self.wtimes[t - 1],
                        self.wtimes[a],
                        self.wtimes[b - 1],
                        (b - a) as f64 / fps
                    );
                }
                if b - a >= min_len {
                    if let Some(ev) = self.try_event(a, b, &pz) {
                        found.push(ev);
                    }
                }
            }
            // Peel this ridge off before looking for the next one.
            for k in 0..t {
                let p = self.path[k];
                let lo = p.saturating_sub(3);
                let hi = (p + 3).min(nb - 1);
                for v in &mut self.work[k * nb + lo..=k * nb + hi] {
                    *v = -2.0;
                }
            }
        }

        let now = self.wtimes[t - 1];
        self.merge(found, now);
        self.release(now, false)
    }

    /// Fold new candidates into the pending set. Best first, so a fundamental found in the same
    /// window as its harmonic wins.
    fn merge(&mut self, mut found: Vec<Event>, now: f64) {
        found.sort_by(|a, b| quality(b).total_cmp(&quality(a)));
        let tol = self.cfg.inlier_hz;
        for ev in found {
            if self.recent.iter().any(|r| same_pass(&ev, r, tol, self.cfg.c)) {
                continue;
            }
            match self.pending.iter_mut().find(|(p, _)| same_pass(&ev, p, tol, self.cfg.c)) {
                Some((p, _)) => {
                    if quality(&ev) > quality(p) {
                        *p = ev;
                    }
                }
                None => self.pending.push((ev, now)),
            }
        }
    }

    /// Report pending passes held long enough (or all of them at end of input).
    fn release(&mut self, now: f64, all: bool) -> Vec<Event> {
        let hold = self.cfg.hold_s;
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            if all || now - self.pending[i].1 >= hold {
                let (mut ev, _) = self.pending.remove(i);
                ev.id = self.next_id;
                self.next_id += 1;
                self.recent.push(ev.clone());
                out.push(ev);
            } else {
                i += 1;
            }
        }
        if self.recent.len() > 64 {
            self.recent.drain(..32);
        }
        out.sort_by(|a, b| a.t_cpa.total_cmp(&b.t_cpa));
        out
    }

    fn try_event(&self, a: usize, b: usize, pz: &[f32]) -> Option<Event> {
        let nb = self.nb;
        let snr = pz[a..b].iter().map(|&z| z as f64).sum::<f64>() / (b - a) as f64;
        if debug() {
            eprintln!("[dbg]   run snr {snr:.2}");
        }
        if snr < self.cfg.snr_min {
            return None;
        }
        let mut ts = Vec::with_capacity(b - a);
        let mut fs = Vec::with_capacity(b - a);
        for k in a..b {
            if pz[k] > 1.0 {
                let row = &self.win[k * nb..(k + 1) * nb];
                ts.push(self.wtimes[k]);
                fs.push(self.fe.bin_hz(subbin(row, self.path[k])));
            }
        }
        if ts.len() < 40 {
            return None;
        }
        let (f, idx) = fit::robust_fit(&ts, &fs, self.cfg.c, self.cfg.inlier_hz)?;
        // At least half the ridge must follow the curve.
        if idx.len() < 40 || idx.len() * 2 < ts.len() {
            return None;
        }
        let ts: Vec<f64> = idx.iter().map(|&i| ts[i]).collect();
        let fs: Vec<f64> = idx.iter().map(|&i| fs[i]).collect();
        let [f0, v, d, tc] = f.p;
        let (t0, t1) = (ts[0], ts[ts.len() - 1]);
        let c = &self.cfg;
        let sweep = f.f_pre - f.f_post;
        if debug() {
            eprintln!(
                "[dbg]   snr {snr:.2} n {} fit f0 {f0:.1} v {v:.1} d {d:.0} tc {tc:.1} rms {:.3} span {t0:.0}-{t1:.0}",
                ts.len(),
                f.rms
            );
        }
        if f.rms > c.rms_max
            || f.rms > 0.15 * sweep
            || !(c.v_min..=c.v_max).contains(&v)
            || !(c.d_min..=c.d_max).contains(&d)
            || tc - t0 < c.min_side_s
            || t1 - tc < c.min_side_s
        {
            return None;
        }
        // The closest approach itself must be seen: the transition lasts about ±d/v, and a
        // real pass is loudest there. A fit that bridges two unrelated tones across a gap
        // puts a sharp, unobserved step in that gap.
        let half = d / v + 2.0;
        let seen = ts.iter().filter(|&&t| (t - tc).abs() <= half).count() as f64;
        let lo = (tc - half).max(self.wtimes[0]);
        let hi = (tc + half).min(self.wtimes[self.filled - 1]);
        let expected = ((hi - lo) * self.fe.fps()).max(1.0);
        if debug() {
            eprintln!("[dbg]   cpa coverage {:.2} over ±{half:.1}s", seen / expected);
        }
        if seen < c.cpa_cover * expected {
            return None;
        }
        // A transition shorter than the analysis window cannot be told from a jump between
        // two tones (an engine changing gear, the ridge hopping lines).
        if d / v < c.min_transition_s {
            if debug() {
                eprintln!("[dbg]   reject: transition d/v {:.1}s", d / v);
            }
            return None;
        }
        // Doppler falls smoothly through the transition; engine RPM changes step and plateau.
        if !falls_smoothly(&ts, &fs, &f.p, c.c) {
            if debug() {
                eprintln!("[dbg]   reject: not a smooth fall");
            }
            return None;
        }

        // Partials of the fitted curve that are also bright.
        let mut harmonics = 0;
        for h in [0.5, 2.0, 3.0] {
            let mut sum = 0.0;
            let mut n = 0usize;
            for k in a..b {
                let fb = self.fe.hz_bin(h * model(self.wtimes[k], &f.p, c.c)).round();
                if fb >= 1.0 && fb < (nb - 1) as f64 {
                    let i = k * nb + fb as usize;
                    sum += self.win[i - 1].max(self.win[i]).max(self.win[i + 1]) as f64;
                    n += 1;
                }
            }
            if n * 2 >= b - a && sum / n as f64 > 1.5 {
                harmonics += 1;
            }
        }

        Some(Event {
            id: 0,
            t_cpa: tc,
            t_start: t0,
            t_end: t1,
            f0,
            f_pre: f.f_pre,
            f_post: f.f_post,
            speed: v,
            dist: d,
            snr,
            rms: f.rms,
            harmonics,
            n_points: f.n,
            points: ts.into_iter().zip(fs).collect(),
        })
    }
}

fn debug() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| std::env::var_os("DOPPLER_DEBUG").is_some())
}

/// Over the transition (tc ± max(d/v, 3 s)), take the median ridge frequency in 2 s steps.
/// Each step must fall by at least 30% of the drop the model predicts for it; at least 80%
/// of the steps (and at least 3) must do so.
fn falls_smoothly(ts: &[f64], fs: &[f64], p: &[f64; 4], c: f64) -> bool {
    let [_, v, d, tc] = *p;
    let half = (d / v).max(3.0);
    let step = 2.0;
    let nsteps = (2.0 * half / step).floor() as usize;
    let mut med: Vec<Option<(f64, f64)>> = Vec::with_capacity(nsteps);
    let mut buf = Vec::new();
    for k in 0..nsteps {
        let a = tc - half + k as f64 * step;
        buf.clear();
        buf.extend(ts.iter().zip(fs).filter(|p| *p.0 >= a && *p.0 < a + step).map(|p| *p.1));
        if buf.len() < 2 {
            med.push(None);
            continue;
        }
        buf.sort_by(f64::total_cmp);
        med.push(Some((a + 0.5 * step, buf[buf.len() / 2])));
    }
    let (mut ok, mut total) = (0usize, 0usize);
    for w in med.windows(2) {
        let (Some((t1, f1)), Some((t2, f2))) = (w[0], w[1]) else { continue };
        let predicted = model(t1, p, c) - model(t2, p, c);
        total += 1;
        if f1 - f2 >= 0.3 * predicted {
            ok += 1;
        }
    }
    total >= 3 && ok as f64 >= 0.8 * total as f64
}

fn quality(e: &Event) -> f64 {
    e.snr * (e.n_points as f64).sqrt()
}

/// Fraction of `a`'s ridge points lying on `b`'s fitted curve or one of its partials
/// (b·h for h in ½, 1, 3/2, 2, 3; tolerance scaled with h).
fn overlap(a: &Event, b: &Event, tol: f64, c: f64) -> f64 {
    let p = [b.f0, b.speed, b.dist, b.t_cpa];
    let hit = a
        .points
        .iter()
        .filter(|&&(t, f)| {
            let m = model(t, &p, c);
            [0.5, 1.0, 1.5, 2.0, 3.0].iter().any(|&h| (h * m - f).abs() <= tol * h.max(1.0))
        })
        .count();
    hit as f64 / a.points.len().max(1) as f64
}

/// Same pass: the two overlap in time and most of either one's ridge lies on the other's
/// curve family. Catches a pass seen again in a later window, on a harmonic, or a tail
/// fitted on its own.
fn same_pass(a: &Event, b: &Event, tol: f64, c: f64) -> bool {
    if a.t_end < b.t_start - 30.0 || b.t_end < a.t_start - 30.0 {
        return false;
    }
    // Real tracks bend (turns, climbs), so curves of one pass drift apart by more than the
    // fit tolerance away from the closest approach: compare with a looser tolerance.
    let tol = 2.0 * tol;
    overlap(a, b, tol, c) >= 0.4 || overlap(b, a, tol, c) >= 0.4
}

fn moving_avg(x: &[f32], n: usize) -> Vec<f32> {
    let h = n / 2;
    let mut pre = vec![0.0f64; x.len() + 1];
    for (i, &v) in x.iter().enumerate() {
        pre[i + 1] = pre[i] + v as f64;
    }
    (0..x.len())
        .map(|i| {
            let a = i.saturating_sub(h);
            let b = (i + h + 1).min(x.len());
            ((pre[b] - pre[a]) / (b - a) as f64) as f32
        })
        .collect()
}

/// Half-open index ranges where `x > thr`, bridging gaps of at most `gap` samples.
fn runs(x: &[f32], thr: f32, gap: usize) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < x.len() {
        if x[i] > thr {
            let a = i;
            while i < x.len() && x[i] > thr {
                i += 1;
            }
            match out.last_mut() {
                Some(last) if a - last.1 <= gap => last.1 = i,
                _ => out.push((a, i)),
            }
        } else {
            i += 1;
        }
    }
    out
}
