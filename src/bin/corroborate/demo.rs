//! Demo:  a node graph, a timeline with lanes and a map,
//! running the engine (`graph`, `sync`, `locate`, `timeline`) on a synthetic protest scenario.
//!
//! - Graph: drag nodes by their header, drag empty space to pan. Click a node to select it
//!   and highlight its evidence trail; double-click to open its window.
//! - Timeline: wheel to zoom, drag to pan. Blocks are drawn time ± error; strong colour means
//!   a narrow margin. Items on a stream's own clock (offset unknown) are faded and outlined.
//! - Every number shown says where it came from. The scenario is synthetic and says so.

use corroborate::core::{Location, Method, NodeSpec, Port, PortKind, Role, Source, SourceKind, TimeRef};
use corroborate::graph::{Graph, NodeId};
use corroborate::locate::{locate, Arrival, Fix, C_SOUND};
use corroborate::sync::{solve, Obs, Offset, Solution};
use corroborate::timeline::{Item, Timeline};
use eframe::egui::{
    self, epaint::CubicBezierShape, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

// Colours, port colours and role names are the workbench's own (main.rs).
use super::{alpha as with_alpha, kind_color, role_text, BG, DIM, LINE, NODE, PANEL, SEL, TEXT, WARN};

// ---------- synthetic scenario ----------

/// Place de la République, Paris: the scenario's origin.
const LAT0: f64 = 48.8676;
const LON0: f64 = 2.3631;
const KY: f64 = 6_371_000.0 * std::f64::consts::PI / 180.0;

fn kx() -> f64 {
    KY * LAT0.to_radians().cos()
}
fn place(x: f64, y: f64, r: f64) -> Location {
    Location::new(LAT0 + y / KY, LON0 + x / kx(), r, Method::Gps).expect("valid scenario place")
}
fn xy(l: &Location) -> (f64, f64) {
    ((l.lon - LON0) * kx(), (l.lat - LAT0) * KY)
}

const NAMES: [&str; 5] = ["Stream A", "Stream B", "Stream C", "Stream D", "Stream E"];
/// Recorder positions (m east, m north). Stream E has no position and no shared flashes.
const RECS: [(f64, f64); 4] = [(0.0, 0.0), (600.0, 50.0), (550.0, 700.0), (-50.0, 650.0)];
/// True clock offsets: unknown to the solver, used only to make the synthetic data.
const TRUE_OFFSETS: [f64; 5] = [0.0, 37.25, -12.5, 80.0, 23.0];
const FLASHES: [f64; 3] = [100.0, 250.0, 420.0];
const FLASH_ERR: f64 = 1.0 / 30.0; // one video frame
const BANG: (f64, f64) = (220.0, 410.0);
const T_BANG: f64 = 500.0;
const ARRIVAL_ERR: f64 = 0.002; // onset picking in audio

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

struct Scenario {
    graph: Graph,
    sol: Solution,
    fix: Option<Fix>,
    arrivals: Vec<Arrival>,
    timeline: Timeline,
    /// Index of the node whose window shows the map.
    locate_node: NodeId,
    sync_node: NodeId,
}

fn spec(name: &'static str, role: Role, inputs: Vec<Port>, outputs: Vec<Port>) -> NodeSpec {
    NodeSpec { name, role, inputs, outputs }
}

fn build_scenario() -> Scenario {
    use PortKind::*;
    let mut g = Graph::new();
    let mut streams = Vec::new();
    for n in NAMES {
        streams.push(g.add(
            n,
            spec("stream", Role::Source(SourceKind::Live), vec![], vec![
                Port::req("video", Video),
                Port::req("audio", Audio),
                Port::req("place", Location),
            ]),
        ));
    }
    let flash = g.add("Flash (video)", spec("flash", Role::Detector, vec![
        Port::req("a", Video), Port::opt("b", Video), Port::opt("c", Video), Port::opt("d", Video), Port::opt("e", Video),
    ], vec![Port::req("flashes", Event)]));
    let bang = g.add("Bang onset (audio)", spec("bang", Role::Detector, vec![
        Port::req("a", Audio), Port::opt("b", Audio), Port::opt("c", Audio), Port::opt("d", Audio), Port::opt("e", Audio),
    ], vec![Port::req("onsets", Event)]));
    let sync = g.add("Sync solver", spec("sync", Role::Solver, vec![
        Port::req("shared", Event), Port::opt("anchor", Time),
    ], vec![Port::req("offsets", Time)]));
    let loc = g.add("Locate (TDOA)", spec("locate", Role::Solver, vec![
        Port::req("onsets", Event), Port::req("offsets", Time),
        Port::req("a", Location), Port::opt("b", Location), Port::opt("c", Location), Port::opt("d", Location),
    ], vec![Port::req("source", Location), Port::req("time", Time)]));
    let rec = g.add("Result record", spec("record", Role::Record, vec![
        Port::req("location", Location), Port::opt("time", Time),
    ], vec![]));
    let ins = ["a", "b", "c", "d", "e"];
    for (i, &s) in streams.iter().enumerate() {
        g.connect(s, "video", flash, ins[i]).expect("wire");
        g.connect(s, "audio", bang, ins[i]).expect("wire");
        if i < 4 {
            g.connect(s, "place", loc, ins[i]).expect("wire");
        }
    }
    g.connect(flash, "flashes", sync, "shared").expect("wire");
    g.connect(bang, "onsets", loc, "onsets").expect("wire");
    g.connect(sync, "offsets", loc, "offsets").expect("wire");
    g.connect(loc, "source", rec, "location").expect("wire");
    g.connect(loc, "time", rec, "time").expect("wire");

    // Flashes seen by streams A-D on their own clocks.
    let mut obs = Vec::new();
    for (k, &t) in FLASHES.iter().enumerate() {
        for i in 0..4 {
            obs.push(Obs { stream: i, event: k, t: t - TRUE_OFFSETS[i], error_s: FLASH_ERR });
        }
    }
    let sol = solve(5, &obs, &[]);

    // Bang onsets on each stream's own clock, then on the shared clock where synced.
    let local = |i: usize| -> f64 {
        let d = if i < 4 { dist(BANG, RECS[i]) } else { 380.0 };
        T_BANG + d / C_SOUND - TRUE_OFFSETS[i]
    };
    let mut arrivals = Vec::new();
    for (i, &r) in RECS.iter().enumerate() {
        if let Offset::Solved { offset, error_s } = sol.offsets[i] {
            arrivals.push(Arrival {
                at: place(r.0, r.1, 5.0),
                t: offset + local(i),
                error_s: (ARRIVAL_ERR.powi(2) + error_s.powi(2)).sqrt(),
            });
        }
    }
    let fix = locate(&arrivals, C_SOUND).ok();

    let mut tl = Timeline::new();
    let lf = tl.add_lane("Flashes (video)");
    let lb = tl.add_lane("Bang onsets (audio)");
    let ls = tl.add_lane("Bang source (TDOA)");
    for (o, ob) in obs.iter().enumerate() {
        if let Offset::Solved { offset, error_s } = sol.offsets[ob.stream] {
            tl.push(lf, Item {
                label: format!("{} · flash {}", NAMES[ob.stream], ob.event + 1),
                time: TimeRef::Absolute { utc: offset + ob.t, error_s: (ob.error_s.powi(2) + error_s.powi(2)).sqrt() },
                source: Source::new("Flash (video)", format!("observation {o}, frame accuracy 1/30 s")),
            });
        }
    }
    for i in 0..5 {
        match sol.offsets[i] {
            Offset::Solved { offset, error_s } => tl.push(lb, Item {
                label: NAMES[i].to_string(),
                time: TimeRef::Absolute { utc: offset + local(i), error_s: (ARRIVAL_ERR.powi(2) + error_s.powi(2)).sqrt() },
                source: Source::new("Bang onset (audio)", "onset ± 2 ms, plus sync error"),
            }),
            Offset::Unsolved => tl.push(lb, Item {
                label: format!("{} · own clock, offset unknown", NAMES[i]),
                time: TimeRef::Relative { t: local(i), clock: NAMES[i].into(), error_s: 30.0 },
                source: Source::new("Bang onset (audio)", "no shared flash: cannot be synced"),
            }),
        };
    }
    if let Some(f) = &fix {
        tl.push(ls, Item {
            label: "bang made".into(),
            time: TimeRef::Absolute { utc: f.t0, error_s: f.t0_error_s },
            source: Source::new("Locate (TDOA)", "fit of 4 arrivals"),
        });
    }
    Scenario { graph: g, sol, fix, arrivals, timeline: tl, locate_node: loc, sync_node: sync }
}

// ---------- app ----------

pub struct Demo {
    sc: Scenario,
    pos: Vec<Pos2>,
    pan: Vec2,
    selected: Option<NodeId>,
    open: Vec<bool>,
    /// Timeline view, seconds on the shared clock (stream A's clock).
    t0: f64,
    t1: f64,
}

const NODE_W: f32 = 200.0;
const HEAD: f32 = 26.0;
const ROW: f32 = 20.0;

impl Demo {
    pub fn new() -> Self {
        let sc = build_scenario();
        let mut pos = Vec::new();
        for i in 0..sc.graph.nodes().len() {
            let p = match i {
                0..=4 => Pos2::new(20.0, 20.0 + i as f32 * 104.0),
                5 => Pos2::new(290.0, 40.0),
                6 => Pos2::new(290.0, 300.0),
                7 => Pos2::new(560.0, 60.0),
                8 => Pos2::new(560.0, 250.0),
                _ => Pos2::new(830.0, 280.0),
            };
            pos.push(p);
        }
        let n = sc.graph.nodes().len();
        let (a, b) = sc.timeline.span_utc().unwrap_or((0.0, 600.0));
        let pad = (b - a) * 0.08;
        // Room on the right for labels of items near the end.
        Self { sc, pos, pan: Vec2::ZERO, selected: None, open: vec![false; n], t0: a - pad, t1: b + 3.0 * pad }
    }

    fn node_rect(&self, origin: Pos2, i: NodeId) -> Rect {
        let n = &self.sc.graph.nodes()[i];
        let rows = n.spec.inputs.len().max(n.spec.outputs.len()) as f32;
        Rect::from_min_size(origin + self.pan + self.pos[i].to_vec2(), Vec2::new(NODE_W, HEAD + rows * ROW + 8.0))
    }

    fn port_pos(&self, origin: Pos2, i: NodeId, idx: usize, out: bool) -> Pos2 {
        let r = self.node_rect(origin, i);
        let y = r.top() + HEAD + 4.0 + idx as f32 * ROW + ROW / 2.0;
        Pos2::new(if out { r.right() } else { r.left() }, y)
    }

    fn graph_view(&mut self, ui: &mut egui::Ui) {
        let (area, bg) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, BG);
        // Dot grid that moves with the pan.
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
        if bg.dragged() {
            self.pan += bg.drag_delta();
        }
        if bg.clicked() {
            self.selected = None;
        }
        let origin = area.min;
        let trail = self.selected.and_then(|s| self.sc.graph.trail(s).ok()).unwrap_or_default();

        // Wires first, under the nodes.
        for w in self.sc.graph.wires() {
            let from = &self.sc.graph.nodes()[w.from];
            let to = &self.sc.graph.nodes()[w.to];
            let kind = from.spec.outputs[w.from_port].kind;
            let a = self.port_pos(origin, w.from, w.from_port, true);
            let b = self.port_pos(origin, w.to, w.to_port, false);
            let dx = ((b.x - a.x).abs() * 0.5).max(40.0);
            let lit = trail.contains(&w.from) && trail.contains(&w.to);
            let col = if self.selected.is_some() && !lit { with_alpha(kind_color(kind), 0.18) } else { kind_color(kind) };
            let width = if lit { 2.6 } else { 1.6 };
            let optional = !to.spec.inputs[w.to_port].required;
            let shape = CubicBezierShape::from_points_stroke(
                [a, a + Vec2::new(dx, 0.0), b - Vec2::new(dx, 0.0), b],
                false,
                Color32::TRANSPARENT,
                Stroke::new(width, if optional { with_alpha(col, col.a() as f32 / 255.0 * 0.6) } else { col }),
            );
            painter.add(shape);
        }

        // Nodes.
        let mut clicked = None;
        let mut opened = None;
        for i in 0..self.sc.graph.nodes().len() {
            let rect = self.node_rect(origin, i);
            let head = Rect::from_min_size(rect.min, Vec2::new(NODE_W, HEAD));
            let resp = ui.interact(rect, egui::Id::new(("node", i)), Sense::click_and_drag());
            if resp.dragged() && resp.interact_pointer_pos().is_some_and(|p| head.contains(p - resp.drag_delta())) {
                self.pos[i] += resp.drag_delta();
            }
            if resp.clicked() {
                clicked = Some(i);
            }
            if resp.double_clicked() {
                opened = Some(i);
            }
            let node = &self.sc.graph.nodes()[i];
            let is_sel = self.selected == Some(i);
            let in_trail = trail.contains(&i);
            let border = if is_sel { SEL } else if in_trail { with_alpha(SEL, 0.6) } else { LINE };
            let fade = self.selected.is_some() && !in_trail;
            painter.rect_filled(rect, 6.0, if fade { with_alpha(NODE, 0.55) } else { NODE });
            painter.rect_stroke(rect, 6.0, Stroke::new(if is_sel { 2.0 } else { 1.0 }, border), StrokeKind::Inside);
            painter.line_segment([Pos2::new(rect.left(), head.bottom()), Pos2::new(rect.right(), head.bottom())], Stroke::new(1.0, LINE));
            let tc = if fade { DIM } else { TEXT };
            painter.text(head.left_center() + Vec2::new(9.0, 0.0), Align2::LEFT_CENTER, &node.label, FontId::proportional(13.0), tc);
            painter.text(head.right_center() - Vec2::new(9.0, 0.0), Align2::RIGHT_CENTER, role_text(node.spec.role), FontId::proportional(10.0), DIM);
            let missing = self.sc.graph.missing(i);
            for (k, p) in node.spec.inputs.iter().enumerate() {
                let c = self.port_pos(origin, i, k, false);
                let col = kind_color(p.kind);
                if p.required {
                    painter.circle_filled(c, 5.0, col);
                } else {
                    painter.circle_filled(c, 5.0, NODE);
                    painter.circle_stroke(c, 4.5, Stroke::new(1.6, col));
                }
                let name_col = if missing.contains(&p.name) { WARN } else if p.required { tc } else { DIM };
                painter.text(c + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, p.name, FontId::proportional(11.5), name_col);
            }
            for (k, p) in node.spec.outputs.iter().enumerate() {
                let c = self.port_pos(origin, i, k, true);
                painter.circle_filled(c, 5.0, kind_color(p.kind));
                painter.text(c - Vec2::new(10.0, 0.0), Align2::RIGHT_CENTER, p.name, FontId::proportional(11.5), tc);
            }
        }
        if let Some(i) = clicked {
            self.selected = Some(i);
        }
        if let Some(i) = opened {
            self.open[i] = true;
            self.selected = Some(i);
        }
        painter.text(area.left_bottom() + Vec2::new(10.0, -8.0), Align2::LEFT_BOTTOM,
            "drag header: move node · drag empty: pan · click: select and show trail · double-click: open window",
            FontId::proportional(11.0), DIM);
    }

    fn timeline_view(&mut self, ui: &mut egui::Ui) {
        let lanes = self.sc.timeline.lanes().to_vec();
        let label_w = 170.0;
        let ruler_h = 22.0;
        let lane_h = 38.0;
        let (area, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ruler_h + lane_h * lanes.len() as f32 + 6.0), Sense::click_and_drag());
        let p = ui.painter_at(area);
        p.rect_filled(area, 0.0, PANEL);
        let track = Rect::from_min_max(Pos2::new(area.left() + label_w, area.top()), area.max);
        let span = (self.t1 - self.t0).max(1e-3);
        // Zoom with the wheel around the pointer; drag to pan.
        if let Some(hp) = resp.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
            if scroll != 0.0 && track.contains(hp) {
                let f = (-scroll as f64 * 0.002).exp();
                let at = self.t0 + ((hp.x - track.left()) / track.width()) as f64 * span;
                self.t0 = at - (at - self.t0) * f;
                self.t1 = at + (self.t1 - at) * f;
            }
        }
        if resp.dragged() {
            let dt = -(resp.drag_delta().x / track.width()) as f64 * span;
            self.t0 += dt;
            self.t1 += dt;
        }
        let (v0, span) = (self.t0, (self.t1 - self.t0).max(1e-3));
        let tx = |t: f64| track.left() + ((t - v0) / span) as f32 * track.width();

        // Ruler.
        let raw = span / 8.0;
        let mag = 10f64.powf(raw.log10().floor());
        let step = [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
        let mut t = (self.t0 / step).ceil() * step;
        while t <= self.t1 {
            let x = tx(t);
            p.line_segment([Pos2::new(x, track.top()), Pos2::new(x, track.bottom())], Stroke::new(1.0, Color32::from_rgb(40, 44, 49)));
            let txt = if step < 1.0 { format!("{t:.2} s") } else { format!("{t:.0} s") };
            p.text(Pos2::new(x + 3.0, track.top() + 4.0), Align2::LEFT_TOP, txt, FontId::proportional(10.5), DIM);
            t += step;
        }
        p.line_segment([Pos2::new(area.left(), area.top() + ruler_h), Pos2::new(area.right(), area.top() + ruler_h)], Stroke::new(1.0, LINE));
        p.text(Pos2::new(area.left() + 8.0, area.top() + 4.0), Align2::LEFT_TOP, "shared clock = Stream A", FontId::proportional(10.5), DIM);

        let colors = [Color32::from_rgb(224, 200, 90), Color32::from_rgb(224, 164, 88), Color32::from_rgb(224, 122, 106)];
        let mut hover_text = None;
        for (li, lane) in lanes.iter().enumerate() {
            let top = area.top() + ruler_h + li as f32 * lane_h;
            let row = Rect::from_min_max(Pos2::new(area.left(), top), Pos2::new(area.right(), top + lane_h));
            p.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, Color32::from_rgb(42, 46, 51)));
            p.text(Pos2::new(area.left() + 8.0, row.center().y), Align2::LEFT_CENTER, &lane.name, FontId::proportional(12.0), TEXT);
            let base = colors[li % colors.len()];
            let n = lane.items.len().max(1) as f32;
            for (k, it) in lane.items.iter().enumerate() {
                // Stack items of one lane in thin sub-rows so overlapping ones stay readable.
                let sub_h = ((lane_h - 8.0) / n).clamp(3.0, 14.0);
                let y0 = top + 4.0 + (k as f32 * sub_h) % (lane_h - 8.0 - sub_h + 0.01);
                let (centre, err, relative) = match &it.time {
                    TimeRef::Absolute { utc, error_s } => (*utc, *error_s, false),
                    TimeRef::Relative { t, error_s, .. } => (*t, *error_s, true),
                };
                let (xa, xb) = (tx(centre - err).max(track.left()), tx(centre + err).min(track.right()));
                let xc = tx(centre);
                if xb < track.left() || xa > track.right() {
                    continue;
                }
                let r = Rect::from_min_max(Pos2::new(xa, y0), Pos2::new(xb.max(xa + 3.0), y0 + sub_h - 1.0));
                if relative {
                    p.rect_filled(r, 2.0, with_alpha(DIM, 0.12));
                    p.rect_stroke(r, 2.0, Stroke::new(1.0, with_alpha(DIM, 0.5)), StrokeKind::Inside);
                    p.text(Pos2::new(xc + 4.0, r.center().y), Align2::LEFT_CENTER, &it.label, FontId::proportional(10.0), DIM);
                } else {
                    p.rect_filled(r, 2.0, with_alpha(base, 0.15 + 0.6 * it.strength() as f32));
                }
                if (track.left()..=track.right()).contains(&xc) {
                    p.line_segment([Pos2::new(xc, r.top()), Pos2::new(xc, r.bottom())], Stroke::new(1.5, if relative { DIM } else { base }));
                }
                if resp.hover_pos().is_some_and(|h| r.expand2(Vec2::new(3.0, 0.0)).contains(h)) {
                    let when = if relative { format!("{centre:.3} s on {}'s own clock", it.label.split(" ·").next().unwrap_or("?")) } else { format!("{centre:.3} s") };
                    hover_text = Some(format!("{}\n{when} ± {err:.3} s\nsource: {} — {}", it.label, it.source.node, it.source.detail));
                }
            }
        }
        if let Some(t) = hover_text {
            resp.on_hover_text(t);
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.selected else {
            ui.colored_label(DIM, "Click a node to see its ports, results and evidence trail.");
            ui.add_space(10.0);
            self.summary(ui);
            return;
        };
        let node = &self.sc.graph.nodes()[i];
        ui.heading(&node.label);
        ui.colored_label(DIM, role_text(node.spec.role));
        ui.add_space(6.0);
        ui.label(egui::RichText::new("INPUTS").small().color(DIM));
        for p in &node.spec.inputs {
            let state = if self.sc.graph.connected_inputs(i).contains(&p.name) { "wired" } else if p.required { "MISSING" } else { "—" };
            ui.label(format!("{} · {:?} · {} · {state}", p.name, p.kind, if p.required { "required" } else { "optional" }));
        }
        ui.label(egui::RichText::new("OUTPUTS").small().color(DIM));
        for p in &node.spec.outputs {
            ui.label(format!("{} · {:?}", p.name, p.kind));
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new("EVIDENCE TRAIL").small().color(DIM));
        for id in self.sc.graph.trail(i).unwrap_or_default() {
            ui.label(format!("· {}", self.sc.graph.nodes()[id].label));
        }
        ui.add_space(8.0);
        if ui.button("Open window").clicked() {
            self.open[i] = true;
        }
    }

    fn summary(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("RESULT").small().color(DIM));
        match &self.sc.fix {
            Some(f) => {
                let truth = place(BANG.0, BANG.1, 0.0);
                ui.label(format!("Bang at {:.5}, {:.5}", f.location.lat, f.location.lon));
                ui.colored_label(WARN, format!("radius {:.1} m (2σ, shared sound)", f.location.radius_m));
                ui.label(format!("made at {:.3} s ± {:.3} s", f.t0, f.t0_error_s));
                ui.colored_label(DIM, format!("synthetic truth is {:.1} m away", f.location.distance_m(&truth)));
            }
            None => {
                ui.colored_label(WARN, "No location: too few synced recorders.");
            }
        }
        ui.add_space(6.0);
        let unsolved: Vec<&str> = self.sc.sol.offsets.iter().enumerate()
            .filter(|(_, o)| matches!(o, Offset::Unsolved)).map(|(i, _)| NAMES[i]).collect();
        if !unsolved.is_empty() {
            ui.colored_label(WARN, format!("Not synced: {} (no shared flash)", unsolved.join(", ")));
        }
    }

    fn node_windows(&mut self, ctx: &egui::Context) {
        for i in 0..self.open.len() {
            if !self.open[i] {
                continue;
            }
            let mut open = true;
            let title = self.sc.graph.nodes()[i].label.clone();
            egui::Window::new(&title).id(egui::Id::new(("win", i))).open(&mut open).default_size([460.0, 380.0]).show(ctx, |ui| {
                if i == self.sc.locate_node {
                    self.map(ui);
                } else if i == self.sc.sync_node {
                    self.offsets_table(ui);
                } else if i < NAMES.len() {
                    self.stream_info(ui, i);
                } else {
                    ui.colored_label(DIM, "This node has no view of its own yet. Its results are on the timeline.");
                }
            });
            self.open[i] = open;
        }
    }

    fn stream_info(&self, ui: &mut egui::Ui, i: usize) {
        ui.label("Live stream, synthetic. Source: scenario");
        match self.sc.sol.offsets[i] {
            Offset::Solved { offset, error_s } => {
                ui.label(format!("clock offset {offset:+.3} s ± {error_s:.3} s (relative to Stream A)"));
            }
            Offset::Unsolved => {
                ui.colored_label(WARN, "clock offset unknown: this stream saw none of the shared flashes");
            }
        }
        if i < RECS.len() {
            let l = place(RECS[i].0, RECS[i].1, 5.0);
            ui.label(format!("position {:.5}, {:.5} ± {:.0} m (GPS, synthetic)", l.lat, l.lon, l.radius_m));
        } else {
            ui.colored_label(WARN, "position unknown");
        }
    }

    fn offsets_table(&self, ui: &mut egui::Ui) {
        ui.label("Offsets found from three flashes seen on video (frame accuracy 1/30 s).");
        ui.colored_label(DIM, if self.sc.sol.absolute { "absolute (UTC)" } else { "relative to Stream A: no anchor given" });
        egui::Grid::new("offsets").striped(true).show(ui, |ui| {
            ui.label("stream");
            ui.label("offset");
            ui.label("± error");
            ui.label("synthetic truth");
            ui.end_row();
            for (i, o) in self.sc.sol.offsets.iter().enumerate() {
                ui.label(NAMES[i]);
                match o {
                    Offset::Solved { offset, error_s } => {
                        ui.label(format!("{offset:+.3} s"));
                        ui.label(format!("{error_s:.3} s"));
                    }
                    Offset::Unsolved => {
                        ui.colored_label(WARN, "unsolved");
                        ui.label("—");
                    }
                }
                ui.colored_label(DIM, format!("{:+.3} s", TRUE_OFFSETS[i] - TRUE_OFFSETS[0]));
                ui.end_row();
            }
        });
        let out = self.sc.sol.outliers(3.0);
        ui.colored_label(if out.is_empty() { DIM } else { WARN }, format!("observations off by more than 3σ: {}", out.len()));
    }

    fn map(&self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(300.0), 320.0), Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 4.0, BG);
        // Fit -200..900 m east and -150..900 m north.
        let (x0, x1, y0, y1) = (-250.0, 850.0, -150.0, 850.0);
        let s = (rect.width() as f64 / (x1 - x0)).min(rect.height() as f64 / (y1 - y0));
        let to = |x: f64, y: f64| Pos2::new(rect.left() + ((x - x0) * s) as f32, rect.bottom() - ((y - y0) * s) as f32);
        for a in &self.sc.arrivals {
            let (x, y) = xy(&a.at);
            let c = to(x, y);
            p.circle_stroke(c, (a.at.radius_m * s) as f32, Stroke::new(1.0, with_alpha(Color32::from_rgb(122, 167, 224), 0.6)));
            p.circle_filled(c, 4.0, Color32::from_rgb(122, 167, 224));
        }
        for (i, r) in RECS.iter().enumerate() {
            p.text(to(r.0, r.1) + Vec2::new(7.0, -7.0), Align2::LEFT_BOTTOM, NAMES[i], FontId::proportional(11.0), TEXT);
        }
        if let Some(f) = &self.sc.fix {
            let (x, y) = xy(&f.location);
            let c = to(x, y);
            p.circle_filled(c, (f.location.radius_m * s) as f32, with_alpha(Color32::from_rgb(224, 122, 106), 0.25));
            p.circle_stroke(c, (f.location.radius_m * s) as f32, Stroke::new(1.5, Color32::from_rgb(224, 122, 106)));
            p.circle_filled(c, 3.0, Color32::from_rgb(224, 122, 106));
            p.text(c + Vec2::new(10.0, 10.0), Align2::LEFT_TOP, format!("bang ± {:.0} m", f.location.radius_m), FontId::proportional(11.0), TEXT);
        }
        let t = to(BANG.0, BANG.1);
        p.line_segment([t - Vec2::new(5.0, 5.0), t + Vec2::new(5.0, 5.0)], Stroke::new(1.0, DIM));
        p.line_segment([t - Vec2::new(5.0, -5.0), t + Vec2::new(5.0, -5.0)], Stroke::new(1.0, DIM));
        // Scale bar: 200 m.
        let a = Pos2::new(rect.left() + 12.0, rect.bottom() - 14.0);
        p.line_segment([a, a + Vec2::new((200.0 * s) as f32, 0.0)], Stroke::new(2.0, TEXT));
        p.text(a + Vec2::new(0.0, -4.0), Align2::LEFT_BOTTOM, "200 m", FontId::proportional(10.5), TEXT);
        ui.colored_label(DIM, "blue: recorders with GPS radius · red: located bang with its radius · ×: synthetic truth");
    }
}

impl Demo {
    /// The demo fills the window below the app's top bar.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("demo_timeline").frame(egui::Frame::new().fill(PANEL).inner_margin(0.0)).show(ui, |ui| {
            self.timeline_view(ui);
        });
        egui::Panel::right("demo_side").exact_size(300.0).frame(egui::Frame::new().fill(PANEL).inner_margin(12.0)).show(ui, |ui| {
            ui.colored_label(WARN, "Synthetic scenario: the data is made up to test the engine.");
            ui.add_space(8.0);
            self.side_panel(ui);
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ui, |ui| {
            self.graph_view(ui);
        });
        let ctx = ui.ctx().clone();
        self.node_windows(&ctx);
    }
}
