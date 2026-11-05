//! Locate a sound from its arrival times at several recorders: TDOA (Time Difference of Arrival).
//!
//! Inputs: for each recorder, its position (a `Location` with its own radius) and the arrival
//! time of the sound on the shared clock (after `sync`), with an error. Unknowns: the source
//! position and the time the sound was made. Model: t_i = T0 + distance_i / c.
//!
//! The error of each arrival includes the recorder's own position error (radius / c), so a
//! vague recorder position widens the result instead of being silently trusted.
//!
//! Method: a coarse grid search for a start point (so the fit does not lock onto a wrong
//! minimum), then weighted Gauss-Newton. The result is a `Location` whose radius is two
//! sigmas of the position covariance along its worst direction (about 86 % for a 2-D normal).
//! Three recorders are the minimum and leave nothing to check the fit against; four or more
//! give residuals that show a bad match.

use crate::core::{Location, Method};
use crate::sync::invert;

/// Speed of sound in air at about 20 °C, m/s.
pub const C_SOUND: f64 = 343.0;
const EARTH_R_M: f64 = 6_371_000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Arrival {
    pub at: Location,
    /// Arrival time on the shared clock, seconds.
    pub t: f64,
    pub error_s: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    pub location: Location,
    /// When the sound was made, shared clock, and its error.
    pub t0: f64,
    pub t0_error_s: f64,
    /// Per arrival: residual in seconds and in sigmas.
    pub residuals: Vec<(f64, f64)>,
    /// True with exactly three recorders: the fit cannot be checked.
    pub unchecked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocateError {
    TooFew(usize),
    Degenerate,
}

/// Local flat frame (metres east, north) around a reference point. Good to a few tens of km.
struct Frame {
    lat0: f64,
    lon0: f64,
    kx: f64,
    ky: f64,
}

impl Frame {
    fn new(lat0: f64, lon0: f64) -> Self {
        let ky = EARTH_R_M * std::f64::consts::PI / 180.0;
        Self { lat0, lon0, kx: ky * lat0.to_radians().cos(), ky }
    }
    fn xy(&self, l: &Location) -> (f64, f64) {
        ((l.lon - self.lon0) * self.kx, (l.lat - self.lat0) * self.ky)
    }
    fn latlon(&self, x: f64, y: f64) -> (f64, f64) {
        (self.lat0 + y / self.ky, self.lon0 + x / self.kx)
    }
}

pub fn locate(arr: &[Arrival], c: f64) -> Result<Fix, LocateError> {
    if arr.len() < 3 {
        return Err(LocateError::TooFew(arr.len()));
    }
    let n = arr.len() as f64;
    let frame = Frame::new(
        arr.iter().map(|a| a.at.lat).sum::<f64>() / n,
        arr.iter().map(|a| a.at.lon).sum::<f64>() / n,
    );
    let pts: Vec<(f64, f64)> = arr.iter().map(|a| frame.xy(&a.at)).collect();
    // Weights: arrival error plus the recorder's position error turned into time.
    let w: Vec<f64> = arr
        .iter()
        .map(|a| {
            let s2 = a.error_s.powi(2) + (a.at.radius_m / c).powi(2);
            1.0 / s2.max(1e-12)
        })
        .collect();
    let wsum: f64 = w.iter().sum();

    // Best T0 for a given point, and the weighted misfit.
    let misfit = |x: f64, y: f64| -> (f64, f64) {
        let d: Vec<f64> = pts.iter().map(|&(px, py)| ((x - px).powi(2) + (y - py).powi(2)).sqrt()).collect();
        let t0 = arr.iter().zip(&d).zip(&w).map(|((a, di), wi)| wi * (a.t - di / c)).sum::<f64>() / wsum;
        let m = arr.iter().zip(&d).zip(&w).map(|((a, di), wi)| wi * (a.t - t0 - di / c).powi(2)).sum();
        (t0, m)
    };

    // Grid search over the recorders' area plus a margin as wide again.
    let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &(px, py) in &pts {
        x0 = x0.min(px);
        x1 = x1.max(px);
        y0 = y0.min(py);
        y1 = y1.max(py);
    }
    let span = (x1 - x0).max(y1 - y0).max(100.0);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let steps = 120;
    let half = span * 1.5;
    let (mut bx, mut by, mut best) = (cx, cy, f64::MAX);
    for i in 0..=steps {
        for j in 0..=steps {
            let x = cx - half + 2.0 * half * i as f64 / steps as f64;
            let y = cy - half + 2.0 * half * j as f64 / steps as f64;
            let (_, m) = misfit(x, y);
            if m < best {
                (bx, by, best) = (x, y, m);
            }
        }
    }

    // Gauss-Newton on (x, y, T0).
    let (mut x, mut y, mut t0) = (bx, by, misfit(bx, by).0);
    let mut cov = None;
    for _ in 0..50 {
        let mut a = vec![vec![0.0; 3]; 3];
        let mut b = vec![0.0; 3];
        for ((&(px, py), ar), &wi) in pts.iter().zip(arr).zip(&w) {
            let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt().max(1e-6);
            let r = ar.t - (t0 + d / c);
            let j = [(x - px) / (d * c), (y - py) / (d * c), 1.0];
            for p in 0..3 {
                b[p] += wi * j[p] * r;
                for q in 0..3 {
                    a[p][q] += wi * j[p] * j[q];
                }
            }
        }
        let inv = invert(a).ok_or(LocateError::Degenerate)?;
        let dx: Vec<f64> = (0..3).map(|p| (0..3).map(|q| inv[p][q] * b[q]).sum()).collect();
        x += dx[0];
        y += dx[1];
        t0 += dx[2];
        cov = Some(inv);
        if dx[0].abs() + dx[1].abs() < 1e-4 && dx[2].abs() < 1e-9 {
            break;
        }
    }
    let cov = cov.ok_or(LocateError::Degenerate)?;
    // Worst-direction sigma of the 2x2 position covariance.
    let (sxx, syy, sxy) = (cov[0][0], cov[1][1], cov[0][1]);
    let tr = sxx + syy;
    let det = sxx * syy - sxy * sxy;
    let lmax = tr / 2.0 + ((tr * tr / 4.0 - det).max(0.0)).sqrt();
    let radius = 2.0 * lmax.max(0.0).sqrt();
    if !radius.is_finite() {
        return Err(LocateError::Degenerate);
    }
    let (lat, lon) = frame.latlon(x, y);
    let residuals = pts
        .iter()
        .zip(arr)
        .zip(&w)
        .map(|((&(px, py), a), wi)| {
            let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
            let r = a.t - (t0 + d / c);
            (r, r * wi.sqrt())
        })
        .collect();
    Ok(Fix {
        location: Location::new(lat, lon, radius, Method::SharedSound).ok_or(LocateError::Degenerate)?,
        t0,
        t0_error_s: cov[2][2].max(0.0).sqrt(),
        residuals,
        unchecked: arr.len() == 3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAT0: f64 = 48.8566;
    const LON0: f64 = 2.3522;

    /// Position `x` m east and `y` m north of the test origin, with a radius.
    fn at(x: f64, y: f64, r: f64) -> Location {
        let f = Frame::new(LAT0, LON0);
        let (lat, lon) = f.latlon(x, y);
        Location::new(lat, lon, r, Method::Gps).unwrap()
    }

    fn arrivals(src: (f64, f64), recs: &[(f64, f64)], err: f64, r: f64) -> Vec<Arrival> {
        recs.iter()
            .map(|&(x, y)| {
                let d = ((x - src.0).powi(2) + (y - src.1).powi(2)).sqrt();
                Arrival { at: at(x, y, r), t: 1000.0 + d / C_SOUND, error_s: err }
            })
            .collect()
    }

    const SQUARE: [(f64, f64); 4] = [(0.0, 0.0), (1000.0, 0.0), (1000.0, 1000.0), (0.0, 1000.0)];

    #[test]
    fn finds_the_source_inside_the_recorders() {
        let fix = locate(&arrivals((300.0, 600.0), &SQUARE, 0.005, 5.0), C_SOUND).unwrap();
        let d = fix.location.distance_m(&at(300.0, 600.0, 0.0));
        assert!(d < 1.0, "off by {d} m");
        assert!((fix.t0 - 1000.0).abs() < 1e-3);
        assert!(!fix.unchecked);
        assert!(fix.residuals.iter().all(|(_, z)| z.abs() < 1e-3));
    }

    #[test]
    fn vague_recorder_positions_widen_the_radius() {
        let sharp = locate(&arrivals((300.0, 600.0), &SQUARE, 0.005, 5.0), C_SOUND).unwrap();
        let vague = locate(&arrivals((300.0, 600.0), &SQUARE, 0.005, 200.0), C_SOUND).unwrap();
        assert!(vague.location.radius_m > 5.0 * sharp.location.radius_m);
    }

    #[test]
    fn source_far_outside_is_less_certain_than_inside() {
        let inside = locate(&arrivals((500.0, 500.0), &SQUARE, 0.01, 5.0), C_SOUND).unwrap();
        let outside = locate(&arrivals((4000.0, 3000.0), &SQUARE, 0.01, 5.0), C_SOUND).unwrap();
        assert!(outside.location.radius_m > inside.location.radius_m);
    }

    #[test]
    fn three_recorders_work_but_are_unchecked() {
        let fix = locate(&arrivals((400.0, 300.0), &SQUARE[..3], 0.005, 5.0), C_SOUND).unwrap();
        assert!(fix.unchecked);
        assert!(fix.location.distance_m(&at(400.0, 300.0, 0.0)) < 1.0);
    }

    #[test]
    fn too_few_recorders_is_an_error() {
        assert_eq!(locate(&arrivals((0.0, 0.0), &SQUARE[..2], 0.01, 5.0), C_SOUND), Err(LocateError::TooFew(2)));
    }

    #[test]
    fn a_bad_arrival_shows_in_the_residuals() {
        let five: Vec<(f64, f64)> = SQUARE.iter().copied().chain([(500.0, -400.0)]).collect();
        let mut a = arrivals((300.0, 600.0), &five, 0.005, 5.0);
        a[2].t += 0.5; // a wrong match, half a second late
        let fix = locate(&a, C_SOUND).unwrap();
        let worst = fix.residuals.iter().map(|(_, z)| z.abs()).fold(0.0, f64::max);
        assert!(worst > 3.0, "worst residual {worst} sigma");
    }
}
