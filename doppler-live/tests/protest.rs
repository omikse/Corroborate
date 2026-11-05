//! End to end: four phone streams at a protest, clocks offset by unknown amounts, a few
//! shared events to sync them, then a bang located from its arrival times.
//!
//! Shows the key limit: sync events must arrive at the same moment everywhere (a flash seen
//! on video) or the travel time of sound biases every offset by up to distance / 343 m/s,
//! which is far too coarse for locating anything. The result's radius says so honestly.

use doppler_live::core::{Location, Method};
use doppler_live::locate::{locate, Arrival, C_SOUND};
use doppler_live::sync::{solve, Obs, Offset};

const LAT0: f64 = 48.8566;
const LON0: f64 = 2.3522;
const KY: f64 = 6_371_000.0 * std::f64::consts::PI / 180.0;

fn place(x: f64, y: f64, r: f64) -> Location {
    let kx = KY * LAT0.to_radians().cos();
    Location::new(LAT0 + y / KY, LON0 + x / kx, r, Method::Gps).unwrap()
}

/// Recorders (metres east, north), their unknown clock offsets, and the bang.
const RECS: [(f64, f64); 4] = [(0.0, 0.0), (600.0, 50.0), (550.0, 700.0), (-50.0, 650.0)];
const OFFSETS: [f64; 4] = [0.0, 37.25, -12.5, 80.0];
const BANG: (f64, f64) = (220.0, 410.0);
const T_BANG: f64 = 500.0;

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Sync events at true times; `src` is where each is made (None = light, instant everywhere).
fn sync_obs(events: &[(f64, Option<(f64, f64)>)], err: f64) -> Vec<Obs> {
    let mut obs = Vec::new();
    for (k, &(t, src)) in events.iter().enumerate() {
        for (i, &r) in RECS.iter().enumerate() {
            let delay = src.map_or(0.0, |s| dist(s, r) / C_SOUND);
            obs.push(Obs { stream: i, event: k, t: t + delay - OFFSETS[i], error_s: err });
        }
    }
    obs
}

/// Sync, then locate the bang from its arrival on each stream's own clock.
fn run(obs: &[Obs], sync_err_floor: f64) -> doppler_live::locate::Fix {
    let sol = solve(4, obs, &[]);
    let arrivals: Vec<Arrival> = RECS
        .iter()
        .enumerate()
        .map(|(i, &r)| {
            let t_local = T_BANG + dist(BANG, r) / C_SOUND - OFFSETS[i];
            let Offset::Solved { offset, error_s } = sol.offsets[i] else { panic!("stream {i} unsolved") };
            let t_err = (0.002f64.powi(2) + error_s.powi(2) + sync_err_floor.powi(2)).sqrt();
            Arrival { at: place(r.0, r.1, 5.0), t: offset + t_local, error_s: t_err }
        })
        .collect();
    locate(&arrivals, C_SOUND).unwrap()
}

#[test]
fn flashes_sync_well_enough_to_locate_the_bang() {
    // Three flashes seen on all four videos, frame-accurate (about 1/30 s).
    let obs = sync_obs(&[(100.0, None), (250.0, None), (420.0, None)], 1.0 / 30.0);
    let fix = run(&obs, 0.0);
    let truth = place(BANG.0, BANG.1, 0.0);
    let off = fix.location.distance_m(&truth);
    eprintln!("flash sync: off by {off:.2} m, radius {:.1} m", fix.location.radius_m);
    assert!(off < 1.0, "off by {off} m");
    // Frame-level sync gives a radius of metres to tens of metres, and the truth lies inside.
    assert!(fix.location.radius_m < 60.0, "radius {}", fix.location.radius_m);
    assert!(off <= fix.location.radius_m);
}

#[test]
fn shared_sounds_bias_sync_and_the_radius_shows_it() {
    // Three chants from places nobody knows, treated as if instant: offsets are biased by
    // the travel time. Honest handling puts that spread into the error (here 2 s for ~700 m).
    let srcs = [(900.0, 900.0), (-300.0, 200.0), (300.0, -500.0)];
    let obs = sync_obs(&[(100.0, Some(srcs[0])), (250.0, Some(srcs[1])), (420.0, Some(srcs[2]))], 2.0);
    let fix = run(&obs, 0.0);
    eprintln!("sound sync: radius {:.0} m", fix.location.radius_m);
    // The result is no longer useful for a street-level location, and the radius says so.
    assert!(fix.location.radius_m > 300.0, "radius {}", fix.location.radius_m);
}
