//! The Audio match node in the workbench: the time module. Each row takes one recording's audio and
//! the time its metadata claims, and gives a better time on its yellow output.
//!
//! Once the graph runs it takes the audio each wired source decodes (16 kHz, kept by the SOURCER) as
//! soon as that source is done, brings it to 8 kHz (the fingerprints' rate, `audio_match::halve`), and
//! fingerprints it, a core per recording; a source that gave no audio has its file decoded here
//! instead. Then it matches every pair (`corroborate::audio_match`) on its own thread, and puts the
//! links (with both clocks named) and their quality on its outputs; the links go into the clock solve
//! from there like any link output. Files only for now: a stream or microphone wired in is named in
//! the status and left out. Compiled into the `corroborate` program.
//!
//! On the node, one row per recording: blue is where its time input puts it, amber where audio puts
//! it. Recordings linked by audio are a synced group, which exists only here. Audio says only how a
//! group's recordings lie against each other, so until the group has an anchor its amber bars are
//! merely shown with the middle of the bars on the middle of its blue ones, and the person may drag
//! them; nothing comes out. A person picks one row's time as the group's anchor: the audio's start
//! snaps to it, every other row keeps its real offset, and the group's times leave on the yellow
//! outputs through the clock solve. Nothing is ever hidden or left out for being far off: a time 200
//! years away stays in view until a person corrects it in a Time node.

use super::*;
use corroborate::audio_match::{self, Clip, Placed};
use corroborate::data::{Data, Event};
use corroborate::module_manifest::{PortLabel, PortLook, Side};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
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

/// One recording's row. Times are seconds since 1970 when `MatchView::real`.
#[derive(Clone, Debug, Default)]
pub struct Row {
    /// Its number: the row of ports `audio n` and `time n`.
    pub n: usize,
    pub name: String,
    /// The source's stable id and clock, when audio is wired in.
    pub uid: Option<String>,
    pub clock: Option<String>,
    pub file: bool,
    /// A wire goes into its audio or time port (else: the free row at the end).
    pub wired: bool,
    /// The media's length, seconds.
    pub len: f64,
    /// What its time input says: the moment, its error and where it comes from.
    pub meta: Option<f64>,
    pub meta_err: f64,
    pub meta_source: String,
    /// The time input is wired to a Time node that has an error, and the error.
    pub time_err: Option<String>,
    pub time_wired: bool,
    /// Where audio puts its start (shown after a match), and how far from there the time input is.
    pub start: Option<f64>,
    pub off: Option<f64>,
    /// In `MatchView::groups`.
    pub group: usize,
    pub stage: Option<Stage>,
    /// The offset is about the recording's own length.
    pub hint: bool,
    /// It was in an anchored group before this run and is not any more.
    pub lost: bool,
}

/// Recordings audio links together, or one alone.
#[derive(Clone, Debug, Default)]
pub struct Group {
    /// What names it while it lives: the smallest clock id in it.
    pub key: String,
    /// Shown on its border; 0: one recording alone, which has no border.
    pub number: usize,
    pub rows: Vec<usize>,
    /// The row holding its anchor.
    pub anchor: Option<usize>,
}

/// What a node's rows are this frame.
#[derive(Debug, Default)]
pub struct MatchView {
    pub rows: Vec<Row>,
    pub groups: Vec<Group>,
    pub done: bool,
    pub running: bool,
    /// Its axis is real time: some row has a time.
    pub real: bool,
    /// The stamps of the group the hint is about agree better read as ends: how far they disagree
    /// as starts and as ends, by group.
    pub spreads: Vec<Option<(f64, f64)>>,
}

impl MatchView {
    pub fn anchored(&self, r: &Row) -> bool {
        self.groups.get(r.group).is_some_and(|g| g.anchor.is_some())
    }
}

/// How close to the recording's own length an offset must be to hint at it, seconds (5 s for now).
const HINT_S: f64 = 5.0;

/// A node's saved rows: which row holds an anchor is kept by the recording's id.
fn anchor_key(uid: &str) -> String {
    format!("anchor {uid}")
}

/// The middle of a recording.
fn mid(t: f64, len: f64) -> f64 {
    t + len / 2.0
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// Its file's name, when the node still has the module's name.
fn row_name(label: &str, path: Option<&str>) -> String {
    match path {
        Some(p) if label == "SOURCER" => p.rsplit(['\\', '/']).next().unwrap_or(p).to_string(),
        _ => label.to_string(),
    }
}

impl App {
    /// What node `i`'s rows are now. Worked out once a frame (the outputs, the node and the clock
    /// evidence all ask).
    pub(crate) fn match_view(&self, i: NodeId) -> Rc<MatchView> {
        #[cfg(not(test))]
        if let Some(v) = self.match_ui.cache.borrow().get(&i) {
            return v.clone();
        }
        let v = Rc::new(self.match_view_now(i));
        #[cfg(not(test))]
        self.match_ui.cache.borrow_mut().insert(i, v.clone());
        v
    }

    fn match_view_now(&self, i: NodeId) -> MatchView {
        let (done, running, stages) = {
            let s = self.nodes[i].state.lock().unwrap();
            (s.done, s.running, s.matching.clone())
        };
        let saved: BTreeSet<&str> = self.nodes[i].kind.rows().iter().filter_map(|(k, _)| k.strip_prefix("anchor ")).collect();
        let spec = &self.graph.nodes()[i].spec;
        let mut rows: Vec<Row> = Vec::new();
        let mut file_k = 0;
        for (k, pair) in spec.inputs.chunks(2).enumerate() {
            let mut r = Row { n: k + 1, ..Default::default() };
            let [audio, time] = pair else { continue };
            if let Some((f, _)) = self.graph.feeder(i, audio.name) {
                let path = self.source_file(f);
                r.name = row_name(&self.nodes[f].label, path.as_deref());
                r.uid = Some(self.nodes[f].uid.clone());
                r.clock = self.clock_of(f);
                r.file = path.is_some();
                r.len = self.source_length(f).unwrap_or_else(|| self.nodes[f].state.lock().unwrap().audio_s);
                if r.file {
                    r.stage = stages.get(file_k).cloned();
                    file_k += 1;
                }
                r.wired = true;
            }
            if let Some((f, o)) = self.graph.feeder(i, time.name) {
                r.time_wired = true;
                r.wired = true;
                match self.value_from(f, o) {
                    Ok(Data::Time { utc: Some(u), error_s, source, .. }) => (r.meta, r.meta_err, r.meta_source) = (Some(u), error_s, source),
                    Ok(_) => {}
                    Err(e) if matches!(self.nodes[f].kind, Kind::Time { .. }) => r.time_err = Some(e),
                    Err(_) => {}
                }
            }
            rows.push(r);
        }
        // Synced groups: rows on one clock, and rows this node's links join.
        let mut parent: Vec<usize> = (0..rows.len()).collect();
        fn root(p: &mut [usize], mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        let by_clock: BTreeMap<String, usize> = rows.iter().enumerate().rev().filter_map(|(k, r)| Some((r.clock.clone()?, k))).collect();
        for (k, r) in rows.iter().enumerate() {
            if let Some(&o) = r.clock.as_ref().and_then(|c| by_clock.get(c)) {
                let (a, b) = (root(&mut parent, k), root(&mut parent, o));
                parent[a.max(b)] = a.min(b);
            }
        }
        if done {
            let uid = &self.nodes[i].uid;
            for l in self.ev.links.iter().filter(|l| l.enabled && l.from == *uid) {
                if let (Some(&a), Some(&b)) = (by_clock.get(l.a.as_str()), by_clock.get(l.b.as_str())) {
                    let (a, b) = (root(&mut parent, a), root(&mut parent, b));
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
        let mut groups: Vec<Group> = Vec::new();
        let mut at_root: BTreeMap<usize, usize> = BTreeMap::new();
        for k in 0..rows.len() {
            let g = *at_root.entry(root(&mut parent, k)).or_insert_with(|| {
                groups.push(Group::default());
                groups.len() - 1
            });
            groups[g].rows.push(k);
            rows[k].group = g;
        }
        let mut number = 0;
        for g in &mut groups {
            g.key = g.rows.iter().filter_map(|&k| rows[k].clock.clone()).min().unwrap_or_default();
            if g.rows.iter().filter(|&&k| rows[k].wired).count() >= 2 && done {
                number += 1;
                g.number = number;
            }
            g.anchor = g.rows.iter().copied().find(|&k| rows[k].meta.is_some() && rows[k].uid.as_deref().is_some_and(|u| saved.contains(u)));
        }
        let shift_of = |g: &Group| self.match_ui.shift.get(&(i, g.key.clone())).copied().unwrap_or(0.0);
        let real = rows.iter().any(|r| r.meta.is_some());
        // The middle of every time there is: where a group with no time at all is shown.
        let all_mids: Vec<f64> = rows.iter().filter_map(|r| r.meta.map(|t| mid(t, r.len))).collect();
        let mut spreads = vec![None; groups.len()];
        if done {
            for (gi, g) in groups.iter().enumerate() {
                let starts: Vec<f64> = rows.iter().map(|r| r.clock.as_deref().map_or(0.0, |c| self.ev.clocks.get(c).at(0.0))).collect();
                let start = |k: usize| starts[k];
                let audio_rows: Vec<usize> = g.rows.iter().copied().filter(|&k| rows[k].clock.is_some()).collect();
                let timed: Vec<usize> = audio_rows.iter().copied().filter(|&k| rows[k].meta.is_some()).collect();
                let linked = audio_rows.len() >= 2;
                if !linked && timed.is_empty() {
                    continue; // alone, with no time: nowhere to show it
                }
                let shift = match g.anchor {
                    Some(a) if rows[a].clock.as_deref().is_some_and(|c| self.ev.clocks.get(c).kind == corroborate::clock::Kind::Absolute) => 0.0,
                    Some(a) => rows[a].meta.unwrap_or(0.0) - start(a),
                    None => {
                        let metas: Vec<f64> = timed.iter().map(|&k| mid(rows[k].meta.unwrap_or(0.0), rows[k].len)).collect();
                        let over: &[usize] = if timed.is_empty() { &audio_rows } else { &timed };
                        let audios: Vec<f64> = over.iter().map(|&k| mid(start(k), rows[k].len)).collect();
                        let centre = mean(&metas).or(mean(&all_mids)).unwrap_or(0.0);
                        centre - mean(&audios).unwrap_or(0.0) + shift_of(g)
                    }
                };
                for &k in &audio_rows {
                    let at = start(k) + shift;
                    rows[k].start = Some(at);
                    rows[k].off = rows[k].meta.map(|t| t - at);
                }
                if linked && g.anchor.is_none() {
                    let recs: Vec<Placed> = audio_rows.iter().map(|&k| Placed { stamp: rows[k].meta, len: rows[k].len, start: start(k) }).collect();
                    spreads[gi] = audio_match::stamp_spread(&recs, false).zip(audio_match::stamp_spread(&recs, true));
                }
                // How far off is about its own length: compared directly once anchored; before an
                // anchor the average has absorbed it, so the stamps' agreement decides.
                let as_ends = spreads[gi].is_some_and(|(s, e): (f64, f64)| s - e > 1.0 && e * 2.0 < s);
                for &k in &audio_rows {
                    rows[k].hint = rows[k].meta.is_some() && if g.anchor.is_some() { rows[k].off.is_some_and(|d| (d.abs() - rows[k].len).abs() <= HINT_S) } else { as_ends };
                }
            }
            if let Some(lost) = self.match_ui.lost.get(&i) {
                for r in rows.iter_mut() {
                    r.lost = r.uid.as_ref().is_some_and(|u| lost.contains(u));
                }
            }
        }
        MatchView { rows, groups, done, running, real, spreads }
    }

    /// The anchors node `i`'s groups have: the audio's start is at that row's time.
    pub(crate) fn match_anchors(&self, i: NodeId) -> Vec<corroborate::clock::Anchor> {
        let v = self.match_view(i);
        let uid = self.nodes[i].uid.clone();
        v.groups
            .iter()
            .filter_map(|g| {
                let r = &v.rows[g.anchor?];
                Some(corroborate::clock::Anchor { clock: r.clock.clone()?, t: 0.0, utc: r.meta?, error_s: r.meta_err, from: uid.clone(), enabled: true })
            })
            .collect()
    }

    /// The better time on output `out` ("synced 3"): none until its group has an anchor.
    pub(crate) fn synced_time(&self, i: NodeId, out: &str) -> Result<Data, String> {
        let n: usize = out.strip_prefix("synced ").and_then(|n| n.parse().ok()).ok_or("no such output")?;
        let v = self.match_view(i);
        let r = v.rows.get(n - 1).ok_or("no such row")?;
        let clock = r.clock.clone().ok_or("no audio wired into this row")?;
        let g = &v.groups[r.group];
        let a = g.anchor.ok_or("its synced group has no anchor")?;
        let (utc, error_s) = self.ev.clocks.to_utc(&clock, 0.0).ok_or("its synced group has no anchor")?;
        Ok(Data::Time { clock, t: 0.0, utc: Some(utc), error_s, source: format!("Audio match, anchored on the time of {}", v.rows[a].name) })
    }

    /// Row `row` becomes its group's anchor, or stops being it. Kept by the recording, so it stays
    /// with the recording after a new run. Logged like any setting.
    pub(crate) fn toggle_anchor(&mut self, i: NodeId, row: usize) {
        let v = self.match_view(i);
        let Some(r) = v.rows.get(row) else { return };
        let Some(uid) = r.uid.clone().filter(|_| r.meta.is_some()) else { return };
        let g = &v.groups[r.group];
        let was = g.anchor;
        let members: Vec<String> = g.rows.iter().filter_map(|&k| v.rows[k].uid.clone()).collect();
        let name = r.name.clone();
        let old = was.map_or("none".to_string(), |a| v.rows[a].name.clone());
        let on = was != Some(row);
        let Kind::AudioMatch { rows } = &mut self.nodes[i].kind else { return };
        rows.retain(|(k, _)| !k.strip_prefix("anchor ").is_some_and(|u| members.iter().any(|m| m == u)));
        if on {
            rows.push((anchor_key(&uid), "on".to_string()));
            self.match_ui.members.insert((i, uid.clone()), members.iter().cloned().collect());
        }
        if let Some(l) = self.match_ui.lost.get_mut(&i) {
            l.retain(|u| !members.contains(u));
        }
        let label = self.nodes[i].label.clone();
        self.changes.push(bus::SettingChange { utc: timebase::now_unix(), node: self.nodes[i].uid.clone(), label, name: format!("anchor of the group with {name}"), old, new: if on { name.clone() } else { "none".to_string() } });
        self.match_ui.cache.borrow_mut().remove(&i);
        self.save_project();
    }

    /// A new result: what left an anchored group since the last one has lost its anchor.
    fn note_lost_anchors(&mut self, i: NodeId, v: &MatchView) {
        let run = self.nodes[i].state.lock().unwrap().run_id;
        if !v.done || self.match_ui.seen_run.get(&i) == Some(&run) {
            return;
        }
        self.match_ui.seen_run.insert(i, run);
        let mut lost: BTreeSet<String> = BTreeSet::new();
        for g in v.groups.iter().filter(|g| g.anchor.is_some()) {
            let holder = v.rows[g.anchor.unwrap_or(0)].uid.clone().unwrap_or_default();
            let now: BTreeSet<String> = g.rows.iter().filter_map(|&k| v.rows[k].uid.clone()).collect();
            if let Some(before) = self.match_ui.members.get(&(i, holder.clone())) {
                lost.extend(before.difference(&now).cloned());
            }
            self.match_ui.members.insert((i, holder), now);
        }
        self.match_ui.lost.insert(i, lost);
        self.match_ui.cache.borrow_mut().remove(&i);
    }

    /// Dropping a SOURCER's audio on a row also draws its time wire, when that row has none.
    pub(crate) fn also_wire_time(&mut self, from: NodeId, fk: usize, to: NodeId, tk: usize) {
        if !matches!(self.nodes[to].kind, Kind::AudioMatch { .. }) || !matches!(self.nodes[from].kind, Kind::Sourcer { .. }) {
            return;
        }
        let (out, inp) = (self.graph.nodes()[from].spec.outputs[fk].name, self.graph.nodes()[to].spec.inputs[tk].name);
        let Some(n) = inp.strip_prefix("audio ") else { return };
        let time = format!("time {n}");
        if out == "audio" && self.graph.feeder(to, &time).is_none() {
            self.wire(from, "time", to, &time);
        }
    }

    /// A project saved before rows had a time each: every recording's time is wired to its row
    /// (through a Time node taking its length away when the person read its stamp as the end), and
    /// the recording whose time was wired into the old time input holds the anchor.
    pub(crate) fn migrate_match(&mut self, i: NodeId, old_time: &[NodeId]) {
        let inputs = self.graph.nodes()[i].spec.inputs.clone();
        let origin = self.nodes[i].pos;
        let mut made = 0.0;
        for pair in inputs.chunks(2) {
            let [audio, time] = pair else { continue };
            let Some((f, _)) = self.graph.feeder(i, audio.name) else { continue };
            if !matches!(self.nodes[f].kind, Kind::Sourcer { .. }) {
                continue;
            }
            let uid = self.nodes[f].uid.clone();
            if self.ev.stamp_end.contains(&uid) {
                let mut rows = corroborate::modules::time::defaults();
                corroborate::modules::time::add_row(&mut rows);
                made += 1.0;
                let view = self.view;
                self.view = self.nodes[i].parent;
                let t = self.add(Kind::Time { rows }, origin - Vec2::new(0.0, made * 200.0));
                self.view = view;
                self.nodes[t].label = format!("Time: end of {}", self.nodes[f].label);
                let inputs = self.nodes[t].kind.spec().inputs;
                self.graph.set_inputs_keep(t, inputs);
                self.wire(f, "time", t, "time");
                self.wire(f, "length", t, "row 1");
                self.wire(t, "result", i, time.name);
            } else {
                self.wire(f, "time", i, time.name);
            }
        }
        let anchors: Vec<String> = old_time.iter().map(|&s| anchor_key(&self.nodes[s].uid)).collect();
        if let Kind::AudioMatch { rows } = &mut self.nodes[i].kind {
            rows.extend(anchors.into_iter().map(|k| (k, "on".to_string())));
        }
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

/// The node's part of the result file: what it gave on each output, and each row's better time.
pub fn json(app: &App, i: NodeId) -> String {
    let outs: Vec<String> = app.nodes[i].state.lock().unwrap().outbox.iter().map(|(n, d)| format!("\"{}\":{}", json_escape(n), d.to_json())).collect();
    let v = app.match_view(i);
    let rows: Vec<String> = v
        .rows
        .iter()
        .filter(|r| r.wired)
        .map(|r| {
            let meta = r.meta.map_or("null".to_string(), |t| format!("\"{}\"", timebase::fmt_utc(t)));
            let synced = app.synced_time(i, &format!("synced {}", r.n)).map_or("null".to_string(), |d| d.to_json());
            let g = &v.groups[r.group];
            format!("{{\"row\":{},\"recording\":\"{}\",\"time_in\":{meta},\"group\":{},\"anchor\":{},\"synced\":{synced}}}", r.n, json_escape(&r.name), g.number, g.anchor == Some(r.n - 1))
        })
        .collect();
    format!(",\"outputs\":{{{}}},\"rows\":[{}]", outs.join(","), rows.join(","))
}

// ---------------------------------------------------------------- the rows on the node

/// What the rows keep between frames.
#[derive(Default)]
pub struct MatchUi {
    /// Axes moved by hand, by node. None: Scale to all is live, the axis follows the bars.
    pub views: HashMap<NodeId, (f64, f64)>,
    /// The tracks on screen: the wheel over one is not the canvas's.
    pub tracks: Vec<Rect>,
    /// How far each unanchored group was dragged, by node and group. It changes no data.
    pub shift: HashMap<(NodeId, String), f64>,
    /// The group being dragged and the axis held still meanwhile.
    grab: Option<(NodeId, String, (f64, f64))>,
    /// Which row is drawn where (row → place), by node, as last drawn (Audio match and Timeline).
    pub slots: HashMap<NodeId, Vec<usize>>,
    /// Who was in each anchored group at the last result, by node and anchor; who has left it since.
    members: HashMap<(NodeId, String), BTreeSet<String>>,
    lost: HashMap<NodeId, BTreeSet<String>>,
    seen_run: HashMap<NodeId, u64>,
    /// Where each row's anchor button is on screen, as last drawn.
    pub anchor_at: HashMap<(NodeId, usize), Rect>,
    /// Each node's rows this frame (cleared at the start of the canvas).
    pub cache: std::cell::RefCell<HashMap<NodeId, Rc<MatchView>>>,
}

// Unzoomed, from the top of the node.
const NODE_W: f32 = 980.0;
const TOP: f32 = HEAD + 4.0;
const BAR_H: f32 = 30.0;
const RULER_H: f32 = 26.0;
const FOOT_H: f32 = 46.0;
const LEFT_W: f32 = 170.0;
const STATUS_W: f32 = 116.0;
const RIGHT_W: f32 = 116.0;
const ROW_DEFAULT: f32 = 64.0;
/// The thinnest row is its file name and nothing else.
const ROW_MIN: f32 = STRIP_H + 2.0;
const ROW_MAX: f32 = 96.0;
const STRIP_H: f32 = 20.0;
/// With ports at least this tall, "audio" and "time" are written on them.
const WORDS_FROM: f32 = 14.0;
/// With ports shorter than this, the anchor button moves up beside the file name.
const ANCHOR_FROM: f32 = 10.0;
/// How far the node's edge is outside the part a look draws in.
const EDGE: f32 = 6.0;

const AMBER: Color32 = Color32::from_rgb(233, 168, 76);
const BLUE: Color32 = Color32::from_rgb(125, 180, 230);
const BOTH: Color32 = Color32::from_rgb(246, 236, 217);
const ANCHOR_BLUE: Color32 = Color32::from_rgb(34, 89, 143);
const OFF: Color32 = Color32::from_rgb(65, 70, 78);
const STRIP: Color32 = Color32::from_rgb(53, 58, 66);
const TRACK: Color32 = Color32::from_rgb(29, 32, 37);
const INK: Color32 = Color32::from_rgb(16, 18, 20);
const SEAM: Color32 = Color32::from_rgb(16, 18, 21);
const GRID: Color32 = Color32::from_rgb(49, 53, 60);
/// How solid the blue and amber bars are.
const BAR_ALPHA: f32 = 0.6;
const RED: Color32 = Color32::from_rgb(224, 82, 79);
/// Group borders, in the order of their numbers.
const GROUP_COLORS: [Color32; 8] = [
    Color32::from_rgb(181, 140, 255),
    Color32::from_rgb(240, 139, 192),
    Color32::from_rgb(92, 200, 176),
    Color32::from_rgb(230, 200, 90),
    Color32::from_rgb(120, 190, 250),
    Color32::from_rgb(250, 150, 100),
    Color32::from_rgb(170, 220, 110),
    Color32::from_rgb(210, 170, 230),
];

const YEAR: f64 = 31_557_600.0;

fn row_height(kind: &Kind) -> f32 {
    kind.rows().iter().find(|(k, _)| k == "row height").and_then(|(_, v)| v.parse().ok()).unwrap_or(ROW_DEFAULT).clamp(ROW_MIN, ROW_MAX)
}

fn by_group(kind: &Kind) -> bool {
    kind.rows().iter().any(|(k, v)| k == "order by group" && v == "true")
}

/// Where a row's parts are, unzoomed from the top of its place.
struct RowGeom {
    strip: f32,
    half: f32,
}

fn geom(rh: f32) -> RowGeom {
    RowGeom { strip: STRIP_H, half: ((rh - 2.0 - STRIP_H) / 2.0).max(0.0) }
}

/// "Scale to all": a click fits the axis to every bar. The circle inside it turns live on and off:
/// while live, the axis follows the bars. Returns (the button was pressed, the circle was).
pub(crate) fn scale_button(ui: &mut egui::Ui, id: NodeId, rect: Rect, z: f32, live: bool) -> (bool, bool) {
    const LABEL: &str = "Scale to all";
    let centre = Pos2::new(rect.right() - 11.0 * z, rect.center().y);
    let dot = Rect::from_center_size(centre, Vec2::splat(18.0 * z));
    let main = ui.interact(rect, egui::Id::new(("scale-to-all", id)), Sense::click());
    main.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, LABEL));
    let circle = ui.interact(dot, egui::Id::new(("scale-live", id)), Sense::click());
    circle.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, live, "live"));
    let fill = if live {
        Color32::from_rgb(81, 88, 98)
    } else if main.hovered() {
        Color32::from_rgb(70, 76, 85)
    } else {
        Color32::from_rgb(58, 63, 71)
    };
    let p = ui.painter();
    p.rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    p.text(Pos2::new(rect.left() + 8.0 * z, rect.center().y), Align2::LEFT_CENTER, LABEL, FontId::proportional(11.5 * z), TEXT);
    let r = 4.5 * z;
    if live {
        p.circle_filled(centre, r, TEXT);
    } else {
        p.circle_stroke(centre, r, Stroke::new(1.2 * z, if circle.hovered() { TEXT } else { DIM }));
    }
    let pressed = (main.clicked(), circle.clicked());
    main.on_hover_text("scale the time axis to show every bar");
    circle.on_hover_text(if live { "live: the axis follows the bars. Click to stop" } else { "make it live: the axis follows the bars" });
    pressed
}

/// The row height slider: one size on screen, whatever the canvas zoom. `at`: the middle of its
/// left end.
pub(crate) fn row_slider(ui: &mut egui::Ui, at: Pos2, z: f32, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    const W: f32 = 110.0;
    const H: f32 = 18.0;
    let rect = Rect::from_min_size(at - Vec2::new(0.0, H * 0.5), Vec2::new(W, H));
    let mut s = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt("row-height"));
    // The node's style is scaled by the zoom: undone here.
    crate::look::scale(&mut s, 1.0 / z);
    s.spacing_mut().slider_width = W;
    s.add(egui::Slider::new(value, range).show_value(false));
}

fn place_y(slot: usize, rh: f32) -> f32 {
    TOP + BAR_H + RULER_H + slot as f32 * rh
}

/// `+12 years`, `-3 min`, `+40.2 s`.
fn gap_text(s: f64) -> String {
    let a = s.abs();
    let sign = if s < 0.0 { "-" } else { "+" };
    if a < 120.0 {
        format!("{sign}{a:.1} s")
    } else if a < 7200.0 {
        format!("{sign}{:.0} min", a / 60.0)
    } else if a < 172_800.0 {
        format!("{sign}{:.0} h", a / 3600.0)
    } else if a < 2.0 * 30.44 * 86_400.0 {
        format!("{sign}{:.0} days", a / 86_400.0)
    } else if a < YEAR {
        format!("{sign}{:.0} months", a / (30.44 * 86_400.0))
    } else {
        format!("{sign}{:.1} years", a / YEAR)
    }
}

fn hsl(h: f32, s: f32, l: f32) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let u = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(u(r), u(g), u(b))
}

/// Green at 0 s, fully red at 30 s, going black by a year: the cell's fill and text.
fn off_color(d: f64) -> (Color32, Color32) {
    let a = d.abs();
    if a <= 30.0 {
        return (hsl(130.0 * (1.0 - a as f32 / 30.0), 0.62, 0.46), INK);
    }
    let f = ((a / 30.0).ln() / (YEAR / 30.0).ln()).min(1.0) as f32;
    (hsl(0.0, 0.65, 0.46 * (1.0 - f)), if f > 0.3 { Color32::WHITE } else { INK })
}

/// What a row's status cell says.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Plain(String),
    Bad(String),
    /// Seconds off, and whether that is about its own length.
    Off(f64, bool),
    /// The row holding its group's anchor.
    Anchor,
    Empty,
}

/// The status cell of row `k`, and whether the whole row is greyed.
pub fn cell(v: &MatchView, k: usize) -> (Cell, bool) {
    let r = &v.rows[k];
    if !r.wired {
        return (Cell::Empty, false);
    }
    if r.time_err.is_some() {
        return (Cell::Bad("Time node error".into()), false);
    }
    if let Some(Stage::Failed(_)) = &r.stage {
        return (Cell::Bad("failed".into()), false);
    }
    if r.uid.is_some() && !r.file {
        return (Cell::Plain("files only".into()), false);
    }
    if v.running && !v.done {
        return (
            match &r.stage {
                Some(Stage::Waiting(s)) if *s > 0.0 => Cell::Plain(format!("decoding {s:.0} s")),
                Some(Stage::Waiting(_)) => Cell::Plain("waiting".into()),
                Some(Stage::Fingerprinting) => Cell::Plain("fingerprinting".into()),
                Some(Stage::Ready(_)) => Cell::Plain("fingerprinted".into()),
                _ => Cell::Plain("waiting".into()),
            },
            false,
        );
    }
    if !v.done {
        return (Cell::Plain(r.meta.map_or("no time metadata".to_string(), day_and_time)), false);
    }
    let g = &v.groups[r.group];
    if g.anchor == Some(k) {
        return (Cell::Anchor, false);
    }
    let linked = g.rows.iter().filter(|&&q| v.rows[q].uid.is_some()).count() >= 2;
    if !linked {
        return (Cell::Plain("no links found".into()), r.meta.is_none());
    }
    if r.lost && g.anchor.is_none() {
        return (Cell::Plain("lost its anchor".into()), false);
    }
    match r.off {
        Some(d) => (Cell::Off(d, r.hint), false),
        None => (Cell::Plain("no time metadata".into()), false),
    }
}

fn day_and_time(t: f64) -> String {
    let s = timebase::fmt_utc(t);
    format!("{}\n{}", &s[..10.min(s.len())], timebase::fmt_utc_clock(t))
}

/// The axis that fits every bar, however far apart they are.
pub(crate) fn fit_axis(bars: &[(f64, f64)]) -> (f64, f64) {
    if bars.is_empty() {
        return (0.0, 600.0);
    }
    let lo = bars.iter().map(|b| b.0).fold(f64::MAX, f64::min);
    let hi = bars.iter().map(|b| b.1).fold(f64::MIN, f64::max);
    let pad = ((hi - lo) * 0.05).max(1.0);
    (lo - pad, hi + pad)
}

/// A round step giving about six ticks across `span` seconds.
pub(crate) fn tick_step(span: f64) -> f64 {
    let want = span / 6.0;
    [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0, 21600.0, 86400.0, 604800.0, 2629800.0, 7889400.0, YEAR, 2.0 * YEAR, 5.0 * YEAR, 10.0 * YEAR, 50.0 * YEAR, 100.0 * YEAR, 500.0 * YEAR].into_iter().find(|s| *s >= want).unwrap_or(want)
}

/// A tick's label: time of day, then the date, then the year as the ticks grow apart.
pub(crate) fn tick_text(t: f64, step: f64) -> String {
    let (c, d) = (timebase::fmt_utc_clock(t), timebase::fmt_utc(t));
    if step < 60.0 {
        c
    } else if step < 86400.0 {
        c[..5.min(c.len())].to_string()
    } else if step < YEAR {
        d[..10.min(d.len())].to_string()
    } else {
        d[..4.min(d.len())].to_string()
    }
}

/// An anchor, drawn in `r`.
fn anchor_icon(p: &egui::Painter, r: Rect, col: Color32) {
    let at = |x: f32, y: f32| r.min + Vec2::new(x / 16.0 * r.width(), y / 16.0 * r.height());
    let st = Stroke::new((r.width() * 0.11).max(1.0), col);
    p.circle_stroke(at(8.0, 3.2), r.width() * 0.11, st);
    p.line_segment([at(8.0, 5.0), at(8.0, 14.0)], st);
    p.line_segment([at(5.0, 7.5), at(11.0, 7.5)], st);
    let arc: Vec<Pos2> = (0..=12)
        .map(|k| {
            let a = std::f32::consts::PI * (k as f32 / 12.0);
            at(8.0 - 4.8 * a.cos(), 10.4 + 3.4 * a.sin())
        })
        .collect();
    p.add(egui::Shape::line(arc, st));
}

/// A mirror, drawn in `r`: a line with a shape and its reflection. It marks a time that may be the
/// recording's end read as its start.
fn mirror_icon(p: &egui::Painter, r: Rect, col: Color32) {
    let at = |x: f32, y: f32| r.min + Vec2::new(x / 16.0 * r.width(), y / 16.0 * r.height());
    let st = Stroke::new((r.width() * 0.09).max(1.0), col);
    p.line_segment([at(8.0, 1.0), at(8.0, 15.0)], st);
    p.add(egui::Shape::convex_polygon(vec![at(6.0, 3.5), at(6.0, 12.5), at(1.0, 12.5)], col, Stroke::NONE));
    p.add(egui::Shape::closed_line(vec![at(10.0, 3.5), at(15.0, 12.5), at(10.0, 12.5)], st));
}

fn shorten(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    } else {
        s.to_string()
    }
}

/// The bits of a row's hover text.
fn row_hover(v: &MatchView, r: &Row) -> String {
    let mut h = vec![r.name.clone()];
    match r.meta {
        Some(t) => h.push(format!("time input: {} UTC ± {} s, {}", timebase::fmt_utc(t), r.meta_err, r.meta_source)),
        None if r.time_wired => h.push("the time input gives no time".to_string()),
        None => h.push("nothing wired into the time input".to_string()),
    }
    if let Some(e) = &r.time_err {
        h.push(e.clone());
    }
    if r.len > 0.0 {
        h.push(format!("length {:.2} s", r.len));
    }
    if let Some(t) = r.start {
        let word = if v.anchored(r) { "audio starts" } else { "audio shown at" };
        h.push(format!("{word} {} UTC", timebase::fmt_utc(t)));
    }
    if let Some((s, e)) = v.spreads.get(r.group).copied().flatten() {
        h.push(format!("this group's times disagree by about {s:.1} s read as starts, {e:.1} s read as ends"));
    }
    if r.hint {
        h.push("off by about its own length: the time may mark the end of the recording. A Time node can subtract the length".to_string());
    }
    if let Some(Stage::Ready(n)) = &r.stage {
        h.push(format!("{n} fingerprint hashes"));
    }
    if let Some(Stage::Failed(e)) = &r.stage {
        h.push(e.clone());
    }
    h.join("\n")
}

/// How this node looks on the canvas: a row per recording, a timeline in each, a status column and
/// the rows' yellow outputs.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, _standard: Vec2) -> Vec2 {
        let n = app.graph.nodes()[i].spec.inputs.len() / 2;
        Vec2::new(NODE_W, place_y(n, row_height(&app.nodes[i].kind)) + FOOT_H + 6.0)
    }

    fn port_look(&self, app: &App, i: NodeId, name: &str) -> Option<PortLook> {
        let rh = row_height(&app.nodes[i].kind);
        let g = geom(rh);
        let number = |base: &str| name.strip_prefix(base).and_then(|n| n.parse::<usize>().ok());
        let row = |n: usize| {
            let k = n.saturating_sub(1);
            let slot = app.match_ui.slots.get(&i).and_then(|s| s.get(k)).copied().unwrap_or(k);
            place_y(slot, rh) + 2.0
        };
        // Thin rows have no room below the name: the dots spread over the whole row.
        let lead = g.strip * (g.half / 12.0).min(1.0);
        let room = rh - 2.0 - lead;
        let dot = (g.half * 0.42).clamp(2.5, 5.0);
        let at = |side: Side, y: f32| Some(PortLook { side: Some(side), at: Some(y), label: Some(PortLabel::Hidden), dot_size: Some(dot), ..Default::default() });
        if let Some(n) = number("audio ") {
            return at(Side::Left, row(n) + lead + room * 0.25);
        }
        if let Some(n) = number("time ") {
            return at(Side::Left, row(n) + lead + room * 0.75);
        }
        if let Some(n) = number("synced ") {
            return at(Side::Right, row(n) + lead + room * 0.5);
        }
        // The other outputs: along the bottom edge, at the right.
        let fixed = ["links", "quality", "clock drift", "same recording", "speed change", "gaps"];
        let k = fixed.iter().position(|f| *f == name)?;
        Some(PortLook { side: Some(Side::Bottom), at: Some(NODE_W - 50.0 - (fixed.len() - 1 - k) as f32 * 102.0), ..Default::default() })
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let body = ui.max_rect();
        let z = app.zoom;
        let v = app.match_view(i);
        app.note_lost_anchors(i, &v);
        let v = app.match_view(i);
        let rh = row_height(&app.nodes[i].kind);
        let g = geom(rh);
        let font = |s: f32| FontId::proportional(s * z);
        let mono = |s: f32| FontId::monospace(s * z);
        let p = ui.painter().with_clip_rect(ui.clip_rect());
        // The rows' colours run out to the node's edges, past the part a look draws in.
        let (edge_l, edge_r) = (body.left() - EDGE * z, body.right() + EDGE * z);
        let wide = ui.painter().with_clip_rect(ui.clip_rect().expand2(Vec2::new(EDGE * z, 0.0)).intersect(app.canvas_rect));

        // Draw order: by group when asked and known, the free row always last.
        let mut order: Vec<usize> = (0..v.rows.len()).collect();
        if by_group(&app.nodes[i].kind) && v.done {
            let rank = |k: usize| {
                let n = v.groups[v.rows[k].group].number;
                (!v.rows[k].wired, if n == 0 { usize::MAX } else { n }, k)
            };
            order.sort_by_key(|&k| rank(k));
        }
        let mut slots = vec![0; order.len()];
        for (s, &k) in order.iter().enumerate() {
            slots[k] = s;
        }
        app.match_ui.slots.insert(i, slots);

        let x0 = body.left();
        let track_x = (x0 + LEFT_W * z, body.right() - (STATUS_W + RIGHT_W) * z);
        let rows_top = body.top() + (BAR_H + RULER_H) * z;
        let row_top = |slot: usize| rows_top + slot as f32 * rh * z;
        let rows_bottom = row_top(order.len());
        let track = Rect::from_min_max(Pos2::new(track_x.0, rows_top - RULER_H * z), Pos2::new(track_x.1, rows_bottom));
        if track.width() <= 10.0 {
            return;
        }

        // The axis.
        let bars: Vec<(f64, f64)> = v.rows.iter().flat_map(|r| [r.meta.map(|t| (t, t + r.len)), r.start.map(|t| (t, t + r.len))]).flatten().collect();
        let fit = fit_axis(&bars);
        let held = app.match_ui.grab.as_ref().filter(|g| g.0 == i).map(|g| g.2);
        let (v0, v1) = held.or(app.match_ui.views.get(&i).copied()).unwrap_or(fit);
        let span = (v1 - v0).max(1e-3);
        let tx = |t: f64| track.left() + ((t - v0) / span) as f32 * track.width();

        // The bar along the top.
        let always = !app.match_ui.views.contains_key(&i);
        let by = by_group(&app.nodes[i].kind);
        let mut toggle_by = false;
        let mut new_rh = rh;
        let y = body.top() + BAR_H * 0.5 * z;
        let button = |ui: &mut egui::Ui, x: f32, w: f32, text: &str, on: bool| {
            let b = egui::Button::new(egui::RichText::new(text).size(11.5 * z)).fill(if on { Color32::from_rgb(81, 88, 98) } else { Color32::from_rgb(58, 63, 71) });
            ui.put(Rect::from_min_size(Pos2::new(x, y - 10.0 * z), Vec2::new(w * z, 20.0 * z)), b).clicked()
        };
        let (mut do_fit, toggle_always) = scale_button(ui, i, Rect::from_min_size(Pos2::new(x0 + 4.0 * z, y - 10.0 * z), Vec2::new(98.0 * z, 20.0 * z)), z, always);
        if button(ui, x0 + 106.0 * z, 96.0, "Order by group", by) {
            toggle_by = true;
        }
        p.text(Pos2::new(x0 + 216.0 * z, y), Align2::LEFT_CENTER, "row height", font(11.5), DIM);
        row_slider(ui, Pos2::new(x0 + 278.0 * z, y), z, &mut new_rh, ROW_MIN..=ROW_MAX);
        let scale = if span < 86400.0 { format!("{} to {}", timebase::fmt_utc_clock(v0), timebase::fmt_utc_clock(v1)) } else { format!("{} to {}", &timebase::fmt_utc(v0)[..10], &timebase::fmt_utc(v1)[..10]) };
        if v.real {
            p.text(Pos2::new(body.right() - 8.0 * z, y), Align2::RIGHT_CENTER, scale, mono(11.5), DIM);
        }
        p.line_segment([Pos2::new(body.left(), body.top() + BAR_H * z), Pos2::new(body.right(), body.top() + BAR_H * z)], Stroke::new(z, SEAM));

        // The ruler, and the grid down the rows.
        p.rect_filled(Rect::from_min_max(track.min, Pos2::new(track.right(), rows_top)), 0.0, TRACK);
        p.rect_filled(Rect::from_min_max(Pos2::new(track.left(), rows_top), track.max), 0.0, TRACK);
        let step = tick_step(span);
        let mut q = (v0 / step).ceil();
        let mut ticks = 0;
        while q * step <= v1 && ticks < 30 {
            let x = tx(q * step);
            p.line_segment([Pos2::new(x, track.top()), Pos2::new(x, track.bottom())], Stroke::new(z.max(1.0), GRID));
            let label = if v.real { tick_text(q * step, step) } else { format!("{:+.0} s", q * step) };
            p.text(Pos2::new(x + 4.0 * z, track.top() + RULER_H * 0.5 * z), Align2::LEFT_CENTER, label, mono(11.0), DIM);
            q += 1.0;
            ticks += 1;
        }
        p.text(Pos2::new(track.right() + STATUS_W * 0.5 * z, track.top() + RULER_H * 0.5 * z), Align2::CENTER_CENTER, "status", font(11.0), DIM);

        // The rows.
        let mut press_anchor: Option<usize> = None;
        for (slot, &k) in order.iter().enumerate() {
            let r = &v.rows[k];
            let top = row_top(slot);
            let (status, dead) = cell(&v, k);
            let fade = |c: Color32| if dead { c.gamma_multiply(0.45) } else { c };
            wide.rect_filled(Rect::from_min_max(Pos2::new(edge_l, top), Pos2::new(edge_r, top + 2.0 * z)), 0.0, SEAM);
            let inner = top + 2.0 * z;
            let strip_h = g.strip * z;
            let half = g.half * z;
            let left_col = Rect::from_min_max(Pos2::new(x0, inner), Pos2::new(track.left(), top + rh * z));
            let words = g.half >= WORDS_FROM;
            if !r.wired {
                // The free row waits for a wire.
                let d = alpha(DIM, 0.8);
                if words {
                    p.text(Pos2::new(x0 + 14.0 * z, inner + strip_h + half * 0.5), Align2::LEFT_CENTER, format!("audio {}", r.n), font(12.0), d);
                    p.text(Pos2::new(x0 + 14.0 * z, inner + strip_h + half * 1.5), Align2::LEFT_CENTER, format!("time {}", r.n), font(12.0), d);
                } else {
                    p.text(Pos2::new(x0 + 14.0 * z, left_col.center().y), Align2::LEFT_CENTER, format!("audio {0}, time {0}", r.n), font(11.5), d);
                }
                continue;
            }
            // With no room on the ports, the anchor button sits beside the name.
            let by_name = g.half < ANCHOR_FROM && r.meta.is_some();
            wide.rect_filled(Rect::from_min_max(Pos2::new(edge_l, inner), Pos2::new(track.left(), inner + strip_h)), 0.0, fade(STRIP));
            p.text(left_col.min + Vec2::new(8.0 * z, strip_h * 0.5), Align2::LEFT_CENTER, shorten(&r.name, if by_name { 21 } else { 25 }), mono(11.5), fade(TEXT));
            // The port swatches: audio over time.
            let audio_sw = Rect::from_min_max(Pos2::new(edge_l, inner + strip_h), Pos2::new(track.left() - z, inner + strip_h + half));
            let time_sw = Rect::from_min_max(Pos2::new(edge_l, audio_sw.bottom()), Pos2::new(track.left() - z, audio_sw.bottom() + half));
            wide.rect_filled(audio_sw, 0.0, fade(if r.uid.is_some() { AMBER } else { OFF }));
            wide.rect_filled(time_sw, 0.0, fade(if r.meta.is_some() { BLUE } else { OFF }));
            if words {
                p.text(Pos2::new(x0 + 21.0 * z, audio_sw.center().y), Align2::LEFT_CENTER, "audio", font(12.0), if r.uid.is_some() { INK } else { DIM });
                p.text(Pos2::new(x0 + 21.0 * z, time_sw.center().y), Align2::LEFT_CENTER, "time", font(12.0), if r.meta.is_some() { INK } else { DIM });
            }
            // The anchor button, on a time, once the audio has placed the row.
            if r.meta.is_some() {
                let s = if by_name { strip_h - 6.0 * z } else { (half - 3.0 * z).min(20.0 * z).max(8.0 * z) };
                let (line, margin) = if by_name { (inner + strip_h * 0.5, 2.0) } else { (time_sw.center().y, 5.0) };
                let br = Rect::from_center_size(Pos2::new(time_sw.right() - s * 0.5 - margin * z, line), Vec2::splat(s));
                app.match_ui.anchor_at.insert((i, k), br);
                let on = v.groups[r.group].anchor == Some(k);
                if v.done {
                    let tip = if on { "this time is the anchor of its synced group: click to take it away" } else { "anchor this synced group on this time" };
                    let resp = ui.interact(br, egui::Id::new(("match-anchor", i, r.n)), Sense::click()).on_hover_text(tip);
                    if resp.clicked() {
                        press_anchor = Some(k);
                    }
                }
                if on {
                    p.rect_filled(br, 0.0, INK);
                }
                // Dark on the blue port, light on the grey name strip.
                let ink = if by_name { TEXT } else { INK };
                p.rect_stroke(br, 0.0, Stroke::new(1.5 * z, if v.done { ink } else { alpha(ink, 0.4) }), StrokeKind::Inside);
                anchor_icon(&p, br.shrink(s * 0.14), if on { BOTH } else if v.done { ink } else { alpha(ink, 0.4) });
            }

            // The timeline.
            let lane = Rect::from_min_max(Pos2::new(track.left(), inner), Pos2::new(track.right(), top + rh * z));
            let band = |b: (f64, f64)| {
                let (a, c) = (tx(b.0).max(lane.left()), tx(b.1).min(lane.right()));
                (c > a || (b.0 <= v1 && b.1 >= v0)).then(|| Rect::from_min_max(Pos2::new(a, lane.top()), Pos2::new(c.max(a + 2.0 * z), lane.bottom())))
            };
            let blue = r.meta.map(|t| (t, t + r.len));
            let amber = r.start.map(|t| (t, t + r.len));
            let (mut gone, mut shown) = (None, false);
            // See-through, so an overlap shows both bars.
            for (bar, col) in [(blue, BLUE), (amber, AMBER)] {
                let Some(b) = bar else { continue };
                match band(b).filter(|_| b.1 >= v0 && b.0 <= v1) {
                    Some(rect) => {
                        p.rect_filled(rect, 0.0, fade(alpha(col, BAR_ALPHA)));
                        shown = true;
                    }
                    None => {
                        let g = if b.1 < v0 { b.1 - v0 } else { b.0 - v1 };
                        gone = Some(gone.map_or(g, |o: f64| if g.abs() < o.abs() { g } else { o }));
                    }
                }
            }
            if let Some(t) = r.meta.filter(|t| *t >= v0 && *t <= v1) {
                let x = tx(t);
                p.rect_filled(Rect::from_min_max(Pos2::new(x - 1.25 * z, lane.top()), Pos2::new(x + 1.25 * z, lane.bottom())), 0.0, fade(ANCHOR_BLUE));
            }
            if let (Some(b), true) = (amber, v.anchored(r)) {
                let x = tx(b.1);
                if x > lane.left() + 16.0 * z && x <= lane.right() + 16.0 * z {
                    let s = (13.0 * z).min(lane.height() - 4.0 * z).max(6.0 * z);
                    anchor_icon(&p, Rect::from_min_size(Pos2::new(x - s - 3.0 * z, lane.top() + 2.0 * z), Vec2::splat(s)), INK);
                }
            }
            if let Some(g) = gone.filter(|_| !shown) {
                let (at, side, text) = if g < 0.0 { (lane.left() + 6.0 * z, Align2::LEFT_CENTER, format!("< {}", gap_text(g))) } else { (lane.right() - 6.0 * z, Align2::RIGHT_CENTER, format!("{} >", gap_text(g))) };
                p.text(Pos2::new(at, lane.center().y), side, text, mono(11.0), DIM);
            }

            // The status cell.
            let cell_r = Rect::from_min_max(Pos2::new(track.right() + z, inner), Pos2::new(track.right() + STATUS_W * z, top + rh * z));
            match &status {
                Cell::Plain(t) => {
                    let t = if rh < 44.0 { t.lines().last().unwrap_or(t).to_string() } else { t.clone() };
                    p.text(cell_r.center(), Align2::CENTER_CENTER, t, mono(12.0), fade(DIM));
                }
                Cell::Bad(t) => {
                    p.text(cell_r.center(), Align2::CENTER_CENTER, t, mono(12.0), RED);
                }
                Cell::Off(d, hint) => {
                    let (bg, fg) = off_color(*d);
                    p.rect_filled(cell_r, 0.0, bg);
                    p.text(cell_r.center(), Align2::CENTER_CENTER, gap_text(*d), mono(13.0), fg);
                    if *hint {
                        let s = (16.0 * z).min(cell_r.height() - 4.0 * z);
                        let icon = Rect::from_center_size(Pos2::new(cell_r.right() - 12.0 * z, cell_r.center().y), Vec2::splat(s));
                        mirror_icon(&p, icon, fg);
                        ui.interact(icon, egui::Id::new(("match-mirror", i, r.n)), Sense::hover()).on_hover_text("off by about its own length: the time may mark the end of the recording, not its start. A Time node can subtract the length");
                    }
                }
                Cell::Anchor => {
                    let s = (22.0 * z).min(cell_r.height() - 2.0 * z);
                    anchor_icon(&p, Rect::from_center_size(cell_r.center(), Vec2::splat(s)), TEXT);
                }
                Cell::Empty => {}
            }

            // The output.
            let out_col = Rect::from_min_max(Pos2::new(cell_r.right() + z, inner), Pos2::new(edge_r, top + rh * z));
            wide.rect_filled(Rect::from_min_size(out_col.min, Vec2::new(out_col.width(), strip_h)), 0.0, fade(STRIP));
            p.text(out_col.min + Vec2::new(8.0 * z, strip_h * 0.5), Align2::LEFT_CENTER, shorten(&r.name, 15), mono(11.5), fade(TEXT));
            let on = v.anchored(r) && r.uid.is_some();
            let out_sw = Rect::from_min_max(Pos2::new(out_col.left(), inner + strip_h), out_col.max);
            wide.rect_filled(out_sw, 0.0, fade(if on { AMBER } else { OFF }));
            if words {
                p.text(Pos2::new(body.right() - 21.0 * z, out_sw.center().y), Align2::RIGHT_CENTER, "time", font(12.0), if on { INK } else { DIM });
            }
        }
        wide.rect_filled(Rect::from_min_max(Pos2::new(edge_l, rows_bottom), Pos2::new(edge_r, rows_bottom + 2.0 * z)), 0.0, SEAM);

        // Group borders: one round each run of neighbouring rows of a group.
        for (gi, grp) in v.groups.iter().enumerate().filter(|(_, g)| g.number > 0) {
            let col = GROUP_COLORS[(grp.number - 1) % GROUP_COLORS.len()];
            let _ = gi;
            let mut s = 0;
            while s < order.len() {
                if !grp.rows.contains(&order[s]) {
                    s += 1;
                    continue;
                }
                let mut e = s;
                while e + 1 < order.len() && grp.rows.contains(&order[e + 1]) {
                    e += 1;
                }
                let rect = Rect::from_min_max(Pos2::new(track.left(), row_top(s) + 2.0 * z), Pos2::new(track.right(), row_top(e + 1)));
                p.rect_stroke(rect, 0.0, Stroke::new(2.0 * z, col), StrokeKind::Inside);
                let tab = Rect::from_min_size(rect.min, Vec2::new(20.0 * z, 16.0 * z).min(rect.size()));
                p.rect_filled(tab, 0.0, col);
                p.text(tab.center(), Align2::CENTER_CENTER, grp.number.to_string(), mono(11.0), INK);
                s = e + 1;
            }
        }

        // The foot: what the node is doing.
        let status_text = app.nodes[i].state.lock().unwrap().status.clone();
        let text = if status_text.is_empty() { "not run".to_string() } else { status_text };
        p.text(Pos2::new(body.left() + 8.0 * z, rows_bottom + 24.0 * z), Align2::LEFT_CENTER, shorten(&text, 70), font(11.5), DIM);

        // The track: the wheel zooms around the pointer, a drag on an unanchored group's amber bar
        // moves the group, any other drag pans, a double-click fits everything.
        app.match_ui.tracks.push(track);
        let resp = ui.interact(Rect::from_min_max(track.min, Pos2::new(track.right(), rows_bottom)), egui::Id::new(("match-track", i)), Sense::click_and_drag());
        let view_now = |app: &App| app.match_ui.views.get(&i).copied().unwrap_or(fit);
        if let Some(h) = resp.hover_pos() {
            let scroll = ui.input(|k| k.smooth_scroll_delta.y + k.smooth_scroll_delta.x);
            if scroll != 0.0 {
                let f = (-scroll as f64 * 0.002).exp();
                let at = v0 + ((h.x - track.left()) / track.width()) as f64 * span;
                app.match_ui.views.insert(i, (at - (at - v0) * f, at + (v1 - at) * f));
            }
        }
        let row_at = |y: f32| -> Option<&Row> {
            let s = ((y - rows_top) / (rh * z)).floor();
            (s >= 0.0).then(|| order.get(s as usize).map(|&k| &v.rows[k])).flatten()
        };
        if resp.drag_started() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let t = v0 + ((pos.x - track.left()) / track.width()) as f64 * span;
                if let Some(r) = row_at(pos.y).filter(|r| r.start.is_some_and(|s| t >= s && t <= s + r.len) && !v.anchored(r)) {
                    let key = v.groups[r.group].key.clone();
                    app.match_ui.grab = Some((i, key, (v0, v1)));
                }
            }
        }
        if resp.dragged() {
            let dt = (resp.drag_delta().x / track.width()) as f64 * span;
            match app.match_ui.grab.clone().filter(|g| g.0 == i) {
                Some((_, key, _)) => *app.match_ui.shift.entry((i, key)).or_default() += dt,
                None => {
                    let (a, b) = view_now(app);
                    app.match_ui.views.insert(i, (a - dt, b - dt));
                }
            }
        }
        if resp.drag_stopped() {
            app.match_ui.grab = None;
        }
        if resp.double_clicked() {
            do_fit = true;
        }
        if let Some(r) = resp.hover_pos().and_then(|h| row_at(h.y)).filter(|r| r.wired) {
            resp.on_hover_text(row_hover(&v, r));
        }
        // While live the axis already fits, and it stays live.
        if do_fit && !always {
            app.match_ui.views.insert(i, fit);
        }
        if toggle_always {
            if always {
                app.match_ui.views.insert(i, (v0, v1));
            } else {
                app.match_ui.views.remove(&i);
            }
        }
        let mut changed = false;
        if let Kind::AudioMatch { rows } = &mut app.nodes[i].kind {
            let set = |rows: &mut Vec<(String, String)>, k: &str, val: String| match rows.iter_mut().find(|(q, _)| q == k) {
                Some((_, x)) => *x = val,
                None => rows.push((k.to_string(), val)),
            };
            if toggle_by {
                set(rows, "order by group", (!by).to_string());
                changed = true;
            }
            if (new_rh - rh).abs() >= 0.5 {
                set(rows, "row height", format!("{new_rh:.0}"));
                changed = true;
            }
        }
        if changed {
            app.save_project();
        }
        if let Some(k) = press_anchor {
            app.toggle_anchor(i, k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_keeps_a_clock_years_off() {
        let t = 1_347_000_000.0;
        let (a, b) = fit_axis(&[(t, t + 100.0), (t + 50.0, t + 200.0), (t + 4e8, t + 4e8 + 60.0)]);
        assert!(a < t && b > t + 4e8 + 60.0, "{a} {b}");
        assert_eq!(fit_axis(&[]), (0.0, 600.0));
    }

    #[test]
    fn gaps_read_plainly() {
        assert_eq!(gap_text(40.2), "+40.2 s");
        assert_eq!(gap_text(-180.0), "-3 min");
        assert_eq!(gap_text(4e8), "+12.7 years");
    }

    #[test]
    fn the_status_colour_goes_from_green_through_red_to_black() {
        let (zero, _) = off_color(0.0);
        assert!(zero.g() > zero.r(), "{zero:?}");
        let (thirty, _) = off_color(30.0);
        assert!(thirty.r() > 2 * thirty.g(), "{thirty:?}");
        let (years, fg) = off_color(YEAR * 3.0);
        assert_eq!((years, fg), (Color32::BLACK, Color32::WHITE));
    }
}
