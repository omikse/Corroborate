//! Straight-pass Doppler model and a small Levenberg–Marquardt fit.
//!
//! Model (observation-time form, propagation delay folded into `tc`):
//!   f(t) = f0 · c / (c + v²·τ / r),  τ = t − tc,  r = √(d² + v²τ²)
//! Parameters: rest frequency f0 (Hz), speed v (m/s), closest-approach slant distance d (m),
//! closest-approach time tc (s).

pub const C_SOUND: f64 = 343.0;

pub fn model(t: f64, p: &[f64; 4], c: f64) -> f64 {
    let [f0, v, d, tc] = *p;
    let tau = t - tc;
    let r = (d * d + v * v * tau * tau).sqrt().max(1e-9);
    f0 * c / (c + v * v * tau / r)
}

#[derive(Clone, Debug)]
pub struct Fit {
    pub p: [f64; 4],
    pub rms: f64,
    pub n: usize,
    /// Error of the closest-approach time `tc`, seconds (1 sigma): `tc_error`.
    pub tc_err: f64,
    /// Asymptote frequencies implied by the fit (approach, recede).
    pub f_pre: f64,
    pub f_post: f64,
}

fn median(v: &mut [f64]) -> f64 {
    let m = v.len() / 2;
    let (_, x, _) = v.select_nth_unstable_by(m, |a, b| a.total_cmp(b));
    *x
}

/// Initial guess from the data: asymptotes from the first/last seventh, `tc` where the curve
/// crosses the midpoint, `d` from the slope there (df/dt at tc ≈ −f0·v²/(c·d)).
pub fn init_guess(ts: &[f64], fs: &[f64], c: f64) -> Option<[f64; 4]> {
    let n = ts.len();
    if n < 10 {
        return None;
    }
    let q = (n / 7).max(5);
    let f_pre = median(&mut fs[..q].to_vec());
    let f_post = median(&mut fs[n - q..].to_vec());
    if f_pre <= f_post {
        return None;
    }
    let v = c * (f_pre - f_post) / (f_pre + f_post);
    let f0 = f_pre * (c - v) / c;
    let mid = 0.5 * (f_pre + f_post);

    // Split point minimising (#below mid before) + (#above mid after).
    let mut below_before = 0usize;
    let above_total = fs.iter().filter(|&&f| f > mid).count();
    let mut above_before = 0usize;
    let mut best = usize::MAX;
    let mut split = n / 2;
    for i in 0..=n {
        let cost = below_before + (above_total - above_before);
        if cost < best {
            best = cost;
            split = i;
        }
        if i < n {
            if fs[i] > mid {
                above_before += 1;
            } else {
                below_before += 1;
            }
        }
    }
    let tc = if split == 0 {
        ts[0]
    } else if split >= n {
        ts[n - 1]
    } else {
        0.5 * (ts[split - 1] + ts[split])
    };

    // Local slope around tc by least squares.
    let (mut sx, mut sy, mut sxx, mut sxy, mut m) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (&t, &f) in ts.iter().zip(fs) {
        if (t - tc).abs() <= 5.0 {
            let x = t - tc;
            sx += x;
            sy += f;
            sxx += x * x;
            sxy += x * f;
            m += 1.0;
        }
    }
    let den = m * sxx - sx * sx;
    let slope = if m >= 3.0 && den > 0.0 { (m * sxy - sx * sy) / den } else { 0.0 };
    let d = if slope < 0.0 {
        (f0 * v * v / (c * -slope)).clamp(50.0, 20_000.0)
    } else {
        1000.0
    };
    Some([f0, v, d, tc])
}

fn cost(ts: &[f64], fs: &[f64], p: &[f64; 4], c: f64) -> f64 {
    ts.iter()
        .zip(fs)
        .map(|(&t, &f)| {
            let r = model(t, p, c) - f;
            r * r
        })
        .sum()
}

fn clamp(p: &mut [f64; 4]) {
    p[0] = p[0].clamp(1.0, 5000.0);
    p[1] = p[1].clamp(1.0, 300.0);
    p[2] = p[2].clamp(5.0, 100_000.0);
}

/// Solve the 4×4 system `a x = b` by Gaussian elimination with partial pivoting.
fn solve4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> Option<[f64; 4]> {
    for col in 0..4 {
        let piv = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for row in col + 1..4 {
            let f = a[row][col] / a[col][col];
            for k in col..4 {
                a[row][k] -= f * a[col][k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0; 4];
    for row in (0..4).rev() {
        let mut s = b[row];
        for k in row + 1..4 {
            s -= a[row][k] * x[k];
        }
        x[row] = s / a[row][row];
    }
    Some(x)
}

/// The fit's Jacobian product JᵀJ at `p` (central differences, steps as in `lm`).
fn jtj_at(ts: &[f64], p: &[f64; 4], c: f64) -> [[f64; 4]; 4] {
    let h = [1e-4 * p[0].abs().max(1.0), 1e-3 * p[1], 1e-3 * p[2], 1e-2];
    let mut jtj = [[0.0; 4]; 4];
    for &t in ts {
        let mut g = [0.0; 4];
        for j in 0..4 {
            let (mut a, mut b) = (*p, *p);
            a[j] += h[j];
            b[j] -= h[j];
            g[j] = (model(t, &a, c) - model(t, &b, c)) / (2.0 * h[j]);
        }
        for i in 0..4 {
            for j in 0..4 {
                jtj[i][j] += g[i] * g[j];
            }
        }
    }
    jtj
}

/// Error (1 sigma, seconds) of the closest-approach time of a fit at `p` with residual `rms`
/// (Hz): sqrt(σ² · (JᵀJ)⁻¹[tc][tc]), σ² the residual variance with 4 parameters taken out.
/// Infinite when the fit does not pin `tc` down.
pub fn tc_error(ts: &[f64], p: &[f64; 4], c: f64, rms: f64) -> f64 {
    let n = ts.len() as f64;
    if n <= 4.0 {
        return f64::INFINITY;
    }
    let var = rms * rms * n / (n - 4.0);
    match solve4(jtj_at(ts, p, c), [0.0, 0.0, 0.0, 1.0]) {
        Some(x) if x[3].is_finite() && x[3] > 0.0 => (var * x[3]).sqrt(),
        _ => f64::INFINITY,
    }
}

/// Levenberg–Marquardt with a central-difference Jacobian.
pub fn lm(ts: &[f64], fs: &[f64], p0: [f64; 4], c: f64, iters: usize) -> [f64; 4] {
    let mut p = p0;
    clamp(&mut p);
    let mut lam = 1e-3;
    let mut cur = cost(ts, fs, &p, c);
    for _ in 0..iters {
        let h = [1e-4 * p[0].abs().max(1.0), 1e-3 * p[1], 1e-3 * p[2], 1e-2];
        let mut jtj = [[0.0; 4]; 4];
        let mut jtr = [0.0; 4];
        for (&t, &f) in ts.iter().zip(fs) {
            let r = model(t, &p, c) - f;
            let mut g = [0.0; 4];
            for j in 0..4 {
                let mut a = p;
                let mut b = p;
                a[j] += h[j];
                b[j] -= h[j];
                g[j] = (model(t, &a, c) - model(t, &b, c)) / (2.0 * h[j]);
            }
            for i in 0..4 {
                jtr[i] += g[i] * r;
                for j in 0..4 {
                    jtj[i][j] += g[i] * g[j];
                }
            }
        }
        let mut improved = false;
        for _ in 0..10 {
            let mut a = jtj;
            for i in 0..4 {
                a[i][i] += lam * jtj[i][i].max(1e-12);
            }
            let Some(dx) = solve4(a, [-jtr[0], -jtr[1], -jtr[2], -jtr[3]]) else {
                lam *= 10.0;
                continue;
            };
            let mut q = [p[0] + dx[0], p[1] + dx[1], p[2] + dx[2], p[3] + dx[3]];
            clamp(&mut q);
            let nc = cost(ts, fs, &q, c);
            if nc.is_finite() && nc < cur {
                let rel = (cur - nc) / cur.max(1e-300);
                p = q;
                cur = nc;
                lam = (lam / 3.0).max(1e-12);
                improved = rel > 1e-12;
                break;
            }
            lam *= 4.0;
        }
        if !improved {
            break;
        }
    }
    p
}

/// Full fit: initial guess, LM, then one refit without points more than 3·rms off.
pub fn fit(ts: &[f64], fs: &[f64], c: f64) -> Option<Fit> {
    let p0 = init_guess(ts, fs, c)?;
    let mut p = lm(ts, fs, p0, c, 200);
    let rms = (cost(ts, fs, &p, c) / ts.len() as f64).sqrt();
    let (mut kt, mut kf) = (Vec::with_capacity(ts.len()), Vec::with_capacity(ts.len()));
    for (&t, &f) in ts.iter().zip(fs) {
        if (model(t, &p, c) - f).abs() <= 3.0 * rms.max(0.05) {
            kt.push(t);
            kf.push(f);
        }
    }
    let (ts, fs) = if kt.len() >= 10 && kt.len() < ts.len() {
        p = lm(&kt, &kf, p, c, 200);
        (kt.as_slice(), kf.as_slice())
    } else {
        (ts, fs)
    };
    let rms = (cost(ts, fs, &p, c) / ts.len() as f64).sqrt();
    let [f0, v, _, _] = p;
    Some(Fit {
        p,
        rms,
        n: ts.len(),
        tc_err: tc_error(ts, &p, c, rms),
        f_pre: f0 * c / (c - v),
        f_post: f0 * c / (c + v),
    })
}

/// Robust fit for ridges that wander onto other tones for part of their length.
///
/// Starts from several contiguous sub-ranges of the data, fits each, then alternates between
/// selecting inliers (|residual| ≤ `tol` Hz over *all* points) and refitting on them. Keeps
/// the start with the most inliers. Returns the fit on its inliers and their indices.
pub fn robust_fit(ts: &[f64], fs: &[f64], c: f64, tol: f64) -> Option<(Fit, Vec<usize>)> {
    let n = ts.len();
    if n < 20 {
        return None;
    }
    const STARTS: [(f64, f64); 7] =
        [(0.0, 1.0), (0.0, 0.5), (0.25, 0.5), (0.5, 0.5), (0.0, 0.67), (0.33, 0.67), (0.15, 0.7)];
    let mut best: Option<(Fit, Vec<usize>)> = None;
    let (mut it, mut iff) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for (a, len) in STARTS {
        let s = (a * n as f64) as usize;
        let e = ((a + len) * n as f64).min(n as f64) as usize;
        if e - s < 20 {
            continue;
        }
        let Some(p0) = init_guess(&ts[s..e], &fs[s..e], c) else { continue };
        let mut p = lm(&ts[s..e], &fs[s..e], p0, c, 100);
        for _ in 0..4 {
            let idx: Vec<usize> = (0..n).filter(|&i| (model(ts[i], &p, c) - fs[i]).abs() <= tol).collect();
            if idx.len() < 20 {
                break;
            }
            it.clear();
            iff.clear();
            it.extend(idx.iter().map(|&i| ts[i]));
            iff.extend(idx.iter().map(|&i| fs[i]));
            p = lm(&it, &iff, p, c, 100);
        }
        let idx: Vec<usize> = (0..n).filter(|&i| (model(ts[i], &p, c) - fs[i]).abs() <= tol).collect();
        if idx.len() < 20 || best.as_ref().is_some_and(|(_, b)| b.len() >= idx.len()) {
            continue;
        }
        it.clear();
        iff.clear();
        it.extend(idx.iter().map(|&i| ts[i]));
        iff.extend(idx.iter().map(|&i| fs[i]));
        let rms = (cost(&it, &iff, &p, c) / idx.len() as f64).sqrt();
        let [f0, v, _, _] = p;
        let fit = Fit { p, rms, n: idx.len(), tc_err: tc_error(&it, &p, c, rms), f_pre: f0 * c / (c - v), f_post: f0 * c / (c + v) };
        best = Some((fit, idx));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robust_fit_ignores_a_steady_tone_segment() {
        let truth = [150.0, 80.0, 1500.0, 100.0];
        let ts: Vec<f64> = (0..800).map(|i| i as f64 * 0.25).collect();
        // Ridge follows the pass (0–200 s), except for 25 s where it sits on a steady 120 Hz tone.
        let fs: Vec<f64> = ts
            .iter()
            .map(|&t| if (110.0..135.0).contains(&t) { 120.0 } else { model(t, &truth, C_SOUND) })
            .collect();
        let (f, idx) = robust_fit(&ts, &fs, C_SOUND, 1.5).expect("fit");
        assert!((f.p[0] - 150.0).abs() < 0.5, "{:?}", f.p);
        assert!((f.p[3] - 100.0).abs() < 1.0, "{:?}", f.p);
        assert!(idx.len() >= 690, "inliers {}", idx.len());
    }

    #[test]
    fn recovers_known_pass() {
        let truth = [112.0, 62.0, 900.0, 80.0];
        let ts: Vec<f64> = (0..640).map(|i| i as f64 * 0.25).collect();
        // Deterministic small wiggle standing in for bin quantisation.
        let fs: Vec<f64> = ts
            .iter()
            .enumerate()
            .map(|(i, &t)| model(t, &truth, C_SOUND) + 0.2 * ((i * 7919 % 13) as f64 / 6.0 - 1.0))
            .collect();
        let f = fit(&ts, &fs, C_SOUND).expect("fit");
        assert!((f.p[0] - 112.0).abs() < 0.3, "{:?}", f.p);
        assert!((f.p[1] - 62.0).abs() < 2.0, "{:?}", f.p);
        assert!((f.p[2] - 900.0).abs() < 60.0, "{:?}", f.p);
        assert!((f.p[3] - 80.0).abs() < 0.5, "{:?}", f.p);
        assert!(f.rms < 0.25, "rms {}", f.rms);
    }

    /// The closest-approach error matches the scatter of `tc` over many noisy passes.
    #[test]
    fn tc_error_matches_scatter() {
        let truth = [112.0, 62.0, 900.0, 80.0];
        let ts: Vec<f64> = (0..640).map(|i| i as f64 * 0.25).collect();
        let mut seed = 12345u64;
        let mut gauss = || {
            // Sum of 12 uniforms minus 6: close to a standard normal.
            let mut s = 0.0;
            for _ in 0..12 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                s += (seed >> 11) as f64 / (1u64 << 53) as f64;
            }
            s - 6.0
        };
        let (mut tcs, mut errs) = (Vec::new(), Vec::new());
        for _ in 0..40 {
            let fs: Vec<f64> = ts.iter().map(|&t| model(t, &truth, C_SOUND) + 1.0 * gauss()).collect();
            let f = fit(&ts, &fs, C_SOUND).expect("fit");
            tcs.push(f.p[3]);
            errs.push(f.tc_err);
        }
        let m = tcs.iter().sum::<f64>() / tcs.len() as f64;
        let sd = (tcs.iter().map(|t| (t - m).powi(2)).sum::<f64>() / (tcs.len() - 1) as f64).sqrt();
        let e = errs.iter().sum::<f64>() / errs.len() as f64;
        assert!(e > 0.0 && e.is_finite(), "error {e}");
        assert!(sd / e > 0.6 && sd / e < 1.6, "scatter {sd:.4} s, error {e:.4} s");
    }

    #[test]
    fn rejects_rising_tone() {
        let ts: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let fs: Vec<f64> = ts.iter().map(|t| 100.0 + 0.1 * t).collect();
        assert!(fit(&ts, &fs, C_SOUND).is_none());
    }
}
