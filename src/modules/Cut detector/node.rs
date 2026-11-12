//! The Cut detector node in the workbench. Once the graph runs it reads the wired recording's file
//! itself (`corroborate::cuts`): every frame for the picture lanes, ffprobe's packets for the
//! keyframes and gaps. What it found (`Scan`) stays in the node's state; the cuts are worked out
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

/// What a run reads: the recording's file, whether its picture is wired, and its source's state
/// with the run it was on before this one (the source's decoded audio is this run's once its run
/// differs and is done).
pub struct Job {
    pub path: String,
    pub video: bool,
    pub audio: bool,
    pub source: Option<(Shared, u64)>,
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
            state.lock().unwrap().status = e;
            return;
        }
    };
    {
        let mut s = state.lock().unwrap();
        s.running = true;
        s.status = "waiting for its turn to decode…".into();
    }
    std::thread::spawn(move || {
        let set = |f: &dyn Fn(&mut runtime::NodeState)| {
            let mut s = state.lock().unwrap();
            if s.run_id == run && !stop.load(Ordering::Relaxed) {
                f(&mut s);
            }
        };
        let mut scan = Scan { path: job.path.clone(), ..Default::default() };
        if job.video {
            let turn = corroborate::ingest::decode_turn(&stop);
            if turn.is_none() {
                return;
            }
            set(&|s| s.status = "reading every frame…".into());
            let picture = cuts::scan_picture(&job.path, &stop, &|secs| set(&|s| s.status = format!("reading every frame: {secs:.0} s…")));
            drop(turn);
            match picture {
                Ok(p) => {
                    scan.len = p.times.last().map_or(0.0, |t| t + p.frame_s);
                    scan.frame_s = p.frame_s;
                    scan.lanes.push(cuts::picture_lane(&p));
                    scan.thumbs = p.thumbs;
                }
                Err(e) if e == "stopped" => return,
                Err(e) => scan.notes.push(format!("picture: {e}")),
            }
            match cuts::scan_packets(&job.path) {
                Ok(p) => {
                    if scan.frame_s == 0.0 {
                        scan.frame_s = p.frame_s;
                    }
                    scan.lanes.push(cuts::packets_lane(&p));
                }
                Err(e) => scan.notes.push(format!("keyframes: {e}")),
            }
        }
        if scan.frame_s == 0.0 {
            scan.frame_s = 1.0 / 30.0;
        }
        let summary = if scan.notes.is_empty() { format!("{} lanes read", scan.lanes.len()) } else { scan.notes.join("; ") };
        set(&|s| {
            s.status = summary.clone();
            s.running = false;
            s.done = true;
        });
        let scan = Arc::new(scan);
        set(&|s| s.cut = Some(scan.clone()));
    });
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

/// What the canvas keeps between frames: the view (once a frame), the filmstrips and the
/// threshold being dragged.
#[derive(Default)]
pub struct CutUi {
    pub cache: RefCell<HashMap<NodeId, Rc<CutView>>>,
    strips: HashMap<NodeId, (u64, TextureHandle)>,
    drag: Option<(NodeId, usize, f32)>,
}

/// One frame's text: 83.25 s as 1:23.25.
pub fn clock_text(t: f64) -> String {
    let m = (t / 60.0).floor();
    let s = t - m * 60.0;
    if m > 0.0 { format!("{m:.0}:{s:05.2}") } else { format!("{s:.2}") }
}

/// The lanes a run will make, before it has: so the node keeps its size.
fn planned_lanes(video: bool) -> Vec<Lane> {
    let empty = |key: &str, name: &str, role: Role, thr: f32| Lane { key: key.into(), name: name.into(), role, unit: String::new(), curve: Vec::new(), peaks: Vec::new(), thr: Some(thr), note: "not read yet: the graph reads it when it runs".into() };
    let mut v = Vec::new();
    if video {
        v.push(empty("picture", "picture", Role::Picture, cuts::PICTURE_THR));
        v.push(empty("keyframes", "keyframes and gaps", Role::Picture, 0.5));
    }
    v
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
    pub(crate) fn cut_job(&self, i: NodeId, before: &HashMap<NodeId, u64>) -> Result<Job, String> {
        let src = self.cut_source(i)?;
        let path = match &self.nodes[src].kind {
            Kind::Sourcer { input } => match sourcer_node::input(input) {
                Some(runtime::Input::File(p)) => p,
                _ => return Err("files only for now: a stream or microphone is not read".into()),
            },
            _ => return Err("wire a SOURCER's audio or video into it".into()),
        };
        let video = self.graph.feeder(i, "video").is_some();
        let audio = self.graph.feeder(i, "audio").is_some();
        let source = before.get(&src).map(|&r| (self.nodes[src].state.clone(), r));
        Ok(Job { path, video, audio, source })
    }

    /// What run signature the cut detectors add: what is wired into each.
    pub(crate) fn cut_signature(&self) -> String {
        let mut s = String::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if n.hidden || !matches!(n.kind, Kind::CutDetector { .. }) {
                continue;
            }
            for p in &self.graph.nodes()[i].spec.inputs {
                if matches!(p.kind, PortKind::Audio | PortKind::Video) {
                    if let Some((f, o)) = self.graph.feeder(i, p.name) {
                        s.push_str(&format!("cut{i}:{}<{f}.{o};", p.name));
                    }
                }
            }
        }
        s
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
        v.lanes = match &v.scan {
            Some(s) => s.lanes.iter().map(|l| Lane { curve: Vec::new(), ..l.clone() }).collect(),
            None => planned_lanes(video),
        };
        v.frame_s = v.scan.as_ref().map_or(1.0 / 30.0, |s| s.frame_s);
        // One lane per wired events input, named by what feeds it.
        for p in self.graph.nodes()[i].spec.inputs.iter().filter(|p| p.name.starts_with("events ")) {
            let Some((f, o)) = self.graph.feeder(i, p.name) else { continue };
            let mut lane = Lane { key: p.name.to_string(), name: format!("{} › {o}", self.node_name(f)), role: Role::Events, unit: String::new(), curve: Vec::new(), peaks: Vec::new(), thr: None, note: String::new() };
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
    format!(",\"clock\":\"{}\",\"length_s\":{},\"lanes\":[{}],\"cuts\":[{}],\"pieces\":[{}]", json_escape(&v.clock), num(v.len), lanes.join(","), cuts_j.join(","), pieces.join(","))
}

// ---------------------------------------------------------------- on the canvas

const NODE_W: f32 = 1000.0;
const NOTE_H: f32 = 20.0;
const FILM_H: f32 = 34.0;
const WAVE_H: f32 = 30.0;
const LANE_H: f32 = 30.0;
const RULER_H: f32 = 16.0;
const LABEL_W: f32 = 132.0;
const TABLE_HEAD: f32 = 18.0;
const TABLE_ROW: f32 = 17.0;
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
    table: f32,
}

fn geom() -> Geom {
    let film = NOTE_H;
    let wave = film + FILM_H;
    let lanes = wave + WAVE_H;
    Geom { film, wave, lanes, ruler: 0.0, table: 0.0 }
}

fn layout(n_lanes: usize) -> Geom {
    let mut g = geom();
    g.ruler = g.lanes + n_lanes as f32 * LANE_H;
    g.table = g.ruler + RULER_H + 8.0;
    g
}

pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, standard: Vec2) -> Vec2 {
        let v = app.cut_view(i);
        let g = layout(v.lanes.len());
        let rows = v.pieces.len().max(1);
        // Tall enough for its own parts and for a port row per output.
        Vec2::new(NODE_W, (HEAD + 4.0 + g.table + TABLE_HEAD + rows as f32 * TABLE_ROW + 30.0).max(standard.y))
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

        // The line on top: what it is doing, or what is missing.
        let status = app.nodes[i].state.lock().unwrap().status.clone();
        let mut note = v.problem.clone().unwrap_or(status);
        if v.off_clock > 0 {
            note.push_str(&format!("; {} events on another clock left out", v.off_clock));
        }
        let on = v.cuts.iter().filter(|c| c.on()).count();
        let head = if v.scan.is_some() { format!("{} cuts on of {} · {} pieces · {}", on, v.cuts.len(), v.pieces.len(), note) } else { note };
        p.text(Pos2::new(area.left(), y(NOTE_H * 0.5)), Align2::LEFT_CENTER, head, font(11.0), DIM);

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
            let fresh = app.cut_ui.strips.get(&i).is_none_or(|(r, _)| *r != run);
            if fresh {
                let n = scan.thumbs.len();
                let mut rgb = Vec::with_capacity(n * cuts::THUMB_W * cuts::THUMB_H * 3);
                for row in 0..cuts::THUMB_H {
                    for t in &scan.thumbs {
                        rgb.extend_from_slice(&t.rgb[row * cuts::THUMB_W * 3..(row + 1) * cuts::THUMB_W * 3]);
                    }
                }
                let img = ColorImage::from_rgb([n * cuts::THUMB_W, cuts::THUMB_H], &rgb);
                let tex = ui.ctx().load_texture(format!("cut-strip-{i}"), img, TextureOptions::LINEAR);
                app.cut_ui.strips.insert(i, (run, tex));
            }
            if let Some((_, tex)) = app.cut_ui.strips.get(&i) {
                let n = scan.thumbs.len();
                let tile = film.height() * cuts::THUMB_W as f32 / cuts::THUMB_H as f32;
                let tiles = ((film.width() / tile).ceil() as usize).max(1);
                for k in 0..tiles {
                    let x0 = film.left() + k as f32 * tile;
                    let t = xt((x0 + tile * 0.5).min(film.right()));
                    let near = scan.thumbs.partition_point(|th| th.t < t).min(n - 1);
                    let r = Rect::from_min_max(Pos2::new(x0, film.top()), Pos2::new((x0 + tile).min(film.right()), film.bottom()));
                    let u0 = near as f32 / n as f32;
                    let u1 = u0 + (r.width() / tile) / n as f32;
                    p.image(tex.id(), r, Rect::from_min_max(Pos2::new(u0, 0.0), Pos2::new(u1, 1.0)), Color32::WHITE);
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

        // Clicks on the plot: a cut switches; elsewhere a cut by hand.
        let plot = Rect::from_min_max(Pos2::new(plot_x.0, y(g.film)), Pos2::new(plot_x.1 - 12.0 * z, y(g.ruler)));
        let resp = ui.interact(plot, ui.id().with(("cut-plot", i)), Sense::click());
        let near_cut = |x: f32| v.cuts.iter().filter(|c| (tx(c.t_pic) - x).abs().min((tx(c.t_snd) - x).abs()) <= 5.0 * z).min_by(|a, b| (tx(a.t_pic) - x).abs().total_cmp(&(tx(b.t_pic) - x).abs()));
        if let Some(pos) = resp.hover_pos() {
            let tip = match near_cut(pos.x) {
                Some(c) => format!("cut at {} s, {}: {}\nclick to switch it {}", clock_text(c.t_pic), if c.on() { "on" } else { "off" }, cut_words(&v, c), if c.on() { "off" } else { "on" }),
                None => format!("{} s: click to cut here by hand", clock_text(xt(pos.x).clamp(0.0, len))),
            };
            resp.clone().on_hover_text_at_pointer(tip);
        }
        if resp.clicked() && v.scan.is_some() {
            if let Some(pos) = resp.interact_pointer_pos() {
                match near_cut(pos.x).cloned() {
                    Some(c) => app.toggle_cut(i, &c),
                    None => {
                        // On the nearest frame.
                        let t = (xt(pos.x).clamp(0.0, len) / v.frame_s).round() * v.frame_s;
                        app.hand_cut(i, t);
                    }
                }
            }
        }

        // The ruler, with each piece's number in its middle.
        let ruler = full(g.ruler, RULER_H);
        let step = crate::audio_match_node::tick_step(len);
        let mut q = 0.0;
        while q <= len + 1e-9 {
            let x = tx(q);
            p.line_segment([Pos2::new(x, ruler.top()), Pos2::new(x, ruler.top() + 4.0 * z)], Stroke::new(z, DIM));
            p.text(Pos2::new(x + 2.0 * z, ruler.top() + 3.0 * z), Align2::LEFT_TOP, clock_text(q), font(9.5), DIM);
            q += step;
        }
        p.line_segment([Pos2::new(area.left(), y(g.ruler + RULER_H + 4.0)), Pos2::new(area.right(), y(g.ruler + RULER_H + 4.0))], Stroke::new(z, SEAM));

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
    }
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
