//! The Audio match node in the workbench. Once the graph runs it takes the audio each wired
//! source decodes (16 kHz, kept by the SOURCER) as soon as that source is done, brings it to
//! 8 kHz (the fingerprints' rate, `audio_match::halve`), and fingerprints it, a core per
//! recording; a source that gave no audio has its file decoded here instead. Then it matches every
//! pair (`corroborate::audio_match`) on its own thread, and puts the links (with both clocks named) and their quality on its outputs; the
//! links go into the clock solve from there like any link output. Files only for now: a stream or
//! microphone wired in is named in the status and left out. Compiled into the `corroborate`
//! program.
//!
//! On the node, one row per wired recording, level with its input: amber is where the file's own
//! time stamp puts it (read as soon as it is wired, before any Run), teal where audio puts it once
//! the match has ended. Audio says only how the recordings lie against each other, so a linked
//! group is placed by real time when it has some (a time wired in), else by the best fit to its
//! stamps (`audio_match::fit_stamps`). Whether a stamp marks a recording's start or its end is the
//! person's decision, made from this view: a click on a row's note changes it, the buttons change
//! them all, and the line beside the buttons says how far the stamps disagree under each reading.
//! A wired time follows the decision. Wheel over the track zooms, a drag pans, a double-click fits
//! it again.

use super::*;
use corroborate::audio_match::{self, Clip, Placed, STAMP_CAP_S};
use corroborate::data::{Data, Event};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;
use std::thread;

/// How far one wired recording has got in a run.
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// Waiting for its source: seconds decoded so far.
    Waiting(f64),
    Fingerprinting,
    Ready(usize),
    Failed(String),
}

/// One wired recording: its source's label, clock and file, and the source's state with the run
/// it was on before this run started (its audio is this run's once its run differs and is done).
pub struct Wired {
    pub label: String,
    pub clock: String,
    pub path: String,
    pub source: Option<(Shared, u64)>,
}

static RUNS: AtomicU64 = AtomicU64::new(1);

impl App {
    /// The rows of node `i`'s timeline, with whether its axis is real time and what says so.
    pub(crate) fn match_rows(&self, i: NodeId, ctx: &egui::Context) -> (Vec<Row>, bool, String, Option<(f64, f64)>) {
        struct Raw {
            y: f32,
            name: String,
            stage: Option<Stage>,
            file: bool,
            stamp: Option<FileStamp>,
            len: f64,
            info: Option<corroborate::clock::ClockInfo>,
            uid: String,
        }
        let (done, stages) = {
            let s = self.nodes[i].state.lock().unwrap();
            (s.done, s.matching.clone())
        };
        let spec = &self.graph.nodes()[i].spec;
        let mut raw: Vec<Raw> = Vec::new();
        let mut file_k = 0;
        for (port, p) in spec.inputs.iter().enumerate().filter(|(_, p)| p.kind == PortKind::Audio) {
            let Some((f, _)) = self.graph.feeder(i, p.name) else { continue };
            let path = match &self.nodes[f].kind {
                Kind::Sourcer { input } => match sourcer_node::input(input) {
                    Some(runtime::Input::File(path)) => Some(path),
                    _ => None,
                },
                _ => None,
            };
            let stage = path.as_ref().and_then(|_| {
                file_k += 1;
                stages.get(file_k - 1).cloned()
            });
            let stamp = path.as_ref().and_then(|p| self.match_ui.stamps.ask(p, ctx)).flatten();
            let audio_s = self.nodes[f].state.lock().unwrap().audio_s;
            let info = self.clock_of(f).map(|c| self.ev.clocks.get(&c));
            raw.push(Raw {
                y: self.port_place(self.canvas_origin, i, port, false).0.y,
                name: self.nodes[f].label.clone(),
                stage,
                file: path.is_some(),
                len: stamp.as_ref().and_then(|s| s.len).filter(|l| *l > 0.0).unwrap_or(audio_s),
                stamp,
                info,
                uid: self.nodes[f].uid.clone(),
            });
        }
        // Linked groups, once the match has ended: two wired recordings or more on one group.
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        if done {
            for (k, r) in raw.iter().enumerate() {
                if let Some(info) = &r.info {
                    groups.entry(info.group.clone()).or_default().push(k);
                }
            }
        }
        let as_end = |r: &Raw| r.stamp.as_ref().is_some_and(|s| !s.sidecar) && self.ev.stamp_end.contains(&r.uid);
        // Per row, once placed: where audio puts it, its stamp's difference, whether real time.
        let mut placed: Vec<Option<(f64, Option<f64>, bool)>> = vec![None; raw.len()];
        let mut spreads: Option<(usize, f64, f64)> = None;
        for members in groups.values().filter(|m| m.len() >= 2) {
            let start = |k: usize| raw[k].info.as_ref().map_or(0.0, |c| c.at(0.0));
            let absolute = raw[members[0]].info.as_ref().is_some_and(|c| c.kind == corroborate::clock::Kind::Absolute);
            let recs: Vec<Placed> = members.iter().map(|&k| Placed { stamp: raw[k].stamp.as_ref().map(|s| s.t), len: raw[k].len, start: start(k), as_end: as_end(&raw[k]) }).collect();
            let fit = audio_match::fit_stamps(&recs, absolute.then_some(0.0));
            let zero = fit.zero.unwrap_or(0.0);
            for (j, &k) in members.iter().enumerate() {
                placed[k] = Some((zero + start(k), fit.diffs[j], absolute));
            }
            // The group with most stamps is the one the two readings are compared on.
            if let (Some(s), Some(e)) = (audio_match::stamp_spread(&recs, false), audio_match::stamp_spread(&recs, true)) {
                let n = recs.iter().filter(|r| r.stamp.is_some()).count();
                if spreads.is_none_or(|b| n > b.0) {
                    spreads = Some((n, s, e));
                }
            }
        }
        let any_time = raw.iter().any(|r| r.stamp.is_some()) || placed.iter().flatten().any(|p| p.2);
        let any_fit = placed.iter().flatten().any(|p| !p.2 && p.1.is_some());
        let any_absolute = placed.iter().flatten().any(|p| p.2);
        let time_note = match self.graph.feeder(i, "time") {
            Some((f, _)) if self.nodes[f].kind.is_source() => format!("time: {}", self.nodes[f].label),
            Some(_) => "time: only a recording's own time counts here".to_string(),
            None if any_absolute => "real time from another source".to_string(),
            None if any_fit => "audio placed by best fit".to_string(),
            None => "time: not wired".to_string(),
        };
        let rows = raw
            .iter()
            .zip(&placed)
            .map(|(r, p)| {
                let tick = r.stamp.as_ref().map(|s| s.t);
                let end = as_end(r);
                let word = if end { "end" } else { "start" };
                let amber = tick.map(|t| if end { (t - r.len, t) } else { (t, t + r.len) });
                let teal = p.map(|p| (p.0, p.0 + r.len));
                let (note, warn) = match (p, &r.stage) {
                    _ if !r.file => ("files only".to_string(), false),
                    (Some((_, Some(d), _)), _) if d.abs() < 3600.0 => (format!("{word} {d:+.1} s"), d.abs() > STAMP_CAP_S),
                    (Some((_, Some(d), _)), _) => (format!("{word} {}", gap_text(*d)), true),
                    (Some((_, None, _)), _) => ("no stamp".to_string(), false),
                    (None, Some(Stage::Failed(_))) => ("failed".to_string(), true),
                    (None, _) if done => ("not linked".to_string(), false),
                    (None, Some(Stage::Waiting(s))) if *s > 0.0 => (format!("decoding {s:.0} s"), false),
                    (None, Some(Stage::Waiting(_))) => ("waiting".to_string(), false),
                    (None, Some(Stage::Fingerprinting)) => ("fingerprinting".to_string(), false),
                    (None, Some(Stage::Ready(_))) => ("fingerprinted".to_string(), false),
                    (None, None) => (if r.stamp.is_some() { word.to_string() } else { "no stamp".to_string() }, false),
                };
                let mut hover = vec![r.name.clone()];
                match &r.stamp {
                    Some(s) if s.sidecar => hover.push(format!("started at {} UTC (our recorder's own time)", timebase::fmt_utc(s.t))),
                    Some(s) => hover.push(format!("file's time stamp: {} UTC", timebase::fmt_utc(s.t))),
                    None => hover.push("no time stamp in the file".to_string()),
                }
                if r.len > 0.0 {
                    hover.push(format!("length {:.2} s", r.len));
                }
                if let Some((at, d, absolute)) = p {
                    if let Some(d) = d {
                        let other = if end { d + r.len } else { d - r.len };
                        let (first, second) = if end { (other, *d) } else { (*d, other) };
                        hover.push(format!("from the group: {first:+.2} s if the stamp is its start, {second:+.2} s if its end"));
                        hover.push(format!("read as its {word} (click the note on the right to change it)"));
                    }
                    let e = r.info.as_ref().map_or(0.0, |c| c.error_at(0.0));
                    if *absolute {
                        hover.push(format!("audio: starts {} UTC ± {}", timebase::fmt_utc(*at), clock_view::err_text(e)));
                    } else {
                        hover.push(format!("audio: starts {} (best fit to the stamps); ± {} against the group", timebase::fmt_utc(*at), clock_view::err_text(e)));
                    }
                }
                if let Some(Stage::Ready(h)) = &r.stage {
                    hover.push(format!("{h} fingerprint hashes"));
                }
                if let Some(Stage::Failed(e)) = &r.stage {
                    hover.push(e.clone());
                }
                let uid = r.stamp.as_ref().filter(|s| !s.sidecar).map(|_| r.uid.clone());
                Row { y: r.y, name: r.name.clone(), note, warn, amber, tick, teal, hover: hover.join("\n"), uid, as_end: end }
            })
            .collect();
        (rows, any_time, time_note, spreads.map(|s| (s.1, s.2)))
    }

    /// The files wired into node `i`, and the labels of sources that are not files. `before`: each
    /// source's run before this run started.
    pub(crate) fn audio_match_inputs(&self, i: NodeId, before: &HashMap<NodeId, u64>) -> (Vec<Wired>, Vec<String>) {
        let (mut files, mut other) = (Vec::new(), Vec::new());
        for p in self.graph.nodes()[i].spec.inputs.iter().filter(|p| p.kind == PortKind::Audio) {
            let Some((f, _)) = self.graph.feeder(i, p.name) else { continue };
            let label = self.nodes[f].label.clone();
            match &self.nodes[f].kind {
                Kind::Sourcer { input } => match sourcer_node::input(input) {
                    Some(runtime::Input::File(path)) => {
                        let source = before.get(&f).map(|&r| (self.nodes[f].state.clone(), r));
                        files.push(Wired { label, clock: self.clock_of(f).unwrap_or_default(), path, source })
                    }
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
        s.matching = vec![Stage::Waiting(0.0); files.len()];
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
        s.status = format!("waiting for {} recordings to decode…", files.len());
    });
    let stop2 = stop.clone();
    thread::spawn(move || {
        let set = |f: &dyn Fn(&mut runtime::NodeState)| {
            let mut s = state.lock().unwrap();
            if s.run_id == run && !stop2.load(Ordering::Relaxed) {
                f(&mut s);
            }
        };
        // Stopped: no results, and it no longer counts as running.
        let stopped = || {
            let mut s = state.lock().unwrap();
            if s.run_id == run {
                s.running = false;
                s.status = "stopped".into();
            }
        };
        // A core per recording at a time: each waits for its source's audio (or decodes the
        // file), then fingerprints it.
        let next = std::sync::atomic::AtomicUsize::new(0);
        let ready = std::sync::atomic::AtomicUsize::new(0);
        let threads = thread::available_parallelism().map_or(4, |n| n.get()).min(files.len());
        let mut decoded: Vec<(usize, Result<Clip, String>)> = thread::scope(|sc| {
            let hs: Vec<_> = (0..threads)
                .map(|_| {
                    sc.spawn(|| {
                        let mut done = Vec::new();
                        loop {
                            let k = next.fetch_add(1, Ordering::Relaxed);
                            let Some(w) = files.get(k) else { break };
                            if stop2.load(Ordering::Relaxed) {
                                break;
                            }
                            let stage = |to: Stage| {
                                set(&|s| {
                                    if let Some(x) = s.matching.get_mut(k) {
                                        *x = to.clone();
                                    }
                                })
                            };
                            let audio = match &w.source {
                                Some((st, before)) => from_source(st, *before, &stop2, &|secs| stage(Stage::Waiting(secs))),
                                None => None,
                            };
                            let audio = match audio {
                                Some(a) if !a.is_empty() => Ok(a),
                                _ if stop2.load(Ordering::Relaxed) => Err("stopped".to_string()),
                                _ => decode(&w.path, &stop2),
                            };
                            if audio.is_ok() {
                                stage(Stage::Fingerprinting);
                            }
                            let clip = audio.map(Clip::new);
                            stage(match &clip {
                                Ok(c) => Stage::Ready(c.hashes()),
                                Err(e) => Stage::Failed(e.clone()),
                            });
                            done.push((k, clip));
                            let n = ready.fetch_add(1, Ordering::Relaxed) + 1;
                            set(&|s| s.status = format!("fingerprinted {n} of {} recordings…", files.len()));
                        }
                        done
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        let mut slots: Vec<Result<Clip, String>> = files.iter().map(|_| Err("decoder crashed".to_string())).collect();
        for (k, c) in decoded.drain(..) {
            slots[k] = c;
        }
        let decoded = slots;
        if stop2.load(Ordering::Relaxed) {
            return stopped();
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
        if stop2.load(Ordering::Relaxed) {
            return stopped();
        }
        let r = results(&used, &found, &errors);
        set(&|s| {
            s.outbox = vec![
                ("links", Data::Custom { kind: "link", json: r.links.clone() }),
                ("quality", Data::Custom { kind: "quality", json: r.quality.clone() }),
                ("clock drift", Data::Json(r.drift.clone())),
                ("same recording", Data::Events(r.same.clone())),
                ("speed change", Data::Events(r.speed.clone())),
                ("gaps", Data::Events(r.gaps.clone())),
            ];
            s.outbox_rev += 1;
            s.status = format!("{}{note}", r.status);
            s.running = false;
            s.done = true;
        });
    });
}

/// The audio source `st` decoded in this run (its run differs from `before` and is done), at the
/// match's rate. None: stopped, or the source ended with no audio (the file is decoded here then).
/// `report` is told the seconds decoded so far.
fn from_source(st: &Shared, before: u64, stop: &AtomicBool, report: &dyn Fn(f64)) -> Option<Vec<f32>> {
    loop {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        let audio = {
            let s = st.lock().unwrap();
            let secs = if s.run_id != before { s.audio_s } else { 0.0 };
            ((s.run_id != before && s.done && !s.running).then(|| s.source.audio.clone()), secs)
        };
        let (audio, secs) = audio;
        report(secs);
        if let Some(a) = audio {
            let a = a.lock().unwrap();
            return (!a.is_empty()).then(|| audio_match::halve(&a));
        }
        thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn decode(path: &str, stop: &AtomicBool) -> Result<Vec<f32>, String> {
    // Its turn among every decoder of the project (the sources' too).
    let Some(_turn) = corroborate::ingest::decode_turn(stop) else { return Err("stopped".into()) };
    let (mut child, rx) = corroborate::ingest::spawn_ffmpeg(path, audio_match::SR)?;
    let mut out = Vec::new();
    for c in rx {
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("stopped".into());
        }
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
    /// Where a pair's offset jumps: an event on the recording that lacks the stretch.
    gaps: Vec<Event>,
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
    let (mut drifts, mut same, mut speed, mut gaps) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
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
        let found_gaps = audio_match::gaps(&p.hits);
        for g in &found_gaps {
            let (w, o) = if g.on_a { (wa, wb) } else { (wb, wa) };
            gaps.push(Event {
                clock: w.clock.clone(),
                t_start: g.t0,
                t_end: g.t1,
                label: format!("{:.2} s missing against {}", g.len_s, o.label),
                detail: format!(
                    "somewhere between these moments {} lacks {:.3} ± {:.3} s that {} has: paused or cut here, or {} has an insert",
                    w.label, g.len_s, g.error_s, o.label, o.label
                ),
                error_s: g.error_s,
            });
        }
        // A jump bends a line through all the links: no drift for a pair with a gap.
        if let Some(d) = audio_match::drift(&p.hits).filter(|_| found_gaps.is_empty()) {
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
    if !gaps.is_empty() {
        status += &format!(", {} gap{}", gaps.len(), if gaps.len() == 1 { "" } else { "s" });
    }
    Results { links: format!("[{}]", links.join(",")), quality, drift: format!("[{}]", drifts.join(",")), same, speed, gaps, status }
}

/// The node's part of the result file: what it gave on each output.
pub fn json(s: &runtime::NodeState) -> String {
    let outs: Vec<String> = s.outbox.iter().map(|(n, d)| format!("\"{}\":{}", json_escape(n), d.to_json())).collect();
    format!(",\"outputs\":{{{}}}", outs.join(","))
}

// ---------------------------------------------------------------- the timeline in the node

/// A file's time stamp, as read from it.
#[derive(Clone, Debug, PartialEq)]
pub struct FileStamp {
    pub t: f64,
    pub len: Option<f64>,
    /// Our own recorder's time of the start, not the file's creation time.
    pub sidecar: bool,
}

fn read_stamp(path: &str) -> Option<FileStamp> {
    if let Some(a) = timebase::sidecar_anchor(path) {
        return Some(FileStamp { t: a.utc0, len: None, sidecar: true });
    }
    let (t, len) = timebase::file_creation_time(path)?;
    Some(FileStamp { t, len, sidecar: false })
}

#[derive(Default)]
struct StampsInner {
    read: HashMap<String, Option<FileStamp>>,
    queue: VecDeque<String>,
    busy: bool,
}

/// File stamps by path, read one file at a time on a thread of their own, once each.
#[derive(Clone, Default)]
pub struct Stamps(Arc<Mutex<StampsInner>>);

impl Stamps {
    /// What is known of `path`: None while it is being read, then the stamp or that it has none.
    fn ask(&self, path: &str, ctx: &egui::Context) -> Option<Option<FileStamp>> {
        let mut s = self.0.lock().unwrap();
        if let Some(r) = s.read.get(path) {
            return Some(r.clone());
        }
        if !s.queue.iter().any(|p| p == path) {
            s.queue.push_back(path.to_string());
        }
        if !s.busy {
            s.busy = true;
            let (me, ctx) = (self.0.clone(), ctx.clone());
            thread::spawn(move || loop {
                let Some(p) = me.lock().unwrap().queue.pop_front() else {
                    me.lock().unwrap().busy = false;
                    return;
                };
                let r = read_stamp(&p);
                me.lock().unwrap().read.insert(p, r);
                ctx.request_repaint();
            });
        }
        None
    }
}

/// What the timelines keep between frames.
#[derive(Default)]
pub struct MatchUi {
    stamps: Stamps,
    /// Axes moved by hand, by node (else fitted to the bars).
    pub views: HashMap<NodeId, (f64, f64)>,
    /// The tracks on screen: the wheel over one is not the canvas's.
    pub tracks: Vec<Rect>,
}

/// One recording's row, as drawn.
pub struct Row {
    pub y: f32,
    pub name: String,
    /// At the right: the stamp's reading and difference, or how far the recording has got.
    pub note: String,
    pub warn: bool,
    /// Where the file's stamp puts it, the stamp itself, and where audio puts it.
    pub amber: Option<(f64, f64)>,
    pub tick: Option<f64>,
    pub teal: Option<(f64, f64)>,
    pub hover: String,
    /// The source's id when its stamp can be read either way (the file's creation time), and how
    /// it is read.
    pub uid: Option<String>,
    pub as_end: bool,
}

const AMBER: Color32 = Color32::from_rgb(235, 165, 55);
const TEAL: Color32 = Color32::from_rgb(55, 190, 215);

/// `+12 years`, `-3 min`, `+40 s`.
fn gap_text(s: f64) -> String {
    let a = s.abs();
    let sign = if s < 0.0 { "-" } else { "+" };
    if a < 120.0 {
        format!("{sign}{a:.0} s")
    } else if a < 7200.0 {
        format!("{sign}{:.0} min", a / 60.0)
    } else if a < 172_800.0 {
        format!("{sign}{:.0} h", a / 3600.0)
    } else if a < 63_072_000.0 {
        format!("{sign}{:.0} days", a / 86_400.0)
    } else {
        format!("{sign}{:.0} years", a / 31_557_600.0)
    }
}

/// The axis that fits the bars, leaving out any far from the rest (a clock years off).
fn fit_axis(bars: &[(f64, f64)]) -> (f64, f64) {
    if bars.is_empty() {
        return (0.0, 600.0);
    }
    let mid = |b: &(f64, f64)| (b.0 + b.1) / 2.0;
    let mut mids: Vec<f64> = bars.iter().map(mid).collect();
    mids.sort_by(f64::total_cmp);
    let m = mids[mids.len() / 2];
    let mut devs: Vec<f64> = mids.iter().map(|x| (x - m).abs()).collect();
    devs.sort_by(f64::total_cmp);
    let far = (devs[devs.len() / 2] * 6.0).max(3600.0);
    let near = || bars.iter().filter(|b| (mid(b) - m).abs() <= far);
    let lo = near().map(|b| b.0).fold(f64::MAX, f64::min);
    let hi = near().map(|b| b.1).fold(f64::MIN, f64::max);
    let pad = ((hi - lo) * 0.04).max(1.0);
    (lo - pad, hi + pad)
}

/// A round step giving about six ticks across `span` seconds.
fn tick_step(span: f64) -> f64 {
    let want = span / 6.0;
    [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0, 21600.0, 86400.0].into_iter().find(|s| *s >= want).unwrap_or(want)
}

/// How this node looks on the canvas: its timeline of file stamps and audio sync.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, _app: &App, _i: NodeId, standard: Vec2) -> Vec2 {
        Vec2::new(standard.x.max(860.0), standard.y.max(HEAD + 7.0 * ROW + 26.0))
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let (rows, real_time, time_note, spreads) = app.match_rows(i, ui.ctx());
        let z = app.zoom;
        let (name_w, note_w) = (135.0 * z, 86.0 * z);
        let track = Rect::from_min_max(Pos2::new(r.left() + name_w, r.top()), Pos2::new(r.right() - note_w, r.bottom()));
        if track.width() <= 10.0 {
            return;
        }
        let bars: Vec<(f64, f64)> = rows.iter().flat_map(|w| [w.amber, w.teal]).flatten().collect();
        let (v0, v1) = app.match_ui.views.get(&i).copied().unwrap_or_else(|| fit_axis(&bars));
        let span = (v1 - v0).max(1e-3);
        let tx = |t: f64| track.left() + ((t - v0) / span) as f32 * track.width();
        let p = ui.painter().with_clip_rect(ui.clip_rect());
        let font = |s: f32| FontId::proportional(s * z);
        let row_h = ROW * z;
        p.rect_filled(track, 2.0 * z, Color32::from_rgb(30, 33, 37));
        // The ruler: the row of the time input, above the recordings.
        let ruler = r.top() + 9.0 * z;
        p.text(Pos2::new(r.left(), ruler), Align2::LEFT_CENTER, shorten(&time_note, 25), font(10.5), if time_note == "time: not wired" { DIM } else { TEXT });
        let step = tick_step(span);
        let mut k = (v0 / step).ceil();
        while k * step <= v1 {
            let x = tx(k * step);
            p.line_segment([Pos2::new(x, track.top()), Pos2::new(x, track.bottom())], Stroke::new(1.0, alpha(LINE, 0.55)));
            let label = if real_time { tick_text(k * step, step) } else { format!("{:+.0} s", k * step) };
            p.text(Pos2::new(x + 3.0 * z, ruler), Align2::LEFT_CENTER, label, font(10.0), DIM);
            k += 1.0;
        }
        for w in &rows {
            p.text(Pos2::new(r.left(), w.y), Align2::LEFT_CENTER, shorten(&w.name, 22), font(11.0), TEXT);
            let band = |b: (f64, f64)| {
                let (a, c) = (tx(b.0).max(track.left()), tx(b.1).min(track.right()));
                (c > a).then(|| Rect::from_min_max(Pos2::new(a, w.y - row_h * 0.36), Pos2::new(c.max(a + 2.0), w.y + row_h * 0.36)))
            };
            let (mut gone, mut shown) = (None, false);
            for (bar, col) in [(w.amber, AMBER), (w.teal, TEAL)] {
                let Some(b) = bar else { continue };
                match band(b) {
                    Some(rect) => {
                        p.rect_filled(rect, 2.0 * z, alpha(col, 0.5));
                        p.rect_stroke(rect, 2.0 * z, Stroke::new(z, alpha(col, 0.9)), StrokeKind::Inside);
                        shown = true;
                    }
                    None => gone = Some(if b.1 < v0 { b.1 - v0 } else { b.0 - v1 }),
                }
            }
            if let Some(t) = w.tick.filter(|t| *t >= v0 && *t <= v1) {
                let x = tx(t);
                p.line_segment([Pos2::new(x, w.y - row_h * 0.46), Pos2::new(x, w.y + row_h * 0.46)], Stroke::new(1.5 * z, AMBER));
            }
            if let Some(g) = gone.filter(|_| !shown) {
                let (at, side, text) = if g < 0.0 { (track.left() + 4.0 * z, Align2::LEFT_CENTER, format!("< {}", gap_text(g))) } else { (track.right() - 4.0 * z, Align2::RIGHT_CENTER, format!("{} >", gap_text(g))) };
                p.text(Pos2::new(at, w.y), side, text, font(10.0), DIM);
            }
            p.text(Pos2::new(track.right() + 6.0 * z, w.y), Align2::LEFT_CENTER, &w.note, font(10.5), if w.warn { WARN } else { DIM });
        }
        // The person's decision: a click on a note changes that row's reading.
        let mut read: Vec<(Vec<String>, bool)> = Vec::new();
        for (k, w) in rows.iter().enumerate() {
            let Some(uid) = &w.uid else { continue };
            let rect = Rect::from_min_max(Pos2::new(track.right() + 2.0 * z, w.y - row_h / 2.0), Pos2::new(r.right(), w.y + row_h / 2.0));
            let them = if w.as_end { "start" } else { "end" };
            if ui.interact(rect, egui::Id::new(("match-note", i, k)), Sense::click()).on_hover_text(format!("click: this stamp marks the recording's {them}")).clicked() {
                read.push((vec![uid.clone()], !w.as_end));
            }
        }
        // Under the rows: every stamp at once, and how far they disagree under each reading.
        let all: Vec<String> = rows.iter().filter_map(|w| w.uid.clone()).collect();
        if let (false, Some(last)) = (all.is_empty(), rows.last()) {
            let y = last.y + row_h;
            let button = |ui: &mut egui::Ui, x: f32, text: &str| ui.put(Rect::from_center_size(Pos2::new(x, y), Vec2::new(58.0 * z, 16.0 * z)), egui::Button::new(egui::RichText::new(text).size(10.0 * z))).clicked();
            if button(ui, r.left() + 30.0 * z, "all start") {
                read.push((all.clone(), false));
            }
            if button(ui, r.left() + 94.0 * z, "all end") {
                read.push((all.clone(), true));
            }
            if let Some((s, e)) = spreads {
                p.text(Pos2::new(track.left() + 6.0 * z, y), Align2::LEFT_CENTER, format!("stamps disagree by about {s:.1} s if all mark the start, {e:.1} s if all mark the end"), font(10.5), TEXT);
            }
        }
        for (uids, end) in read {
            app.set_stamp_end(&uids, end);
        }
        // The track: wheel zooms around the pointer, a drag pans, a double-click fits it again.
        app.match_ui.tracks.push(track);
        let resp = ui.interact(track, egui::Id::new(("match-track", i)), Sense::click_and_drag());
        if let Some(h) = resp.hover_pos() {
            let scroll = ui.input(|k| k.smooth_scroll_delta.y + k.smooth_scroll_delta.x);
            if scroll != 0.0 {
                let f = (-scroll as f64 * 0.002).exp();
                let at = v0 + ((h.x - track.left()) / track.width()) as f64 * span;
                app.match_ui.views.insert(i, (at - (at - v0) * f, at + (v1 - at) * f));
            }
        }
        if resp.dragged() {
            let dt = (resp.drag_delta().x / track.width()) as f64 * span;
            let (a, b) = app.match_ui.views.get(&i).copied().unwrap_or((v0, v1));
            app.match_ui.views.insert(i, (a - dt, b - dt));
        }
        if resp.double_clicked() {
            app.match_ui.views.remove(&i);
        }
        let hovered = resp.hover_pos().and_then(|h| rows.iter().find(|w| (w.y - h.y).abs() <= row_h / 2.0));
        if let Some(w) = hovered {
            resp.on_hover_text(w.hover.clone());
        }
    }
}

fn shorten(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    } else {
        s.to_string()
    }
}

/// A tick's time of day: seconds shown only when the ticks are closer than a minute.
fn tick_text(t: f64, step: f64) -> String {
    let c = timebase::fmt_utc_clock(t);
    if step >= 60.0 && c.len() >= 5 {
        c[..5].to_string()
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_leaves_out_a_clock_years_off() {
        let t = 1_347_000_000.0;
        let (a, b) = fit_axis(&[(t, t + 100.0), (t + 50.0, t + 200.0), (t + 40.0, t + 90.0), (t + 4e8, t + 4e8 + 60.0)]);
        assert!(a < t && a > t - 60.0 && b > t + 200.0 && b < t + 260.0, "{a} {b}");
        assert_eq!(fit_axis(&[]), (0.0, 600.0));
    }

    #[test]
    fn gaps_read_plainly() {
        assert_eq!(gap_text(40.2), "+40 s");
        assert_eq!(gap_text(-180.0), "-3 min");
        assert_eq!(gap_text(4e8), "+13 years");
    }
}
