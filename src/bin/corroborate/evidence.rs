//! The project's clocks: which clock each node's times are on, the evidence the nodes give about
//! them, and the solve (`corroborate::clock`).
//!
//! - **Clocks**: each source node is its own clock, named by its stable id, unless Same clock
//!   (right-click on several selected sources) gave them one shared id. A node fed audio is on
//!   the clock of that audio's source.
//! - **Evidence**, gathered every frame and solved again when it changes:
//!   - each source's own anchor (file metadata, stream times, this PC's clock);
//!   - a time wired into an input that module.toml marks `anchors = "audio"`: the audio starts at
//!     that time (an anchor when the time is UTC, a link when it is on another clock);
//!   - every `anchor` and `link` output in the project, wired or not;
//!   - manual anchors and links (saved in the project).
//! - Each piece of evidence can be switched off (its key in `evidence_off`, saved in the project).
//!
//! Project lines: `clock|<node id>|<clock id>`, `off|<evidence key>`,
//! `anchor|<clock>|<t>|<utc>|<error_s>`, `link|<a>|<t_a>|<b>|<t_b>|<error_s>`.

use super::*;
use corroborate::clock::{self, Anchor as ClockAnchor, Link};
use corroborate::data::Data;
use std::collections::{BTreeMap, BTreeSet};

/// The evidence and what the solve made of it.
#[derive(Default)]
pub struct Evidence {
    /// Source node id → shared clock id (Same clock).
    pub share: BTreeMap<String, String>,
    /// Keys of the evidence switched off.
    pub off: BTreeSet<String>,
    /// Anchors and links made by hand (the timeline).
    pub manual_anchors: Vec<ClockAnchor>,
    pub manual_links: Vec<Link>,
    /// All evidence last gathered, with its on/off state, and the clocks solved from it.
    pub anchors: Vec<ClockAnchor>,
    pub links: Vec<Link>,
    pub clocks: clock::Clocks,
    /// How sure the shown errors are, and the largest error allowed (seconds, at that confidence).
    pub conf: clock_view::Confidence,
    pub max_error: Option<f64>,
    /// The clock group last clicked on a wire box (the timeline shows its row).
    pub focus: Option<String>,
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
        let audio = self.graph.nodes()[i].spec.inputs.iter().find(|p| p.kind == PortKind::Audio)?;
        let (f, _) = self.graph.feeder(i, audio.name)?;
        self.clock_at(f, depth + 1)
    }

    /// The clock of what is wired into input `port` of node `i`.
    fn clock_into(&self, i: NodeId, port: &str) -> Option<String> {
        let (f, _) = self.graph.feeder(i, port)?;
        self.clock_of(f)
    }

    /// Time inputs of node `i` that anchor another input's clock (module.toml `anchors`).
    fn anchoring_inputs(&self, i: NodeId) -> Vec<(String, String)> {
        match self.manifest_of(i) {
            Some(m) => m.anchors.clone(),
            None if matches!(self.nodes[i].kind, Kind::Planes) => corroborate::modules::planes::manifest().anchors.clone(),
            None => Vec::new(),
        }
    }

    /// Every piece of evidence in the project now, each with its on/off state.
    pub(crate) fn gather_evidence(&self) -> (Vec<ClockAnchor>, Vec<Link>) {
        let (mut an, mut ln) = (Vec::new(), Vec::new());
        for i in 0..self.nodes.len() {
            let n = &self.nodes[i];
            if n.hidden {
                continue;
            }
            // A source's own anchor: its time 0 is at utc0.
            if n.kind.is_source() {
                let a = n.state.lock().unwrap().anchor.clone();
                if let (Some(a), Some(c)) = (a, self.clock_of(i)) {
                    an.push(ClockAnchor { clock: c, t: 0.0, utc: a.utc0, error_s: a.error_s, from: n.uid.clone(), enabled: true });
                }
            }
            // A time wired into an anchoring input: the audio starts at that time.
            for (tin, ain) in self.anchoring_inputs(i) {
                let (Some((f, o)), Some(ac)) = (self.graph.feeder(i, &tin), self.clock_into(i, &ain)) else { continue };
                let Ok(Data::Time { clock: tc, t, utc, error_s, .. }) = self.value_from(f, o) else { continue };
                if tc == ac {
                    continue; // the audio's own time: nothing new
                }
                match utc {
                    Some(u) => an.push(ClockAnchor { clock: ac, t: 0.0, utc: u, error_s, from: n.uid.clone(), enabled: true }),
                    None => ln.push(Link { a: ac, t_a: 0.0, b: tc, t_b: t, error_s, from: n.uid.clone(), enabled: true }),
                }
            }
            // Anchor and link outputs, wired or not.
            for p in &self.graph.nodes()[i].spec.outputs {
                let PortKind::Custom(k @ ("anchor" | "link")) = p.kind else { continue };
                let Ok(Data::Custom { json, .. }) = self.value_from(i, p.name) else { continue };
                let own = self.clock_of(i).unwrap_or_else(|| n.uid.clone());
                if k == "anchor" {
                    an.extend(clock::anchors_from_json(&json, &own, &n.uid));
                } else {
                    ln.extend(clock::links_from_json(&json, &own, &n.uid));
                }
            }
        }
        an.extend(self.ev.manual_anchors.iter().cloned());
        ln.extend(self.ev.manual_links.iter().cloned());
        for a in &mut an {
            a.enabled = !self.ev.off.contains(&a.key());
        }
        for l in &mut ln {
            l.enabled = !self.ev.off.contains(&l.key());
        }
        (an, ln)
    }

    /// Gathers the evidence and solves the clocks again when it changed.
    pub(crate) fn update_clocks(&mut self) {
        let (an, ln) = self.gather_evidence();
        if an != self.ev.anchors || ln != self.ev.links {
            self.ev.clocks = clock::solve(&an, &ln);
            (self.ev.anchors, self.ev.links) = (an, ln);
        }
    }

    /// Switches a piece of evidence on or off (by its key); kept in the project and logged in the
    /// result file's setting changes.
    #[allow(dead_code)] // the Sync list (timeline) uses it
    pub(crate) fn set_evidence(&mut self, key: &str, on: bool) {
        let was = !self.ev.off.contains(key);
        if was == on {
            return;
        }
        if on {
            self.ev.off.remove(key);
        } else {
            self.ev.off.insert(key.to_string());
        }
        self.changes.push(bus::SettingChange {
            utc: timebase::now_unix(),
            node: "clocks".into(),
            label: "evidence".into(),
            name: key.to_string(),
            old: if was { "on" } else { "off" }.into(),
            new: if on { "on" } else { "off" }.into(),
        });
        self.save_project();
    }

    /// Same clock: sources `nodes` share one clock (named by the smallest of their clocks), with
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

    /// Separate clocks: sources `nodes` each go back to their own clock.
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

    /// The project lines for clocks and evidence (see the top of this file).
    pub(crate) fn evidence_text(&self) -> String {
        let mut t = String::new();
        for (n, c) in &self.ev.share {
            t.push_str(&format!("clock|{}|{}\n", clean(n), clean(c)));
        }
        for k in &self.ev.off {
            t.push_str(&format!("off|{}\n", clean(k)));
        }
        for a in &self.ev.manual_anchors {
            t.push_str(&format!("anchor|{}|{}|{}|{}\n", clean(&a.clock), a.t, a.utc, a.error_s));
        }
        for l in &self.ev.manual_links {
            t.push_str(&format!("link|{}|{}|{}|{}|{}\n", clean(&l.a), l.t_a, clean(&l.b), l.t_b, l.error_s));
        }
        t
    }

    /// Reads one project line of clocks and evidence; false when it is not one.
    pub(crate) fn evidence_line(&mut self, f: &[&str]) -> bool {
        let num = |s: &str| s.trim().parse::<f64>().ok();
        match f {
            ["clock", n, c] => {
                self.ev.share.insert(n.to_string(), c.to_string());
            }
            ["off", k] => {
                self.ev.off.insert(k.to_string());
            }
            ["anchor", c, t, u, e] => {
                if let (Some(t), Some(utc), Some(error_s)) = (num(t), num(u), num(e)) {
                    self.ev.manual_anchors.push(ClockAnchor { clock: c.to_string(), t, utc, error_s, from: "manual".into(), enabled: true });
                }
            }
            ["link", a, ta, b, tb, e] => {
                if let (Some(t_a), Some(t_b), Some(error_s)) = (num(ta), num(tb), num(e)) {
                    self.ev.manual_links.push(Link { a: a.to_string(), t_a, b: b.to_string(), t_b, error_s, from: "manual".into(), enabled: true });
                }
            }
            _ => return false,
        }
        true
    }
}
