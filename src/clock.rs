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
//! than 3 sigma off the solution is a contradiction.
//!
//! Clocks drift: each has a rate as well as an offset (`sync`), learnt from evidence at
//! different times on the same clocks and otherwise 0 ± `RATE_PRIOR`. So a clock's error is
//! smallest at its evidence (`center`) and grows away from it; `at`, `back` and `error_at` give
//! time and error at any moment. `offset` is where the clock's 0 lands, `error_s` the error at
//! `center`. Clocks of one device (several files of one phone, the cut parts of one file) run at
//! one speed: `fit_against` gives the clocks of a group that share a device one rate.
//!
//! There is no project-wide pool of evidence any more: Audio match fits its own links
//! (`fit_against`) and ADS-B check its own passes (`solve`); times reach other nodes on wires.

use crate::sync::{self, AnchorObs, Fit, Obs, Offset};
use std::collections::{BTreeMap, BTreeSet};

pub type ClockId = String;

/// UTC's clock id. A time on it is Unix seconds.
pub const UTC: &str = "utc";

/// Smallest error used in the solve, seconds: an error of 0 would be infinitely sure.
const MIN_ERROR_S: f64 = 1e-3;

/// Residuals over this many sigmas are contradictions.
const CONTRADICTION_SIGMA: f64 = 3.0;

/// How fast an unknown clock may run against UTC (1 sigma): 100 ppm, 0.36 s an hour. Good
/// phones drift 2–14 ppm, 13 of 16 devices measured were within 100 ppm, the worst 417 ppm
/// (Guggenberger, Lux, Böszörmenyi 2015).
pub const RATE_PRIOR: f64 = 100e-6;

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
    /// The error at `center` (where the clock's evidence is), seconds (1 sigma).
    pub error_s: f64,
    /// How much faster the group's time runs than this clock, minus 1 (1e-6 is 1 ppm), its
    /// error, and where the clock's evidence is (the mean time on the clock, seconds).
    pub rate: f64,
    pub rate_error: f64,
    pub center: f64,
    /// The solved fit; `at`, `back` and `error_at` read it.
    pub fit: Fit,
    /// `"utc"` for absolute clocks, the reference clock for relative ones, itself when unanchored.
    pub group: ClockId,
    /// Each piece of evidence's share of the variance (error²) at `center`, largest first.
    /// With `speed_share`, sums to error².
    pub breakdown: Vec<(Ev, f64)>,
    /// The share of the variance at `center` from clock speeds no evidence pins down.
    pub speed_share: f64,
}

const ZERO: Fit = Fit { offset: 0.0, rate: 0.0, center: 0.0, var_offset: 0.0, var_rate: 0.0, cov: 0.0 };

impl ClockInfo {
    fn alone(id: &str) -> ClockInfo {
        ClockInfo::from_fit(Kind::None, id.into(), ZERO, Vec::new(), 0.0)
    }

    fn from_fit(kind: Kind, group: ClockId, fit: Fit, breakdown: Vec<(Ev, f64)>, speed_share: f64) -> ClockInfo {
        ClockInfo {
            kind,
            offset: fit.at(0.0),
            error_s: fit.var_offset.sqrt(),
            rate: fit.rate,
            rate_error: fit.var_rate.sqrt(),
            center: fit.center,
            fit,
            group,
            breakdown,
            speed_share,
        }
    }

    /// Moment `t` on this clock in the group's time (UTC when absolute).
    pub fn at(&self, t: f64) -> f64 {
        self.fit.at(t)
    }

    /// The moment on this clock at group time `s`.
    pub fn back(&self, s: f64) -> f64 {
        self.fit.back(s)
    }

    /// The error (1 sigma) of `at(t)`: smallest at the evidence, growing with unknown drift.
    pub fn error_at(&self, t: f64) -> f64 {
        self.fit.error_at(t)
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
            return ClockInfo::from_fit(Kind::Absolute, UTC.into(), ZERO, Vec::new(), 0.0);
        }
        self.clocks.get(id).cloned().unwrap_or_else(|| ClockInfo::alone(id))
    }

    /// Time `t` on `clock` in UTC, with the clock's error, when the clock is absolute.
    pub fn to_utc(&self, clock: &str, t: f64) -> Option<(f64, f64)> {
        let c = self.get(clock);
        (c.kind == Kind::Absolute).then(|| (c.at(t), c.error_at(t)))
    }

    pub fn is_contradiction(&self, e: Ev) -> bool {
        self.contradictions.iter().any(|(c, _)| *c == e)
    }
}

/// An anchor in a group's own numbering: clock, t, offset there (UTC − t), error.
type LocalAnchor = (Ev, usize, f64, f64, f64);
/// A link in a group's own numbering: a, t_a, b, t_b, error.
type LocalLink = (Ev, usize, f64, usize, f64, f64);

/// Fits of `m` clocks from the switched-on evidence of one connected group.
fn offsets(m: usize, anchors: &[LocalAnchor], links: &[LocalLink], rate_of: &[usize]) -> Vec<Fit> {
    // A link is an event seen on two clocks; its error is split so the pair adds up to it.
    let obs: Vec<Obs> = links
        .iter()
        .enumerate()
        .flat_map(|(k, &(_, a, t_a, b, t_b, e))| {
            let e = e / std::f64::consts::SQRT_2;
            [Obs { stream: a, event: k, t: t_a, error_s: e }, Obs { stream: b, event: k, t: t_b, error_s: e }]
        })
        .collect();
    let an: Vec<AnchorObs> = anchors.iter().map(|&(_, s, t, offset, e)| AnchorObs { stream: s, t, offset, error_s: e }).collect();
    sync::solve_shared(m, &obs, &an, RATE_PRIOR, rate_of)
        .offsets
        .into_iter()
        .map(|o| match o {
            Offset::Solved(f) => f,
            Offset::Unsolved => Fit { offset: f64::NAN, var_offset: f64::NAN, ..ZERO },
        })
        .collect()
}

/// Solve every clock from the anchors and links (switched-off ones are ignored).
pub fn solve(anchors: &[Anchor], links: &[Link]) -> Clocks {
    solve_inner(anchors, links, &BTreeMap::new(), true)
}

/// As `solve`, without each piece of evidence's share of the error (`breakdown` stays empty and
/// `speed_share` 0): one fit instead of one per piece of evidence, for a node that fits its own
/// links. A relative group's reference clock (the first by id) has offset 0 and speed 0, so every
/// other clock's fit is against it.
pub fn fit(anchors: &[Anchor], links: &[Link]) -> Clocks {
    solve_inner(anchors, links, &BTreeMap::new(), false)
}

/// Each group of linked clocks fitted from `links`, in the seconds of the group's average clock:
/// every clock's speed has the same prior (±`RATE_PRIOR` against real time, as in `solve`), so
/// the fit puts the group's speeds around 0, one vote per speed, each weighted by how well the
/// links measure it, none dropped. Clocks with the same `device` (files of one phone) share one
/// speed: one vote. Holding one clock still instead, and pulling the others toward its speed,
/// placed Jiku GT_090912's starts 21 ms off the truth (median, 168 ms for 95 %) against 4 ms
/// (25 ms), read in that clock's seconds.
///
/// Moment t on a clock is `at(t)` seconds of the average clock from the start (moment 0) of the
/// group's reference clock (the one in `refs` it holds, else its first by id); `read_in` reads a
/// fit in another clock's seconds. The errors are each clock's against the reference, from a fit
/// with the reference held still. Also each link's leftover: how far apart its two moments land,
/// seconds (NaN off the fit).
pub fn fit_against(links: &[Link], refs: &BTreeSet<ClockId>, device: &BTreeMap<ClockId, String>) -> (BTreeMap<ClockId, Fit>, Vec<f64>) {
    // The reference held still: the solve measures a relative group against its first clock by
    // id, so the reference is named to sort first. This gives the groups and the errors.
    let name = |c: &str| if refs.contains(c) { format!("0{c}") } else { format!("1{c}") };
    let renamed: Vec<Link> = links.iter().map(|l| Link { a: name(&l.a), b: name(&l.b), ..l.clone() }).collect();
    let renamed_device: BTreeMap<ClockId, String> = device.iter().map(|(c, d)| (name(c), d.clone())).collect();
    let held = solve_inner(&[], &renamed, &renamed_device, false);
    let ref_of: BTreeMap<ClockId, ClockId> = held.clocks.iter().map(|(k, c)| (k[1..].to_string(), c.group[1..].to_string())).collect();
    // Every clock alike, each group anchored on its reference (where does not matter: it is
    // measured from the reference's start). This gives the values.
    let roots: BTreeSet<&ClockId> = ref_of.values().collect();
    let anchors: Vec<Anchor> = roots.iter().map(|c| Anchor { clock: (*c).clone(), t: 0.0, utc: 0.0, error_s: 1.0, from: String::new(), enabled: true }).collect();
    let alike = solve_inner(&anchors, links, device, false);
    let mut out = BTreeMap::new();
    for (c, f) in &ref_of {
        let (gc, gf) = (alike.get(c).fit, alike.get(f).fit);
        let err = held.get(&name(c)).fit;
        if !gc.offset.is_finite() || !gf.offset.is_finite() {
            continue;
        }
        let fit = Fit { offset: gc.offset - gf.at(0.0), var_offset: err.var_offset, var_rate: err.var_rate, cov: err.cov, ..gc };
        out.insert(c.clone(), fit);
    }
    let leftover = links
        .iter()
        .map(|l| match (out.get(&l.a), out.get(&l.b)) {
            (Some(a), Some(b)) if ref_of.get(&l.a) == ref_of.get(&l.b) => a.at(l.t_a) - b.at(l.t_b),
            _ => f64::NAN,
        })
        .collect();
    (out, leftover)
}

/// As `solve`, but clocks with the same `device` run at one speed (each keeps its own offset),
/// within a group of linked clocks. Used by the Jiku evaluation (tests/jiku.rs) on all its links
/// at once; in the app, Audio match fits each of its groups with `fit_against`.
pub fn solve_devices(anchors: &[Anchor], links: &[Link], device: &BTreeMap<ClockId, String>) -> Clocks {
    solve_inner(anchors, links, device, true)
}

/// Fit `c` read in the seconds of clock `r` (both fitted to one time): moment t on `c` is moment
/// `at(t)` on `r`. Keeps `c`'s errors.
pub fn read_in(c: &Fit, r: &Fit) -> Fit {
    Fit { offset: r.back(c.at(c.center)) - c.center, rate: (1.0 + c.rate) / (1.0 + r.rate) - 1.0, ..*c }
}

fn solve_inner(anchors: &[Anchor], links: &[Link], device: &BTreeMap<ClockId, String>, shares: bool) -> Clocks {
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
            an.push((Ev::Anchor(i), c, a.t, a.utc - a.t, err(a.error_s)));
        }
    }
    for (i, l) in links.iter().enumerate().filter(|(_, l)| l.enabled) {
        let e = err(l.error_s);
        match (ix(&l.a), ix(&l.b)) {
            (Some(a), Some(b)) if a != b => ln.push((Ev::Link(i), a, l.t_a, b, l.t_b, e)),
            (Some(a), None) => an.push((Ev::Link(i), a, l.t_a, l.t_b - l.t_a, e)),
            (None, Some(b)) => an.push((Ev::Link(i), b, l.t_b, l.t_a - l.t_b, e)),
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
        let ga: Vec<LocalAnchor> = an.iter().filter(|x| roots[x.1] == r).map(|&(e, c, t, o, s)| (e, local(c), t, o, s)).collect();
        let gl: Vec<LocalLink> =
            ln.iter().filter(|x| roots[x.1] == r).map(|&(e, a, ta, b, tb, s)| (e, local(a), ta, local(b), tb, s)).collect();
        if ga.is_empty() && gl.is_empty() {
            for &c in &members {
                out.clocks.insert(ids[c].into(), ClockInfo::alone(ids[c]));
            }
            continue;
        }
        let m = members.len();
        // Rate keys: clocks of one device share theirs, every other clock has its own.
        let rate_of: Vec<usize> = (0..m)
            .map(|c| match device.get(ids[members[c]]) {
                Some(d) => (0..m).find(|&o| device.get(ids[members[o]]) == Some(d)).unwrap(),
                None => c,
            })
            .collect();
        let base = offsets(m, &ga, &gl, &rate_of);
        let (kind, group) = if ga.is_empty() { (Kind::Relative, ids[members[0]].to_string()) } else { (Kind::Absolute, UTC.to_string()) };

        // Each piece of evidence's share: move it by its error and see how far each clock moves.
        let mut parts: Vec<Vec<(Ev, f64)>> = vec![Vec::new(); m];
        for k in (0..ga.len() + gl.len()).filter(|_| shares) {
            let (mut pa, mut pl) = (ga.clone(), gl.clone());
            let ev = if k < ga.len() {
                pa[k].3 += pa[k].4;
                pa[k].0
            } else {
                let j = k - ga.len();
                pl[j].2 += pl[j].5;
                pl[j].0
            };
            for (c, f) in offsets(m, &pa, &pl, &rate_of).into_iter().enumerate() {
                let at = base[c].center;
                let d = f.at(at) - base[c].at(at);
                parts[c].push((ev, d * d));
            }
        }
        for (c, mut br) in parts.into_iter().enumerate() {
            br.retain(|(_, v)| *v > 1e-18);
            br.sort_by(|x, y| y.1.total_cmp(&x.1));
            let speed_share = if shares { (base[c].var_offset - br.iter().map(|x| x.1).sum::<f64>()).max(0.0) } else { 0.0 };
            out.clocks.insert(ids[members[c]].into(), ClockInfo::from_fit(kind, group.clone(), base[c], br, speed_share));
        }

        // Residuals of each piece of evidence against the solution.
        for &(ev, c, t, o, s) in &ga {
            let z = (base[c].at(t) - t - o) / s;
            if z.abs() > CONTRADICTION_SIGMA {
                out.contradictions.push((ev, z));
            }
        }
        for &(ev, a, ta, b, tb, s) in &gl {
            let z = (base[a].at(ta) - base[b].at(tb)) / s;
            if z.abs() > CONTRADICTION_SIGMA {
                out.contradictions.push((ev, z));
            }
        }
    }
    out.contradictions.sort_by(|x, y| x.0.cmp(&y.0));
    out
}

impl Anchor {
    /// What names it for its on/off switch (saved in the project): who gave it, and where.
    pub fn key(&self) -> String {
        format!("anchor {} {} {:.3}", self.from, self.clock, self.t)
    }
}

impl Link {
    /// What names it for its on/off switch (saved in the project): who gave it, and where.
    pub fn key(&self) -> String {
        format!("link {} {} {:.3} {} {:.3}", self.from, self.a, self.t_a, self.b, self.t_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real date's size must not cost precision: one loose anchor at today's UTC and many tight
    /// links give the same clocks as the anchor at 0, moved by that much.
    #[test]
    fn a_real_date_does_not_cost_precision() {
        const DATE: f64 = 1_347_196_681.0;
        let ids: Vec<String> = (0..12).map(|k| format!("c{k:02}")).collect();
        let mut links = Vec::new();
        for a in 0..12 {
            for b in a + 1..12 {
                for k in 0..8 {
                    let t = 10.0 + 20.0 * k as f64;
                    let wobble = ((a * 7 + b * 13 + k * 3) % 11) as f64 * 5e-4;
                    links.push(Link { a: ids[a].clone(), t_a: t, b: ids[b].clone(), t_b: t + (a as f64 - b as f64) * 17.0 + wobble, error_s: 0.004, from: "m".into(), enabled: true });
                }
            }
        }
        let anchor = |utc: f64| vec![Anchor { clock: ids[0].clone(), t: 0.0, utc, error_s: 149.0, from: "s".into(), enabled: true }];
        let (near, far) = (solve(&anchor(0.0), &links), solve(&anchor(DATE), &links));
        assert_eq!(far.contradictions.len(), near.contradictions.len(), "the same evidence agrees just as well");
        for id in &ids {
            let (n, f) = (near.get(id), far.get(id));
            assert!((f.at(0.0) - DATE - n.at(0.0)).abs() < 1e-3, "{id}: {} against {}", f.at(0.0) - DATE, n.at(0.0));
            assert!((f.rate - n.rate).abs() < 1e-9, "{id}: rate {} against {}", f.rate, n.rate);
        }
    }

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
            let sum: f64 = info.breakdown.iter().map(|x| x.1).sum::<f64>() + info.speed_share;
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
    #[test]
    fn a_drifting_clock_is_fitted_not_contradicted() {
        // b runs 300 ppm slow against a over an hour: links 5 ms each, every 10 minutes.
        let l: Vec<Link> = (0..7).map(|k| {
            let t_a = 600.0 * k as f64;
            link("a", t_a, "b", (t_a - 20.0) / (1.0 + 300e-6), 0.005)
        }).collect();
        let c = solve(&[anchor("a", 0.0, 1000.0, 0.01)], &l);
        assert!(c.contradictions.is_empty(), "{:?}", c.contradictions);
        let (a, b) = (c.get("a"), c.get("b"));
        // Only the speed of b against a is known; against UTC each keeps half the unknown.
        assert!((b.rate - a.rate - 300e-6).abs() < 3e-6, "rates {} {}", a.rate, b.rate);
        // 3000 s on b is 20 s + 3000.9 s on a.
        let on_a = a.back(b.at(3000.0));
        assert!((on_a - (20.0 + 3000.0 * (1.0 + 300e-6))).abs() < 0.01, "{on_a}");
        assert!((b.back(b.at(1234.0)) - 1234.0).abs() < 1e-9);
        // Without rates the hour's 1 s slide would contradict the links.
        let fixed = |l: &Link| (l.t_a - 20.0 - l.t_b).abs();
        assert!(fixed(&l[6]) > 1.0);
    }

    /// `fit` is `solve` without the breakdown: the same fits, measured against the first clock.
    #[test]
    fn fit_is_solve_without_the_breakdown() {
        let l = [link("a", 1.0, "b", 0.0, 0.5), link("b", 4.0, "c", 15.0, 1.5), link("a", 300.0, "c", 312.1, 1.0)];
        let (full, quick) = (solve(&[], &l), fit(&[], &l));
        for id in ["a", "b", "c"] {
            assert_eq!(full.get(id).fit, quick.get(id).fit, "{id}");
            assert!(quick.get(id).breakdown.is_empty());
        }
        assert_eq!((quick.get("a").offset, quick.get("a").rate), (0.0, 0.0), "a is the reference");
    }

    #[test]
    fn files_of_one_phone_share_its_speed() {
        // b1 and b2 are two files of one phone, 300 ppm slow against a. b1 overlaps a for an
        // hour; b2 is linked once.
        let mut l: Vec<Link> = (0..7).map(|k| {
            let t_a = 600.0 * k as f64;
            link("a", t_a, "b1", (t_a - 20.0) / (1.0 + 300e-6), 0.005)
        }).collect();
        l.push(link("a", 5000.0, "b2", (5000.0 - 4000.0) / (1.0 + 300e-6), 0.005));
        let phone: BTreeMap<ClockId, String> = [("b1".into(), "phone".into()), ("b2".into(), "phone".into())].into();
        let (alone, shared) = (solve(&[], &l).get("b2"), solve_inner(&[], &l, &phone, true).get("b2"));
        assert!(alone.rate.abs() < 1e-9 && (shared.rate - 300e-6).abs() < 3e-6, "{} {}", alone.rate, shared.rate);
        let t = shared.center + 1800.0;
        assert!(shared.error_at(t) < 0.02 && alone.error_at(t) > 0.15, "{} {}", shared.error_at(t), alone.error_at(t));
    }

    #[test]
    fn one_anchor_errors_grow_with_unknown_drift() {
        let c = solve(&[anchor("a", 100.0, 1100.0, 0.01)], &[]);
        let a = c.get("a");
        assert!((a.error_s - 0.01).abs() < 1e-6 && a.center == 100.0);
        let (_, e) = c.to_utc("a", 100.0 + 3600.0).unwrap();
        // An hour from its anchor, an unknown speed adds 0.36 s (100 ppm).
        assert!((e - 0.36).abs() < 0.01, "{e}");
    }
}
