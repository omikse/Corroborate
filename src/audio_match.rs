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
//! - The workbench takes the sources' 16 kHz 16-bit audio and halves it (`halve`) instead of
//!   decoding at 8 kHz again; on the same test (JIKU_HALVE=1) that gives 501 links on the 78
//!   pairs, none over 50 ms off (median 5.6 ms, 95 % 21.9 ms), 42 of 44 edges: the same.
//! - Each pair's own drift (`drift`, a line through its links) is within 24 ppm of the truth
//!   (median) but 300 ppm at 95 %, 3 of 53 pairs outside 3 sigma: overlaps are short and phones
//!   move; the clock solve over all links (one speed per phone) does far better.
//! - Same recording (`same_recording`): links between two phones correlate (Pearson's, windows
//!   lined up) 0.18 median, 0.49 at 95 %, 0.64 at most; a video's audio against its own .m4a
//!   1.000. The fingerprint score between phones: 0.009 median, 0.056 at most.
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

/// The correlation (Pearson's, of the two windows lined up) over which a pair's links say the
/// two files are one recording: a copy, a re-upload, a re-encode. Two microphones in one room
/// hear the same sound through different paths, rooms and noise, and correlate far lower.
pub const SAME_CORR: f64 = 0.8;
/// A pair's clocks differing by more than this (ppm), beyond 3 sigma, is a recording played at
/// another speed rather than drift: quartz clocks stay within about 300 (Jiku's phones: 266 at
/// most). The fingerprints only match up to about 1 % (10000 ppm) of speed change.
pub const SPEED_CHANGE_PPM: f64 = 1000.0;
/// Smallest jump in a pair's offset called a gap, seconds (beyond 5 sigma of the two links):
/// 0.1 s is 34 m of sound, more than a phone walks between two chunks.
pub const GAP_MIN_S: f64 = 0.1;
/// Shortest span of links a pair's drift is given for, seconds.
pub const DRIFT_SPAN_S: f64 = 30.0;

fn match_config() -> WangMatchConfig {
    WangMatchConfig { min_score: 0.0, min_votes: MIN_VOTES, min_prominence: MIN_PROMINENCE, ..Default::default() }
}

/// Audio at twice `SR` (the sources' 16 kHz, as 16-bit samples) brought to `SR`: a lowpass at
/// 3.6 kHz (windowed sinc, 63 taps, Blackman: about -70 dB from 4.4 kHz, where 8 kHz's
/// alias would land in the band the fingerprints use), then every second sample. Lets the match
/// use the audio its sources decoded instead of decoding every file again.
pub fn halve(x: &[i16]) -> Vec<f32> {
    const HALF: usize = 31;
    let fc = 3600.0 / (2.0 * SR as f64);
    let taps: Vec<f32> = (0..=2 * HALF)
        .map(|i| {
            let n = i as f64 - HALF as f64;
            let sinc = if n == 0.0 { 2.0 * fc } else { (2.0 * std::f64::consts::PI * fc * n).sin() / (std::f64::consts::PI * n) };
            let a = 2.0 * std::f64::consts::PI * i as f64 / (2 * HALF) as f64;
            (sinc * (0.42 - 0.5 * a.cos() + 0.08 * (2.0 * a).cos())) as f32
        })
        .collect();
    let sum: f32 = taps.iter().sum();
    let taps: Vec<f32> = taps.iter().map(|t| t / sum / 32768.0).collect();
    let n = x.len();
    (0..n.div_ceil(2))
        .map(|k| {
            let c = 2 * k;
            if c >= HALF && c + HALF < n {
                taps.iter().zip(&x[c - HALF..=c + HALF]).map(|(t, &v)| t * v as f32).sum()
            } else {
                // The edges: samples outside the recording count as silence.
                (0..=2 * HALF).filter_map(|j| (c + j).checked_sub(HALF).filter(|&i| i < n).map(|i| taps[j] * x[i] as f32)).sum()
            }
        })
        .collect()
}

/// How far from the group's time a file's stamp may be and still count, seconds: phones' clocks
/// are off by seconds, and a stamp further out says nothing about when the group happened.
pub const STAMP_CAP_S: f64 = 30.0;

/// One recording of a linked group, for `fit_stamps`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// The file's time stamp (seconds since 1970), if it has one.
    pub stamp: Option<f64>,
    /// Its length, seconds.
    pub len: f64,
    /// Where audio puts its start on the group's clock.
    pub start: f64,
    /// The person's reading: the stamp marks the recording's end, not its start.
    pub as_end: bool,
}

impl Placed {
    /// The group's clock 0 in real time, if this stamp is read as `as_end` says.
    fn zero(&self, as_end: bool) -> Option<f64> {
        self.stamp.map(|s| s - self.start - if as_end { self.len } else { 0.0 })
    }
}

/// Where the group's clock 0 is in real time, as the stamps have it (None: no stamp), and how many
/// seconds later than that each recording's stamp says (None: it has no stamp).
#[derive(Clone, Debug, PartialEq)]
pub struct StampFit {
    pub zero: Option<f64>,
    pub diffs: Vec<Option<f64>>,
}

/// Lines a linked group up with its files' stamps, each read as the person chose. The zero is the
/// one most recordings agree on, each counting at most `STAMP_CAP_S`, so a clock years off cannot
/// drag it, then the median of the stamps that fit. `fixed`: the zero is already known (the group
/// has real time), and the stamps are only measured against it.
pub fn fit_stamps(recs: &[Placed], fixed: Option<f64>) -> StampFit {
    let zeros: Vec<Option<f64>> = recs.iter().map(|r| r.zero(r.as_end)).collect();
    let zero = fixed.or_else(|| {
        let cost = |z: f64| -> f64 { zeros.iter().flatten().map(|c| (c - z).abs().min(STAMP_CAP_S)).sum() };
        let best = zeros.iter().flatten().copied().min_by(|a, b| cost(*a).total_cmp(&cost(*b)))?;
        let mut fits: Vec<f64> = zeros.iter().flatten().copied().filter(|c| (c - best).abs() <= STAMP_CAP_S).collect();
        fits.sort_by(f64::total_cmp);
        Some(fits[fits.len() / 2])
    });
    StampFit { zero, diffs: zeros.iter().map(|c| c.zip(zero).map(|(c, z)| c - z)).collect() }
}

/// How far a group's stamps typically disagree (median distance from their median, seconds) if
/// all are read as the start, or all as the end: the number to judge that reading by. None with
/// fewer than two stamps.
pub fn stamp_spread(recs: &[Placed], as_end: bool) -> Option<f64> {
    let mut z: Vec<f64> = recs.iter().filter_map(|r| r.zero(as_end)).collect();
    if z.len() < 2 {
        return None;
    }
    z.sort_by(f64::total_cmp);
    let m = z[z.len() / 2];
    let mut d: Vec<f64> = z.iter().map(|x| (x - m).abs()).collect();
    d.sort_by(f64::total_cmp);
    Some(d[d.len() / 2])
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

    /// How many fingerprint hashes it has.
    pub fn hashes(&self) -> usize {
        self.fp.hashes.len()
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
    /// The fingerprint score (matching hashes per hash of the chunk).
    pub score: f32,
    /// Pearson's correlation of the two windows lined up at the refined lag (to the sample).
    pub corr: f64,
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
        let wb = &b.audio[ib..ib + w];
        match gcc_phat(planner, wb, &a.audio[ia..ia + w + 2 * s]) {
            Some((lag, psr)) if psr >= MIN_PSR && (lag - s as f64).abs() / sr <= AGREE_S => {
                let k = ia + lag.round().max(0.0) as usize;
                out.hits.push(Hit {
                    t_a: (ia as f64 + lag) / sr + WIN_S / 2.0,
                    t_b,
                    error_s: LINK_ERROR_S * (SURE_PSR / psr).max(1.0).powf(1.5),
                    psr,
                    votes: r.votes,
                    prominence: r.prominence,
                    score: r.score,
                    corr: a.audio.get(k..k + w).map_or(0.0, |wa| pearson(wa, wb)),
                })
            }
            _ => out.weak += 1,
        }
    }
    out
}

/// Pearson's correlation of two equally long signals.
pub fn pearson(x: &[f32], y: &[f32]) -> f64 {
    let n = x.len().min(y.len()) as f64;
    if n < 2.0 {
        return 0.0;
    }
    let (mx, my) = (x.iter().map(|&v| v as f64).sum::<f64>() / n, y.iter().map(|&v| v as f64).sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (&a, &b) in x.iter().zip(y) {
        let (a, b) = (a as f64 - mx, b as f64 - my);
        sxy += a * b;
        sxx += a * a;
        syy += b * b;
    }
    if sxx > 0.0 && syy > 0.0 { sxy / (sxx * syy).sqrt() } else { 0.0 }
}

/// How fast clock a runs against clock b, from a pair's links: t_a − t_b fitted as a line over
/// t_b, weighted by each link's error.
#[derive(Clone, Debug, PartialEq)]
pub struct Drift {
    /// a's seconds per b's second, minus 1, in ppm (positive: a runs fast).
    pub ppm: f64,
    /// Its error (1 sigma), ppm: from the links' errors, grown by how much more they scatter
    /// about the line than those errors say (phones moving, echoes).
    pub error_ppm: f64,
    /// From the first link to the last, on b's clock, seconds; and how many links.
    pub span_s: f64,
    pub links: usize,
}

/// The pair's drift, when its links span `DRIFT_SPAN_S` or more (three links at least).
pub fn drift(hits: &[Hit]) -> Option<Drift> {
    if hits.len() < 3 {
        return None;
    }
    let (lo, hi) = hits.iter().fold((f64::MAX, f64::MIN), |(l, h), x| (l.min(x.t_b), h.max(x.t_b)));
    if hi - lo < DRIFT_SPAN_S {
        return None;
    }
    let w: Vec<f64> = hits.iter().map(|h| 1.0 / h.error_s.max(1e-6).powi(2)).collect();
    let sw: f64 = w.iter().sum();
    let mx = hits.iter().zip(&w).map(|(h, w)| w * h.t_b).sum::<f64>() / sw;
    let my = hits.iter().zip(&w).map(|(h, w)| w * (h.t_a - h.t_b)).sum::<f64>() / sw;
    let sxx: f64 = hits.iter().zip(&w).map(|(h, w)| w * (h.t_b - mx).powi(2)).sum();
    let sxy: f64 = hits.iter().zip(&w).map(|(h, w)| w * (h.t_b - mx) * (h.t_a - h.t_b - my)).sum();
    let r = sxy / sxx;
    let chi2: f64 = hits.iter().zip(&w).map(|(h, w)| w * (h.t_a - h.t_b - my - r * (h.t_b - mx)).powi(2)).sum();
    let scale = (chi2 / (hits.len() - 2) as f64).sqrt().max(1.0);
    Some(Drift { ppm: r * 1e6, error_ppm: scale / sxx.sqrt() * 1e6, span_s: hi - lo, links: hits.len() })
}

/// Whether a pair's two files are one recording: half its links or more over `SAME_CORR`. On
/// Jiku, links between two phones correlate 0.18 (median), 0.49 (95 %), 0.64 at most; a video's
/// audio against its own .m4a, 1.000.
pub fn same_recording(hits: &[Hit]) -> bool {
    !hits.is_empty() && hits.iter().filter(|h| h.corr >= SAME_CORR).count() * 2 >= hits.len()
}

/// Whether a pair's drift says one was played at another speed (`SPEED_CHANGE_PPM`).
pub fn speed_changed(d: &Drift) -> bool {
    d.ppm.abs() - 3.0 * d.error_ppm > SPEED_CHANGE_PPM
}

/// Where a pair's offset jumps between two links in a row: one recording lacks a stretch the
/// other has (it was paused, or cut), or has one the other lacks (an insert).
#[derive(Clone, Debug, PartialEq)]
pub struct Gap {
    /// Which recording lacks it: a (true) or b. The jump alone cannot tell a cut from an insert
    /// in the other; a third recording can.
    pub on_a: bool,
    /// Between these two moments of that recording (its links either side), seconds.
    pub t0: f64,
    pub t1: f64,
    /// How much it lacks, seconds, and the error of that (1 sigma).
    pub len_s: f64,
    pub error_s: f64,
}

/// The gaps in a pair's links (`GAP_MIN_S`).
pub fn gaps(hits: &[Hit]) -> Vec<Gap> {
    let mut h: Vec<&Hit> = hits.iter().collect();
    h.sort_by(|x, y| x.t_b.total_cmp(&y.t_b));
    h.windows(2)
        .filter_map(|w| {
            let (x, y) = (w[0], w[1]);
            let d = (y.t_a - y.t_b) - (x.t_a - x.t_b);
            let err = x.error_s.hypot(y.error_s);
            (d.abs() >= GAP_MIN_S.max(5.0 * err)).then(|| {
                // a moved on d more than b: b lacks d seconds that a has.
                let on_a = d < 0.0;
                let (t0, t1) = if on_a { (x.t_a, y.t_a) } else { (x.t_b, y.t_b) };
                Gap { on_a, t0, t1, len_s: d.abs(), error_s: err }
            })
        })
        .collect()
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

    fn event(secs: f64) -> Vec<f32> {
        crate::synth::notes(secs, SR, 7)
    }

    fn placed(stamp: f64, len: f64, start: f64, as_end: bool) -> Placed {
        Placed { stamp: Some(stamp), len, start, as_end }
    }

    const T: f64 = 1_347_000_000.0;

    /// Stamps that mark the end, read as the end, agree; read as the start they do not, and the
    /// numbers say so. Nothing decides the reading but the person.
    #[test]
    fn stamps_are_read_as_the_person_says() {
        let lens = [(100.0, 0.0), (90.0, 40.0), (200.0, 75.0)];
        let read = |as_end: bool| -> Vec<Placed> { lens.iter().map(|&(l, st): &(f64, f64)| placed(T + st + l, l, st, as_end)).collect() };
        let f = fit_stamps(&read(true), None);
        assert_eq!(f.zero, Some(T));
        assert!(f.diffs.iter().all(|d| *d == Some(0.0)), "{f:?}");
        let g = fit_stamps(&read(false), None);
        assert!(g.diffs.iter().any(|d| d.unwrap().abs() > 5.0), "read as the start they disagree: {g:?}");
        assert_eq!(stamp_spread(&read(false), true), Some(0.0));
        assert!(stamp_spread(&read(false), false).unwrap() > 5.0);
    }

    #[test]
    fn stamps_missing_wrong_and_known_zero() {
        let recs = [placed(T + 0.6, 100.0, 0.0, false), Placed { stamp: None, len: 50.0, start: 10.0, as_end: false }, placed(T + 75.0, 200.0, 75.0, false), placed(T + 40.0, 80.0, 40.2, false), placed(T + 4e8, 60.0, 5.0, false)];
        let f = fit_stamps(&recs, None);
        assert!((f.zero.unwrap() - T).abs() < 0.7, "a clock years off does not drag it: {f:?}");
        assert_eq!(f.diffs[1], None);
        assert!(f.diffs[4].unwrap() > 3e8);
        let none = [Placed { stamp: None, len: 1.0, start: 0.0, as_end: false }];
        assert_eq!(fit_stamps(&none, None), StampFit { zero: None, diffs: vec![None] });
        assert_eq!(stamp_spread(&none, false), None);
        // The group has real time: stamps are measured against it, not moved.
        let f = fit_stamps(&[placed(T + 110.0, 100.0, T + 8.0, true)], Some(0.0));
        assert_eq!((f.zero, f.diffs[0]), (Some(0.0), Some(2.0)));
    }

    /// 16 kHz to 8 kHz: a 1 kHz tone keeps its level, a 5 kHz one (which would alias to 3 kHz)
    /// is gone, a click stays at its time, and the length halves.
    #[test]
    fn halve_keeps_the_band_and_the_time() {
        let tone = |f: f64| -> Vec<i16> { (0..32_000).map(|i| ((2.0 * std::f64::consts::PI * f * i as f64 / 16_000.0).sin() * 16_000.0) as i16).collect() };
        let rms = |x: &[f32]| (x[500..15_500].iter().map(|v| v * v).sum::<f32>() / 15_000.0).sqrt();
        let low = halve(&tone(1000.0));
        assert_eq!(low.len(), 16_000);
        let want = 16_000.0 / 32_768.0 / 2f32.sqrt();
        assert!((rms(&low) / want - 1.0).abs() < 0.01, "1 kHz kept: {} vs {want}", rms(&low));
        let high = halve(&tone(5000.0));
        assert!(rms(&high) < want * 0.001, "5 kHz gone (-60 dB): {}", rms(&high));
        let mut click = vec![0i16; 8000];
        click[4000] = 30_000;
        let out = halve(&click);
        let peak = (0..out.len()).max_by(|&a, &b| out[a].abs().total_cmp(&out[b].abs())).unwrap();
        assert_eq!(peak, 2000, "a click at 0.25 s stays at 0.25 s");
    }

    #[test]
    fn finds_the_overlap_and_its_offset() {
        let ev = event(200.0);
        // a hears 0–120 s of the event, b 70.0123–200 s, with its own noise and an echo off a
        // wall 2 m further (another microphone in the room).
        let a = Clip::new(ev[..120 * SR as usize].to_vec());
        let start = (70.0123 * SR as f64) as usize;
        let mut n = 7u32;
        let echo = 47;
        let b_audio: Vec<f32> = ev[start..]
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                n = n.wrapping_mul(1_103_515_245).wrapping_add(12345);
                v + 0.8 * ev[start + i - echo] + 0.2 * ((n >> 16) as f32 / 32768.0 - 1.0)
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
        let other = Clip::new(crate::synth::notes(200.0, SR, 8));
        assert!(match_pair(&other, &b, &mut RealFftPlanner::new()).hits.is_empty());
        // Two microphones (each its own noise) are not one recording; a copy is.
        assert!(!same_recording(&p.hits), "{p:?}");
        let copy = Clip::new(a.audio.iter().map(|&v| 0.7 * v).collect());
        let c = match_pair(&a, &copy, &mut RealFftPlanner::new());
        assert!(same_recording(&c.hits), "{c:?}");
    }

    /// `audio` resampled to play `1 + ppm / 1e6` times slower (linear interpolation).
    fn stretch(audio: &[f32], ppm: f64) -> Vec<f32> {
        let k = 1.0 / (1.0 + ppm * 1e-6);
        (0..((audio.len() - 1) as f64 / k) as usize)
            .map(|i| {
                let x = i as f64 * k;
                let (j, f) = (x as usize, (x - x.floor()) as f32);
                audio[j] * (1.0 - f) + audio[j + 1] * f
            })
            .collect()
    }

    #[test]
    fn measures_drift_and_tells_a_speed_change() {
        let ev = event(240.0);
        let a = Clip::new(ev.clone());
        // b's clock runs 200 ppm slow: each of its seconds is 1.0002 of a's. Within quartz drift.
        for (ppm, changed) in [(200.0, false), (3000.0, true)] {
            let b = Clip::new(stretch(&ev, -ppm).iter().map(|&v| 0.5 * v).collect());
            let p = match_pair(&a, &b, &mut RealFftPlanner::new());
            let d = drift(&p.hits).expect("drift");
            // t_a = t_b · (1 + ppm): a's clock counts ppm more per b's second.
            assert!((d.ppm - ppm).abs() < 3.0 * d.error_ppm.max(5.0), "{ppm}: {d:?}");
            assert_eq!(speed_changed(&d), changed, "{ppm}: {d:?}");
        }
        assert!(drift(&[]).is_none());
    }

    #[test]
    fn finds_a_stretch_cut_out() {
        // b is the event with 7.3 s cut out at 100 s (and quieter); a is all of it.
        let ev = event(240.0);
        let a = Clip::new(ev.clone());
        let cut = (100.0 * SR as f64) as usize..(107.3 * SR as f64) as usize;
        let b_audio: Vec<f32> = ev[..cut.start].iter().chain(&ev[cut.end..]).map(|&v| 0.6 * v).collect();
        let p = match_pair(&a, &Clip::new(b_audio), &mut RealFftPlanner::new());
        let g = gaps(&p.hits);
        assert_eq!(g.len(), 1, "{g:?}");
        assert!(!g[0].on_a && (g[0].len_s - 7.3).abs() < 0.01, "{g:?}");
        assert!(g[0].t0 < 100.0 && g[0].t1 > 100.0, "the cut lies between its links: {g:?}");
        // Seen the other way round (b as a), the same stretch is lacking on that side.
        let g2 = gaps(&p.hits.iter().map(|h| Hit { t_a: h.t_b, t_b: h.t_a, ..h.clone() }).collect::<Vec<_>>());
        assert!(g2.len() == 1 && g2[0].on_a, "{g2:?}");
        // No cut, no gap.
        let whole = match_pair(&a, &Clip::new(ev.iter().map(|&v| 0.6 * v).collect()), &mut RealFftPlanner::new());
        assert!(gaps(&whole.hits).is_empty());
    }
}
