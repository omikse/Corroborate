//! Dating by the hum: the hum track matched against the grid log store (`grid_store`).
//!
//! For every whole-second start of the audio in a range (±SEARCH_S around a claimed time, or a
//! grid's whole log when nothing is claimed), the counted readings are compared with the log
//! smoothed over the detector's 16 s window: rms of the difference with its mean removed (a
//! recorder's clock error shifts the whole track a little). All starts at once by FFT
//! correlation: with the log L (0 where missing), its mask M and the readings r_j at t_j,
//!   k = Σ M[o+t_j],  S1 = Σ (r_j M − L)[o+t_j],  S2 = Σ (r_j² M − 2 r_j L + L²)[o+t_j]
//! are three correlations of the patterns (1, r, r²) with (M, L, L²); rms² = S2/k − (S1/k)².
//! Blocks of the log run in parallel. The best start counts when it is clear: error under
//! MAX_RMS_HZ and the runner-up (more than 2 min away, or another grid) MIN_MARGIN times worse.

use super::grid_store::{Store, BASE_HZ};
use super::{sections, EnfPoint, WIN_S};
use crate::timebase::fmt_utc;
use realfft::num_complex::Complex;
use realfft::RealFftPlanner;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// How far from a claimed start the match looks, s.
pub const SEARCH_S: f64 = 3600.0;
/// Fewest counted readings to try a match.
pub const MIN_READINGS: usize = 120;
/// The best start's error (rms, Hz) must be under this…
pub const MAX_RMS_HZ: f64 = 0.010;
/// …and the runner-up's at least this many times larger.
pub const MIN_MARGIN: f64 = 2.0;
/// Candidates closer than this (s) are the same match.
const APART_S: i64 = 120;

/// One possible start of the audio in one grid's log.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub grid: String,
    /// UTC start of the audio, s.
    pub start: i64,
    pub rms_hz: f64,
    /// The hum's mean offset from the log, Hz (recorder clock error).
    pub offset_hz: f64,
}

/// The counted readings as (whole seconds from the first, Hz − BASE_HZ), and the first's time.
fn readings(pts: &[EnfPoint]) -> (Vec<(usize, f64)>, i64) {
    let ok: Vec<EnfPoint> = sections(pts).into_iter().filter(|p| p.ok).collect();
    let t0 = ok.first().map_or(0, |p| p.t.round() as i64);
    (ok.iter().map(|p| ((p.t.round() as i64 - t0) as usize, p.hz - BASE_HZ)).collect(), t0)
}

/// The best few starts (at least APART_S apart) of the audio between `from` and `to` (UTC s) in
/// `grid`'s log, best first. `read(t, n)`: n seconds of log from UTC t, Hz, NaN where missing.
pub fn search(pts: &[EnfPoint], grid: &str, read: &(dyn Fn(i64, usize) -> Vec<f64> + Sync), from: i64, to: i64) -> Vec<Candidate> {
    let (rs, tf) = readings(pts);
    if rs.len() < MIN_READINGS || to < from {
        return Vec::new();
    }
    let p = rs.last().unwrap().0 + 1;
    let n = (4 * p).next_power_of_two().max(1 << 20);
    let step = n - p + 1;
    // x[i] is the smoothed log at UTC u0 + i; the start `from + o` puts reading j at x[o + t_j].
    let u0 = from + tf;
    let outputs = (to - from + 1) as usize;
    let blocks = outputs.div_ceil(step);
    let mut planner = RealFftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);
    // The patterns' spectra, conjugated (correlation).
    let spec = |vals: &dyn Fn(f64) -> f64| -> Vec<Complex<f64>> {
        let mut x = vec![0.0; n];
        for &(t, r) in &rs {
            x[t] = vals(r);
        }
        let mut s = fwd.make_output_vec();
        fwd.process(&mut x, &mut s).unwrap();
        s.iter().map(|c| c.conj()).collect()
    };
    let (pa, pr, pr2) = (spec(&|_| 1.0), spec(&|r| r), spec(&|r| r * r));
    let need = (rs.len() * 9).div_ceil(10) as f64;
    let w = WIN_S as usize;
    let next = AtomicUsize::new(0);
    let found = Mutex::new(Vec::<(i64, f64, f64)>::new());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(blocks.max(1));
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                let (mut m, mut l, mut l2) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
                let (mut fm, mut fl, mut fl2) = (fwd.make_output_vec(), fwd.make_output_vec(), fwd.make_output_vec());
                let (mut k, mut s1, mut s2) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
                let mut local = Vec::new();
                loop {
                    let b = next.fetch_add(1, Ordering::Relaxed);
                    if b >= blocks {
                        break;
                    }
                    let o0 = b * step;
                    // Raw log from 8 s before x[0] (the window is [τ−8, τ+8)).
                    let raw = read(u0 + o0 as i64 - (w / 2) as i64, n + w);
                    let (mut sum, mut bad) = (0.0, 0usize);
                    for v in &raw[..w] {
                        if v.is_finite() { sum += v - BASE_HZ } else { bad += 1 }
                    }
                    let mut any = false;
                    for i in 0..n {
                        let ok = bad == 0;
                        let s = if ok { sum / w as f64 } else { 0.0 };
                        (m[i], l[i], l2[i]) = (if ok { 1.0 } else { 0.0 }, s, s * s);
                        any |= ok;
                        let (out, inn) = (raw[i], raw[i + w]);
                        if out.is_finite() { sum -= out - BASE_HZ } else { bad -= 1 }
                        if inn.is_finite() { sum += inn - BASE_HZ } else { bad += 1 }
                    }
                    if !any {
                        continue;
                    }
                    fwd.process(&mut m, &mut fm).unwrap();
                    fwd.process(&mut l, &mut fl).unwrap();
                    fwd.process(&mut l2, &mut fl2).unwrap();
                    let mut ck: Vec<Complex<f64>> = fm.iter().zip(&pa).map(|(a, b)| a * b).collect();
                    let mut c1: Vec<Complex<f64>> = (0..fm.len()).map(|i| fm[i] * pr[i] - fl[i] * pa[i]).collect();
                    let mut c2: Vec<Complex<f64>> = (0..fm.len()).map(|i| fm[i] * pr2[i] - 2.0 * fl[i] * pr[i] + fl2[i] * pa[i]).collect();
                    // DC and Nyquist bins of a real signal's spectrum are real.
                    for c in [&mut ck, &mut c1, &mut c2] {
                        c[0].im = 0.0;
                        c.last_mut().unwrap().im = 0.0;
                    }
                    inv.process(&mut ck, &mut k).unwrap();
                    inv.process(&mut c1, &mut s1).unwrap();
                    inv.process(&mut c2, &mut s2).unwrap();
                    let nn = n as f64;
                    let mut rms: Vec<f64> = (0..step.min(outputs - o0))
                        .map(|o| {
                            let kk = (k[o] / nn).round();
                            if kk < need {
                                return f64::INFINITY;
                            }
                            let mean = s1[o] / nn / kk;
                            (s2[o] / nn / kk - mean * mean).max(0.0).sqrt()
                        })
                        .collect();
                    // The block's best few, apart.
                    for _ in 0..5 {
                        let Some((o, &v)) = rms.iter().enumerate().min_by(|a, b| a.1.total_cmp(b.1)) else { break };
                        if !v.is_finite() {
                            break;
                        }
                        local.push(((o0 + o) as i64, v, s1[o] / nn / (k[o] / nn).round()));
                        let (a, e) = (o.saturating_sub(APART_S as usize), (o + APART_S as usize + 1).min(rms.len()));
                        rms[a..e].fill(f64::INFINITY);
                    }
                }
                found.lock().unwrap().extend(local);
            });
        }
    });
    let mut all = found.into_inner().unwrap();
    all.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut best: Vec<(i64, f64, f64)> = Vec::new();
    for c in all {
        if best.iter().all(|b| (b.0 - c.0).abs() > APART_S) {
            best.push(c);
            if best.len() == 5 {
                break;
            }
        }
    }
    best.into_iter().map(|(o, rms, mean)| Candidate { grid: grid.into(), start: from + o, rms_hz: rms, offset_hz: mean }).collect()
}

/// The result of matching the hum against the store.
#[derive(Clone, Debug, PartialEq)]
pub struct Dated {
    /// The claimed start, when a time was wired (UTC s).
    pub claimed: Option<f64>,
    pub best: Candidate,
    pub runner_up: Option<Candidate>,
    /// Grids and the span searched, in words.
    pub searched: String,
    pub readings: usize,
    pub confirmed: bool,
}

impl Dated {
    pub fn start(&self) -> f64 {
        self.best.start as f64
    }

    pub fn margin(&self) -> f64 {
        self.runner_up.as_ref().map_or(f64::INFINITY, |r| r.rms_hz / self.best.rms_hz)
    }

    /// The verdict in a few words.
    pub fn verdict(&self) -> String {
        match (self.claimed, self.confirmed) {
            (Some(c), true) => format!("time confirmed ({:+.0} s)", self.start() - c),
            (Some(_), false) => "time not confirmed".into(),
            (None, true) => format!("dated {} UTC ({})", fmt_utc(self.start()).replace('T', " ").trim_end_matches(".000Z"), self.best.grid),
            (None, false) => "no clear match".into(),
        }
    }

    pub fn lines(&self) -> Vec<String> {
        let mut v = vec![format!("searched {}", self.searched)];
        if let Some(c) = self.claimed {
            v.push(format!("claimed {}", fmt_utc(c)));
        }
        v.push(format!("found {} ({})", fmt_utc(self.start()), self.best.grid));
        v.push(format!("error {:.2} mHz (need <{:.0})", self.best.rms_hz * 1000.0, MAX_RMS_HZ * 1000.0));
        if let Some(r) = &self.runner_up {
            v.push(format!("next best {:.2} mHz, ×{:.1} (need ×{MIN_MARGIN:.0}): {} {}", r.rms_hz * 1000.0, self.margin(), fmt_utc(r.start as f64), r.grid));
        }
        v.push(format!("{} readings, offset {:+.1} mHz", self.readings, self.best.offset_hz * 1000.0));
        v
    }

    pub fn to_json(&self) -> String {
        let c = |c: &Candidate| format!("{{\"grid\":\"{}\",\"start_utc\":\"{}\",\"rms_hz\":{:.6},\"offset_hz\":{:.6}}}", c.grid, fmt_utc(c.start as f64), c.rms_hz, c.offset_hz);
        format!(
            "{{\"claimed_utc\":{},\"best\":{},\"runner_up\":{},\"searched\":\"{}\",\"readings\":{},\"confirmed\":{}}}",
            self.claimed.map_or("null".into(), |t| format!("\"{}\"", fmt_utc(t))),
            c(&self.best),
            self.runner_up.as_ref().map_or("null".into(), c),
            crate::timebase::json_escape(&self.searched),
            self.readings,
            self.confirmed
        )
    }
}

/// Match against `grids` of the store: around `claimed` (± SEARCH_S) when given, else each
/// grid's whole log. `tolerance_s`: how far from the claim the best start may be and confirm it.
pub fn date(pts: &[EnfPoint], store: &Store, grids: &[String], claimed: Option<(f64, f64)>) -> Result<Dated, String> {
    let n = readings(pts).0.len();
    if n < MIN_READINGS {
        return Err(format!("{n} readings count, need {MIN_READINGS} to match"));
    }
    if grids.is_empty() {
        return Err("no grid log in the store for this place and frequency".into());
    }
    let mut cands = Vec::new();
    let mut spans = Vec::new();
    for g in grids {
        let months = store.months(g);
        let (Some(first), Some(last)) = (months.first(), months.last()) else { continue };
        let (lo, hi) = (first.t0, last.t0 + last.n as i64);
        let (from, to) = match claimed {
            Some((c, _)) => ((c - SEARCH_S) as i64, (c + SEARCH_S) as i64),
            None => (lo, hi),
        };
        let (from, to) = (from.max(lo - 86_400), to.min(hi));
        if from > to {
            continue;
        }
        spans.push(format!("{g} {}–{}", &fmt_utc(from as f64)[..10], &fmt_utc(to as f64)[..10]));
        cands.extend(search(pts, g, &|t, len| store.read(g, t, len), from, to));
    }
    cands.sort_by(|a, b| a.rms_hz.total_cmp(&b.rms_hz));
    let best = cands.first().cloned().ok_or(if spans.is_empty() { "the store's logs do not cover the time".to_string() } else { "no start had enough log under the readings".to_string() })?;
    let runner_up = cands.iter().find(|c| c.grid != best.grid || (c.start - best.start).abs() > APART_S).cloned();
    let margin = runner_up.as_ref().map_or(f64::INFINITY, |r| r.rms_hz / best.rms_hz);
    let near = claimed.is_none_or(|(c, err)| (best.start as f64 - c).abs() <= err.max(10.0));
    let confirmed = best.rms_hz < MAX_RMS_HZ && margin >= MIN_MARGIN && near;
    Ok(Dated { claimed: claimed.map(|c| c.0), best, runner_up, searched: spans.join(", "), readings: n, confirmed })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A random-walk log and a hum track cut from it (with a constant offset and noise).
    fn case() -> (Vec<f64>, Vec<EnfPoint>, usize) {
        let mut seed = 5u64;
        let mut u = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 11) as f64 / (1u64 << 53) as f64) - 0.5
        };
        let mut f = 50.0;
        let mut raw: Vec<f64> = (0..3_000_000)
            .map(|_| {
                f += 0.002 * u() - 0.0002 * (f - 50.0);
                f
            })
            .collect();
        // A gap in the log.
        raw[200_000..260_000].fill(f64::NAN);
        let at = 2_345_678;
        let pts = (8..1500)
            .map(|t| {
                let i = at + t;
                let hz = raw[i - 8..i + 8].iter().sum::<f64>() / 16.0 + 0.003 + 0.0005 * u();
                EnfPoint { t: t as f64, hz, nominal: 50.0, harmonic: 2, snr_db: 20.0, err_hz: 0.001, used: 2, ok: true, reject: None }
            })
            .collect();
        (raw, pts, at)
    }

    fn reader(raw: &[f64], t0: i64) -> impl Fn(i64, usize) -> Vec<f64> + Sync + '_ {
        move |t, n| (0..n).map(|k| raw.get((t - t0) as usize + k).copied().filter(|_| t - t0 + k as i64 >= 0).unwrap_or(f64::NAN)).collect()
    }

    /// Found at its true start among 3 million, against brute force at the same starts.
    #[test]
    fn finds_the_start_blind() {
        let (raw, pts, at) = case();
        let t0 = 1_000_000_000;
        let c = search(&pts, "T", &reader(&raw, t0), t0, t0 + raw.len() as i64 - 2000);
        assert_eq!(c[0].start, t0 + at as i64, "{c:?}");
        assert!(c[0].rms_hz < 0.0006 && c[1].rms_hz > 5.0 * c[0].rms_hz, "{c:?}");
        assert!((c[0].offset_hz - 0.003).abs() < 0.0002, "{c:?}");
        // Brute force at the runner-up agrees with the FFT.
        let (rs, tf) = readings(&pts);
        let o = (c[1].start - t0 + tf) as usize;
        let d: Vec<f64> = rs.iter().map(|&(t, r)| r - (raw[o + t - 8..o + t + 8].iter().sum::<f64>() / 16.0 - 50.0)).collect();
        let mean = d.iter().sum::<f64>() / d.len() as f64;
        let rms = (d.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / d.len() as f64).sqrt();
        assert!((rms - c[1].rms_hz).abs() < 1e-6, "{rms} vs {}", c[1].rms_hz);
    }
}
