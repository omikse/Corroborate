//! Which clock each node's times are on. There is no project-wide pool of evidence and no global
//! clock solve any more (Time on wires, stage 4, 2026-11-11): a time reaches a node only on a wire,
//! and each node that measures clocks (Audio match) fits its own.
//!
//! - **Recordings**: each source node is its own recording (its own clock inside, for playheads
//!   and pieces), named by its stable id, unless One recording (right-click on several selected
//!   sources) gave them one shared id: a label only, saying the files are one recording's parts.
//!   A node fed audio belongs to the recording of that audio's source.
//! - Projects saved before stage 4 may hold evidence lines (`anchor|`, `link|`, `off|`,
//!   `stamp-end|`): they are read, dropped, and counted in a message.
//!
//! Project lines: `format|2`, `recording|<node id>|<recording id>`.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Which sources share a clock, and what an old project held that is no longer read.
#[derive(Default)]
pub struct Evidence {
    /// Source node id → shared recording id (One recording).
    pub share: BTreeMap<String, String>,
    /// While loading: old evidence lines dropped (anchors, links, switches).
    pub dropped: (usize, usize, usize),
}

impl App {
    /// The clock node `i`'s times are on: a source's own (or shared) clock, else the clock of
    /// the audio fed into it. None when it has neither.
    pub(crate) fn clock_of(&self, i: NodeId) -> Option<String> {
        self.clock_at(i, 0)
    }

    fn clock_at(&self, i: NodeId, depth: usize) -> Option<String> {
        let n = self.nodes.get(i)?;
        if n.kind.is_source() {
            return Some(self.ev.share.get(&n.uid).cloned().unwrap_or_else(|| n.uid.clone()));
        }
        if depth > 8 {
            return None;
        }
        // A cut detector is on its recording's clock (wired as video or audio).
        if matches!(n.kind, Kind::CutDetector { .. }) {
            return self.clock_at(self.cut_source(i).ok()?, depth + 1);
        }
        let audio = self.graph.nodes()[i].spec.inputs.iter().find(|p| p.kind == PortKind::Audio)?;
        let (f, o) = self.graph.feeder(i, audio.name)?;
        // A piece of a cut recording is a clock of its own.
        if let Some(c) = self.piece_clock(f, o) {
            return Some(c);
        }
        self.clock_at(f, depth + 1)
    }

    /// One recording: sources `nodes` share one recording (named by the smallest of their clocks), with
    /// every source already sharing a clock with one of them.
    pub(crate) fn same_clock(&mut self, nodes: &[NodeId]) {
        let srcs: Vec<NodeId> = nodes.iter().copied().filter(|&i| self.nodes[i].kind.is_source()).collect();
        let clocks: BTreeSet<String> = srcs.iter().filter_map(|&i| self.clock_of(i)).collect();
        let Some(id) = clocks.iter().next().cloned() else { return };
        for (_, c) in self.ev.share.iter_mut() {
            if clocks.contains(c) {
                *c = id.clone();
            }
        }
        for i in srcs {
            self.ev.share.insert(self.nodes[i].uid.clone(), id.clone());
        }
        self.save_project();
    }

    /// Separate recordings: sources `nodes` each go back to their own.
    pub(crate) fn separate_clocks(&mut self, nodes: &[NodeId]) {
        for &i in nodes {
            self.ev.share.remove(&self.nodes[i].uid);
        }
        // A shared clock left with one source is that source's own again.
        let mut count: BTreeMap<String, usize> = BTreeMap::new();
        for c in self.ev.share.values() {
            *count.entry(c.clone()).or_default() += 1;
        }
        self.ev.share.retain(|_, c| count[c] > 1);
        self.save_project();
    }

    /// Whether node `i` is a source sharing its clock with another.
    pub(crate) fn shares_clock(&self, i: NodeId) -> bool {
        self.ev.share.contains_key(&self.nodes[i].uid)
    }

    /// The project lines for clocks (see the top of this file).
    pub(crate) fn evidence_text(&self) -> String {
        // `format|2`: Audio match's rows have a time each.
        let mut t = String::from("format|2\n");
        for (n, c) in &self.ev.share {
            t.push_str(&format!("recording|{}|{}\n", clean(n), clean(c)));
        }
        t
    }

    /// Reads one project line of clocks; an old evidence line is counted and dropped. False when
    /// it is not one.
    pub(crate) fn evidence_line(&mut self, f: &[&str]) -> bool {
        match f {
            ["recording", n, c] => {
                self.ev.share.insert(n.to_string(), c.to_string());
            }
            ["format", _] | ["stamp-end", _] => {}
            ["anchor", ..] => self.ev.dropped.0 += 1,
            ["link", ..] => self.ev.dropped.1 += 1,
            ["off", _] => self.ev.dropped.2 += 1,
            _ => return false,
        }
        true
    }

    /// The message for old evidence lines dropped while loading, if any; resets the count.
    pub(crate) fn dropped_evidence(&mut self) -> Option<String> {
        let (a, l, o) = std::mem::take(&mut self.ev.dropped);
        if a + l + o == 0 {
            return None;
        }
        let n = |k: usize, one: &str, many: &str| (k > 0).then(|| format!("{k} {}", if k == 1 { one } else { many }));
        let what: Vec<String> = [n(a, "anchor", "anchors"), n(l, "link", "links"), n(o, "evidence switch", "evidence switches")].into_iter().flatten().collect();
        Some(format!("dropped from the old project: {} (there is no global clock solve any more; a time made by hand is a Value node of type time, wired where it is needed)", what.join(", ")))
    }

    /// A node's name, or its module's when it has none.
    pub(crate) fn node_name(&self, i: NodeId) -> String {
        if self.nodes[i].label.is_empty() { self.graph.nodes()[i].spec.name.to_string() } else { self.nodes[i].label.clone() }
    }
}
