//! The cut detector on the Jiku Mobile Video Dataset (event GT_090912: phones filming one event,
//! with a hand-made sync ground truth; see tests/jiku.rs). Fetch the data first:
//!     python tools/jiku.py GT_090912
//! then:
//!     cargo test --profile fast --test jiku_cuts -- --ignored --nocapture
//!
//! - `a_jump_cut_is_found_against_another_phone`: a recording with a few seconds cut out (a jump
//!   cut: same place, same sound, nothing for the picture to see), compared with another phone
//!   that heard the same event: the compare lane must find the cut, located to a tenth of a second,
//!   and how much is missing.
//! - `shuffled_pieces_land_where_they_were`: a recording cut into four and put back together in
//!   another order; the detector finds the three joins against the other phones; each piece, matched
//!   on its own against the other phones, must land where the ground truth says it was, and their
//!   true order follows.

use corroborate::audio_match::{self, Clip, SR};
use corroborate::cuts;
use std::path::{Path, PathBuf};

struct Rec {
    name: String,
    offset: f64,
    speed: f64,
    audio: Vec<f32>,
}

impl Rec {
    fn len(&self) -> f64 {
        self.audio.len() as f64 / SR as f64
    }
    /// Event time of moment t (offset + t · speed, the reading tests/jiku.rs scores best).
    fn event(&self, t: f64) -> f64 {
        self.offset + t * self.speed
    }
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join("jiku").join("GT_090912")
}

fn decode(p: &Path) -> Vec<f32> {
    let (mut child, rx) = corroborate::ingest::spawn_ffmpeg(p.to_str().unwrap(), SR).unwrap();
    let mut out = Vec::new();
    for c in rx {
        out.extend(c);
    }
    let _ = child.wait();
    out
}

fn hms(s: &str) -> f64 {
    let f: Vec<f64> = s.split(':').map(|x| x.parse().unwrap()).collect();
    f[0] * 86400.0 + f[1] * 3600.0 + f[2] * 60.0 + f[3]
}

/// The recordings named in `names` (all with an .m4a when empty), with their ground truth.
fn load(names: &[&str]) -> Vec<Rec> {
    let xml = std::fs::read_to_string(dir().join("GT_090912.xml")).expect("fetch the data first: python tools/jiku.py GT_090912");
    let attr = |l: &str, k: &str| l.split(&format!("{k}=\"")).nth(1).and_then(|r| r.split('"').next()).map(str::to_string);
    xml.lines()
        .filter(|l| l.contains("<recording "))
        .filter_map(|l| {
            let name = attr(l, "name")?;
            if !names.is_empty() && !names.contains(&name.as_str()) {
                return None;
            }
            let p = dir().join(format!("{name}.m4a"));
            p.exists().then(|| Rec { offset: hms(&attr(l, "offset").unwrap()), speed: attr(l, "speed").unwrap().parse().unwrap(), audio: decode(&p), name })
        })
        .collect()
}

/// Seconds of `a` that `b` also heard.
fn overlap(a: &Rec, b: &Rec) -> f64 {
    a.event(a.len()).min(b.event(b.len())) - a.offset.max(b.offset)
}

/// A recording with the longest overlap with another one, and the others overlapping it by 60 s
/// or more.
fn pick(all: &[Rec]) -> (usize, Vec<usize>) {
    let mut best = (0, Vec::new(), 0.0);
    for a in 0..all.len() {
        let others: Vec<usize> = (0..all.len()).filter(|&b| b != a && overlap(&all[a], &all[b]) >= 60.0).collect();
        let total: f64 = others.iter().map(|&b| overlap(&all[a], &all[b])).sum();
        if total > best.2 && all[a].len() >= 120.0 {
            best = (a, others, total);
        }
    }
    (best.0, best.1)
}

#[test]
#[ignore]
fn a_jump_cut_is_found_against_another_phone() {
    let all = load(&[]);
    let (a, others) = pick(&all);
    let ours = &all[a];
    let b = *others.iter().max_by(|x, y| overlap(ours, &all[**x]).total_cmp(&overlap(ours, &all[**y]))).unwrap();
    let theirs = &all[b];
    // Where they overlap, 40 s in: 3 s taken out.
    let start = (theirs.offset - ours.offset).max(0.0) / ours.speed;
    let t = start + 40.0;
    let (k0, k1) = ((t * SR as f64) as usize, ((t + 3.0) * SR as f64) as usize);
    let mut cut = ours.audio[..k0].to_vec();
    cut.extend_from_slice(&ours.audio[k1..]);
    println!("{} cut at {t:.2} s (3 s out), against {} ({:.0} s overlap)", ours.name, theirs.name, overlap(ours, theirs));
    let lane = cuts::compare_lane("compare 1", &theirs.name, &Clip::new(cut.clone()), &Clip::new(theirs.audio.clone()));
    println!("{}", lane.note);
    for p in &lane.peaks {
        println!("  {:.2} s: {:.2} s, {}", p.t, p.value, p.note);
    }
    let level = cuts::level_lane(&cut);
    let noise = cuts::noise_lane(&cut);
    for l in [&level, &noise] {
        let near: Vec<(f64, f32)> = l.peaks.iter().filter(|p| (p.t - t).abs() < 1.0).map(|p| (p.t, p.value)).collect();
        let over = l.peaks.iter().filter(|p| p.value >= l.thr.unwrap()).count();
        println!("{}: at the cut {near:?}; {over} marks over the threshold in {:.0} s", l.name, cut.len() as f64 / SR as f64);
        let mut v: Vec<f32> = l.peaks.iter().map(|p| p.value).collect();
        v.sort_by(|a, b| b.total_cmp(a));
        println!("  strongest marks: {:?}", v.iter().take(12).map(|x| (x * 10.0).round() / 10.0).collect::<Vec<_>>());
    }
    let found: Vec<&cuts::Peak> = lane.peaks.iter().filter(|p| p.value >= lane.thr.unwrap()).collect();
    assert_eq!(found.len(), 1, "one jump");
    assert!((found[0].t - t).abs() <= 0.1, "found at {:.3}, cut at {t:.3}", found[0].t);
    assert!((found[0].value - 3.0).abs() <= 0.05, "{} s missing", found[0].value);
}

#[test]
#[ignore]
fn shuffled_pieces_land_where_they_were() {
    let all = load(&[]);
    let (a, others) = pick(&all);
    let ours = &all[a];
    // Four pieces of 45 s from the stretch most of the others heard, put together as 3, 1, 4, 2.
    let covers = |t: f64| others.iter().filter(|&&b| { let e = ours.event(t); e >= all[b].offset + 5.0 && e <= all[b].event(all[b].len()) - 5.0 }).count();
    let span = 180.0;
    let lo = (0..((ours.len() - span - 20.0).max(1.0) as usize)).step_by(5).map(|s| s as f64 + 10.0).max_by_key(|&s| (0..=18).map(|k| covers(s + k as f64 * 10.0)).min().unwrap_or(0)).unwrap();
    let piece = span / 4.0;
    let bounds: Vec<(f64, f64)> = (0..4).map(|k| (lo + k as f64 * piece, lo + (k + 1) as f64 * piece)).collect();
    let order = [2usize, 0, 3, 1];
    let mut joined = Vec::new();
    let mut truth = Vec::new(); // (where it starts in the joined file, its true start in ours)
    for &k in &order {
        let (s, e) = bounds[k];
        truth.push((joined.len() as f64 / SR as f64, s));
        joined.extend_from_slice(&ours.audio[(s * SR as f64) as usize..(e * SR as f64) as usize]);
    }
    println!("{}: pieces of {piece:.1} s from {lo:.1} s, joined as 3 1 4 2; {} other phones, {} hear all of it", ours.name, others.len(), (0..=18).map(|k| covers(lo + k as f64 * 10.0)).min().unwrap_or(0));
    let clip = Clip::new(joined.clone());
    let mut lanes = Vec::new();
    for (n, &b) in others.iter().enumerate() {
        let l = cuts::compare_lane(&format!("compare {}", n + 1), &all[b].name, &clip, &Clip::new(all[b].audio.clone()));
        println!("{}: {}", l.name, l.peaks.iter().map(|p| format!("{:.2} ({:+.1})", p.t, p.value)).collect::<Vec<_>>().join(", "));
        lanes.push(l);
    }
    let found = cuts::find_cuts(&lanes, &vec![None; lanes.len()], &[], 1.0 / 30.0);
    let on: Vec<(f64, f64)> = found.iter().filter(|c| c.on()).map(|c| (c.t_snd, c.error_s)).collect();
    println!("cuts on (time, error): {on:?}; true joins {:?}", truth.iter().skip(1).map(|t| t.0).collect::<Vec<_>>());
    assert_eq!(on.len(), 3, "{on:?}");
    // Each join within its stated error (three sigma), and within a second.
    for (&(t, e), want) in on.iter().zip(truth.iter().skip(1)) {
        assert!((t - want.0).abs() <= (3.0 * e).max(0.1) && (t - want.0).abs() <= 1.0, "join found at {t:.3} ± {e:.3}, true {:.3}", want.0);
    }
    // Each piece on its own, matched against every other phone: where its start lands on the
    // event's timeline, against where the ground truth says it was.
    let total = joined.len() as f64 / SR as f64;
    let ps = cuts::pieces(total, &found);
    let mut planner = realfft::RealFftPlanner::<f32>::new();
    let mut placed = Vec::new();
    for (p, &(join, true_start)) in ps.iter().zip(&truth) {
        // Where the piece found starts in the recording: the true start, moved as far as the
        // join was found off.
        let true_start = true_start + (p.a0 - join);
        let audio = joined[(p.a0 * SR as f64) as usize..(p.a1 * SR as f64) as usize].to_vec();
        let pc = Clip::new(audio);
        // Each other phone's links give the piece's start on the event's timeline.
        let mut at = Vec::new();
        for &b in &others {
            let pair = audio_match::match_pair(&Clip::new(all[b].audio.clone()), &pc, &mut planner);
            for h in &pair.hits {
                // Piece moment t_b is phone b's moment t_a: the piece starts at event(t_a) − t_b · speed.
                at.push(all[b].event(h.t_a) - h.t_b * ours.speed);
            }
        }
        at.sort_by(f64::total_cmp);
        let got = at.get(at.len() / 2).copied().unwrap_or(f64::NAN);
        let want = ours.event(true_start);
        println!("piece {} ({:.1} s): {} links, lands at {got:.3}, truth {want:.3}, off {:.1} ms", p.n, p.len(), at.len(), (got - want) * 1e3);
        assert!((got - want).abs() <= 0.05, "piece {} off by {:.3} s", p.n, got - want);
        placed.push((got, p.n));
    }
    placed.sort_by(|x, y| x.0.total_cmp(&y.0));
    let true_order: Vec<usize> = placed.iter().map(|p| p.1).collect();
    // Joined as 3 1 4 2: piece 2 of the file is the true first, then 4, 1, 3.
    assert_eq!(true_order, [2, 4, 1, 3]);
}
