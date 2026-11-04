//! doppler-view: live display of the detector.
//!
//! Waveform, scrolling whitened spectrogram (exactly what the detector searches), a live
//! spectrum slice on the same frequency axis, raw ridges, held candidates and reported passes
//! drawn as their fitted Doppler curves. Audio is processed on a worker thread at real time
//! (or faster with --speed); the window only reads shared state.

use doppler_live::fit::model;
use doppler_live::{ingest, Config, Detector, Event};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, ColorImage, FontFamily, FontId, Pos2, Rect, Sense, Shape, Stroke,
    TextureHandle, TextureOptions,
};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const USAGE: &str = "\
doppler-view: live view of the Doppler-curve detector

USAGE:
  doppler-view <file-or-url> [--start <s>] [--speed <x>]
  doppler-view --stdin            (raw f32le mono 4000 Hz on stdin)

  --start <s>   seek into a file before playing (via ffmpeg)
  --speed <x>   playback speed for files (default 1 = real time; live streams run at 1)";

const WAVE_S: f64 = 5.0;

// Palette: near-black field, phosphor accents.
const BG: Color32 = Color32::from_rgb(8, 9, 11);
const PANEL: Color32 = Color32::from_rgb(13, 15, 18);
const GRID: Color32 = Color32::from_rgb(34, 38, 44);
const DIM: Color32 = Color32::from_rgb(110, 118, 128);
const TEXT: Color32 = Color32::from_rgb(198, 206, 214);
const WAVE: Color32 = Color32::from_rgb(120, 220, 160);
const RIDGE: Color32 = Color32::from_rgb(80, 200, 230);
const HIGH: Color32 = Color32::from_rgb(140, 255, 90);
const MED: Color32 = Color32::from_rgb(255, 190, 60);
const LOW: Color32 = Color32::from_rgb(150, 150, 150);

struct Shared {
    sr: f64,
    offset: f64,
    source: String,
    freqs: Vec<f64>,
    /// Last WAVE_S seconds of samples, ring.
    wave: Vec<f32>,
    wave_head: usize,
    wave_n: u64,
    /// Spectrogram ring: `cols` columns of `nb` z values.
    cols: usize,
    spec: Vec<f32>,
    spec_t: Vec<f64>,
    spec_head: usize,
    spec_frames: u64,
    now: f64,
    ridges: Vec<Vec<(f64, f64)>>,
    pending: Vec<Event>,
    events: Vec<Event>,
    dsp_s: f64,
    audio_s: f64,
    done: bool,
    live: bool,
    status: String,
    c: f64,
}

fn grade_color(g: &str) -> Color32 {
    match g {
        "high" => HIGH,
        "medium" => MED,
        _ => LOW,
    }
}

/// z → colour: black, deep blue, violet, orange, pale yellow.
fn colormap(z: f32) -> [u8; 4] {
    const STOPS: [(f32, [f32; 3]); 6] = [
        (0.00, [4.0, 5.0, 8.0]),
        (0.22, [18.0, 22.0, 60.0]),
        (0.45, [90.0, 30.0, 120.0]),
        (0.65, [200.0, 60.0, 70.0]),
        (0.85, [250.0, 160.0, 40.0]),
        (1.00, [255.0, 250.0, 200.0]),
    ];
    let x = ((z + 1.0) / 7.0).clamp(0.0, 1.0);
    let mut i = 0;
    while i + 1 < STOPS.len() - 1 && x > STOPS[i + 1].0 {
        i += 1;
    }
    let (a, ca) = STOPS[i];
    let (b, cb) = STOPS[i + 1];
    let u = ((x - a) / (b - a)).clamp(0.0, 1.0);
    [
        (ca[0] + u * (cb[0] - ca[0])) as u8,
        (ca[1] + u * (cb[1] - ca[1])) as u8,
        (ca[2] + u * (cb[2] - ca[2])) as u8,
        255,
    ]
}

fn fmt_clock(t: f64) -> String {
    let t = t.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}

// ---------------------------------------------------------------- worker

fn worker(sh: Arc<Mutex<Shared>>, src: Source, speed: f64) {
    let cfg = Config::default();
    let sr = cfg.sr;
    let c = cfg.c;
    let mut det = Detector::new(cfg);
    let t0 = Instant::now();
    let mut n: u64 = 0;
    let live = matches!(src, Source::Stdin) || src.is_url();

    let mut handle = |x: &[f32], det: &mut Detector| {
        // Pace to real time × speed, in small slices so the display moves smoothly.
        for piece in x.chunks(200) {
            if !live {
                let target = n as f64 / sr / speed;
                let el = t0.elapsed().as_secs_f64();
                if target > el {
                    thread::sleep(Duration::from_secs_f64(target - el));
                }
            }
            let tick = Instant::now();
            let mut frames: Vec<(f64, Vec<f32>)> = Vec::new();
            let evs = det.push_with(piece, |t, z| frames.push((t, z.to_vec())));
            let cost = tick.elapsed().as_secs_f64();
            n += piece.len() as u64;

            let mut s = sh.lock().unwrap();
            let wl = s.wave.len();
            for &v in piece {
                let h = s.wave_head;
                s.wave[h] = v;
                s.wave_head = (h + 1) % wl;
            }
            s.wave_n += piece.len() as u64;
            let nb = s.freqs.len();
            for (t, z) in frames {
                let h = s.spec_head;
                s.spec[h * nb..(h + 1) * nb].copy_from_slice(&z);
                s.spec_t[h] = t;
                s.spec_head = (h + 1) % s.cols;
                s.spec_frames += 1;
                s.now = t;
                s.ridges = det.ridges().to_vec();
                s.pending = det.pending().cloned().collect();
            }
            for e in evs {
                println!("{}", e.to_json(None));
                s.events.push(e);
            }
            s.dsp_s += cost;
            s.audio_s = n as f64 / sr;
            s.c = c;
        }
    };

    let r = match &src {
        Source::Wav(p) => ingest::read_wav(p, sr, |x| handle(x, &mut det)),
        Source::Ffmpeg(p, start) => match ingest::spawn_ffmpeg_from(p, sr as u32, *start) {
            Ok((mut child, rx)) => {
                for x in rx {
                    handle(&x, &mut det);
                }
                let _ = child.wait();
                Ok(())
            }
            Err(e) => Err(e),
        },
        Source::Stdin => {
            for x in ingest::spawn_stdin((sr / 4.0) as usize) {
                handle(&x, &mut det);
            }
            Ok(())
        }
    };
    let evs = det.flush();
    let mut s = sh.lock().unwrap();
    for e in evs {
        println!("{}", e.to_json(None));
        s.events.push(e);
    }
    s.pending.clear();
    s.done = true;
    s.status = match r {
        Ok(()) => "end of input".into(),
        Err(e) => format!("error: {e}"),
    };
}

enum Source {
    Wav(String),
    Ffmpeg(String, Option<f64>),
    Stdin,
}

impl Source {
    fn is_url(&self) -> bool {
        matches!(self, Source::Ffmpeg(p, _) if p.contains("://"))
    }
}

// ---------------------------------------------------------------- app

struct App {
    sh: Arc<Mutex<Shared>>,
    tex: Option<TextureHandle>,
    tex_frames: u64,
    img: Vec<u8>,
}

impl App {
    fn new(cc: &eframe::CreationContext, sh: Arc<Mutex<Shared>>) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        cc.egui_ctx.all_styles_mut(|style| {
            for (_, f) in style.text_styles.iter_mut() {
                f.family = FontFamily::Monospace;
            }
            style.visuals.panel_fill = PANEL;
            style.visuals.window_fill = PANEL;
            style.visuals.extreme_bg_color = BG;
            style.visuals.override_text_color = Some(TEXT);
        });
        Self { sh, tex: None, tex_frames: u64::MAX, img: Vec::new() }
    }
}

fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(33));
        let sh = self.sh.clone();
        let s = sh.lock().unwrap();
        let nb = s.freqs.len();
        let (fmin, fmax) = (s.freqs[0], s.freqs[nb - 1]);

        // Rebuild the spectrogram image when new frames have arrived.
        if s.spec_frames != self.tex_frames {
            self.tex_frames = s.spec_frames;
            let (w, h) = (s.cols, nb);
            self.img.resize(w * h * 4, 0);
            let filled = (s.spec_frames as usize).min(w);
            for col in 0..w {
                // Column 0 = oldest; right edge = now.
                let age = w - 1 - col;
                let src = if age < filled { Some((s.spec_head + w - 1 - age) % w) } else { None };
                for row in 0..h {
                    let bin = h - 1 - row;
                    let px = match src {
                        Some(k) => colormap(s.spec[k * nb + bin]),
                        None => [4, 5, 8, 255],
                    };
                    let i = (row * w + col) * 4;
                    self.img[i..i + 4].copy_from_slice(&px);
                }
            }
            let image = ColorImage::from_rgba_unmultiplied([w, h], &self.img);
            match &mut self.tex {
                Some(t) => t.set(image, TextureOptions::LINEAR),
                None => self.tex = Some(ctx.load_texture("spec", image, TextureOptions::LINEAR)),
            }
        }

        // ------------------------------------------------ top bar
        egui::Panel::top("top").frame(egui::Frame::new().fill(BG).inner_margin(8.0)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("DOPPLER-LIVE").font(mono(15.0)).color(HIGH).strong());
                ui.label(egui::RichText::new("▸").color(DIM));
                ui.label(egui::RichText::new(&s.source).color(DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let load = 100.0 * s.dsp_s / s.audio_s.max(1e-9);
                    let high = s.events.iter().filter(|e| e.grade() == "high").count();
                    let state = if s.done {
                        s.status.clone()
                    } else if s.live {
                        "● LIVE".into()
                    } else {
                        "▶ PLAYING".into()
                    };
                    ui.label(egui::RichText::new(state).color(if s.done { DIM } else { MED }));
                    ui.label(egui::RichText::new(format!(
                        "T {}   DSP {:5.2}%   PASSES {} ({} high)",
                        fmt_clock(s.offset + s.now),
                        load,
                        s.events.len(),
                        high
                    )));
                });
            });
        });

        // ------------------------------------------------ bottom status
        egui::Panel::bottom("bottom").frame(egui::Frame::new().fill(BG).inner_margin(6.0)).show(ui, |ui| {
            let df = s.freqs[1] - s.freqs[0];
            ui.label(egui::RichText::new(format!(
                "sr {:.0} Hz  │  band {:.0}–{:.0} Hz  │  Δf {:.2} Hz  │  hop 0.25 s  │  window {:.0} s  │  whitened z  │  \
                 ridge ┄ cyan  │  candidate ┄ white  │  pass: high / medium / low",
                s.sr,
                fmin,
                fmax,
                df,
                s.cols as f64 / 4.0
            ))
            .font(mono(11.0))
            .color(DIM));
        });

        // ------------------------------------------------ event list
        egui::Panel::right("events").exact_size(330.0).frame(egui::Frame::new().fill(PANEL).inner_margin(10.0)).show(ui, |ui| {
            ui.label(egui::RichText::new("DETECTED PASSES").font(mono(13.0)).color(TEXT).strong());
            ui.add_space(4.0);
            ui.label(egui::RichText::new(" #  T_CPA     f0     v     d    grade").font(mono(11.0)).color(DIM));
            ui.separator();
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for e in &s.events {
                    let line = format!(
                        "{:>2}  {}  {:5.1}  {:3.0}  {:4.1}k  {}",
                        e.id,
                        fmt_clock(s.offset + e.t_cpa),
                        e.f0,
                        e.speed,
                        e.dist / 1000.0,
                        e.grade()
                    );
                    ui.label(egui::RichText::new(line).font(mono(12.0)).color(grade_color(e.grade())));
                    ui.label(
                        egui::RichText::new(format!(
                            "    {:.0}→{:.0} Hz  snr {:.2}  rms {:.2}  h{}",
                            e.f_pre, e.f_post, e.snr, e.rms, e.harmonics
                        ))
                        .font(mono(10.5))
                        .color(DIM),
                    );
                }
                if s.events.is_empty() {
                    ui.label(egui::RichText::new("listening…").color(DIM));
                }
            });
        });

        // ------------------------------------------------ main area
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(10.0)).show(ui, |ui| {
            let avail = ui.available_size();
            let wave_h = (avail.y * 0.18).clamp(80.0, 160.0);

            // Waveform
            let (wrect, _) = ui.allocate_exact_size(vec2(avail.x, wave_h), Sense::hover());
            draw_wave(ui, wrect, &s);
            ui.add_space(8.0);

            // Spectrogram + slice
            let rest = ui.available_size();
            let slice_w = 120.0;
            let axis_l = 52.0;
            let axis_b = 20.0;
            let (area, _) = ui.allocate_exact_size(rest, Sense::hover());
            let spec_rect = Rect::from_min_max(
                pos2(area.left() + axis_l, area.top()),
                pos2(area.right() - slice_w - 10.0, area.bottom() - axis_b),
            );
            let slice_rect = Rect::from_min_max(
                pos2(spec_rect.right() + 10.0, spec_rect.top()),
                pos2(area.right(), spec_rect.bottom()),
            );
            let p = ui.painter();
            p.rect_filled(spec_rect, 0.0, Color32::from_rgb(4, 5, 8));
            if let Some(t) = &self.tex {
                p.image(t.id(), spec_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }

            let span = s.cols as f64 / 4.0;
            let t_right = s.now;
            let t_left = t_right - span;
            let x_of = |t: f64| spec_rect.left() + ((t - t_left) / span) as f32 * spec_rect.width();
            let y_of = |f: f64| spec_rect.bottom() - ((f - fmin) / (fmax - fmin)) as f32 * spec_rect.height();

            // Frequency grid and labels.
            let mut f = (fmin / 50.0).ceil() * 50.0;
            while f <= fmax {
                let y = y_of(f);
                p.line_segment([pos2(spec_rect.left(), y), pos2(spec_rect.right(), y)], Stroke::new(0.5, GRID.gamma_multiply(0.6)));
                p.line_segment([pos2(slice_rect.left(), y), pos2(slice_rect.right(), y)], Stroke::new(0.5, GRID));
                p.text(pos2(spec_rect.left() - 6.0, y), Align2::RIGHT_CENTER, format!("{f:.0}"), mono(10.5), DIM);
                f += 50.0;
            }
            p.text(pos2(area.left() + 2.0, spec_rect.top() - 2.0), Align2::LEFT_BOTTOM, "Hz", mono(10.5), DIM);

            // Time grid every 20 s, labelled relative to now.
            let mut k = 0.0;
            while k <= span {
                let x = x_of(t_right - k);
                p.line_segment([pos2(x, spec_rect.top()), pos2(x, spec_rect.bottom())], Stroke::new(0.5, GRID.gamma_multiply(0.6)));
                let lbl = if k == 0.0 { "now".to_string() } else { format!("-{k:.0}s") };
                p.text(pos2(x, spec_rect.bottom() + 4.0), Align2::CENTER_TOP, lbl, mono(10.5), DIM);
                k += 20.0;
            }

            let clip = p.with_clip_rect(spec_rect);
            // Raw ridges from the latest search.
            for r in &s.ridges {
                let pts: Vec<Pos2> = r.iter().step_by(2).map(|&(t, f)| pos2(x_of(t), y_of(f))).collect();
                if pts.len() > 1 {
                    clip.extend(Shape::dashed_line(&pts, Stroke::new(1.0, RIDGE.gamma_multiply(0.7)), 3.0, 3.0));
                }
            }
            // Candidates being held.
            for e in &s.pending {
                let pts = curve(e, s.c, t_left, t_right, &x_of, &y_of);
                clip.extend(Shape::dashed_line(&pts, Stroke::new(1.5, Color32::WHITE), 6.0, 4.0));
            }
            // Reported passes: fitted curve, extended a little past the data, CPA tick, label.
            for e in &s.events {
                if e.t_end + 40.0 < t_left {
                    continue;
                }
                let col = grade_color(e.grade());
                let pts = curve(e, s.c, t_left, t_right, &x_of, &y_of);
                if pts.len() > 1 {
                    clip.add(Shape::line(pts, Stroke::new(2.0, col)));
                }
                let pf = [e.f0, e.speed, e.dist, e.t_cpa];
                let xc = x_of(e.t_cpa);
                let yc = y_of(model(e.t_cpa, &pf, s.c));
                clip.line_segment([pos2(xc, yc - 14.0), pos2(xc, yc + 14.0)], Stroke::new(1.0, col));
                clip.text(
                    pos2(xc + 6.0, yc - 16.0),
                    Align2::LEFT_BOTTOM,
                    format!("#{} {:.0} m/s {:.1} km [{}]", e.id, e.speed, e.dist / 1000.0, e.grade()),
                    mono(11.0),
                    col,
                );
            }
            p.rect_stroke(spec_rect, 0.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);

            // Live spectrum slice (latest frame), same frequency axis.
            p.rect_filled(slice_rect, 0.0, Color32::from_rgb(4, 5, 8));
            if s.spec_frames > 0 {
                let k = (s.spec_head + s.cols - 1) % s.cols;
                let row = &s.spec[k * nb..(k + 1) * nb];
                let x_of_z = |z: f32| slice_rect.left() + ((z + 1.0) / 8.0).clamp(0.0, 1.0) * slice_rect.width();
                let pts: Vec<Pos2> = row
                    .iter()
                    .enumerate()
                    .step_by(2)
                    .map(|(i, &z)| pos2(x_of_z(z), y_of(s.freqs[i])))
                    .collect();
                p.with_clip_rect(slice_rect).add(Shape::line(pts, Stroke::new(1.0, WAVE)));
                let x0 = x_of_z(0.0);
                p.line_segment([pos2(x0, slice_rect.top()), pos2(x0, slice_rect.bottom())], Stroke::new(0.5, DIM));
            }
            p.rect_stroke(slice_rect, 0.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);
            p.text(pos2(slice_rect.center().x, slice_rect.bottom() + 4.0), Align2::CENTER_TOP, "z now", mono(10.5), DIM);
        });
    }
}

/// Fitted curve of `e` over the visible span, extended 20 s beyond its data.
fn curve(e: &Event, c: f64, t_left: f64, t_right: f64, x_of: &dyn Fn(f64) -> f32, y_of: &dyn Fn(f64) -> f32) -> Vec<Pos2> {
    let p = [e.f0, e.speed, e.dist, e.t_cpa];
    let a = (e.t_start - 20.0).max(t_left);
    let b = (e.t_end + 20.0).min(t_right);
    if b <= a {
        return Vec::new();
    }
    let n = 200;
    (0..=n)
        .map(|i| {
            let t = a + (b - a) * i as f64 / n as f64;
            pos2(x_of(t), y_of(model(t, &p, c)))
        })
        .collect()
}

fn draw_wave(ui: &egui::Ui, rect: Rect, s: &Shared) {
    let p = ui.painter();
    p.rect_filled(rect, 0.0, Color32::from_rgb(4, 5, 8));
    let mid = rect.center().y;
    p.line_segment([pos2(rect.left(), mid), pos2(rect.right(), mid)], Stroke::new(0.5, GRID));
    let wl = s.wave.len();
    let n = (s.wave_n as usize).min(wl);
    if n > 0 {
        // Peak for auto-scale; min/max envelope per pixel column.
        let at = |i: usize| s.wave[(s.wave_head + wl - n + i) % wl];
        let mut peak = 1e-6f32;
        for i in 0..n {
            peak = peak.max(at(i).abs());
        }
        let cols = rect.width().max(1.0) as usize;
        let per = (wl as f64 / cols as f64).max(1.0);
        let off = wl - n;
        let mut shapes = Vec::with_capacity(cols);
        for c in 0..cols {
            let a = (c as f64 * per) as usize;
            let b = (((c + 1) as f64 * per) as usize).min(wl);
            if b <= off {
                continue;
            }
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for i in a.max(off)..b {
                let v = at(i - off);
                lo = lo.min(v);
                hi = hi.max(v);
            }
            let x = rect.left() + c as f32;
            let h = rect.height() * 0.46;
            shapes.push(Shape::line_segment([pos2(x, mid - hi / peak * h), pos2(x, mid - lo / peak * h)], Stroke::new(1.0, WAVE)));
        }
        p.extend(shapes);
        let rms = (0..n).map(|i| at(i) * at(i)).sum::<f32>() / n as f32;
        p.text(
            rect.left_top() + vec2(6.0, 4.0),
            Align2::LEFT_TOP,
            format!("WAVEFORM  last {WAVE_S:.0} s   peak {:.3}   rms {:.1} dBFS   (auto-scaled)", peak, 10.0 * rms.max(1e-12).log10()),
            mono(10.5),
            DIM,
        );
    }
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);
}

// ---------------------------------------------------------------- main

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut input: Option<String> = None;
    let mut start: Option<f64> = None;
    let mut speed = 1.0;
    let mut stdin = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--start" => start = args.next().and_then(|v| v.parse().ok()),
            "--speed" => speed = args.next().and_then(|v| v.parse().ok()).unwrap_or(1.0),
            "--stdin" => stdin = true,
            "-h" | "--help" => {
                eprintln!("{USAGE}");
                return Ok(());
            }
            s => input = Some(s.to_string()),
        }
    }
    let src = if stdin {
        Source::Stdin
    } else {
        let i = input.ok_or(USAGE)?;
        if i.to_ascii_lowercase().ends_with(".wav") && start.is_none() {
            Source::Wav(i)
        } else {
            Source::Ffmpeg(i, start)
        }
    };
    let label = match &src {
        Source::Wav(p) | Source::Ffmpeg(p, _) => {
            let name = p.rsplit(['/', '\\']).next().unwrap_or(p);
            if name.len() > 60 { format!("{}…", &name[..60]) } else { name.to_string() }
        }
        Source::Stdin => "stdin".into(),
    };

    let cfg = Config::default();
    let det = Detector::new(cfg.clone());
    let freqs = det.bin_freqs();
    let cols = (cfg.window_s * cfg.sr / cfg.hop as f64) as usize;
    let nb = freqs.len();
    let sh = Arc::new(Mutex::new(Shared {
        sr: cfg.sr,
        offset: start.unwrap_or(0.0),
        source: if speed != 1.0 { format!("{label}  ×{speed}") } else { label },
        freqs,
        wave: vec![0.0; (WAVE_S * cfg.sr) as usize],
        wave_head: 0,
        wave_n: 0,
        cols,
        spec: vec![-1.0; cols * nb],
        spec_t: vec![0.0; cols],
        spec_head: 0,
        spec_frames: 0,
        now: 0.0,
        ridges: Vec::new(),
        pending: Vec::new(),
        events: Vec::new(),
        dsp_s: 0.0,
        audio_s: 0.0,
        done: false,
        live: matches!(src, Source::Stdin) || src.is_url(),
        status: String::new(),
        c: cfg.c,
    }));
    drop(det);

    let w = sh.clone();
    thread::spawn(move || worker(w, src, speed));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1480.0, 900.0]).with_title("doppler-view"),
        ..Default::default()
    };
    eframe::run_native("doppler-view", options, Box::new(move |cc| Ok(Box::new(App::new(cc, sh)))))
        .map_err(|e| e.to_string())
}
