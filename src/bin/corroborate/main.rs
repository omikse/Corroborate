//! Corroborate: the workbench. Sources, detectors and plugins are nodes in a graph; wires
//! decide what feeds what. the graph runs always (a change restarts it); each node's window shows its raw data;
//! the timeline shows every finding on one clock with its time error; the Record node keeps
//! a result JSON (findings, facts and the graph as the evidence trail) up to date on disk.
//!
//! Graph: scroll to zoom, drag a node by its header, drag empty space to pan, drag from an output port to an
//! input port to wire them (types must match), right-click an input to unwire it, right-click
//! a node or empty space for more (rename, delete). Nodes have no windows: each shows its data and
//! controls on the canvas.
//!
//! This file draws the graph, the timeline and the windows' frames. What each node shows and does
//! is in its module folder (`src/modules/<node>/node.rs`); the matches on `Kind` here only call it.

// Release builds are a Windows GUI program: no console window. Debug builds keep the console
// for diagnostics.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(test)]
mod drive;
#[cfg(test)]
mod project_tests;
#[cfg(test)]
mod bus_tests;
#[cfg(test)]
mod look_tests;
mod bus;
mod layout;
mod look;
mod nodes;
mod runtime;
mod script;

// Each node's workbench code lives in its own module folder (src/modules/<node>/node.rs).
#[path = "../../modules/Combine locations/node.rs"]
mod combine_locations_node;
#[path = "../../modules/ENF (power hum)/node.rs"]
mod enf_node;
#[path = "../../modules/Grid frequencies (50 or 60 Hz)/node.rs"]
mod grid_frequencies_node;
#[path = "../../modules/Invert location/node.rs"]
mod invert_location_node;
#[path = "../../modules/Manual data/node.rs"]
mod manual_data_node;
#[path = "../../modules/Map/node.rs"]
mod map_node;
#[path = "../../modules/Planes (Doppler)/node.rs"]
mod planes_node;
#[path = "../../modules/Plugin/node.rs"]
mod plugin_node;
#[path = "../../modules/Result record/node.rs"]
mod result_record_node;
#[path = "../../modules/SOURCER/node.rs"]
mod sourcer_node;

use corroborate::core::{PortKind, Role, SourceKind};
use corroborate::graph::{Graph, GraphError, NodeId};
use corroborate::timebase::{self, json_escape, Anchor};
use eframe::egui::{
    self, epaint::CubicBezierShape, Align2, Color32, ColorImage, FontId, Pos2, Rect, Sense, Stroke, StrokeKind,
    TextureHandle, TextureOptions, Vec2,
};
use nodes::Kind;
use corroborate::area::{robinson, Area, GridMap, WorldMask, COLS, ROBINSON_ASPECT, ROWS};
use runtime::{Input, Job, NodeState, Shared};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- look

const BG: Color32 = Color32::from_rgb(23, 25, 28);
const PANEL: Color32 = Color32::from_rgb(31, 34, 38);
const NODE: Color32 = Color32::from_rgb(42, 46, 51);
const LINE: Color32 = Color32::from_rgb(58, 63, 70);
const TEXT: Color32 = Color32::from_rgb(216, 220, 226);
const DIM: Color32 = Color32::from_rgb(138, 146, 156);
const WARN: Color32 = Color32::from_rgb(224, 164, 88);
const SEL: Color32 = Color32::from_rgb(95, 179, 161);
const GOOD: Color32 = Color32::from_rgb(140, 220, 110);
const LIVE: Color32 = Color32::from_rgb(255, 80, 80);

fn kind_color(k: PortKind) -> Color32 {
    match k {
        PortKind::Audio => Color32::from_rgb(224, 164, 88),
        PortKind::Video => Color32::from_rgb(224, 200, 90),
        PortKind::Time => Color32::from_rgb(122, 167, 224),
        PortKind::Location => Color32::from_rgb(199, 133, 201),
        PortKind::Track => Color32::from_rgb(143, 209, 107),
        PortKind::Event => Color32::from_rgb(224, 122, 106),
        _ => Color32::from_rgb(154, 163, 173),
    }
}

fn role_text(r: Role) -> &'static str {
    match r {
        Role::Source(SourceKind::Live) => "source · live",
        Role::Source(SourceKind::File) => "source · file",
        Role::Source(SourceKind::Reference) => "source · manual",
        Role::Detector => "detector",
        Role::Transform => "transform",
        Role::Solver => "solver",
        Role::Record => "record",
        Role::View => "view",
    }
}

fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn grade_color(g: &str) -> Color32 {
    match g {
        "high" => GOOD,
        "medium" => WARN,
        _ => DIM,
    }
}

/// The error of an anchor's clock, seconds, judged from where it came from. A labelled
/// estimate, not a measurement: shown with the time on the timeline and in the result.
fn anchor_error_s(a: &Anchor) -> f64 {
    let s = a.source.to_ascii_lowercase();
    if s.contains("program-date-time") || s.contains("hls") || s.contains("sidecar") || s.contains("recording") {
        1.0
    } else if s.contains("received") || s.contains("pc clock") || s.contains("clock") {
        10.0
    } else {
        600.0
    }
}

fn colormap() -> [[u8; 4]; 256] {
    const STOPS: [(f32, [f32; 3]); 6] = [
        (0.00, [4.0, 5.0, 8.0]),
        (0.22, [18.0, 22.0, 60.0]),
        (0.45, [90.0, 30.0, 120.0]),
        (0.65, [200.0, 60.0, 70.0]),
        (0.85, [250.0, 160.0, 40.0]),
        (1.00, [255.0, 250.0, 200.0]),
    ];
    let mut lut = [[0u8; 4]; 256];
    for (q, px) in lut.iter_mut().enumerate() {
        let x = q as f32 / 255.0;
        let mut i = 0;
        while i + 1 < STOPS.len() - 1 && x > STOPS[i + 1].0 {
            i += 1;
        }
        let (a, ca) = STOPS[i];
        let (b, cb) = STOPS[i + 1];
        let u = ((x - a) / (b - a)).clamp(0.0, 1.0);
        *px = [
            (ca[0] + u * (cb[0] - ca[0])) as u8,
            (ca[1] + u * (cb[1] - ca[1])) as u8,
            (ca[2] + u * (cb[2] - ca[2])) as u8,
            255,
        ];
    }
    lut
}

fn fmt_clock(utc: f64) -> String {
    timebase::fmt_utc_clock(utc)
}

/// A finding on the timeline: its span on the shared clock, the clock's error, and how it looks.
struct Mark {
    t0: f64,
    t1: f64,
    err: f64,
    label: String,
    detail: String,
    col: Color32,
}

/// A node's clock: source seconds to UTC (when anchored), and that clock's error in seconds.
fn clock(a: &Option<Anchor>) -> (impl Fn(f64) -> f64 + '_, f64) {
    let err = a.as_ref().map_or(0.0, anchor_error_s);
    (move |t: f64| a.as_ref().map_or(t, |a| a.utc(t)), err)
}

// ---------------------------------------------------------------- project

struct NodeData {
    kind: Kind,
    label: String,
    pos: Pos2,
    state: Shared,
    /// Deleted from the workspace (ids stay stable; a deleted node has no wires).
    hidden: bool,
    /// WAV files this source's audio is being recorded into (by Result record nodes).
    taps: result_record_node::Taps,
    /// Folded to its title and port names (right-click → Collapse); it keeps running and wired.
    collapsed: bool,
}

fn project_file() -> Option<PathBuf> {
    // Tests drive the workbench headless (see drive.rs) and never read or write a project.
    if cfg!(test) {
        return None;
    }
    Some(corroborate::home().join("settings").join("project.txt"))
}

fn results_dir() -> PathBuf {
    corroborate::home().join("results")
}

fn clean(s: &str) -> String {
    s.replace(['|', '\n', '\r'], " ")
}

// ---------------------------------------------------------------- app

struct App {
    graph: Graph,
    nodes: Vec<NodeData>,
    pan: Vec2,
    /// Canvas magnification, kept separate from the graph's saved node positions.
    zoom: f32,
    selected: Option<NodeId>,
    /// Wire being dragged from (node, output index).
    wiring: Option<(NodeId, usize)>,
    msg: String,
    stop: Option<Arc<AtomicBool>>,
    /// Recordings in progress, by record node, and each record's last note.
    recordings: HashMap<NodeId, result_record_node::Recording>,
    record_notes: HashMap<NodeId, String>,
    /// What the running graph was started from; a change (held for a moment) restarts it.
    run_sig: Option<String>,
    sig_seen: (String, Instant),
    textures: HashMap<NodeId, (u64, f64, TextureHandle)>,
    lut: [[u8; 4]; 256],
    /// Timeline view (UTC seconds when anchored, else source seconds), and whether it follows the end.
    tl: (f64, f64),
    follow: bool,
    mics: Vec<String>,
    /// The Earth image under every map (loaded at the first frame).
    earth: Option<TextureHandle>,
    /// Areas worked out this frame, by node (see `area_from`).
    area_cache: std::cell::RefCell<HashMap<NodeId, Result<(Arc<Area>, String), String>>>,
    last_project_save: Instant,
    /// Where the canvas starts on screen (last frame); tests use it to find nodes.
    canvas_origin: Pos2,
    /// Programs on the data bus, by node, while the graph runs; when inputs were last sent.
    programs: HashMap<NodeId, bus::Program>,
    bus_sent: Instant,
    /// Settings changed on the canvas (for the result file), text being typed into setting
    /// boxes, and why a typed value was refused.
    changes: Vec<bus::SettingChange>,
    drafts: HashMap<(NodeId, String), String>,
    setting_errors: HashMap<(NodeId, String), String>,
    /// How many times the graph has (re)started.
    runs: u64,
    /// Module manifests by id (looks, ports), read again each second; and when.
    looks: HashMap<&'static str, Arc<corroborate::module_manifest::Manifest>>,
    looks_at: Instant,
    /// look.rhai scripts, compiled, by module id; and what each node's script last gave.
    scripts: script::Scripts,
}

const NODE_W: f32 = 210.0;
const HEAD: f32 = 26.0;
const ROW: f32 = 20.0;

impl App {
    /// An empty workbench: no nodes, nothing loaded.
    fn empty() -> Self {
        Self {
            graph: Graph::new(),
            nodes: Vec::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            selected: None,
            wiring: None,
            msg: String::new(),
            stop: None,
            recordings: HashMap::new(),
            record_notes: HashMap::new(),
            run_sig: None,
            sig_seen: (String::new(), Instant::now()),
            textures: HashMap::new(),
            lut: colormap(),
            tl: (0.0, 600.0),
            follow: true,
            mics: Vec::new(),
            earth: None,
            area_cache: Default::default(),
            last_project_save: Instant::now(),
            canvas_origin: Pos2::ZERO,
            programs: HashMap::new(),
            bus_sent: Instant::now(),
            changes: Vec::new(),
            drafts: HashMap::new(),
            setting_errors: HashMap::new(),
            runs: 0,
            looks: HashMap::new(),
            looks_at: Instant::now(),
            scripts: Default::default(),
        }
    }

    fn new(start: Option<String>, autorun: bool) -> Self {
        let mut app = Self::empty();
        if std::env::args().any(|a| a == "--new") {
            // --new: an empty canvas (the old project file is kept as project.bak.txt).
            let dir = corroborate::home().join("settings");
            let _ = std::fs::copy(dir.join("project.txt"), dir.join("project.bak.txt"));
        } else {
            app.load_project();
        }
        if let Some(src) = start {
            // A path or URL on the command line goes into the first source node.
            let kind = Kind::Sourcer { input: src };
            match app.nodes.iter().position(|n| !n.hidden && n.kind.is_source()) {
                Some(i) => app.nodes[i].kind = kind,
                None => {
                    app.add(kind, Pos2::new(30.0, 60.0));
                }
            }
        }
        if autorun {
            app.run();
        }
        app
    }

    fn add(&mut self, kind: Kind, pos: Pos2) -> NodeId {
        let id = self.graph.add(kind.default_label(), kind.spec());
        self.nodes.push(NodeData { label: kind.default_label(), kind, pos, state: Default::default(), hidden: false, taps: Default::default(), collapsed: false });
        id
    }

    /// A wire made in code (the Result record, tests). A port the node no longer has is skipped
    /// quietly.
    fn wire(&mut self, a: NodeId, out: &str, b: NodeId, inp: &str) {
        match self.graph.connect(a, out, b, inp) {
            Ok(()) | Err(GraphError::NoOutput { .. } | GraphError::NoInput { .. }) => {}
            Err(e) => self.msg = describe(&e),
        }
    }

    // ---------------- project file

    fn save_project(&mut self) {
        if let Some(path) = project_file() {
            self.save_project_to(&path);
        }
        self.last_project_save = Instant::now();
    }

    /// Writes the project to `path`: first beside it as `<name>.tmp`, then renamed over it, so a
    /// crash while saving leaves the previous project whole, never half a file.
    fn save_project_to(&self, path: &std::path::Path) -> bool {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.project_text()).is_ok() && std::fs::rename(&tmp, path).is_ok()
    }

    /// The project file's text: visible nodes (renumbered), Manual rows and wires, one per line.
    fn project_text(&self) -> String {
        let mut text = String::from("# corroborate project: node|kind|x|y|label|param[|collapsed], row|node|key|value, wire|from|out|to|in\n");
        // Only visible nodes, renumbered.
        let keep: Vec<NodeId> = (0..self.nodes.len()).filter(|&i| !self.nodes[i].hidden).collect();
        let new_id = |old: NodeId| keep.iter().position(|&k| k == old);
        for &i in &keep {
            let n = &self.nodes[i];
            let (k, param) = (n.kind.spec().name, n.kind.saved_param());
            let folded = if n.collapsed { "|collapsed" } else { "" };
            text.push_str(&format!("node|{k}|{:.0}|{:.0}|{}|{}{folded}\n", n.pos.x, n.pos.y, clean(&n.label), clean(&param)));
        }
        for (ni, &i) in keep.iter().enumerate() {
            for (k, v) in self.nodes[i].kind.rows() {
                text.push_str(&format!("row|{ni}|{}|{}\n", clean(k), clean(v)));
            }
        }
        for w in self.graph.wires() {
            if let (Some(a), Some(b)) = (new_id(w.from), new_id(w.to)) {
                let out = self.graph.nodes()[w.from].spec.outputs[w.from_port].name;
                let inp = self.graph.nodes()[w.to].spec.inputs[w.to_port].name;
                text.push_str(&format!("wire|{a}|{out}|{b}|{inp}\n"));
            }
        }
        text
    }

    fn load_project(&mut self) -> bool {
        let Some(text) = project_file().and_then(|p| std::fs::read_to_string(p).ok()) else { return false };
        self.load_project_text(&text)
    }

    /// Adds the nodes, rows and wires of a project file's text. Lines it does not understand
    /// are skipped. False when it held no nodes.
    fn load_project_text(&mut self, text: &str) -> bool {
        let mut rows: Vec<(usize, String, String)> = Vec::new();
        let mut wires: Vec<(usize, String, usize, String)> = Vec::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('|').collect();
            match f.as_slice() {
                ["node", k, x, y, label, param, rest @ ..] if rest.len() <= 1 => {
                    let kind = Kind::from_saved(k, param.to_string());
                    let id = self.add(kind, Pos2::new(x.parse().unwrap_or(40.0), y.parse().unwrap_or(40.0)));
                    if !label.trim().is_empty() {
                        self.nodes[id].label = label.to_string();
                    }
                    self.nodes[id].collapsed = rest.first().is_some_and(|f| f.trim() == "collapsed");
                }
                ["row", n, k, v] => rows.push((n.parse().unwrap_or(usize::MAX), k.to_string(), v.to_string())),
                ["wire", a, o, b, i] => wires.push((a.parse().unwrap_or(usize::MAX), o.to_string(), b.parse().unwrap_or(usize::MAX), i.to_string())),
                _ => {}
            }
        }
        if self.nodes.is_empty() {
            return false;
        }
        for (n, k, v) in rows {
            if let Some(node) = self.nodes.get_mut(n) {
                node.kind.add_row(k, v);
            }
        }
        // A node whose module this tool lacks keeps the ports its saved wires use.
        for s in 0..self.nodes.len() {
            if !matches!(self.nodes[s].kind, Kind::Missing { .. }) {
                continue;
            }
            let ins: Vec<&str> = wires.iter().filter(|w| w.2 == s).map(|w| w.3.as_str()).collect();
            let outs: Vec<(&str, PortKind)> = wires
                .iter()
                .filter(|w| w.0 == s && w.2 < self.nodes.len())
                .map(|w| {
                    let k = self.graph.nodes()[w.2].spec.inputs.iter().find(|p| p.name == w.3).map_or(PortKind::Any, |p| p.kind);
                    (w.1.as_str(), k)
                })
                .collect();
            let (ins, outs) = nodes::missing_ports(&ins, &outs);
            self.graph.set_inputs(s, ins);
            self.graph.set_outputs(s, outs);
        }
        for (a, o, b, i) in wires {
            if a < self.nodes.len() && b < self.nodes.len() {
                if matches!(self.nodes[b].kind, Kind::ResultRecord { .. }) {
                    // Old projects named the record's inputs by kind: any name takes the free input.
                    self.wire_record(a, &o, b);
                } else {
                    let _ = self.graph.connect(a, &o, b, &i);
                }
            }
        }
        self.sync_record_ports();
        // Graph labels follow the node labels.
        true
    }

    // ---------------- run

    fn running(&self) -> bool {
        self.nodes.iter().any(|n| n.state.lock().unwrap().running)
    }

    /// The area a node gives on its location output, and how it was reached. Worked out once per
    /// frame per node (maps, statuses and the record all ask); `graph_view` clears the cache.
    fn area_from(&self, from: NodeId) -> Result<(Arc<Area>, String), String> {
        if let Some(r) = self.area_cache.borrow().get(&from) {
            return r.clone();
        }
        let r = self.area_uncached(from);
        self.area_cache.borrow_mut().insert(from, r.clone());
        r
    }

    fn area_uncached(&self, from: NodeId) -> Result<(Arc<Area>, String), String> {
        let n = &self.nodes[from];
        match &n.kind {
            Kind::Enf { .. } => enf_node::area_out(&n.state),
            Kind::ManualData { rows } => manual_data_node::area_out(rows),
            Kind::ResultRecord { .. } => self.record_area_out(from),
            Kind::CombineLocations { any } => self.combine_area_out(from, *any),
            Kind::InvertLocation => self.invert_area_out(from),
            Kind::Module { .. } | Kind::Plugin { .. } => self.outbox_area(from),
            _ => Err("this node gives no area".into()),
        }
    }

    fn run(&mut self) {
        self.stop_all();
        self.save_project();
        self.runs += 1;
        self.start_programs();
        let dir = results_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.msg = format!("cannot create {}: {e}", dir.display());
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let mut started = 0;
        for i in 0..self.nodes.len() {
            let n = &self.nodes[i];
            if n.hidden || !n.kind.is_source() {
                continue;
            }
            let input = match &n.kind {
                Kind::Sourcer { input } => sourcer_node::input(input),
                _ => None,
            };
            let Some(input) = input else {
                n.state.lock().unwrap().status = "nothing to open: set it in the node".into();
                continue;
            };
            let mut job = Job {
                label: n.label.clone(),
                input,
                source: n.state.clone(),
                planes: Vec::new(),
                enfs: Vec::new(),
                plugins: Vec::new(),
                stop: stop.clone(),
                work_dir: dir.clone(),
                taps: n.taps.clone(),
                buses: self.audio_buses(i),
            };
            for (t, _) in self.graph.targets(i, "audio") {
                let tn = &self.nodes[t];
                if tn.hidden {
                    continue;
                }
                match &tn.kind {
                    Kind::Planes => job.planes.push(tn.state.clone()),
                    Kind::Enf { .. } => job.enfs.push(self.enf_job(t)),
                    Kind::Plugin { path } => job.plugins.push(self.plugin_job(t, path)),
                    _ => {}
                }
            }
            runtime::start(job);
            started += 1;
        }
        self.run_sig = Some(self.run_signature());
        if started == 0 {
            return;
        }
        self.stop = Some(stop);
        self.follow = true;
        self.msg = format!("running {started} source(s)");
    }

    /// What a run is made of: each source's input and what its audio feeds. The graph runs
    /// always; when this changes (and stays changed for a moment, while typing) it restarts.
    fn run_signature(&self) -> String {
        let mut s = String::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if n.hidden {
                continue;
            }
            if let Kind::Sourcer { input } = &n.kind {
                s.push_str(&format!("{i}:{input}|"));
                for (t, _) in self.graph.targets(i, "audio") {
                    // Wiring audio into a record taps it while recording: no restart.
                    if matches!(self.nodes[t].kind, Kind::ResultRecord { .. }) {
                        continue;
                    }
                    s.push_str(&format!("{t}:{}:{};", self.nodes[t].kind.spec().name, self.nodes[t].kind.saved_param()));
                }
            }
            if let Kind::Module { settings, .. } = &n.kind {
                // A program gets its settings when it starts.
                s.push_str(&format!("{i}:{settings:?}|"));
            }
            if let Kind::ManualData { rows } = &n.kind {
                // Plugins read the place when they start.
                if self.graph.targets(i, "place").iter().any(|(t, _)| matches!(self.nodes[*t].kind, Kind::Plugin { .. })) {
                    s.push_str(&format!("{i}:{rows:?}|"));
                }
            }
        }
        s
    }

    /// Start or restart the graph when what it runs from has changed.
    fn keep_running(&mut self) {
        let sig = self.run_signature();
        if self.run_sig.as_ref() == Some(&sig) {
            return;
        }
        if self.sig_seen.0 != sig {
            self.sig_seen = (sig, Instant::now());
            if self.run_sig.is_some() {
                return;
            }
        }
        if self.run_sig.is_none() || self.sig_seen.1.elapsed() > Duration::from_millis(500) {
            self.run();
        }
    }

    fn stop_all(&mut self) {
        if let Some(s) = self.stop.take() {
            s.store(true, Ordering::Relaxed);
        }
        self.programs.clear();
    }

    // ---------------- result record

    /// The result JSON of nodes `ids`: each with its results, and the wires between them.
    fn result_json(&self, ids: &[NodeId]) -> String {
        let mut nodes_j = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if n.hidden || !ids.contains(&i) {
                continue;
            }
            let s = n.state.lock().unwrap();
            let anchor = s.anchor.clone();
            let anchor_j = match &anchor {
                Some(a) => format!(
                    "{{\"utc0\":\"{}\",\"source\":\"{}\",\"error_s\":{}}}",
                    timebase::fmt_utc(a.utc0),
                    json_escape(&a.source),
                    anchor_error_s(a)
                ),
                None => "null".into(),
            };
            let mut body = format!(
                "\"id\":{i},\"label\":\"{}\",\"kind\":\"{}\",\"source\":\"{}\",\"anchor\":{anchor_j},\"status\":\"{}\"",
                json_escape(&n.label),
                n.kind.spec().name,
                json_escape(&s.source_key),
                json_escape(&s.status)
            );
            let own = match &n.kind {
                Kind::Planes => planes_node::json(&s),
                Kind::Enf { .. } => enf_node::json(&s),
                Kind::GridFrequencies { path } => grid_frequencies_node::json(path),
                Kind::Plugin { path } => plugin_node::json(path, &s),
                Kind::ManualData { rows } => manual_data_node::json(rows),
                Kind::Sourcer { .. } => sourcer_node::json(&s),
                Kind::Map | Kind::InvertLocation | Kind::CombineLocations { .. } => String::new(),
                Kind::Missing { name, .. } => format!(",\"module_missing\":\"{}\"", json_escape(name)),
                Kind::Module { .. } => {
                    drop(s);
                    self.module_json(i)
                }
                Kind::ResultRecord { .. } => {
                    drop(s);
                    self.record_json(i)
                }
            };
            body.push_str(&own);
            nodes_j.push(format!("{{{body}}}"));
        }
        let wires: Vec<String> = self
            .graph
            .wires()
            .iter()
            .filter(|w| ids.contains(&w.from) && ids.contains(&w.to))
            .map(|w| {
                format!(
                    "{{\"from\":{},\"out\":\"{}\",\"to\":{},\"in\":\"{}\"}}",
                    w.from,
                    self.graph.nodes()[w.from].spec.outputs[w.from_port].name,
                    w.to,
                    self.graph.nodes()[w.to].spec.inputs[w.to_port].name
                )
            })
            .collect();
        format!(
            "{{\"app\":\"corroborate\",\"written_utc\":\"{}\",\"note\":\"every node with its results; wires and setting changes are the evidence trail; anchor error_s is an estimate from the anchor's source\",\"nodes\":[\n{}\n],\"wires\":[{}],\"setting_changes\":{}}}\n",
            timebase::fmt_utc(timebase::now_unix()),
            nodes_j.join(",\n"),
            wires.join(","),
            self.changes_json(ids)
        )
    }

    // ---------------- graph

    /// Port rows of a node: the longer of its input and output lists.
    fn port_rows(&self, i: NodeId) -> f32 {
        let spec = &self.graph.nodes()[i].spec;
        spec.inputs.len().max(spec.outputs.len()).max(1) as f32
    }

    fn node_rect(&self, origin: Pos2, i: NodeId) -> Rect {
        let size = if self.nodes[i].collapsed {
            // Title and one row per port, nothing else.
            Vec2::new(NODE_W, HEAD + self.port_rows(i) * ROW + 6.0)
        } else if let Some((w, h)) = self.frame_look(i).size {
            // module.toml's [look] size, or what its look.rhai says.
            Vec2::new(w, h)
        } else {
            let standard = Vec2::new(NODE_W, HEAD + self.port_rows(i) * ROW + 26.0);
            look::of(&self.nodes[i].kind).size(self, i, standard)
        };
        Rect::from_min_size(origin + self.pan + self.nodes[i].pos.to_vec2() * self.zoom, size * self.zoom)
    }

    /// The part of a node a `NodeLook` draws in: below the title, above the status line. The port
    /// dots and names are drawn over it.
    fn body_rect(&self, rect: Rect) -> Rect {
        let z = self.zoom;
        Rect::from_min_max(Pos2::new(rect.left() + 6.0 * z, rect.top() + (HEAD + 4.0) * z), Pos2::new(rect.right() - 6.0 * z, rect.bottom() - 22.0 * z))
    }

    fn add_menu(&mut self, ui: &mut egui::Ui, at: Pos2) {
        let mut pick: Option<Kind> = None;
        // Headings and entries come from the modules' module.toml (category, label, variants).
        for (section, entries) in nodes::menu() {
            ui.label(egui::RichText::new(section.to_uppercase()).small().color(DIM));
            for (label, k) in entries {
                if ui.button(label).clicked() {
                    pick = Some(k);
                }
            }
            if section == nodes::PLUGINS && ui.button("Plugin from file…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("program", &["exe"]).pick_file() {
                    pick = Some(Kind::Plugin { path: p.to_string_lossy().into_owned() });
                }
            }
        }
        if let Some(k) = pick {
            let id = self.add(k, at);
            self.selected = Some(id);
            self.save_project();
            ui.close();
        }
    }

    fn graph_view(&mut self, ui: &mut egui::Ui) {
        self.area_cache.borrow_mut().clear();
        self.refresh_looks();
        let (area, bg) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, BG);
        // Scale around the pointer, keeping the item under it in place while scrolling.
        if let Some(pointer) = bg.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let world = (pointer - area.min - self.pan) / self.zoom;
                self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.4, 2.5);
                self.pan = pointer - area.min - world * self.zoom;
            }
        }
        // Background dots: one mesh of small squares, and never closer than 16 px when zoomed out.
        let mut step = 22.0 * self.zoom;
        while step < 16.0 {
            step *= 2.0;
        }
        let off = Vec2::new(self.pan.x.rem_euclid(step), self.pan.y.rem_euclid(step));
        let mut dots = egui::epaint::Mesh::default();
        let mut y = area.top() + off.y;
        while y < area.bottom() {
            let mut x = area.left() + off.x;
            while x < area.right() {
                dots.add_colored_rect(Rect::from_center_size(Pos2::new(x, y), Vec2::splat(2.0)), Color32::from_rgb(43, 47, 53));
                x += step;
            }
            y += step;
        }
        painter.add(egui::Shape::mesh(dots));
        if bg.dragged() && self.wiring.is_none() {
            self.pan += bg.drag_delta();
        }
        if bg.clicked() {
            self.selected = None;
        }
        let origin = area.min;
        self.canvas_origin = origin;
        let menu_at = (bg.interact_pointer_pos().or(ui.ctx().pointer_hover_pos()).unwrap_or(area.center()) - origin - self.pan) / self.zoom;
        bg.context_menu(|ui| self.add_menu(ui, menu_at.to_pos2()));
        // Scripted looks of the nodes on screen, run again only when what they read changed.
        self.run_scripts(origin, area, ui.input(|i| i.time));

        let trail = self.selected.and_then(|s| self.graph.trail(s).ok()).unwrap_or_default();
        // Wires out of a node fed by a running live stream pulse in the direction of flow.
        let live: Vec<bool> = self.nodes.iter().map(|n| n.state.lock().map(|s| s.running && s.live).unwrap_or(false)).collect();
        let now = ui.input(|i| i.time);
        for w in self.graph.wires() {
            let kind = self.graph.nodes()[w.from].spec.outputs[w.from_port].kind;
            let (a, sa) = self.port_place(origin, w.from, w.from_port, true);
            let (b, sb) = self.port_place(origin, w.to, w.to_port, false);
            let lit = trail.contains(&w.from) && trail.contains(&w.to);
            let col = if self.selected.is_some() && !lit { alpha(kind_color(kind), 0.2) } else { kind_color(kind) };
            let pts = layout::wire_points(a, sa, b, sb);
            let width = if lit { 2.6 } else { 1.7 };
            if live[w.from] {
                vein(&painter, pts, col, width * self.zoom, now, self.zoom);
            } else {
                painter.add(CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, Stroke::new(width, col)));
            }
        }

        let mut clicked = None;
        let mut delete = None;
        let mut drop_on: Option<(NodeId, usize)> = None;
        let mut unwire: Option<(NodeId, &'static str)> = None;
        let pointer = ui.ctx().pointer_latest_pos();
        let released = ui.input(|i| i.pointer.any_released());
        for i in 0..self.nodes.len() {
            if self.nodes[i].hidden {
                continue;
            }
            let nl = look::of(&self.nodes[i].kind);
            let rect = self.node_rect(origin, i);
            let head = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEAD * self.zoom));
            let resp = ui.interact(rect, egui::Id::new(("node", i)), Sense::click_and_drag());
            if resp.dragged() && self.wiring.is_none() {
                self.nodes[i].pos += resp.drag_delta() / self.zoom;
            }
            if resp.clicked() {
                clicked = Some(i);
            }
            let mut fold = false;
            let collapsed = self.nodes[i].collapsed;
            let label = &mut self.nodes[i].label;
            resp.context_menu(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(label);
                });
                if ui.button(if collapsed { "Expand" } else { "Collapse" }).clicked() {
                    fold = true;
                    ui.close();
                }
                if ui.button("Delete node").clicked() {
                    delete = Some(i);
                    ui.close();
                }
            });
            if fold {
                self.nodes[i].collapsed = !collapsed;
                self.save_project();
                continue;
            }
            let spec = self.graph.nodes()[i].spec.clone();
            let is_sel = self.selected == Some(i);
            let in_trail = trail.contains(&i);
            let fade = self.selected.is_some() && !in_trail;
            let frame = self.frame_look(i);
            let fill = frame.fill.map_or_else(|| nl.fill(self, i), |[r, g, b]| Color32::from_rgb(r, g, b));
            let radius = frame.radius.unwrap_or(6.0) * self.zoom;
            painter.rect_filled(rect, radius, if fade { alpha(fill, 0.6) } else { fill });
            let border = if is_sel { SEL } else if in_trail { alpha(SEL, 0.6) } else { LINE };
            painter.rect_stroke(rect, radius, Stroke::new((if is_sel { 2.0 } else { 1.0 }) * self.zoom, border), StrokeKind::Inside);
            painter.line_segment([Pos2::new(rect.left(), head.bottom()), Pos2::new(rect.right(), head.bottom())], Stroke::new(self.zoom, LINE));
            let tc = if fade { DIM } else { TEXT };
            let lab = &self.nodes[i].label;
            let lab = if lab.chars().count() > 21 { format!("{}…", lab.chars().take(20).collect::<String>()) } else { lab.clone() };
            painter.text(head.left_center() + Vec2::new(9.0 * self.zoom, 0.0), Align2::LEFT_CENTER, lab, FontId::proportional(13.0 * self.zoom), tc);
            painter.text(head.right_center() - Vec2::new(9.0 * self.zoom, 0.0), Align2::RIGHT_CENTER, role_text(spec.role), FontId::proportional(10.0 * self.zoom), DIM);
            // The node's own drawing and controls: its look.rhai when it has one (which may call
            // the folder's Rust `Look` as native()), else that `Look`.
            let show_status = frame.status.unwrap_or_else(|| nl.status());
            let body = if frame.status != Some(false) { self.body_rect(rect) } else { Rect::from_min_max(self.body_rect(rect).min, rect.max - Vec2::splat(6.0 * self.zoom)) };
            if !collapsed && body.width() > 0.0 && body.height() > 0.0 {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt(("body", i)));
                child.set_clip_rect(body.intersect(area));
                look::scale(&mut child, self.zoom);
                if !self.draw_script(&mut child, i, body) {
                    nl.body(self, &mut child, i);
                }
            }
            // Status line at the bottom of the node.
            if !collapsed && show_status {
                let (status, running) = {
                    let s = self.nodes[i].state.lock().unwrap();
                    let st = match &self.nodes[i].kind {
                        Kind::Plugin { .. } | Kind::Module { .. } => plugin_node::status(&s).unwrap_or_else(|| s.status.clone()),
                        Kind::ManualData { rows } => manual_data_node::status(rows),
                        Kind::Map => self.map_status(i),
                        Kind::CombineLocations { .. } | Kind::InvertLocation => match self.area_from(i) {
                            Ok((a, _)) => map_node::area_status(&a),
                            Err(e) => e,
                        },
                        Kind::ResultRecord { .. } => self.record_status(i),
                        Kind::Missing { .. } => "module not found: add its folder to modules\\".into(),
                        _ => s.status.clone(),
                    };
                    (st, s.running)
                };
                let status = if status.chars().count() > 34 { format!("{}…", status.chars().take(33).collect::<String>()) } else { status };
                let sy = rect.bottom() - 12.0 * self.zoom;
                if running {
                    let on = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
                    painter.circle_filled(Pos2::new(rect.left() + 12.0 * self.zoom, sy), 3.5 * self.zoom, if on { LIVE } else { alpha(LIVE, 0.3) });
                }
                painter.text(Pos2::new(rect.left() + 22.0 * self.zoom, sy), Align2::LEFT_CENTER, status, FontId::proportional(10.5 * self.zoom), DIM);
            }

            let missing = self.graph.missing(i);
            for (k, p) in spec.inputs.iter().enumerate() {
                let pl = self.port_look(i, p);
                if !self.port_shown(i, p, false, &pl) {
                    continue;
                }
                let (c, side) = self.port_place(origin, i, k, false);
                // An input that takes anything: grey and open until wired, then the wired kind's colour and name.
                let fed = (p.kind == PortKind::Any).then(|| self.graph.feeder(i, p.name)).flatten();
                let fed_kind = fed.and_then(|(f, o)| self.graph.nodes()[f].spec.outputs.iter().find(|x| x.name == o).map(|x| x.kind));
                let col = match fed_kind {
                    Some(fk) => kind_color(fk),
                    None if p.kind == PortKind::Any => Color32::from_rgb(110, 116, 124),
                    None => kind_color(p.kind),
                };
                let name: String = match (fed, fed_kind) {
                    (Some((_, o)), Some(fk)) => format!("{o} · {}", corroborate::modules::result_record::kind_name(fk)),
                    _ if p.kind == PortKind::Any => "+".into(),
                    _ => p.name.into(),
                };
                let nc = if missing.contains(&p.name) { WARN } else if p.required { tc } else { DIM };
                let nc = if fed_kind.is_some() { tc } else { nc };
                layout::draw_port(&painter, c, side, &pl, col, fed_kind.is_some() || p.required, &name, nc, self.zoom);
                let pr = ui.interact(Rect::from_center_size(c, Vec2::splat(16.0 * self.zoom)), egui::Id::new(("in", i, k)), Sense::click());
                if pr.secondary_clicked() {
                    unwire = Some((i, p.name));
                }
                if self.wiring.is_some() && released && pointer.is_some_and(|q| q.distance(c) < 12.0 * self.zoom) {
                    drop_on = Some((i, k));
                }
            }
            for (k, p) in spec.outputs.iter().enumerate() {
                let pl = self.port_look(i, p);
                if !self.port_shown(i, p, true, &pl) {
                    continue;
                }
                let (c, side) = self.port_place(origin, i, k, true);
                layout::draw_port(&painter, c, side, &pl, kind_color(p.kind), true, p.name, tc, self.zoom);
                let pr = ui.interact(Rect::from_center_size(c, Vec2::splat(16.0 * self.zoom)), egui::Id::new(("out", i, k)), Sense::drag());
                if pr.drag_started() {
                    self.wiring = Some((i, k));
                }
            }
        }

        // Wire being dragged.
        if let Some((n, k)) = self.wiring {
            let (a, sa) = self.port_place(origin, n, k, true);
            if let Some(b) = pointer {
                let kind = self.graph.nodes()[n].spec.outputs[k].kind;
                painter.add(CubicBezierShape::from_points_stroke(
                    layout::wire_points(a, sa, b, corroborate::module_manifest::Side::Left),
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(2.0 * self.zoom, kind_color(kind)),
                ));
            }
            if released {
                if let Some((t, ti)) = drop_on {
                    let out = self.graph.nodes()[n].spec.outputs[k].name;
                    let inp = self.graph.nodes()[t].spec.inputs[ti].name;
                    match self.graph.connect(n, out, t, inp) {
                        Ok(()) => {
                            self.msg = format!("wired {}.{out} → {}.{inp}", self.nodes[n].label, self.nodes[t].label);
                            self.sync_record_ports();
                            self.save_project();
                        }
                        Err(e) => self.msg = describe(&e),
                    }
                }
                self.wiring = None;
            }
        }
        if let Some((i, name)) = unwire {
            if self.graph.disconnect(i, name) {
                self.msg = format!("unwired {}.{name}", self.nodes[i].label);
                self.sync_record_ports();
                self.save_project();
            }
        }
        if let Some(i) = delete {
            if self.recordings.contains_key(&i) {
                self.record_end(i);
            }
            self.graph.isolate(i);
            self.nodes[i].hidden = true;
            self.sync_record_ports();
            self.selected = None;
            self.save_project();
        }
        if let Some(i) = clicked {
            self.selected = Some(i);
        }
        painter.text(
            area.left_bottom() + Vec2::new(10.0, -8.0),
            Align2::LEFT_BOTTOM,
            "scroll: zoom · drag header: move · drag empty: pan · drag output to input: wire · right-click input: unwire · right-click: add node / rename / collapse / delete",
            FontId::proportional(11.0),
            DIM,
        );
    }

    // ---------------- timeline

    fn timeline_view(&mut self, ui: &mut egui::Ui) {
        struct Lane {
            name: String,
            utc: bool,
            note: String,
            marks: Vec<Mark>,
        }
        let mut lanes: Vec<Lane> = Vec::new();
        let mut any_utc = false;
        let mut end = f64::MIN;
        for n in self.nodes.iter().filter(|n| !n.hidden) {
            let s = n.state.lock().unwrap();
            let a = s.anchor.clone();
            let (to, err) = clock(&a);
            let marks = match &n.kind {
                Kind::Planes => planes_node::marks(&s),
                Kind::Enf { .. } => enf_node::marks(&s),
                Kind::Plugin { .. } => plugin_node::marks(&s),
                // A module on the bus has a lane once it has sent events.
                Kind::Module { .. } => match bus::marks(&s) {
                    m if m.is_empty() => continue,
                    m => m,
                },
                _ => continue,
            };
            if s.audio_s > 0.0 {
                end = end.max(to(s.audio_s));
            }
            any_utc |= a.is_some();
            let note = match &a {
                Some(a) => format!("clock: {} (± {:.0} s)", a.source, err),
                None if s.audio_s > 0.0 => "no UTC anchor: source time".into(),
                None => String::new(),
            };
            lanes.push(Lane { name: n.label.clone(), utc: a.is_some(), note, marks });
        }

        let label_w = 190.0;
        let ruler_h = 22.0;
        let lane_h = 34.0;
        let h = ruler_h + lane_h * lanes.len().max(1) as f32 + 4.0;
        let (area, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click_and_drag());
        let p = ui.painter_at(area);
        p.rect_filled(area, 0.0, PANEL);
        let track = Rect::from_min_max(Pos2::new(area.left() + label_w, area.top()), area.max);

        if self.follow && end > f64::MIN {
            let span = (self.tl.1 - self.tl.0).max(60.0);
            self.tl = (end - span * 0.95, end + span * 0.05);
        }
        let span = (self.tl.1 - self.tl.0).max(1e-3);
        if let Some(hp) = resp.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
            if scroll != 0.0 && track.contains(hp) {
                let f = (-scroll as f64 * 0.002).exp();
                let at = self.tl.0 + ((hp.x - track.left()) / track.width()) as f64 * span;
                self.tl = (at - (at - self.tl.0) * f, at + (self.tl.1 - at) * f);
            }
        }
        if resp.dragged() {
            let dt = -(resp.drag_delta().x / track.width()) as f64 * span;
            self.tl = (self.tl.0 + dt, self.tl.1 + dt);
            self.follow = false;
        }
        let (v0, span) = (self.tl.0, (self.tl.1 - self.tl.0).max(1e-3));
        let tx = |t: f64| track.left() + ((t - v0) / span) as f32 * track.width();

        let raw = span / 8.0;
        let mag = 10f64.powf(raw.log10().floor());
        let step = [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
        let mut t = (v0 / step).ceil() * step;
        while t <= self.tl.1 {
            let x = tx(t);
            p.line_segment([Pos2::new(x, track.top()), Pos2::new(x, track.bottom())], Stroke::new(1.0, Color32::from_rgb(40, 44, 49)));
            let txt = if any_utc { fmt_clock(t) } else { format!("{t:.0} s") };
            p.text(Pos2::new(x + 3.0, track.top() + 4.0), Align2::LEFT_TOP, txt, FontId::proportional(10.5), DIM);
            t += step;
        }
        p.line_segment([Pos2::new(area.left(), area.top() + ruler_h), Pos2::new(area.right(), area.top() + ruler_h)], Stroke::new(1.0, LINE));
        p.text(
            Pos2::new(area.left() + 8.0, area.top() + 4.0),
            Align2::LEFT_TOP,
            if any_utc { "UTC" } else { "source time" },
            FontId::proportional(10.5),
            DIM,
        );
        let mut hover = None;
        for (li, lane) in lanes.iter().enumerate() {
            let top = area.top() + ruler_h + li as f32 * lane_h;
            let row = Rect::from_min_max(Pos2::new(area.left(), top), Pos2::new(area.right(), top + lane_h));
            p.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, Color32::from_rgb(42, 46, 51)));
            p.text(Pos2::new(area.left() + 8.0, top + 11.0), Align2::LEFT_CENTER, &lane.name, FontId::proportional(12.0), TEXT);
            p.text(Pos2::new(area.left() + 8.0, top + 25.0), Align2::LEFT_CENTER, &lane.note, FontId::proportional(9.5), DIM);
            if any_utc && !lane.utc && !lane.marks.is_empty() {
                p.text(Pos2::new(track.left() + 8.0, row.center().y), Align2::LEFT_CENTER, "own clock, offset unknown: not placed on UTC", FontId::proportional(10.5), DIM);
                continue;
            }
            for m in &lane.marks {
                let (xa, xb) = (tx(m.t0 - m.err), tx(m.t1 + m.err));
                if xb < track.left() || xa > track.right() {
                    continue;
                }
                let band = Rect::from_min_max(Pos2::new(xa.max(track.left()), top + 5.0), Pos2::new(xb.min(track.right()), top + lane_h - 5.0));
                let strength = (1.0 / (1.0 + m.err)) as f32;
                p.rect_filled(band, 2.0, alpha(m.col, 0.10 + 0.15 * strength));
                let core = Rect::from_min_max(
                    Pos2::new(tx(m.t0).max(track.left()), top + 8.0),
                    Pos2::new(tx(m.t1).min(track.right()).max(tx(m.t0) + 2.0), top + lane_h - 8.0),
                );
                p.rect_filled(core, 2.0, alpha(m.col, 0.35 + 0.55 * strength));
                if core.width() > 40.0 {
                    p.text(core.left_center() + Vec2::new(3.0, 0.0), Align2::LEFT_CENTER, &m.label, FontId::proportional(10.0), Color32::BLACK);
                }
                if resp.hover_pos().is_some_and(|h| band.expand(2.0).contains(h)) {
                    hover = Some(format!("{}\n{}\ntime error ± {:.0} s ({})", m.label, m.detail, m.err, lane.note));
                }
            }
        }
        if lanes.is_empty() {
            p.text(Pos2::new(track.left() + 8.0, area.top() + ruler_h + 14.0), Align2::LEFT_CENTER, "findings from detectors and plugins appear here", FontId::proportional(11.0), DIM);
        }
        if let Some(t) = hover {
            resp.on_hover_text(t);
        }
    }

    // ---------------- frame

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("CORROBORATE").strong().color(TEXT));
            // Which build this is: the commit it was built from (+ when there were uncommitted
            // changes), set by run.cmd; "dev" for a plain cargo build.
            ui.colored_label(DIM, option_env!("CORROBORATE_BUILD").unwrap_or("dev"));
            ui.separator();
            {
                // The graph runs always: changes restart it. Recording is in the Result record node.
                if ui.button("↻ Restart").on_hover_text("start every source again from its beginning").clicked() {
                    self.run();
                }
                ui.menu_button("Add node", |ui| {
                    let at = Pos2::new(400.0, 300.0) - self.pan;
                    self.add_menu(ui, at);
                });
                if ui.button("Results folder").clicked() {
                    let d = results_dir();
                    let _ = std::fs::create_dir_all(&d);
                    let _ = std::process::Command::new("explorer").arg(d).spawn();
                }
                if !self.follow && ui.button("Follow timeline").clicked() {
                    self.follow = true;
                }
            }
            ui.colored_label(DIM, &self.msg);
        });
    }
}

fn describe(e: &GraphError) -> String {
    match e {
        GraphError::KindMismatch { from, to } => format!("cannot wire {from} into {to}: the types differ"),
        GraphError::InputTaken { port, .. } => format!("input '{port}' already has a wire (right-click it to unwire)"),
        GraphError::Cycle => "that wire would make a loop".into(),
        other => format!("{other:?}"),
    }
}

/// A wire carrying a live signal: a thin line with swellings that travel from output to input,
/// like a pulse in a vein.
fn vein(painter: &egui::Painter, pts: [Pos2; 4], col: Color32, width: f32, now: f64, zoom: f32) {
    const N: usize = 64;
    let at = |t: f32| {
        let u = 1.0 - t;
        (pts[0].to_vec2() * u * u * u + pts[1].to_vec2() * 3.0 * u * u * t + pts[2].to_vec2() * 3.0 * u * t * t + pts[3].to_vec2() * t * t * t).to_pos2()
    };
    let line: Vec<Pos2> = (0..=N).map(|k| at(k as f32 / N as f32)).collect();
    let mut along = 0.0f32;
    let spacing = 70.0 * zoom;
    for s in line.windows(2) {
        let (p, q) = (s[0], s[1]);
        let len = p.distance(q);
        let mid = along + len * 0.5;
        along += len;
        // Phase falls behind with time, so each crest moves towards the input.
        let phase = (mid / spacing) as f64 - now * 1.6;
        let v = (0.5 + 0.5 * (phase * std::f64::consts::TAU).cos()).powi(6) as f32;
        painter.line_segment([p, q], Stroke::new(width * (2.5 + 5.0 * v), alpha(col, 0.10 * v)));
        painter.line_segment([p, q], Stroke::new(width * (0.8 + 1.6 * v), alpha(col, 0.45 + 0.55 * v)));
    }
}

/// What a source node shows under its controls: status, audio so far, and its UTC clock.
fn source_info(ui: &mut egui::Ui, st: &Shared) {
    let s = st.lock().unwrap();
    look::line(ui, if s.status.is_empty() { "not run".to_string() } else { s.status.clone() }, if s.running { LIVE } else { TEXT });
    if s.audio_s > 0.0 {
        look::line(ui, format!("audio: {:.0} s{}", s.audio_s, if s.live { " (live)" } else { "" }), TEXT);
    }
    match &s.anchor {
        Some(a) => {
            look::line(ui, format!("time 0 = {} UTC", timebase::fmt_utc(a.utc0)), TEXT);
            look::line(ui, format!("± {:.0} s · from {}", anchor_error_s(a), a.source), DIM);
        }
        None => look::line(ui, "no UTC anchor yet", DIM),
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }
}

impl App {
    /// One frame of the whole workbench. Separate from `eframe::App` so tests can drive it
    /// headless (drive.rs).
    fn frame(&mut self, ui: &mut egui::Ui) {
        if self.earth.is_none() {
            self.earth = Some(map_node::load_earth(ui.ctx()));
        }
        egui::Panel::top("bar").frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(10, 6))).show(ui, |ui| {
            self.top_bar(ui);
        });
        egui::Panel::bottom("timeline").frame(egui::Frame::new().fill(PANEL).inner_margin(0.0)).show(ui, |ui| {
            self.timeline_view(ui);
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ui, |ui| {
            self.graph_view(ui);
        });
        let ctx = ui.ctx().clone();

        let running = self.running();
        self.keep_running();
        self.feed_programs();
        self.record_tick();
        if self.last_project_save.elapsed() > Duration::from_secs(10) {
            self.save_project();
        }
        // Live wires pulse: redraw smoothly while a live stream runs.
        let live = self.nodes.iter().any(|n| n.state.lock().map(|s| s.running && s.live).unwrap_or(false));
        ctx.request_repaint_after(Duration::from_millis(if live { 33 } else if running { 200 } else { 1000 }));
    }
}

fn main() -> eframe::Result {
    // corroborate [path or URL] [--run] [--new]: --run starts the saved project at once
    // (unattended runs); --new opens an empty canvas.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let autorun = args.iter().any(|a| a == "--run");
    let start = args.into_iter().find(|a| !a.starts_with("--"));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1400.0, 780.0]).with_title("Corroborate"),
        ..Default::default()
    };
    eframe::run_native(
        "corroborate",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(App::new(start, autorun)))
        }),
    )
}

#[cfg(test)]
mod canvas_timing {
    use super::*;

    /// Time to build and tessellate one canvas frame (no GPU): maps, an
    /// inverted area and the record. Run: cargo test --release --bin corroborate canvas -- --ignored --nocapture
    #[test]
    #[ignore]
    fn canvas_frame_time() {
        let ctx = egui::Context::default();
        let mut app = App::empty();
        let rows = vec![("coordinates".to_string(), "50, 15".to_string()), ("radius_km".to_string(), "2500".to_string())];
        let man = app.add(Kind::ManualData { rows }, Pos2::new(20.0, 420.0));
        let inv = app.add(Kind::InvertLocation, Pos2::new(250.0, 420.0));
        app.wire(man, "place", inv, "location");
        let rec = app.add(Kind::ResultRecord { folder: String::new() }, Pos2::new(480.0, 60.0));
        app.wire_record(man, "place", rec);
        for (k, from, out) in [(0, man, "place"), (1, inv, "location"), (2, rec, "location")] {
            let m = app.add(Kind::Map, Pos2::new(480.0 + k as f32 * 340.0, 420.0));
            app.wire(from, out, m, "location");
        }
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 780.0))),
            ..Default::default()
        };
        let frame = |app: &mut App| {
            let out = ctx.run_ui(input(), |ui| {
                if app.earth.is_none() {
                    app.earth = Some(map_node::load_earth(ui.ctx()));
                }
                egui::CentralPanel::default().show(ui, |ui| app.graph_view(ui));
            });
            let n = ctx.tessellate(out.shapes, out.pixels_per_point).len();
            std::hint::black_box(n);
        };
        for _ in 0..3 {
            frame(&mut app);
        }
        let frames = 30;
        let t = Instant::now();
        for f in 0..frames {
            app.zoom = 0.6 + (f % 10) as f32 * 0.08;
            frame(&mut app);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!("canvas frame: {ms:.1} ms ({} nodes)", app.nodes.len());
    }
}
