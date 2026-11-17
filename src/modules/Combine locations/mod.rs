//! Combine node: locations added and taken away. Each wired location is a row with a sign: the +
//! rows are joined (all must hold: AND; any may: OR), then every − row is taken away. With no +
//! row it starts from the whole world.

use crate::area::WorldMask;
use crate::core::NodeSpec;

crate::builtin_manifest!();

/// The mode as written in project files and labels: "any" (OR) or "all" (AND).
pub fn mode(any: bool) -> &'static str {
    if any { "any" } else { "all" }
}

pub fn is_any(mode: &str) -> bool {
    mode == "any"
}

/// The node's name for a mode: its [[variant]] in module.toml.
pub fn label(any: bool) -> String {
    let m = manifest();
    m.variants.iter().find(|v| v.settings.iter().any(|(k, x)| k == "mode" && x == mode(any))).map_or_else(|| format!("{} ({})", m.label, mode(any)), |v| v.label.clone())
}

/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
}

/// The row a project keeps for a location taken away: by the stable id of the node it comes
/// from, so it keeps its sign when other wires come and go.
pub fn minus_key(uid: &str) -> String {
    format!("minus {uid}")
}

/// Whether the location from node `uid` is taken away.
pub fn is_minus(rows: &[(String, String)], uid: &str) -> bool {
    let k = minus_key(uid);
    rows.iter().any(|(q, v)| *q == k && v == "true")
}

/// Turn the sign of the location from node `uid` over.
pub fn flip(rows: &mut Vec<(String, String)>, uid: &str) {
    let k = minus_key(uid);
    if is_minus(rows, uid) {
        rows.retain(|(q, _)| *q != k);
    } else {
        rows.retain(|(q, _)| *q != k);
        rows.push((k, "true".into()));
    }
}

/// The + locations joined (`any`: OR, else AND), then every − location taken away. No +
/// location: the whole world, minus the − ones.
pub fn combine(any: bool, plus: &[&WorldMask], minus: &[&WorldMask]) -> WorldMask {
    let mut m = match plus.split_first() {
        None => WorldMask::world(),
        Some((first, rest)) => rest.iter().fold((*first).clone(), |m, x| if any { m.or(x) } else { m.and(x) }),
    };
    for x in minus {
        m = m.and(&x.not());
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(lat: f64, lon: f64) -> WorldMask {
        WorldMask::circle(lat, lon, 800.0)
    }

    #[test]
    fn plus_rows_join_by_mode() {
        let (a, b) = (circle(50.0, 10.0), circle(52.0, 14.0));
        assert_eq!(combine(false, &[&a, &b], &[]), a.and(&b));
        assert_eq!(combine(true, &[&a, &b], &[]), a.or(&b));
        assert_eq!(combine(false, &[&a], &[]), a);
    }

    #[test]
    fn minus_rows_are_taken_away() {
        let (a, b) = (circle(50.0, 10.0), circle(52.0, 14.0));
        let m = combine(false, &[&a], &[&b]);
        assert_eq!(m, a.and(&b.not()));
        assert!(m.and(&b).is_empty());
        assert!(m.fraction() < a.fraction() && !m.is_empty());
    }

    #[test]
    fn minus_only_starts_from_the_whole_world() {
        let b = circle(52.0, 14.0);
        assert_eq!(combine(true, &[], &[&b]), b.not());
        assert_eq!(combine(false, &[], &[]), WorldMask::world());
    }

    #[test]
    fn nothing_left_is_a_contradiction() {
        let (a, b) = (circle(50.0, 10.0), circle(-30.0, 140.0));
        assert!(combine(false, &[&a, &b], &[]).is_empty());
        assert!(combine(false, &[&a], &[&a]).is_empty());
    }

    #[test]
    fn signs_are_kept_by_node() {
        let mut rows = Vec::new();
        assert!(!is_minus(&rows, "n3"));
        flip(&mut rows, "n3");
        assert!(is_minus(&rows, "n3") && !is_minus(&rows, "n4"));
        flip(&mut rows, "n3");
        assert!(rows.is_empty());
    }
}
