//! Standard canvas editing, kept apart from drawing (main.rs `graph_view`):
//!
//! - select: click a node; Shift- or Ctrl-click adds or removes it; drag a box on empty space
//!   (Shift or Ctrl adds to what is selected); Ctrl+A selects all; Escape clears;
//! - move: drag any selected node and all selected nodes move together;
//! - pan: middle-drag, or hold Space and drag; F or Home frames the selection (else everything);
//! - delete: Delete or Backspace;
//! - copy, cut, paste, duplicate: Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+D. The clipboard holds project
//!   text (the same lines as the project file), so nodes paste into another Corroborate too and
//!   can be read in any text editor;
//! - undo, redo: Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z. Anything that changes the project is one step:
//!   moves, wires, settings, names, collapsing, adding, deleting. Undoing a setting is logged
//!   in the result file like any other setting change.
//!
//! Keys do nothing while a text box has the keyboard (it has its own copy, paste and undo).

use super::*;
use std::collections::BTreeSet;

/// One node as undo keeps it.
#[derive(Clone)]
pub struct NodeSnap {
    kind: Kind,
    label: String,
    pos: Pos2,
    collapsed: bool,
}

/// The project at one moment: nodes by id (`None`: deleted) and wires by port name.
#[derive(Clone)]
pub struct Snap {
    nodes: Vec<Option<NodeSnap>>,
    wires: Vec<(NodeId, String, NodeId, String)>,
    key: String,
}

/// Undo steps. A change becomes a step once it has held for a moment with no button down, so
/// a drag or a burst of typing is one step; undo takes any change not yet a step first.
pub struct History {
    snaps: Vec<Snap>,
    at: usize,
    seen: (String, Instant),
}

impl Default for History {
    fn default() -> Self {
        Self { snaps: Vec::new(), at: 0, seen: (String::new(), Instant::now()) }
    }
}

const STEPS: usize = 200;
const SETTLE: Duration = Duration::from_millis(400);
const CLIP_HEAD: &str = "# corroborate project";

/// Selection kept beside `App::selected` (the node whose trail lights up).
#[derive(Default)]
pub struct Selection {
    pub picked: BTreeSet<NodeId>,
    /// Box being dragged, from this screen point.
    pub boxing: Option<Pos2>,
    /// Last copy, for when the system clipboard cannot be read.
    pub clip: String,
}

impl App {
    pub(crate) fn is_picked(&self, i: NodeId) -> bool {
        self.sel.picked.contains(&i)
    }

    pub(crate) fn pick_only(&mut self, i: NodeId) {
        self.sel.picked = BTreeSet::from([i]);
        self.selected = Some(i);
    }

    pub(crate) fn pick_toggle(&mut self, i: NodeId) {
        if !self.sel.picked.remove(&i) {
            self.sel.picked.insert(i);
            self.selected = Some(i);
        } else if self.selected == Some(i) {
            self.selected = self.sel.picked.iter().next_back().copied();
        }
    }

    pub(crate) fn pick_none(&mut self) {
        self.sel.picked.clear();
        self.selected = None;
    }

    pub(crate) fn pick_all(&mut self) {
        self.sel.picked = (0..self.nodes.len()).filter(|&i| !self.nodes[i].hidden).collect();
        self.selected = self.sel.picked.iter().next_back().copied();
    }

    /// Selected nodes still on the canvas, in id order.
    pub(crate) fn picked(&self) -> Vec<NodeId> {
        self.sel.picked.iter().copied().filter(|&i| i < self.nodes.len() && !self.nodes[i].hidden).collect()
    }

    /// Deletes nodes: their wires go, recordings into them end; ids stay (undo brings them back).
    pub(crate) fn delete_nodes(&mut self, ids: &[NodeId]) {
        if ids.is_empty() {
            return;
        }
        for &i in ids {
            if self.recordings.contains_key(&i) {
                self.record_end(i);
            }
            self.graph.isolate(i);
            self.nodes[i].hidden = true;
        }
        self.sync_many_ports();
        self.pick_none();
        self.msg = format!("deleted {} node(s)", ids.len());
        self.save_project();
    }

    // ---------------- clipboard

    pub(crate) fn copy(&mut self, ctx: &egui::Context) -> usize {
        let ids = self.picked();
        if ids.is_empty() {
            return 0;
        }
        let text = self.nodes_text(&ids);
        ctx.copy_text(text.clone());
        self.sel.clip = text;
        self.msg = format!("copied {} node(s)", ids.len());
        ids.len()
    }

    /// Adds the nodes of copied project text, their top-left corner at `at` (canvas units), and
    /// selects them. Text that is not Corroborate nodes is ignored.
    pub(crate) fn paste(&mut self, text: &str, at: Pos2) -> Vec<NodeId> {
        let text = if text.trim_start().starts_with(CLIP_HEAD) { text.to_string() } else { return Vec::new() };
        let new = self.append_text(&text, false);
        if new.is_empty() {
            return new;
        }
        let min = new.iter().map(|&i| self.nodes[i].pos).fold(Pos2::new(f32::MAX, f32::MAX), |a, p| a.min(p));
        for &i in &new {
            self.nodes[i].pos += at - min;
        }
        self.sel.picked = new.iter().copied().collect();
        self.selected = new.last().copied();
        self.msg = format!("pasted {} node(s)", new.len());
        self.save_project();
        new
    }

    /// Copies of the selected nodes and the wires between them, a little down and right.
    pub(crate) fn duplicate(&mut self) -> Vec<NodeId> {
        let ids = self.picked();
        if ids.is_empty() {
            return ids;
        }
        let min = ids.iter().map(|&i| self.nodes[i].pos).fold(Pos2::new(f32::MAX, f32::MAX), |a, p| a.min(p));
        let text = self.nodes_text(&ids);
        self.paste(&text, min + Vec2::splat(30.0))
    }

    // ---------------- undo

    /// What undo compares: every node on the canvas and every wire.
    fn snap_key(&self) -> String {
        let mut k = String::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if n.hidden {
                continue;
            }
            let name = self.graph.nodes()[i].spec.name;
            k.push_str(&format!("{i}|{name}|{}|{:?}|{}|{}|{}|{}\n", n.kind.saved_param(), n.kind.rows(), n.label, n.pos.x, n.pos.y, n.collapsed));
        }
        let mut w: Vec<String> = self
            .graph
            .wires()
            .iter()
            .map(|w| {
                let out = self.graph.nodes()[w.from].spec.outputs[w.from_port].name;
                // A record's input names follow the order it was wired in: not part of the step.
                let inp = if matches!(self.nodes[w.to].kind, Kind::ResultRecord { .. }) { "" } else { self.graph.nodes()[w.to].spec.inputs[w.to_port].name };
                format!("{}.{out}>{}.{inp}", w.from, w.to)
            })
            .collect();
        w.sort();
        k.push_str(&w.join(";"));
        k
    }

    fn snap(&self, key: String) -> Snap {
        let nodes = self
            .nodes
            .iter()
            .map(|n| (!n.hidden).then(|| NodeSnap { kind: n.kind.clone(), label: n.label.clone(), pos: n.pos, collapsed: n.collapsed }))
            .collect();
        let wires = self
            .graph
            .wires()
            .iter()
            .map(|w| {
                let g = self.graph.nodes();
                (w.from, g[w.from].spec.outputs[w.from_port].name.to_string(), w.to, g[w.to].spec.inputs[w.to_port].name.to_string())
            })
            .collect();
        Snap { nodes, wires, key }
    }

    /// Makes the current project an undo step if it differs from the last one.
    fn commit(&mut self, key: String) {
        let h = &self.history;
        if h.snaps.get(h.at).is_some_and(|s| s.key == key) {
            return;
        }
        let s = self.snap(key);
        let h = &mut self.history;
        if !h.snaps.is_empty() {
            h.snaps.truncate(h.at + 1);
        }
        h.snaps.push(s);
        if h.snaps.len() > STEPS {
            h.snaps.remove(0);
        }
        h.at = h.snaps.len() - 1;
    }

    /// Each frame: a change that has settled becomes an undo step.
    pub(crate) fn history_tick(&mut self, ctx: &egui::Context) {
        let key = self.snap_key();
        if self.history.snaps.is_empty() {
            self.commit(key);
            return;
        }
        if self.history.snaps[self.history.at].key == key {
            return;
        }
        if self.history.seen.0 != key {
            self.history.seen = (key, Instant::now());
            ctx.request_repaint_after(SETTLE);
            return;
        }
        if self.history.seen.1.elapsed() >= SETTLE && !ctx.input(|i| i.pointer.any_down()) {
            self.commit(key);
        } else {
            ctx.request_repaint_after(SETTLE);
        }
    }

    pub(crate) fn undo(&mut self) -> bool {
        self.commit(self.snap_key());
        if self.history.at == 0 {
            self.msg = "nothing to undo".into();
            return false;
        }
        self.history.at -= 1;
        self.restore();
        self.msg = "undone".into();
        true
    }

    pub(crate) fn redo(&mut self) -> bool {
        self.commit(self.snap_key());
        if self.history.at + 1 >= self.history.snaps.len() {
            self.msg = "nothing to redo".into();
            return false;
        }
        self.history.at += 1;
        self.restore();
        self.msg = "redone".into();
        true
    }

    /// Puts the project back as the current undo step has it. Nodes keep their ids and what
    /// they have worked out; a node the step lacks is deleted, one it has comes back.
    fn restore(&mut self) {
        let s = self.history.snaps[self.history.at].clone();
        for i in 0..self.nodes.len() {
            match s.nodes.get(i).cloned().flatten() {
                Some(n) => {
                    let old = self.nodes[i].kind.settings();
                    for (name, v) in n.kind.settings() {
                        let was = old.iter().find(|(k, _)| *k == name).map(|(_, o)| o.clone()).unwrap_or_default();
                        if was != v && !self.nodes[i].hidden {
                            let label = n.label.clone();
                            self.changes.push(bus::SettingChange { utc: timebase::now_unix(), node: self.nodes[i].uid.clone(), label, name, old: was, new: v });
                        }
                    }
                    let d = &mut self.nodes[i];
                    d.kind = n.kind;
                    d.label = n.label;
                    d.pos = n.pos;
                    d.collapsed = n.collapsed;
                    d.hidden = false;
                }
                None if !self.nodes[i].hidden => {
                    if self.recordings.contains_key(&i) {
                        self.record_end(i);
                    }
                    self.nodes[i].hidden = true;
                }
                None => {}
            }
            self.graph.isolate(i);
        }
        self.sync_many_ports();
        for (a, o, b, inp) in &s.wires {
            if *a < self.nodes.len() && *b < self.nodes.len() {
                if matches!(self.nodes[*b].kind, Kind::ResultRecord { .. }) {
                    self.wire_many(*a, o, *b);
                } else {
                    let _ = self.graph.connect(*a, o, *b, inp);
                }
            }
        }
        self.sync_many_ports();
        self.sel.picked.retain(|&i| !self.nodes[i].hidden);
        if self.selected.is_some_and(|i| self.nodes[i].hidden) {
            self.selected = None;
        }
        self.wiring = None;
        self.wiring_in = None;
        self.drop_menu = None;
        self.drafts.clear();
        self.setting_errors.clear();
        self.save_project();
    }

    // ---------------- keys and view

    /// Canvas keys and clipboard events, unless a text box has the keyboard. `at`: where a
    /// paste goes (canvas units).
    pub(crate) fn canvas_keys(&mut self, ctx: &egui::Context, area: Rect, at: Pos2) {
        if ctx.text_edit_focused() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for e in events {
            match e {
                egui::Event::Copy => {
                    self.copy(ctx);
                }
                egui::Event::Cut => {
                    if self.copy(ctx) > 0 {
                        self.delete_nodes(&self.picked());
                    }
                }
                egui::Event::Paste(text) => {
                    let text = if text.trim_start().starts_with(CLIP_HEAD) { text } else { self.sel.clip.clone() };
                    self.paste(&text, at);
                }
                egui::Event::Key { key, pressed: true, modifiers: m, .. } => match key {
                    egui::Key::Delete | egui::Key::Backspace if !m.any() => self.delete_nodes(&self.picked()),
                    egui::Key::A if m.command => self.pick_all(),
                    egui::Key::D if m.command => {
                        self.duplicate();
                    }
                    egui::Key::Z if m.command && m.shift => {
                        self.redo();
                    }
                    egui::Key::Z if m.command => {
                        self.undo();
                    }
                    egui::Key::Y if m.command => {
                        self.redo();
                    }
                    egui::Key::Escape => {
                        self.pick_none();
                        self.wiring = None;
                        self.wiring_in = None;
                        self.sel.boxing = None;
                    }
                    egui::Key::F | egui::Key::Home if !m.any() => self.frame_nodes(area),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    /// Pans and zooms so the selected nodes (else all) fill the canvas.
    pub(crate) fn frame_nodes(&mut self, area: Rect) {
        let ids = if self.sel.picked.is_empty() { (0..self.nodes.len()).filter(|&i| !self.nodes[i].hidden).collect() } else { self.picked() };
        if ids.is_empty() {
            return;
        }
        // Node rects in canvas units (no pan, no zoom).
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        let bounds = ids.iter().map(|&i| self.node_rect(Pos2::ZERO, i)).reduce(|a, b| a.union(b)).unwrap();
        let margin = 60.0;
        self.zoom = ((area.width() - margin) / bounds.width()).min((area.height() - margin) / bounds.height()).clamp(0.4, 1.5);
        self.pan = area.center() - area.min - bounds.center().to_vec2() * self.zoom;
    }

    /// The selection box: started by right-dragging empty canvas, drawn while dragging, applied
    /// on release to every node it touches. With Shift or Ctrl those nodes are toggled; with
    /// neither, they replace the selection.
    pub(crate) fn box_select(&mut self, ui: &egui::Ui, bg: &egui::Response, painter: &egui::Painter, origin: Pos2) {
        if bg.drag_started_by(egui::PointerButton::Secondary) && self.wiring.is_none() && self.wiring_in.is_none() {
            self.sel.boxing = ui.input(|i| i.pointer.press_origin());
        }
        let Some(start) = self.sel.boxing else { return };
        let Some(now) = ui.ctx().pointer_latest_pos() else { return };
        let r = Rect::from_two_pos(start, now);
        painter.rect(r, 2.0, alpha(SEL, 0.08), Stroke::new(1.0, alpha(SEL, 0.7)), StrokeKind::Inside);
        if bg.drag_stopped() || !ui.input(|i| i.pointer.secondary_down()) {
            self.sel.boxing = None;
            let m = ui.input(|i| i.modifiers);
            let toggle = m.shift || m.command;
            if !toggle {
                self.pick_none();
            }
            for i in 0..self.nodes.len() {
                if !self.nodes[i].hidden && self.node_rect(origin, i).intersects(r) {
                    if toggle && self.sel.picked.contains(&i) {
                        self.sel.picked.remove(&i);
                        if self.selected == Some(i) {
                            self.selected = self.sel.picked.iter().next().copied();
                        }
                    } else {
                        self.sel.picked.insert(i);
                        self.selected = Some(i);
                    }
                }
            }
        }
    }
}
