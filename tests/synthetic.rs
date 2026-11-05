use corroborate::synth::{render, Pass};
use corroborate::{Config, Detector, Event};

fn run(x: &[f32]) -> Vec<Event> {
    let mut d = Detector::new(Config::default());
    let mut ev = Vec::new();
    for c in x.chunks(1000) {
        ev.extend(d.push(c));
    }
    ev.extend(d.flush());
    ev
}

#[test]
fn finds_a_clear_pass() {
    let p = Pass { f0: 105.0, v: 60.0, d: 900.0, tc: 200.0, amps: vec![0.12, 0.08, 0.04] };
    let ev = run(&render(4000.0, 360.0, &[p], None, 1.0, 1));
    assert_eq!(ev.len(), 1, "{ev:#?}");
    let e = &ev[0];
    assert!((e.t_cpa - 200.0).abs() < 3.0, "t_cpa {}", e.t_cpa);
    // The ridge may lock onto a harmonic; f0 is then a multiple.
    let h = (e.f0 / 105.0).round();
    assert!((e.f0 - h * 105.0).abs() < 1.5 * h, "f0 {}", e.f0);
    assert!((e.speed - 60.0).abs() < 8.0, "speed {}", e.speed);
}

#[test]
fn finds_two_passes_and_ignores_hum() {
    let ps = [
        Pass { f0: 105.0, v: 60.0, d: 900.0, tc: 200.0, amps: vec![0.12, 0.08, 0.04] },
        Pass { f0: 88.0, v: 75.0, d: 1600.0, tc: 430.0, amps: vec![0.08, 0.06] },
    ];
    let ev = run(&render(4000.0, 600.0, &ps, Some((60.0, 0.3)), 1.0, 7));
    let mut tcs: Vec<f64> = ev.iter().map(|e| e.t_cpa).collect();
    tcs.sort_by(f64::total_cmp);
    assert_eq!(tcs.len(), 2, "{ev:#?}");
    assert!((tcs[0] - 200.0).abs() < 3.0 && (tcs[1] - 430.0).abs() < 4.0, "{tcs:?}");
}

#[test]
fn noise_only_gives_nothing() {
    for seed in 1..=3 {
        let ev = run(&render(4000.0, 600.0, &[], Some((60.0, 0.3)), 1.0, seed));
        assert!(ev.is_empty(), "seed {seed}: {ev:#?}");
    }
}
