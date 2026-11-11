//! The trace of a time: where the time on an output came from, walked back along the wires.
//!
//! Nothing extra travels on a wire. The trace is read from the nodes as they are now, so it cannot
//! disagree with the canvas: each node the time passed, what it did to the time, and the time it
//! gave. A port's hover shows it, and the result file writes it for every time output.

use super::*;
use corroborate::data::Data;

/// One node a time passed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Step {
    pub node: String,
    pub did: String,
    pub value: String,
}

/// At most this many steps: a longer chain is cut, oldest first.
const MAX_STEPS: usize = 32;

impl App {
    /// The trace of the time on output `out` of node `i`, oldest first.
    pub(crate) fn trace(&self, i: NodeId, out: &str) -> Vec<Step> {
        let mut steps = Vec::new();
        let mut seen = Vec::new();
        let mut at = Some((i, out.to_string()));
        while let Some((n, o)) = at.take() {
            if seen.contains(&(n, o.clone())) || steps.len() >= MAX_STEPS {
                break;
            }
            seen.push((n, o.clone()));
            let v = self.value_from(n, &o);
            let value = match &v {
                Ok(Data::Time { utc: Some(u), error_s, .. }) => format!("{} ±{error_s:.3} s", timebase::fmt_utc(*u)),
                Ok(d) => crate::bus::summary(d),
                Err(e) => format!("nothing: {e}"),
            };
            steps.push(Step { node: self.nodes[n].label.clone(), did: self.did(n, &o, &v), value });
            at = self.time_before(n, &o).map(|(f, fo)| (f, fo.to_string()));
        }
        steps.reverse();
        steps
    }

    /// What node `i` did to the time on output `out`.
    fn did(&self, i: NodeId, out: &str, v: &Result<Data, String>) -> String {
        match &self.nodes[i].kind {
            Kind::Time { .. } => self.time_did(i),
            Kind::AudioMatch { .. } => {
                let n = out.strip_prefix("end ").or(out.strip_prefix("synced ")).and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
                if out.starts_with("end ") && self.end_passes(i, n) {
                    return "passed its wired end time straight through".into();
                }
                let own = match v {
                    Ok(Data::Time { source, .. }) => source.clone(),
                    _ => String::new(),
                };
                let view = self.match_view(i);
                match view.rows.get(n.wrapping_sub(1)).and_then(|r| r.off) {
                    Some(d) => format!("{own}; this row's own time was {d:+.3} s off"),
                    None => own,
                }
            }
            _ => match v {
                Ok(Data::Time { source, .. }) => source.clone(),
                _ => String::new(),
            },
        }
    }

    /// The output the time on output `out` of node `i` was made from, if it came along a wire.
    fn time_before(&self, i: NodeId, out: &str) -> Option<(NodeId, &'static str)> {
        match &self.nodes[i].kind {
            Kind::Time { .. } => self.graph.feeder(i, "time"),
            Kind::AudioMatch { .. } => {
                let (end, n) = match (out.strip_prefix("synced "), out.strip_prefix("end ")) {
                    (Some(n), _) => (false, n.parse::<usize>().ok()?),
                    (_, Some(n)) => (true, n.parse::<usize>().ok()?),
                    _ => return None,
                };
                if end && self.end_passes(i, n) {
                    return self.graph.feeder(i, &format!("end in {n}"));
                }
                // Anchored: the anchor row's time; unanchored: the row's own.
                let v = self.match_view(i);
                let r = v.rows.get(n.checked_sub(1)?)?;
                let k = v.groups.get(r.group).and_then(|g| g.anchor).unwrap_or(n - 1);
                self.graph.feeder(i, &format!("time {}", k + 1))
            }
            _ => None,
        }
    }

    /// Audio match row `n`'s wired end goes out unchanged: audio did not link the row.
    fn end_passes(&self, i: NodeId, n: usize) -> bool {
        let v = self.match_view(i);
        self.graph.feeder(i, &format!("end in {n}")).is_some() && !v.rows.get(n.wrapping_sub(1)).is_some_and(|r| v.linked(r))
    }

    /// The trace as hover text: a numbered line per node, oldest first.
    pub(crate) fn trace_text(&self, i: NodeId, out: &str) -> String {
        let steps = self.trace(i, out);
        let lines: Vec<String> = steps.iter().enumerate().map(|(k, s)| format!("{} {}: {}\n   {}", k + 1, s.node, s.did, s.value)).collect();
        format!("where it came from, walked back along the wires:\n{}", lines.join("\n"))
    }

    /// The traces of node `i`'s time outputs that give a time, for the result file:
    /// `,"traces":{"out":[{"node":…,"did":…,"value":…},…]}`, or nothing.
    pub(crate) fn traces_json(&self, i: NodeId) -> String {
        let outs: Vec<&'static str> = self.graph.nodes()[i].spec.outputs.iter().filter(|p| p.kind == PortKind::Time).map(|p| p.name).collect();
        let t: Vec<String> = outs
            .into_iter()
            .filter(|o| self.value_from(i, o).is_ok())
            .map(|o| {
                let s: Vec<String> = self
                    .trace(i, o)
                    .iter()
                    .map(|s| format!("{{\"node\":\"{}\",\"did\":\"{}\",\"value\":\"{}\"}}", json_escape(&s.node), json_escape(&s.did), json_escape(&s.value)))
                    .collect();
                format!("\"{}\":[{}]", json_escape(o), s.join(","))
            })
            .collect();
        if t.is_empty() { String::new() } else { format!(",\"traces\":{{{}}}", t.join(",")) }
    }
}
