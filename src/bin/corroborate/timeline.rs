//! The timeline panel: one row per clock group, events with their margins.
//!
//! - **Rows**: absolute groups first (all on the shared UTC axis), then relative, then unanchored.
//!   A faint line separates rows of one kind, a strong one separates kinds. A row opens into one
//!   sub-row per source. Row colours are the canvas glow's (`clock_view::PALETTE`).
//! - **Floating rows** (relative and unanchored) sit on their own offset: `TlState.shift` is added
//!   to their times; they are drawn fainter, with a dashed top line.
//! - **Events**: a moment is a mark, a span is a bar. The clock's error is the margin around it,
//!   fading out from the centre; the event's own measurement error is the whisker. Both at the
//!   chosen confidence; red when the combined error is over the maximum.
//! - A click on a wire box on the canvas (`Evidence.focus`) opens and highlights that group's row.

use super::*;
use crate::clock_view::{groups_of, Confidence, Group};
use corroborate::clock::{Clocks, Kind as Ck};
use corroborate::data::{Data, Event};
use std::collections::{BTreeMap, BTreeSet};

/// One event on a row, in the numbering of its group (before any shift).
#[derive(Clone, Debug, PartialEq)]
pub struct TlEvent {
    pub node: NodeId,
    pub t0: f64,
    pub t1: f64,
    /// The clock's error at this time (1 sigma).
    pub clock_err: f64,
    /// The event's own measurement error (1 sigma).
    pub own_err: f64,
    pub label: String,
    pub detail: String,
}

impl TlEvent {
    /// Clock and own error combined (1 sigma).
    pub fn err(&self) -> f64 {
        self.clock_err.hypot(self.own_err)
    }
}

/// The events of one source's clock.
#[derive(Clone, Debug, PartialEq)]
pub struct SubRow {
    pub clock: String,
    pub name: String,
    pub events: Vec<TlEvent>,
}

/// One clock group.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub group: Group,
    pub subs: Vec<SubRow>,
}

impl Row {
    pub fn events(&self) -> impl Iterator<Item = &TlEvent> {
        self.subs.iter().flat_map(|s| s.events.iter())
    }
    /// The earliest time on the row (group numbering).
    pub fn first(&self) -> Option<f64> {
        self.events().map(|e| e.t0).min_by(|a, b| a.total_cmp(b))
    }
}

/// The rows for `events` (node, event): by group, absolute first. A time is placed by its clock's
/// offset (UTC for absolute clocks, the group's reference clock for relative ones, as is when
/// unanchored).
pub fn build_rows(clocks: &Clocks, events: &[(NodeId, Event)], name: &dyn Fn(&str) -> String) -> Vec<Row> {
    let used: BTreeSet<String> = events.iter().map(|(_, e)| e.clock.clone()).collect();
    let groups = groups_of(clocks, &used);
    let mut rows: Vec<Row> = groups.into_iter().map(|group| Row { group, subs: Vec::new() }).collect();
    for (node, e) in events {
        let info = clocks.get(&e.clock);
        let Some(row) = rows.iter_mut().find(|r| r.group.id == info.group) else { continue };
        let si = match row.subs.iter().position(|s| s.clock == e.clock) {
            Some(si) => si,
            None => {
                row.subs.push(SubRow { clock: e.clock.clone(), name: name(&e.clock), events: Vec::new() });
                row.subs.len() - 1
            }
        };
        row.subs[si].events.push(TlEvent {
            node: *node,
            t0: info.offset + e.t_start,
            t1: info.offset + e.t_end,
            clock_err: info.error_s,
            own_err: e.error_s,
            label: e.label.clone(),
            detail: e.detail.clone(),
        });
    }
    for r in &mut rows {
        r.subs.sort_by(|a, b| a.clock.cmp(&b.clock));
    }
    rows
}

/// Where a floating row starts when the user has not moved it: its first event under the first
/// event of the absolute rows (or the view's start), so it is in sight. 0 when nothing is absolute.
pub fn default_shift(rows: &[Row], row: &Row, view_start: f64) -> f64 {
    let Some(first) = row.first() else { return 0.0 };
    let base = rows.iter().filter(|r| r.group.kind == Ck::Absolute).filter_map(|r| r.first()).min_by(|a, b| a.total_cmp(b));
    match base {
        Some(b) => b - first,
        None if rows.iter().any(|r| r.group.kind == Ck::Absolute) => view_start - first,
        None => 0.0,
    }
}

/// The timeline's state: what is open, picked and where the floating rows are.
pub struct TlState {
    pub open: bool,
    pub expanded: BTreeSet<String>,
    pub picked: Option<String>,
    /// Floating row (group id) → seconds added to its times.
    pub shift: BTreeMap<String, f64>,
}

impl Default for TlState {
    fn default() -> Self {
        TlState { open: true, expanded: BTreeSet::new(), picked: None, shift: BTreeMap::new() }
    }
}

impl App {
    /// Every event on an output of the project, with the node it came from, and the end of the
    /// audio heard so far on the timeline's axis.
    pub(crate) fn timeline_events(&self) -> (Vec<(NodeId, Event)>, f64) {
        let mut out = Vec::new();
        let mut end = f64::MIN;
        for i in 0..self.nodes.len() {
            let n = &self.nodes[i];
            if n.hidden {
                continue;
            }
            let clock = self.clock_of(i).unwrap_or_default();
            if n.kind.is_source() {
                let a = n.state.lock().unwrap().audio_s;
                if a > 0.0 {
                    let info = self.ev.clocks.get(&clock);
                    end = end.max(if info.kind == Ck::Absolute { info.offset + a } else { a });
                }
            }
            for p in &self.graph.nodes()[i].spec.outputs {
                if p.kind != PortKind::Event {
                    continue;
                }
                if let Ok(Data::Events(v)) = self.value_from(i, p.name) {
                    out.extend(v.into_iter().map(|e| (i, e)));
                }
            }
            if matches!(n.kind, Kind::Enf { .. }) {
                let s = n.state.lock().unwrap();
                out.extend(s.enf.segments.iter().map(|g| {
                    (
                        i,
                        Event {
                            clock: clock.clone(),
                            t_start: g.t_start,
                            t_end: g.t_end,
                            label: format!("{:.3} Hz", g.mean_hz),
                            detail: format!("hum {:.3} Hz ({:.3}–{:.3}), harmonic {}", g.mean_hz, g.min_hz, g.max_hz, g.harmonic),
                            error_s: 0.0,
                        },
                    )
                }));
            }
        }
        (out, end)
    }

    /// The timeline panel's contents.
    pub(crate) fn timeline_view(&mut self, ui: &mut egui::Ui) {
        let (events, end) = self.timeline_events();
        let names: BTreeMap<String, String> = events.iter().map(|(_, e)| (e.clock.clone(), self.clock_name(&e.clock))).collect();
        let rows = build_rows(&self.ev.clocks, &events, &|c| names.get(c).cloned().unwrap_or_else(|| c.to_string()));
        let any_abs = rows.iter().any(|r| r.group.kind == Ck::Absolute);
        let sig = self.ev.conf.sigmas();
        let max = self.ev.max_error;

        // A click on a wire box brought us a group: show its row.
        if let Some(f) = self.ev.focus.take() {
            self.tls.open = true;
            self.tls.expanded.insert(f.clone());
            self.tls.picked = Some(f);
        }

        // Header: title, confidence, maximum error.
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if ui.small_button(if self.tls.open { "▼" } else { "▲" }).on_hover_text("show or hide the timeline").clicked() {
                self.tls.open = !self.tls.open;
            }
            ui.label(egui::RichText::new("Timeline").strong().color(TEXT));
            ui.colored_label(DIM, if any_abs { "UTC" } else { "source time" });
            ui.separator();
            ui.colored_label(DIM, "margins at");
            for c in [Confidence::P68, Confidence::P95, Confidence::P997] {
                ui.selectable_value(&mut self.ev.conf, c, c.label());
            }
            ui.separator();
            let mut on = self.ev.max_error.is_some();
            if ui.checkbox(&mut on, "max ±").on_hover_text("events whose error is over this turn red").changed() {
                self.ev.max_error = on.then_some(self.ev.max_error.unwrap_or(5.0));
            }
            if let Some(m) = &mut self.ev.max_error {
                ui.add(egui::DragValue::new(m).range(0.001..=86_400.0).speed(0.1).suffix(" s"));
            }
            if !self.follow && ui.small_button("Follow").clicked() {
                self.follow = true;
            }
        });
        if !self.tls.open {
            return;
        }
        ui.separator();

        // The visible rows: a group row, then its sources when open.
        enum Line<'a> {
            Group(&'a Row),
            Sub(&'a Row, &'a SubRow),
        }
        let mut lines: Vec<Line> = Vec::new();
        for r in &rows {
            lines.push(Line::Group(r));
            if self.tls.expanded.contains(&r.group.id) {
                lines.extend(r.subs.iter().map(|s| Line::Sub(r, s)));
            }
        }

        let label_w = 190.0;
        let ruler_h = 20.0;
        let lane_h = 30.0;
        let shift_of = |tls: &TlState, r: &Row, v0: f64| -> f64 {
            if r.group.kind == Ck::Absolute {
                0.0
            } else {
                tls.shift.get(&r.group.id).copied().unwrap_or_else(|| default_shift(&rows, r, v0))
            }
        };

        let h = ruler_h + lane_h * lines.len().max(1) as f32 + 4.0;
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
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

            // Ruler.
            let raw = span / 8.0;
            let mag = 10f64.powf(raw.log10().floor());
            let step = [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
            let mut t = (v0 / step).ceil() * step;
            while t <= self.tl.1 {
                let x = tx(t);
                p.line_segment([Pos2::new(x, track.top()), Pos2::new(x, area.bottom())], Stroke::new(1.0, Color32::from_rgb(40, 44, 49)));
                let txt = if any_abs { fmt_clock(t) } else { format!("{t:.0} s") };
                p.text(Pos2::new(x + 3.0, track.top() + 3.0), Align2::LEFT_TOP, txt, FontId::proportional(10.5), DIM);
                t += step;
            }
            p.line_segment([Pos2::new(area.left(), area.top() + ruler_h), Pos2::new(area.right(), area.top() + ruler_h)], Stroke::new(1.0, LINE));

            let mut hover: Option<String> = None;
            let mut pick_node: Option<NodeId> = None;
            let mut toggle: Option<String> = None;
            let mut prev_kind: Option<Ck> = None;
            for (li, line) in lines.iter().enumerate() {
                let top = area.top() + ruler_h + li as f32 * lane_h;
                let row_rect = Rect::from_min_max(Pos2::new(area.left(), top), Pos2::new(area.right(), top + lane_h));
                let (r, sub) = match line {
                    Line::Group(r) => (*r, None),
                    Line::Sub(r, s) => (*r, Some(*s)),
                };
                let floating = r.group.kind != Ck::Absolute;
                let faint = if r.group.kind == Ck::None { 0.45 } else { 1.0 };
                let col = r.group.color;
                // Lines between rows: strong between kinds, faint inside one kind.
                let strong = sub.is_none() && prev_kind.is_some_and(|k| k != r.group.kind);
                if sub.is_none() {
                    prev_kind = Some(r.group.kind);
                }
                p.line_segment(
                    [row_rect.left_top(), row_rect.right_top()],
                    Stroke::new(1.0, if strong { Color32::from_rgb(90, 96, 104) } else { Color32::from_rgb(42, 46, 51) }),
                );
                if self.tls.picked.as_deref() == Some(r.group.id.as_str()) {
                    p.rect_filled(row_rect, 0.0, alpha(col, 0.08));
                }
                if floating && sub.is_none() {
                    // Dashed top edge: this row is not tied to UTC.
                    let mut x = track.left();
                    while x < area.right() {
                        p.line_segment([Pos2::new(x, top + 0.5), Pos2::new((x + 5.0).min(area.right()), top + 0.5)], Stroke::new(1.5, alpha(col, 0.6 * faint)));
                        x += 9.0;
                    }
                }
                p.rect_filled(Rect::from_min_max(Pos2::new(area.left(), top + 2.0), Pos2::new(area.left() + 4.0, top + lane_h - 2.0)), 1.0, alpha(col, 0.9 * faint));
                // Row label (click: open or close, pick).
                let label_rect = Rect::from_min_max(Pos2::new(area.left(), top), Pos2::new(track.left(), top + lane_h));
                let (title, note) = match sub {
                    None => {
                        let info = self.ev.clocks.get(&r.group.id);
                        let kind = match r.group.kind {
                            Ck::Absolute => format!("UTC · ± {}", clock_view::err_text(r.subs.iter().map(|s| self.ev.clocks.get(&s.clock).error_s).fold(0.0, f64::max) * sig)),
                            Ck::Relative => format!("relative · ± {}", clock_view::err_text(info.error_s * sig)),
                            Ck::None => "no anchor, no link".to_string(),
                        };
                        let who = if r.subs.len() == 1 { r.subs[0].name.clone() } else { format!("{} sources", r.subs.len()) };
                        (format!("{} {who}", if self.tls.expanded.contains(&r.group.id) { "▾" } else { "▸" }), kind)
                    }
                    Some(s) => (format!("   {}", s.name), format!("   {} events", s.events.len())),
                };
                p.text(Pos2::new(area.left() + 10.0, top + 10.0), Align2::LEFT_CENTER, title, FontId::proportional(12.0), alpha(TEXT, faint));
                p.text(Pos2::new(area.left() + 10.0, top + 22.0), Align2::LEFT_CENTER, note, FontId::proportional(9.5), DIM);
                if sub.is_none() && resp.clicked_by(egui::PointerButton::Primary) && resp.interact_pointer_pos().is_some_and(|h| label_rect.contains(h)) {
                    toggle = Some(r.group.id.clone());
                }
                if floating && sub.is_none() && resp.hovered() && resp.hover_pos().is_some_and(|h| label_rect.contains(h)) {
                    hover = Some(format!("{}\nfloating: the row's times are shifted by {:+.1} s", self.breakdown_lines(&r.group.id).join("\n"), shift_of(&self.tls, r, v0)));
                } else if sub.is_none() && resp.hover_pos().is_some_and(|h| label_rect.contains(h)) {
                    hover = Some(self.breakdown_lines(&r.group.id).join("\n"));
                }

                // Events: on the group row all of them, on a source's row its own.
                let shift = shift_of(&self.tls, r, v0);
                let subs: Vec<&SubRow> = match sub {
                    Some(s) => vec![s],
                    None => r.subs.iter().collect(),
                };
                for s in subs {
                    for e in &s.events {
                        let (t0, t1) = (e.t0 + shift, e.t1 + shift);
                        let reach = e.clock_err * sig;
                        let whisker = e.own_err * sig;
                        let (xa, xb) = (tx(t0 - reach - whisker), tx(t1 + reach + whisker));
                        if xb < track.left() || xa > track.right() {
                            continue;
                        }
                        let over = max.is_some_and(|m| e.err() * sig > m);
                        let ec = if over { Color32::from_rgb(235, 80, 80) } else { col };
                        let (cy, half) = (top + lane_h * 0.5, lane_h * 0.5 - 5.0);
                        // Clock margin: fades out from the centre.
                        let (m0, m1) = (tx(t0 - reach), tx(t1 + reach));
                        const LAYERS: usize = 6;
                        for k in 0..LAYERS {
                            let f = 1.0 - k as f32 / LAYERS as f32;
                            let (a, b) = (tx(t0) + (m0 - tx(t0)) * f, tx(t1) + (m1 - tx(t1)) * f);
                            let r = Rect::from_min_max(Pos2::new(a.max(track.left()), cy - half), Pos2::new(b.min(track.right()).max(a.max(track.left()) + 1.0), cy + half));
                            p.rect_filled(r, 2.0, alpha(ec, 0.035 * faint));
                        }
                        // The event: a mark for a moment, a bar for a span.
                        let (x0, x1) = (tx(t0).max(track.left()), tx(t1).min(track.right()));
                        let core = Rect::from_min_max(Pos2::new(x0, cy - half + 3.0), Pos2::new(x1.max(x0 + 2.0), cy + half - 3.0));
                        p.rect_filled(core, 2.0, alpha(ec, 0.85 * faint));
                        if core.width() > 44.0 {
                            p.text(core.left_center() + Vec2::new(3.0, 0.0), Align2::LEFT_CENTER, &e.label, FontId::proportional(10.0), Color32::BLACK);
                        }
                        // Measurement error: whiskers from the event's ends.
                        if whisker > 0.0 {
                            let w = Stroke::new(1.0, alpha(Color32::WHITE, 0.7 * faint));
                            let (a, b) = (tx(t0 - whisker).max(track.left()), tx(t1 + whisker).min(track.right()));
                            p.line_segment([Pos2::new(a, cy), Pos2::new(b, cy)], w);
                            p.line_segment([Pos2::new(a, cy - 4.0), Pos2::new(a, cy + 4.0)], w);
                            p.line_segment([Pos2::new(b, cy - 4.0), Pos2::new(b, cy + 4.0)], w);
                        }
                        let hit = Rect::from_min_max(Pos2::new(xa.max(track.left()), cy - half), Pos2::new(xb.min(track.right()).max(xa + 3.0), cy + half)).expand2(Vec2::new(2.0, 0.0));
                        if resp.hover_pos().is_some_and(|h| hit.contains(h)) {
                            let when = if r.group.kind == Ck::Absolute { format!("{} UTC", fmt_clock(t0)) } else { format!("{} on {}", clock_view::rel_text(e.t0), s.name) };
                            hover = Some(format!(
                                "{}\n{}\n{when}\nclock ± {} · own ± {} · together ± {} ({})",
                                e.label,
                                e.detail,
                                clock_view::err_text(e.clock_err * sig),
                                clock_view::err_text(e.own_err * sig),
                                clock_view::err_text(e.err() * sig),
                                self.ev.conf.label(),
                            ));
                            if resp.clicked_by(egui::PointerButton::Primary) {
                                pick_node = Some(e.node);
                            }
                        }
                    }
                }
            }
            if rows.is_empty() {
                p.text(Pos2::new(track.left() + 8.0, area.top() + ruler_h + 14.0), Align2::LEFT_CENTER, "findings from detectors and plugins appear here", FontId::proportional(11.0), DIM);
            }
            if let Some(g) = toggle {
                if !self.tls.expanded.remove(&g) {
                    self.tls.expanded.insert(g.clone());
                }
                self.tls.picked = Some(g);
            }
            if let Some(n) = pick_node {
                self.pick_only(n);
            }
            if let Some(t) = hover {
                resp.on_hover_text(t);
            }
        });
    }
}
