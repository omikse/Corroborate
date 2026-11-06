//! The clocks on the canvas: a soft glow behind each clock group's nodes and wires, and a box at
//! the middle of every wire that carries times.
//!
//! - **Group**: clocks that can be compared. All absolute clocks are one group (UTC); relative
//!   clocks are grouped by their links; an unanchored clock is a group of its own.
//! - **Glow**: the group's colour (one palette, shared with the timeline). Solid for absolute,
//!   dashed for relative, faint for unanchored. A node on several groups is striped.
//! - **Wire box**: top line the time (UTC when known, else "+00:41.2 on <source>"), or the count and
//!   span of a list; bottom line ± the error (clock error and the event's own error combined by
//!   root-sum-square) at the chosen confidence, red when over the maximum. Zoomed out it keeps only
//!   the ± line, then nothing. Hovering lists what the error is made of; a click picks the group.

use super::*;
use corroborate::clock::{self, Clocks, Ev, Kind as Ck};
use corroborate::data::Data;
use corroborate::timebase;
use std::collections::BTreeSet;

/// Group colours: the first is UTC's, the rest go round in order of kind then id.
pub const PALETTE: [Color32; 8] = [
    Color32::from_rgb(96, 165, 250),
    Color32::from_rgb(240, 171, 92),
    Color32::from_rgb(167, 139, 250),
    Color32::from_rgb(110, 200, 140),
    Color32::from_rgb(240, 110, 150),
    Color32::from_rgb(90, 200, 210),
    Color32::from_rgb(210, 200, 90),
    Color32::from_rgb(200, 130, 90),
];

/// How sure the shown errors are: 1, 2 or 3 sigma.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[allow(dead_code)] // the timeline's confidence switch picks P68 and P997
pub enum Confidence {
    P68,
    #[default]
    P95,
    P997,
}

impl Confidence {
    pub fn sigmas(self) -> f64 {
        match self {
            Confidence::P68 => 1.0,
            Confidence::P95 => 2.0,
            Confidence::P997 => 3.0,
        }
    }
    #[allow(dead_code)] // the timeline's confidence switch uses it
    pub fn label(self) -> &'static str {
        match self {
            Confidence::P68 => "68%",
            Confidence::P95 => "95%",
            Confidence::P997 => "99.7%",
        }
    }
}

/// One clock group in use.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// `"utc"`, a relative group's reference clock, or an unanchored clock.
    pub id: String,
    pub kind: Ck,
    pub color: Color32,
}

/// The groups in use and, per node, which of them it is on.
pub struct CanvasGroups {
    pub groups: Vec<Group>,
    pub of_node: Vec<Vec<usize>>,
}

/// The group each of `used` clocks is in, absolute first, with its colour.
pub fn groups_of(clocks: &Clocks, used: &BTreeSet<String>) -> Vec<Group> {
    let set: BTreeSet<(Ck, String)> = used.iter().map(|c| {
        let i = clocks.get(c);
        (i.kind, i.group)
    }).collect();
    let mut other = 0;
    set.into_iter()
        .map(|(kind, id)| {
            let color = if kind == Ck::Absolute {
                PALETTE[0]
            } else {
                other += 1;
                PALETTE[1 + (other - 1) % (PALETTE.len() - 1)]
            };
            Group { id, kind, color }
        })
        .collect()
}

/// (fill alpha, dashed) of a group's glow.
pub fn glow_style(kind: Ck) -> (f32, bool) {
    match kind {
        Ck::Absolute => (0.16, false),
        Ck::Relative => (0.12, true),
        Ck::None => (0.05, false),
    }
}

fn timey(k: PortKind) -> bool {
    matches!(k, PortKind::Time | PortKind::Event | PortKind::Track | PortKind::Custom("anchor" | "link"))
}

/// `±` text of a number of seconds.
pub fn err_text(s: f64) -> String {
    if s < 0.01 {
        format!("{:.0} ms", s * 1000.0)
    } else if s < 10.0 {
        format!("{s:.2} s")
    } else if s < 60.0 {
        format!("{s:.1} s")
    } else {
        format!("{}m{:02.0}s", (s / 60.0) as i64, s % 60.0)
    }
}

/// `+00:41.2` (a time on a clock that is not tied to UTC).
pub fn rel_text(t: f64) -> String {
    let a = t.abs();
    format!("{}{:02}:{:04.1}", if t < 0.0 { "-" } else { "+" }, (a / 60.0) as i64, a % 60.0)
}

/// What a wire's box says.
#[derive(Clone, Debug, PartialEq)]
pub struct WireText {
    pub top: String,
    /// `± 5.00 s`; empty when the value has no error.
    pub bottom: String,
    pub over: bool,
    /// The clock the value is on (for the hover list); empty when not known.
    pub clock: String,
}

/// The box text of `data` at `conf`; `max` is the largest error allowed (seconds, at `conf`), `name`
/// gives a clock's source name.
pub fn wire_text(clocks: &Clocks, data: &Data, conf: Confidence, max: Option<f64>, name: &dyn Fn(&str) -> String) -> Option<WireText> {
    let rss = |a: f64, b: f64| (a * a + b * b).sqrt();
    // Clock error added to a measurement error when the clock is absolute.
    let total = |clock: &str, e: f64| if clocks.get(clock).kind == Ck::Absolute { rss(clocks.get(clock).error_s, e) } else { e };
    let hhmm = |u: f64| timebase::fmt_utc_clock(u)[..5].to_string();
    let (top, err, clock) = match data {
        Data::Time { clock, t, utc, error_s, .. } => match (utc, clocks.to_utc(clock, *t)) {
            (Some(u), _) => (format!("{} UTC", timebase::fmt_utc_clock(*u)), Some(*error_s), clock.clone()),
            (None, Some((u, _))) => (format!("{} UTC", timebase::fmt_utc_clock(u)), Some(total(clock, *error_s)), clock.clone()),
            (None, None) => (format!("{} on {}", rel_text(*t), name(clock)), Some(*error_s), clock.clone()),
        },
        Data::Events(v) => {
            if v.is_empty() {
                return Some(WireText { top: "no events".into(), bottom: String::new(), over: false, clock: String::new() });
            }
            let n = v.len();
            let plural = if n == 1 { "event" } else { "events" };
            let span = span_text(clocks, v.iter().map(|e| (e.clock.as_str(), e.t_start, e.t_end)), name, &hhmm);
            let err = v.iter().map(|e| total(&e.clock, e.error_s)).fold(0.0, f64::max);
            (format!("{n} {plural}{span}"), Some(err), v[0].clock.clone())
        }
        Data::Tracks(v) => {
            if v.is_empty() {
                return Some(WireText { top: "no points".into(), bottom: String::new(), over: false, clock: String::new() });
            }
            let span = span_text(clocks, v.iter().map(|p| (p.clock.as_str(), p.t, p.t)), name, &hhmm);
            let err = v.iter().map(|p| total(&p.clock, 0.0)).fold(0.0, f64::max);
            (format!("{} points{span}", v.len()), Some(err), v[0].clock.clone())
        }
        Data::Custom { kind: "anchor", json } => {
            let a = clock::anchors_from_json(json, "", "");
            let err = a.iter().map(|x| x.error_s).fold(0.0, f64::max);
            (format!("{} anchor{}", a.len(), if a.len() == 1 { "" } else { "s" }), (!a.is_empty()).then_some(err), a.first().map(|x| x.clock.clone()).unwrap_or_default())
        }
        Data::Custom { kind: "link", json } => {
            let l = clock::links_from_json(json, "", "");
            let err = l.iter().map(|x| x.error_s).fold(0.0, f64::max);
            (format!("{} link{}", l.len(), if l.len() == 1 { "" } else { "s" }), (!l.is_empty()).then_some(err), l.first().map(|x| x.a.clone()).unwrap_or_default())
        }
        _ => return None,
    };
    let shown = err.map(|e| e * conf.sigmas());
    Some(WireText {
        top,
        bottom: shown.map_or(String::new(), |e| format!("± {}", err_text(e))),
        over: matches!((shown, max), (Some(e), Some(m)) if e > m),
        clock,
    })
}

/// `, 12:50–12:58` or `, +00:41–+02:10 on X` or ` on 2 clocks`, for items of (clock, start, end).
fn span_text<'a>(clocks: &Clocks, items: impl Iterator<Item = (&'a str, f64, f64)>, name: &dyn Fn(&str) -> String, hhmm: &dyn Fn(f64) -> String) -> String {
    let items: Vec<_> = items.collect();
    let utc: Vec<(f64, f64)> = items.iter().filter_map(|&(c, a, b)| Some((clocks.to_utc(c, a)?.0, clocks.to_utc(c, b)?.0))).collect();
    if utc.len() == items.len() {
        let (lo, hi) = utc.iter().fold((f64::MAX, f64::MIN), |(l, h), &(a, b)| (l.min(a), h.max(b)));
        return format!(", {}–{}", hhmm(lo), hhmm(hi));
    }
    let own: BTreeSet<&str> = items.iter().map(|i| i.0).collect();
    if own.len() == 1 {
        let (lo, hi) = items.iter().fold((f64::MAX, f64::MIN), |(l, h), &(_, a, b)| (l.min(a), h.max(b)));
        return format!(", {}–{} on {}", rel_text(lo), rel_text(hi), name(items[0].0));
    }
    format!(" on {} clocks", own.len())
}

impl App {
    /// The clocks node `i` is on: its own, and those of the times, events, tracks, anchors and
    /// links wired into it.
    pub(crate) fn node_clocks(&self, i: NodeId) -> Vec<String> {
        let mut v: Vec<String> = self.clock_of(i).into_iter().collect();
        for p in &self.graph.nodes()[i].spec.inputs {
            let Some((f, out)) = self.graph.feeder(i, p.name) else { continue };
            let Some(o) = self.graph.nodes()[f].spec.outputs.iter().find(|o| o.name == out) else { continue };
            if !timey(o.kind) {
                continue;
            }
            if let Some(c) = self.clock_of(f) {
                if !v.contains(&c) {
                    v.push(c);
                }
            }
        }
        v
    }

    /// The groups in use and which each node is on.
    pub(crate) fn canvas_groups(&self) -> CanvasGroups {
        let per: Vec<Vec<String>> = (0..self.nodes.len()).map(|i| if self.nodes[i].hidden { Vec::new() } else { self.node_clocks(i) }).collect();
        let used: BTreeSet<String> = per.iter().flatten().cloned().collect();
        let groups = groups_of(&self.ev.clocks, &used);
        let of_node = per
            .iter()
            .map(|cs| {
                let mut g: Vec<usize> = cs.iter().filter_map(|c| groups.iter().position(|g| g.id == self.ev.clocks.get(c).group)).collect();
                g.sort_unstable();
                g.dedup();
                g
            })
            .collect();
        CanvasGroups { groups, of_node }
    }

    /// The name of the source a clock belongs to.
    pub(crate) fn clock_name(&self, clock: &str) -> String {
        if clock == clock::UTC {
            return "UTC".into();
        }
        for i in 0..self.nodes.len() {
            if self.nodes[i].kind.is_source() && self.clock_of(i).as_deref() == Some(clock) {
                return self.node_name(i);
            }
        }
        clock.to_string()
    }

    pub(crate) fn node_name(&self, i: NodeId) -> String {
        if self.nodes[i].label.is_empty() { self.graph.nodes()[i].spec.name.to_string() } else { self.nodes[i].label.clone() }
    }

    /// What `clock`'s error is made of, largest first: one line each.
    pub(crate) fn breakdown_lines(&self, clock: &str) -> Vec<String> {
        let info = self.ev.clocks.get(clock);
        let total: f64 = info.breakdown.iter().map(|b| b.1).sum();
        let kind = match info.kind {
            Ck::Absolute => "absolute",
            Ck::Relative => "relative",
            Ck::None => "unanchored",
        };
        let mut lines = vec![format!("{} · {kind} · ± {}", self.clock_name(clock), err_text(info.error_s * self.ev.conf.sigmas()))];
        for (e, var) in &info.breakdown {
            let from = match e {
                Ev::Anchor(i) => self.ev.anchors.get(*i).map(|a| (a.from.clone(), "anchor")),
                Ev::Link(i) => self.ev.links.get(*i).map(|l| (l.from.clone(), "link")),
            };
            let Some((from, what)) = from else { continue };
            let who = match (0..self.nodes.len()).find(|&i| self.nodes[i].uid == from) {
                Some(i) => self.node_name(i),
                None => from,
            };
            lines.push(format!("{who} ({what}): ± {} · {:.0}%", err_text(var.sqrt() * self.ev.conf.sigmas()), if total > 0.0 { 100.0 * var / total } else { 0.0 }));
        }
        lines
    }

    /// The glow behind the nodes and wires of each clock group (under the wires and nodes).
    pub(crate) fn draw_glow(&self, ui: &egui::Ui, painter: &egui::Painter, origin: Pos2) {
        let cg = self.canvas_groups();
        if cg.groups.is_empty() {
            return;
        }
        let z = self.zoom;
        for w in self.graph.wires() {
            let Some(&g) = cg.of_node[w.from].iter().find(|g| cg.of_node[w.to].contains(g)) else { continue };
            let grp = &cg.groups[g];
            let (a, sa) = self.port_place(origin, w.from, w.from_port, true);
            let (b, sb) = self.port_place(origin, w.to, w.to_port, false);
            let (al, dashed) = glow_style(grp.kind);
            let stroke = Stroke::new(12.0 * z, alpha(grp.color, al));
            let curve = CubicBezierShape::from_points_stroke(layout::wire_points(a, sa, b, sb), false, Color32::TRANSPARENT, Stroke::NONE);
            if dashed {
                painter.extend(egui::Shape::dashed_line(&curve.flatten(Some(0.5)), stroke, 14.0 * z, 8.0 * z));
            } else {
                painter.add(CubicBezierShape::from_points_stroke(layout::wire_points(a, sa, b, sb), false, Color32::TRANSPARENT, stroke));
            }
        }
        for i in 0..self.nodes.len() {
            let gs = &cg.of_node[i];
            if self.nodes[i].hidden || gs.is_empty() {
                continue;
            }
            let rect = self.node_rect(origin, i).expand(10.0 * z);
            if gs.len() == 1 {
                let grp = &cg.groups[gs[0]];
                let (al, dashed) = glow_style(grp.kind);
                painter.rect_filled(rect, 10.0 * z, alpha(grp.color, al));
                if dashed {
                    let r = rect;
                    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                    painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.5, alpha(grp.color, 0.6)), 6.0 * z, 4.0 * z));
                }
            } else {
                // Stripes, one colour per group.
                let w = 12.0 * z;
                let (mut x, mut k) = (rect.left(), 0);
                while x < rect.right() {
                    let grp = &cg.groups[gs[k % gs.len()]];
                    let (al, _) = glow_style(grp.kind);
                    painter.rect_filled(Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new((x + w).min(rect.right()), rect.bottom())), 0.0, alpha(grp.color, al * 1.5));
                    x += w;
                    k += 1;
                }
            }
            // Hovering the margin lists what the group's error is made of.
            let clock = self.node_clocks(i).into_iter().next().unwrap_or_default();
            ui.interact(rect, egui::Id::new(("glow", i)), Sense::hover()).on_hover_ui(|ui| {
                for l in self.breakdown_lines(&clock) {
                    ui.label(l);
                }
            });
        }
    }

    /// The boxes at the middle of the wires that carry times. A click picks the group (the
    /// timeline shows its row).
    pub(crate) fn draw_wire_boxes(&mut self, ui: &egui::Ui, painter: &egui::Painter, origin: Pos2) {
        let z = self.zoom;
        if z < 0.5 {
            return;
        }
        let full = z >= 0.7;
        let mut focus = None;
        let cg = self.canvas_groups();
        let name = |c: &str| self.clock_name(c);
        for (wi, w) in self.graph.wires().iter().enumerate() {
            let out = &self.graph.nodes()[w.from].spec.outputs[w.from_port];
            if !timey(out.kind) {
                continue;
            }
            let Ok(data) = self.value_from(w.from, out.name) else { continue };
            let Some(t) = wire_text(&self.ev.clocks, &data, self.ev.conf, self.ev.max_error, &name) else { continue };
            let (a, sa) = self.port_place(origin, w.from, w.from_port, true);
            let (b, sb) = self.port_place(origin, w.to, w.to_port, false);
            let p = layout::wire_points(a, sa, b, sb);
            let mid = Pos2::new((p[0].x + 3.0 * p[1].x + 3.0 * p[2].x + p[3].x) / 8.0, (p[0].y + 3.0 * p[1].y + 3.0 * p[2].y + p[3].y) / 8.0);
            let size = 11.0 * z.min(1.2);
            let bottom_col = if t.over { LIVE } else { DIM };
            let top = painter.layout_no_wrap(t.top.clone(), egui::FontId::proportional(size), TEXT);
            let bottom = painter.layout_no_wrap(t.bottom.clone(), egui::FontId::proportional(size), bottom_col);
            let (tw, th) = if full { (top.size().x, top.size().y) } else { (0.0, 0.0) };
            let (bw, bh) = if t.bottom.is_empty() { (0.0, 0.0) } else { (bottom.size().x, bottom.size().y) };
            if tw + bw == 0.0 {
                continue;
            }
            let rect = Rect::from_center_size(mid, Vec2::new(tw.max(bw) + 10.0, th + bh + 6.0));
            let group = self.ev.clocks.get(&t.clock);
            let group_col = cg.groups.iter().find(|g| g.id == group.group).map_or(DIM, |g| g.color);
            painter.rect_filled(rect, 4.0, alpha(PANEL, 0.92));
            painter.rect_stroke(rect, 4.0, Stroke::new(1.0, if t.over { LIVE } else { alpha(group_col, 0.8) }), egui::StrokeKind::Inside);
            let mut y = rect.top() + 3.0;
            if full {
                painter.galley(Pos2::new(rect.center().x - tw / 2.0, y), top, TEXT);
                y += th;
            }
            if bh > 0.0 {
                painter.galley(Pos2::new(rect.center().x - bw / 2.0, y), bottom, bottom_col);
            }
            let resp = ui.interact(rect, egui::Id::new(("wirebox", wi)), Sense::click());
            if resp.clicked() {
                focus = Some(group.group.clone());
            }
            resp.on_hover_ui(|ui| {
                for l in self.breakdown_lines(&t.clock) {
                    ui.label(l);
                }
            });
        }
        if focus.is_some() {
            self.ev.focus = focus;
        }
    }
}
