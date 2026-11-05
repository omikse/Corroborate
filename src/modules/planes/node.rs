//! The Planes node in the workbench: what it keeps while running, its window (spectrogram with
//! passes and plugin tracks), its timeline marks and its part of the result file.
//! Compiled into the `corroborate` program (see `src/bin/corroborate/main.rs`).

use super::*;
use corroborate::modules::planes::detect::{Config, Event};
use corroborate::modules::planes::Planes;

/// Spectrogram value range kept for display (whitened z-scores).
const Z_LO: f32 = -1.0;
const Z_HI: f32 = 6.0;

/// What a Planes node has produced so far.
#[derive(Default)]
pub struct PlanesState {
    pub freqs: Vec<f64>,
    pub hist: Vec<u8>,
    pub hist_t: Vec<f64>,
    /// Frames ever added: changes whenever the spectrogram changes.
    pub frames: u64,
    pub passes: Vec<Event>,
    pub pending: Vec<Event>,
    pub ridges: Vec<Vec<(f64, f64)>>,
}

fn quantise(z: f32) -> u8 {
    (((z - Z_LO) / (Z_HI - Z_LO)).clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A detector for one Planes node at the start of a run.
pub fn start(st: &Shared) -> Planes {
    let p = Planes::new(Config::default());
    st.lock().unwrap().planes.freqs = p.detector().bin_freqs();
    p
}

/// Feed one block of the shared 16 kHz audio.
pub fn push(p: &mut Planes, st: &Shared, x: &[f32], anchor: &Option<Anchor>, audio_s: f64) {
    let mut frames: Vec<(f64, Vec<u8>)> = Vec::new();
    let evs = p.push_with(x, |t, z| frames.push((t, z.iter().map(|&v| quantise(v)).collect())));
    let mut s = st.lock().unwrap();
    for (t, q) in frames {
        s.planes.hist.extend_from_slice(&q);
        s.planes.hist_t.push(t);
        s.planes.frames += 1;
    }
    s.planes.ridges = p.detector().ridges().to_vec();
    s.planes.pending = p.detector().pending().cloned().collect();
    s.planes.passes.extend(evs);
    s.anchor = anchor.clone();
    s.audio_s = audio_s;
    s.status = format!("{} passes", s.planes.passes.len());
}

/// End of input: passes still held are reported.
pub fn flush(p: &mut Planes, st: &Shared) {
    let evs = p.flush_events();
    let mut s = st.lock().unwrap();
    s.planes.passes.extend(evs);
    s.planes.pending.clear();
    s.status = format!("{} passes", s.planes.passes.len());
}

pub fn marks(s: &NodeState) -> Vec<Mark> {
    let (to, err) = clock(&s.anchor);
    s.planes
        .passes
        .iter()
        .map(|e| Mark {
            t0: to(e.t_start),
            t1: to(e.t_end),
            err,
            label: format!("{:.0} m/s", e.speed),
            detail: format!(
                "pass {}: closest approach {}, {:.0} m/s, {:.0} m, f0 {:.1} Hz, grade {}",
                e.id,
                s.anchor.as_ref().map_or(format!("{:.1} s", e.t_cpa), |a| format!("{} UTC", fmt_clock(a.utc(e.t_cpa)))),
                e.speed,
                e.dist,
                e.f0,
                e.grade()
            ),
            col: grade_color(e.grade()),
        })
        .collect()
}

pub fn json(s: &NodeState) -> String {
    let p: Vec<String> = s.planes.passes.iter().map(|e| e.to_json(s.anchor.as_ref())).collect();
    format!(",\"passes\":[{}]", p.join(","))
}

impl App {
    pub(crate) fn planes_window(&mut self, ui: &mut egui::Ui, i: NodeId) {
        let st = self.nodes[i].state.clone();
        // Plugin tracks on the same source, drawn over the spectrogram (adsb-check's predicted curves).
        let key = st.lock().unwrap().source_key.clone();
        let overlays: Vec<(Color32, Vec<(String, Vec<(f64, f64, bool)>)>)> = self
            .nodes
            .iter()
            .filter(|n| !n.hidden && matches!(n.kind, Kind::Plugin { .. }))
            .filter_map(|n| {
                let s = n.state.lock().unwrap();
                (s.source_key == key && !key.is_empty() && !s.plugin.tracks.is_empty()).then(|| {
                    let c = s.plugin.color.map_or(Color32::WHITE, |[r, g, b]| Color32::from_rgb(r, g, b));
                    (c, s.plugin.tracks.clone())
                })
            })
            .collect();
        let s = st.lock().unwrap();
        let pl = &s.planes;
        ui.horizontal(|ui| {
            ui.label(format!("{} passes · {} candidates · {:.0} s analysed", pl.passes.len(), pl.pending.len(), s.audio_s));
            ui.separator();
            ui.label("show");
            ui.add(egui::DragValue::new(&mut self.spec_minutes).range(0.0..=600.0).suffix(" min").speed(0.5));
            ui.colored_label(DIM, "(0 = all)");
        });
        let nb = pl.freqs.len();
        let nf = pl.hist_t.len();
        if nb == 0 || nf == 0 {
            ui.colored_label(DIM, "no audio yet: press Run");
            return;
        }
        let t_end = *pl.hist_t.last().unwrap();
        let t_start = if self.spec_minutes > 0.0 { (t_end - self.spec_minutes * 60.0).max(pl.hist_t[0]) } else { pl.hist_t[0] };
        let f_first = pl.hist_t.partition_point(|&t| t < t_start);
        let cols_n = nf - f_first;
        let w = cols_n.clamp(1, 2048);
        // Rebuild the texture when new frames arrived or the span changed.
        let stamp = (pl.frames, t_start);
        let fresh = self.textures.get(&i).is_none_or(|(f, ts, _)| (*f, *ts) != stamp);
        if fresh {
            let mut px = vec![0u8; w * nb * 4];
            for c in 0..w {
                let a = f_first + c * cols_n / w;
                let b = (f_first + (c + 1) * cols_n / w).max(a + 1).min(nf);
                for bin in 0..nb {
                    let mut m = 0u8;
                    for f in a..b {
                        m = m.max(pl.hist[f * nb + bin]);
                    }
                    let row = nb - 1 - bin;
                    px[(row * w + c) * 4..(row * w + c) * 4 + 4].copy_from_slice(&self.lut[m as usize]);
                }
            }
            let img = ColorImage::from_rgba_unmultiplied([w, nb], &px);
            match self.textures.get_mut(&i) {
                Some((f, ts, tex)) => {
                    tex.set(img, TextureOptions::LINEAR);
                    *f = stamp.0;
                    *ts = stamp.1;
                }
                None => {
                    let tex = ui.ctx().load_texture(format!("spec{i}"), img, TextureOptions::LINEAR);
                    self.textures.insert(i, (stamp.0, stamp.1, tex));
                }
            }
        }
        let tex = self.textures[&i].2.id();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 360.0), Sense::hover());
        let plot = Rect::from_min_max(rect.min + Vec2::new(44.0, 0.0), rect.max - Vec2::new(0.0, 18.0));
        let p = ui.painter_at(rect);
        p.image(tex, plot, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let (f_lo, f_hi) = (pl.freqs[0], pl.freqs[nb - 1]);
        let x_of = |t: f64| plot.left() + ((t - t_start) / (t_end - t_start).max(1e-6)) as f32 * plot.width();
        let y_of = |f: f64| plot.bottom() - ((f - f_lo) / (f_hi - f_lo)) as f32 * plot.height();
        for k in 0..=4 {
            let f = f_lo + (f_hi - f_lo) * k as f64 / 4.0;
            p.text(Pos2::new(plot.left() - 4.0, y_of(f)), Align2::RIGHT_CENTER, format!("{f:.0}"), FontId::proportional(10.0), DIM);
        }
        p.text(Pos2::new(rect.left(), plot.top()), Align2::LEFT_TOP, "Hz", FontId::proportional(10.0), DIM);
        let a = s.anchor.clone();
        let tlabel = |t: f64| a.as_ref().map_or(format!("{t:.0} s"), |a| format!("{} UTC", fmt_clock(a.utc(t))));
        p.text(Pos2::new(plot.left(), rect.bottom()), Align2::LEFT_BOTTOM, tlabel(t_start), FontId::proportional(10.0), DIM);
        p.text(Pos2::new(plot.right(), rect.bottom()), Align2::RIGHT_BOTTOM, tlabel(t_end), FontId::proportional(10.0), DIM);
        let clip = p.with_clip_rect(plot);
        // Plugin tracks (for adsb-check: the Doppler curve the flight data predicts).
        for (c, tracks) in &overlays {
            for (_, pts) in tracks {
                for &(t, hz, ok) in pts {
                    if t >= t_start && (f_lo..=f_hi).contains(&hz) {
                        clip.circle_filled(Pos2::new(x_of(t), y_of(hz)), 1.4, alpha(*c, if ok { 0.9 } else { 0.4 }));
                    }
                }
            }
        }
        let mut hover = None;
        for (e, pending) in pl.passes.iter().map(|e| (e, false)).chain(pl.pending.iter().map(|e| (e, true))) {
            if e.t_end < t_start {
                continue;
            }
            let col = if pending { alpha(TEXT, 0.5) } else { grade_color(e.grade()) };
            let pts: Vec<Pos2> = e.points.iter().map(|&(t, f)| Pos2::new(x_of(t), y_of(f))).collect();
            for w in pts.windows(2) {
                clip.line_segment([w[0], w[1]], Stroke::new(1.6, col));
            }
            if let Some(first) = pts.first() {
                clip.text(*first + Vec2::new(0.0, -6.0), Align2::LEFT_BOTTOM, format!("{:.0} m/s · {:.0} m", e.speed, e.dist), FontId::proportional(10.0), col);
                if resp.hover_pos().is_some_and(|h| pts.iter().any(|q| q.distance(h) < 6.0)) {
                    hover = Some(format!(
                        "pass {}{}\nclosest approach {}\nspeed {:.0} m/s, distance {:.0} m, f0 {:.1} Hz\nSNR {:.1}, fit rms {:.2} Hz, grade {}",
                        e.id,
                        if pending { " (candidate)" } else { "" },
                        tlabel(e.t_cpa),
                        e.speed,
                        e.dist,
                        e.f0,
                        e.snr,
                        e.rms,
                        e.grade()
                    ));
                }
            }
        }
        drop(s);
        if let Some(t) = hover {
            resp.on_hover_text(t);
        }
        ui.colored_label(DIM, "green/orange/grey: passes by grade · white: candidates · dots: plugin tracks (adsb-check: predicted curves from flight data)");
    }
}

/// How this node looks on the canvas: the standard look. Override `NodeLook` methods here to
/// change it (see `src/bin/corroborate/look.rs`).
pub struct Look;

impl crate::look::NodeLook for Look {}
