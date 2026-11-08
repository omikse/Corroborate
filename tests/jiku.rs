//! Real-world sync test on the Jiku Mobile Video Dataset: phones filming one event, with a
//! hand-made synchronization ground truth (Guggenberger, Lux, Böszörmenyi, MMM 2015) giving each
//! recording's offset and speed on the event's timeline.
//!
//! Fetch the data first (audio only, never committed):
//!     python tools/jiku.py GT_090912
//! then:
//!     cargo test --profile fast --test jiku -- --ignored --nocapture
//!
//! What it does: the audio match (`corroborate::audio_match`) matches every pair of recordings
//! by itself (fingerprints find which pairs overlap and roughly where, GCC-PHAT, generalized
//! cross-correlation with phase transform, refines each match); the ground truth is used only to
//! score the result. Method and findings: the `audio_match` module docs.
//! The links go into the clock solve (offsets and drift; recordings of one phone, named in the
//! file name, share one speed), and the solved mapping between each overlapping pair is scored
//! against the ground truth's. One speed per phone took the 95th percentile of the mapped moments
//! from 30 to 22 ms (median 5.8 to 5.0 ms).
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

use corroborate::audio_match::{self, Clip, SR};
use corroborate::clock::{self, Link};
use std::path::{Path, PathBuf};

struct Rec {
    name: String,
    device: String,
    speed: f64,
    offset: f64,
    clip: Option<Clip>,
}

impl Rec {
    fn len_s(&self) -> f64 {
        self.clip.as_ref().map_or(0.0, |c| c.len_s())
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

/// At 8 kHz from ffmpeg; with JIKU_HALVE=1 as the workbench does it: the sources' 16 kHz 16-bit
/// audio, halved (`audio_match::halve`).
fn decode(path: &Path) -> Vec<f32> {
    let halve = std::env::var("JIKU_HALVE").is_ok_and(|v| v == "1");
    let sr = if halve { 2 * SR } else { SR };
    let (mut child, rx) = corroborate::ingest::spawn_ffmpeg(&path.to_string_lossy(), sr).expect("ffmpeg");
    let mut out = Vec::new();
    for c in rx {
        out.extend(c);
    }
    let _ = child.wait();
    if halve {
        let pcm: Vec<i16> = out.iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
        return audio_match::halve(&pcm);
    }
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
                clip: None,
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
                    r.clip = Some(Clip::new(decode(&dir.join(format!("{}.m4a", r.name)))));
                }
            });
        }
    });
    recs
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

/// Links from the audio match over every pair of recordings: links, pairs with a link, chunks
/// tried, fingerprint candidates, and candidates the cross-correlation did not confirm.
fn links(recs: &[Rec]) -> (Vec<Link>, usize, usize, usize, usize, Vec<(usize, usize, audio_match::Pair)>) {
    let clips: Vec<&Clip> = recs.iter().map(|r| r.clip.as_ref().unwrap()).collect();
    let found = audio_match::match_all(&clips);
    let (mut out, mut linked, mut chunks, mut cands, mut weak) = (Vec::new(), 0, 0, 0, 0);
    for (a, b, p) in found.clone() {
        chunks += p.chunks;
        cands += p.candidates;
        weak += p.weak;
        linked += usize::from(!p.hits.is_empty());
        for h in p.hits {
            out.push(Link {
                a: recs[a].name.clone(),
                t_a: h.t_a,
                b: recs[b].name.clone(),
                t_b: h.t_b,
                error_s: h.error_s,
                from: format!("psr {:.0}", h.psr),
                enabled: true,
            });
        }
    }
    (out, linked, chunks, cands, weak, found)
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
    let t0 = std::time::Instant::now();
    let (ls, linked, chunks, cands, weak, found_pairs) = links(&recs);
    println!(
        "{} recordings, {} pairs overlapping 20 s or more in the truth; audio match ({:.0} s): {chunks} chunks, {cands} fingerprint candidates, {weak} not confirmed, {} links on {linked} pairs",
        recs.len(),
        pairs.len(),
        t0.elapsed().as_secs_f64(),
        ls.len()
    );
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

    // Recordings of one phone run at one speed (the device is in the file name).
    let device: std::collections::BTreeMap<String, String> = recs.iter().map(|r| (r.name.clone(), r.device.clone())).collect();
    let alone = clock::solve(&[], &ls);
    let clocks = clock::solve_devices(&[], &ls, &device);
    println!("{} contradictions (over 3 sigma)", clocks.contradictions.len());
    // How far each link sits from the solve (the solve's own fit, against the measurement).
    let mut fit: Vec<f64> = ls
        .iter()
        .filter_map(|l| {
            let (ca, cb) = (clocks.get(&l.a), clocks.get(&l.b));
            (ca.group == cb.group && ca.kind != clock::Kind::None).then(|| (cb.back(ca.at(l.t_a)) - l.t_b).abs())
        })
        .collect();
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

    // Drift: each linked pair's relative speed, solved and in the ground truth (dt_b / dt_a),
    // with each recording's own speed and with one speed per phone.
    let (conv, p95) = best.unwrap();
    let rel = |clocks: &clock::Clocks| -> Vec<(String, f64, f64)> {
        let mut rel = Vec::new();
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
        rel
    };
    // Each phone's speed against its group's reference, solved (± error) and in the truth. On
    // GT_090912 phones 2, 3 and 4 agree with each other in every group, while phone 0 (the
    // references) is 84 to 391 ppm off the others depending on the group: solved per recording,
    // phone 0's own files differ by 50-130 ppm, more than their errors. A clock does not do that,
    // a moving phone does: walking 10 m in 200 s changes the sound's delay by 29 ms, 145 ppm.
    let mut seen = std::collections::BTreeSet::new();
    for r in &recs {
        let c = clocks.get(&r.name);
        if c.kind == clock::Kind::None || !seen.insert((c.group.clone(), r.device.clone())) {
            continue;
        }
        let gref = &recs[idx(&c.group)];
        let truth = match conv {
            Conv::Mul => r.speed / gref.speed - 1.0,
            Conv::Div => gref.speed / r.speed - 1.0,
        };
        println!(
            "group {} phone {}: solved {:+.0} ± {:.0} ppm, truth {:+.0} ppm ({:+.1} sigma)",
            &c.group[c.group.len() - 4..],
            r.device,
            c.rate * 1e6,
            c.rate_error * 1e6,
            truth * 1e6,
            (c.rate - truth) / c.rate_error.max(1e-9)
        );
    }
    for (what, c) in [("own speed per recording", &alone), ("one speed per phone", &clocks)] {
        let r = rel(c);
        let mut d: Vec<f64> = r.iter().map(|x| (x.1 - x.2).abs()).collect();
        if d.is_empty() {
            continue;
        }
        println!("relative speed of {} linked pairs, solved − truth, {what}: median {:.0} ppm, 95% {:.0} ppm", d.len(), pct(&mut d, 0.5) * 1e6, pct(&mut d, 0.95) * 1e6);
        for (devs, sv, tr) in r.iter().filter(|x| (x.1 - x.2).abs() > 20e-6) {
            println!("  devices {devs}: solved {:+.0} ppm, truth {:+.0} ppm", sv * 1e6, tr * 1e6);
        }
    }
    // Each pair's own drift (a line through its links, `audio_match::drift`) against the truth.
    let mut dd = Vec::new();
    let mut outside = 0;
    for (a, b, p) in &found_pairs {
        let Some(d) = audio_match::drift(&p.hits) else { continue };
        let truth = match conv {
            Conv::Mul => recs[*b].speed / recs[*a].speed - 1.0,
            Conv::Div => recs[*a].speed / recs[*b].speed - 1.0,
        } * 1e6;
        dd.push((d.ppm - truth).abs());
        outside += usize::from((d.ppm - truth).abs() > 3.0 * d.error_ppm);
        assert!(!audio_match::speed_changed(&d), "{}-{}: {d:?} called a speed change", recs[*a].name, recs[*b].name);
    }
    if !dd.is_empty() {
        println!("pair drift from the links alone, {} pairs: off the truth by median {:.0} ppm, 95% {:.0} ppm; {outside} outside 3 sigma", dd.len(), pct(&mut dd, 0.5), pct(&mut dd, 0.95));
    }
    // Two phones never correlate like one recording: the highest correlation of any link.
    let mut corr: Vec<f64> = found_pairs.iter().flat_map(|(_, _, p)| p.hits.iter().map(|h| h.corr)).collect();
    let mut score: Vec<f64> = found_pairs.iter().flat_map(|(_, _, p)| p.hits.iter().map(|h| h.score as f64)).collect();
    println!(
        "links' correlation between phones: median {:.2}, 95% {:.2}, max {:.2}; fingerprint score median {:.3}, max {:.3}",
        pct(&mut corr, 0.5),
        pct(&mut corr, 0.95),
        pct(&mut corr, 1.0),
        pct(&mut score, 0.5),
        pct(&mut score, 1.0)
    );
    let gapped: Vec<String> = found_pairs.iter().flat_map(|(a, b, p)| audio_match::gaps(&p.hits).into_iter().map(move |g| format!("{}-{}: {g:?}", a, b))).collect();
    println!("{} gaps (none expected: phone recordings, unedited)", gapped.len());
    assert!(gapped.is_empty(), "gaps in unedited recordings: {gapped:?}");
    let same: Vec<String> = found_pairs.iter().filter(|(_, _, p)| audio_match::same_recording(&p.hits)).map(|(a, b, _)| format!("{}-{}", recs[*a].name, recs[*b].name)).collect();
    assert!(same.is_empty(), "two phones called one recording: {same:?}");
    // The same recording twice: each video's own audio track (decoded from the .mp4) against
    // its .m4a.
    let dir = data_dir("GT_090912");
    let mut copies = 0;
    for r in recs.iter().take(4) {
        let mp4 = dir.join(format!("{}.mp4", r.name));
        if !mp4.exists() {
            continue;
        }
        let video = Clip::new(decode(&mp4));
        let p = audio_match::match_pair(r.clip.as_ref().unwrap(), &video, &mut realfft::RealFftPlanner::new());
        let d = audio_match::drift(&p.hits);
        let c: Vec<String> = p.hits.iter().map(|h| format!("{:.3}", h.corr)).collect();
        println!("{} .m4a vs .mp4: {} links, correlation {}, drift {:?}", r.name, p.hits.len(), c.join(" "), d.as_ref().map(|d| (d.ppm, d.error_ppm)));
        assert!(audio_match::same_recording(&p.hits), "{}: its own video not called the same recording", r.name);
        copies += 1;
    }
    println!("{copies} recordings found the same recording as their own video");

    // What this test holds the audio path to. The ground truth is itself only "almost all within
    // 10 ms" with lab drift values per phone model (± 17 ppm), and phones move in the crowd (1 m is
    // 3 ms), so the solve is held to its own links (within their errors) and the ground truth
    // only loosely. First blind run (all 50 recordings): 498 links on all 78 overlapping pairs,
    // none on a pair that does not overlap, none over 50 ms off; 42 of 44 edges (95 %); 2
    // contradictions; moments 95 % within 16 ms.
    assert!(ls.len() > pairs.len(), "too few links from the audio");
    assert_eq!(le.iter().filter(|&&e| e > 0.1).count(), 0, "a link over 100 ms off the ground truth: a wrong match");
    assert!(found as f64 >= 0.9 * gt_edges as f64, "synchronization rate under 90 % (the paper's fingerprinting: 52 %)");
    assert!(clocks.contradictions.len() * 100 <= ls.len(), "over 1 % of the links contradict the solve: {}", clocks.contradictions.len());
    assert!(p95 < 0.030, "95% of mapped moments within 30 ms of the ground truth ({conv:?}): {:.1} ms", p95 * 1e3);
}
