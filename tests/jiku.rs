//! Real-world sync test on the Jiku Mobile Video Dataset: phones filming one event, with a
//! hand-made synchronization ground truth (Guggenberger, Lux, Böszörmenyi, MMM 2015) giving each
//! recording's offset and speed on the event's timeline.
//!
//! Fetch the data first (audio only, never committed):
//!     python tools/jiku.py GT_090912
//! then:
//!     cargo test --profile fast --test jiku -- --ignored --nocapture
//!
//! What it does, as the planned audio match module would: for every pair of recordings that
//! overlap, cross-correlate short windows of their audio along the overlap (GCC-PHAT, generalized
//! cross-correlation with phase transform). The ground truth is used only to know which pairs
//! overlap and roughly where (± 2 s, ignoring its speeds); every link's time comes from the audio.
//! The links go into the clock solve (offsets and drift), and the solved mapping between each
//! overlapping pair is scored against the ground truth's.
//!
//! The ground truth gives `offset` (where a recording starts on the event's timeline) and
//! `speed`, its sync points in the speed-corrected time. The paper (Table 2) gives the speeds as
//! drifts measured in the lab on the same phone models: a positive drift (the real sample rate
//! above the nominal) makes playback longer than the event, so the runtime is shortened by
//! `speed` = 1 − drift: moment t of a recording is at offset + t · speed (Galaxy S II +15.95
//! ms/min = 266 ppm, speed 0.999734; Galaxy Nexus +4.78 ms/min = 80 ppm, speed 0.999921). Being
//! lab values for the model, not the phone, they carry about ±1.7 ppm between units and up to
//! ±17 ppm with temperature, and the paper puts almost all sync points within 10 ms. Both readings
//! of `speed` are still scored, as a check.
//!
//! Numbers to compare with (same paper, GT_090912): Haitsma–Kalker fingerprinting found 23 of the
//! 44 spanning-tree edges (52 % synchronization rate); its sync points were mostly within ±50 ms,
//! and refining them by cross-correlating 1 s of audio brought the mean error to about 11 ms.

use corroborate::clock::{self, Link};
use realfft::num_complex::Complex32;
use realfft::RealFftPlanner;
use std::path::{Path, PathBuf};

const SR: u32 = 8000;
/// Window correlated at each step, seconds; steps along an overlap; search either side.
const WIN: f64 = 8.0;
const STEP: f64 = 30.0;
const SEARCH: f64 = 2.0;
/// Peak-to-sidelobe ratio a correlation peak needs to count as a match.
const MIN_PSR: f64 = 12.0;
/// The error given to each link (1 sigma), seconds.
const LINK_ERR: f64 = 0.002;

struct Rec {
    name: String,
    device: String,
    speed: f64,
    offset: f64,
    audio: Vec<f32>,
}

impl Rec {
    fn len_s(&self) -> f64 {
        self.audio.len() as f64 / SR as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Conv {
    /// Event time of moment t: offset + t / speed.
    Div,
    /// Event time of moment t: offset + t · speed.
    Mul,
}

impl Conv {
    fn to_event(self, r: &Rec, t: f64) -> f64 {
        match self {
            Conv::Div => r.offset + t / r.speed,
            Conv::Mul => r.offset + t * r.speed,
        }
    }

    fn from_event(self, r: &Rec, s: f64) -> f64 {
        match self {
            Conv::Div => (s - r.offset) * r.speed,
            Conv::Mul => (s - r.offset) / r.speed,
        }
    }
}

fn data_dir(event: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join("jiku").join(event)
}

/// "d:hh:mm:ss.fffffff" → seconds.
fn dhms(s: &str) -> f64 {
    let p: Vec<f64> = s.split(':').map(|x| x.parse().unwrap()).collect();
    ((p[0] * 24.0 + p[1]) * 60.0 + p[2]) * 60.0 + p[3]
}

fn attr<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let k = format!("{key}=\"");
    let i = line.find(&k)? + k.len();
    let j = line[i..].find('"')?;
    Some(&line[i..i + j])
}

fn decode(path: &Path) -> Vec<f32> {
    let (mut child, rx) = corroborate::ingest::spawn_ffmpeg(&path.to_string_lossy(), SR).expect("ffmpeg");
    let mut out = Vec::new();
    for c in rx {
        out.extend(c);
    }
    let _ = child.wait();
    out
}

fn load(event: &str) -> Vec<Rec> {
    let dir = data_dir(event);
    let xml = std::fs::read_to_string(dir.join(format!("{event}.xml"))).unwrap_or_else(|_| panic!("no ground truth in {}: run python tools/jiku.py {event}", dir.display()));
    let mut recs: Vec<Rec> = xml
        .lines()
        .filter(|l| l.trim_start().starts_with("<recording "))
        .map(|l| {
            let name = attr(l, "name").unwrap().to_string();
            Rec {
                device: name.split('_').nth(2).unwrap().to_string(),
                speed: attr(l, "speed").unwrap().parse().unwrap(),
                offset: dhms(attr(l, "offset").unwrap()),
                audio: Vec::new(),
                name,
            }
        })
        .collect();
    // Recordings whose audio is not there yet are left out (and said so).
    let n = recs.len();
    recs.retain(|r| dir.join(format!("{}.m4a", r.name)).exists());
    if recs.len() < n {
        println!("{} of {n} recordings missing (python tools/jiku.py {event}); scoring the rest", n - recs.len());
    }
    assert!(!recs.is_empty(), "no audio: run python tools/jiku.py {event}");
    std::thread::scope(|s| {
        for chunk in recs.chunks_mut(8) {
            let dir = &dir;
            s.spawn(move || {
                for r in chunk {
                    r.audio = decode(&dir.join(format!("{}.m4a", r.name)));
                }
            });
        }
    });
    recs
}

/// GCC-PHAT of `a` within `b` (b longer): the lag in samples where `a` best fits in `b`, with
/// sub-sample interpolation, and the peak's ratio to the other lags (peak − mean) / std.
fn gcc_phat(planner: &mut RealFftPlanner<f32>, a: &[f32], b: &[f32]) -> Option<(f64, f64)> {
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

/// Pairs that overlap by at least 20 s on the ground truth's timeline (either reading of speed
/// gives the same pairs at this scale).
fn overlaps(recs: &[Rec]) -> Vec<(usize, usize, f64, f64)> {
    let mut out = Vec::new();
    for a in 0..recs.len() {
        for b in a + 1..recs.len() {
            let (ra, rb) = (&recs[a], &recs[b]);
            let s = ra.offset.max(rb.offset);
            let e = (ra.offset + ra.len_s()).min(rb.offset + rb.len_s());
            if e - s >= 20.0 {
                out.push((a, b, s, e));
            }
        }
    }
    out
}

/// Links from the audio along every overlap.
fn links(recs: &[Rec], pairs: &[(usize, usize, f64, f64)]) -> (Vec<Link>, usize, usize) {
    let mut planner = RealFftPlanner::<f32>::new();
    let (mut out, mut tried, mut weak) = (Vec::new(), 0, 0);
    let w = (WIN * SR as f64) as usize;
    for &(a, b, s, e) in pairs {
        let (ra, rb) = (&recs[a], &recs[b]);
        let mut ev = s + 1.0;
        while ev + WIN + 1.0 <= e {
            // Only the ground truth's offsets, no speed: the search covers the rest.
            let ta = ev - ra.offset;
            let guess = ev - rb.offset;
            let b0 = guess - SEARCH;
            ev += STEP;
            let (ia, ib) = ((ta * SR as f64) as usize, (b0 * SR as f64).round() as isize);
            let nb = w + 2 * (SEARCH * SR as f64) as usize;
            if ib < 0 || ib as usize + nb > rb.audio.len() || ia + w > ra.audio.len() {
                continue;
            }
            tried += 1;
            let seg_a = &ra.audio[ia..ia + w];
            let seg_b = &rb.audio[ib as usize..ib as usize + nb];
            let Some((lag, psr)) = gcc_phat(&mut planner, seg_a, seg_b) else { continue };
            if psr < MIN_PSR {
                weak += 1;
                continue;
            }
            let t_a = ia as f64 / SR as f64 + WIN / 2.0;
            let t_b = (ib as f64 + lag) / SR as f64 + WIN / 2.0;
            out.push(Link { a: ra.name.clone(), t_a, b: rb.name.clone(), t_b, error_s: LINK_ERR, from: format!("psr {psr:.0}"), enabled: true });
        }
    }
    (out, tried, weak)
}

/// Spanning-tree edges of the overlap graph: recordings minus connected clusters.
fn tree_edges(n: usize, edges: impl Iterator<Item = (usize, usize)>) -> usize {
    let mut p: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        if p[x] != x {
            let r = find(p, p[x]);
            p[x] = r;
        }
        p[x]
    }
    let mut k = 0;
    for (a, b) in edges {
        let (ra, rb) = (find(&mut p, a), find(&mut p, b));
        if ra != rb {
            p[ra] = rb;
            k += 1;
        }
    }
    k
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

#[test]
#[ignore = "needs the Jiku audio: python tools/jiku.py GT_090912"]
fn jiku_gt_090912_sync_from_audio() {
    let recs = load("GT_090912");
    let pairs = overlaps(&recs);
    let (ls, tried, weak) = links(&recs, &pairs);
    println!("{} recordings, {} overlapping pairs, {tried} windows, {} links, {weak} weak", recs.len(), pairs.len(), ls.len());
    // Each link against the ground truth, before the solve (the paper's Fig. 1 and 3 bands).
    let idx = |n: &str| recs.iter().position(|r| r.name == n).unwrap();
    let link_err = |l: &Link| {
        let (a, b) = (&recs[idx(&l.a)], &recs[idx(&l.b)]);
        (l.t_b - Conv::Mul.from_event(b, Conv::Mul.to_event(a, l.t_a))).abs()
    };
    let le: Vec<f64> = ls.iter().map(link_err).collect();
    let within = |ms: f64| le.iter().filter(|&&e| e * 1e3 <= ms).count();
    let mut le_sorted = le.clone();
    let links_gt_p95 = if le.is_empty() { f64::NAN } else { pct(&mut le_sorted, 0.95) };
    println!("links against the ground truth: median {:.1} ms, 95% {:.1} ms", pct(&mut le_sorted, 0.5) * 1e3, links_gt_p95 * 1e3);
    println!(
        "links within 5 ms: {}, 10 ms: {}, 50 ms: {}, over 100 ms (wrong matches): {}",
        within(5.0),
        within(10.0),
        within(50.0),
        le.iter().filter(|&&e| e > 0.1).count()
    );
    // Synchronization rate: spanning-tree edges found by right links (within 100 ms) out of the
    // ground truth's (every overlap).
    let all: Vec<(usize, usize)> = (0..recs.len())
        .flat_map(|a| (a + 1..recs.len()).map(move |b| (a, b)))
        .filter(|&(a, b)| {
            let (ra, rb) = (&recs[a], &recs[b]);
            ra.offset.max(rb.offset) < (ra.offset + ra.len_s()).min(rb.offset + rb.len_s())
        })
        .collect();
    let gt_edges = tree_edges(recs.len(), all.iter().copied());
    let found = tree_edges(recs.len(), ls.iter().zip(&le).filter(|(_, e)| **e <= 0.1).map(|(l, _)| (idx(&l.a), idx(&l.b))));
    println!("synchronization rate: {found} of {gt_edges} spanning-tree edges ({:.0} %; Haitsma–Kalker in the paper: 23 of 44, 52 %)", 100.0 * found as f64 / gt_edges as f64);

    let clocks = clock::solve(&[], &ls);
    println!("{} contradictions (over 3 sigma)", clocks.contradictions.len());
    // How far each link sits from the solve (the solve's own fit, against the measurement).
    let mut fit: Vec<f64> = ls
        .iter()
        .filter_map(|l| {
            let (ca, cb) = (clocks.get(&l.a), clocks.get(&l.b));
            (ca.group == cb.group && ca.kind != clock::Kind::None).then(|| (cb.back(ca.at(l.t_a)) - l.t_b).abs())
        })
        .collect();
    let fit_p95 = if fit.is_empty() { f64::NAN } else { pct(&mut fit.clone(), 0.95) };
    if !fit.is_empty() {
        println!("links from the solve: median {:.1} ms, 95% {:.1} ms, max {:.1} ms", pct(&mut fit, 0.5) * 1e3, pct(&mut fit, 0.95) * 1e3, pct(&mut fit, 1.0) * 1e3);
    }

    // Score: for each overlapping pair both solved into one group, map moments of a to b with
    // the solve and with the ground truth.
    let mut scored = 0;
    let mut unsolved = 0;
    let mut best: Option<(Conv, f64)> = None;
    for conv in [Conv::Div, Conv::Mul] {
        let mut errs = Vec::new();
        unsolved = 0;
        for &(a, b, s, e) in &pairs {
            let (ca, cb) = (clocks.get(&recs[a].name), clocks.get(&recs[b].name));
            if ca.kind == clock::Kind::None || ca.group != cb.group {
                unsolved += 1;
                continue;
            }
            for ev in [s + 1.0, 0.5 * (s + e), e - 1.0] {
                let ta = conv.from_event(&recs[a], ev);
                let solved = cb.back(ca.at(ta));
                let truth = conv.from_event(&recs[b], conv.to_event(&recs[a], ta));
                errs.push((solved - truth).abs());
            }
        }
        scored = errs.len();
        let (p50, p95, max) = (pct(&mut errs, 0.5), pct(&mut errs, 0.95), pct(&mut errs, 1.0));
        println!("speed read as {conv:?}: error median {:.1} ms, 95% {:.1} ms, max {:.1} ms", p50 * 1e3, p95 * 1e3, max * 1e3);
        if best.is_none_or(|(_, b)| p95 < b) {
            best = Some((conv, p95));
        }
    }
    println!("{scored} moments scored, {unsolved} of {} pairs not linked into one group", pairs.len());

    // Drift: each linked pair's relative speed, solved and in the ground truth (dt_b / dt_a).
    let (conv, p95) = best.unwrap();
    let mut rel: Vec<(String, f64, f64)> = Vec::new();
    for &(a, b, _, _) in &pairs {
        let (ca, cb) = (clocks.get(&recs[a].name), clocks.get(&recs[b].name));
        if ca.kind == clock::Kind::None || ca.group != cb.group || !ls.iter().any(|l| l.a == recs[a].name && l.b == recs[b].name) {
            continue;
        }
        let solved = (1.0 + ca.rate) / (1.0 + cb.rate) - 1.0;
        let truth = match conv {
            Conv::Mul => recs[a].speed / recs[b].speed - 1.0,
            Conv::Div => recs[b].speed / recs[a].speed - 1.0,
        };
        rel.push((format!("{}-{}", recs[a].device, recs[b].device), solved, truth));
    }
    let mut d: Vec<f64> = rel.iter().map(|x| (x.1 - x.2).abs()).collect();
    if !d.is_empty() {
        println!("relative speed of {} linked pairs, solved − truth: median {:.0} ppm, 95% {:.0} ppm", d.len(), pct(&mut d, 0.5) * 1e6, pct(&mut d, 0.95) * 1e6);
    }
    for (devs, sv, tr) in rel.iter().filter(|x| (x.1 - x.2).abs() > 20e-6) {
        println!("  devices {devs}: solved {:+.0} ppm, truth {:+.0} ppm", sv * 1e6, tr * 1e6);
    }
    // What this test holds the audio path to. The ground truth is itself only "almost all within
    // 10 ms" with lab drift values per phone model (± 17 ppm), and phones move in the crowd (1 m is
    // 3 ms), so the solve is held to its own links and the ground truth only loosely.
    assert!(ls.len() > pairs.len(), "too few links from the audio");
    assert_eq!(le.iter().filter(|&&e| e > 0.1).count(), 0, "a link over 100 ms off the ground truth: a wrong match");
    assert!(found as f64 >= 0.52 * gt_edges as f64, "synchronization rate under the paper's fingerprinting baseline (52 %)");
    assert!(fit_p95 < 0.010, "the solve fits its links within 10 ms (95%): {:.1} ms", fit_p95 * 1e3);
    assert!(p95 < 0.050, "95% of mapped moments within 50 ms of the ground truth ({conv:?}): {:.1} ms", p95 * 1e3);
}
