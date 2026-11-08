//! The Audio match node in the workbench. Once the graph runs it decodes each wired file again
//! (8 kHz mono, the fingerprints' rate), matches every pair (`corroborate::audio_match`) on its
//! own thread, and puts the links (with both clocks named) and their quality on its outputs; the
//! links go into the clock solve from there like any link output. Files only for now: a stream or
//! microphone wired in is named in the status and left out. Compiled into the `corroborate`
//! program.

use super::*;
use corroborate::audio_match::{self, Clip};
use corroborate::data::{Data, Event};
use std::sync::atomic::AtomicU64;
use std::thread;

/// One wired recording: its source's label, clock and file.
pub struct Wired {
    pub label: String,
    pub clock: String,
    pub path: String,
}

static RUNS: AtomicU64 = AtomicU64::new(1);

impl App {
    /// The files wired into node `i`, and the labels of sources that are not files.
    pub(crate) fn audio_match_inputs(&self, i: NodeId) -> (Vec<Wired>, Vec<String>) {
        let (mut files, mut other) = (Vec::new(), Vec::new());
        for p in &self.graph.nodes()[i].spec.inputs {
            let Some((f, _)) = self.graph.feeder(i, p.name) else { continue };
            let label = self.nodes[f].label.clone();
            match &self.nodes[f].kind {
                Kind::Sourcer { input } => match sourcer_node::input(input) {
                    Some(runtime::Input::File(path)) => files.push(Wired { label, clock: self.clock_of(f).unwrap_or_default(), path }),
                    _ => other.push(label),
                },
                _ => other.push(label),
            }
        }
        (files, other)
    }
}

/// Decodes and matches `files` on a thread of its own, writing into `state` (unless `stop` is
/// set first, or a newer run took the state over).
pub fn start(state: Shared, files: Vec<Wired>, other: Vec<String>, stop: Arc<AtomicBool>) {
    let run = RUNS.fetch_add(1, Ordering::Relaxed);
    {
        let mut s = state.lock().unwrap();
        s.reset();
        s.run_id = run;
    }
    let note = if other.is_empty() { String::new() } else { format!(" ({} left out: files only)", other.join(", ")) };
    let set = |f: &dyn Fn(&mut runtime::NodeState)| {
        let mut s = state.lock().unwrap();
        if s.run_id == run && !stop.load(Ordering::Relaxed) {
            f(&mut s);
        }
    };
    if files.len() < 2 {
        set(&|s| s.status = format!("wire two recordings or more{note}"));
        return;
    }
    set(&|s| {
        s.running = true;
        s.status = format!("decoding {} recordings…", files.len());
    });
    let stop2 = stop.clone();
    thread::spawn(move || {
        let set = |f: &dyn Fn(&mut runtime::NodeState)| {
            let mut s = state.lock().unwrap();
            if s.run_id == run && !stop2.load(Ordering::Relaxed) {
                f(&mut s);
            }
        };
        let decoded: Vec<Result<Clip, String>> = thread::scope(|sc| {
            let hs: Vec<_> = files.iter().map(|w| sc.spawn(|| decode(&w.path).map(Clip::new))).collect();
            hs.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("decoder crashed".into()))).collect()
        });
        if stop2.load(Ordering::Relaxed) {
            return;
        }
        let mut clips = Vec::new();
        let mut used = Vec::new();
        let mut errors = Vec::new();
        for (w, c) in files.iter().zip(decoded) {
            match c {
                Ok(c) => {
                    clips.push(c);
                    used.push(w);
                }
                Err(e) => errors.push(format!("{}: {e}", w.label)),
            }
        }
        set(&|s| s.status = format!("matching {} pairs…", clips.len() * clips.len().saturating_sub(1) / 2));
        let refs: Vec<&Clip> = clips.iter().collect();
        let found = audio_match::match_all(&refs);
        let r = results(&used, &found, &errors);
        set(&|s| {
            s.outbox = vec![
                ("links", Data::Custom { kind: "link", json: r.links.clone() }),
                ("quality", Data::Custom { kind: "quality", json: r.quality.clone() }),
                ("clock drift", Data::Json(r.drift.clone())),
                ("same recording", Data::Events(r.same.clone())),
                ("speed change", Data::Events(r.speed.clone())),
            ];
            s.outbox_rev += 1;
            s.status = format!("{}{note}", r.status);
            s.running = false;
            s.done = true;
        });
    });
}

fn decode(path: &str) -> Result<Vec<f32>, String> {
    let (mut child, rx) = corroborate::ingest::spawn_ffmpeg(path, audio_match::SR)?;
    let mut out = Vec::new();
    for c in rx {
        out.extend(c);
    }
    let _ = child.wait();
    if out.is_empty() { Err("no audio".into()) } else { Ok(out) }
}

/// What the match gives on the node's outputs.
struct Results {
    /// JSON list of the `link` type, both clocks named.
    links: String,
    /// The `quality` type's fields, then the counts it rests on and each pair's.
    quality: String,
    /// JSON list, one per pair with a long enough span of links: how fast a's clock runs
    /// against b's (`audio_match::drift`).
    drift: String,
    /// For each pair that is one recording twice, an event on each of the two clocks over the
    /// matched stretch; for each pair one of which plays at another speed, one on b's clock.
    same: Vec<Event>,
    speed: Vec<Event>,
    status: String,
}

/// The stretch of clock b (`a`: of clock a) the hits cover, with half a window either side.
fn span(hits: &[audio_match::Hit], a: bool) -> (f64, f64) {
    let t = |h: &audio_match::Hit| if a { h.t_a } else { h.t_b };
    let lo = hits.iter().map(t).fold(f64::MAX, f64::min) - audio_match::WIN_S / 2.0;
    let hi = hits.iter().map(t).fold(f64::MIN, f64::max) + audio_match::WIN_S / 2.0;
    (lo.max(0.0), hi)
}

fn results(used: &[&Wired], found: &[(usize, usize, audio_match::Pair)], errors: &[String]) -> Results {
    let (mut drifts, mut same, mut speed) = (Vec::new(), Vec::new(), Vec::new());
    let mut links = Vec::new();
    let mut pairs = Vec::new();
    let (mut chunks, mut cands, mut weak, mut linked) = (0, 0, 0, 0);
    let mut group: Vec<usize> = (0..used.len()).collect();
    fn root(g: &mut [usize], x: usize) -> usize {
        if g[x] != x {
            let r = root(g, g[x]);
            g[x] = r;
        }
        g[x]
    }
    for (a, b, p) in found {
        let (wa, wb) = (used[*a], used[*b]);
        chunks += p.chunks;
        cands += p.candidates;
        weak += p.weak;
        if !p.hits.is_empty() {
            linked += 1;
            let (ra, rb) = (root(&mut group, *a), root(&mut group, *b));
            group[ra] = rb;
        }
        if audio_match::same_recording(&p.hits) {
            let corr = p.hits.iter().map(|h| h.corr).fold(0.0, f64::max);
            for (w, o, on_a) in [(wa, wb, true), (wb, wa, false)] {
                let (t0, t1) = span(&p.hits, on_a);
                same.push(Event {
                    clock: w.clock.clone(),
                    t_start: t0,
                    t_end: t1,
                    label: format!("same recording as {}", o.label),
                    detail: format!("the audio correlates {corr:.3} (two microphones stay under {}): a copy, not a second witness", audio_match::SAME_CORR),
                    error_s: 0.0,
                });
            }
        }
        if let Some(d) = audio_match::drift(&p.hits) {
            drifts.push(format!(
                "{{\"a\":\"{}\",\"b\":\"{}\",\"ppm\":{:.1},\"error_ppm\":{:.1},\"span_s\":{:.1},\"links\":{}}}",
                json_escape(&wa.label),
                json_escape(&wb.label),
                d.ppm,
                d.error_ppm,
                d.span_s,
                d.links
            ));
            if audio_match::speed_changed(&d) {
                let (t0, t1) = span(&p.hits, false);
                speed.push(Event {
                    clock: wb.clock.clone(),
                    t_start: t0,
                    t_end: t1,
                    label: format!("speed {:+.2} % against {}", d.ppm * 1e-4, wa.label),
                    detail: format!(
                        "{} plays {:.0} ± {:.0} ppm {} than {} over {:.0} s: far beyond clock drift ({} ppm), so one was sped up or slowed down",
                        wb.label,
                        d.ppm.abs(),
                        d.error_ppm,
                        if d.ppm > 0.0 { "faster" } else { "slower" },
                        wa.label,
                        d.span_s,
                        audio_match::SPEED_CHANGE_PPM
                    ),
                    error_s: 0.0,
                });
            }
        }
        for h in &p.hits {
            links.push(format!(
                "{{\"a\":\"{}\",\"t_a\":{:.6},\"b\":\"{}\",\"t_b\":{:.6},\"error_s\":{:.6},\"source\":\"audio match: {} / {}, PSR {:.1}, {} fingerprint votes\"}}",
                json_escape(&wa.clock),
                h.t_a,
                json_escape(&wb.clock),
                h.t_b,
                h.error_s,
                json_escape(&wa.label),
                json_escape(&wb.label),
                h.psr,
                h.votes
            ));
        }
        pairs.push(format!(
            "{{\"a\":\"{}\",\"b\":\"{}\",\"links\":{},\"chunks\":{},\"candidates\":{},\"unconfirmed\":{}}}",
            json_escape(&wa.label),
            json_escape(&wb.label),
            p.hits.len(),
            p.chunks,
            p.candidates,
            p.weak
        ));
    }
    let n = used.len();
    let groups = (0..n).filter(|&x| root(&mut group, x) == x).count();
    let level = if links.is_empty() {
        "none"
    } else if groups == 1 {
        "good"
    } else {
        "weak"
    };
    let verdict = match level {
        "none" => format!("no two of the {n} recordings hear the same sound"),
        "good" => format!("all {n} recordings linked into one group"),
        _ => format!("{n} recordings in {groups} groups: some hear nothing the others do"),
    };
    let errs: Vec<String> = errors.iter().map(|e| format!("\"{}\"", json_escape(e))).collect();
    let quality = format!(
        "{{\"level\":\"{level}\",\"verdict\":\"{}\",\"recordings\":{n},\"groups\":{groups},\"pairs\":{},\"pairs_linked\":{linked},\"links\":{},\"chunks\":{chunks},\"candidates\":{cands},\"unconfirmed\":{weak},\"errors\":[{}],\"per_pair\":[{}]}}",
        json_escape(&verdict),
        found.len(),
        links.len(),
        errs.join(","),
        pairs.join(",")
    );
    let mut status = format!("{} links, {linked} of {} pairs, {groups} group{}", links.len(), found.len(), if groups == 1 { "" } else { "s" });
    if !same.is_empty() {
        status += &format!(", {} same recording", same.len() / 2);
    }
    if !speed.is_empty() {
        status += &format!(", {} speed change", speed.len());
    }
    Results { links: format!("[{}]", links.join(",")), quality, drift: format!("[{}]", drifts.join(",")), same, speed, status }
}

/// The node's part of the result file: what it gave on each output.
pub fn json(s: &runtime::NodeState) -> String {
    let outs: Vec<String> = s.outbox.iter().map(|(n, d)| format!("\"{}\":{}", json_escape(n), d.to_json())).collect();
    format!(",\"outputs\":{{{}}}", outs.join(","))
}

/// How this node looks on the canvas: the standard look.
pub struct Look;

impl crate::look::NodeLook for Look {}
