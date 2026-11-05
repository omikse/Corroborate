//! Corroborate: the workbench. Sources, detectors and plugins are nodes in a graph; wires
//! decide what feeds what. Run starts every source; each node's window shows its raw data;
//! the timeline shows every finding on one clock with its time error; the Record node keeps
//! a result JSON (findings, facts and the graph as the evidence trail) up to date on disk.
//!
//! Graph: drag a node by its header, drag empty space to pan, drag from an output port to an
//! input port to wire them (types must match), right-click an input to unwire it, right-click
//! a node or empty space for more. Double-click a node to open its window.

mod demo;
mod nodes;
mod runtime;

use doppler_live::core::{PortKind, Role, SourceKind};
use doppler_live::graph::{Graph, GraphError, NodeId};
use doppler_live::timebase::{self, json_escape, Anchor};
use eframe::egui::{
    self, epaint::CubicBezierShape, Align2, Color32, ColorImage, FontId, Pos2, Rect, Sense, Stroke, StrokeKind,
    TextureHandle, TextureOptions, Vec2,
};
use nodes::Kind;
use runtime::{Input, Job, PluginJob, Shared};
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

// ---------------------------------------------------------------- project

struct NodeData {
    kind: Kind,
    label: String,
    pos: Pos2,
    state: Shared,
    /// Deleted from the workspace (ids stay stable; a deleted node has no wires).
    hidden: bool,
}

fn project_file() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("corroborate").join("project.txt"))
}

fn results_dir() -> PathBuf {
    let base = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Desktop").join("record").join("corroborate")
}

fn clean(s: &str) -> String {
    s.replace(['|', '\n', '\r'], " ")
}

// ---------------------------------------------------------------- app

#[derive(PartialEq)]
enum Mode {
    Work,
    Demo,
}

struct App {
    mode: Mode,
    demo: Option<demo::Demo>,
    graph: Graph,
    nodes: Vec<NodeData>,
    pan: Vec2,
    selected: Option<NodeId>,
    open: Vec<bool>,
    /// Wire being dragged from (node, output index).
    wiring: Option<(NodeId, usize)>,
    msg: String,
    stop: Option<Arc<AtomicBool>>,
    result_file: Option<PathBuf>,
    last_save: Instant,
    saved_note: String,
    textures: HashMap<NodeId, (u64, f64, TextureHandle)>,
    lut: [[u8; 4]; 256],
    /// Timeline view (UTC seconds when anchored, else source seconds), and whether it follows the end.
    tl: (f64, f64),
    follow: bool,
    /// Minutes of spectrogram shown in Planes windows (0 = all).
    spec_minutes: f64,
    mics: Vec<String>,
    last_project_save: Instant,
}

const NODE_W: f32 = 210.0;
const HEAD: f32 = 26.0;
const ROW: f32 = 20.0;

impl App {
    fn new(start: Option<String>, autorun: bool) -> Self {
        let mut app = Self {
            mode: Mode::Work,
            demo: None,
            graph: Graph::new(),
            nodes: Vec::new(),
            pan: Vec2::ZERO,
            selected: None,
            open: Vec::new(),
            wiring: None,
            msg: String::new(),
            stop: None,
            result_file: None,
            last_save: Instant::now(),
            saved_note: String::new(),
            textures: HashMap::new(),
            lut: colormap(),
            tl: (0.0, 600.0),
            follow: true,
            spec_minutes: 0.0,
            mics: Vec::new(),
            last_project_save: Instant::now(),
        };
        if !app.load_project() {
            app.default_project();
        }
        if let Some(src) = start {
            // A path or URL on the command line goes into the first source node.
            let kind = if src.contains("://") { Kind::Stream { url: src } } else { Kind::File { path: src } };
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
        self.nodes.push(NodeData { label: kind.default_label(), kind, pos, state: Default::default(), hidden: false });
        self.open.push(false);
        id
    }

    fn wire(&mut self, a: NodeId, out: &str, b: NodeId, inp: &str) {
        if let Err(e) = self.graph.connect(a, out, b, inp) {
            self.msg = describe(&e);
        }
    }

    /// Stream → Planes and ADS-B check; Manual data gives ADS-B check the site; all into Record.
    fn default_project(&mut self) {
        let src = self.add(Kind::Stream { url: String::new() }, Pos2::new(30.0, 40.0));
        let mut rows = nodes::manual_defaults();
        rows.push(("elevation".into(), String::new()));
        let man = self.add(Kind::Manual { rows }, Pos2::new(30.0, 260.0));
        let planes = self.add(Kind::Planes, Pos2::new(330.0, 30.0));
        let rec = self.add(Kind::Record, Pos2::new(660.0, 60.0));
        self.wire(src, "audio", planes, "audio");
        self.wire(src, "time", planes, "time");
        self.wire(planes, "passes", rec, nodes::RECORD_EVENTS[0]);
        self.wire(man, "facts", rec, nodes::RECORD_FACTS[0]);
        let bundled = nodes::bundled_plugins();
        if let Some(adsb) = bundled.iter().find(|p| p.contains("adsb-check")) {
            let a = self.add(Kind::Plugin { path: adsb.clone() }, Pos2::new(330.0, 220.0));
            self.wire(src, "audio", a, "audio");
            self.wire(src, "time", a, "time");
            self.wire(man, "place", a, "place");
            self.wire(a, "findings", rec, nodes::RECORD_EVENTS[1]);
            self.wire(a, "facts", rec, nodes::RECORD_FACTS[1]);
        } else {
            self.msg = "adsb-check not found next to this program: add plugins with right-click ▸ Add node".into();
        }
    }

    // ---------------- project file

    fn save_project(&mut self) {
        let Some(path) = project_file() else { return };
        let mut text = String::from("# corroborate project: node|kind|x|y|label|param, row|node|key|value, wire|from|out|to|in\n");
        // Only visible nodes, renumbered.
        let keep: Vec<NodeId> = (0..self.nodes.len()).filter(|&i| !self.nodes[i].hidden).collect();
        let new_id = |old: NodeId| keep.iter().position(|&k| k == old);
        for &i in &keep {
            let n = &self.nodes[i];
            let (k, param) = match &n.kind {
                Kind::File { path } => ("file", path.clone()),
                Kind::Stream { url } => ("stream", url.clone()),
                Kind::Mic { device } => ("mic", device.clone()),
                Kind::Manual { .. } => ("manual", String::new()),
                Kind::Planes => ("planes", String::new()),
                Kind::Enf => ("enf", String::new()),
                Kind::Plugin { path } => ("plugin", path.clone()),
                Kind::Record => ("record", String::new()),
            };
            text.push_str(&format!("node|{k}|{:.0}|{:.0}|{}|{}\n", n.pos.x, n.pos.y, clean(&n.label), clean(&param)));
        }
        for (ni, &i) in keep.iter().enumerate() {
            if let Kind::Manual { rows } = &self.nodes[i].kind {
                for (k, v) in rows {
                    text.push_str(&format!("row|{ni}|{}|{}\n", clean(k), clean(v)));
                }
            }
        }
        for w in self.graph.wires() {
            if let (Some(a), Some(b)) = (new_id(w.from), new_id(w.to)) {
                let out = self.graph.nodes()[w.from].spec.outputs[w.from_port].name;
                let inp = self.graph.nodes()[w.to].spec.inputs[w.to_port].name;
                text.push_str(&format!("wire|{a}|{out}|{b}|{inp}\n"));
            }
        }
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&path, text);
        self.last_project_save = Instant::now();
    }

    fn load_project(&mut self) -> bool {
        let Some(text) = project_file().and_then(|p| std::fs::read_to_string(p).ok()) else { return false };
        let mut rows: Vec<(usize, String, String)> = Vec::new();
        let mut wires: Vec<(usize, String, usize, String)> = Vec::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('|').collect();
            match f.as_slice() {
                ["node", k, x, y, label, param] => {
                    let p = param.to_string();
                    let kind = match *k {
                        "file" => Kind::File { path: p },
                        "stream" => Kind::Stream { url: p },
                        "mic" => Kind::Mic { device: p },
                        "manual" => Kind::Manual { rows: Vec::new() },
                        "planes" => Kind::Planes,
                        "enf" => Kind::Enf,
                        "plugin" => Kind::Plugin { path: p },
                        "record" => Kind::Record,
                        _ => continue,
                    };
                    let id = self.add(kind, Pos2::new(x.parse().unwrap_or(40.0), y.parse().unwrap_or(40.0)));
                    if !label.trim().is_empty() {
                        self.nodes[id].label = label.to_string();
                    }
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
            if let Some(Kind::Manual { rows }) = self.nodes.get_mut(n).map(|x| &mut x.kind) {
                rows.push((k, v));
            }
        }
        for (a, o, b, i) in wires {
            if a < self.nodes.len() && b < self.nodes.len() {
                let _ = self.graph.connect(a, &o, b, &i);
            }
        }
        // Graph labels follow the node labels.
        true
    }

    // ---------------- run

    fn running(&self) -> bool {
        self.nodes.iter().any(|n| n.state.lock().unwrap().running)
    }

    fn manual_rows_into(&self, id: NodeId) -> Vec<(String, String)> {
        let mut rows = Vec::new();
        for inp in ["place", "time"] {
            if let Some((from, _)) = self.graph.feeder(id, inp) {
                if let Kind::Manual { rows: r } = &self.nodes[from].kind {
                    for (k, v) in r {
                        if !rows.iter().any(|(q, _): &(String, String)| q == k) {
                            rows.push((k.clone(), v.clone()));
                        }
                    }
                }
            }
        }
        rows
    }

    fn run(&mut self) {
        self.stop_all();
        self.save_project();
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
                Kind::File { path } if !path.trim().is_empty() => Input::File(path.trim().to_string()),
                Kind::Stream { url } if !url.trim().is_empty() => Input::Url(url.trim().to_string()),
                Kind::Mic { device } if !device.trim().is_empty() => Input::Mic(device.trim().to_string()),
                _ => {
                    n.state.lock().unwrap().status = "nothing to open: set it in the node's window".into();
                    continue;
                }
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
            };
            for (t, _) in self.graph.targets(i, "audio") {
                let tn = &self.nodes[t];
                if tn.hidden {
                    continue;
                }
                match &tn.kind {
                    Kind::Planes => job.planes.push(tn.state.clone()),
                    Kind::Enf => job.enfs.push(tn.state.clone()),
                    Kind::Plugin { path } => {
                        job.plugins.push(PluginJob { path: path.clone(), state: tn.state.clone(), manual: self.manual_rows_into(t) })
                    }
                    _ => {}
                }
            }
            runtime::start(job);
            started += 1;
        }
        if started == 0 {
            self.msg = "no source to run: open a source node (double-click) and set its file, URL or microphone".into();
            return;
        }
        self.stop = Some(stop);
        self.result_file = Some(dir.join(format!("corroborate-{}.json", timebase::utc_stamp(timebase::now_unix()))));
        self.last_save = Instant::now();
        self.follow = true;
        self.msg = format!("running {started} source(s); results kept in {}", dir.display());
    }

    fn stop_all(&mut self) {
        if let Some(s) = self.stop.take() {
            s.store(true, Ordering::Relaxed);
        }
    }

    // ---------------- result record

    fn result_json(&self) -> String {
        let mut nodes_j = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if n.hidden {
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
            match &n.kind {
                Kind::Planes => {
                    let p: Vec<String> = s.passes.iter().map(|e| e.to_json(anchor.as_ref())).collect();
                    body.push_str(&format!(",\"passes\":[{}]", p.join(",")));
                }
                Kind::Enf => {
                    let p: Vec<String> = s.enf_segments.iter().map(|e| e.to_json(anchor.as_ref())).collect();
                    body.push_str(&format!(",\"hum_runs\":[{}]", p.join(",")));
                }
                Kind::Plugin { path } => {
                    let f: Vec<String> = s
                        .plugin
                        .findings
                        .iter()
                        .map(|f| {
                            let utc = anchor.as_ref().map_or("null".to_string(), |a| format!("\"{}\"", timebase::fmt_utc(a.utc(f.t_start))));
                            format!(
                                "{{\"t_start\":{:.2},\"t_end\":{:.2},\"utc_start\":{utc},\"label\":\"{}\",\"detail\":\"{}\"}}",
                                f.t_start,
                                f.t_end,
                                json_escape(&f.label),
                                json_escape(&f.detail)
                            )
                        })
                        .collect();
                    let m: Vec<String> =
                        s.plugin.meta.iter().map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v))).collect();
                    body.push_str(&format!(
                        ",\"program\":\"{}\",\"state\":\"{}\",\"plugin_status\":\"{}\",\"dropped_s\":{:.1},\"findings\":[{}],\"facts\":{{{}}}",
                        json_escape(path),
                        json_escape(&s.plugin.state),
                        json_escape(&s.plugin.status),
                        s.plugin.dropped_s,
                        f.join(","),
                        m.join(",")
                    ));
                }
                Kind::Manual { rows } => {
                    let m: Vec<String> = rows
                        .iter()
                        .filter(|(k, v)| !k.trim().is_empty() && !v.trim().is_empty())
                        .map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k.trim()), json_escape(v.trim())))
                        .collect();
                    body.push_str(&format!(",\"facts\":{{{}}}", m.join(",")));
                }
                Kind::File { .. } | Kind::Stream { .. } | Kind::Mic { .. } => {
                    body.push_str(&format!(",\"audio_s\":{:.1},\"live\":{}", s.audio_s, s.live));
                }
                Kind::Record => {}
            }
            nodes_j.push(format!("{{{body}}}"));
        }
        let wires: Vec<String> = self
            .graph
            .wires()
            .iter()
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
            "{{\"app\":\"corroborate\",\"written_utc\":\"{}\",\"note\":\"every node with its results; wires are the evidence trail; anchor error_s is an estimate from the anchor's source\",\"nodes\":[\n{}\n],\"wires\":[{}]}}\n",
            timebase::fmt_utc(timebase::now_unix()),
            nodes_j.join(",\n"),
            wires.join(",")
        )
    }

    fn save_result(&mut self) {
        if !self.nodes.iter().any(|n| !n.hidden && matches!(n.kind, Kind::Record)) {
            return;
        }
        let Some(path) = self.result_file.clone() else { return };
        let tmp = path.with_extension("json.tmp");
        let text = self.result_json();
        let r = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path));
        self.saved_note = match r {
            Ok(()) => format!("saved {} UTC", fmt_clock(timebase::now_unix())),
            Err(e) => format!("could not save: {e}"),
        };
        self.last_save = Instant::now();
    }

    // ---------------- graph

    fn node_rect(&self, origin: Pos2, i: NodeId) -> Rect {
        let spec = &self.graph.nodes()[i].spec;
        let rows = spec.inputs.len().max(spec.outputs.len()).max(1) as f32;
        Rect::from_min_size(origin + self.pan + self.nodes[i].pos.to_vec2(), Vec2::new(NODE_W, HEAD + rows * ROW + 26.0))
    }

    fn port_pos(&self, origin: Pos2, i: NodeId, idx: usize, out: bool) -> Pos2 {
        let r = self.node_rect(origin, i);
        Pos2::new(if out { r.right() } else { r.left() }, r.top() + HEAD + 4.0 + idx as f32 * ROW + ROW / 2.0)
    }

    fn add_menu(&mut self, ui: &mut egui::Ui, at: Pos2) {
        let mut pick: Option<Kind> = None;
        ui.label(egui::RichText::new("SOURCES").small().color(DIM));
        if ui.button("File").clicked() {
            pick = Some(Kind::File { path: String::new() });
        }
        if ui.button("Stream (URL)").clicked() {
            pick = Some(Kind::Stream { url: String::new() });
        }
        if ui.button("Microphone").clicked() {
            pick = Some(Kind::Mic { device: String::new() });
        }
        if ui.button("Manual data").clicked() {
            pick = Some(Kind::Manual { rows: nodes::manual_defaults() });
        }
        ui.label(egui::RichText::new("DETECTORS").small().color(DIM));
        if ui.button("Planes (Doppler)").clicked() {
            pick = Some(Kind::Planes);
        }
        if ui.button("Power hum (ENF)").clicked() {
            pick = Some(Kind::Enf);
        }
        ui.label(egui::RichText::new("PLUGINS").small().color(DIM));
        for p in nodes::bundled_plugins() {
            if ui.button(nodes::plugin_label(&p)).clicked() {
                pick = Some(Kind::Plugin { path: p });
            }
        }
        if ui.button("Plugin from file…").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("program", &["exe"]).pick_file() {
                pick = Some(Kind::Plugin { path: p.to_string_lossy().into_owned() });
            }
        }
        ui.label(egui::RichText::new("OUTPUT").small().color(DIM));
        if ui.button("Result record").clicked() {
            pick = Some(Kind::Record);
        }
        if let Some(k) = pick {
            let id = self.add(k, at);
            self.selected = Some(id);
            self.save_project();
            ui.close();
        }
    }

    fn graph_view(&mut self, ui: &mut egui::Ui) {
        let (area, bg) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, BG);
        let step = 22.0;
        let off = Vec2::new(self.pan.x.rem_euclid(step), self.pan.y.rem_euclid(step));
        let mut y = area.top() + off.y;
        while y < area.bottom() {
            let mut x = area.left() + off.x;
            while x < area.right() {
                painter.circle_filled(Pos2::new(x, y), 1.0, Color32::from_rgb(43, 47, 53));
                x += step;
            }
            y += step;
        }
        if bg.dragged() && self.wiring.is_none() {
            self.pan += bg.drag_delta();
        }
        if bg.clicked() {
            self.selected = None;
        }
        let origin = area.min;
        let menu_at = bg.interact_pointer_pos().or(ui.ctx().pointer_hover_pos()).unwrap_or(area.center()) - origin - self.pan;
        bg.context_menu(|ui| self.add_menu(ui, menu_at.to_pos2()));

        let trail = self.selected.and_then(|s| self.graph.trail(s).ok()).unwrap_or_default();
        for w in self.graph.wires() {
            let kind = self.graph.nodes()[w.from].spec.outputs[w.from_port].kind;
            let a = self.port_pos(origin, w.from, w.from_port, true);
            let b = self.port_pos(origin, w.to, w.to_port, false);
            let dx = ((b.x - a.x).abs() * 0.5).max(40.0);
            let lit = trail.contains(&w.from) && trail.contains(&w.to);
            let col = if self.selected.is_some() && !lit { alpha(kind_color(kind), 0.2) } else { kind_color(kind) };
            painter.add(CubicBezierShape::from_points_stroke(
                [a, a + Vec2::new(dx, 0.0), b - Vec2::new(dx, 0.0), b],
                false,
                Color32::TRANSPARENT,
                Stroke::new(if lit { 2.6 } else { 1.7 }, col),
            ));
        }

        let mut clicked = None;
        let mut opened = None;
        let mut delete = None;
        let mut drop_on: Option<(NodeId, usize)> = None;
        let mut unwire: Option<(NodeId, &'static str)> = None;
        let pointer = ui.ctx().pointer_latest_pos();
        let released = ui.input(|i| i.pointer.any_released());
        for i in 0..self.nodes.len() {
            if self.nodes[i].hidden {
                continue;
            }
            let rect = self.node_rect(origin, i);
            let head = Rect::from_min_size(rect.min, Vec2::new(NODE_W, HEAD));
            let resp = ui.interact(rect, egui::Id::new(("node", i)), Sense::click_and_drag());
            if resp.dragged() && self.wiring.is_none() {
                self.nodes[i].pos += resp.drag_delta();
            }
            if resp.clicked() {
                clicked = Some(i);
            }
            if resp.double_clicked() {
                opened = Some(i);
            }
            resp.context_menu(|ui| {
                if ui.button("Open window").clicked() {
                    opened = Some(i);
                    ui.close();
                }
                if ui.button("Delete node").clicked() {
                    delete = Some(i);
                    ui.close();
                }
            });
            let spec = self.graph.nodes()[i].spec.clone();
            let is_sel = self.selected == Some(i);
            let in_trail = trail.contains(&i);
            let fade = self.selected.is_some() && !in_trail;
            painter.rect_filled(rect, 6.0, if fade { alpha(NODE, 0.6) } else { NODE });
            let border = if is_sel { SEL } else if in_trail { alpha(SEL, 0.6) } else { LINE };
            painter.rect_stroke(rect, 6.0, Stroke::new(if is_sel { 2.0 } else { 1.0 }, border), StrokeKind::Inside);
            painter.line_segment([Pos2::new(rect.left(), head.bottom()), Pos2::new(rect.right(), head.bottom())], Stroke::new(1.0, LINE));
            let tc = if fade { DIM } else { TEXT };
            let lab = &self.nodes[i].label;
            let lab = if lab.chars().count() > 21 { format!("{}…", lab.chars().take(20).collect::<String>()) } else { lab.clone() };
            painter.text(head.left_center() + Vec2::new(9.0, 0.0), Align2::LEFT_CENTER, lab, FontId::proportional(13.0), tc);
            painter.text(head.right_center() - Vec2::new(9.0, 0.0), Align2::RIGHT_CENTER, role_text(spec.role), FontId::proportional(10.0), DIM);
            // Status line at the bottom of the node.
            let (status, running) = {
                let s = self.nodes[i].state.lock().unwrap();
                let st = match &self.nodes[i].kind {
                    Kind::Plugin { .. } if !s.plugin.state.is_empty() => {
                        let first = s.plugin.status.lines().next().unwrap_or("");
                        if first.is_empty() { s.plugin.state.clone() } else { first.to_string() }
                    }
                    Kind::Manual { rows } => format!("{} facts", rows.iter().filter(|(_, v)| !v.trim().is_empty()).count()),
                    Kind::Record => self.saved_note.clone(),
                    _ => s.status.clone(),
                };
                (st, s.running)
            };
            let status = if status.chars().count() > 34 { format!("{}…", status.chars().take(33).collect::<String>()) } else { status };
            let sy = rect.bottom() - 12.0;
            if running {
                let on = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
                painter.circle_filled(Pos2::new(rect.left() + 12.0, sy), 3.5, if on { LIVE } else { alpha(LIVE, 0.3) });
            }
            painter.text(Pos2::new(rect.left() + 22.0, sy), Align2::LEFT_CENTER, status, FontId::proportional(10.5), DIM);

            let missing = self.graph.missing(i);
            for (k, p) in spec.inputs.iter().enumerate() {
                let c = self.port_pos(origin, i, k, false);
                let col = kind_color(p.kind);
                if p.required {
                    painter.circle_filled(c, 5.0, col);
                } else {
                    painter.circle_filled(c, 5.0, NODE);
                    painter.circle_stroke(c, 4.5, Stroke::new(1.6, col));
                }
                let nc = if missing.contains(&p.name) { WARN } else if p.required { tc } else { DIM };
                painter.text(c + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, p.name, FontId::proportional(11.5), nc);
                let pr = ui.interact(Rect::from_center_size(c, Vec2::splat(16.0)), egui::Id::new(("in", i, k)), Sense::click());
                if pr.secondary_clicked() {
                    unwire = Some((i, p.name));
                }
                if self.wiring.is_some() && released && pointer.is_some_and(|q| q.distance(c) < 12.0) {
                    drop_on = Some((i, k));
                }
            }
            for (k, p) in spec.outputs.iter().enumerate() {
                let c = self.port_pos(origin, i, k, true);
                painter.circle_filled(c, 5.0, kind_color(p.kind));
                painter.text(c - Vec2::new(10.0, 0.0), Align2::RIGHT_CENTER, p.name, FontId::proportional(11.5), tc);
                let pr = ui.interact(Rect::from_center_size(c, Vec2::splat(16.0)), egui::Id::new(("out", i, k)), Sense::drag());
                if pr.drag_started() {
                    self.wiring = Some((i, k));
                }
            }
        }

        // Wire being dragged.
        if let Some((n, k)) = self.wiring {
            let a = self.port_pos(origin, n, k, true);
            if let Some(b) = pointer {
                let kind = self.graph.nodes()[n].spec.outputs[k].kind;
                let dx = ((b.x - a.x).abs() * 0.5).max(40.0);
                painter.add(CubicBezierShape::from_points_stroke(
                    [a, a + Vec2::new(dx, 0.0), b - Vec2::new(dx, 0.0), b],
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(2.0, kind_color(kind)),
                ));
            }
            if released {
                if let Some((t, ti)) = drop_on {
                    let out = self.graph.nodes()[n].spec.outputs[k].name;
                    let inp = self.graph.nodes()[t].spec.inputs[ti].name;
                    match self.graph.connect(n, out, t, inp) {
                        Ok(()) => {
                            self.msg = format!("wired {}.{out} → {}.{inp}", self.nodes[n].label, self.nodes[t].label);
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
                self.save_project();
            }
        }
        if let Some(i) = delete {
            self.graph.isolate(i);
            self.nodes[i].hidden = true;
            self.open[i] = false;
            self.selected = None;
            self.save_project();
        }
        if let Some(i) = clicked {
            self.selected = Some(i);
        }
        if let Some(i) = opened {
            self.open[i] = true;
            self.selected = Some(i);
        }
        painter.text(
            area.left_bottom() + Vec2::new(10.0, -8.0),
            Align2::LEFT_BOTTOM,
            "drag header: move · drag empty: pan · drag output to input: wire · right-click input: unwire · right-click: add node / delete · double-click: open",
            FontId::proportional(11.0),
            DIM,
        );
    }

    // ---------------- timeline

    fn timeline_view(&mut self, ui: &mut egui::Ui) {
        struct Mark {
            t0: f64,
            t1: f64,
            err: f64,
            label: String,
            detail: String,
            col: Color32,
        }
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
            let err = a.as_ref().map_or(0.0, anchor_error_s);
            let to = |t: f64| a.as_ref().map_or(t, |a| a.utc(t));
            let mut marks = Vec::new();
            match &n.kind {
                Kind::Planes => {
                    for e in &s.passes {
                        marks.push(Mark {
                            t0: to(e.t_start),
                            t1: to(e.t_end),
                            err,
                            label: format!("{:.0} m/s", e.speed),
                            detail: format!(
                                "pass {}: closest approach {}, {:.0} m/s, {:.0} m, f0 {:.1} Hz, grade {}",
                                e.id,
                                a.as_ref().map_or(format!("{:.1} s", e.t_cpa), |a| format!("{} UTC", fmt_clock(a.utc(e.t_cpa)))),
                                e.speed,
                                e.dist,
                                e.f0,
                                e.grade()
                            ),
                            col: grade_color(e.grade()),
                        });
                    }
                }
                Kind::Enf => {
                    for g in &s.enf_segments {
                        marks.push(Mark {
                            t0: to(g.t_start),
                            t1: to(g.t_end),
                            err,
                            label: format!("{:.3} Hz", g.mean_hz),
                            detail: format!("hum {:.3} Hz ({:.3}–{:.3}), harmonic {}", g.mean_hz, g.min_hz, g.max_hz, g.harmonic),
                            col: Color32::from_rgb(110, 170, 255),
                        });
                    }
                }
                Kind::Plugin { .. } => {
                    let col = s.plugin.color.map_or(Color32::from_rgb(200, 200, 200), |[r, g, b]| Color32::from_rgb(r, g, b));
                    for f in &s.plugin.findings {
                        marks.push(Mark { t0: to(f.t_start), t1: to(f.t_end), err, label: f.label.clone(), detail: f.detail.clone(), col });
                    }
                }
                _ => continue,
            }
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

    // ---------------- node windows

    fn windows(&mut self, ctx: &egui::Context) {
        for i in 0..self.nodes.len() {
            if !self.open[i] || self.nodes[i].hidden {
                continue;
            }
            let mut open = true;
            let title = self.nodes[i].label.clone();
            let size = if matches!(self.nodes[i].kind, Kind::Planes) { [820.0, 470.0] } else { [460.0, 360.0] };
            egui::Window::new(&title).id(egui::Id::new(("win", i))).open(&mut open).default_size(size).show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| self.window_body(ui, i));
            });
            self.open[i] = open;
        }
    }

    fn window_body(&mut self, ui: &mut egui::Ui, i: NodeId) {
        let running = self.running();
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.nodes[i].label);
        });
        ui.separator();
        let state = self.nodes[i].state.clone();
        match &mut self.nodes[i].kind {
            Kind::File { path } => {
                ui.horizontal(|ui| {
                    ui.label("File");
                    ui.add(egui::TextEdit::singleline(path).desired_width(300.0));
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_file() {
                            *path = p.to_string_lossy().into_owned();
                        }
                    }
                });
                source_info(ui, &state);
            }
            Kind::Stream { url } => {
                ui.horizontal(|ui| {
                    ui.label("URL");
                    ui.add(egui::TextEdit::singleline(url).desired_width(340.0).hint_text("https://… (YouTube links are resolved with yt-dlp)"));
                });
                source_info(ui, &state);
            }
            Kind::Mic { device } => {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("mic", i)).selected_text(if device.is_empty() { "choose a microphone" } else { device.as_str() }).show_ui(ui, |ui| {
                        for m in &self.mics {
                            ui.selectable_value(device, m.clone(), m);
                        }
                    });
                    if ui.button("Refresh").clicked() {
                        match doppler_live::ingest::list_microphones() {
                            Ok(m) => self.mics = m,
                            Err(e) => self.msg = e,
                        }
                    }
                });
                source_info(ui, &state);
            }
            Kind::Manual { rows } => {
                ui.colored_label(DIM, "Facts you know. Wired into a plugin, they are passed to it at Run (coordinates as lat, lon; elevation in m).");
                let mut remove = None;
                egui::Grid::new(("manual", i)).num_columns(3).show(ui, |ui| {
                    for (r, (k, v)) in rows.iter_mut().enumerate() {
                        ui.add(egui::TextEdit::singleline(k).desired_width(110.0));
                        ui.add(egui::TextEdit::singleline(v).desired_width(240.0));
                        if ui.small_button("✕").clicked() {
                            remove = Some(r);
                        }
                        ui.end_row();
                    }
                });
                if let Some(r) = remove {
                    rows.remove(r);
                }
                if ui.button("+ field").clicked() {
                    rows.push((String::new(), String::new()));
                }
            }
            Kind::Planes => self.planes_window(ui, i),
            Kind::Enf => enf_window(ui, &state),
            Kind::Plugin { path } => {
                ui.colored_label(DIM, format!("program: {path}"));
                plugin_window(ui, &state);
            }
            Kind::Record => {
                ui.label("Keeps one result JSON for the current run: every node's findings and facts, and the wires (the evidence trail). Saved every 30 s while running and when the run ends.");
                match &self.result_file {
                    Some(p) => {
                        ui.colored_label(TEXT, p.display().to_string());
                        ui.colored_label(DIM, &self.saved_note);
                    }
                    None => {
                        ui.colored_label(DIM, "not running yet");
                    }
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.result_file.is_some(), egui::Button::new("Save now")).clicked() {
                        self.save_result();
                    }
                    if ui.button("Open folder").clicked() {
                        let d = results_dir();
                        let _ = std::fs::create_dir_all(&d);
                        let _ = std::process::Command::new("explorer").arg(d).spawn();
                    }
                });
            }
        }
        if running && self.nodes[i].kind.is_source() {
            ui.colored_label(DIM, "Changes to sources take effect at the next Run.");
        }
    }

    fn planes_window(&mut self, ui: &mut egui::Ui, i: NodeId) {
        let st = self.nodes[i].state.clone();
        // Plugin tracks on the same source, drawn over the spectrogram (adsb-check's predicted curves).
        let key = st.lock().unwrap().source_key.clone();
        let overlays: Vec<(Color32, Vec<(String, Vec<(f64, f64, bool)>)>)> = self
            .nodes
            .iter()
            .filter(|n| !n.hidden && matches!(n.kind, Kind::Plugin { .. }))
            .filter_map(|n| {
                let s = n.state.lock().unwrap();
                (s.source_key == key && !key.is_empty() && !s.plugin.tracks.is_empty()).then(|| {
                    let c = s.plugin.color.map_or(Color32::WHITE, |[r, g, b]| Color32::from_rgb(r, g, b));
                    (c, s.plugin.tracks.clone())
                })
            })
            .collect();
        let s = st.lock().unwrap();
        ui.horizontal(|ui| {
            ui.label(format!("{} passes · {} candidates · {:.0} s analysed", s.passes.len(), s.pending.len(), s.audio_s));
            ui.separator();
            ui.label("show");
            ui.add(egui::DragValue::new(&mut self.spec_minutes).range(0.0..=600.0).suffix(" min").speed(0.5));
            ui.colored_label(DIM, "(0 = all)");
        });
        let nb = s.freqs.len();
        let nf = s.hist_t.len();
        if nb == 0 || nf == 0 {
            ui.colored_label(DIM, "no audio yet: press Run");
            return;
        }
        let t_end = *s.hist_t.last().unwrap();
        let t_start = if self.spec_minutes > 0.0 { (t_end - self.spec_minutes * 60.0).max(s.hist_t[0]) } else { s.hist_t[0] };
        let f_first = s.hist_t.partition_point(|&t| t < t_start);
        let cols_n = nf - f_first;
        let w = cols_n.clamp(1, 2048);
        // Rebuild the texture when new frames arrived or the span changed.
        let stamp = (s.frames, t_start);
        let fresh = self.textures.get(&i).is_none_or(|(f, ts, _)| (*f, *ts) != stamp);
        if fresh {
            let mut px = vec![0u8; w * nb * 4];
            for c in 0..w {
                let a = f_first + c * cols_n / w;
                let b = (f_first + (c + 1) * cols_n / w).max(a + 1).min(nf);
                for bin in 0..nb {
                    let mut m = 0u8;
                    for f in a..b {
                        m = m.max(s.hist[f * nb + bin]);
                    }
                    let row = nb - 1 - bin;
                    px[(row * w + c) * 4..(row * w + c) * 4 + 4].copy_from_slice(&self.lut[m as usize]);
                }
            }
            let img = ColorImage::from_rgba_unmultiplied([w, nb], &px);
            match self.textures.get_mut(&i) {
                Some((f, ts, tex)) => {
                    tex.set(img, TextureOptions::LINEAR);
                    *f = stamp.0;
                    *ts = stamp.1;
                }
                None => {
                    let tex = ui.ctx().load_texture(format!("spec{i}"), img, TextureOptions::LINEAR);
                    self.textures.insert(i, (stamp.0, stamp.1, tex));
                }
            }
        }
        let tex = self.textures[&i].2.id();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 360.0), Sense::hover());
        let plot = Rect::from_min_max(rect.min + Vec2::new(44.0, 0.0), rect.max - Vec2::new(0.0, 18.0));
        let p = ui.painter_at(rect);
        p.image(tex, plot, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let (f_lo, f_hi) = (s.freqs[0], s.freqs[nb - 1]);
        let x_of = |t: f64| plot.left() + ((t - t_start) / (t_end - t_start).max(1e-6)) as f32 * plot.width();
        let y_of = |f: f64| plot.bottom() - ((f - f_lo) / (f_hi - f_lo)) as f32 * plot.height();
        for k in 0..=4 {
            let f = f_lo + (f_hi - f_lo) * k as f64 / 4.0;
            p.text(Pos2::new(plot.left() - 4.0, y_of(f)), Align2::RIGHT_CENTER, format!("{f:.0}"), FontId::proportional(10.0), DIM);
        }
        p.text(Pos2::new(rect.left(), plot.top()), Align2::LEFT_TOP, "Hz", FontId::proportional(10.0), DIM);
        let a = s.anchor.clone();
        let tlabel = |t: f64| a.as_ref().map_or(format!("{t:.0} s"), |a| format!("{} UTC", fmt_clock(a.utc(t))));
        p.text(Pos2::new(plot.left(), rect.bottom()), Align2::LEFT_BOTTOM, tlabel(t_start), FontId::proportional(10.0), DIM);
        p.text(Pos2::new(plot.right(), rect.bottom()), Align2::RIGHT_BOTTOM, tlabel(t_end), FontId::proportional(10.0), DIM);
        let clip = p.with_clip_rect(plot);
        // Plugin tracks (for adsb-check: the Doppler curve the flight data predicts).
        for (c, tracks) in &overlays {
            for (_, pts) in tracks {
                for &(t, hz, ok) in pts {
                    if t >= t_start && (f_lo..=f_hi).contains(&hz) {
                        clip.circle_filled(Pos2::new(x_of(t), y_of(hz)), 1.4, alpha(*c, if ok { 0.9 } else { 0.4 }));
                    }
                }
            }
        }
        let mut hover = None;
        for (e, pending) in s.passes.iter().map(|e| (e, false)).chain(s.pending.iter().map(|e| (e, true))) {
            if e.t_end < t_start {
                continue;
            }
            let col = if pending { alpha(TEXT, 0.5) } else { grade_color(e.grade()) };
            let pts: Vec<Pos2> = e.points.iter().map(|&(t, f)| Pos2::new(x_of(t), y_of(f))).collect();
            for w in pts.windows(2) {
                clip.line_segment([w[0], w[1]], Stroke::new(1.6, col));
            }
            if let Some(first) = pts.first() {
                clip.text(*first + Vec2::new(0.0, -6.0), Align2::LEFT_BOTTOM, format!("{:.0} m/s · {:.0} m", e.speed, e.dist), FontId::proportional(10.0), col);
                if resp.hover_pos().is_some_and(|h| pts.iter().any(|q| q.distance(h) < 6.0)) {
                    hover = Some(format!(
                        "pass {}{}\nclosest approach {}\nspeed {:.0} m/s, distance {:.0} m, f0 {:.1} Hz\nSNR {:.1}, fit rms {:.2} Hz, grade {}",
                        e.id,
                        if pending { " (candidate)" } else { "" },
                        tlabel(e.t_cpa),
                        e.speed,
                        e.dist,
                        e.f0,
                        e.snr,
                        e.rms,
                        e.grade()
                    ));
                }
            }
        }
        drop(s);
        if let Some(t) = hover {
            resp.on_hover_text(t);
        }
        ui.colored_label(DIM, "green/orange/grey: passes by grade · white: candidates · dots: plugin tracks (adsb-check: predicted curves from flight data)");
    }

    // ---------------- frame

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("CORROBORATE").strong().color(TEXT));
            ui.separator();
            if self.mode == Mode::Work {
                let running = self.running();
                if running {
                    if ui.button(egui::RichText::new("■ Stop").color(LIVE)).clicked() {
                        self.stop_all();
                    }
                } else if ui.button(egui::RichText::new("▶ Run").color(GOOD)).clicked() {
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
            ui.separator();
            let label = if self.mode == Mode::Demo { "◀ Back to workspace" } else { "Demo: protest scenario" };
            if ui.button(label).clicked() {
                self.mode = if self.mode == Mode::Demo { Mode::Work } else { Mode::Demo };
                if self.demo.is_none() {
                    self.demo = Some(demo::Demo::new());
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

fn source_info(ui: &mut egui::Ui, st: &Shared) {
    let s = st.lock().unwrap();
    ui.label(format!("status: {}", s.status));
    if s.audio_s > 0.0 {
        ui.label(format!("audio: {:.0} s{}", s.audio_s, if s.live { " (live)" } else { "" }));
    }
    match &s.anchor {
        Some(a) => {
            ui.label(format!("time 0 = {} UTC", timebase::fmt_utc(a.utc0)));
            ui.colored_label(DIM, format!("from: {} · estimated error ± {:.0} s", a.source, anchor_error_s(a)));
        }
        None => {
            ui.colored_label(DIM, "no UTC anchor yet");
        }
    }
}

fn plot_series(ui: &mut egui::Ui, series: &[(Color32, Vec<(f64, f64, bool)>)], height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, BG);
    let pts = series.iter().flat_map(|(_, v)| v.iter());
    let (mut t0, mut t1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &(t, y, _) in pts {
        t0 = t0.min(t);
        t1 = t1.max(t);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    if t0 >= t1 {
        p.text(rect.center(), Align2::CENTER_CENTER, "no readings yet", FontId::proportional(11.0), DIM);
        return;
    }
    if y1 - y0 < 1e-3 {
        y0 -= 0.05;
        y1 += 0.05;
    }
    let plot = rect.shrink2(Vec2::new(50.0, 10.0));
    let x = |t: f64| plot.left() + ((t - t0) / (t1 - t0)) as f32 * plot.width();
    let y = |v: f64| plot.bottom() - ((v - y0) / (y1 - y0)) as f32 * plot.height();
    for (c, v) in series {
        for &(t, val, ok) in v {
            p.circle_filled(Pos2::new(x(t), y(val)), 1.5, alpha(*c, if ok { 0.95 } else { 0.3 }));
        }
    }
    p.text(Pos2::new(rect.left() + 4.0, plot.top()), Align2::LEFT_TOP, format!("{y1:.3}"), FontId::proportional(10.0), DIM);
    p.text(Pos2::new(rect.left() + 4.0, plot.bottom()), Align2::LEFT_BOTTOM, format!("{y0:.3}"), FontId::proportional(10.0), DIM);
    p.text(Pos2::new(plot.left(), rect.bottom()), Align2::LEFT_BOTTOM, format!("{t0:.0} s"), FontId::proportional(10.0), DIM);
    p.text(Pos2::new(plot.right(), rect.bottom()), Align2::RIGHT_BOTTOM, format!("{t1:.0} s"), FontId::proportional(10.0), DIM);
}

fn enf_window(ui: &mut egui::Ui, st: &Shared) {
    let s = st.lock().unwrap();
    ui.label(format!("status: {}", s.status));
    let pts: Vec<(f64, f64, bool)> = s.enf_points.iter().map(|p| (p.t, p.hz / p.harmonic as f64, p.ok)).collect();
    drop(s);
    ui.colored_label(DIM, "grid frequency (Hz, folded to the fundamental) over source time; faded = unclear reading");
    plot_series(ui, &[(Color32::from_rgb(110, 170, 255), pts)], 220.0);
    let s = st.lock().unwrap();
    for g in s.enf_segments.iter().rev().take(20) {
        ui.label(format!("{:.0}–{:.0} s · {:.3} Hz ({:.3}–{:.3}) · h{}", g.t_start, g.t_end, g.mean_hz, g.min_hz, g.max_hz, g.harmonic));
    }
}

fn plugin_window(ui: &mut egui::Ui, st: &Shared) {
    let s = st.lock().unwrap();
    let pl = s.plugin.clone();
    drop(s);
    ui.label(format!("{} · {}", if pl.name.is_empty() { "plugin" } else { &pl.name }, pl.state));
    if !pl.about.is_empty() {
        ui.colored_label(DIM, &pl.about);
    }
    if pl.dropped_s > 0.0 {
        ui.colored_label(WARN, format!("missed {:.0} s of live audio (could not keep up)", pl.dropped_s));
    }
    if !pl.status.is_empty() {
        ui.separator();
        for line in pl.status.lines() {
            ui.label(line);
        }
    }
    if !pl.log.is_empty() {
        ui.colored_label(DIM, format!("log: {}", pl.log));
    }
    if !pl.tracks.is_empty() {
        let c = pl.color.map_or(Color32::WHITE, |[r, g, b]| Color32::from_rgb(r, g, b));
        let all: Vec<(f64, f64, bool)> = pl.tracks.iter().flat_map(|(_, v)| v.iter().copied()).collect();
        ui.colored_label(DIM, format!("{} tracks (also drawn on the Planes spectrogram of the same source)", pl.tracks.len()));
        plot_series(ui, &[(c, all)], 140.0);
    }
    if !pl.findings.is_empty() {
        ui.separator();
        ui.label(egui::RichText::new(format!("FINDINGS ({})", pl.findings.len())).small().color(DIM));
        for f in pl.findings.iter().rev().take(40) {
            ui.label(format!("{:.0}–{:.0} s · {}", f.t_start, f.t_end, f.label)).on_hover_text(&f.detail);
        }
    }
    if !pl.meta.is_empty() {
        ui.separator();
        ui.label(egui::RichText::new("FACTS").small().color(DIM));
        egui::Grid::new(ui.next_auto_id()).num_columns(2).striped(true).show(ui, |ui| {
            for (k, v) in &pl.meta {
                ui.colored_label(DIM, k);
                ui.label(v);
                ui.end_row();
            }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("bar").frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(10, 6))).show(ui, |ui| {
            self.top_bar(ui);
        });
        if self.mode == Mode::Demo {
            if let Some(d) = self.demo.as_mut() {
                d.show(ui);
            }
            return;
        }
        egui::Panel::bottom("timeline").frame(egui::Frame::new().fill(PANEL).inner_margin(0.0)).show(ui, |ui| {
            self.timeline_view(ui);
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ui, |ui| {
            self.graph_view(ui);
        });
        let ctx = ui.ctx().clone();
        self.windows(&ctx);

        let running = self.running();
        if self.result_file.is_some() && self.last_save.elapsed() > Duration::from_secs(if running { 30 } else { 3600 * 24 }) {
            self.save_result();
        }
        // Save once more right after the run ends.
        if !running && self.stop.is_some() {
            self.stop = None;
            self.save_result();
        }
        if self.last_project_save.elapsed() > Duration::from_secs(10) {
            self.save_project();
        }
        ctx.request_repaint_after(Duration::from_millis(if running { 200 } else { 1000 }));
    }
}

fn main() -> eframe::Result {
    // corroborate [path or URL] [--run]: --run starts the saved project at once (unattended runs).
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
