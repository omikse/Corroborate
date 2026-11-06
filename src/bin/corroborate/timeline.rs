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
//! - **Sync list** (right of the rows): every anchor and link with who gave it, its error, its share
//!   of the clock's error and an on/off switch; contradictions in red; a click picks the node.
//!   Links are drawn between rows (offset ± error), anchors are pinned to the axis.
//! - A click on a wire box on the canvas (`Evidence.focus`) opens and highlights that group's row.

use super::*;
use crate::clock_view::{groups_of, Confidence, Group};
use corroborate::clock::{Anchor as ClockAnchor, Clocks, Ev, Kind as Ck, Link};
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
    /// The floating row being dragged (group id).
    pub drag: Option<String>,
    /// A dropped row waiting for the user to confirm the link (or anchor) and its error.
    pub pending: Option<Pending>,
    /// Where the rows were last drawn: the track, the view start and span, each group row's centre y.
    pub geom: Option<(Rect, f64, f64)>,
    pub row_y: Vec<(String, f32)>,
    /// Events tests put on the timeline.
    #[cfg(test)]
    pub extra: Vec<(NodeId, Event)>,
}

impl Default for TlState {
    fn default() -> Self {
        TlState {
            open: true,
            expanded: BTreeSet::new(),
            picked: None,
            shift: BTreeMap::new(),
            drag: None,
            pending: None,
            geom: None,
            row_y: Vec::new(),
            #[cfg(test)]
            extra: Vec::new(),
        }
    }
}

/// Where a dragged row's event met another row's event.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    /// The dragged row's group.
    pub group: String,
    /// Moment `t_a` on clock `a` (the dragged row) …
    pub a: String,
    pub t_a: f64,
    /// … is moment `t_b` on clock `b`, or UTC `t_b` when the other row is absolute.
    pub b: String,
    pub t_b: f64,
    pub absolute: bool,
    pub error_s: f64,
    /// Seconds the dragged row still has to move so the two events meet.
    pub gap: f64,
}

/// The closest pair of events, one on the dragged row `moving` and one on another row, less than
/// `tol` seconds apart as drawn. `shift` gives each row's shift, `offset` each clock's offset.
pub fn drop_match(rows: &[Row], moving: &str, shift: &dyn Fn(&Row) -> f64, offset: &dyn Fn(&str) -> f64, tol: f64) -> Option<Pending> {
    let m = rows.iter().find(|r| r.group.id == moving)?;
    let sm = shift(m);
    let mut best: Option<Pending> = None;
    for o in rows.iter().filter(|r| r.group.id != moving) {
        let so = shift(o);
        for sa in &m.subs {
            for ea in &sa.events {
                for sb in &o.subs {
                    for eb in &sb.events {
                        let gap = (eb.t0 + so) - (ea.t0 + sm);
                        if gap.abs() < tol && best.as_ref().is_none_or(|b| gap.abs() < b.gap.abs()) {
                            let absolute = o.group.kind == Ck::Absolute;
                            best = Some(Pending {
                                group: moving.to_string(),
                                a: sa.clock.clone(),
                                t_a: ea.t0 - offset(&sa.clock),
                                b: if absolute { corroborate::clock::UTC.to_string() } else { sb.clock.clone() },
                                t_b: if absolute { eb.t0 } else { eb.t0 - offset(&sb.clock) },
                                absolute,
                                error_s: 0.0,
                                gap,
                            });
                        }
                    }
                }
            }
        }
    }
    best
}

/// One line of the Sync list: a piece of evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncEntry {
    pub ev: Ev,
    pub key: String,
    pub enabled: bool,
    /// Over 3 sigma off the solution (with its residual in sigmas).
    pub contradiction: Option<f64>,
    pub title: String,
    /// Who gave it, its error and its share of the clock's error.
    pub detail: String,
    /// Share of the variance of the clock it ties down (0..1); 0 when switched off.
    pub share: f64,
    /// Stable id of the node that gave it (`"manual"` for hand-made evidence).
    pub from: String,
}

/// The Sync list: anchors, then links. `sig` is the display confidence in sigmas.
pub fn sync_entries(anchors: &[ClockAnchor], links: &[Link], clocks: &Clocks, sig: f64, clock_name: &dyn Fn(&str) -> String, who: &dyn Fn(&str) -> String) -> Vec<SyncEntry> {
    let share = |ev: Ev, clock: &str| -> f64 {
        let info = clocks.get(clock);
        let total: f64 = info.breakdown.iter().map(|b| b.1).sum();
        info.breakdown.iter().find(|b| b.0 == ev).map_or(0.0, |b| if total > 0.0 { b.1 / total } else { 0.0 })
    };
    let mut out = Vec::new();
    for (i, a) in anchors.iter().enumerate() {
        let ev = Ev::Anchor(i);
        let at = if a.t == 0.0 { String::new() } else { format!(" at {}", clock_view::rel_text(a.t)) };
        let sh = share(ev, &a.clock);
        out.push(SyncEntry {
            ev,
            key: a.key(),
            enabled: a.enabled,
            contradiction: clocks.contradictions.iter().find(|c| c.0 == ev).map(|c| c.1),
            title: format!("anchor · {}{at} = {} UTC", clock_name(&a.clock), timebase::fmt_utc_clock(a.utc)),
            detail: format!("{} · ± {} · {:.0}%", who(&a.from), clock_view::err_text(a.error_s * sig), sh * 100.0),
            share: sh,
            from: a.from.clone(),
        });
    }
    for (i, l) in links.iter().enumerate() {
        let ev = Ev::Link(i);
        let sh = share(ev, if l.a == corroborate::clock::UTC { &l.b } else { &l.a });
        out.push(SyncEntry {
            ev,
            key: l.key(),
            enabled: l.enabled,
            contradiction: clocks.contradictions.iter().find(|c| c.0 == ev).map(|c| c.1),
            title: format!("link · {} {} = {} {}", clock_name(&l.a), clock_view::rel_text(l.t_a), clock_name(&l.b), clock_view::rel_text(l.t_b)),
            detail: format!("{} · offset {:+.2} s ± {} · {:.0}%", who(&l.from), l.t_b - l.t_a, clock_view::err_text(l.error_s * sig), sh * 100.0),
            share: sh,
            from: l.from.clone(),
        });
    }
    out
}

/// Distance from point `p` to the segment `a`–`b`.
fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let (ab, ap) = (b - a, p - a);
    let l2 = ab.length_sq();
    let t = if l2 > 0.0 { (ap.dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).length()
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
        #[cfg(test)]
        out.extend(self.tls.extra.iter().cloned());
        (out, end)
    }

    /// Makes the pending link (or anchor) manual evidence; the solve runs again next frame and
    /// the dragged row joins the other row's group.
    pub(crate) fn confirm_link(&mut self) {
        let Some(p) = self.tls.pending.take() else { return };
        if p.absolute {
            self.ev.manual_anchors.push(ClockAnchor { clock: p.a, t: p.t_a, utc: p.t_b, error_s: p.error_s, from: "manual".into(), enabled: true });
        } else {
            self.ev.manual_links.push(Link { a: p.a, t_a: p.t_a, b: p.b, t_b: p.t_b, error_s: p.error_s, from: "manual".into(), enabled: true });
        }
        self.tls.shift.remove(&p.group);
        self.update_clocks();
    }

    /// The box asking for a dropped row's link error.
    fn pending_box(&mut self, ctx: &egui::Context) {
        let Some(pd) = &self.tls.pending else { return };
        let (na, nb) = (self.clock_name(&pd.a), if pd.absolute { "UTC".to_string() } else { self.clock_name(&pd.b) });
        let Some(p) = &mut self.tls.pending else { return };
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new("Manual link").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            if p.absolute {
                ui.label(format!("{na} {} is {} UTC", clock_view::rel_text(p.t_a), fmt_clock(p.t_b)));
            } else {
                ui.label(format!("{na} {} is {nb} {}", clock_view::rel_text(p.t_a), clock_view::rel_text(p.t_b)));
            }
            ui.horizontal(|ui| {
                ui.label("error ± (1 sigma)");
                ui.add(egui::DragValue::new(&mut p.error_s).range(0.0001..=86_400.0).speed(0.01).suffix(" s"));
            });
            ui.horizontal(|ui| {
                ok = ui.button(if p.absolute { "Anchor" } else { "Link" }).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if ok {
            self.confirm_link();
        } else if cancel {
            self.tls.pending = None;
        }
    }

    /// The Sync list: every anchor and link with an on/off switch.
    fn sync_list(&mut self, ui: &mut egui::Ui, height: f32) {
        let sig = self.ev.conf.sigmas();
        let entries = {
            let cn = |c: &str| self.clock_name(c);
            let who = |f: &str| match (0..self.nodes.len()).find(|&i| self.nodes[i].uid == f) {
                Some(i) => self.node_name(i),
                None => f.to_string(),
            };
            sync_entries(&self.ev.anchors, &self.ev.links, &self.ev.clocks, sig, &cn, &who)
        };
        let bad = entries.iter().filter(|e| e.contradiction.is_some()).count();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Sync").strong().color(TEXT));
            ui.colored_label(DIM, format!("{} anchors, {} links", self.ev.anchors.len(), self.ev.links.len()));
            if bad > 0 {
                ui.colored_label(Color32::from_rgb(235, 80, 80), format!("{bad} contradict"));
            }
        });
        let mut toggle: Option<(String, bool)> = None;
        let mut pick: Option<String> = None;
        egui::ScrollArea::vertical().id_salt("sync-list").max_height((height - 20.0).max(50.0)).auto_shrink([false, true]).show(ui, |ui| {
            if entries.is_empty() {
                ui.colored_label(DIM, "no anchors or links yet: a source's time, a time wired into Planes, ADS-B matches");
            }
            for e in &entries {
                ui.horizontal(|ui| {
                    let mut on = e.enabled;
                    if ui.checkbox(&mut on, "").on_hover_text("use this piece of evidence").changed() {
                        toggle = Some((e.key.clone(), on));
                    }
                    let col = if e.contradiction.is_some() { Color32::from_rgb(235, 80, 80) } else if e.enabled { TEXT } else { DIM };
                    ui.vertical(|ui| {
                        let r = ui.add(egui::Label::new(egui::RichText::new(&e.title).size(11.5).color(col)).sense(Sense::click()));
                        ui.label(egui::RichText::new(&e.detail).size(9.5).color(DIM));
                        if let Some(z) = e.contradiction {
                            ui.label(egui::RichText::new(format!("contradicts the rest ({z:+.1} sigma)")).size(9.5).color(Color32::from_rgb(235, 80, 80)));
                        }
                        if r.clicked() {
                            pick = Some(e.from.clone());
                        }
                        r.on_hover_text("click to select the node that gave it");
                    });
                });
                ui.separator();
            }
        });
        if let Some((k, on)) = toggle {
            self.set_evidence(&k, on);
        }
        if let Some(f) = pick {
            if let Some(i) = (0..self.nodes.len()).find(|&i| self.nodes[i].uid == f) {
                self.pick_only(i);
            }
        }
    }

    /// Play / pause, and whether this is a simulation or live (with Live: back to now).
    pub(crate) fn play_controls(&mut self, ui: &mut egui::Ui) {
        if ui.small_button(if self.ph.playing { "Pause" } else { "Play" }).on_hover_text("Space: play or pause the selected node's playhead").clicked() {
            self.ph.playing = !self.ph.playing;
        }
        if self.is_live() {
            ui.colored_label(Color32::from_rgb(235, 80, 80), "live");
            if ui.small_button("Live").on_hover_text("jump the playhead back to now").clicked() {
                self.go_live();
            }
        } else {
            ui.colored_label(DIM, "simulation");
        }
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
            if ui.small_button(if self.tls.open { "v" } else { "^" }).on_hover_text("show or hide the timeline").clicked() {
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
            ui.separator();
            self.play_controls(ui);
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
        let sync_w = (ui.available_width() * 0.4).min(320.0);
        let rows_w = ui.available_width() - sync_w - 8.0;
        let sync_h = h.max(70.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui(Vec2::new(sync_w, sync_h), |ui| self.sync_list(ui, sync_h));
            ui.separator();
            ui.allocate_ui(Vec2::new(rows_w, 0.0), |ui| {
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
            // A drag on a floating row moves that row; elsewhere it pans.
            let line_at = |y: f32| -> Option<&Row> {
                let li = ((y - area.top() - ruler_h) / lane_h).floor();
                (li >= 0.0).then(|| lines.get(li as usize)).flatten().map(|l| match l {
                    Line::Group(r) | Line::Sub(r, _) => *r,
                })
            };
            if resp.drag_started_by(egui::PointerButton::Primary) {
                let at = ui.input(|i| i.pointer.press_origin());
                self.tls.drag = at.and_then(|h| line_at(h.y)).filter(|r| r.group.kind != Ck::Absolute).map(|r| r.group.id.clone());
            }
            if resp.dragged() {
                let dt = (resp.drag_delta().x / track.width()) as f64 * span;
                match self.tls.drag.clone().and_then(|g| rows.iter().find(|r| r.group.id == g)) {
                    Some(r) => {
                        let cur = shift_of(&self.tls, r, self.tl.0);
                        self.tls.shift.insert(r.group.id.clone(), cur + dt);
                    }
                    None => {
                        self.tl = (self.tl.0 - dt, self.tl.1 - dt);
                        self.follow = false;
                    }
                }
            }
            if resp.drag_stopped() {
                if let Some(g) = self.tls.drag.take() {
                    let per_px = span / track.width() as f64;
                    let v0 = self.tl.0;
                    let clocks = &self.ev.clocks;
                    let found = drop_match(&rows, &g, &|r| shift_of(&self.tls, r, v0), &|c| clocks.get(c).offset, 8.0 * per_px);
                    if let Some(mut m) = found {
                        // Snap the row so the two events meet; the error starts at what a few
                        // pixels are worth at this zoom.
                        let snapped = rows.iter().find(|r| r.group.id == g).map(|r| shift_of(&self.tls, r, v0) + m.gap);
                        if let Some(sh) = snapped {
                            self.tls.shift.insert(g.clone(), sh);
                        }
                        let e = 4.0 * per_px;
                        let mag = 10f64.powf(e.log10().floor());
                        m.error_s = (e / mag).ceil() * mag;
                        m.gap = 0.0;
                        self.tls.pending = Some(m);
                    }
                }
            }
            let (v0, span) = (self.tl.0, (self.tl.1 - self.tl.0).max(1e-3));
            let tx = |t: f64| track.left() + ((t - v0) / span) as f32 * track.width();
            self.tls.geom = Some((track, v0, span));
            self.tls.row_y = lines
                .iter()
                .enumerate()
                .filter_map(|(li, l)| match l {
                    Line::Group(r) => Some((r.group.id.clone(), area.top() + ruler_h + li as f32 * lane_h + lane_h * 0.5)),
                    Line::Sub(..) => None,
                })
                .collect();

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
            let floating_hover = resp.hover_pos().and_then(|h| line_at(h.y)).is_some_and(|r| r.group.kind != Ck::Absolute);
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
                        (format!("{} {who}", if self.tls.expanded.contains(&r.group.id) { "v" } else { ">" }), kind)
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
            // Evidence on the rows: links between them, anchors pinned to the axis.
            let mut ypos: BTreeMap<&str, (f32, f64)> = BTreeMap::new();
            for (li, line) in lines.iter().enumerate() {
                let cy = area.top() + ruler_h + li as f32 * lane_h + lane_h * 0.5;
                match line {
                    Line::Group(r) => {
                        let sh = shift_of(&self.tls, r, v0);
                        for s in &r.subs {
                            ypos.entry(s.clock.as_str()).or_insert((cy, sh));
                        }
                    }
                    Line::Sub(r, s) => {
                        ypos.insert(s.clock.as_str(), (cy, shift_of(&self.tls, r, v0)));
                    }
                }
            }
            let red = Color32::from_rgb(235, 80, 80);
            let mut ev_hover: Option<String> = None;
            let hp = resp.hover_pos();
            let pin = |p: &egui::Painter, x: f32, y: f32, c: Color32, err_px: f32| {
                p.line_segment([Pos2::new(x - err_px, y), Pos2::new(x + err_px, y)], Stroke::new(1.0, c));
                p.add(egui::Shape::convex_polygon(vec![Pos2::new(x, y - 5.0), Pos2::new(x + 5.0, y), Pos2::new(x, y + 5.0), Pos2::new(x - 5.0, y)], c, Stroke::NONE));
            };
            for (i, a) in self.ev.anchors.iter().enumerate() {
                let (true, Some(&(y, _))) = (a.enabled, ypos.get(a.clock.as_str())) else { continue };
                let c = if self.ev.clocks.is_contradiction(Ev::Anchor(i)) { red } else { Color32::from_rgb(240, 220, 120) };
                let x = tx(a.utc);
                if x < track.left() || x > track.right() {
                    continue;
                }
                pin(&p, x, y, c, (tx(a.utc + a.error_s * sig) - x).max(2.0));
                if hp.is_some_and(|h| (h - Pos2::new(x, y)).length() < 8.0) {
                    ev_hover = Some(format!("anchor: {} at {} UTC ± {} ({})", self.clock_name(&a.clock), fmt_clock(a.utc), clock_view::err_text(a.error_s * sig), self.ev.conf.label()));
                }
            }
            for (i, l) in self.ev.links.iter().enumerate() {
                if !l.enabled {
                    continue;
                }
                let c = if self.ev.clocks.is_contradiction(Ev::Link(i)) { red } else { Color32::from_rgb(150, 210, 255) };
                let end = |clock: &str, t: f64| ypos.get(clock).map(|&(y, sh)| (tx(self.ev.clocks.get(clock).offset + t + sh), y));
                let (ea, eb) = if l.a == corroborate::clock::UTC {
                    (None, end(&l.b, l.t_b))
                } else if l.b == corroborate::clock::UTC {
                    (end(&l.a, l.t_a), None)
                } else {
                    (end(&l.a, l.t_a), end(&l.b, l.t_b))
                };
                // A link to UTC is an anchor on the other clock: pin it.
                if let (Some((x, y)), None) | (None, Some((x, y))) = (ea, eb) {
                    pin(&p, x, y, c, 3.0);
                    continue;
                }
                let (Some(a), Some(b)) = (ea, eb) else { continue };
                let err_px = (tx(v0 + l.error_s * sig) - tx(v0)).max(2.0);
                p.line_segment([Pos2::new(a.0, a.1), Pos2::new(b.0, b.1)], Stroke::new(1.5, alpha(c, 0.8)));
                for (x, y) in [a, b] {
                    pin(&p, x, y, c, err_px);
                }
                if hp.is_some_and(|h| seg_dist(h, Pos2::new(a.0, a.1), Pos2::new(b.0, b.1)) < 5.0) {
                    ev_hover = Some(format!(
                        "link: {} {} = {} {}\noffset {:+.2} s ± {} ({})",
                        self.clock_name(&l.a),
                        clock_view::rel_text(l.t_a),
                        self.clock_name(&l.b),
                        clock_view::rel_text(l.t_b),
                        l.t_b - l.t_a,
                        clock_view::err_text(l.error_s * sig),
                        self.ev.conf.label()
                    ));
                }
            }
            if ev_hover.is_some() && hover.is_none() {
                hover = ev_hover;
            }
            // Playheads: one line per row, stronger in the active group; a click on the ruler
            // moves the active group's (or, with nothing selected, the UTC one).
            let ph_col = Color32::from_rgb(255, 210, 90);
            let active = self.active_clock().map(|c| self.ev.clocks.get(&c));
            for (li, line) in lines.iter().enumerate() {
                let r = match line {
                    Line::Group(r) | Line::Sub(r, _) => *r,
                };
                let abs = r.group.kind == Ck::Absolute;
                let Some(pos) = self.ph.get(abs, &r.group.id) else { continue };
                let x = tx(pos + shift_of(&self.tls, r, v0));
                if x < track.left() || x > track.right() {
                    continue;
                }
                let on = active.as_ref().is_some_and(|a| (a.kind == Ck::Absolute && abs) || a.group == r.group.id);
                let top = area.top() + ruler_h + li as f32 * lane_h;
                p.line_segment([Pos2::new(x, top), Pos2::new(x, top + lane_h)], Stroke::new(if on { 2.0 } else { 1.0 }, if on { ph_col } else { ph_col.gamma_multiply(0.5) }));
            }
            if resp.clicked_by(egui::PointerButton::Primary) {
                if let Some(h) = resp.interact_pointer_pos().filter(|h| h.y < area.top() + ruler_h && h.x > track.left()) {
                    let t = v0 + ((h.x - track.left()) / track.width()) as f64 * span;
                    match active.as_ref().filter(|a| a.kind != Ck::Absolute) {
                        Some(a) => {
                            let sh = rows.iter().find(|r| r.group.id == a.group).map_or(0.0, |r| shift_of(&self.tls, r, v0));
                            self.ph.set(false, &a.group, t - sh);
                        }
                        None => self.ph.set(true, "", t),
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
            } else if floating_hover {
                resp.on_hover_text("drag the row onto an event on another row to link them");
            }
        });
            });
        });
        self.pending_box(ui.ctx());
    }
}
