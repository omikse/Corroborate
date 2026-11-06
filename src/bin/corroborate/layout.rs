//! Where a node's ports sit and how its frame looks, from its module.toml (`[look]`, the port look
//! keys) and its look.rhai (`script.rs`), on top of what the workbench does by itself: inputs on
//! the left, outputs on the right, each side's ports one row apart around the middle of the edge.
//! Nothing a look says can add, remove or rewire a port; a wired port is always drawn.

use super::*;
use corroborate::core::Port;
use corroborate::module_manifest::{DotShape, FrameLook, Manifest, PortLabel, PortLook, Side};

/// The outward direction of an edge (wires leave a port this way).
pub fn normal(side: Side) -> Vec2 {
    match side {
        Side::Left => Vec2::new(-1.0, 0.0),
        Side::Right => Vec2::new(1.0, 0.0),
        Side::Top => Vec2::new(0.0, -1.0),
        Side::Bottom => Vec2::new(0.0, 1.0),
    }
}

/// A wire from a port on side `sa` at `a` to one on side `sb` at `b`.
pub fn wire_points(a: Pos2, sa: Side, b: Pos2, sb: Side) -> [Pos2; 4] {
    // Side to side as it always was: half the horizontal distance, at least 40 px.
    let across = matches!(sa, Side::Left | Side::Right) && matches!(sb, Side::Left | Side::Right);
    let d = if across { (b.x - a.x).abs() * 0.5 } else { (b - a).length() * 0.5 }.max(40.0);
    [a, a + normal(sa) * d, b + normal(sb) * d, b]
}

fn color(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

impl App {
    /// Module manifests by id, read again each second (so a module.toml or look.rhai changed in
    /// the modules folder shows at once), plus those of modules carried by nodes.
    pub(crate) fn refresh_looks(&mut self) {
        let unknown = self.nodes.iter().any(|n| matches!(&n.kind, Kind::Module { m, .. } if !self.looks.contains_key(m.id)));
        if !self.looks.is_empty() && !unknown && self.looks_at.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.looks_at = Instant::now();
        let mut map: HashMap<&'static str, Arc<Manifest>> = nodes::modules().into_iter().map(|m| (m.id, Arc::new(m))).collect();
        for n in &self.nodes {
            if let Kind::Module { m, .. } = &n.kind {
                map.entry(m.id).or_insert_with(|| m.clone());
            }
        }
        self.looks = map;
        self.reload_scripts();
    }

    /// The manifest of node `i`'s module (the newest read of it).
    pub(crate) fn manifest_of(&self, i: NodeId) -> Option<Arc<Manifest>> {
        if let Some(m) = self.looks.get(self.graph.nodes()[i].spec.name) {
            return Some(m.clone());
        }
        match &self.nodes[i].kind {
            Kind::Module { m, .. } => Some(m.clone()),
            _ => None,
        }
    }

    /// The frame of node `i`: module.toml's [look], under what its script says now.
    pub(crate) fn frame_look(&self, i: NodeId) -> FrameLook {
        let base = self.manifest_of(i).map(|m| m.frame.clone()).unwrap_or_default();
        match self.script_frame(i) {
            Some(s) => FrameLook { size: s.size.or(base.size), fill: s.fill.or(base.fill), radius: s.radius.or(base.radius), status: s.status.or(base.status) },
            None => base,
        }
    }

    /// How port `p` of node `i` looks: module.toml, under what its script says now.
    pub(crate) fn port_look(&self, i: NodeId, p: &Port) -> PortLook {
        let base = self.manifest_of(i).map(|m| m.port_look(p.name)).unwrap_or_default();
        match self.script_port(i, p.name) {
            Some(over) => base.with(&over),
            None => base,
        }
    }

    /// False for a port its look hides, unless a wire uses it.
    pub(crate) fn port_shown(&self, i: NodeId, p: &Port, out: bool, look: &PortLook) -> bool {
        if !look.hidden {
            return true;
        }
        if out {
            !self.graph.targets(i, p.name).is_empty()
        } else {
            self.graph.feeder(i, p.name).is_some()
        }
    }

    /// Where port `idx` (an output when `out`) of node `i` is on screen, and on which edge.
    pub(crate) fn port_place(&self, origin: Pos2, i: NodeId, idx: usize, out: bool) -> (Pos2, Side) {
        let r = self.node_rect(origin, i);
        let z = self.zoom;
        let home = if out { Side::Right } else { Side::Left };
        if self.nodes[i].collapsed {
            // Collapsed: rows from under the title, inputs left, outputs right.
            let y = r.top() + (HEAD + 3.0 + (idx as f32 + 0.5) * ROW) * z;
            return (Pos2::new(if out { r.right() } else { r.left() }, y), home);
        }
        let spec = &self.graph.nodes()[i].spec;
        let Some(me) = (if out { spec.outputs.get(idx) } else { spec.inputs.get(idx) }) else { return (r.center(), home) };
        let look = self.port_look(i, me);
        let side = look.side.unwrap_or(home);
        let edge = |d: f32| match side {
            Side::Left => Pos2::new(r.left(), r.top() + d),
            Side::Right => Pos2::new(r.right(), r.top() + d),
            Side::Top => Pos2::new(r.left() + d, r.top()),
            Side::Bottom => Pos2::new(r.left() + d, r.bottom()),
        };
        if let Some(at) = look.at {
            return (edge(at * z), side);
        }
        // Spaced evenly: the shown ports on this edge that have no `at` of their own.
        let mut same: Vec<(bool, usize)> = Vec::new();
        for (o, list) in [(false, &spec.inputs), (true, &spec.outputs)] {
            let h = if o { Side::Right } else { Side::Left };
            for (k, p) in list.iter().enumerate() {
                let l = if (o, k) == (out, idx) { look.clone() } else { self.port_look(i, p) };
                if l.side.unwrap_or(h) == side && l.at.is_none() && ((o, k) == (out, idx) || self.port_shown(i, p, o, &l)) {
                    same.push((o, k));
                }
            }
        }
        let n = same.len().max(1) as f32;
        let k = same.iter().position(|&x| x == (out, idx)).unwrap_or(0) as f32;
        match side {
            Side::Left | Side::Right => (edge(r.height() / 2.0 + (k - (n - 1.0) / 2.0) * ROW * z), side),
            Side::Top | Side::Bottom => (edge(r.width() * (k + 1.0) / (n + 1.0)), side),
        }
    }
}

/// A port's dot and name. `filled` false draws a ring (an optional input not yet wired).
#[allow(clippy::too_many_arguments)]
pub fn draw_port(p: &egui::Painter, c: Pos2, side: Side, look: &PortLook, col: Color32, filled: bool, name: &str, text_col: Color32, z: f32) {
    let col = look.color.map_or(col, color);
    let s = look.dot_size.unwrap_or(5.0) * z;
    match look.dot.unwrap_or(DotShape::Circle) {
        DotShape::Circle if filled => {
            p.circle_filled(c, s, col);
        }
        DotShape::Circle => {
            p.circle_filled(c, s, NODE);
            p.circle_stroke(c, s - 0.5 * z, Stroke::new(1.6 * z, col));
        }
        DotShape::Square => {
            let r = Rect::from_center_size(c, Vec2::splat(s * 1.8));
            if filled {
                p.rect_filled(r, 1.0 * z, col);
            } else {
                p.rect_filled(r, 1.0 * z, NODE);
                p.rect_stroke(r, 1.0 * z, Stroke::new(1.6 * z, col), StrokeKind::Inside);
            }
        }
        DotShape::Diamond => {
            let pts = vec![c + Vec2::new(0.0, -s * 1.2), c + Vec2::new(s * 1.2, 0.0), c + Vec2::new(0.0, s * 1.2), c + Vec2::new(-s * 1.2, 0.0)];
            let fill = if filled { col } else { NODE };
            p.add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.6 * z, col)));
        }
    }
    let label = look.label.unwrap_or(PortLabel::Inside);
    if label == PortLabel::Hidden || name.is_empty() {
        return;
    }
    // Inside: towards the node's middle; outside: away from it.
    let out = normal(side) * if label == PortLabel::Outside { 1.0 } else { -1.0 };
    let at = c + out * (s + 5.0 * z);
    let align = match (out.x > 0.5, out.x < -0.5, out.y > 0.5) {
        (true, _, _) => Align2::LEFT_CENTER,
        (_, true, _) => Align2::RIGHT_CENTER,
        (_, _, true) => Align2::CENTER_TOP,
        _ => Align2::CENTER_BOTTOM,
    };
    p.text(at, align, name, FontId::proportional(11.5 * z), text_col);
}
