//! Node groups, as in Blender:
//!
//! - Ctrl+G puts the selected nodes into a new group: one node in their place, with an input
//!   for each wire coming in and an output for each wire going out.
//! - Tab opens the selected group (its nodes, between a Group Input and a Group Output node);
//!   Tab again, or the path at the top left, goes back out. Alt+G ungroups: the nodes come back out.
//! - Inside, the Group Input's outputs are the group's inputs and the Group Output's inputs are its
//!   outputs. Wiring a node to the empty `+` port of either adds a port.
//! - Copies of a group (Ctrl+D, copy and paste, Add node → Groups) are uses of one definition: a
//!   change inside one of them (a node added, moved, set or wired) is made in all of them. Each
//!   use keeps its own wires outside, and its nodes keep their own results.
//!
//! Under the view there is one flat graph: the nodes inside a group are ordinary nodes (their
//! `parent` is the group), and a wire into a group is a wire into the node inside it. So running,
//! clocks, the timeline and the result file see through groups. What a group adds is which nodes
//! the canvas shows at once, and its ports: `in` and `out` rows on the group node naming which
//! inner node's port each one is (by stable id), so a port stays even while nothing outside is wired
//! to it.

use super::*;
use corroborate::core::{NodeSpec, Port};
use corroborate::module_manifest as mm;
use std::collections::HashSet;

/// Project-file names of the three node kinds.
pub const GROUP: &str = "group";
pub const GROUP_INPUT: &str = "group-input";
pub const GROUP_OUTPUT: &str = "group-output";
/// The empty port at the end of the Group Input and Group Output nodes.
const NEW_PORT: &str = "+";

/// One port of a group: its name and kind, and the ports inside that it is. An input may feed
/// several inner inputs; an output is one inner output.
#[derive(Clone, Debug, PartialEq)]
pub struct GPort {
    pub name: String,
    pub kind: PortKind,
    /// (stable node id, port name).
    pub ends: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Iface {
    pub inputs: Vec<GPort>,
    pub outputs: Vec<GPort>,
}

impl Iface {
    /// Read from a group's rows: `in` / `out` = `name;kind;id@port,id@port`.
    pub fn of(rows: &[(String, String)]) -> Iface {
        let mut f = Iface::default();
        for (k, v) in rows {
            let mut parts = v.splitn(3, ';');
            let (Some(name), Some(kind), ends) = (parts.next(), parts.next(), parts.next().unwrap_or("")) else { continue };
            let ends = ends.split(',').filter_map(|e| e.split_once('@')).map(|(u, p)| (u.to_string(), p.to_string())).collect();
            let p = GPort { name: name.to_string(), kind: mm::port_kind(kind).unwrap_or(PortKind::Any), ends };
            match k.as_str() {
                "in" => f.inputs.push(p),
                "out" => f.outputs.push(p),
                _ => {}
            }
        }
        f
    }

    pub fn rows(&self) -> Vec<(String, String)> {
        let row = |k: &str, p: &GPort| {
            let ends: Vec<String> = p.ends.iter().map(|(u, q)| format!("{u}@{q}")).collect();
            (k.to_string(), format!("{};{};{}", p.name, mm::kind_id(p.kind), ends.join(",")))
        };
        self.inputs.iter().map(|p| row("in", p)).chain(self.outputs.iter().map(|p| row("out", p))).collect()
    }

    pub fn ports(&self, list: &[GPort]) -> Vec<Port> {
        list.iter().map(|p| Port::opt(mm::intern(&p.name), p.kind)).collect()
    }

    /// `rows` with the ids changed by `f` (pasted nodes get new ids).
    pub fn rows_mapped(&self, f: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
        self.map_ids(f).rows()
    }

    /// The ids in it changed by `f` (an end whose id `f` drops is dropped).
    fn map_ids(&self, f: impl Fn(&str) -> Option<String>) -> Iface {
        let m = |ps: &[GPort]| {
            ps.iter()
                .map(|p| GPort { ends: p.ends.iter().filter_map(|(u, q)| Some((f(u)?, q.clone()))).collect(), ..p.clone() })
                .collect()
        };
        Iface { inputs: m(&self.inputs), outputs: m(&self.outputs) }
    }
}

/// A name not in `taken`: `base`, else `base 2`, `base 3`…
fn unique(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base} {n}")).find(|c| !taken.contains(c)).unwrap()
}

/// A wire as the canvas shows it at one level: between nodes shown there (a group stands for
/// the nodes inside it; the Group Input and Output nodes for what is outside), and the real
/// ports it stands for.
#[derive(Clone, Debug)]
pub struct Shown {
    pub from: NodeId,
    pub from_port: usize,
    pub to: NodeId,
    pub to_port: usize,
    /// The real output, if a real wire is behind it (inside a group, a port wired to nothing
    /// outside is still shown from the Group Input).
    pub real_from: Option<(NodeId, usize)>,
    pub real_to: (NodeId, &'static str),
}

impl App {
    // ---------------- where nodes are

    /// On the canvas now: not deleted, and in the group being shown.
    pub(crate) fn on_canvas(&self, i: NodeId) -> bool {
        !self.nodes[i].hidden && self.nodes[i].parent == self.view
    }

    /// What stands for node `x` at level `level`: itself, or the group there that holds it. None:
    /// `x` is not inside `level`.
    pub(crate) fn rep(&self, x: NodeId, level: Option<NodeId>) -> Option<NodeId> {
        let mut c = x;
        for _ in 0..64 {
            if self.nodes[c].parent == level {
                return Some(c);
            }
            c = self.nodes[c].parent?;
        }
        None
    }

    /// True when `x` is inside group `g` (at any depth).
    pub(crate) fn is_inside(&self, x: NodeId, g: NodeId) -> bool {
        x != g && self.rep(x, Some(g)).is_some()
    }

    /// Every node inside group `g`, at any depth.
    pub(crate) fn inside(&self, g: NodeId) -> Vec<NodeId> {
        (0..self.nodes.len()).filter(|&x| !self.nodes[x].hidden && self.is_inside(x, g)).collect()
    }

    /// `ids` and everything inside the groups among them.
    pub(crate) fn with_inside(&self, ids: &[NodeId]) -> Vec<NodeId> {
        let mut all: Vec<NodeId> = (0..self.nodes.len()).filter(|&x| !self.nodes[x].hidden && ids.iter().any(|&g| x == g || self.is_inside(x, g))).collect();
        all.dedup();
        all
    }

    fn by_uid(&self, u: &str) -> Option<NodeId> {
        self.nodes.iter().position(|n| n.uid == u && !n.hidden)
    }

    pub(crate) fn iface(&self, g: NodeId) -> Iface {
        match &self.nodes[g].kind {
            Kind::Group { rows, .. } => Iface::of(rows),
            _ => Iface::default(),
        }
    }

    fn set_iface(&mut self, g: NodeId, f: &Iface) {
        if let Kind::Group { rows, .. } = &mut self.nodes[g].kind {
            *rows = f.rows();
        }
        self.sync_ports(g);
    }

    /// The Group Input or Group Output node inside group `g`.
    fn port_node(&self, g: NodeId, input: bool) -> Option<NodeId> {
        (0..self.nodes.len()).find(|&x| {
            let n = &self.nodes[x];
            !n.hidden && n.parent == Some(g) && if input { matches!(n.kind, Kind::GroupInput) } else { matches!(n.kind, Kind::GroupOutput) }
        })
    }

    fn is_port_node(&self, i: NodeId) -> bool {
        matches!(self.nodes[i].kind, Kind::GroupInput | Kind::GroupOutput)
    }

    fn out_name(&self, i: NodeId, port: usize) -> &'static str {
        self.graph.nodes()[i].spec.outputs[port].name
    }

    fn in_name(&self, i: NodeId, port: usize) -> &'static str {
        self.graph.nodes()[i].spec.inputs[port].name
    }

    /// The selected node's evidence trail, with the groups holding any of it (they light up
    /// too) and, inside a group, its Group Input and Output. A selected group's trail is its nodes'.
    pub(crate) fn lit_trail(&self) -> Vec<NodeId> {
        let Some(s) = self.selected else { return Vec::new() };
        let mut t: Vec<NodeId> = Vec::new();
        for x in self.with_inside(&[s]) {
            t.extend(self.graph.trail(x).unwrap_or_default());
        }
        let mut up = Vec::new();
        for &x in &t {
            let mut c = self.nodes[x].parent;
            while let Some(g) = c {
                up.push(g);
                c = self.nodes[g].parent;
            }
        }
        t.extend(up);
        if let Some(g) = self.view {
            t.extend(self.port_node(g, true));
            t.extend(self.port_node(g, false));
        }
        t.push(s);
        t.sort_unstable();
        t.dedup();
        t
    }

    // ---------------- the wires a level shows

    /// Where real input `inp` of `t` is shown at `level`: on `t`, or on the group holding it.
    fn shown_in(&self, t: NodeId, inp: &str, level: Option<NodeId>) -> Option<(NodeId, usize)> {
        let r = self.rep(t, level)?;
        if r == t {
            return self.graph.nodes()[t].spec.inputs.iter().position(|p| p.name == inp).map(|k| (t, k));
        }
        let u = &self.nodes[t].uid;
        self.iface(r).inputs.iter().position(|p| p.ends.iter().any(|(eu, ep)| eu == u && ep == inp)).map(|k| (r, k))
    }

    /// Where real output `out` of `s` is shown at `level`.
    fn shown_out(&self, s: NodeId, out: &str, level: Option<NodeId>) -> Option<(NodeId, usize)> {
        let r = self.rep(s, level)?;
        if r == s {
            return self.graph.nodes()[s].spec.outputs.iter().position(|p| p.name == out).map(|k| (s, k));
        }
        let u = &self.nodes[s].uid;
        self.iface(r).outputs.iter().position(|p| p.ends.iter().any(|(eu, ep)| eu == u && ep == out)).map(|k| (r, k))
    }

    /// The wires shown at `level` (None: the top). A wire inside one group shown there is not
    /// shown; several wires from one port into a group are one line each (draw them once).
    pub(crate) fn shown_wires(&self, level: Option<NodeId>) -> Vec<Shown> {
        let mut out = Vec::new();
        let vif = level.map(|g| self.iface(g)).unwrap_or_default();
        let gi = level.and_then(|g| self.port_node(g, true));
        let go = level.and_then(|g| self.port_node(g, false));
        let index_of = |list: &[GPort], u: &str, q: &str| list.iter().position(|p| p.ends.iter().any(|(eu, ep)| eu == u && ep == q));
        for w in self.graph.wires() {
            let (a, b) = (w.from, w.to);
            let (out_n, in_n) = (self.out_name(a, w.from_port), self.in_name(b, w.to_port));
            let (ra, rb) = (self.rep(a, level), self.rep(b, level));
            if (ra.is_none() && rb.is_none()) || ra == rb {
                continue;
            }
            let from = match ra {
                Some(_) => self.shown_out(a, out_n, level),
                None => gi.zip(index_of(&vif.inputs, &self.nodes[b].uid, in_n)),
            };
            let to = match rb {
                Some(_) => self.shown_in(b, in_n, level),
                None => go.zip(index_of(&vif.outputs, &self.nodes[a].uid, out_n)),
            };
            if let (Some(f), Some(t)) = (from, to) {
                out.push(Shown { from: f.0, from_port: f.1, to: t.0, to_port: t.1, real_from: Some((a, w.from_port)), real_to: (b, in_n) });
            }
        }
        // Inside a group: its ports wired to nothing outside are still linked inside.
        if let Some(g) = level {
            for (k, p) in vif.inputs.iter().enumerate() {
                for (u, q) in &p.ends {
                    let Some(t) = self.by_uid(u) else { continue };
                    if self.graph.feeder(t, q).is_some() {
                        continue;
                    }
                    let Some(name) = self.graph.nodes()[t].spec.inputs.iter().find(|x| x.name == q).map(|x| x.name) else { continue };
                    if let (Some(gi), Some(to)) = (gi, self.shown_in(t, q, level)) {
                        out.push(Shown { from: gi, from_port: k, to: to.0, to_port: to.1, real_from: None, real_to: (t, name) });
                    }
                }
            }
            for (k, p) in vif.outputs.iter().enumerate() {
                let Some((u, q)) = p.ends.first() else { continue };
                let Some(s) = self.by_uid(u) else { continue };
                if self.graph.targets(s, q).iter().any(|(t, _)| !self.is_inside(*t, g)) {
                    continue;
                }
                let Some(port) = self.graph.nodes()[s].spec.outputs.iter().position(|x| x.name == q) else { continue };
                if let (Some(go), Some(from)) = (go, self.shown_out(s, q, level)) {
                    let name = self.graph.nodes()[go].spec.inputs.get(k).map_or(NEW_PORT, |x| x.name);
                    out.push(Shown { from: from.0, from_port: from.1, to: go, to_port: k, real_from: Some((s, port)), real_to: (go, name) });
                }
            }
        }
        out
    }

    /// `shown_wires` with each line once.
    pub(crate) fn shown_lines(&self) -> Vec<Shown> {
        let mut seen = HashSet::new();
        let g = self.graph.nodes();
        self.shown_wires(self.view)
            .into_iter()
            .filter(|s| s.from_port < g[s.from].spec.outputs.len() && s.to_port < g[s.to].spec.inputs.len())
            .filter(|s| seen.insert((s.from, s.from_port, s.to, s.to_port)))
            .collect()
    }

    // ---------------- wiring through groups

    /// Removes `(t, inp)` from the inputs of the groups holding it, from `from` outwards (None:
    /// none): it is fed from inside them now.
    fn forget_end(&mut self, t: NodeId, inp: &str, from: Option<NodeId>) {
        let u = self.nodes[t].uid.clone();
        let mut g = from;
        while let Some(x) = g {
            if self.is_inside(t, x) {
                let mut f = self.iface(x);
                for p in &mut f.inputs {
                    p.ends.retain(|(eu, ep)| !(*eu == u && ep == inp));
                }
                self.set_iface(x, &f);
            }
            g = self.nodes[x].parent;
        }
    }

    /// The real output feeding group `g`'s input `k` from outside it, if any.
    fn outside_feeder(&self, g: NodeId, k: usize) -> Option<(NodeId, &'static str)> {
        let f = self.iface(g);
        f.inputs.get(k)?.ends.iter().find_map(|(u, q)| {
            let t = self.by_uid(u)?;
            self.graph.feeder(t, q).filter(|(s, _)| !self.is_inside(*s, g))
        })
    }

    /// The real inputs a shown input stands for: a node's own, or those inside a group port.
    fn real_targets(&self, to: NodeId, k: usize) -> Vec<(NodeId, &'static str)> {
        match &self.nodes[to].kind {
            Kind::Group { .. } => self.iface(to).inputs.get(k).map_or(Vec::new(), |p| {
                p.ends.iter().filter_map(|(u, q)| {
                    let t = self.by_uid(u)?;
                    Some((t, self.graph.nodes()[t].spec.inputs.iter().find(|x| x.name == q)?.name))
                }).collect()
            }),
            _ => vec![(to, self.in_name(to, k))],
        }
    }

    /// A wire drawn on the canvas from shown output (`from`, `fk`) to shown input (`to`, `tk`),
    /// made in the real graph (and in the groups' ports). The input's old wire goes. Ok: what the
    /// status line says.
    pub(crate) fn connect_shown(&mut self, from: NodeId, fk: usize, to: NodeId, tk: usize) -> Result<String, String> {
        let level = self.view;
        let fkind = self.graph.nodes()[from].spec.outputs[fk].kind;
        let label = |a: &App, i: NodeId| a.nodes[i].label.clone();
        let said = format!("wired {}.{} → {}.{}", label(self, from), self.out_name(from, fk), label(self, to), self.in_name(to, tk));
        match (&self.nodes[from].kind, &self.nodes[to].kind) {
            (Kind::GroupInput, Kind::GroupOutput) => return Err("a group's input cannot go straight to its output".into()),
            (Kind::GroupInput, _) => {
                let g = level.ok_or("no group")?;
                let targets = self.real_targets(to, tk);
                let Some(&(_, i0)) = targets.first() else { return Err("nothing inside uses this port".into()) };
                let tkind = self.graph.nodes()[to].spec.inputs[tk].kind;
                let n = self.iface(g).inputs.len();
                if fk < n && !NodeSpec::can_connect(&Port::opt("", fkind), &Port::opt("", tkind)) {
                    return Err(format!("{} ({fkind:?}) cannot feed {} ({tkind:?})", self.iface(g).inputs[fk].name, self.in_name(to, tk)));
                }
                let feeder = if fk < n { self.outside_feeder(g, fk) } else { None };
                for &(t, inp) in &targets {
                    self.graph.disconnect(t, inp);
                    self.forget_end(t, inp, Some(g));
                }
                // Ports are only dropped next frame, so `fk` still names the same one.
                let mut f = self.iface(g);
                let k = if fk < n {
                    fk
                } else {
                    // The empty port: a new input of the group, named after the input it feeds.
                    let taken: Vec<String> = f.inputs.iter().map(|p| p.name.clone()).collect();
                    f.inputs.push(GPort { name: unique(i0, &taken), kind: tkind, ends: Vec::new() });
                    f.inputs.len() - 1
                };
                for &(t, inp) in &targets {
                    f.inputs[k].ends.push((self.nodes[t].uid.clone(), inp.to_string()));
                }
                self.set_iface(g, &f);
                if let Some((s, o)) = feeder {
                    for &(t, inp) in &targets {
                        self.graph.connect(s, o, t, inp).map_err(|e| describe(&e))?;
                    }
                }
                Ok(said)
            }
            (_, Kind::GroupOutput) => {
                let g = level.ok_or("no group")?;
                let (s, o) = self.real_source(from, fk).ok_or("nothing inside gives this port")?;
                let mut f = self.iface(g);
                let su = self.nodes[s].uid.clone();
                if tk >= f.outputs.len() {
                    let taken: Vec<String> = f.outputs.iter().map(|p| p.name.clone()).collect();
                    f.outputs.push(GPort { name: unique(o, &taken), kind: fkind, ends: vec![(su, o.to_string())] });
                    self.set_iface(g, &f);
                    return Ok(said);
                }
                if f.outputs[tk].kind != fkind {
                    return Err(format!("{} is {:?}, not {:?}", f.outputs[tk].name, f.outputs[tk].kind, fkind));
                }
                // The wires outside from the old output move to the new one.
                let old = f.outputs[tk].ends.first().and_then(|(u, q)| Some((self.by_uid(u)?, q.clone())));
                let outside: Vec<(NodeId, &'static str)> = old.map_or(Vec::new(), |(os, oq)| self.graph.targets(os, &oq).into_iter().filter(|(t, _)| !self.is_inside(*t, g)).collect());
                f.outputs[tk].ends = vec![(su, o.to_string())];
                self.set_iface(g, &f);
                for (t, inp) in outside {
                    self.graph.disconnect(t, inp);
                    self.graph.connect(s, o, t, inp).map_err(|e| describe(&e))?;
                }
                Ok(said)
            }
            _ => {
                let (s, o) = self.real_source(from, fk).ok_or("nothing inside gives this port")?;
                let targets = self.real_targets(to, tk);
                if targets.is_empty() {
                    return Err("nothing inside uses this port".into());
                }
                let tkind = self.graph.nodes()[to].spec.inputs[tk].kind;
                if !NodeSpec::can_connect(&Port::opt("", fkind), &Port::opt("", tkind)) {
                    return Err(format!("kinds differ: {fkind:?} → {tkind:?}"));
                }
                for &(t, inp) in &targets {
                    let old = self.graph.feeder(t, inp);
                    self.graph.disconnect(t, inp);
                    if let Err(e) = self.graph.connect(s, o, t, inp) {
                        if let Some((os, oo)) = old {
                            let _ = self.graph.connect(os, oo, t, inp);
                        }
                        return Err(describe(&e));
                    }
                    // Fed from this level now, not through the group being shown (a group's own
                    // port, below this level, keeps it).
                    self.forget_end(t, inp, level);
                }
                Ok(said)
            }
        }
    }

    /// The real output a shown output stands for.
    fn real_source(&self, from: NodeId, k: usize) -> Option<(NodeId, &'static str)> {
        match &self.nodes[from].kind {
            Kind::Group { .. } => {
                let f = self.iface(from);
                let (u, q) = f.outputs.get(k)?.ends.first()?;
                let s = self.by_uid(u)?;
                Some((s, self.graph.nodes()[s].spec.outputs.iter().find(|x| x.name == q)?.name))
            }
            Kind::GroupInput => None,
            _ => Some((from, self.out_name(from, k))),
        }
    }

    /// Right-click on shown input `k` of `i`: its wire goes (a Group Output's port goes too).
    pub(crate) fn unwire_shown(&mut self, i: NodeId, k: usize) -> bool {
        let level = self.view;
        match &self.nodes[i].kind {
            Kind::GroupOutput => {
                let Some(g) = level else { return false };
                let mut f = self.iface(g);
                if k >= f.outputs.len() {
                    return false;
                }
                let p = f.outputs.remove(k);
                if let Some((s, q)) = p.ends.first().and_then(|(u, q)| Some((self.by_uid(u)?, q.clone()))) {
                    for (t, inp) in self.graph.targets(s, &q) {
                        if !self.is_inside(t, g) {
                            self.graph.disconnect(t, inp);
                        }
                    }
                }
                self.set_iface(g, &f);
                true
            }
            _ => {
                let group = matches!(self.nodes[i].kind, Kind::Group { .. });
                let mut any = false;
                for (t, inp) in self.real_targets(i, k) {
                    let outside = self.graph.feeder(t, inp).is_some_and(|(s, _)| !group || !self.is_inside(s, i));
                    if outside {
                        any |= self.graph.disconnect(t, inp);
                    }
                    // Linked from the Group Input of the group shown: the link goes.
                    if let Some(g) = level {
                        let before = self.iface(g);
                        self.forget_end(t, inp, Some(g));
                        any |= self.iface(g) != before;
                    }
                }
                any
            }
        }
    }

    // ---------------- Ctrl+G, Alt+G, Tab

    /// Ctrl+G: the selected nodes into a new group, in their place. False when nothing is selected.
    pub(crate) fn group_selected(&mut self) -> bool {
        let sel: Vec<NodeId> = self.picked().into_iter().filter(|&i| !self.is_port_node(i)).collect();
        if sel.is_empty() {
            self.msg = "select the nodes to group first".into();
            return false;
        }
        let level = self.view;
        let shown = self.shown_wires(level);
        let defs: Vec<String> = self.nodes.iter().filter(|n| !n.hidden).filter_map(|n| if let Kind::Group { def, .. } = &n.kind { Some(def.clone()) } else { None }).collect();
        let def = unique("Group", &defs);
        let bounds = sel.iter().map(|&i| self.node_rect(Pos2::ZERO, i)).reduce(|a, b| a.union(b)).unwrap();
        let (pan, zoom) = (self.pan, self.zoom);
        let world = |r: Rect| Rect::from_min_size((r.min - pan) / zoom, r.size() / zoom);
        let b = world(bounds);
        let g = self.add(Kind::Group { def: def.clone(), rows: Vec::new() }, Pos2::new(b.center().x - NODE_W / 2.0, b.center().y - 40.0));
        for (k, &i) in sel.iter().enumerate() {
            self.nodes[i].parent = Some(g);
            self.nodes[i].member = format!("m{}", k + 1);
        }
        let gi = self.add(Kind::GroupInput, Pos2::new(b.left() - NODE_W - 80.0, b.center().y - 40.0));
        let go = self.add(Kind::GroupOutput, Pos2::new(b.right() + 80.0, b.center().y - 40.0));
        for (x, m) in [(gi, "in"), (go, "out")] {
            self.nodes[x].parent = Some(g);
            self.nodes[x].member = m.into();
        }
        // Ports: one input per output outside feeding in (it may feed several nodes inside),
        // one output per output inside feeding out.
        let mut f = Iface::default();
        let mut in_keys: Vec<(NodeId, usize)> = Vec::new();
        let mut out_keys: Vec<(NodeId, usize)> = Vec::new();
        for s in &shown {
            let (fi, ti) = (sel.contains(&s.from), sel.contains(&s.to));
            if ti && !fi {
                let (t, inp) = s.real_to;
                let end = (self.nodes[t].uid.clone(), inp.to_string());
                match in_keys.iter().position(|&k| k == (s.from, s.from_port)) {
                    Some(k) => f.inputs[k].ends.push(end),
                    None => {
                        let mut kind = self.graph.nodes()[s.from].spec.outputs[s.from_port].kind;
                        if kind == PortKind::Any {
                            kind = self.graph.nodes()[t].spec.inputs.iter().find(|p| p.name == inp).map_or(kind, |p| p.kind);
                        }
                        let taken: Vec<String> = f.inputs.iter().map(|p| p.name.clone()).collect();
                        f.inputs.push(GPort { name: unique(inp, &taken), kind, ends: vec![end] });
                        in_keys.push((s.from, s.from_port));
                    }
                }
            } else if fi && !ti {
                let Some((a, port)) = s.real_from else { continue };
                if out_keys.contains(&(a, port)) {
                    continue;
                }
                let p = &self.graph.nodes()[a].spec.outputs[port];
                let taken: Vec<String> = f.outputs.iter().map(|p| p.name.clone()).collect();
                f.outputs.push(GPort { name: unique(p.name, &taken), kind: p.kind, ends: vec![(self.nodes[a].uid.clone(), p.name.to_string())] });
                out_keys.push((a, port));
            }
        }
        // The group shown keeps the ends it had; nothing else changes in the graph.
        self.set_iface(g, &f);
        self.pick_only(g);
        self.msg = format!("grouped {} node(s) into {def}: Tab opens it, Alt+G ungroups", sel.len());
        self.save_project();
        true
    }

    /// Alt+G: the selected groups' nodes back out in their place.
    pub(crate) fn ungroup_selected(&mut self) -> bool {
        let groups: Vec<NodeId> = self.picked().into_iter().filter(|&i| matches!(self.nodes[i].kind, Kind::Group { .. })).collect();
        if groups.is_empty() {
            self.msg = "select a group to ungroup".into();
            return false;
        }
        let mut out = Vec::new();
        for g in groups {
            let kids: Vec<NodeId> = (0..self.nodes.len()).filter(|&x| !self.nodes[x].hidden && self.nodes[x].parent == Some(g)).collect();
            let (ports, inner): (Vec<NodeId>, Vec<NodeId>) = kids.into_iter().partition(|&x| self.is_port_node(x));
            let n = inner.len().max(1) as f32;
            let c = inner.iter().fold(Vec2::ZERO, |a, &x| a + self.nodes[x].pos.to_vec2()) / n;
            let shift = self.nodes[g].pos.to_vec2() - c;
            for &x in &inner {
                self.nodes[x].parent = self.nodes[g].parent;
                self.nodes[x].member = self.nodes[g].member.clone();
                self.nodes[x].pos += shift;
            }
            for x in ports.into_iter().chain([g]) {
                self.graph.isolate(x);
                self.nodes[x].hidden = true;
            }
            // Each keeps a member name of its own (made unique next frame).
            for &x in &inner {
                self.nodes[x].member = String::new();
            }
            out.extend(inner);
        }
        self.sel.picked = out.iter().copied().collect();
        self.selected = out.last().copied();
        self.msg = format!("ungrouped {} node(s)", out.len());
        self.save_project();
        true
    }

    /// Tab: into the selected group, or out of the one shown.
    pub(crate) fn tab(&mut self, area: Rect) {
        let g = self.picked().into_iter().find(|&i| matches!(self.nodes[i].kind, Kind::Group { .. }));
        match g {
            Some(g) => self.enter_group(g, area),
            None => {
                self.leave_group();
            }
        }
    }

    pub(crate) fn enter_group(&mut self, g: NodeId, area: Rect) {
        self.view_stack.push((self.pan, self.zoom));
        self.view = Some(g);
        self.pick_none();
        self.frame_nodes(area);
        self.msg = format!("in {}: Tab goes back out", self.nodes[g].label);
    }

    pub(crate) fn leave_group(&mut self) -> bool {
        let Some(g) = self.view else { return false };
        self.view = self.nodes[g].parent;
        if let Some((pan, zoom)) = self.view_stack.pop() {
            (self.pan, self.zoom) = (pan, zoom);
        }
        self.pick_only(g);
        self.msg = String::new();
        true
    }

    /// The path at the top left of the canvas while inside a group: a click on a name goes there.
    pub(crate) fn group_path(&mut self, ui: &egui::Ui, area: Rect) {
        let Some(v) = self.view else { return };
        let mut chain = vec![v];
        while let Some(p) = self.nodes[*chain.last().unwrap()].parent {
            chain.push(p);
        }
        chain.reverse();
        let painter = ui.painter_at(area);
        let mut x = area.left() + 10.0;
        let y = area.top() + 14.0;
        let mut go: Option<usize> = None;
        let names: Vec<String> = std::iter::once("Top".to_string()).chain(chain.iter().map(|&g| self.nodes[g].label.clone())).collect();
        for (k, name) in names.iter().enumerate() {
            let last = k + 1 == names.len();
            let g = painter.layout_no_wrap(name.clone(), FontId::proportional(14.0), if last { TEXT } else { SEL });
            let r = Rect::from_min_size(Pos2::new(x, y - g.size().y / 2.0), g.size());
            painter.galley(r.min, g, TEXT);
            if !last {
                let resp = ui.interact(r, egui::Id::new(("group-path", k)), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.clone()));
                if resp.clicked() {
                    go = Some(k);
                }
            }
            x = r.right() + 6.0;
            if !last {
                painter.text(Pos2::new(x, y), Align2::LEFT_CENTER, "›", FontId::proportional(14.0), DIM);
                x += 14.0;
            }
        }
        painter.text(Pos2::new(x + 12.0, y), Align2::LEFT_CENTER, "Tab: back out", FontId::proportional(11.0), DIM);
        if let Some(k) = go {
            while self.view.is_some() && self.depth() > k {
                self.leave_group();
            }
        }
    }

    fn depth(&self) -> usize {
        let mut d = 0;
        let mut c = self.view;
        while let Some(g) = c {
            d += 1;
            c = self.nodes[g].parent;
        }
        d
    }

    // ---------------- keeping groups right

    /// Every frame: member names, ports that lost their nodes, the ports' nodes, and a change
    /// inside one use of a definition made in the others.
    pub(crate) fn sync_groups(&mut self) {
        let groups: Vec<NodeId> = (0..self.nodes.len()).filter(|&i| !self.nodes[i].hidden && matches!(self.nodes[i].kind, Kind::Group { .. })).collect();
        if groups.is_empty() {
            return;
        }
        // A node whose group is gone goes too (a deleted group's nodes, after an undo).
        for i in 0..self.nodes.len() {
            if !self.nodes[i].hidden && self.nodes[i].parent.is_some_and(|p| self.nodes[p].hidden) {
                self.graph.isolate(i);
                self.nodes[i].hidden = true;
            }
        }
        for &g in &groups {
            // Member names: unique among the group's nodes (new nodes, pasted ones).
            let kids: Vec<NodeId> = (0..self.nodes.len()).filter(|&x| !self.nodes[x].hidden && self.nodes[x].parent == Some(g)).collect();
            let mut taken: Vec<String> = Vec::new();
            for &x in &kids {
                let m = self.nodes[x].member.clone();
                if m.is_empty() || taken.contains(&m) {
                    let n = (1..).map(|n| format!("m{n}")).find(|c| !taken.contains(c) && !kids.iter().any(|&y| self.nodes[y].member == *c)).unwrap();
                    self.nodes[x].member = n;
                }
                taken.push(self.nodes[x].member.clone());
            }
            // Ports whose node inside is gone.
            let f = self.iface(g);
            let alive = |u: &str| self.by_uid(u).is_some_and(|x| self.is_inside(x, g));
            let mut nf = f.clone();
            for p in nf.inputs.iter_mut().chain(nf.outputs.iter_mut()) {
                p.ends.retain(|(u, _)| alive(u));
            }
            nf.inputs.retain(|p| !p.ends.is_empty());
            nf.outputs.retain(|p| !p.ends.is_empty());
            if nf != f {
                self.set_iface(g, &nf);
            }
            self.sync_ports(g);
        }
        self.sync_defs(&groups);
    }

    /// The group's node, Group Input and Group Output get its ports.
    fn sync_ports(&mut self, g: NodeId) {
        let f = self.iface(g);
        let (ins, outs) = (f.ports(&f.inputs), f.ports(&f.outputs));
        if self.graph.nodes()[g].spec.inputs != ins {
            self.graph.set_inputs(g, ins.clone());
        }
        if self.graph.nodes()[g].spec.outputs != outs {
            self.graph.set_outputs(g, outs.clone());
        }
        let plus = Port::opt(NEW_PORT, PortKind::Any);
        if let Some(gi) = self.port_node(g, true) {
            let want: Vec<Port> = ins.into_iter().chain([plus.clone()]).collect();
            if self.graph.nodes()[gi].spec.outputs != want {
                self.graph.set_outputs(gi, want);
            }
        }
        if let Some(go) = self.port_node(g, false) {
            let want: Vec<Port> = outs.into_iter().chain([plus]).collect();
            if self.graph.nodes()[go].spec.inputs != want {
                self.graph.set_inputs(go, want);
            }
        }
    }

    /// The path of member names from group `g` down to `x` ("m1/m3").
    fn path_in(&self, g: NodeId, x: NodeId) -> String {
        let mut parts = vec![self.nodes[x].member.clone()];
        let mut c = x;
        while let Some(p) = self.nodes[c].parent {
            if p == g {
                break;
            }
            parts.push(self.nodes[p].member.clone());
            c = p;
        }
        parts.reverse();
        parts.join("/")
    }

    /// The nodes inside `g` by path, parents before what is inside them.
    fn paths(&self, g: NodeId) -> Vec<(String, NodeId)> {
        let mut v: Vec<(String, NodeId)> = self.inside(g).into_iter().map(|x| (self.path_in(g, x), x)).collect();
        v.sort_by(|a, b| (a.0.matches('/').count(), &a.0).cmp(&(b.0.matches('/').count(), &b.0)));
        v
    }

    /// A group's ports with ids turned into paths inside `g` (comparable between uses).
    fn iface_rel(&self, g: NodeId, rows: &[(String, String)]) -> Iface {
        Iface::of(rows).map_ids(|u| self.by_uid(u).filter(|&x| self.is_inside(x, g)).map(|x| self.path_in(g, x)))
    }

    /// What a definition is, as one use of it holds it: the nodes inside (kind, settings, name,
    /// place, folded), the wires between them, and the ports.
    pub(crate) fn def_text(&self, g: NodeId) -> String {
        let mut s = String::new();
        for (path, x) in self.paths(g) {
            let n = &self.nodes[x];
            let rows = match &n.kind {
                Kind::Group { rows, .. } => format!("{:?}", self.iface_rel(g, rows)),
                k => format!("{:?}", k.rows()),
            };
            s.push_str(&format!("{path}|{}|{}|{rows}|{}|{:.0}|{:.0}|{}\n", n.kind.spec().name, n.kind.saved_param(), n.label, n.pos.x, n.pos.y, n.collapsed));
        }
        let mut w: Vec<String> = self
            .graph
            .wires()
            .iter()
            .filter(|w| self.is_inside(w.from, g) && self.is_inside(w.to, g))
            .map(|w| format!("{}.{}>{}.{}", self.path_in(g, w.from), self.out_name(w.from, w.from_port), self.path_in(g, w.to), self.in_name(w.to, w.to_port)))
            .collect();
        w.sort();
        s.push_str(&w.join(";"));
        if let Kind::Group { rows, .. } = &self.nodes[g].kind {
            s.push_str(&format!("\n{:?}", self.iface_rel(g, rows)));
        }
        s
    }

    /// Uses of one definition kept the same: the one changed (the one shown, else the first that
    /// differs from what the definition was) is copied into the others.
    fn sync_defs(&mut self, groups: &[NodeId]) {
        let mut by_def: Vec<(String, Vec<NodeId>)> = Vec::new();
        for &g in groups {
            let Kind::Group { def, .. } = &self.nodes[g].kind else { continue };
            match by_def.iter_mut().find(|(d, _)| d == def) {
                Some((_, v)) => v.push(g),
                None => by_def.push((def.clone(), vec![g])),
            }
        }
        for (def, uses) in by_def {
            let texts: Vec<String> = uses.iter().map(|&g| self.def_text(g)).collect();
            let known = self.group_defs.get(&def).cloned();
            if texts.iter().all(|t| *t == texts[0]) {
                self.group_defs.insert(def, texts[0].clone());
                continue;
            }
            let shown = |a: &App, g: NodeId| a.view.is_some_and(|v| v == g || a.is_inside(v, g));
            let changed = (0..uses.len())
                .find(|&k| known.as_ref() != Some(&texts[k]) && shown(self, uses[k]))
                .or_else(|| (0..uses.len()).find(|&k| known.as_ref() != Some(&texts[k])))
                .unwrap_or(0);
            let a = uses[changed];
            for &b in &uses {
                // A use inside another (pasted into itself) is left alone: it would grow for ever.
                if b != a && !self.is_inside(b, a) && !self.is_inside(a, b) {
                    self.copy_group(a, b);
                }
            }
            self.group_defs.insert(def, self.def_text(a));
        }
    }

    /// Makes group `b` hold what group `a` holds. `b` keeps its wires outside (by port name) and
    /// its nodes that `a` also has keep their ids and results.
    fn copy_group(&mut self, a: NodeId, b: NodeId) {
        // B's wires outside, by port name.
        let bf = self.iface(b);
        let feeds: Vec<(String, (NodeId, &'static str))> = (0..bf.inputs.len()).filter_map(|k| Some((bf.inputs[k].name.clone(), self.outside_feeder(b, k)?))).collect();
        let mut sends: Vec<(String, Vec<(NodeId, &'static str)>)> = Vec::new();
        for p in &bf.outputs {
            if let Some((s, q)) = p.ends.first().and_then(|(u, q)| Some((self.by_uid(u)?, q.clone()))) {
                sends.push((p.name.clone(), self.graph.targets(s, &q).into_iter().filter(|(t, _)| !self.is_inside(*t, b)).collect()));
            }
        }
        // Every wire with an end inside B goes; they are made again below.
        let into_b: Vec<(NodeId, &'static str)> = self
            .graph
            .wires()
            .iter()
            .filter(|w| self.is_inside(w.to, b) || self.is_inside(w.from, b))
            .map(|w| (w.to, self.in_name(w.to, w.to_port)))
            .collect();
        for (t, inp) in into_b {
            self.graph.disconnect(t, inp);
        }
        // Nodes: same paths, same nodes.
        let pa = self.paths(a);
        let pb: HashMap<String, NodeId> = self.paths(b).into_iter().collect();
        for (path, x) in &pb {
            if !pa.iter().any(|(p, _)| p == path) {
                self.graph.isolate(*x);
                self.nodes[*x].hidden = true;
            }
        }
        let mut map: HashMap<NodeId, NodeId> = HashMap::from([(a, b)]);
        for (path, x) in &pa {
            let y = match pb.get(path).filter(|&&y| !self.nodes[y].hidden) {
                Some(&y) => y,
                None => {
                    let parent = self.nodes[*x].parent.and_then(|p| map.get(&p).copied()).unwrap_or(b);
                    let y = self.add(self.nodes[*x].kind.clone(), self.nodes[*x].pos);
                    self.nodes[y].parent = Some(parent);
                    self.nodes[y].member = self.nodes[*x].member.clone();
                    y
                }
            };
            map.insert(*x, y);
        }
        let uid_map: HashMap<String, String> = map.iter().map(|(x, y)| (self.nodes[*x].uid.clone(), self.nodes[*y].uid.clone())).collect();
        for (_, x) in &pa {
            let y = map[x];
            let kind = match &self.nodes[*x].kind {
                Kind::Group { def, rows } => Kind::Group { def: def.clone(), rows: Iface::of(rows).map_ids(|u| uid_map.get(u).cloned()).rows() },
                k => k.clone(),
            };
            let (label, pos, collapsed) = (self.nodes[*x].label.clone(), self.nodes[*x].pos, self.nodes[*x].collapsed);
            let n = &mut self.nodes[y];
            if n.kind.spec().name != kind.spec().name || n.kind.saved_param() != kind.saved_param() || n.kind.rows() != kind.rows() {
                n.kind = kind;
                let spec = n.kind.spec();
                if !matches!(n.kind, Kind::GroupInput | Kind::GroupOutput) {
                    self.graph.set_inputs(y, spec.inputs);
                    self.graph.set_outputs(y, spec.outputs);
                }
            }
            let n = &mut self.nodes[y];
            (n.label, n.pos, n.collapsed) = (label, pos, collapsed);
        }
        let new_f = self.iface(a).map_ids(|u| uid_map.get(u).cloned());
        self.set_iface(b, &new_f);
        for (_, x) in &pa {
            if matches!(self.nodes[map[x]].kind, Kind::Group { .. }) {
                self.sync_ports(map[x]);
            }
        }
        // Wires inside, as A has them.
        let inner: Vec<(NodeId, &'static str, NodeId, &'static str)> = self
            .graph
            .wires()
            .iter()
            .filter(|w| self.is_inside(w.from, a) && self.is_inside(w.to, a))
            .map(|w| (w.from, self.out_name(w.from, w.from_port), w.to, self.in_name(w.to, w.to_port)))
            .collect();
        for (s, o, t, i) in inner {
            let _ = self.graph.connect(map[&s], o, map[&t], i);
        }
        // B's wires outside, again.
        let bf = self.iface(b);
        for (name, (s, o)) in feeds {
            if let Some(p) = bf.inputs.iter().find(|p| p.name == name) {
                for (u, q) in &p.ends {
                    if let Some(t) = self.by_uid(u) {
                        let _ = self.graph.connect(s, o, t, q);
                    }
                }
            }
        }
        for (name, ts) in sends {
            if let Some((s, q)) = bf.outputs.iter().find(|p| p.name == name).and_then(|p| p.ends.first()).and_then(|(u, q)| Some((self.by_uid(u)?, q.clone()))) {
                for (t, inp) in ts {
                    let _ = self.graph.connect(s, &q, t, inp);
                }
            }
        }
        self.sync_many_ports();
    }

    /// Definitions in the project, each with one use of it (Add node → Groups).
    pub(crate) fn group_defs_list(&self) -> Vec<(String, NodeId)> {
        let mut v: Vec<(String, NodeId)> = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if let (false, Kind::Group { def, .. }) = (n.hidden, &n.kind) {
                if !v.iter().any(|(d, _)| d == def) {
                    v.push((def.clone(), i));
                }
            }
        }
        v
    }

    /// One more use of the definition group `g` is, at `at` (canvas units).
    pub(crate) fn add_group_use(&mut self, g: NodeId, at: Pos2) -> Option<NodeId> {
        // Not inside a use of itself.
        let def = self.nodes[g].kind.saved_param();
        let mut c = self.view;
        while let Some(x) = c {
            if matches!(&self.nodes[x].kind, Kind::Group { def: d, .. } if *d == def) {
                self.msg = format!("{def} cannot go inside itself");
                return None;
            }
            c = self.nodes[x].parent;
        }
        let text = self.nodes_text(&self.with_inside(&[g]));
        self.paste(&text, at);
        self.picked().into_iter().find(|&i| matches!(self.nodes[i].kind, Kind::Group { .. }))
    }
}

// ---------------------------------------------------------------- look

/// A group: what is inside it, and the keys.
pub struct GroupLook;

impl look::NodeLook for GroupLook {
    /// Wide enough for its port names and three lines between them.
    fn size(&self, _app: &App, _i: NodeId, standard: Vec2) -> Vec2 {
        Vec2::new(standard.x.max(340.0), standard.y.max(HEAD + 3.0 * 17.0 + 20.0))
    }

    fn fill(&self, _app: &App, _i: NodeId) -> Color32 {
        Color32::from_rgb(38, 56, 52)
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = look::content(app, ui, i);
        let def = app.nodes[i].kind.saved_param();
        let uses = app.nodes.iter().filter(|n| !n.hidden && matches!(&n.kind, Kind::Group { def: d, .. } if *d == def)).count();
        let n = app.inside(i).into_iter().filter(|&x| !app.is_port_node(x)).count();
        look::area(ui, r, |ui| {
            look::line(ui, format!("{n} node(s) inside"), TEXT);
            look::line(ui, if uses > 1 { format!("{def} · {uses} uses") } else { def }, DIM);
            look::line(ui, "Tab: open · Alt+G: ungroup", DIM);
        });
    }

    fn status(&self) -> bool {
        false
    }
}

/// The Group Input and Group Output nodes inside a group.
pub struct PortsLook;

impl look::NodeLook for PortsLook {
    fn fill(&self, _app: &App, _i: NodeId) -> Color32 {
        Color32::from_rgb(34, 44, 50)
    }

    fn status(&self) -> bool {
        false
    }
}
