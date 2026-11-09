//! The node graph: nodes with typed ports, wires between them, and the checks that run
//! before anything executes.
//!
//! Rules enforced here:
//! - A wire joins an output and an input of the same `PortKind`.
//! - An input takes at most one wire (one writer). An output may feed many inputs.
//! - No cycles: a node cannot depend on its own result.
//!
//! Also here: which required inputs are still missing, a run order (upstream first), and the
//! evidence trail of a node (everything it depends on, transitively).

use crate::core::{NodeSpec, Port, PortKind};
use crate::module_manifest::Many;

pub type NodeId = usize;

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: NodeId,
    /// Name shown in the graph, for example "Aircraft (Doppler) #2".
    pub label: String,
    pub spec: NodeSpec,
}

/// A connection from output `from_port` of node `from` to input `to_port` of node `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wire {
    pub from: NodeId,
    pub from_port: usize,
    pub to: NodeId,
    pub to_port: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    UnknownNode(NodeId),
    NoOutput { node: NodeId, port: String },
    NoInput { node: NodeId, port: String },
    KindMismatch { from: String, to: String },
    /// The type is defined differently in two places (src/types.rs).
    TypeConflict(String),
    InputTaken { node: NodeId, port: String },
    Cycle,
}

#[derive(Default, Debug)]
pub struct Graph {
    nodes: Vec<Node>,
    wires: Vec<Wire>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn wires(&self) -> &[Wire] {
        &self.wires
    }

    pub fn add(&mut self, label: impl Into<String>, spec: NodeSpec) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node { id, label: label.into(), spec });
        id
    }

    fn node(&self, id: NodeId) -> Result<&Node, GraphError> {
        self.nodes.get(id).ok_or(GraphError::UnknownNode(id))
    }

    /// Connect `from`'s output named `out` to `to`'s input named `inp`.
    pub fn connect(&mut self, from: NodeId, out: &str, to: NodeId, inp: &str) -> Result<(), GraphError> {
        let src = self.node(from)?;
        let dst = self.node(to)?;
        let (from_port, op) = src
            .spec
            .outputs
            .iter()
            .enumerate()
            .find(|(_, p)| p.name == out)
            .ok_or_else(|| GraphError::NoOutput { node: from, port: out.to_string() })?;
        let (to_port, ip) = dst
            .spec
            .inputs
            .iter()
            .enumerate()
            .find(|(_, p)| p.name == inp)
            .ok_or_else(|| GraphError::NoInput { node: to, port: inp.to_string() })?;
        if !NodeSpec::can_connect(op, ip) {
            return Err(GraphError::KindMismatch {
                from: format!("{}.{} ({:?})", src.label, op.name, op.kind),
                to: format!("{}.{} ({:?})", dst.label, ip.name, ip.kind),
            });
        }
        if let Some(e) = crate::types::conflict(crate::module_manifest::kind_id(op.kind)) {
            return Err(GraphError::TypeConflict(e));
        }
        if self.wires.iter().any(|w| w.to == to && w.to_port == to_port) {
            return Err(GraphError::InputTaken { node: to, port: inp.to_string() });
        }
        // Adding from -> to closes a cycle if `to` already reaches `from`.
        if from == to || self.reaches(to, from) {
            return Err(GraphError::Cycle);
        }
        self.wires.push(Wire { from, from_port, to, to_port });
        Ok(())
    }

    /// Remove the wire into `to`'s input named `inp`, if there is one.
    pub fn disconnect(&mut self, to: NodeId, inp: &str) -> bool {
        let Some(port) = self.nodes.get(to).and_then(|n| n.spec.inputs.iter().position(|p| p.name == inp)) else {
            return false;
        };
        let before = self.wires.len();
        self.wires.retain(|w| !(w.to == to && w.to_port == port));
        self.wires.len() != before
    }

    /// Give node `id` new outputs (a source whose outputs follow what it opens). Wires keep their
    /// output by name; wires from an output that is gone are removed.
    pub fn set_outputs(&mut self, id: NodeId, outputs: Vec<crate::core::Port>) {
        let Some(node) = self.nodes.get_mut(id) else { return };
        if node.spec.outputs == outputs {
            return;
        }
        let old = std::mem::replace(&mut node.spec.outputs, outputs);
        let new = &node.spec.outputs;
        self.wires.retain_mut(|w| {
            if w.from != id {
                return true;
            }
            match new.iter().position(|p| p.name == old[w.from_port].name && p.kind == old[w.from_port].kind) {
                Some(k) => {
                    w.from_port = k;
                    true
                }
                None => false,
            }
        });
    }

    /// Replace node `id`'s inputs (a node whose module is missing gets them from its saved wires).
    /// Must be called before wiring them.
    pub fn set_inputs(&mut self, id: NodeId, inputs: Vec<crate::core::Port>) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.spec.inputs = inputs;
        }
        self.wires.retain(|w| w.to != id);
    }

    /// As `set_inputs`, but wires keep their input by name; wires into an input that is gone are
    /// removed.
    pub fn set_inputs_keep(&mut self, id: NodeId, inputs: Vec<crate::core::Port>) {
        let Some(node) = self.nodes.get_mut(id) else { return };
        if node.spec.inputs == inputs {
            return;
        }
        let old = std::mem::replace(&mut node.spec.inputs, inputs);
        let new = &node.spec.inputs;
        self.wires.retain_mut(|w| {
            if w.to != id {
                return true;
            }
            match new.iter().position(|p| p.name == old[w.to_port].name) {
                Some(k) => {
                    w.to_port = k;
                    true
                }
                None => false,
            }
        });
    }

    /// Pack node `id`'s `many` input (its last inputs, named `names` in order): its wired ports
    /// first, in order, then one free port, at most `names.len()`. Other inputs stay as they are.
    pub fn pack_many(&mut self, id: NodeId, names: &[&'static str]) {
        self.pack_rows(id, &Many { inputs: names.to_vec(), per_row: 1, outputs: Vec::new() });
    }

    /// Where node `id`'s growing ports start: its first `many` input and its first per-row output.
    fn many_start(&self, id: NodeId, m: &Many) -> Option<(usize, usize)> {
        let node = self.nodes.get(id)?;
        let fixed = node.spec.inputs.iter().position(|p| m.inputs.contains(&p.name)).unwrap_or(node.spec.inputs.len());
        let fixed_out = node.spec.outputs.iter().position(|p| m.outputs.contains(&p.name)).unwrap_or(node.spec.outputs.len());
        Some((fixed, fixed_out))
    }

    /// Node `id`'s first `rows` rows of growing ports, kinds taken from its first row.
    fn rows_ports(&self, id: NodeId, m: &Many, rows: usize) -> Option<(Vec<Port>, Vec<Port>)> {
        let (fixed, fixed_out) = self.many_start(id, m)?;
        let spec = &self.nodes.get(id)?.spec;
        let w = m.per_row.max(1);
        let mut inputs = spec.inputs[..fixed].to_vec();
        inputs.extend(m.inputs[..rows * w].iter().enumerate().map(|(k, s)| Port::opt(s, spec.inputs.get(fixed + k % w).map_or(PortKind::Any, |p| p.kind))));
        let mut outputs = spec.outputs[..fixed_out].to_vec();
        let kind = spec.outputs.get(fixed_out).map_or(PortKind::Any, |p| p.kind);
        outputs.extend(m.outputs[..rows.min(m.outputs.len())].iter().map(|s| Port::req(s, kind)));
        Some((inputs, outputs))
    }

    /// Pack node `id`'s rows of growing ports: the rows with a wire, input or output, first, in
    /// order, their ports moving together, then one free row, at most as many as `m` names.
    pub fn pack_rows(&mut self, id: NodeId, m: &Many) {
        let Some((fixed, fixed_out)) = self.many_start(id, m) else { return };
        let w = m.per_row.max(1);
        let max_rows = m.inputs.len() / w;
        let has_out = !m.outputs.is_empty();
        let mut used: Vec<usize> = self
            .wires
            .iter()
            .filter_map(|x| {
                if x.to == id && x.to_port >= fixed {
                    Some((x.to_port - fixed) / w)
                } else if has_out && x.from == id && x.from_port >= fixed_out {
                    Some(x.from_port - fixed_out)
                } else {
                    None
                }
            })
            .collect();
        used.sort_unstable();
        used.dedup();
        used.truncate(max_rows);
        self.wires.retain_mut(|x| {
            if x.to == id && x.to_port >= fixed {
                let Ok(slot) = used.binary_search(&((x.to_port - fixed) / w)) else { return false };
                x.to_port = fixed + slot * w + (x.to_port - fixed) % w;
            }
            if has_out && x.from == id && x.from_port >= fixed_out {
                let Ok(slot) = used.binary_search(&(x.from_port - fixed_out)) else { return false };
                x.from_port = fixed_out + slot;
            }
            true
        });
        let n = (used.len() + 1).min(max_rows);
        let Some((inputs, outputs)) = self.rows_ports(id, m, n) else { return };
        let node = &mut self.nodes[id];
        node.spec.inputs = inputs;
        node.spec.outputs = outputs;
    }

    /// Make sure node `id` has the row its growing port `name` (input, or output when `out`) is
    /// in, so a wire saved by name can go back. `pack_rows` closes any gap afterwards.
    pub fn grow_to(&mut self, id: NodeId, m: &Many, name: &str, out: bool) {
        let w = m.per_row.max(1);
        let row = if out { m.outputs.iter().position(|s| *s == name) } else { m.inputs.iter().position(|s| *s == name).map(|k| k / w) };
        let Some(row) = row else { return };
        let Some((fixed, _)) = self.many_start(id, m) else { return };
        let have = (self.nodes[id].spec.inputs.len() - fixed) / w;
        if row < have {
            return;
        }
        let Some((inputs, outputs)) = self.rows_ports(id, m, row + 1) else { return };
        let node = &mut self.nodes[id];
        node.spec.inputs = inputs;
        node.spec.outputs = outputs;
    }

    /// Names of the inputs of `id` that have a wire.
    pub fn connected_inputs(&self, id: NodeId) -> Vec<&'static str> {
        let Some(node) = self.nodes.get(id) else { return Vec::new() };
        self.wires
            .iter()
            .filter(|w| w.to == id)
            .map(|w| node.spec.inputs[w.to_port].name)
            .collect()
    }

    /// Required inputs of `id` that have no wire yet. Empty means the node can run.
    pub fn missing(&self, id: NodeId) -> Vec<&'static str> {
        let Some(node) = self.nodes.get(id) else { return Vec::new() };
        let connected = self.connected_inputs(id);
        node.spec
            .inputs
            .iter()
            .filter(|p| p.required && !connected.contains(&p.name))
            .map(|p| p.name)
            .collect()
    }

    /// True when `to` is reachable from `from` by following wires downstream.
    fn reaches(&self, from: NodeId, to: NodeId) -> bool {
        let mut stack = vec![from];
        let mut seen = vec![false; self.nodes.len()];
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if std::mem::replace(&mut seen[n], true) {
                continue;
            }
            stack.extend(self.wires.iter().filter(|w| w.from == n).map(|w| w.to));
        }
        false
    }

    /// Run order: every node after the nodes it depends on (Kahn's algorithm, ties by id).
    pub fn order(&self) -> Result<Vec<NodeId>, GraphError> {
        let n = self.nodes.len();
        let mut indeg = vec![0usize; n];
        for w in &self.wires {
            indeg[w.to] += 1;
        }
        let mut ready: Vec<NodeId> = (0..n).filter(|&i| indeg[i] == 0).collect();
        let mut out = Vec::with_capacity(n);
        while let Some(i) = ready.first().copied() {
            ready.remove(0);
            out.push(i);
            for w in self.wires.iter().filter(|w| w.from == i) {
                indeg[w.to] -= 1;
                if indeg[w.to] == 0 {
                    ready.push(w.to);
                    ready.sort_unstable();
                }
            }
        }
        if out.len() == n { Ok(out) } else { Err(GraphError::Cycle) }
    }

    /// Remove every wire into or out of `id`: used when a node is deleted from the workspace.
    pub fn isolate(&mut self, id: NodeId) {
        self.wires.retain(|w| w.from != id && w.to != id);
    }

    /// Wires leaving output `out` of node `id`, as (target node, target input name).
    pub fn targets(&self, id: NodeId, out: &str) -> Vec<(NodeId, &'static str)> {
        let Some(port) = self.nodes.get(id).and_then(|n| n.spec.outputs.iter().position(|p| p.name == out)) else {
            return Vec::new();
        };
        self.wires
            .iter()
            .filter(|w| w.from == id && w.from_port == port)
            .map(|w| (w.to, self.nodes[w.to].spec.inputs[w.to_port].name))
            .collect()
    }

    /// The node and output feeding input `inp` of `id`, if wired.
    pub fn feeder(&self, id: NodeId, inp: &str) -> Option<(NodeId, &'static str)> {
        let port = self.nodes.get(id)?.spec.inputs.iter().position(|p| p.name == inp)?;
        let w = self.wires.iter().find(|w| w.to == id && w.to_port == port)?;
        Some((w.from, self.nodes[w.from].spec.outputs[w.from_port].name))
    }

    /// Evidence trail of `id`: every node it depends on, directly or not, in run order,
    /// ending with `id` itself.
    pub fn trail(&self, id: NodeId) -> Result<Vec<NodeId>, GraphError> {
        self.node(id)?;
        let mut keep = vec![false; self.nodes.len()];
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if std::mem::replace(&mut keep[n], true) {
                continue;
            }
            stack.extend(self.wires.iter().filter(|w| w.to == n).map(|w| w.from));
        }
        Ok(self.order()?.into_iter().filter(|&i| keep[i]).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Role, SourceKind};

    fn source() -> NodeSpec {
        NodeSpec {
            name: "file",
            role: Role::Source(SourceKind::File),
            inputs: vec![],
            outputs: vec![Port::req("audio", PortKind::Audio), Port::req("time", PortKind::Time)],
        }
    }

    fn detector() -> NodeSpec {
        NodeSpec {
            name: "planes",
            role: Role::Detector,
            inputs: vec![Port::req("audio", PortKind::Audio), Port::opt("time", PortKind::Time)],
            outputs: vec![Port::req("pass", PortKind::Event)],
        }
    }

    fn solver() -> NodeSpec {
        NodeSpec {
            name: "sync",
            role: Role::Solver,
            inputs: vec![Port::req("a", PortKind::Event), Port::opt("b", PortKind::Event)],
            outputs: vec![Port::req("time", PortKind::Time)],
        }
    }

    #[test]
    fn connects_matching_kinds_and_rejects_mismatch() {
        let mut g = Graph::new();
        let f = g.add("file", source());
        let p = g.add("planes", detector());
        assert!(g.connect(f, "audio", p, "audio").is_ok());
        // Time into the audio input is a kind mismatch.
        let e = g.connect(f, "time", p, "audio").unwrap_err();
        assert!(matches!(e, GraphError::KindMismatch { .. }));
    }

    #[test]
    fn input_takes_one_wire_but_output_can_fan_out() {
        let mut g = Graph::new();
        let f = g.add("file", source());
        let p1 = g.add("planes 1", detector());
        let p2 = g.add("planes 2", detector());
        assert!(g.connect(f, "audio", p1, "audio").is_ok());
        assert!(g.connect(f, "audio", p2, "audio").is_ok());
        let e = g.connect(f, "audio", p1, "audio").unwrap_err();
        assert!(matches!(e, GraphError::InputTaken { .. }));
    }

    #[test]
    fn rejects_cycles_and_self_loops() {
        let mut g = Graph::new();
        let spec = || NodeSpec {
            name: "relay",
            role: Role::Solver,
            inputs: vec![Port::opt("in", PortKind::Event)],
            outputs: vec![Port::req("out", PortKind::Event)],
        };
        let x = g.add("x", spec());
        let y = g.add("y", spec());
        assert!(g.connect(x, "out", y, "in").is_ok());
        assert_eq!(g.connect(y, "out", x, "in"), Err(GraphError::Cycle));
        assert_eq!(g.connect(x, "out", x, "in"), Err(GraphError::Cycle));
    }

    #[test]
    fn missing_lists_unconnected_required_inputs() {
        let mut g = Graph::new();
        let f = g.add("file", source());
        let p = g.add("planes", detector());
        assert_eq!(g.missing(p), vec!["audio"]);
        g.connect(f, "audio", p, "audio").unwrap();
        assert!(g.missing(p).is_empty());
    }

    #[test]
    fn order_and_trail_follow_the_wires() {
        let mut g = Graph::new();
        let f = g.add("file", source());
        let p = g.add("planes", detector());
        let other = g.add("unrelated", source());
        let s = g.add("sync", solver());
        g.connect(f, "audio", p, "audio").unwrap();
        g.connect(p, "pass", s, "a").unwrap();
        let order = g.order().unwrap();
        let pos = |id| order.iter().position(|&x| x == id).unwrap();
        assert!(pos(f) < pos(p) && pos(p) < pos(s));
        // The trail of the solver: file, planes, and the solver itself; not the unrelated node.
        assert_eq!(g.trail(s).unwrap(), vec![f, p, s]);
        assert!(!g.trail(s).unwrap().contains(&other));
    }

    #[test]
    fn isolate_targets_and_feeder() {
        let mut g = Graph::new();
        let f = g.add("file", source());
        let p = g.add("planes", detector());
        let q = g.add("planes 2", detector());
        g.connect(f, "audio", p, "audio").unwrap();
        g.connect(f, "audio", q, "audio").unwrap();
        g.connect(f, "time", p, "time").unwrap();
        assert_eq!(g.targets(f, "audio"), vec![(p, "audio"), (q, "audio")]);
        assert_eq!(g.feeder(p, "time"), Some((f, "time")));
        g.isolate(f);
        assert!(g.wires().is_empty());
        assert_eq!(g.feeder(p, "time"), None);
    }

    /// Rows of growing ports stay together when one is emptied: the audio and time of a row, and
    /// the wire out of its output, all move up.
    #[test]
    fn rows_of_ports_move_together() {
        let m = Many { inputs: vec!["audio 1", "time 1", "audio 2", "time 2", "audio 3", "time 3"], per_row: 2, outputs: vec!["out 1", "out 2", "out 3"] };
        let mut g = Graph::new();
        let f = g.add("file", source());
        let n = g.add("match", NodeSpec { name: "m", role: Role::Detector, inputs: vec![Port::opt("audio 1", PortKind::Audio), Port::opt("time 1", PortKind::Time)], outputs: vec![Port::req("links", PortKind::Event), Port::req("out 1", PortKind::Time)] });
        let p = g.add("planes", detector());
        g.pack_rows(n, &m);
        g.connect(f, "audio", n, "audio 1").unwrap();
        g.pack_rows(n, &m);
        assert_eq!(g.nodes()[n].spec.inputs.iter().map(|p| p.name).collect::<Vec<_>>(), ["audio 1", "time 1", "audio 2", "time 2"]);
        assert_eq!(g.nodes()[n].spec.outputs.iter().map(|p| p.name).collect::<Vec<_>>(), ["links", "out 1", "out 2"]);
        let f2 = g.add("file 2", source());
        g.connect(f2, "audio", n, "audio 2").unwrap();
        g.connect(f2, "time", n, "time 2").unwrap();
        g.pack_rows(n, &m);
        g.connect(n, "out 2", p, "time").unwrap();
        g.pack_rows(n, &m);
        // Row 1 emptied: row 2 and the wire out of it become row 1.
        assert!(g.disconnect(n, "audio 1"));
        g.pack_rows(n, &m);
        assert_eq!(g.feeder(n, "audio 1"), Some((f2, "audio")));
        assert_eq!(g.feeder(n, "time 1"), Some((f2, "time")));
        assert_eq!(g.feeder(p, "time"), Some((n, "out 1")));
        assert_eq!(g.nodes()[n].spec.inputs.len(), 4, "one row wired and a free one");
        // A row whose only wire goes out stays.
        assert!(g.disconnect(n, "audio 1") && g.disconnect(n, "time 1"));
        g.pack_rows(n, &m);
        assert_eq!(g.feeder(p, "time"), Some((n, "out 1")));
        // A row saved further on can be wired after growing to it.
        g.grow_to(n, &m, "time 3", false);
        g.connect(f, "time", n, "time 3").unwrap();
        g.pack_rows(n, &m);
        assert_eq!(g.feeder(n, "time 2"), Some((f, "time")), "the gap closes");
    }

    #[test]
    fn disconnect_removes_the_wire() {
        let mut g = Graph::new();
        let f = g.add("file", source());
        let p = g.add("planes", detector());
        g.connect(f, "audio", p, "audio").unwrap();
        assert!(g.disconnect(p, "audio"));
        assert_eq!(g.missing(p), vec!["audio"]);
        assert!(!g.disconnect(p, "audio"));
    }
}
