//! Sync solver: finds each stream's clock offset from events heard (or seen) in several
//! streams at once, plus any anchors (streams whose UTC offset is already known).
//!
//! Model, for stream i and shared event k:
//!     offset_i + t_ik = T_k   (± error_ik)
//! where t_ik is the event's time on stream i's own clock, offset_i turns that clock into
//! the shared one, and T_k is when the event happened. An anchor adds offset_i = a_i (± error).
//! The error of an observation should include the travel-time spread of the sound (up to
//! distance / 343 m/s), which is what the location step later uses.
//!
//! Weighted least squares over all offsets and event times. Without any anchor, the first
//! connected stream is the reference (offset 0) and the result is relative to it. Streams
//! with no path of shared events to an anchor or the reference are reported as unsolved,
//! never guessed. Every offset comes with its error, and every observation with its
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
    /// UTC of the stream's clock zero, Unix seconds.
    pub offset: f64,
    pub error_s: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Offset {
    /// Offset and one-sigma error, seconds. Absolute (UTC) when anchors were given,
    /// otherwise relative to the reference stream.
    Solved { offset: f64, error_s: f64 },
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

/// Solve offsets for `n_streams` streams.
pub fn solve(n_streams: usize, obs: &[Obs], anchors: &[AnchorObs]) -> Solution {
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

    // Unknowns: offsets of solvable streams, then times of events touching them.
    let s_idx: Vec<Option<usize>> = {
        let mut k = 0;
        solvable.iter().map(|&s| s.then(|| { k += 1; k - 1 })).collect()
    };
    let n_s = s_idx.iter().flatten().count();
    let mut e_idx: Vec<Option<usize>> = vec![None; n_events];
    let mut k = n_s;
    for o in obs.iter().filter(|o| solvable[o.stream]) {
        if e_idx[o.event].is_none() {
            e_idx[o.event] = Some(k);
            k += 1;
        }
    }
    let n = k;
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
    for o in obs.iter().filter(|o| solvable[o.stream]) {
        let (si, ei) = (s_idx[o.stream].unwrap(), e_idx[o.event].unwrap());
        // offset_i - T_k = -t_ik
        add(&[(si, 1.0), (ei, -1.0)], -o.t, 1.0 / (o.error_s * o.error_s));
    }
    if absolute {
        for an in anchors {
            if let Some(si) = s_idx[an.stream] {
                add(&[(si, 1.0)], an.offset, 1.0 / (an.error_s * an.error_s));
            }
        }
    } else if let Some(si) = reference.and_then(|r| s_idx[r]) {
        // Gauge: the reference stream's offset is zero by definition. A stiff constraint,
        // so its own error comes out near zero and the others are relative to it.
        add(&[(si, 1.0)], 0.0, 1e12);
    }

    let inv = invert(a);
    let x: Vec<f64> = match &inv {
        Some(m) => (0..n).map(|i| (0..n).map(|j| m[i][j] * b[j]).sum()).collect(),
        None => vec![f64::NAN; n],
    };
    let offsets = (0..n_streams)
        .map(|i| match (s_idx[i], &inv) {
            (Some(si), Some(m)) => Offset::Solved { offset: x[si], error_s: m[si][si].max(0.0).sqrt() },
            _ => Offset::Unsolved,
        })
        .collect();
    let residuals = obs
        .iter()
        .map(|o| match (s_idx[o.stream], e_idx[o.event]) {
            (Some(si), Some(ei)) if inv.is_some() => {
                let r = x[si] + o.t - x[ei];
                (r, r / o.error_s)
            }
            _ => (f64::NAN, f64::NAN),
        })
        .collect();
    Solution { offsets, absolute, reference, residuals }
}

/// Gauss-Jordan inverse with partial pivoting. None when singular.
fn invert(mut a: Vec<Vec<f64>>) -> Option<Vec<Vec<f64>>> {
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
            Offset::Solved { offset, error_s } => (*offset, *error_s),
            Offset::Unsolved => panic!("unsolved"),
        }
    }

    #[test]
    fn relative_offsets_without_anchor() {
        let (truth, obs) = scene(0.1);
        let s = solve(3, &obs, &[]);
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
        let s = solve(3, &obs, &[AnchorObs { stream: 1, offset: truth[1], error_s: 2.0 }]);
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
        let one = solve(3, &obs, &[AnchorObs { stream: 0, offset: truth[0], error_s: 2.0 }]);
        let two = solve(3, &obs, &[
            AnchorObs { stream: 0, offset: truth[0], error_s: 2.0 },
            AnchorObs { stream: 2, offset: truth[2], error_s: 2.0 },
        ]);
        assert!(solved(&two.offsets[1]).1 < solved(&one.offsets[1]).1);
    }

    #[test]
    fn stream_without_shared_events_is_unsolved() {
        let (_, mut obs) = scene(0.1);
        // Stream 3 only hears event 9, which nobody else heard.
        obs.push(Obs { stream: 3, event: 9, t: 5.0, error_s: 0.1 });
        let s = solve(4, &obs, &[]);
        assert_eq!(s.offsets[3], Offset::Unsolved);
        assert!(matches!(s.offsets[2], Offset::Solved { .. }));
    }

    #[test]
    fn wrong_match_shows_as_outlier() {
        let (_, mut obs) = scene(0.1);
        // Stream 2's copy of event 1 was matched to the wrong bang, 5 s off.
        let bad = obs.iter().position(|o| o.stream == 2 && o.event == 1).unwrap();
        obs[bad].t += 5.0;
        let s = solve(3, &obs, &[]);
        let out = s.outliers(3.0);
        assert!(out.contains(&bad), "outliers {out:?}");
    }

    #[test]
    fn noisier_observations_give_larger_errors() {
        let (_, tight) = scene(0.05);
        let (_, loose) = scene(1.0);
        let a = solve(3, &tight, &[]);
        let b = solve(3, &loose, &[]);
        assert!(solved(&b.offsets[2]).1 > solved(&a.offsets[2]).1);
    }
}
