//! Cut detector node: where a recording is cut, and into which pieces. The method is in the
//! library (`crate::cuts`); the node picks its inputs, keeps the person's choices (thresholds,
//! cuts switched on or off, cuts by hand) in its rows, and gives each piece as a recording of its
//! own: `audio n`, `video n`, `time n` and `length n`.
//!
//! Rows saved with the project: `pieces` (how many there were, so wires into piece outputs hold
//! while a project opens), `thr <lane>` (a lane's threshold) and `cut <seconds>` (`on`, `off` or
//! `hand`).

use crate::core::{NodeSpec, Port, PortKind};
use crate::module_manifest::intern;

crate::builtin_manifest!();

/// The outputs of one piece, in order.
pub const PIECE_PORTS: [&str; 4] = ["audio", "video", "time", "length"];

/// Piece `n`'s output of kind `base` ("audio 3").
pub fn piece_port(base: &str, n: usize) -> &'static str {
    intern(&format!("{base} {n}"))
}

/// Which piece an output is of, and what it is: ("audio", 3) for "audio 3".
pub fn piece_of_port(name: &str) -> Option<(&'static str, usize)> {
    let (base, n) = name.rsplit_once(' ')?;
    let base = PIECE_PORTS.iter().find(|b| **b == base)?;
    Some((base, n.parse().ok().filter(|n| *n >= 1)?))
}

fn kind_of(base: &str) -> PortKind {
    match base {
        "audio" => PortKind::Audio,
        "video" => PortKind::Video,
        "time" => PortKind::Time,
        _ => PortKind::Custom("length"),
    }
}

/// A saved row's value.
pub fn get<'a>(rows: &'a [(String, String)], key: &str) -> Option<&'a str> {
    rows.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim())
}

/// How many pieces the rows say there were (at least one).
pub fn saved_pieces(rows: &[(String, String)]) -> usize {
    get(rows, "pieces").and_then(|v| v.parse().ok()).unwrap_or(1).clamp(1, 512)
}

/// The outputs of `n` pieces after `cuts`.
pub fn outputs(n: usize) -> Vec<Port> {
    let mut v = vec![Port::req("cuts", PortKind::Event)];
    for k in 1..=n {
        v.extend(PIECE_PORTS.iter().map(|b| Port::req(piece_port(b, k), kind_of(b))));
    }
    v
}

/// Its ports: its module.toml's inputs, and the outputs of the pieces its rows say it has.
pub fn spec(rows: &[(String, String)]) -> NodeSpec {
    let mut s = manifest().spec();
    s.outputs = outputs(saved_pieces(rows));
    s
}

/// The person's switches in the rows: (seconds, choice).
pub fn switches(rows: &[(String, String)]) -> Vec<(f64, crate::cuts::Switch)> {
    rows.iter().filter_map(|(k, v)| Some((k.strip_prefix("cut ")?.trim().parse().ok()?, crate::cuts::Switch::of(v)?))).collect()
}

/// A lane's threshold, when the person set one.
pub fn threshold(rows: &[(String, String)], lane: &str) -> Option<f32> {
    get(rows, &format!("thr {lane}")).and_then(|v| v.parse().ok()).filter(|v: &f32| v.is_finite())
}

/// Sets row `key` to `v` (None: removes it).
pub fn set(rows: &mut Vec<(String, String)>, key: &str, v: Option<String>) {
    rows.retain(|(k, _)| k != key);
    if let Some(v) = v {
        rows.push((key.to_string(), v));
    }
}

/// A switch's row key: the cut's time to the millisecond.
pub fn cut_key(t: f64) -> String {
    format!("cut {t:.3}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cuts::Switch;

    #[test]
    fn ports_follow_the_saved_pieces() {
        let s = spec(&[]);
        let ins: Vec<&str> = s.inputs.iter().map(|p| p.name).collect();
        assert_eq!(ins, ["audio", "video", "time", "compare with 1", "events 1"]);
        let outs: Vec<&str> = s.outputs.iter().map(|p| p.name).collect();
        assert_eq!(outs, ["cuts", "audio 1", "video 1", "time 1", "length 1"]);
        let rows = vec![("pieces".to_string(), "3".to_string())];
        assert_eq!(spec(&rows).outputs.len(), 1 + 3 * 4);
        assert_eq!(piece_of_port("length 3"), Some(("length", 3)));
        assert_eq!(piece_of_port("cuts"), None);
        assert_eq!(piece_of_port("compare with 2"), None);
    }

    #[test]
    fn switches_and_thresholds_are_rows() {
        let mut rows = Vec::new();
        set(&mut rows, &cut_key(12.3456), Some("off".into()));
        set(&mut rows, &cut_key(40.0), Some("hand".into()));
        set(&mut rows, "thr picture", Some("14".into()));
        assert_eq!(switches(&rows), [(12.346, Switch::Off), (40.0, Switch::Hand)]);
        assert_eq!(threshold(&rows, "picture"), Some(14.0));
        set(&mut rows, &cut_key(40.0), None);
        assert_eq!(switches(&rows).len(), 1);
    }
}
