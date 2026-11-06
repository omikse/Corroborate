//! Clocks: which clock a time is on, how clocks relate to each other and to UTC
//! (Coordinated Universal Time), and how sure that is.
//!
//! Each source (a file, a stream, a device) has its own clock, named by a `ClockId`: by default
//! its source node's stable id. `"utc"` is UTC itself, in Unix seconds. Modules and the user
//! give evidence:
//! - an `Anchor`: moment `t` on a clock is at `utc`;
//! - a `Link`: moment `t_a` on clock `a` is moment `t_b` on clock `b` (a link to `"utc"` is an
//!   anchor).
//!
//! `solve` turns the evidence into one `ClockInfo` per clock, through the weighted least squares
//! of `sync::solve`. Clocks joined by links form a group. A group with an anchor is absolute (on
//! UTC); one without is relative (to its reference clock, the first by id); a clock with no
//! evidence is unanchored (`Kind::None`). Errors are 1 sigma and add up along chains by
//! root-sum-square. Each clock's error is split into the share of each piece of evidence
//! (`breakdown`): the solve is linear, so a share is exactly (sensitivity × error)². Evidence more
//! than 3 sigma off the solution is a contradiction. Drift is not modelled: only offsets.

use crate::sync::{self, AnchorObs, Obs, Offset};
use std::collections::{BTreeMap, BTreeSet};

pub type ClockId = String;

/// UTC's clock id. A time on it is Unix seconds.
pub const UTC: &str = "utc";

/// Smallest error used in the solve, seconds: an error of 0 would be infinitely sure.
const MIN_ERROR_S: f64 = 1e-3;

/// Residuals over this many sigmas are contradictions.
const CONTRADICTION_SIGMA: f64 = 3.0;

/// Moment `t` on `clock` is at `utc` (Unix seconds), ± `error_s` (1 sigma).
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub clock: ClockId,
    pub t: f64,
    pub utc: f64,
    pub error_s: f64,
    /// Stable id of the node that gave it, or `"manual"`.
    pub from: String,
    pub enabled: bool,
}

/// Moment `t_a` on clock `a` is moment `t_b` on clock `b`, ± `error_s` (1 sigma).
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub a: ClockId,
    pub t_a: f64,
    pub b: ClockId,
    pub t_b: f64,
    pub error_s: f64,
    /// Stable id of the node that gave it, or `"manual"`.
    pub from: String,
    pub enabled: bool,
}

/// How a clock is tied down. Ordered as the timeline lists groups: absolute first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Absolute,
    Relative,
    None,
}

/// A piece of evidence: its index in `solve`'s anchors or links.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ev {
    Anchor(usize),
    Link(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClockInfo {
    pub kind: Kind,
    /// Absolute: the UTC of this clock's 0. Relative: what to add to a time on this clock to get
    /// the time on the group's reference clock. None: 0.
    pub offset: f64,
    /// The offset's error, seconds (1 sigma).
    pub error_s: f64,
    /// `"utc"` for absolute clocks, the reference clock for relative ones, itself when unanchored.
    pub group: ClockId,
    /// Each piece of evidence's share of the variance (error²), largest first. Sums to error².
    pub breakdown: Vec<(Ev, f64)>,
}

impl ClockInfo {
    fn alone(id: &str) -> ClockInfo {
        ClockInfo { kind: Kind::None, offset: 0.0, error_s: 0.0, group: id.into(), breakdown: Vec::new() }
    }
}

/// The solved clocks of a project.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clocks {
    /// Every clock named by the evidence (switched off or not).
    pub clocks: BTreeMap<ClockId, ClockInfo>,
    /// Evidence more than 3 sigma off the solution, with its residual in sigmas.
    pub contradictions: Vec<(Ev, f64)>,
}

impl Clocks {
    /// A clock's info; a clock no evidence names is unanchored.
    pub fn get(&self, id: &str) -> ClockInfo {
        if id == UTC {
            return ClockInfo { kind: Kind::Absolute, offset: 0.0, error_s: 0.0, group: UTC.into(), breakdown: Vec::new() };
        }
        self.clocks.get(id).cloned().unwrap_or_else(|| ClockInfo::alone(id))
    }

    /// Time `t` on `clock` in UTC, with the clock's error, when the clock is absolute.
    pub fn to_utc(&self, clock: &str, t: f64) -> Option<(f64, f64)> {
        let c = self.get(clock);
        (c.kind == Kind::Absolute).then(|| (c.offset + t, c.error_s))
    }

    pub fn is_contradiction(&self, e: Ev) -> bool {
        self.contradictions.iter().any(|(c, _)| *c == e)
    }
}

/// An anchor in a group's own numbering: clock, offset (UTC of its 0), error.
type LocalAnchor = (Ev, usize, f64, f64);
/// A link in a group's own numbering: a, t_a, b, t_b, error.
type LocalLink = (Ev, usize, f64, usize, f64, f64);

/// Offsets of `m` clocks from the switched-on evidence of one connected group.
fn offsets(m: usize, anchors: &[LocalAnchor], links: &[LocalLink]) -> Vec<(f64, f64)> {
    // A link is an event seen on two clocks; its error is split so the pair adds up to it.
    let obs: Vec<Obs> = links
        .iter()
        .enumerate()
        .flat_map(|(k, &(_, a, t_a, b, t_b, e))| {
            let e = e / std::f64::consts::SQRT_2;
            [Obs { stream: a, event: k, t: t_a, error_s: e }, Obs { stream: b, event: k, t: t_b, error_s: e }]
        })
        .collect();
    let an: Vec<AnchorObs> = anchors.iter().map(|&(_, s, offset, e)| AnchorObs { stream: s, offset, error_s: e }).collect();
    sync::solve(m, &obs, &an)
        .offsets
        .into_iter()
        .map(|o| match o {
            Offset::Solved { offset, error_s } => (offset, error_s),
            Offset::Unsolved => (f64::NAN, f64::NAN),
        })
        .collect()
}

/// Solve every clock from the anchors and links (switched-off ones are ignored).
pub fn solve(anchors: &[Anchor], links: &[Link]) -> Clocks {
    let err = |e: f64| if e.is_finite() { e.max(MIN_ERROR_S) } else { MIN_ERROR_S };
    let mut names: BTreeSet<&str> = BTreeSet::new();
    names.extend(anchors.iter().map(|a| a.clock.as_str()));
    names.extend(links.iter().flat_map(|l| [l.a.as_str(), l.b.as_str()]));
    names.remove(UTC);
    let ids: Vec<&str> = names.into_iter().collect();
    let ix = |c: &str| ids.binary_search(&c).ok();

    // Evidence in clock numbers; a link to UTC is an anchor.
    let mut an: Vec<LocalAnchor> = Vec::new();
    let mut ln: Vec<LocalLink> = Vec::new();
    for (i, a) in anchors.iter().enumerate().filter(|(_, a)| a.enabled) {
        if let Some(c) = ix(&a.clock) {
            an.push((Ev::Anchor(i), c, a.utc - a.t, err(a.error_s)));
        }
    }
    for (i, l) in links.iter().enumerate().filter(|(_, l)| l.enabled) {
        let e = err(l.error_s);
        match (ix(&l.a), ix(&l.b)) {
            (Some(a), Some(b)) if a != b => ln.push((Ev::Link(i), a, l.t_a, b, l.t_b, e)),
            (Some(a), None) => an.push((Ev::Link(i), a, l.t_b - l.t_a, e)),
            (None, Some(b)) => an.push((Ev::Link(i), b, l.t_a - l.t_b, e)),
            _ => {}
        }
    }

    // Groups: clocks joined by links.
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    fn root(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for &(_, a, _, b, _, _) in &ln {
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        parent[ra.max(rb)] = ra.min(rb);
    }

    let roots: Vec<usize> = (0..ids.len()).map(|c| root(&mut parent, c)).collect();
    let mut out = Clocks::default();
    for r in 0..ids.len() {
        let members: Vec<usize> = (0..ids.len()).filter(|&c| roots[c] == r).collect();
        if members.is_empty() {
            continue;
        }
        let local = |c: usize| members.binary_search(&c).unwrap();
        let ga: Vec<LocalAnchor> = an.iter().filter(|x| roots[x.1] == r).map(|&(e, c, o, s)| (e, local(c), o, s)).collect();
        let gl: Vec<LocalLink> =
            ln.iter().filter(|x| roots[x.1] == r).map(|&(e, a, ta, b, tb, s)| (e, local(a), ta, local(b), tb, s)).collect();
        if ga.is_empty() && gl.is_empty() {
            for &c in &members {
                out.clocks.insert(ids[c].into(), ClockInfo::alone(ids[c]));
            }
            continue;
        }
        let m = members.len();
        let base = offsets(m, &ga, &gl);
        let (kind, group) = if ga.is_empty() { (Kind::Relative, ids[members[0]].to_string()) } else { (Kind::Absolute, UTC.to_string()) };

        // Each piece of evidence's share: move it by its error and see how far each clock moves.
        let mut shares: Vec<Vec<(Ev, f64)>> = vec![Vec::new(); m];
        for k in 0..ga.len() + gl.len() {
            let (mut pa, mut pl) = (ga.clone(), gl.clone());
            let ev = if k < ga.len() {
                pa[k].2 += pa[k].3;
                pa[k].0
            } else {
                let j = k - ga.len();
                pl[j].2 += pl[j].5;
                pl[j].0
            };
            for (c, (o, _)) in offsets(m, &pa, &pl).into_iter().enumerate() {
                let d = o - base[c].0;
                shares[c].push((ev, d * d));
            }
        }
        for (c, mut br) in shares.into_iter().enumerate() {
            br.retain(|(_, v)| *v > 1e-18);
            br.sort_by(|x, y| y.1.total_cmp(&x.1));
            let (offset, error_s) = base[c];
            out.clocks.insert(ids[members[c]].into(), ClockInfo { kind, offset, error_s, group: group.clone(), breakdown: br });
        }

        // Residuals of each piece of evidence against the solution.
        for &(ev, c, o, s) in &ga {
            let z = (base[c].0 - o) / s;
            if z.abs() > CONTRADICTION_SIGMA {
                out.contradictions.push((ev, z));
            }
        }
        for &(ev, a, ta, b, tb, s) in &gl {
            let z = (base[a].0 + ta - base[b].0 - tb) / s;
            if z.abs() > CONTRADICTION_SIGMA {
                out.contradictions.push((ev, z));
            }
        }
    }
    out.contradictions.sort_by(|x, y| x.0.cmp(&y.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(a: &str, t_a: f64, b: &str, t_b: f64, e: f64) -> Link {
        Link { a: a.into(), t_a, b: b.into(), t_b, error_s: e, from: "test".into(), enabled: true }
    }

    fn anchor(c: &str, t: f64, utc: f64, e: f64) -> Anchor {
        Anchor { clock: c.into(), t, utc, error_s: e, from: "test".into(), enabled: true }
    }

    #[test]
    fn errors_add_up_along_a_chain() {
        // a → b → c → d, each link ±1 s: d is ±√3 s from a.
        let l = [link("a", 10.0, "b", 4.0, 1.0), link("b", 7.0, "c", 1.0, 1.0), link("c", 2.0, "d", 50.0, 1.0)];
        let c = solve(&[], &l);
        let d = c.get("d");
        assert_eq!(d.kind, Kind::Relative);
        assert_eq!(d.group, "a");
        assert!((d.error_s - 3f64.sqrt()).abs() < 1e-3, "{}", d.error_s);
        // d's 50 is c's 2, which is b's 8, which is a's 14.
        assert!((d.offset + 50.0 - 14.0).abs() < 1e-6, "{}", d.offset);
        assert!(c.contradictions.is_empty());
    }

    #[test]
    fn an_anchor_makes_the_group_absolute() {
        let c = solve(&[anchor("b", 0.0, 1000.0, 2.0)], &[link("a", 5.0, "b", 3.0, 1.0)]);
        assert_eq!(c.get("a").kind, Kind::Absolute);
        assert_eq!(c.get("a").group, UTC);
        let (utc, e) = c.to_utc("a", 5.0).unwrap();
        assert!((utc - 1003.0).abs() < 1e-6);
        assert!((e - 5f64.sqrt()).abs() < 1e-3, "{e}");
        assert_eq!(c.to_utc("utc", 7.0), Some((7.0, 0.0)));
    }

    #[test]
    fn a_link_to_utc_is_an_anchor() {
        let c = solve(&[], &[link("a", 2.0, UTC, 1000.0, 1.0)]);
        assert_eq!(c.to_utc("a", 0.0).map(|x| x.0), Some(998.0));
    }

    #[test]
    fn two_agreeing_anchors_beat_one() {
        let one = solve(&[anchor("a", 0.0, 100.0, 2.0)], &[]);
        let two = solve(&[anchor("a", 0.0, 100.0, 2.0), anchor("a", 10.0, 110.5, 2.0)], &[]);
        assert!(two.get("a").error_s < one.get("a").error_s);
        assert!(two.contradictions.is_empty());
    }

    #[test]
    fn disagreeing_anchors_are_contradictions() {
        let c = solve(&[anchor("a", 0.0, 100.0, 1.0), anchor("a", 0.0, 120.0, 1.0)], &[]);
        assert_eq!(c.contradictions.iter().map(|x| x.0).collect::<Vec<_>>(), [Ev::Anchor(0), Ev::Anchor(1)]);
        assert!(c.is_contradiction(Ev::Anchor(1)));
    }

    #[test]
    fn a_clock_without_evidence_stays_unanchored() {
        let c = solve(&[anchor("a", 0.0, 100.0, 1.0)], &[link("b", 0.0, "c", 1.0, 1.0)]);
        assert_eq!(c.get("zzz").kind, Kind::None);
        assert_eq!(c.get("zzz").group, "zzz");
        assert_eq!(c.get("b").kind, Kind::Relative);
        assert_eq!(c.get("c").group, "b");
        assert_eq!(c.to_utc("b", 0.0), None);
    }

    #[test]
    fn the_breakdown_sums_to_the_variance() {
        let c = solve(
            &[anchor("a", 0.0, 100.0, 2.0), anchor("c", 0.0, 90.0, 3.0)],
            &[link("a", 1.0, "b", 0.0, 0.5), link("b", 4.0, "c", 15.0, 1.5), link("a", 3.0, "c", 13.0, 1.0)],
        );
        for (id, info) in &c.clocks {
            let sum: f64 = info.breakdown.iter().map(|x| x.1).sum();
            assert!((sum - info.error_s.powi(2)).abs() < 1e-6 * info.error_s.powi(2).max(1.0), "{id}: {sum} vs {}", info.error_s.powi(2));
            assert!(info.breakdown.windows(2).all(|w| w[0].1 >= w[1].1), "largest first");
        }
        // b hangs off a by its 0.5 s link, so a's anchor is b's biggest share.
        assert_eq!(c.get("b").breakdown[0].0, Ev::Anchor(0));
    }

    #[test]
    fn switching_evidence_off_changes_the_result() {
        let mut l = vec![link("a", 0.0, "b", 0.0, 1.0)];
        let a = [anchor("a", 0.0, 100.0, 1.0)];
        assert_eq!(solve(&a, &l).get("b").kind, Kind::Absolute);
        l[0].enabled = false;
        let c = solve(&a, &l);
        assert_eq!(c.get("b").kind, Kind::None);
        assert!(c.clocks.contains_key("b"), "a switched-off link still names its clocks");
    }
}
