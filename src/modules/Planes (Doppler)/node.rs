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

pub fn json(s: &NodeState) -> String {
    let p: Vec<String> = s.planes.passes.iter().map(|e| e.to_json(s.anchor.as_ref())).collect();
    format!(",\"passes\":[{}]", p.join(","))
}

/// What part of the spectrogram the node shows. Only the picture: the detector always sees all audio.
#[derive(Clone, Copy)]
struct View {
    /// Seconds shown; 0 = everything.
    span: f64,
    /// Right edge in audio seconds; None = follow the newest audio.
    end: Option<f64>,
    /// Frequency range shown; None = the detector's whole band.
    band: Option<(f64, f64)>,
}

/// The right edge of node `i`'s view in audio seconds (None: following the newest audio).
#[allow(dead_code)]
pub fn view_end(ctx: &egui::Context, i: NodeId) -> Option<f64> {
    ctx.data(|d| d.get_temp::<View>(egui::Id::new(("planes-view", i)))).and_then(|v| v.end)
}

impl Default for View {
    fn default() -> Self {
        Self { span: 160.0, end: None, band: None }
    }
}

const LIST_W: f32 = 290.0;
/// The old doppler-live viewer's colours.
const BG: Color32 = Color32::from_rgb(10, 11, 13);
const PANEL: Color32 = Color32::from_rgb(22, 24, 28);
const ACCENT: Color32 = Color32::from_rgb(120, 224, 150);
const BLUE: Color32 = Color32::from_rgb(90, 120, 220);

/// The Planes node on the canvas: the spectrogram with detected curves and plugin tracks, view
/// controls (zoom and move time and Hz, Follow) and the list of aircraft; clicking one centres it.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, _app: &App, _i: NodeId, _standard: Vec2) -> Vec2 {
        Vec2::new(1100.0, 500.0)
    }

    fn fill(&self, _app: &App, _i: NodeId) -> Color32 {
        BG
    }

    fn status(&self) -> bool {
        false
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let z = app.zoom;
        let id = egui::Id::new(("planes-view", i));
        let mut v: View = ui.data(|d| d.get_temp(id)).unwrap_or_default();
        // Follow the linked playhead when it moves: centre it (this stops Follow, as a pan does).
        let seen: u64 = ui.data(|d| d.get_temp(id.with("ph"))).unwrap_or(0);
        if seen != app.ph.rev {
            if let Some(t) = app.clock_of(i).and_then(|c| app.playhead_on(&c)) {
                v.end = Some(t + if v.span > 0.0 { v.span } else { 160.0 } * 0.5);
                ui.data_mut(|d| d.insert_temp(id, v));
            }
            ui.data_mut(|d| d.insert_temp(id.with("ph"), app.ph.rev));
        }
        let st = app.nodes[i].state.clone();
        let key = st.lock().unwrap().source_key.clone();
        let overlays: Vec<(Color32, Vec<(String, Vec<(f64, f64, bool)>)>)> = app
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
        let a = s.anchor.clone();
        let tlabel = |t: f64| a.as_ref().map_or(fmt_rel(t), |a| format!("{} UTC", fmt_clock(a.utc(t))));
        let font = FontId::monospace(10.0 * z);
        let list = Rect::from_min_max(Pos2::new(r.right() - LIST_W * z, r.top()), r.max);
        let left = Rect::from_min_max(r.min, Pos2::new(list.left() - 8.0 * z, r.bottom()));
        let bar = Rect::from_min_max(left.min, Pos2::new(left.right(), left.top() + 22.0 * z));
        let nb = pl.freqs.len();
        let nf = pl.hist_t.len();
        let have = nb > 0 && nf > 0;
        let (t_first, t_last) = if have { (pl.hist_t[0], *pl.hist_t.last().unwrap()) } else { (0.0, 0.0) };
        let mut jump = None;
        // The old viewer's theme: near-black panels, green accents, monospace text.
        ui.painter().rect_filled(r, 0.0, BG);
        ui.painter().text(
            bar.left_center() + Vec2::new(4.0 * z, 0.0),
            Align2::LEFT_CENTER,
            format!("PASSES {}  candidates {}  AUDIO {}  view {:.0} s", pl.passes.len(), pl.pending.len(), fmt_rel(s.audio_s), if v.span > 0.0 { v.span } else { t_last - t_first }),
            font.clone(),
            ACCENT,
        );
        // PLANES box on the right: the detected aircraft.
        let pbox = list.shrink(2.0 * z);
        ui.painter().rect_filled(pbox, 2.0 * z, PANEL);
        ui.painter().rect_filled(Rect::from_min_size(pbox.min, Vec2::new(3.0 * z, pbox.height())), 0.0, ACCENT);
        crate::look::area(ui, pbox.shrink2(Vec2::new(10.0 * z, 6.0 * z)), |ui| {
            ui.label(egui::RichText::new("PLANES").monospace().color(TEXT));
            crate::look::line(ui, format!("aircraft Doppler curves · {:.0}-{:.0} Hz", pl.freqs.first().unwrap_or(&0.0), pl.freqs.last().unwrap_or(&0.0)), DIM);
            crate::look::line(ui, "click a pass to jump to it", DIM);
            crate::look::line(ui, "  #  T_CPA      f0     v    d  grade", DIM);
            egui::ScrollArea::vertical().id_salt(("planes-list", i)).auto_shrink([false; 2]).show(ui, |ui| {
                if pl.passes.is_empty() {
                    crate::look::line(ui, "none yet", DIM);
                }
                for e in &pl.passes {
                    let t = format!("{:>3}  {}  {:5.1}  {:4.0}  {:.1}k  {}", e.id, tlabel(e.t_cpa), e.f0, e.speed, e.dist / 1000.0, e.grade());
                    let b = ui.add(egui::Button::new(egui::RichText::new(t).small().monospace().color(grade_color(e.grade()))).frame(false));
                    if b.clicked() {
                        jump = Some(e.clone());
                    }
                    b.on_hover_text(format!("SNR (signal-to-noise ratio) {:.2}, fit rms {:.2} Hz, {:.0}–{:.0} s", e.snr, e.rms, e.t_start, e.t_end));
                }
            });
        });
        let plot_r = Rect::from_min_max(Pos2::new(left.left() + 56.0 * z, bar.bottom() + 4.0 * z), Pos2::new(left.right(), left.bottom() - 40.0 * z));
        // Left: the whole band with the shown Hz range as a box. Bottom: the whole audio with the
        // shown time as a green window, and FOLLOW.
        let fbar = Rect::from_min_max(Pos2::new(left.left() + 4.0 * z, plot_r.top()), Pos2::new(left.left() + 18.0 * z, plot_r.bottom()));
        let tbar = Rect::from_min_max(Pos2::new(plot_r.left(), left.bottom() - 18.0 * z), Pos2::new(plot_r.right() - 90.0 * z, left.bottom() - 2.0 * z));
        let fbtn = Rect::from_min_max(Pos2::new(tbar.right() + 8.0 * z, tbar.top()), Pos2::new(plot_r.right(), tbar.bottom()));
        if !have {
            ui.painter().text(plot_r.center(), Align2::CENTER_CENTER, "no audio yet: wire a source", font, DIM);
            drop(s);
            ui.data_mut(|d| d.insert_temp(id, v));
            return;
        }
        let (f_lo0, f_hi0) = (pl.freqs[0], pl.freqs[nb - 1]);
        // Jump: centre the clicked aircraft in time and Hz.
        if let Some(e) = &jump {
            let pad = ((e.t_end - e.t_start) * 0.5).max(10.0);
            v.span = e.t_end - e.t_start + 2.0 * pad;
            v.end = Some(e.t_end + pad);
            let (lo, hi) = e.points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &(_, f)| (lo.min(f), hi.max(f)));
            if lo < hi {
                let m = (hi - lo) * 0.6 + 5.0;
                v.band = Some(((lo - m).max(f_lo0), (hi + m).min(f_hi0)));
            }
        }
        let resp = ui.interact(plot_r, id.with("plot"), Sense::click_and_drag());
        let span_of = |v: &View| if v.span > 0.0 { v.span } else { t_last - t_first }.max(1.0);
        let t1 = v.end.unwrap_or(t_last);
        let span = span_of(&v);
        let t0 = t1 - span;
        let (f_lo, f_hi) = v.band.unwrap_or((f_lo0, f_hi0));
        // Wheel: zoom time at the cursor; Shift+wheel: zoom Hz; drag: move time and Hz.
        if let Some(h) = resp.hover_pos() {
            let (dy, shift) = ui.input_mut(|inp| {
                let d = inp.smooth_scroll_delta.y + inp.smooth_scroll_delta.x;
                inp.smooth_scroll_delta = Vec2::ZERO;
                (d, inp.modifiers.shift)
            });
            if dy != 0.0 {
                let k = (-(dy as f64) / 200.0).exp();
                if shift {
                    let fc = f_hi - ((h.y - plot_r.top()) / plot_r.height()) as f64 * (f_hi - f_lo);
                    let (lo, hi) = ((fc - (fc - f_lo) * k).max(f_lo0), (fc + (f_hi - fc) * k).min(f_hi0));
                    if hi - lo > 2.0 {
                        v.band = Some((lo, hi));
                    }
                } else {
                    let tc = t0 + ((h.x - plot_r.left()) / plot_r.width()) as f64 * span;
                    let ns = (span * k).clamp(5.0, (t_last - t_first).max(5.0));
                    let ne = tc + (t1 - tc) * ns / span;
                    v.span = ns;
                    v.end = if v.end.is_none() && ne >= t_last { None } else { Some(ne.min(t_last)) };
                }
            }
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            v.end = Some((t1 - (d.x / plot_r.width()) as f64 * span).clamp(t_first + 1.0, t_last));
            let df = (d.y / plot_r.height()) as f64 * (f_hi - f_lo);
            if df != 0.0 && f_lo + df >= f_lo0 && f_hi + df <= f_hi0 {
                v.band = Some((f_lo + df, f_hi + df));
            }
        }
        if resp.double_clicked() {
            v = View::default();
        }
        // Frequency bar: drag or click moves the Hz box; wheel resizes it.
        let fr = ui.interact(fbar.expand(3.0 * z), id.with("fbar"), Sense::click_and_drag());
        let hz_at = |y: f32| f_lo0 + ((fbar.bottom() - y) / fbar.height()).clamp(0.0, 1.0) as f64 * (f_hi0 - f_lo0);
        if let Some(h) = fr.interact_pointer_pos().filter(|_| fr.dragged() || fr.clicked()) {
            let (lo, hi) = v.band.unwrap_or((f_lo0, f_hi0));
            let half = (hi - lo) / 2.0;
            let c = hz_at(h.y).clamp(f_lo0 + half, f_hi0 - half);
            v.band = Some((c - half, c + half));
        }
        if fr.hovered() {
            let dy = ui.input_mut(|inp| std::mem::take(&mut inp.smooth_scroll_delta).y);
            if dy != 0.0 {
                let (lo, hi) = v.band.unwrap_or((f_lo0, f_hi0));
                let (c, half) = ((lo + hi) / 2.0, ((hi - lo) / 2.0 * (-(dy as f64) / 200.0).exp()).clamp(2.0, (f_hi0 - f_lo0) / 2.0));
                let c = c.clamp(f_lo0 + half, f_hi0 - half);
                v.band = Some((c - half, c + half));
            }
        }
        // Time bar: drag or click moves the window; wheel resizes it.
        let tr = ui.interact(tbar, id.with("tbar"), Sense::click_and_drag());
        let t_at = |x: f32| t_first + ((x - tbar.left()) / tbar.width()).clamp(0.0, 1.0) as f64 * (t_last - t_first);
        if let Some(h) = tr.interact_pointer_pos().filter(|_| tr.dragged() || tr.clicked()) {
            let sp = span_of(&v);
            let e = (t_at(h.x) + sp / 2.0).min(t_last);
            v.end = if e >= t_last && v.end.is_none() { None } else { Some(e) };
        }
        if tr.hovered() {
            let dy = ui.input_mut(|inp| std::mem::take(&mut inp.smooth_scroll_delta).y);
            if dy != 0.0 {
                v.span = (span_of(&v) * (-(dy as f64) / 200.0).exp()).clamp(5.0, (t_last - t_first).max(5.0));
            }
        }
        if ui.interact(fbtn, id.with("follow"), Sense::click()).clicked() {
            v.end = if v.end.is_none() { Some(t_last) } else { None };
        }
        // Draw with the view as it is after this frame's input.
        let t1 = v.end.unwrap_or(t_last);
        let t0 = t1 - span_of(&v);
        let (f_lo, f_hi) = v.band.unwrap_or((f_lo0, f_hi0));
        let f_first = pl.hist_t.partition_point(|&t| t < t0).min(nf - 1);
        let f_last = pl.hist_t.partition_point(|&t| t <= t1).max(f_first + 1).min(nf);
        let cols_n = f_last - f_first;
        let w = cols_n.clamp(1, 2048);
        // The picture is rebuilt when new frames arrive or the view moves in time.
        let stamp = (pl.frames, f_first as f64 * 1e7 + f_last as f64);
        if app.textures.get(&i).is_none_or(|(f, ts, _)| (*f, *ts) != stamp) {
            let mut px = vec![0u8; w * nb * 4];
            for c in 0..w {
                let a = f_first + c * cols_n / w;
                let b = (f_first + (c + 1) * cols_n / w).max(a + 1).min(nf);
                for bin in 0..nb {
                    let m = (a..b).map(|f| pl.hist[f * nb + bin]).max().unwrap_or(0);
                    let row = nb - 1 - bin;
                    px[(row * w + c) * 4..(row * w + c) * 4 + 4].copy_from_slice(&app.lut[m as usize]);
                }
            }
            let img = ColorImage::from_rgba_unmultiplied([w, nb], &px);
            match app.textures.get_mut(&i) {
                Some((f, ts, tex)) => {
                    tex.set(img, TextureOptions::LINEAR);
                    (*f, *ts) = stamp;
                }
                None => {
                    let tex = ui.ctx().load_texture(format!("spec{i}"), img, TextureOptions::LINEAR);
                    app.textures.insert(i, (stamp.0, stamp.1, tex));
                }
            }
        }
        let tex = app.textures[&i].2.id();
        let p = ui.painter_at(left);
        p.rect_filled(plot_r, 0.0, Color32::BLACK);
        let x_of = |t: f64| plot_r.left() + ((t - t0) / (t1 - t0).max(1e-6)) as f32 * plot_r.width();
        let y_of = |f: f64| plot_r.bottom() - ((f - f_lo) / (f_hi - f_lo)) as f32 * plot_r.height();
        // Columns may not fill [t0, t1] near the start; Hz zoom crops the texture's rows.
        let img_r = Rect::from_min_max(Pos2::new(x_of(pl.hist_t[f_first]), plot_r.top()), Pos2::new(x_of(pl.hist_t[f_last - 1]), plot_r.bottom()));
        let uv_y = |f: f64| (1.0 - (f - f_lo0) / (f_hi0 - f_lo0)) as f32;
        p.with_clip_rect(plot_r).image(tex, img_r, Rect::from_min_max(Pos2::new(0.0, uv_y(f_hi)), Pos2::new(1.0, uv_y(f_lo))), Color32::WHITE);
        // The two bars and FOLLOW.
        let fy = |f: f64| fbar.bottom() - ((f - f_lo0) / (f_hi0 - f_lo0)) as f32 * fbar.height();
        p.rect_filled(fbar, 2.0 * z, PANEL);
        p.rect_stroke(Rect::from_min_max(Pos2::new(fbar.left(), fy(f_hi)), Pos2::new(fbar.right(), fy(f_lo))), 1.0, Stroke::new(1.2 * z, TEXT), egui::StrokeKind::Inside);
        let tx = |t: f64| tbar.left() + ((t - t_first) / (t_last - t_first).max(1e-6)) as f32 * tbar.width();
        p.rect_filled(tbar, 2.0 * z, PANEL);
        for e in &pl.passes {
            p.line_segment([Pos2::new(tx(e.t_cpa), tbar.top() + 3.0 * z), Pos2::new(tx(e.t_cpa), tbar.bottom() - 3.0 * z)], Stroke::new(2.0 * z, BLUE));
        }
        p.rect_stroke(Rect::from_min_max(Pos2::new(tx(t0.max(t_first)), tbar.top()), Pos2::new(tx(t1).max(tx(t0.max(t_first)) + 3.0), tbar.bottom())), 1.0, Stroke::new(1.5 * z, ACCENT), egui::StrokeKind::Inside);
        p.text(Pos2::new(tbar.left() - 6.0 * z, tbar.center().y), Align2::RIGHT_CENTER, "time", font.clone(), DIM);
        let on = v.end.is_none();
        p.rect_stroke(fbtn, 2.0 * z, Stroke::new(1.0 * z, if on { ACCENT } else { DIM }), egui::StrokeKind::Inside);
        p.text(fbtn.center(), Align2::CENTER_CENTER, "● FOLLOW", font.clone(), if on { ACCENT } else { DIM });
        for k in 0..=5 {
            let f = f_lo + (f_hi - f_lo) * k as f64 / 5.0;
            p.text(Pos2::new(plot_r.left() - 3.0 * z, y_of(f)), Align2::RIGHT_CENTER, format!("{f:.0}"), font.clone(), DIM);
        }
        p.text(Pos2::new(left.left(), plot_r.top()), Align2::LEFT_TOP, "Hz", font.clone(), DIM);
        for k in 0..=6 {
            let t = t0 + (t1 - t0) * k as f64 / 6.0;
            let al = if k == 0 { Align2::LEFT_TOP } else if k == 6 { Align2::RIGHT_TOP } else { Align2::CENTER_TOP };
            p.text(Pos2::new(x_of(t), plot_r.bottom() + 3.0 * z), al, tlabel(t), font.clone(), DIM);
        }
        let clip = p.with_clip_rect(plot_r);
        for (c, tracks) in &overlays {
            for (_, pts) in tracks {
                for &(t, hz, ok) in pts {
                    if (t0..=t1).contains(&t) && (f_lo..=f_hi).contains(&hz) {
                        clip.circle_filled(Pos2::new(x_of(t), y_of(hz)), 1.4 * z, alpha(*c, if ok { 0.9 } else { 0.4 }));
                    }
                }
            }
        }
        let mut hover = None;
        for (e, pending) in pl.passes.iter().map(|e| (e, false)).chain(pl.pending.iter().map(|e| (e, true))) {
            if e.t_end < t0 || e.t_start > t1 {
                continue;
            }
            let col = if pending { alpha(TEXT, 0.5) } else { grade_color(e.grade()) };
            let pts: Vec<Pos2> = e.points.iter().map(|&(t, f)| Pos2::new(x_of(t), y_of(f))).collect();
            for w in pts.windows(2) {
                clip.line_segment([w[0], w[1]], Stroke::new(1.6 * z, col));
            }
            if let Some(first) = pts.first() {
                clip.text(*first + Vec2::new(0.0, -6.0 * z), Align2::LEFT_BOTTOM, format!("#{} {:.0} m/s · {:.0} m", e.id, e.speed, e.dist), font.clone(), col);
                if resp.hover_pos().is_some_and(|h| pts.iter().any(|q| q.distance(h) < 6.0)) {
                    hover = Some(format!(
                        "pass {}{}\nclosest approach {}\nspeed {:.0} m/s, distance {:.0} m, f0 {:.1} Hz\nSNR (signal-to-noise ratio) {:.1}, fit rms {:.2} Hz, grade {}",
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
        resp.on_hover_text(hover.unwrap_or_else(|| "wheel: zoom time · Shift+wheel: zoom Hz · drag: move · double-click: reset view".into()));
        ui.data_mut(|d| d.insert_temp(id, v));
    }
}

/// Audio seconds as H:MM:SS.
fn fmt_rel(t: f64) -> String {
    let s = t.max(0.0) as u64;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}
