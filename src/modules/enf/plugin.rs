//! enf-plugin: the ENF (Electrical Network Frequency) module as a plugin (for planes-view and Corroborate).
//!
//! A separate program: reads raw f32 little-endian mono 16 kHz audio on stdin and writes JSON
//! lines on stdout (protocol in `corroborate::plugin`). Import it in planes-view with
//! Plugins ▸ Import plugin….
//!
//! It reports:
//! - the measured hum as four tracks, at harmonics 1–4 of the reading (`h1`…`h4`);
//! - each run of clear hum (≥ 30 s) as a finding;
//! - the grid, the latest reading and the band to view, as they change.

use corroborate::modules::enf::{Enf, EnfPoint};
use corroborate::plugin::{self, PluginFinding};
use corroborate::{ingest, Finding, Module};
use std::io::Write;

const COLOR: [u8; 3] = [120, 170, 255];

/// Which grids run at this nominal frequency.
fn grid_region(nominal: f64) -> &'static str {
    if nominal < 55.0 {
        "50 Hz grid: Europe, Africa, most of Asia, Australia, much of South America"
    } else {
        "60 Hz grid: North America, parts of South America, Korea, Taiwan, Philippines, west Japan"
    }
}

struct Out<W: Write> {
    w: W,
    /// The band last sent as the view: (harmonic, nominal Hz).
    view: Option<(u32, u32)>,
}

impl<W: Write> Out<W> {
    fn line(&mut self, s: String) {
        let _ = writeln!(self.w, "{s}");
    }

    fn report(&mut self, enf: &mut Enf, found: Vec<Finding>) {
        let pts: Vec<EnfPoint> = enf.take_points();
        for p in &pts {
            for k in 1..=4u32 {
                self.line(plugin::track(&format!("h{k}"), p.t, p.hz * k as f64, p.ok));
            }
        }
        if !pts.is_empty() {
            let latest = pts.iter().rev().find(|q| q.ok);
            let text = match (enf.nominal(), latest) {
                (Some(n), Some(q)) => format!(
                    "{}\nnow {:.4} Hz ({:+.1} mHz)  snr {:.0} dB  h{}",
                    grid_region(n),
                    q.hz,
                    (q.hz - n) * 1000.0,
                    q.snr_db,
                    q.harmonic
                ),
                (Some(n), None) => format!("{}\nno clear reading in the last second", grid_region(n)),
                _ => "no clear grid hum (yet)".to_string(),
            };
            self.line(plugin::status(&text));
            // View: 1 Hz around the harmonic in use, sent again when it changes.
            if let (Some(n), Some(q)) = (enf.nominal(), latest) {
                let key = (q.harmonic, n.round() as u32);
                if self.view != Some(key) {
                    self.view = Some(key);
                    let c = n * q.harmonic as f64;
                    self.line(plugin::view(c - 0.5, c + 0.5));
                }
            }
        }
        for f in found {
            if let Finding::Enf(g) = f {
                let h = g.harmonic as f64;
                self.line(plugin::finding(&PluginFinding {
                    id: g.id,
                    t_start: g.t_start,
                    t_end: g.t_end,
                    f_lo: g.min_hz * h,
                    f_hi: g.max_hz * h,
                    label: format!(
                        "{:.4} Hz  {:+.0}..{:+.0} mHz",
                        g.mean_hz,
                        (g.min_hz - g.nominal) * 1000.0,
                        (g.max_hz - g.nominal) * 1000.0
                    ),
                    detail: format!(
                        "harmonic {} · mean SNR {:.1} dB · {} readings · {:.0} Hz grid",
                        g.harmonic, g.mean_snr_db, g.n_points, g.nominal
                    ),
                }));
            }
        }
    }
}

fn main() {
    let mut enf = Enf::new();
    let stdout = std::io::stdout();
    let mut out = Out { w: std::io::BufWriter::new(stdout.lock()), view: None };
    out.line(plugin::hello("ENF", "mains hum (Electrical Network Frequency)", COLOR));
    out.line(plugin::status(&format!("waiting for 16 s of audio…\n{}", enf.describe())));
    if out.w.flush().is_err() {
        return;
    }
    // 0.05 s chunks, the same size the viewer sends.
    for x in ingest::spawn_stdin(800) {
        let found = enf.push(&x);
        out.report(&mut enf, found);
        if out.w.flush().is_err() {
            // The viewer has gone.
            return;
        }
    }
    let found = enf.flush();
    out.report(&mut enf, found);
    let _ = out.w.flush();
}
