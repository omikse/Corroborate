//! Audio match: finds where recordings of one event hear the same sound, and turns that into
//! links between their clocks (moment `t_a` on one is moment `t_b` on the other) for the clock
//! solve (`clock`, `sync`), which learns offsets and drift from them.
//!
//! Method, after AudioAlign (Guggenberger, Lux, Böszörmenyi): fingerprints find which pairs
//! overlap and roughly where, cross-correlation refines each match to a fraction of a sample.
//! 1. Each clip (8 kHz mono) is fingerprinted with Wang's landmarks (the Shazam method: pairs of
//!    spectrogram peaks hashed with their frequency and time gap; audiofp's `wang-v1`, 62.5
//!    frames per second).
//! 2. Every `CHUNK_S` of clip b is matched against all of clip a: matching hashes vote for one
//!    time offset, and a clear peak (`MIN_VOTES`, `MIN_PROMINENCE`) is a candidate. Chunks
//!    rather than whole clips because drift smears a whole clip's votes: 300 ppm over 1000 s is
//!    0.3 s, 19 frames.
//! 3. Each candidate is refined with GCC-PHAT (generalized cross-correlation with phase
//!    transform): `WIN_S` of b around the chunk's middle within a's audio at the candidate
//!    offset ± `SEARCH_S`. The peak must stand `MIN_PSR` (peak-to-sidelobe ratio) above the other
//!    lags and agree with the fingerprints within `AGREE_S`; the refined lag gives the link,
//!    interpolated between samples, with an error from its PSR.
//!
//! Tested on the Jiku Mobile Video Dataset (tests/jiku.rs, event GT_090912: 50 phone recordings
//! of one event, with a hand-made sync ground truth by Guggenberger, Lux, Böszörmenyi, MMM 2015):
//! - The ground truth maps moment t of a recording to offset + t · speed on the event's timeline,
//!   speed = 1 − the drift measured in the lab for the phone model (Galaxy S II +15.95 ms/min =
//!   266 ppm, Galaxy Nexus +4.78 ms/min = 80 ppm, Nexus S −0.37, Galaxy S +0.26, S III +0.34);
//!   lab values carry about ±0.1 ms/min between units and ±1 ms/min with temperature. Its sync
//!   points are almost all within 10 ms, so it cannot judge errors much under that.
//! - The time stamps in the file names (the phone's clock at the start) are useless for sync:
//!   most are over 1 s off, some years (January 2000).
//! - Baseline in the paper: Haitsma–Kalker fingerprints found 23 of 44 spanning-tree edges on
//!   GT_090912 (52 % synchronization rate; 18–86 % on the other events), errors mostly ±50 ms;
//!   cross-correlating 1 s around them brought the mean to about 11 ms. High correlation peaks
//!   can still be wrong, and noisy tracks give flat correlation.
//! - Synchronization rate (spanning-tree edges found / the ground truth's) is the number to
//!   report.
//! - Phones in a crowd move: walking 10 m in 200 s changes the sound's delay by 29 ms, which
//!   looks like 145 ppm of drift. One phone's files on GT_090912 disagree by 50–130 ppm, so
//!   drift learnt from short overlaps is partly motion; the clock solve gives one phone's files
//!   one speed (`clock::solve_devices`).
//! - This method, blind (no ground truth used), on all 50 recordings: 498 links on all 78 pairs
//!   overlapping 20 s or more and on no other pair, none over 50 ms off the truth (median
//!   5.4 ms); 42 of the ground truth's 44 spanning-tree edges (95 %, against 52 % in the paper);
//!   after the clock solve, moments map between recordings within 5.0 ms (median) and 16.1 ms
//!   (95 %); pairs' relative speeds within 4 ppm (median) and 90 ppm (95 %) with one speed per
//!   phone, 27 and 127 ppm without. Matching all 1225 pairs takes about a second on 12 threads;
//!   decoding and fingerprinting the audio take most of the rest.
//! - Later work on Jiku: Başaran, Cemgil, Anarım (DSP 2017, sequential Monte Carlo over pairwise
//!   match / no-match); Uikey … Saini (MTAP 2024, overlap from landmark scatter plots and a
//!   CNN); ASTAnet (Singh … Saini, ICME 2025, a Siamese vision transformer). Another dataset with
//!   sync ground truth: the BBC Philharmonic multi-sensor concert dataset (MMSys 2015).

use audiofp::classical::{Wang, WangConfig, WangFingerprint};
use audiofp::matching::{Matcher, WangMatchConfig, WangMatcher, WangRefIndex};
use audiofp::{Fingerprinter, SampleRate};
use realfft::num_complex::Complex32;
use realfft::RealFftPlanner;

/// Sample rate of the clips, Hz (the fingerprints' own).
pub const SR: u32 = 8000;
/// Each chunk of clip b matched against clip a, seconds.
pub const CHUNK_S: f64 = 15.0;
/// Audio cross-correlated to refine a candidate, seconds, and how far either side of the
/// fingerprint's offset it searches.
pub const WIN_S: f64 = 8.0;
pub const SEARCH_S: f64 = 0.25;
/// Peak-to-sidelobe ratio a refined match needs, and how far it may land from the fingerprints'
/// offset. On Jiku the fingerprints' candidates were right 292 times out of 293 (within 35 ms),
/// and refined ones at PSR 4–12 were all within 50 ms of the truth, median 7–10 ms (at PSR 12
/// and over, 3.4 ms); the one wrong candidate correlated at PSR 3.9.
pub const MIN_PSR: f64 = 5.0;
pub const AGREE_S: f64 = 0.06;
/// The error (1 sigma) of a link at `SURE_PSR` or over, seconds; under it the error grows as
/// (SURE_PSR / PSR)^1.5 (PSR 6: 11 ms), after the Jiku errors by PSR.
pub const LINK_ERROR_S: f64 = 0.004;
pub const SURE_PSR: f64 = 12.0;

/// The fingerprint vote a chunk needs: a peak of `MIN_VOTES` hashes standing `MIN_PROMINENCE`
/// times over the other offsets. audiofp's score (votes per query hash, 0.15 by default) is for
/// telling the same recording; two phones in a crowd share far fewer hashes (on a synthetic
/// event 0.02–0.05 with noise), while the prominence of a true offset stays in the hundreds.
pub const MIN_VOTES: u32 = 8;
pub const MIN_PROMINENCE: f32 = 10.0;

fn match_config() -> WangMatchConfig {
    WangMatchConfig { min_score: 0.0, min_votes: MIN_VOTES, min_prominence: MIN_PROMINENCE, ..Default::default() }
}

/// A recording ready to match: its audio (`SR`, mono) and fingerprint.
pub struct Clip {
    pub audio: Vec<f32>,
    fp: WangFingerprint,
    index: Option<WangRefIndex>,
}

impl Clip {
    pub fn new(audio: Vec<f32>) -> Clip {
        let mut wang = Wang::new(WangConfig { max_input_samples: None, max_hashes: None, ..Default::default() });
        let fp = wang
            .extract(&audio, SampleRate::HZ_8000)
            .unwrap_or(WangFingerprint { hashes: Vec::new(), frames_per_sec: SR as f32 / 128.0 });
        let index = WangRefIndex::build(&fp, &match_config());
        Clip { audio, fp, index }
    }

    pub fn len_s(&self) -> f64 {
        self.audio.len() as f64 / SR as f64
    }
}

/// A refined match: moment `t_a` on clip a is moment `t_b` on clip b.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub t_a: f64,
    pub t_b: f64,
    /// Its error (1 sigma), seconds, from the PSR.
    pub error_s: f64,
    /// The cross-correlation's peak-to-sidelobe ratio.
    pub psr: f64,
    /// The fingerprint vote: matching hashes at the winning offset, and its prominence over the
    /// other offsets.
    pub votes: u32,
    pub prominence: f32,
}

/// What matching clip b against clip a found, and how much was tried: its match quality.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pair {
    pub hits: Vec<Hit>,
    /// Chunks of b tried, chunks the fingerprints matched, and of those the ones the
    /// cross-correlation did not confirm.
    pub chunks: usize,
    pub candidates: usize,
    pub weak: usize,
}

/// Matches every `CHUNK_S` of `b` against `a`.
pub fn match_pair(a: &Clip, b: &Clip, planner: &mut RealFftPlanner<f32>) -> Pair {
    let mut out = Pair::default();
    let Some(index) = &a.index else { return out };
    let matcher = WangMatcher::new(match_config());
    let fps = b.fp.frames_per_sec as f64;
    let chunk = (CHUNK_S * fps) as u32;
    let sr = SR as f64;
    let mut c0 = 0u32;
    while (c0 as f64 / fps) + CHUNK_S <= b.len_s() {
        out.chunks += 1;
        let hashes = b.fp.hashes.iter().filter(|h| h.t_anchor >= c0 && h.t_anchor < c0 + chunk).map(|h| audiofp::classical::WangHash { hash: h.hash, t_anchor: h.t_anchor - c0 }).collect();
        let q = WangFingerprint { hashes, frames_per_sec: b.fp.frames_per_sec };
        let r = matcher.match_one_prebuilt(&q, index);
        let start = c0;
        c0 += chunk;
        if !r.is_match {
            continue;
        }
        out.candidates += 1;
        // Query frame f (from the chunk's start) is reference frame f + offset.
        let shift = (r.offset.frames as f64 - start as f64) / fps;
        let mid = start as f64 / fps + CHUNK_S / 2.0;
        let (ib, w, s) = (((mid - WIN_S / 2.0) * sr) as usize, (WIN_S * sr) as usize, (SEARCH_S * sr) as usize);
        let ia = ((mid + shift - WIN_S / 2.0) * sr).round() as isize - s as isize;
        if ia < 0 || ia as usize + w + 2 * s > a.audio.len() || ib + w > b.audio.len() {
            out.weak += 1;
            continue;
        }
        let ia = ia as usize;
        let t_b = ib as f64 / sr + WIN_S / 2.0;
        match gcc_phat(planner, &b.audio[ib..ib + w], &a.audio[ia..ia + w + 2 * s]) {
            Some((lag, psr)) if psr >= MIN_PSR && (lag - s as f64).abs() / sr <= AGREE_S => out.hits.push(Hit {
                t_a: (ia as f64 + lag) / sr + WIN_S / 2.0,
                t_b,
                error_s: LINK_ERROR_S * (SURE_PSR / psr).max(1.0).powf(1.5),
                psr,
                votes: r.votes,
                prominence: r.prominence,
            }),
            _ => out.weak += 1,
        }
    }
    out
}

/// Every pair of clips (a before b in the list), matched on all cores.
pub fn match_all(clips: &[&Clip]) -> Vec<(usize, usize, Pair)> {
    let pairs: Vec<(usize, usize)> = (0..clips.len()).flat_map(|a| (a + 1..clips.len()).map(move |b| (a, b))).collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut out: Vec<(usize, usize, Pair)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut planner = RealFftPlanner::<f32>::new();
                    let mut done = Vec::new();
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&(a, b)) = pairs.get(k) else { break };
                        done.push((a, b, match_pair(clips[a], clips[b], &mut planner)));
                    }
                    done
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap()).collect()
    });
    out.sort_by_key(|x| (x.0, x.1));
    out
}

/// GCC-PHAT of `a` within `b` (b longer): the lag in samples where `a` best fits in `b`, with
/// sub-sample interpolation, and the peak's ratio to the other lags (peak − mean) / std.
pub fn gcc_phat(planner: &mut RealFftPlanner<f32>, a: &[f32], b: &[f32]) -> Option<(f64, f64)> {
    if a.is_empty() || b.len() < a.len() {
        return None;
    }
    let n = (a.len() + b.len()).next_power_of_two();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);
    let spec = |x: &[f32]| {
        let mut buf = vec![0.0f32; n];
        buf[..x.len()].copy_from_slice(x);
        let mut out = fwd.make_output_vec();
        fwd.process(&mut buf, &mut out).unwrap();
        out
    };
    let (fa, fb) = (spec(a), spec(b));
    let mut cross: Vec<Complex32> = fa
        .iter()
        .zip(&fb)
        .map(|(x, y)| {
            let c = x.conj() * y;
            let m = c.norm();
            if m > 1e-12 { c / m } else { Complex32::new(0.0, 0.0) }
        })
        .collect();
    cross[0] = Complex32::new(0.0, 0.0);
    let last = cross.len() - 1;
    cross[last].im = 0.0;
    let mut corr = vec![0.0f32; n];
    inv.process(&mut cross, &mut corr).ok()?;
    // c[k] = sum a[i] b[i + k]: a starting k samples into b.
    let lags = &corr[..=b.len() - a.len()];
    let (k, &peak) = lags.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1))?;
    let side: Vec<f64> = lags.iter().enumerate().filter(|(i, _)| i.abs_diff(k) > 40).map(|(_, &v)| v as f64).collect();
    if side.is_empty() {
        return None;
    }
    let mean = side.iter().sum::<f64>() / side.len() as f64;
    let sd = (side.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / side.len() as f64).sqrt();
    let psr = (peak as f64 - mean) / sd.max(1e-12);
    let frac = if k > 0 && k + 1 < lags.len() {
        let (l, c, r) = (lags[k - 1] as f64, peak as f64, lags[k + 1] as f64);
        let d = l - 2.0 * c + r;
        if d.abs() > 1e-12 { 0.5 * (l - r) / d } else { 0.0 }
    } else {
        0.0
    };
    Some((k as f64 + frac, psr))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic "event" sound: a new decaying note every 0.1 s at a random pitch, a few
    /// ringing at once (what landmarks are made of), over a little noise.
    fn event(secs: f64) -> Vec<f32> {
        let n = (secs * SR as f64) as usize;
        let mut x = 0x2545_f491_u64;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut notes: Vec<(usize, f64)> = Vec::new();
        (0..n)
            .map(|i| {
                if i % 800 == 0 {
                    notes.push((i, 200.0 + 3000.0 * rnd()));
                    notes.retain(|&(s, _)| i - s < 4000);
                }
                let mut v = 0.05 * (rnd() * 2.0 - 1.0);
                for &(s, f) in &notes {
                    let t = (i - s) as f64 / SR as f64;
                    v += 0.3 * (-t * 6.0).exp() * (2.0 * std::f64::consts::PI * f * t).sin();
                }
                v as f32
            })
            .collect()
    }

    #[test]
    fn finds_the_overlap_and_its_offset() {
        let ev = event(200.0);
        // a hears 0–120 s of the event, b 70.0123–200 s, with its own noise.
        let a = Clip::new(ev[..120 * SR as usize].to_vec());
        let start = (70.0123 * SR as f64) as usize;
        let mut n = 7u32;
        let b_audio: Vec<f32> = ev[start..]
            .iter()
            .map(|&v| {
                n = n.wrapping_mul(1_103_515_245).wrapping_add(12345);
                v + 0.1 * ((n >> 16) as f32 / 32768.0 - 1.0)
            })
            .collect();
        let b = Clip::new(b_audio);
        let p = match_pair(&a, &b, &mut RealFftPlanner::new());
        // b's 0–50 s overlap a: chunks at 0 and 15 and 30 s fit fully.
        assert!(p.hits.len() >= 2, "{p:?}");
        for h in &p.hits {
            let want = h.t_b + start as f64 / SR as f64;
            assert!((h.t_a - want).abs() < 0.001, "{} vs {want}", h.t_a);
        }
        // An unrelated clip matches nothing.
        let other = Clip::new(event(200.0).iter().rev().copied().collect());
        assert!(match_pair(&other, &b, &mut RealFftPlanner::new()).hits.is_empty());
    }
}
