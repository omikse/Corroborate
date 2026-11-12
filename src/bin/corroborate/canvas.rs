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
//! - group: Ctrl+G groups the selected nodes, Tab opens and closes a group, Alt+G ungroups
//!   (groups.rs);
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
    parent: Option<NodeId>,
    member: String,
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
    /// The selection when the box started (the box adds to or toggles it with Shift or Ctrl),
    /// and which button drags it.
    pub box_base: BTreeSet<NodeId>,
    pub box_button: Option<egui::PointerButton>,
    pub box_toggle: bool,
    /// Last copy, for when the system clipboard cannot be read.
    pub clip: String,
}

impl App {
    pub(crate) fn is_picked(&self, i: NodeId) -> bool {
        self.sel.picked.contains(&i)
    }

    /// Selects node `i` alone. A node inside another group (picked on the timeline, in the
    /// recordings list) brings the canvas to that group.
    pub(crate) fn pick_only(&mut self, i: NodeId) {
        if i < self.nodes.len() && !self.nodes[i].hidden && self.nodes[i].parent != self.view {
            self.view = self.nodes[i].parent;
            let mut depth = 0;
            let mut c = self.view;
            while let Some(g) = c {
                depth += 1;
                c = self.nodes[g].parent;
            }
            self.view_stack = vec![(self.pan, self.zoom); depth];
        }
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
        self.sel.picked = (0..self.nodes.len()).filter(|&i| self.on_canvas(i)).collect();
        self.selected = self.sel.picked.iter().next_back().copied();
    }

    /// Selected nodes still on the canvas, in id order.
    pub(crate) fn picked(&self) -> Vec<NodeId> {
        self.sel.picked.iter().copied().filter(|&i| i < self.nodes.len() && self.on_canvas(i)).collect()
    }

    /// Deletes nodes: their wires go, recordings into them end; ids stay (undo brings them back).
    pub(crate) fn delete_nodes(&mut self, ids: &[NodeId]) {
        // A group goes with what is inside it; a group's Group Input and Output stay.
        let ids: Vec<NodeId> = ids.iter().copied().filter(|&i| !matches!(self.nodes[i].kind, Kind::GroupInput | Kind::GroupOutput)).collect();
        if ids.is_empty() {
            return;
        }
        let shown = ids.len();
        let ids = self.with_inside(&ids);
        for &i in &ids {
            if self.recordings.contains_key(&i) {
                self.record_end(i);
            }
            self.graph.isolate(i);
            self.nodes[i].hidden = true;
        }
        self.sync_many_ports();
        self.pick_none();
        self.msg = format!("deleted {shown} node(s)");
        self.save_project();
    }

    // ---------------- clipboard

    pub(crate) fn copy(&mut self, ctx: &egui::Context) -> usize {
        let ids = self.picked();
        if ids.is_empty() {
            return 0;
        }
        let text = self.nodes_text(&self.with_inside(&ids));
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
        // Moved are the nodes pasted at this level; what is inside a pasted group keeps its place there.
        let top: Vec<NodeId> = new.iter().copied().filter(|&i| self.nodes[i].parent == self.view).collect();
        let min = top.iter().map(|&i| self.nodes[i].pos).fold(Pos2::new(f32::MAX, f32::MAX), |a, p| a.min(p));
        for &i in &top {
            self.nodes[i].pos += at - min;
        }
        let new = top;
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
        let text = self.nodes_text(&self.with_inside(&ids));
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
            k.push_str(&format!("{i}|{name}|{}|{:?}|{}|{}|{}|{}|{:?}|{}\n", n.kind.saved_param(), n.kind.rows(), n.label, n.pos.x, n.pos.y, n.collapsed, n.parent, n.member));
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
            .map(|n| (!n.hidden).then(|| NodeSnap { kind: n.kind.clone(), label: n.label.clone(), pos: n.pos, collapsed: n.collapsed, parent: n.parent, member: n.member.clone() }))
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

    /// The folder `old` is now `new`: undo steps read the recordings inside it from there too.
    pub(crate) fn history_paths_moved(&mut self, old: &std::path::Path, new: &std::path::Path) {
        for n in self.history.snaps.iter_mut().flat_map(|s| s.nodes.iter_mut().flatten()) {
            if let Kind::Sourcer { input } = &mut n.kind {
                if let Some(to) = Self::file_of(input).and_then(|f| recordings::moved_with(old, new, &f)) {
                    *input = to.to_string_lossy().into_owned();
                }
            }
        }
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
                    d.parent = n.parent;
                    d.member = n.member;
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
            if matches!(self.nodes[i].kind, Kind::Time { .. }) {
                let inputs = self.nodes[i].kind.spec().inputs;
                self.graph.set_inputs_keep(i, inputs);
            }
            if matches!(self.nodes[i].kind, Kind::CutDetector { .. }) {
                let outputs = self.nodes[i].kind.spec().outputs;
                self.graph.set_outputs(i, outputs);
            }
        }
        self.sync_many_ports();
        for (a, o, b, inp) in &s.wires {
            if *a < self.nodes.len() && *b < self.nodes.len() {
                if matches!(self.nodes[*b].kind, Kind::ResultRecord { .. }) {
                    self.wire_many(*a, o, *b);
                } else {
                    let _ = self.connect_grown(*a, o, *b, inp);
                }
            }
        }
        self.sync_many_ports();
        self.sel.picked.retain(|&i| !self.nodes[i].hidden);
        // Inside a group the step does not have: back to the top.
        if self.view.is_some_and(|g| self.nodes[g].hidden) {
            self.view = None;
            self.view_stack.clear();
        }
        self.sync_groups();
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
                    egui::Key::G if m.command => {
                        self.group_selected();
                    }
                    egui::Key::G if m.alt => {
                        self.ungroup_selected();
                    }
                    egui::Key::Tab if !m.any() => self.tab(area),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    /// Pans and zooms so the selected nodes (else all) fill the canvas.
    pub(crate) fn frame_nodes(&mut self, area: Rect) {
        let ids = if self.picked().is_empty() { (0..self.nodes.len()).filter(|&i| self.on_canvas(i)).collect() } else { self.picked() };
        self.frame_ids(area, ids);
    }

    /// Lays out the nodes on this level by their wires (`node_layout`) and shows them all. One
    /// undo step.
    pub(crate) fn clean_up(&mut self, t: f64) {
        use corroborate::node_layout::{place, Item, Link};
        let ids: Vec<NodeId> = (0..self.nodes.len()).filter(|&i| self.on_canvas(i)).collect();
        if ids.is_empty() {
            return;
        }
        // Whatever just changed is its own step, so that undo brings back exactly this layout.
        self.commit(self.snap_key());
        // Sizes and ports in canvas units (no pan, no zoom).
        let (pan, zoom) = (self.pan, self.zoom);
        (self.pan, self.zoom) = (Vec2::ZERO, 1.0);
        // Scripts run only for the nodes on screen: the rest would give a stale size. Twice,
        // because a script is told the size its node had.
        for _ in 0..2 {
            self.run_scripts(Pos2::ZERO, Rect::EVERYTHING, t);
        }
        let items: Vec<Item> = ids
            .iter()
            .map(|&i| {
                let r = self.node_rect(Pos2::ZERO, i);
                Item { x: r.left(), y: r.top(), w: r.width(), h: r.height() }
            })
            .collect();
        let slot = |n: NodeId| ids.iter().position(|&i| i == n);
        let links: Vec<Link> = self
            .shown_lines()
            .iter()
            .filter_map(|s| {
                let (from, to) = (slot(s.from)?, slot(s.to)?);
                let from_y = self.port_place(Pos2::ZERO, s.from, s.from_port, true).0.y - items[from].y;
                let to_y = self.port_place(Pos2::ZERO, s.to, s.to_port, false).0.y - items[to].y;
                Some(Link { from, from_y, to, to_y })
            })
            .collect();
        (self.pan, self.zoom) = (pan, zoom);
        for (&i, (x, y)) in ids.iter().zip(place(&items, &links)) {
            self.nodes[i].pos = Pos2::new(x, y);
        }
        self.frame_all = true;
        self.msg = format!("arranged {} nodes (Ctrl+Z puts them back)", ids.len());
        self.save_project();
    }

    /// Pans and zooms so these nodes fill the canvas.
    pub(crate) fn frame_ids(&mut self, area: Rect, ids: Vec<NodeId>) {
        if ids.is_empty() {
            return;
        }
        // Node rects in canvas units (no pan, no zoom).
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
        let bounds = ids.iter().map(|&i| self.node_rect(Pos2::ZERO, i)).reduce(|a, b| a.union(b)).unwrap();
        let margin = 60.0;
        self.zoom = ((area.width() - margin) / bounds.width()).min((area.height() - margin) / bounds.height()).clamp(MIN_ZOOM, 1.5);
        self.pan = area.center() - area.min - bounds.center().to_vec2() * self.zoom;
    }

    /// The selection box: started by dragging empty canvas (left or right button), drawn above
    /// the nodes while dragging. Every node it touches is selected at once, as the box grows or
    /// shrinks. With Shift or Ctrl held, those nodes are toggled against the selection from
    /// before; with neither, they replace it (a box over nothing clears it).
    pub(crate) fn box_select(&mut self, ui: &egui::Ui, bg: &egui::Response, painter: &egui::Painter, origin: Pos2) {
        for b in [egui::PointerButton::Primary, egui::PointerButton::Secondary] {
            if bg.drag_started_by(b) && self.wiring.is_none() && self.wiring_in.is_none() {
                self.sel.boxing = ui.input(|i| i.pointer.press_origin());
                self.sel.box_button = Some(b);
                self.sel.box_base = self.sel.picked.clone();
                self.sel.box_toggle = false;
            }
        }
        let Some(start) = self.sel.boxing else { return };
        let Some(now) = ui.ctx().pointer_latest_pos() else { return };
        let r = Rect::from_two_pos(start, now);
        // Above the nodes, clipped to the canvas.
        let top = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("box-select"))).with_clip_rect(painter.clip_rect());
        top.rect(r, 2.0, alpha(SEL, 0.08), Stroke::new(1.0, alpha(SEL, 0.8)), StrokeKind::Inside);
        // Live: the selection is the base with every touched node added (or toggled).
        // Shift or Ctrl at any time during the drag makes it a toggling box.
        self.sel.box_toggle |= ui.input(|i| i.modifiers.shift || i.modifiers.command);
        let toggle = self.sel.box_toggle;
        let mut picked = if toggle { self.sel.box_base.clone() } else { BTreeSet::new() };
        let mut last = None;
        for i in 0..self.nodes.len() {
            if self.on_canvas(i) && self.node_rect(origin, i).intersects(r) {
                if toggle && self.sel.box_base.contains(&i) {
                    picked.remove(&i);
                } else {
                    picked.insert(i);
                    last = Some(i);
                }
            }
        }
        if picked != self.sel.picked {
            self.sel.picked = picked;
            self.selected = last.or_else(|| self.selected.filter(|s| self.sel.picked.contains(s))).or_else(|| self.sel.picked.iter().next().copied());
        }
        let down = match self.sel.box_button {
            Some(egui::PointerButton::Primary) => ui.input(|i| i.pointer.primary_down()),
            _ => ui.input(|i| i.pointer.secondary_down()),
        };
        if bg.drag_stopped() || !down {
            self.sel.boxing = None;
            self.sel.box_button = None;
            self.sel.box_base.clear();
        }
    }
}
