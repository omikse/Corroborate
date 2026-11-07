//! Sync solver: finds each stream's clock offset and speed from events heard (or seen) in
//! several streams at once, plus any anchors (moments whose shared time is already known).
//!
//! Model, for stream i and shared event k:
//!     t_ik + offset_i + rate_i · (t_ik − center_i) = T_k   (± error_ik)
//! where t_ik is the event's time on stream i's own clock, offset_i turns that clock into the
//! shared one at `center_i` (the mean time of the stream's evidence), rate_i is how much faster
//! the shared time runs than the stream's clock (clock drift: 1e-6 is 1 ppm, 3.6 ms an hour),
//! and T_k is when the event happened. An anchor says moment t on stream i is at shared time u.
//! The error of an observation should include the travel-time spread of the sound (up to
//! distance / 343 m/s), which is what the location step later uses.
//!
//! Recording devices drift by a few to a few hundred ppm (Guggenberger, Lux, Böszörmenyi 2015,
//! "An Analysis of Time Drift in Hand-Held Recording Devices": 2–14 ppm for good phones, up to
//! 417 ppm audio and −554 ppm video), so each rate has a prior of 0 ± `rate_prior` (1 sigma).
//! A rate is only learnt from evidence at different times on the same pair of clocks; without
//! such evidence it stays 0 and its prior makes errors grow away from the evidence.
//! Streams recorded by one device (several files of one phone, the cut parts of one file) run at
//! one speed: `solve_shared` gives them one rate, each keeping its own offset, so evidence on
//! any of them teaches the speed of all.
//!
//! Weighted least squares over all offsets, rates and event times. Without any anchor, the first
//! connected stream is the reference (offset 0, rate 0) and the result is relative to it. Streams
//! with no path of shared events to an anchor or the reference are reported as unsolved,
//! never guessed. Every offset and rate comes with its error, and every observation with its
//! residual, so outliers (wrong matches) are visible.

#[derive(Clone, Debug, PartialEq)]
pub struct Obs {
    pub stream: usize,
    pub event: usize,
    /// Event time on the stream's own clock, seconds.
    pub t: f64,
    /// One-sigma error, seconds. Must be > 0.
    pub error_s: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnchorObs {
    pub stream: usize,
    /// The moment on the stream's clock, seconds.
    pub t: f64,
    /// Shared time (UTC, Unix seconds) of that moment, minus `t`: the offset it implies there.
    pub offset: f64,
    pub error_s: f64,
}

/// A solved clock: shared time of moment t is `t + offset + rate · (t − center)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    /// Offset at `center`, seconds.
    pub offset: f64,
    /// Rate of the shared time against this clock, minus 1 (1e-6 is 1 ppm).
    pub rate: f64,
    /// The mean time of this clock's evidence, seconds on the clock.
    pub center: f64,
    /// Variance of `offset`, of `rate`, and their covariance.
    pub var_offset: f64,
    pub var_rate: f64,
    pub cov: f64,
}

impl Fit {
    /// Shared time of moment `t` on this clock.
    pub fn at(&self, t: f64) -> f64 {
        t + self.offset + self.rate * (t - self.center)
    }

    /// The moment on this clock at shared time `s` (inverse of `at`).
    pub fn back(&self, s: f64) -> f64 {
        (s - self.offset + self.rate * self.center) / (1.0 + self.rate)
    }

    /// Error (1 sigma) of `at(t)`.
    pub fn error_at(&self, t: f64) -> f64 {
        let d = t - self.center;
        (self.var_offset + d * d * self.var_rate + 2.0 * d * self.cov).max(0.0).sqrt()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Offset {
    /// Absolute (UTC) when anchors were given, otherwise relative to the reference stream.
    Solved(Fit),
    /// No chain of shared events to an anchor or the reference.
    Unsolved,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Solution {
    pub offsets: Vec<Offset>,
    /// True when anchors fixed the result to UTC; false when relative to `reference`.
    pub absolute: bool,
    pub reference: Option<usize>,
    /// Per observation (same order as the input): residual in seconds and in sigmas.
    pub residuals: Vec<(f64, f64)>,
    /// Per anchor (same order as the input): residual in seconds and in sigmas.
    pub anchor_residuals: Vec<(f64, f64)>,
}

impl Solution {
    /// Observations more than `k` sigmas off: likely wrong matches.
    pub fn outliers(&self, k: f64) -> Vec<usize> {
        self.residuals
            .iter()
            .enumerate()
            .filter(|(_, (_, z))| z.abs() > k)
            .map(|(i, _)| i)
            .collect()
    }
}

/// Solve offsets and rates for `n_streams` streams; `rate_prior` is the 1-sigma spread of an
/// unknown rate (0 pins every rate at 0: offsets only).
pub fn solve(n_streams: usize, obs: &[Obs], anchors: &[AnchorObs], rate_prior: f64) -> Solution {
    let own: Vec<usize> = (0..n_streams).collect();
    solve_shared(n_streams, obs, anchors, rate_prior, &own)
}

/// As `solve`, but streams with the same `rate_of` key share one rate (one device's clock).
/// Streams sharing the reference's key share its rate of 0.
pub fn solve_shared(n_streams: usize, obs: &[Obs], anchors: &[AnchorObs], rate_prior: f64, rate_of: &[usize]) -> Solution {
    // Which streams can be solved: connected through shared events to an anchor (or reference).
    let n_events = obs.iter().map(|o| o.event + 1).max().unwrap_or(0);
    let mut comp: Vec<usize> = (0..n_streams + n_events).collect();
    fn find(c: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while c[r] != r {
            r = c[r];
        }
        let mut y = x;
        while c[y] != r {
            let nx = c[y];
            c[y] = r;
            y = nx;
        }
        r
    }
    for o in obs {
        let (a, b) = (find(&mut comp, o.stream), find(&mut comp, n_streams + o.event));
        comp[a] = b;
    }
    let absolute = !anchors.is_empty();
    let reference = if absolute {
        None
    } else {
        obs.iter().map(|o| o.stream).min()
    };
    let roots: Vec<usize> = if absolute {
        anchors.iter().map(|a| find(&mut comp, a.stream)).collect()
    } else {
        reference.map(|r| vec![find(&mut comp, r)]).unwrap_or_default()
    };
    let solvable: Vec<bool> = (0..n_streams).map(|i| roots.contains(&find(&mut comp, i))).collect();

    // Each stream's center: the mean time of its evidence (keeps offset and rate apart).
    let mut sums = vec![(0.0, 0usize); n_streams];
    for (s, t) in obs.iter().map(|o| (o.stream, o.t)).chain(anchors.iter().map(|a| (a.stream, a.t))) {
        sums[s].0 += t;
        sums[s].1 += 1;
    }
    let center: Vec<f64> = sums.iter().map(|&(s, n)| if n > 0 { s / n as f64 } else { 0.0 }).collect();

    // Unknowns: the offset of each solvable stream and the rate of each solvable device (rate
    // key). The reference stream has neither: its offset and rate are zero by definition (the
    // gauge), and so is the rate of its device. Event times are unknowns too, but each appears
    // only in its own observations, so it is eliminated exactly (Schur complement) and worked out
    // afterwards: the system stays as small as the streams however many events.
    let is_ref = |i: usize| reference == Some(i);
    let ref_key = reference.map(|r| rate_of[r]);
    let mut n = 0;
    let o_idx: Vec<Option<usize>> = (0..n_streams).map(|i| (solvable[i] && !is_ref(i)).then(|| { n += 1; n - 1 })).collect();
    let mut keys: Vec<(usize, usize)> = Vec::new();
    let r_idx: Vec<Option<usize>> = (0..n_streams)
        .map(|i| {
            if !solvable[i] || is_ref(i) || ref_key == Some(rate_of[i]) {
                return None;
            }
            Some(match keys.iter().find(|k| k.0 == rate_of[i]) {
                Some(k) => k.1,
                None => {
                    keys.push((rate_of[i], n));
                    n += 1;
                    n - 1
                }
            })
        })
        .collect();
    // A stream's terms at moment t on its clock: offset_i + rate_i (t - c_i).
    let terms = |s: usize, t: f64| -> Vec<(usize, f64)> {
        o_idx[s].map(|oi| (oi, 1.0)).into_iter().chain(r_idx[s].map(|ri| (ri, t - center[s]))).collect()
    };
    let mut a = vec![vec![0.0; n]; n];
    let mut b = vec![0.0; n];
    // Each row: coefficients (index, value), target, weight.
    let mut add = |terms: &[(usize, f64)], y: f64, w: f64| {
        for &(i, ci) in terms {
            b[i] += w * ci * y;
            for &(j, cj) in terms {
                a[i][j] += w * ci * cj;
            }
        }
    };
    // Row of an observation: c·x - T_k = y with c its stream's terms and y = -t_ik
    // (offset_i + rate_i (t - c_i) - T_k = -t_ik). Per event, with u = Σ w c, v = Σ w y and
    // W = Σ w over its observations, eliminating T_k adds Σ w c cᵀ - u uᵀ / W and Σ w c y - u v / W,
    // and T_k = (u·x - v) / W.
    let mut by_event: Vec<Vec<&Obs>> = vec![Vec::new(); n_events];
    for o in obs.iter().filter(|o| solvable[o.stream]) {
        by_event[o.event].push(o);
    }
    // Per event: u (merged by index), v, W.
    let mut elim: Vec<(Vec<(usize, f64)>, f64, f64)> = Vec::with_capacity(n_events);
    for ev in &by_event {
        let (mut u, mut v, mut ww): (Vec<(usize, f64)>, f64, f64) = (Vec::new(), 0.0, 0.0);
        for o in ev {
            let w = 1.0 / (o.error_s * o.error_s);
            let c = terms(o.stream, o.t);
            add(&c, -o.t, w);
            for (i, ci) in c {
                match u.iter_mut().find(|x| x.0 == i) {
                    Some(x) => x.1 += w * ci,
                    None => u.push((i, w * ci)),
                }
            }
            v -= w * o.t;
            ww += w;
        }
        if ww > 0.0 {
            add(&u, v, -1.0 / ww);
        }
        elim.push((u, v, ww));
    }
    if absolute {
        for an in anchors {
            if solvable[an.stream] {
                add(&terms(an.stream, an.t), an.offset, 1.0 / (an.error_s * an.error_s));
            }
        }
    }
    // Each rate's prior: 0 ± rate_prior.
    let w_rate = 1.0 / rate_prior.max(1e-12).powi(2);
    for &(_, ri) in &keys {
        add(&[(ri, 1.0)], 0.0, w_rate);
    }

    let inv = invert(a);
    let x: Vec<f64> = match &inv {
        Some(m) => (0..n).map(|i| (0..n).map(|j| m[i][j] * b[j]).sum()).collect(),
        None => vec![f64::NAN; n],
    };
    let offsets = (0..n_streams)
        .map(|i| match &inv {
            Some(m) if solvable[i] => {
                let get = |k: Option<usize>, l: Option<usize>| k.zip(l).map_or(0.0, |(k, l)| m[k][l]);
                Offset::Solved(Fit {
                    offset: o_idx[i].map_or(0.0, |k| x[k]),
                    rate: r_idx[i].map_or(0.0, |k| x[k]),
                    center: center[i],
                    var_offset: get(o_idx[i], o_idx[i]).max(0.0),
                    var_rate: get(r_idx[i], r_idx[i]).max(0.0),
                    cov: get(o_idx[i], r_idx[i]),
                })
            }
            _ => Offset::Unsolved,
        })
        .collect();
    let at = |s: usize, t: f64| -> Option<f64> {
        (inv.is_some() && solvable[s]).then(|| t + terms(s, t).iter().map(|&(k, c)| c * x[k]).sum::<f64>())
    };
    // Event times, back from the eliminated rows.
    let event_t: Vec<Option<f64>> = elim
        .iter()
        .map(|(u, v, ww)| (*ww > 0.0 && inv.is_some()).then(|| (u.iter().map(|&(i, ui)| ui * x[i]).sum::<f64>() - v) / ww))
        .collect();
    let residuals = obs
        .iter()
        .map(|o| match (at(o.stream, o.t), event_t[o.event]) {
            (Some(s), Some(te)) => {
                let r = s - te;
                (r, r / o.error_s)
            }
            _ => (f64::NAN, f64::NAN),
        })
        .collect();
    let anchor_residuals = anchors
        .iter()
        .map(|an| match at(an.stream, an.t) {
            Some(s) if absolute => {
                let r = s - (an.t + an.offset);
                (r, r / an.error_s)
            }
            _ => (f64::NAN, f64::NAN),
        })
        .collect();
    Solution { offsets, absolute, reference, residuals, anchor_residuals }
}

/// Gauss-Jordan inverse with partial pivoting. None when singular.
pub(crate) fn invert(mut a: Vec<Vec<f64>>) -> Option<Vec<Vec<f64>>> {
    let n = a.len();
    let mut inv: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| (i == j) as u8 as f64).collect()).collect();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        inv.swap(c, p);
        let d = a[c][c];
        for j in 0..n {
            a[c][j] /= d;
            inv[c][j] /= d;
        }
        for r in 0..n {
            if r != c && a[r][c] != 0.0 {
                let f = a[r][c];
                for j in 0..n {
                    a[r][j] -= f * a[c][j];
                    inv[r][j] -= f * inv[c][j];
                }
            }
        }
    }
    Some(inv)
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: f64 = 1e-4;

    /// Three streams with true offsets, four shared events at true times.
    fn scene(err: f64) -> (Vec<f64>, Vec<Obs>) {
        let offsets = vec![1000.0, 1012.5, 987.25];
        let events = [1050.0, 1100.0, 1130.0, 1190.0];
        let mut obs = Vec::new();
        for (k, &tk) in events.iter().enumerate() {
            for (i, &oi) in offsets.iter().enumerate() {
                obs.push(Obs { stream: i, event: k, t: tk - oi, error_s: err });
            }
        }
        (offsets, obs)
    }

    fn solved(o: &Offset) -> (f64, f64) {
        match o {
            Offset::Solved(f) => (f.at(0.0), f.error_at(f.center)),
            Offset::Unsolved => panic!("unsolved"),
        }
    }

    #[test]
    fn relative_offsets_without_anchor() {
        let (truth, obs) = scene(0.1);
        let s = solve(3, &obs, &[], P);
        assert!(!s.absolute);
        assert_eq!(s.reference, Some(0));
        for i in 0..3 {
            let (o, _) = solved(&s.offsets[i]);
            assert!((o - (truth[i] - truth[0])).abs() < 1e-6, "stream {i}: {o}");
        }
        assert!(s.outliers(3.0).is_empty());
    }

    #[test]
    fn anchor_makes_offsets_absolute_and_carries_its_error() {
        let (truth, obs) = scene(0.1);
        let s = solve(3, &obs, &[AnchorObs { stream: 1, t: 0.0, offset: truth[1], error_s: 2.0 }], P);
        assert!(s.absolute);
        for i in 0..3 {
            let (o, e) = solved(&s.offsets[i]);
            assert!((o - truth[i]).abs() < 1e-6);
            // The anchor's 2 s error dominates every stream's absolute error.
            assert!(e > 1.9 && e < 2.1, "stream {i} error {e}");
        }
    }

    #[test]
    fn two_anchors_shrink_the_error() {
        let (truth, obs) = scene(0.1);
        let one = solve(3, &obs, &[AnchorObs { stream: 0, t: 0.0, offset: truth[0], error_s: 2.0 }], P);
        let two = solve(3, &obs, &[
            AnchorObs { stream: 0, t: 0.0, offset: truth[0], error_s: 2.0 },
            AnchorObs { stream: 2, t: 0.0, offset: truth[2], error_s: 2.0 },
        ], P);
        assert!(solved(&two.offsets[1]).1 < solved(&one.offsets[1]).1);
    }

    #[test]
    fn stream_without_shared_events_is_unsolved() {
        let (_, mut obs) = scene(0.1);
        // Stream 3 only hears event 9, which nobody else heard.
        obs.push(Obs { stream: 3, event: 9, t: 5.0, error_s: 0.1 });
        let s = solve(4, &obs, &[], P);
        assert_eq!(s.offsets[3], Offset::Unsolved);
        assert!(matches!(s.offsets[2], Offset::Solved(_)));
    }

    #[test]
    fn wrong_match_shows_as_outlier() {
        let (_, mut obs) = scene(0.1);
        // Stream 2's copy of event 1 was matched to the wrong bang, 5 s off.
        let bad = obs.iter().position(|o| o.stream == 2 && o.event == 1).unwrap();
        obs[bad].t += 5.0;
        let s = solve(3, &obs, &[], P);
        let out = s.outliers(3.0);
        assert!(out.contains(&bad), "outliers {out:?}");
    }

    #[test]
    fn noisier_observations_give_larger_errors() {
        let (_, tight) = scene(0.05);
        let (_, loose) = scene(1.0);
        let a = solve(3, &tight, &[], P);
        let b = solve(3, &loose, &[], P);
        assert!(solved(&b.offsets[2]).1 > solved(&a.offsets[2]).1);
    }

    /// Stream 1 runs 300 ppm slow against stream 0 (an iPod-like device) over an hour: shared
    /// events every 5 minutes, 5 ms each.
    fn drifting(rate: f64) -> Vec<Obs> {
        let mut obs = Vec::new();
        for k in 0..13 {
            let shared = 300.0 * k as f64;
            obs.push(Obs { stream: 0, event: k, t: shared, error_s: 0.005 });
            // shared = t1 + 20 + rate * t1
            obs.push(Obs { stream: 1, event: k, t: (shared - 20.0) / (1.0 + rate), error_s: 0.005 });
        }
        obs
    }

    #[test]
    fn drift_is_learnt_from_evidence_along_the_overlap() {
        let s = solve(2, &drifting(300e-6), &[], P);
        let Offset::Solved(f) = s.offsets[1] else { panic!() };
        assert!((f.rate - 300e-6).abs() < 2e-6, "rate {}", f.rate);
        assert!(f.var_rate.sqrt() < 5e-6, "rate error {}", f.var_rate.sqrt());
        assert!((f.at(0.0) - 20.0).abs() < 0.01, "offset at 0: {}", f.at(0.0));
        assert!((f.back(f.at(1234.5)) - 1234.5).abs() < 1e-9);
        // Fitted, nothing is left over: a 1 s end-to-end slide is not a contradiction.
        assert!(s.outliers(3.0).is_empty(), "{:?}", s.residuals);
        // With rates pinned at 0 (offsets only), the slide shows as wrong matches.
        assert!(!solve(2, &drifting(300e-6), &[], 0.0).outliers(3.0).is_empty());
    }

    #[test]
    fn recordings_of_one_device_share_its_speed() {
        // Streams 1 and 2 are two files of one phone running 300 ppm slow. Stream 1 overlaps
        // stream 0 for an hour; stream 2 shares a single event with it, an hour later.
        let mut obs = drifting(300e-6);
        let shared = 7200.0;
        obs.push(Obs { stream: 0, event: 13, t: shared, error_s: 0.005 });
        obs.push(Obs { stream: 2, event: 13, t: (shared - 500.0) / (1.0 + 300e-6), error_s: 0.005 });
        let fit = |s: &Solution| match s.offsets[2] {
            Offset::Solved(f) => f,
            Offset::Unsolved => panic!(),
        };
        let alone = fit(&solve(3, &obs, &[], P));
        let one = fit(&solve_shared(3, &obs, &[], P, &[0, 1, 1]));
        assert!(alone.rate.abs() < 1e-9, "alone it cannot learn: {}", alone.rate);
        assert!((one.rate - 300e-6).abs() < 2e-6, "shared rate {}", one.rate);
        // Half an hour from its only event, the shared speed keeps stream 2 sure.
        let t = one.center + 1800.0;
        assert!(one.error_at(t) < 0.02 && alone.error_at(t) > 0.15, "{} {}", one.error_at(t), alone.error_at(t));
        // Sharing the reference's device pins the rate at 0, offsets still solved.
        let pinned = solve_shared(3, &obs, &[], P, &[0, 0, 1]);
        let Offset::Solved(f1) = pinned.offsets[1] else { panic!() };
        assert!(f1.rate == 0.0 && f1.var_rate == 0.0);
    }

    #[test]
    fn one_link_leaves_the_rate_at_its_prior_and_errors_grow_away_from_it() {
        let obs = vec![Obs { stream: 0, event: 0, t: 100.0, error_s: 0.01 }, Obs { stream: 1, event: 0, t: 50.0, error_s: 0.01 }];
        let s = solve(2, &obs, &[], P);
        let Offset::Solved(f) = s.offsets[1] else { panic!() };
        assert!(f.rate.abs() < 1e-12 && (f.var_rate.sqrt() - P).abs() < 1e-9);
        assert!((f.at(50.0) - 100.0).abs() < 1e-9);
        let (near, hour) = (f.error_at(50.0), f.error_at(50.0 + 3600.0));
        assert!(near < 0.02, "{near}");
        // An hour away the unknown speed adds 0.36 s (100 ppm).
        assert!((hour - (near * near + 0.36f64.powi(2)).sqrt()).abs() < 1e-3, "{hour}");
    }
}
