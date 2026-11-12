//! The Cut detector node in the workbench. Once the graph runs it reads the wired recording's file
//! itself (`corroborate::cuts`): every frame for the picture lanes, ffprobe's packets for the
//! keyframes and gaps; its sound (the source's decoded audio, at the audio match's 8 kHz) for the
//! sound level and background noise; and each recording wired into `compare with` for a compare
//! lane against it. What it found (`Scan`) stays in the node's state; the cuts are worked out
//! from it every frame, with the thresholds, switches and hand cuts kept in the node's rows and the
//! events wired in, so changing any of them needs no new run. Each piece is a recording of its
//! own: `audio n`, `video n` (streams), `time n` (as played: the wired start plus where the piece
//! sits in the file) and `length n`. Compiled into the `corroborate` program.

use super::*;
use corroborate::cuts::{self, Cut, Lane, Piece, Role, Switch};
use corroborate::data::{Data, Event};
use corroborate::modules::cut_detector as cd;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;

static RUNS: AtomicU64 = AtomicU64::new(1);

/// What a run found in the recording.
#[derive(Default)]
pub struct Scan {
    /// The file, its length and one frame's length, seconds.
    pub path: String,
    pub len: f64,
    pub frame_s: f64,
    /// The computed lanes (events lanes are added from the wires).
    pub lanes: Vec<Lane>,
    pub thumbs: Vec<cuts::Thumb>,
    /// What could not be read, lane by lane.
    pub notes: Vec<String>,
}

/// What a run reads: the recording's file, whether its picture and sound are wired, and its
/// SOURCER's state (its decoded sound, once that run is done: the queue starts the detector after
/// it).
pub struct Job {
    pub path: String,
    pub video: bool,
    pub audio: bool,
    pub source: Option<Shared>,
    /// The recordings wired into `compare with`.
    pub compare: Vec<Other>,
}

/// A recording to compare with: its input's key (`compare 2`), its name, its file and its
/// source's state as in `Job`.
pub struct Other {
    pub key: String,
    pub name: String,
    pub path: String,
    pub source: Option<Shared>,
}

pub use crate::jobs::{Progress, StepState};

/// The steps of a run, by name.
const FRAMES: &str = "every frame";
const KEYFRAMES: &str = "keyframes";
const SOUND_STEP: &str = "sound";
const LEVELS: &str = "level and noise";

/// A recording's sound at the audio match's rate: its source's decoded audio when that run is
/// done, else its file decoded here.
fn sound_of(path: &str, source: &Option<Shared>, stop: &AtomicBool) -> Result<Vec<f32>, String> {
    match source.as_ref().and_then(crate::audio_match_node::from_source) {
        Some(a) => Ok(a),
        None => crate::audio_match_node::decode(path, stop),
    }
}

/// Reads the recording on a thread of its own, writing into `state` (unless `stop` is set first,
/// or a newer run took the state over). `job` is why there is nothing to read when it is an error.
pub fn start(state: Shared, job: Result<Job, String>, stop: Arc<AtomicBool>) {
    let run = RUNS.fetch_add(1, Ordering::Relaxed);
    {
        let mut s = state.lock().unwrap();
        s.reset();
        s.run_id = run;
    }
    let job = match job {
        Ok(j) => j,
        Err(e) => {
            let mut s = state.lock().unwrap();
            s.progress.finish(crate::jobs::End::Failed(e.clone()));
            s.status = e;
            return;
        }
    };
    {
        let mut names: Vec<String> = Vec::new();
        if job.video {
            names.extend([FRAMES.into(), KEYFRAMES.into()]);
        }
        if job.audio || !job.compare.is_empty() {
            names.push(SOUND_STEP.into());
        }
        if job.audio {
            names.push(LEVELS.into());
        }
        names.extend(job.compare.iter().map(|o| format!("compare: {}", o.name)));
        let mut s = state.lock().unwrap();
        s.running = true;
        s.status = "waiting for its turn to decode…".into();
        s.progress = Progress::new(names);
        // First it waits for a turn to decode the picture.
        if job.video {
            s.progress.waits = Some(crate::jobs::turn_waits());
        }
    }
    std::thread::spawn(move || {
        work(&state, run, &job, &stop);
        // Ended without saying how: stopped (or a newer run has it).
        let mut s = state.lock().unwrap();
        if s.run_id == run && s.running {
            s.running = false;
            s.progress.finish(if stop.load(Ordering::Relaxed) { crate::jobs::End::Cancelled } else { crate::jobs::End::Failed("ended without a result".into()) });
        }
    });
}

/// The run itself (`start`'s thread).
fn work(state: &Shared, run: u64, job: &Job, stop: &Arc<AtomicBool>) {
    {
        let set = |f: &dyn Fn(&mut runtime::NodeState)| {
            let mut s = state.lock().unwrap();
            if s.run_id == run && !stop.load(Ordering::Relaxed) {
                f(&mut s);
            }
        };
        // A step's state, how far it has got, and what went wrong.
        let step = |name: &str, st: StepState, frac: f32, note: &str| set(&|s| s.progress.step(name, st, frac, note));
        let waits = |w: String| set(&|s| s.progress.waits = Some(w.clone()));
        let end = |summary: String, failed: bool| {
            set(&|s| {
                s.status = summary.clone();
                s.running = false;
                s.done = true;
                s.progress.finish(if failed { crate::jobs::End::Failed(summary.clone()) } else { crate::jobs::End::Done });
            })
        };
        let path = job.path.clone();
        // How long the recording is, for how far each step has got.
        let total = corroborate::timebase::file_facts(&path).len.filter(|l| *l > 0.0);
        let part = |secs: f64| total.map_or(-1.0, |l| (secs / l).clamp(0.0, 1.0) as f32);
        let mut scan = Scan { path: path.clone(), ..Default::default() };
        if job.video {
            waits(crate::jobs::turn_waits());
            let turn = corroborate::ingest::decode_turn(stop);
            if turn.is_none() {
                return;
            }
            set(&|s| s.status = "reading every frame…".into());
            step(FRAMES, StepState::Working, 0.0, "");
            let picture = cuts::scan_picture(&path, &stop, &|secs| {
                set(&|s| s.status = format!("reading every frame: {secs:.0} s…"));
                step(FRAMES, StepState::Working, part(secs), "");
            });
            drop(turn);
            match picture {
                Ok(p) => {
                    scan.len = p.times.last().map_or(0.0, |t| t + p.frame_s);
                    scan.frame_s = p.frame_s;
                    scan.lanes.push(cuts::picture_lane(&p));
                    scan.thumbs = p.thumbs;
                    step(FRAMES, StepState::Done, 1.0, "");
                }
                Err(e) if e == "stopped" => return,
                Err(e) => {
                    step(FRAMES, StepState::Failed, 0.0, &e);
                    scan.notes.push(format!("picture: {e}"));
                }
            }
            step(KEYFRAMES, StepState::Working, -1.0, "");
            match cuts::scan_packets(&path) {
                Ok(p) => {
                    if scan.frame_s == 0.0 {
                        scan.frame_s = p.frame_s;
                    }
                    scan.lanes.push(cuts::packets_lane(&p));
                    step(KEYFRAMES, StepState::Done, 1.0, "");
                }
                Err(e) => {
                    step(KEYFRAMES, StepState::Failed, 0.0, &e);
                    scan.notes.push(format!("keyframes: {e}"));
                }
            }
        }
        if job.audio || !job.compare.is_empty() {
            set(&|s| s.status = "reading the sound…".into());
            step(SOUND_STEP, StepState::Working, -1.0, "");
            let sound = sound_of(&path, &job.source, stop);
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match sound {
                Ok(a) => {
                    step(SOUND_STEP, StepState::Done, 1.0, "");
                    if scan.len == 0.0 {
                        scan.len = a.len() as f64 / cuts::SOUND_SR;
                    }
                    if job.audio {
                        set(&|s| s.status = "sound level and background noise…".into());
                        step(LEVELS, StepState::Working, -1.0, "");
                        scan.lanes.push(cuts::level_lane(&a));
                        scan.lanes.push(cuts::noise_lane(&a));
                        step(LEVELS, StepState::Done, 1.0, "");
                    }
                    if !job.compare.is_empty() {
                        set(&|s| s.status = "fingerprinting the sound…".into());
                        let ours = corroborate::audio_match::Clip::new(a);
                        for o in &job.compare {
                            let name = format!("compare: {}", o.name);
                            step(&name, StepState::Working, -1.0, "");
                            set(&|s| s.status = format!("comparing with {}…", o.name));
                            let theirs = sound_of(&o.path, &o.source, stop);
                            if stop.load(Ordering::Relaxed) {
                                return;
                            }
                            match theirs {
                                Ok(theirs) => {
                                    scan.lanes.push(cuts::compare_lane(&o.key, &o.name, &ours, &corroborate::audio_match::Clip::new(theirs)));
                                    step(&name, StepState::Done, 1.0, "");
                                }
                                Err(e) => {
                                    step(&name, StepState::Failed, 0.0, &e);
                                    scan.notes.push(format!("{}: {e}", o.name));
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    step(SOUND_STEP, StepState::Failed, 0.0, &e);
                    scan.notes.push(format!("sound: {e}"));
                }
            }
        }
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if scan.frame_s == 0.0 {
            scan.frame_s = 1.0 / 30.0;
        }
        let summary = if scan.notes.is_empty() { format!("{} lanes read", scan.lanes.len()) } else { scan.notes.join("; ") };
        // The scan before the end: what needs it starts once it has ended.
        let failed = scan.lanes.is_empty();
        let scan = Arc::new(scan);
        set(&|s| s.cut = Some(scan.clone()));
        end(summary, failed);
    }
}

/// The node's cuts and pieces now.
#[derive(Default)]
pub struct CutView {
    /// The source of the recording, and why there is none.
    pub src: Option<NodeId>,
    pub problem: Option<String>,
    pub scan: Option<Arc<Scan>>,
    /// Every lane (computed ones without their curves, then one per events input).
    pub lanes: Vec<Lane>,
    /// Each lane's threshold now (None: marks only).
    pub thr: Vec<Option<f32>>,
    pub cuts: Vec<Cut>,
    pub pieces: Vec<Piece>,
    pub len: f64,
    pub frame_s: f64,
    pub clock: String,
    /// Events left out for being on another clock.
    pub off_clock: usize,
}

/// What the canvas keeps between frames: the view (once a frame), the filmstrips, the
/// threshold being dragged and the players.
#[derive(Default)]
pub struct CutUi {
    pub cache: RefCell<HashMap<NodeId, Rc<CutView>>>,
    strips: HashMap<NodeId, Strip>,
    drag: Option<(NodeId, usize, f32)>,
    players: HashMap<NodeId, Player>,
    /// The cut each node's player is on (its place among the node's cuts).
    focus: HashMap<NodeId, usize>,
    /// Playing a cut: the node, and where on its recording the play stops.
    pub until: Option<(NodeId, f64)>,
}

/// A filmstrip's pictures in one texture, in rows (a texture has a largest side, 8192 or 16384
/// pixels: a picture a second of a long recording in one row would not fit). `step`: every how
/// many pictures one is kept, when even rows would not fit.
struct Strip {
    run: u64,
    tex: TextureHandle,
    cols: usize,
    rows: usize,
    step: usize,
}

/// A player's pictures: those read for the window `want` of the recording `path`, each with its
/// time, and why there are none.
#[derive(Default)]
struct Frames {
    path: String,
    want: (f64, f64),
    round: u64,
    busy: bool,
    got: Vec<(f64, Vec<u8>)>,
    note: String,
    stop: Option<Arc<AtomicBool>>,
}

/// A node's player: its pictures and the one on screen (its time and texture).
#[derive(Default)]
struct Player {
    frames: Arc<Mutex<Frames>>,
    tex: Option<(f64, TextureHandle)>,
}

/// Makes sure the pictures round `t` are read into `frames`: a few seconds ahead when playing,
/// read again before the window runs out.
fn ask_frames(frames: &Arc<Mutex<Frames>>, path: &str, t: f64, playing: bool, ctx: &egui::Context) {
    {
        let mut f = frames.lock().unwrap();
        let ahead = if playing { 2.0 } else { 0.0 };
        if f.path == path && t >= f.want.0 && t < f.want.1 - ahead {
            return;
        }
        if let Some(s) = f.stop.take() {
            s.store(true, Ordering::Relaxed);
        }
        let from = (t - 0.5).max(0.0);
        let to = t + if playing { 8.0 } else { 2.0 };
        f.path = path.to_string();
        f.want = (from, to);
        f.round += 1;
        f.busy = true;
        let stop = Arc::new(AtomicBool::new(false));
        f.stop = Some(stop.clone());
        let (round, me, path, ctx) = (f.round, frames.clone(), path.to_string(), ctx.clone());
        std::thread::spawn(move || {
            let r = cuts::frames_at(&path, from, to - from, &stop);
            let mut f = me.lock().unwrap();
            if f.round != round {
                return;
            }
            f.busy = false;
            match r {
                Ok(got) => {
                    f.got = got;
                    f.note.clear();
                }
                Err(e) if e == "stopped" => {}
                Err(e) => f.note = e,
            }
            ctx.request_repaint();
        });
    }
}

/// A piece fed to another node: the source of the recording, where it sits in the file, its
/// clock, a stable key for the rows of the node it feeds, its name, and the phone id it suggests
/// (the file's: every piece is from the same device).
pub struct Fed {
    pub src: NodeId,
    pub piece: Piece,
    pub clock: String,
    pub uid: String,
    pub name: String,
    pub phone: String,
}

/// Where a node downstream put a piece: its start and length, on real time (`real`) or on its
/// group's own clock, and which node.
#[derive(Clone, Debug)]
pub struct Placed {
    pub n: usize,
    pub at: f64,
    pub len: f64,
    pub real: bool,
    pub by: String,
}

/// One frame's text: 83.25 s as 1:23.25.
pub fn clock_text(t: f64) -> String {
    let m = (t / 60.0).floor();
    let s = t - m * 60.0;
    if m > 0.0 { format!("{m:.0}:{s:05.2}") } else { format!("{s:.2}") }
}

/// The lanes a run will make, before it has: so the node keeps its size. `compare`: the keys and
/// names of the recordings wired into `compare with`.
fn planned_lanes(video: bool, audio: bool, compare: &[(String, String)]) -> Vec<Lane> {
    let empty = |key: &str, name: &str, role: Role, thr: f32| Lane { key: key.into(), name: name.into(), role, unit: String::new(), curve: Vec::new(), peaks: Vec::new(), thr: Some(thr), alone: key != "noise", note: "not read yet: the graph reads it when it runs".into() };
    let mut v = Vec::new();
    if video {
        v.push(empty("picture", "picture", Role::Picture, cuts::PICTURE_THR));
        v.push(empty("keyframes", "keyframes and gaps", Role::Picture, 0.5));
    }
    if audio {
        v.push(empty("level", "sound level", Role::Sound, cuts::LEVEL_THR));
        v.push(empty("noise", "background noise", Role::Sound, cuts::NOISE_THR));
    }
    for (key, name) in compare {
        v.push(empty(key, &format!("compare: {name}"), Role::Sound, corroborate::audio_match::GAP_MIN_S as f32));
    }
    v
}

/// A recording's name: its file's, when its node still has the module's name.
fn rec_name(app: &App, f: NodeId) -> String {
    match app.source_file(f) {
        Some(p) if app.nodes[f].label == "SOURCER" => p.rsplit(['\\', '/']).next().unwrap_or(&p).to_string(),
        _ => app.node_name(f),
    }
}

impl App {
    /// The source node of the recording a cut detector splits: its video's, else its audio's.
    /// Both wired from different sources is a mistake it names.
    pub(crate) fn cut_source(&self, i: NodeId) -> Result<NodeId, String> {
        let v = self.graph.feeder(i, "video").map(|(f, _)| f);
        let a = self.graph.feeder(i, "audio").map(|(f, _)| f);
        match (v, a) {
            (Some(v), Some(a)) if v != a => Err("its audio and video come from different sources: wire one recording".into()),
            (Some(f), _) | (None, Some(f)) => Ok(f),
            (None, None) => Err("wire a recording's audio or video into it".into()),
        }
    }

    /// What a run of cut detector `i` reads.
    pub(crate) fn cut_job(&self, i: NodeId) -> Result<Job, String> {
        let src = self.cut_source(i)?;
        let downloads = self.downloads_dir();
        let path = self.rec_path(src, &downloads)?;
        let video = self.graph.feeder(i, "video").is_some();
        let audio = self.graph.feeder(i, "audio").is_some();
        let source = Some(self.nodes[src].state.clone());
        let mut compare = Vec::new();
        for (key, f) in self.cut_compared(i) {
            if let Ok(p) = self.rec_path(f, &downloads) {
                compare.push(Other { key, name: rec_name(self, f), path: p, source: Some(self.nodes[f].state.clone()) });
            }
        }
        Ok(Job { path, video, audio, source, compare })
    }

    /// The file SOURCER `f` reads: a file, or the copy of a video from the web in `dir` (its
    /// SOURCER downloads it first: the queue runs it before).
    fn rec_path(&self, f: NodeId, dir: &std::path::Path) -> Result<String, String> {
        let Kind::Sourcer { input } = &self.nodes[f].kind else { return Err("wire a SOURCER's audio or video into it".into()) };
        match sourcer_node::input(input) {
            Some(runtime::Input::File(p)) => Ok(p),
            Some(runtime::Input::Url(u)) => sourcer_node::listed(&u, dir).ok_or_else(|| format!("'{}' kept no copy of the video", self.node_name(f))),
            Some(runtime::Input::Mic(_)) => Err("a microphone is not read: record it to a file first".into()),
            None => Err("its SOURCER has nothing to open".into()),
        }
    }

    /// The recordings wired into node `i`'s `compare with` inputs: each lane's key and the source.
    pub(crate) fn cut_compared(&self, i: NodeId) -> Vec<(String, NodeId)> {
        self.graph.nodes()[i]
            .spec
            .inputs
            .iter()
            .filter_map(|p| {
                let n = p.name.strip_prefix("compare with ")?;
                let (f, _) = self.graph.feeder(i, p.name)?;
                Some((format!("compare {n}"), f))
            })
            .collect()
    }

    /// The piece on output `out` ("audio 2", "video 2") of node `f`, when `f` is a Cut detector.
    pub(crate) fn piece_feed(&self, f: NodeId, out: &str) -> Option<Fed> {
        if !matches!(self.nodes[f].kind, Kind::CutDetector { .. }) {
            return None;
        }
        let (base, n) = cd::piece_of_port(out)?;
        if base != "audio" && base != "video" {
            return None;
        }
        let v = self.cut_view(f);
        let piece = *v.pieces.get(n - 1)?;
        let src = v.src?;
        let file = rec_name(self, src);
        Some(Fed { src, piece, clock: self.piece_clock(f, out)?, uid: format!("{}.{n}", self.nodes[f].uid), name: format!("{file} · piece {n}"), phone: format!("{file} (cut)") })
    }

    /// Where the Audio match nodes fed node `i`'s pieces put them.
    pub(crate) fn placed(&self, i: NodeId) -> Vec<Placed> {
        let mut out = Vec::new();
        for (t, inp) in self.graph.wires().iter().filter(|w| w.from == i).map(|w| (w.to, self.graph.nodes()[w.to].spec.inputs[w.to_port].name)).collect::<Vec<_>>() {
            if !matches!(self.nodes[t].kind, Kind::AudioMatch { .. }) {
                continue;
            }
            let Some((_, out_name)) = self.graph.feeder(t, inp) else { continue };
            let Some((_, n)) = cd::piece_of_port(out_name) else { continue };
            let Some(k) = inp.strip_prefix("audio ").and_then(|k| k.parse::<usize>().ok()) else { continue };
            let mv = self.match_view(t);
            if let Some(r) = mv.rows.iter().find(|r| r.n == k) {
                if let Some(at) = r.start {
                    out.push(Placed { n, at, len: r.len, real: mv.real, by: self.node_name(t) });
                }
            }
        }
        out.sort_by(|a, b| a.at.total_cmp(&b.at));
        out
    }

    pub(crate) fn cut_view(&self, i: NodeId) -> Rc<CutView> {
        #[cfg(not(test))]
        if let Some(v) = self.cut_ui.cache.borrow().get(&i) {
            return v.clone();
        }
        let v = Rc::new(self.cut_view_now(i));
        #[cfg(not(test))]
        self.cut_ui.cache.borrow_mut().insert(i, v.clone());
        v
    }

    fn cut_view_now(&self, i: NodeId) -> CutView {
        let rows: &[(String, String)] = self.nodes[i].kind.rows();
        let mut v = CutView::default();
        match self.cut_source(i) {
            Ok(f) => v.src = Some(f),
            Err(e) => v.problem = Some(e),
        }
        v.clock = v.src.and_then(|f| self.clock_of(f)).unwrap_or_default();
        v.scan = self.nodes[i].state.lock().unwrap().cut.clone();
        let video = self.graph.feeder(i, "video").is_some();
        let audio = self.graph.feeder(i, "audio").is_some();
        v.lanes = match &v.scan {
            Some(s) => s.lanes.iter().map(|l| Lane { curve: Vec::new(), ..l.clone() }).collect(),
            None => {
                let compare: Vec<(String, String)> = self.cut_compared(i).into_iter().map(|(k, f)| (k, rec_name(self, f))).collect();
                planned_lanes(video, audio, &compare)
            }
        };
        v.frame_s = v.scan.as_ref().map_or(1.0 / 30.0, |s| s.frame_s);
        // One lane per wired events input, named by what feeds it.
        for p in self.graph.nodes()[i].spec.inputs.iter().filter(|p| p.name.starts_with("events ")) {
            let Some((f, o)) = self.graph.feeder(i, p.name) else { continue };
            let mut lane = Lane { key: p.name.to_string(), name: format!("{} › {o}", self.node_name(f)), role: Role::Events, unit: String::new(), curve: Vec::new(), peaks: Vec::new(), thr: None, alone: false, note: String::new() };
            match self.value_from(f, o) {
                Ok(Data::Events(evs)) => {
                    for e in evs {
                        if !e.clock.is_empty() && !v.clock.is_empty() && e.clock != v.clock {
                            v.off_clock += 1;
                            continue;
                        }
                        lane.peaks.push(cuts::Peak { t: e.t_start, value: 1.0, error_s: e.error_s, note: if e.label.is_empty() { e.detail } else { e.label }, lasting: None });
                    }
                    lane.note = format!("{} events; each confirms a cut within its error (one frame when it has none)", lane.peaks.len());
                }
                Ok(_) => lane.note = "not events".into(),
                Err(e) => lane.note = e,
            }
            v.lanes.push(lane);
        }
        v.thr = v.lanes.iter().map(|l| l.thr.map(|d| cd::threshold(rows, &l.key).unwrap_or(d))).collect();
        v.len = v.src.and_then(|f| self.source_length(f)).or(v.scan.as_ref().map(|s| s.len)).unwrap_or(0.0);
        if v.scan.is_some() {
            v.cuts = cuts::find_cuts(&v.lanes, &v.thr, &cd::switches(rows), v.frame_s);
        } else {
            // Before a run: hand cuts only.
            v.cuts = cuts::find_cuts(&[], &[], &cd::switches(rows), v.frame_s);
        }
        if v.len > 0.0 {
            v.pieces = cuts::pieces(v.len, &v.cuts);
        }
        v
    }

    /// The clock piece `out` ("audio 3") of cut detector `f` is on: the first piece is the file's
    /// own clock (its moments are the file's), every later one a clock of its own.
    pub(crate) fn piece_clock(&self, f: NodeId, out: &str) -> Option<String> {
        if !matches!(self.nodes[f].kind, Kind::CutDetector { .. }) {
            return None;
        }
        let (_, n) = cd::piece_of_port(out)?;
        let src = self.cut_source(f).ok()?;
        let c = self.clock_of(src)?;
        Some(if n == 1 { c } else { format!("{c}~{n}") })
    }

    /// Cut detector `i`'s output `out`.
    pub(crate) fn cut_value(&self, i: NodeId, out: &str) -> Result<Data, String> {
        let v = self.cut_view(i);
        if let Some(p) = &v.problem {
            return Err(p.clone());
        }
        if out == "cuts" {
            if v.scan.is_none() && v.cuts.is_empty() {
                return Err("not read yet".into());
            }
            let on: Vec<&Cut> = v.cuts.iter().filter(|c| c.on()).collect();
            return Ok(Data::Events(
                on.iter()
                    .enumerate()
                    .map(|(k, c)| Event { clock: v.clock.clone(), t_start: c.t_pic, t_end: c.t_pic, label: if c.hand() { format!("cut {} (by hand)", k + 1) } else { format!("cut {}", k + 1) }, detail: cut_words(&v, c), error_s: c.error_s })
                    .collect(),
            ));
        }
        let (base, n) = cd::piece_of_port(out).ok_or_else(|| format!("no output {out}"))?;
        let p = *v.pieces.get(n - 1).ok_or_else(|| format!("there are {} pieces now", v.pieces.len()))?;
        match base {
            "length" => Ok(Data::Custom { kind: "length", json: format!("{{\"seconds\":{}}}", p.len()) }),
            "time" => {
                let (f, o) = self.graph.feeder(i, "time").ok_or("wire the recording's time into the detector")?;
                let Data::Time { clock, t, utc: Some(u), error_s, source } = self.value_from(f, o)? else { return Err("its time has no date yet".into()) };
                // The wired moment is on the file's clock: the file starts that much before it.
                let start = if clock == v.clock { u - t } else { u };
                Ok(Data::Time {
                    clock: self.piece_clock(i, out).unwrap_or_default(),
                    t: 0.0,
                    utc: Some(start + p.a0),
                    error_s,
                    source: format!("as played: {source}, + {} s into the file (cuts before it may hide time)", clock_text(p.a0)),
                })
            }
            _ => Err("a stream, not a value".into()),
        }
    }

    /// Keeps each cut detector's piece outputs as many as its pieces (saved in its rows).
    pub(crate) fn sync_cut_ports(&mut self) {
        for i in 0..self.nodes.len() {
            if self.nodes[i].hidden || !matches!(self.nodes[i].kind, Kind::CutDetector { .. }) {
                continue;
            }
            let v = self.cut_view(i);
            let n = if v.scan.is_some() && v.len > 0.0 { v.pieces.len().max(1) } else { cd::saved_pieces(self.nodes[i].kind.rows()) };
            if let Kind::CutDetector { rows } = &mut self.nodes[i].kind {
                if cd::saved_pieces(rows) != n || cd::get(rows, "pieces").is_none() {
                    cd::set(rows, "pieces", Some(n.to_string()));
                }
            }
            if self.graph.nodes()[i].spec.outputs.len() != cd::outputs(n).len() {
                self.graph.set_outputs(i, cd::outputs(n));
            }
        }
        self.cut_play_tick();
    }

    /// A cut being played pauses where its play ends (or stops being one when the play was
    /// paused, or moved to another node).
    fn cut_play_tick(&mut self) {
        let Some((n, end)) = self.cut_ui.until else { return };
        if !self.ph.playing || self.selected != Some(n) {
            self.cut_ui.until = None;
            return;
        }
        let Some(clock) = self.clock_of(n) else { return };
        if self.playhead_on(&clock).is_some_and(|t| t >= end) {
            // A slow frame may have carried it past: it stops on the end.
            self.set_playhead(&clock, end);
            self.ph.playing = false;
            self.cut_ui.until = None;
        }
    }

    /// Node `i`'s player goes to cut `k` (its place among the node's cuts) and plays it, from
    /// `CUT_PLAY_S` before to `CUT_PLAY_S` after, picture and sound.
    pub(crate) fn play_cut(&mut self, i: NodeId, k: usize) {
        let v = self.cut_view(i);
        let Some(c) = v.cuts.get(k) else { return };
        self.cut_ui.focus.insert(i, k);
        let t = c.t_pic.min(c.t_snd);
        let end = c.t_pic.max(c.t_snd) + CUT_PLAY_S;
        self.play_from(i, (t - CUT_PLAY_S).max(0.0));
        self.cut_ui.until = Some((i, end));
    }

    /// Node `i`'s playhead to `t` on its recording, playing.
    fn play_from(&mut self, i: NodeId, t: f64) {
        self.pick_only(i);
        if let Some(c) = self.clock_of(i) {
            self.set_playhead(&c, t);
        }
        self.ph.playing = true;
    }

    /// Play or pause node `i`'s player.
    fn play_pause_cut(&mut self, i: NodeId) {
        if self.selected == Some(i) && self.ph.playing {
            self.ph.playing = false;
            self.cut_ui.until = None;
            return;
        }
        self.pick_only(i);
        self.cut_ui.until = None;
        self.ph.playing = true;
    }

    /// Node `i`'s playhead to `t` on its recording, without playing.
    fn seek_cut(&mut self, i: NodeId, t: f64) {
        if let Some(c) = self.clock_of(i) {
            self.set_playhead(&c, t);
        }
    }

    /// The times of the pictures node `i`'s player has read.
    #[cfg(test)]
    pub(crate) fn cut_player_times(&self, i: NodeId) -> Vec<f64> {
        self.cut_ui.players.get(&i).map_or(Vec::new(), |p| p.frames.lock().unwrap().got.iter().map(|f| f.0).collect())
    }

    fn cut_rows(&mut self, i: NodeId) -> Option<&mut Vec<(String, String)>> {
        match &mut self.nodes[i].kind {
            Kind::CutDetector { rows } => Some(rows),
            _ => None,
        }
    }

    /// Switches cut `c` of node `i` the other way.
    pub(crate) fn toggle_cut(&mut self, i: NodeId, c: &Cut) {
        let want = !c.on();
        let near = |t: f64| (t - c.t_pic).abs() <= cuts::SWITCH_S || (t - c.t_snd).abs() <= cuts::SWITCH_S;
        let Some(rows) = self.cut_rows(i) else { return };
        rows.retain(|(k, _)| !k.strip_prefix("cut ").and_then(|t| t.trim().parse::<f64>().ok()).is_some_and(near));
        // Back to what the lanes say needs no row; a hand cut switched off is gone.
        if want != c.auto && !(c.hand() && !want) {
            cd::set(rows, &cd::cut_key(c.t_pic), Some(if want { Switch::On } else { Switch::Off }.word().into()));
        }
        let label = self.nodes[i].label.clone();
        let old = if c.on() { "on" } else { "off" };
        let new = if want { "on" } else { "off" };
        self.changes.push(bus::SettingChange { utc: timebase::now_unix(), node: self.nodes[i].uid.clone(), label, name: format!("cut at {} s", clock_text(c.t_pic)), old: old.into(), new: new.into() });
        self.cut_ui.cache.borrow_mut().remove(&i);
        self.save_project();
    }

    /// A cut by hand at `t` seconds in node `i`'s recording.
    pub(crate) fn hand_cut(&mut self, i: NodeId, t: f64) {
        let Some(rows) = self.cut_rows(i) else { return };
        cd::set(rows, &cd::cut_key(t), Some(Switch::Hand.word().into()));
        let label = self.nodes[i].label.clone();
        self.changes.push(bus::SettingChange { utc: timebase::now_unix(), node: self.nodes[i].uid.clone(), label, name: format!("cut at {} s", clock_text(t)), old: "none".into(), new: "by hand".into() });
        self.cut_ui.cache.borrow_mut().remove(&i);
        self.save_project();
    }

    /// Lane `key`'s threshold in node `i` set to `v` (`log`: the drag has ended, so it goes into
    /// the result file's setting changes, from `was`).
    pub(crate) fn set_cut_threshold(&mut self, i: NodeId, key: &str, v: f32, log: Option<f32>) {
        let Some(rows) = self.cut_rows(i) else { return };
        cd::set(rows, &format!("thr {key}"), Some(format!("{v:.3}")));
        if let Some(was) = log {
            let label = self.nodes[i].label.clone();
            self.changes.push(bus::SettingChange { utc: timebase::now_unix(), node: self.nodes[i].uid.clone(), label, name: format!("threshold of {key}"), old: format!("{was:.3}"), new: format!("{v:.3}") });
            self.save_project();
        }
        self.cut_ui.cache.borrow_mut().remove(&i);
    }
}

/// A cut in words: its times and the cues behind it.
fn cut_words(v: &CutView, c: &Cut) -> String {
    let mut parts = Vec::new();
    if c.hand() {
        parts.push("by hand: no cue reached".to_string());
    }
    let lead = c.lead();
    if lead.abs() >= v.frame_s / 2.0 {
        parts.push(format!("picture at {} s, sound at {} s ({} {:.2} s)", clock_text(c.t_pic), clock_text(c.t_snd), if lead > 0.0 { "J-cut: sound first by" } else { "L-cut: sound later by" }, lead.abs()));
    }
    for q in &c.cues {
        let l = &v.lanes[q.lane];
        let th = v.thr.get(q.lane).copied().flatten().unwrap_or(0.0);
        parts.push(format!("{} {:.2} at {} s ({} threshold {th:.2}{})", l.name, q.value, clock_text(q.t), if q.passes { "over" } else { "under" }, if q.flash { ", a flash" } else { "" }));
    }
    for e in &c.confirms {
        parts.push(format!("confirmed by {} at {} s ± {:.2}", v.lanes[e.lane].name, clock_text(e.t), e.error_s));
    }
    if !c.why.is_empty() && !c.on() {
        parts.push(c.why.clone());
    }
    match c.switch {
        Some(Switch::On) if !c.auto => parts.push("switched on by hand".into()),
        Some(Switch::Off) => parts.push("switched off by hand".into()),
        _ => {}
    }
    parts.join("; ")
}

/// Node `i`'s part of the result file: its lanes, every cut (on or off, with its cues) and the
/// pieces.
pub fn json(app: &App, i: NodeId) -> String {
    let v = app.cut_view(i);
    let num = |x: f64| if x.is_finite() { format!("{x:.4}") } else { "null".into() };
    let lanes: Vec<String> = v
        .lanes
        .iter()
        .zip(&v.thr)
        .map(|(l, t)| format!("{{\"key\":\"{}\",\"name\":\"{}\",\"marks\":{},\"threshold\":{},\"note\":\"{}\"}}", json_escape(&l.key), json_escape(&l.name), l.peaks.len(), t.map_or("null".into(), |t| num(t as f64)), json_escape(&l.note)))
        .collect();
    let cuts_j: Vec<String> = v
        .cuts
        .iter()
        .map(|c| {
            format!(
                "{{\"t_picture\":{},\"t_sound\":{},\"error_s\":{},\"on\":{},\"by_lanes\":{},\"switch\":{},\"words\":\"{}\"}}",
                num(c.t_pic),
                num(c.t_snd),
                num(c.error_s),
                c.on(),
                c.auto,
                c.switch.map_or("null".into(), |s| format!("\"{}\"", s.word())),
                json_escape(&cut_words(&v, c))
            )
        })
        .collect();
    let pieces: Vec<String> = v.pieces.iter().map(|p| format!("{{\"n\":{},\"video\":[{},{}],\"audio\":[{},{}]}}", p.n, num(p.v0), num(p.v1), num(p.a0), num(p.a1))).collect();
    let file = v.scan.as_ref().map_or(String::new(), |s| s.path.clone());
    format!(",\"file\":\"{}\",\"clock\":\"{}\",\"length_s\":{},\"lanes\":[{}],\"cuts\":[{}],\"pieces\":[{}]", json_escape(&file), json_escape(&v.clock), num(v.len), lanes.join(","), cuts_j.join(","), pieces.join(","))
}

// ---------------------------------------------------------------- on the canvas

const NODE_W: f32 = 1000.0;
const NOTE_H: f32 = 24.0;
/// The player: its height, and its picture's size.
const PLAYER_H: f32 = 150.0;
const PIC_W: f32 = 240.0;
const PIC_H: f32 = 135.0;
/// A cut plays from this long before it to this long after it, seconds.
pub const CUT_PLAY_S: f64 = 2.0;
const DONE: Color32 = Color32::from_rgb(140, 210, 120);
const FILM_H: f32 = 34.0;
const WAVE_H: f32 = 30.0;
const LANE_H: f32 = 30.0;
const RULER_H: f32 = 16.0;
const LABEL_W: f32 = 132.0;
const TABLE_HEAD: f32 = 18.0;
const TABLE_ROW: f32 = 17.0;
const PLACED_H: f32 = 44.0;
const TRACK: Color32 = Color32::from_rgb(29, 32, 37);
const SEAM: Color32 = Color32::from_rgb(16, 18, 21);
const RED: Color32 = Color32::from_rgb(224, 82, 79);
const OFF: Color32 = Color32::from_rgb(120, 126, 136);
const CONFIRM: Color32 = Color32::from_rgb(125, 180, 230);
const WAVE: Color32 = Color32::from_rgb(224, 164, 88);
const PICTURE: Color32 = Color32::from_rgb(224, 200, 90);
const SOUND: Color32 = Color32::from_rgb(224, 164, 88);
const EVENT: Color32 = Color32::from_rgb(224, 122, 106);

fn lane_color(r: Role) -> Color32 {
    match r {
        Role::Picture => PICTURE,
        Role::Sound | Role::Both => SOUND,
        Role::Events => EVENT,
    }
}

/// Where the parts of the node sit, unzoomed, from the top of the body.
struct Geom {
    film: f32,
    wave: f32,
    lanes: f32,
    ruler: f32,
    player: f32,
    table: f32,
}

fn geom() -> Geom {
    let film = NOTE_H;
    let wave = film + FILM_H;
    let lanes = wave + WAVE_H;
    Geom { film, wave, lanes, ruler: 0.0, player: 0.0, table: 0.0 }
}

fn layout(n_lanes: usize) -> Geom {
    let mut g = geom();
    g.ruler = g.lanes + n_lanes as f32 * LANE_H;
    g.player = g.ruler + RULER_H + 8.0;
    g.table = g.player + PLAYER_H + 8.0;
    g
}

/// What a click in the player or on the rows asks for.
enum Act {
    Play(usize),
    PlayPause,
    Toggle(usize),
    Seek(f64),
    Hand(f64),
}

pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, standard: Vec2) -> Vec2 {
        let v = app.cut_view(i);
        let g = layout(v.lanes.len());
        let rows = v.pieces.len().max(1);
        let strip = if app.placed(i).is_empty() { 0.0 } else { PLACED_H };
        // Tall enough for its own parts and for a port row per output.
        Vec2::new(NODE_W, (HEAD + 4.0 + g.table + TABLE_HEAD + rows as f32 * TABLE_ROW + strip + 30.0).max(standard.y))
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let z = app.zoom;
        let v = app.cut_view(i);
        let area = crate::look::content(app, ui, i);
        let p = ui.painter().with_clip_rect(ui.clip_rect());
        let font = |s: f32| FontId::proportional(s * z);
        let g = layout(v.lanes.len());
        let y = |u: f32| area.top() + u * z;
        let plot_x = (area.left() + LABEL_W * z, area.right() - 6.0 * z);
        if plot_x.1 - plot_x.0 < 20.0 {
            return;
        }
        let len = v.len.max(1e-3);
        let tx = |t: f64| plot_x.0 + (t / len) as f32 * (plot_x.1 - plot_x.0);
        let xt = |x: f32| ((x - plot_x.0) / (plot_x.1 - plot_x.0)) as f64 * len;

        // The line on top: each step of the run and how far it has got, then what it found; or
        // what is missing.
        let (status, prog, running) = {
            let s = app.nodes[i].state.lock().unwrap();
            (s.status.clone(), s.progress.clone(), s.running)
        };
        let mut act: Option<Act> = None;
        {
            let mid = y(NOTE_H * 0.5);
            let mut x = area.left();
            // A dot (the step's state) when `dot` is given, then the text.
            let mut put = |dot: Option<Color32>, text: String, col: Color32, tip: Option<String>| -> Rect {
                if let Some(d) = dot {
                    p.circle_filled(Pos2::new(x + 3.5 * z, mid), 3.5 * z, d);
                    x += 11.0 * z;
                }
                let g = p.layout_no_wrap(text, font(11.0), col);
                let r = Rect::from_min_size(Pos2::new(x, mid - g.size().y * 0.5), g.size());
                p.galley(r.min, g, col);
                if let Some(t) = tip.filter(|t| !t.is_empty()) {
                    ui.interact(r, ui.id().with(("cut-step", i, x as i32)), Sense::hover()).on_hover_text(t);
                }
                x = r.right() + 10.0 * z;
                r
            };
            if let Some(e) = &v.problem {
                put(None, e.clone(), WARN, None);
            } else if prog.steps.is_empty() {
                put(None, if status.is_empty() { "not read yet: Run reads the recording".into() } else { status.clone() }, if running { TEXT } else { DIM }, None);
            } else {
                for s in &prog.steps {
                    match s.state {
                        StepState::Done => {
                            put(Some(DONE), s.name.clone(), DONE, Some(s.note.clone()));
                        }
                        StepState::Failed => {
                            put(Some(RED), s.name.clone(), RED, Some(s.note.clone()));
                        }
                        StepState::Waiting => {
                            put(Some(OFF), s.name.clone(), DIM, None);
                        }
                        StepState::Working => {
                            let pct = if s.frac >= 0.0 { format!(" {:.0}%", s.frac * 100.0) } else { String::new() };
                            let r = put(Some(PLAYHEAD), format!("{}{pct}", s.name), TEXT, Some(status.clone()));
                            // A bar under it: how far, or a block running along it when that is
                            // not known.
                            let bar = Rect::from_min_max(Pos2::new(r.left(), r.bottom() + 1.0 * z), Pos2::new(r.right(), r.bottom() + 3.0 * z));
                            p.rect_filled(bar, 0.0, TRACK);
                            if s.frac >= 0.0 {
                                p.rect_filled(Rect::from_min_max(bar.min, Pos2::new(bar.left() + bar.width() * s.frac, bar.bottom())), 0.0, PLAYHEAD);
                            } else {
                                let ph = (ui.ctx().input(|i| i.time) * 0.8).fract() as f32;
                                let w = bar.width() * 0.3;
                                let x0 = bar.left() + (bar.width() + w) * ph - w;
                                p.rect_filled(Rect::from_min_max(Pos2::new(x0.max(bar.left()), bar.top()), Pos2::new((x0 + w).min(bar.right()), bar.bottom())), 0.0, PLAYHEAD);
                            }
                        }
                    }
                }
                let on = v.cuts.iter().filter(|c| c.on()).count();
                let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
                match (prog.took, prog.started) {
                    (Some(took), _) => {
                        put(None, format!("done in {}", crate::jobs::took_text(took)), DONE, None);
                        put(None, format!("{on} of {} on · {}", plural(v.cuts.len(), "cut"), plural(v.pieces.len(), "piece")), TEXT, None);
                    }
                    (None, Some(t0)) => {
                        put(None, sourcer_node::fmt_secs(t0.elapsed().as_secs_f64()), DIM, None);
                    }
                    _ => {}
                }
                if v.off_clock > 0 {
                    put(None, format!("{} events on another clock left out", v.off_clock), WARN, None);
                }
            }
            if running {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        // Labels on the left.
        let label = |text: &str, top: f32, h: f32, col: Color32| {
            p.text(Pos2::new(area.left() + 2.0 * z, y(top + h * 0.5)), Align2::LEFT_CENTER, text, font(11.0), col);
        };
        label("frames", g.film, FILM_H, DIM);
        label("sound", g.wave, WAVE_H, DIM);
        let full = |top: f32, h: f32| Rect::from_min_max(Pos2::new(plot_x.0, y(top)), Pos2::new(plot_x.1, y(top + h) - 2.0 * z));

        // The filmstrip: a picture at the middle of each tile.
        let film = full(g.film, FILM_H);
        p.rect_filled(film, 0.0, TRACK);
        if let Some(scan) = v.scan.as_ref().filter(|s| !s.thumbs.is_empty()) {
            let run = app.nodes[i].state.lock().unwrap().run_id;
            let fresh = app.cut_ui.strips.get(&i).is_none_or(|s| s.run != run);
            if fresh {
                let n = scan.thumbs.len();
                let max_side = ui.ctx().input(|inp| inp.max_texture_side);
                let (cols, rows, step) = strip_layout(n, max_side);
                let (tw, th) = (cuts::THUMB_W, cuts::THUMB_H);
                let mut rgb = vec![0u8; cols * tw * rows * th * 3];
                for (k, t) in scan.thumbs.iter().step_by(step).enumerate() {
                    let (cx, cy) = (k % cols, k / cols);
                    for row in 0..th {
                        let at = ((cy * th + row) * cols * tw + cx * tw) * 3;
                        rgb[at..at + tw * 3].copy_from_slice(&t.rgb[row * tw * 3..(row + 1) * tw * 3]);
                    }
                }
                let img = ColorImage::from_rgb([cols * tw, rows * th], &rgb);
                let tex = ui.ctx().load_texture(format!("cut-strip-{i}"), img, TextureOptions::LINEAR);
                app.cut_ui.strips.insert(i, Strip { run, tex, cols, rows, step });
            }
            if let Some(s) = app.cut_ui.strips.get(&i) {
                let n = scan.thumbs.len();
                let kept = n.div_ceil(s.step);
                let tile = film.height() * cuts::THUMB_W as f32 / cuts::THUMB_H as f32;
                let tiles = ((film.width() / tile).ceil() as usize).max(1);
                // A tile's picture, inset by half a pixel so its neighbours do not bleed in.
                let (du, dv) = (0.5 / (s.cols * cuts::THUMB_W) as f32, 0.5 / (s.rows * cuts::THUMB_H) as f32);
                for k in 0..tiles {
                    let x0 = film.left() + k as f32 * tile;
                    let t = xt((x0 + tile * 0.5).min(film.right()));
                    let near = (scan.thumbs.partition_point(|th| th.t < t).min(n - 1) / s.step).min(kept - 1);
                    let r = Rect::from_min_max(Pos2::new(x0, film.top()), Pos2::new((x0 + tile).min(film.right()), film.bottom()));
                    let u0 = (near % s.cols) as f32 / s.cols as f32;
                    let v0 = (near / s.cols) as f32 / s.rows as f32;
                    let u1 = u0 + (r.width() / tile) / s.cols as f32;
                    p.image(s.tex.id(), r, Rect::from_min_max(Pos2::new(u0 + du, v0 + dv), Pos2::new(u1 - du, v0 + 1.0 / s.rows as f32 - dv)), Color32::WHITE);
                }
            }
        }

        // The waveform: the source's, every 0.1 s.
        let wave = full(g.wave, WAVE_H);
        p.rect_filled(wave, 0.0, TRACK);
        if let Some(src) = v.src {
            let s = app.nodes[src].state.lock().unwrap();
            let pk = &s.source.peaks;
            if !pk.is_empty() {
                let cols = wave.width() as usize;
                for c in 0..cols {
                    let (a, b) = (xt(wave.left() + c as f32), xt(wave.left() + c as f32 + 1.0));
                    let (k0, k1) = (((a * 10.0) as usize).min(pk.len()), ((b * 10.0).ceil() as usize).clamp(0, pk.len()));
                    if k1 <= k0 {
                        continue;
                    }
                    let (lo, hi) = pk[k0..k1].iter().fold((0.0f32, 0.0f32), |(lo, hi), q| (lo.min(q[0]), hi.max(q[1])));
                    let mid = wave.center().y;
                    let x = wave.left() + c as f32 + 0.5;
                    p.line_segment([Pos2::new(x, mid - hi * wave.height() * 0.5), Pos2::new(x, mid - lo * wave.height() * 0.5)], Stroke::new(1.0, alpha(WAVE, 0.7)));
                }
            }
        }

        // The lanes.
        let scan_lanes: &[Lane] = v.scan.as_ref().map_or(&[], |s| &s.lanes);
        let mut drag_end: Option<(usize, f32, f32)> = None;
        for (k, lane) in v.lanes.iter().enumerate() {
            let top = g.lanes + k as f32 * LANE_H;
            let r = full(top, LANE_H);
            p.rect_filled(r, 0.0, TRACK);
            let col = lane_color(lane.role);
            let name_r = Rect::from_min_max(Pos2::new(area.left(), y(top)), Pos2::new(plot_x.0 - 4.0 * z, y(top + LANE_H)));
            let galley = p.layout_no_wrap(lane.name.clone(), font(11.0), col);
            let shown = if galley.size().x > name_r.width() { format!("{}…", lane.name.chars().take(16).collect::<String>()) } else { lane.name.clone() };
            label(&shown, top, LANE_H, col);
            ui.interact(name_r, ui.id().with(("cut-lane", i, k)), Sense::hover()).on_hover_text(format!("{}\n{}", lane.name, lane.note));
            let th = v.thr.get(k).copied().flatten();
            let curve: &[(f64, f32)] = scan_lanes.iter().find(|l| l.key == lane.key).map_or(&[], |l| &l.curve);
            let top_v = lane.peaks.iter().map(|q| q.value).chain(curve.iter().map(|c| c.1)).fold(th.unwrap_or(1.0) * 2.0, f32::max).max(1e-3);
            let vy = |val: f32| r.bottom() - (val / top_v).clamp(0.0, 1.0) * (r.height() - 2.0 * z);
            // The curve: the highest value in each column.
            if !curve.is_empty() {
                let cols = r.width().max(1.0) as usize;
                let mut best = vec![0.0f32; cols];
                for &(t, val) in curve {
                    let c = (((t / len) as f32) * cols as f32) as usize;
                    if let Some(b) = best.get_mut(c.min(cols - 1)) {
                        *b = b.max(val);
                    }
                }
                for (c, &b) in best.iter().enumerate().filter(|(_, b)| **b > 0.0) {
                    let x = r.left() + c as f32 + 0.5;
                    p.line_segment([Pos2::new(x, r.bottom()), Pos2::new(x, vy(b))], Stroke::new(1.0, alpha(col, 0.45)));
                }
            }
            // The marks.
            for q in &lane.peaks {
                let x = tx(q.t);
                if lane.role == Role::Events {
                    let e = (q.error_s.max(v.frame_s) / len) as f32 * (plot_x.1 - plot_x.0);
                    p.line_segment([Pos2::new(x - e, r.center().y), Pos2::new(x + e, r.center().y)], Stroke::new(1.0 * z, alpha(EVENT, 0.6)));
                    p.circle_filled(Pos2::new(x, r.center().y), 3.0 * z, EVENT);
                } else {
                    let passes = th.is_some_and(|th| q.value >= th);
                    p.line_segment([Pos2::new(x, r.bottom()), Pos2::new(x, vy(q.value))], Stroke::new(if passes { 2.0 } else { 1.0 } * z, if passes { col } else { alpha(col, 0.5) }));
                }
            }
            // The threshold, and its handle at the right end.
            if let Some(th) = th {
                let held = app.cut_ui.drag.filter(|d| d.0 == i && d.1 == k).map(|d| d.2);
                let th = held.unwrap_or(th);
                let ly = vy(th);
                p.extend(egui::Shape::dashed_line(&[Pos2::new(r.left(), ly), Pos2::new(r.right(), ly)], Stroke::new(1.0 * z, alpha(TEXT, 0.35)), 4.0 * z, 3.0 * z));
                let handle = Rect::from_center_size(Pos2::new(r.right() - 5.0 * z, ly), Vec2::splat(10.0 * z));
                p.add(egui::Shape::convex_polygon(vec![Pos2::new(handle.right(), ly - 4.0 * z), Pos2::new(handle.right(), ly + 4.0 * z), Pos2::new(handle.left(), ly)], TEXT, Stroke::NONE));
                let resp = ui.interact(handle.expand(2.0 * z), ui.id().with(("cut-thr", i, k)), Sense::drag()).on_hover_text(format!("threshold {th:.2}: drag up or down"));
                if resp.dragged() {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let nv = ((r.bottom() - pos.y) / (r.height() - 2.0 * z) * top_v).clamp(0.0, top_v);
                        let was = app.cut_ui.drag.filter(|d| d.0 == i && d.1 == k).map_or(v.thr[k].unwrap_or(nv), |d| d.2);
                        let _ = was;
                        app.cut_ui.drag = Some((i, k, nv));
                    }
                }
                if resp.drag_stopped() {
                    if let Some((_, _, nv)) = app.cut_ui.drag.take() {
                        drag_end = Some((k, nv, v.thr[k].unwrap_or(nv)));
                    }
                }
            }
        }
        if let Some((k, nv, was)) = drag_end {
            let key = v.lanes[k].key.clone();
            app.set_cut_threshold(i, &key, nv, Some(was));
        }

        // The cuts across the rows: the picture's time over the picture rows, the sound's over the
        // sound rows.
        let pic_rows: Vec<(f32, f32)> = std::iter::once((g.film, FILM_H)).chain(v.lanes.iter().enumerate().filter(|(_, l)| l.role == Role::Picture).map(|(k, _)| (g.lanes + k as f32 * LANE_H, LANE_H))).collect();
        let snd_rows: Vec<(f32, f32)> = std::iter::once((g.wave, WAVE_H)).chain(v.lanes.iter().enumerate().filter(|(_, l)| l.role != Role::Picture).map(|(k, _)| (g.lanes + k as f32 * LANE_H, LANE_H))).collect();
        for c in &v.cuts {
            let (col, w) = if c.on() { (RED, 2.0) } else { (OFF, 1.0) };
            let stroke = Stroke::new(w * z, col);
            let line = |x: f32, top: f32, h: f32| {
                let a = Pos2::new(x, y(top));
                let b = Pos2::new(x, y(top + h) - 2.0 * z);
                if c.on() {
                    p.line_segment([a, b], stroke);
                } else {
                    p.extend(egui::Shape::dashed_line(&[a, b], stroke, 3.0 * z, 3.0 * z));
                }
            };
            for &(top, h) in &pic_rows {
                line(tx(c.t_pic), top, h);
            }
            for &(top, h) in &snd_rows {
                line(tx(c.t_snd), top, h);
            }
            if !c.confirms.is_empty() {
                p.circle_filled(Pos2::new(tx(c.t_pic), y(g.ruler) - 4.0 * z), 3.0 * z, CONFIRM);
            }
            if c.hand() {
                p.text(Pos2::new(tx(c.t_pic) + 3.0 * z, y(g.film) + 2.0 * z), Align2::LEFT_TOP, "hand", font(10.0), RED);
            }
        }

        // The cut the player is on, and the playhead, across the rows.
        let focus = app.cut_ui.focus.get(&i).copied().filter(|k| *k < v.cuts.len());
        if let Some(c) = focus.map(|k| &v.cuts[k]) {
            let (x, top) = (tx(c.t_pic), y(g.film) - 1.0 * z);
            p.add(egui::Shape::convex_polygon(vec![Pos2::new(x - 4.0 * z, top - 5.0 * z), Pos2::new(x + 4.0 * z, top - 5.0 * z), Pos2::new(x, top)], TEXT, Stroke::NONE));
        }
        let ph_t = if v.clock.is_empty() { None } else { app.playhead_on(&v.clock) };
        if let Some(t) = ph_t.filter(|t| (0.0..=len).contains(t)) {
            p.line_segment([Pos2::new(tx(t), y(g.film)), Pos2::new(tx(t), y(g.ruler + RULER_H))], Stroke::new(1.5 * z, PLAYHEAD));
        }

        // Clicks on the rows: a cut plays; elsewhere the playhead moves there, and a double click
        // cuts there by hand.
        let plot = Rect::from_min_max(Pos2::new(plot_x.0, y(g.film)), Pos2::new(plot_x.1 - 12.0 * z, y(g.ruler)));
        let resp = ui.interact(plot, ui.id().with(("cut-plot", i)), Sense::click());
        let near_cut = |x: f32| v.cuts.iter().enumerate().filter(|(_, c)| (tx(c.t_pic) - x).abs().min((tx(c.t_snd) - x).abs()) <= 5.0 * z).min_by(|a, b| (tx(a.1.t_pic) - x).abs().total_cmp(&(tx(b.1.t_pic) - x).abs())).map(|(k, _)| k);
        if let Some(pos) = resp.hover_pos() {
            let tip = match near_cut(pos.x).map(|k| &v.cuts[k]) {
                Some(c) => format!("cut at {} s, {}: {}\nclick to play it ({CUT_PLAY_S:.0} s either side); switch it in the player below", clock_text(c.t_pic), if c.on() { "on" } else { "off" }, cut_words(&v, c)),
                None => format!("{} s: click to move the playhead here, double-click to cut here by hand", clock_text(xt(pos.x).clamp(0.0, len))),
            };
            resp.clone().on_hover_text_at_pointer(tip);
        }
        if let Some(pos) = resp.interact_pointer_pos() {
            let t = xt(pos.x).clamp(0.0, len);
            if resp.double_clicked() && v.scan.is_some() && near_cut(pos.x).is_none() {
                // On the nearest frame.
                act = Some(Act::Hand((t / v.frame_s).round() * v.frame_s));
            } else if resp.clicked() {
                act = Some(match near_cut(pos.x) {
                    Some(k) => Act::Play(k),
                    None => Act::Seek(t),
                });
            }
        }

        // The ruler, with each piece's number in its middle; a click or drag moves the playhead.
        let ruler = full(g.ruler, RULER_H);
        let rr = ui.interact(ruler, ui.id().with(("cut-ruler", i)), Sense::click_and_drag()).on_hover_text("click or drag to move the playhead");
        if rr.clicked() || rr.dragged() {
            if let Some(pos) = rr.interact_pointer_pos() {
                act = Some(Act::Seek(xt(pos.x).clamp(0.0, len)));
            }
        }
        let step = crate::audio_match_node::tick_step(len);
        let mut q = 0.0;
        while q <= len + 1e-9 {
            let x = tx(q);
            p.line_segment([Pos2::new(x, ruler.top()), Pos2::new(x, ruler.top() + 4.0 * z)], Stroke::new(z, DIM));
            p.text(Pos2::new(x + 2.0 * z, ruler.top() + 3.0 * z), Align2::LEFT_TOP, clock_text(q), font(9.5), DIM);
            q += step;
        }
        p.line_segment([Pos2::new(area.left(), y(g.ruler + RULER_H + 4.0)), Pos2::new(area.right(), y(g.ruler + RULER_H + 4.0))], Stroke::new(z, SEAM));

        // The player: the picture at the playhead, every frame of it (read a few seconds at a
        // time), and the sound through the program's sound output; buttons to go from cut to cut.
        let pic = Rect::from_min_size(Pos2::new(area.left(), y(g.player)), Vec2::new(PIC_W * z, PIC_H * z));
        p.rect_filled(pic, 2.0 * z, Color32::BLACK);
        let playing = app.ph.playing && app.selected == Some(i);
        let has_video = app.graph.feeder(i, "video").is_some();
        let pic_note = match (&v.scan, ph_t) {
            _ if !has_video => Some("no video wired: the sound plays alone".to_string()),
            (None, _) => Some("the picture shows once Run has read the recording".to_string()),
            (Some(_), None) => Some("no playhead yet".to_string()),
            (Some(scan), Some(t)) => {
                let Player { frames, tex } = app.cut_ui.players.entry(i).or_default();
                ask_frames(frames, &scan.path, t, playing, ui.ctx());
                let f = frames.lock().unwrap();
                let pick = f.got.iter().rev().find(|(ft, _)| *ft <= t + 1e-3).or(f.got.first());
                match pick {
                    Some((ft, rgb)) => {
                        let ft = *ft;
                        let img = || ColorImage::from_rgb([cuts::PLAY_W, cuts::PLAY_H], rgb);
                        match tex {
                            Some((shown, h)) if *shown != ft => {
                                h.set(img(), TextureOptions::LINEAR);
                                *shown = ft;
                            }
                            Some(_) => {}
                            None => *tex = Some((ft, ui.ctx().load_texture(format!("cut-play-{i}"), img(), TextureOptions::LINEAR))),
                        }
                        if let Some((_, h)) = tex {
                            p.image(h.id(), pic, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                        }
                        p.text(pic.left_bottom() + Vec2::new(4.0, -3.0) * z, Align2::LEFT_BOTTOM, clock_text(ft), FontId::monospace(10.0 * z), TEXT);
                        // Far from the picture shown: the right one is being read.
                        (f.busy && (t - ft).abs() > 0.5).then(|| "reading…".to_string())
                    }
                    None if f.busy => Some("reading the picture…".to_string()),
                    None => Some(if f.note.is_empty() { "no picture here".to_string() } else { f.note.clone() }),
                }
            }
        };
        if let Some(n) = pic_note {
            p.text(pic.center(), Align2::CENTER_CENTER, n, font(10.5), DIM);
        }
        let ctl = Rect::from_min_max(Pos2::new(pic.right() + 14.0 * z, pic.top()), Pos2::new(area.right(), y(g.player + PLAYER_H)));
        let n_cuts = v.cuts.len();
        let at = ph_t.unwrap_or(0.0);
        let prev = match focus {
            Some(k) => k.checked_sub(1),
            None => v.cuts.iter().rposition(|c| c.t_pic < at - 0.01),
        };
        let next = match focus {
            Some(k) => (k + 1 < n_cuts).then_some(k + 1),
            None => v.cuts.iter().position(|c| c.t_pic > at + 0.01),
        };
        let focused = focus.map(|k| (k, &v.cuts[k]));
        let words = focused.map(|(_, c)| cut_words(&v, c));
        crate::look::area(ui, ctl, |ui| {
            ui.horizontal(|ui| {
                if ui.add_enabled(prev.is_some(), egui::Button::new("◀ cut").small()).on_hover_text("play the cut before").clicked() {
                    act = prev.map(Act::Play);
                }
                let which = match focused {
                    Some((k, _)) => format!("cut {} of {n_cuts}", k + 1),
                    None if n_cuts == 0 => "no cuts".to_string(),
                    None => format!("{n_cuts} cut{}", if n_cuts == 1 { "" } else { "s" }),
                };
                ui.label(egui::RichText::new(which).color(TEXT));
                if ui.add_enabled(next.is_some(), egui::Button::new("cut ▶").small()).on_hover_text("play the cut after").clicked() {
                    act = next.map(Act::Play);
                }
                ui.add_space(12.0);
                if ui.small_button(if playing { "Pause" } else { "Play" }).on_hover_text("play or pause from the playhead (Space)").clicked() {
                    act = Some(Act::PlayPause);
                }
                if let Some((k, c)) = focused {
                    if ui.small_button("Play the cut again").on_hover_text(format!("from {CUT_PLAY_S:.0} s before it to {CUT_PLAY_S:.0} s after")).clicked() {
                        act = Some(Act::Play(k));
                    }
                    let to = if c.on() { "Switch it off" } else { "Switch it on" };
                    if ui.small_button(to).on_hover_text("saved with the project and written to the result file").clicked() {
                        act = Some(Act::Toggle(k));
                    }
                }
            });
            let mono = |t: String, c: Color32| egui::RichText::new(t).monospace().color(c);
            ui.label(mono(format!("playhead {} of {}", clock_text(at), clock_text(len)), DIM));
            match (focused, &words) {
                (Some((_, c)), Some(w)) => {
                    let state = if c.on() { ("on", RED) } else { ("off", OFF) };
                    ui.horizontal(|ui| {
                        ui.label(mono(format!("at {} s ± {:.3}", clock_text(c.t_pic), c.error_s), TEXT));
                        ui.label(egui::RichText::new(state.0).color(state.1).strong());
                    });
                    ui.add(egui::Label::new(egui::RichText::new(w).small().color(DIM)).wrap());
                }
                _ => {
                    ui.add(egui::Label::new(egui::RichText::new(format!("Click a cut on the rows, or ◀ cut and cut ▶, to play it: {CUT_PLAY_S:.0} s before it to {CUT_PLAY_S:.0} s after, picture and sound. Click the rows elsewhere to move the playhead; double-click to cut there by hand.")).small().color(DIM)).wrap());
                }
            }
        });

        // The pieces.
        let cols: [(&str, f32); 6] = [("piece", 0.0), ("picture in the file", 46.0), ("sound in the file", 196.0), ("length", 346.0), ("cut before it", 416.0), ("time out", 0.0)];
        let ty = |row: f32| y(g.table + TABLE_HEAD + row * TABLE_ROW + TABLE_ROW * 0.5);
        let time_x = area.right() - 250.0 * z;
        for (name, x) in cols {
            let x = if name == "time out" { time_x } else { area.left() + x * z };
            p.text(Pos2::new(x, y(g.table + TABLE_HEAD * 0.5)), Align2::LEFT_CENTER, name, font(10.5), DIM);
        }
        let on_cuts: Vec<&Cut> = v.cuts.iter().filter(|c| c.on() && c.t_pic > 0.0 && c.t_pic < len).collect();
        for (r, pc) in v.pieces.iter().enumerate() {
            let row = r as f32;
            let mono = FontId::monospace(10.5 * z);
            p.text(Pos2::new(area.left() + 4.0 * z, ty(row)), Align2::LEFT_CENTER, pc.n.to_string(), mono.clone(), TEXT);
            p.text(Pos2::new(area.left() + 46.0 * z, ty(row)), Align2::LEFT_CENTER, format!("{} – {}", clock_text(pc.v0), clock_text(pc.v1)), mono.clone(), TEXT);
            let same = (pc.a0 - pc.v0).abs() < 1e-6 && (pc.a1 - pc.v1).abs() < 1e-6;
            p.text(Pos2::new(area.left() + 196.0 * z, ty(row)), Align2::LEFT_CENTER, if same { "same".to_string() } else { format!("{} – {}", clock_text(pc.a0), clock_text(pc.a1)) }, mono.clone(), if same { DIM } else { TEXT });
            p.text(Pos2::new(area.left() + 346.0 * z, ty(row)), Align2::LEFT_CENTER, format!("{:.2} s", pc.len()), mono.clone(), TEXT);
            let before = if r == 0 { "start of the file".to_string() } else { on_cuts.get(r - 1).map_or(String::new(), |c| short_cues(&v, c)) };
            let galley = p.layout_no_wrap(before.clone(), font(10.5), DIM);
            let room = time_x - (area.left() + 416.0 * z) - 8.0 * z;
            let shown = if galley.size().x > room { format!("{}…", before.chars().take((room / (6.0 * z)) as usize).collect::<String>()) } else { before.clone() };
            p.text(Pos2::new(area.left() + 416.0 * z, ty(row)), Align2::LEFT_CENTER, shown, font(10.5), DIM);
            let out = cd::piece_port("time", pc.n);
            let t = match app.cut_value(i, out) {
                Ok(Data::Time { utc: Some(u), error_s, .. }) => format!("{} ± {error_s:.2} s", timebase::fmt_utc(u)),
                Ok(_) => String::new(),
                Err(e) => e,
            };
            p.text(Pos2::new(time_x, ty(row)), Align2::LEFT_CENTER, t, font(10.5), DIM);
        }

        // Where the nodes downstream put the pieces: in their true order, gaps hatched.
        let placed = app.placed(i);
        if !placed.is_empty() {
            let top = g.table + TABLE_HEAD + v.pieces.len().max(1) as f32 * TABLE_ROW + 6.0;
            label("placed", top, PLACED_H - 10.0, DIM);
            let r = full(top, PLACED_H - 10.0);
            p.rect_filled(r, 0.0, TRACK);
            let lo = placed.iter().map(|q| q.at).fold(f64::MAX, f64::min);
            let hi = placed.iter().map(|q| q.at + q.len).fold(f64::MIN, f64::max);
            let span = (hi - lo).max(1e-3);
            let px = |t: f64| r.left() + ((t - lo) / span) as f32 * r.width();
            for (k, q) in placed.iter().enumerate() {
                let bar = Rect::from_min_max(Pos2::new(px(q.at), r.top() + 3.0 * z), Pos2::new(px(q.at + q.len).max(px(q.at) + 2.0), r.bottom() - 3.0 * z));
                p.rect_filled(bar, 2.0 * z, alpha(SOUND, 0.55));
                p.text(bar.center(), Align2::CENTER_CENTER, q.n.to_string(), font(11.0), TEXT);
                if let Some(next) = placed.get(k + 1) {
                    let gap = next.at - (q.at + q.len);
                    if gap > 0.1 {
                        let g = Rect::from_min_max(Pos2::new(bar.right(), bar.top()), Pos2::new(px(next.at), bar.bottom()));
                        let mut x = g.left();
                        while x < g.right() {
                            p.line_segment([Pos2::new(x, g.bottom()), Pos2::new((x + g.height()).min(g.right()), g.top())], Stroke::new(z, alpha(DIM, 0.5)));
                            x += 5.0 * z;
                        }
                        if g.width() > 40.0 * z {
                            p.text(g.center(), Align2::CENTER_CENTER, format!("{gap:.1} s"), font(9.5), DIM);
                        }
                    }
                }
            }
            let order: Vec<String> = placed.iter().map(|q| q.n.to_string()).collect();
            let in_order = placed.windows(2).all(|w| w[0].n < w[1].n);
            let words = format!("{} by {}: {}{}", if placed[0].real { "real time" } else { "on the group's clock" }, placed[0].by, order.join(" "), if in_order { " (the file's order)" } else { " (true order: not the file's)" });
            p.text(Pos2::new(r.right(), r.bottom() + 2.0 * z), Align2::RIGHT_TOP, words, font(10.0), if in_order { DIM } else { RED });
        }

        match act {
            Some(Act::Play(k)) => app.play_cut(i, k),
            Some(Act::PlayPause) => app.play_pause_cut(i),
            Some(Act::Toggle(k)) => {
                let c = v.cuts[k].clone();
                app.toggle_cut(i, &c);
            }
            Some(Act::Seek(t)) => app.seek_cut(i, t),
            Some(Act::Hand(t)) => app.hand_cut(i, t),
            None => {}
        }
    }
}

/// How `n` filmstrip pictures go into one texture whose sides are at most `max_side` pixels:
/// columns, rows, and every how many pictures one is kept (1 unless even rows would not fit).
pub fn strip_layout(n: usize, max_side: usize) -> (usize, usize, usize) {
    let per_row = (max_side / cuts::THUMB_W).max(1);
    let fit = per_row * (max_side / cuts::THUMB_H).max(1);
    let step = n.div_ceil(fit).max(1);
    let kept = n.div_ceil(step).max(1);
    let cols = kept.min(per_row);
    (cols, kept.div_ceil(cols), step)
}

/// The cues behind a cut, short: lanes over their thresholds and confirmations.
fn short_cues(v: &CutView, c: &Cut) -> String {
    if c.hand() {
        return "by hand".into();
    }
    let mut parts: Vec<String> = c.cues.iter().filter(|q| q.passes).map(|q| format!("{} {:.1}", v.lanes[q.lane].name, q.value)).collect();
    parts.dedup();
    if !c.confirms.is_empty() {
        parts.push(format!("+ events {}", c.confirms.len()));
    }
    if c.switch == Some(Switch::On) && !c.auto {
        parts.push("on by hand".into());
    }
    parts.join(", ")
}
