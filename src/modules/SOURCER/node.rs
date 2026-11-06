//! The SOURCER node in the workbench: on the canvas, a source box (file, web page or stream URL,
//! with Browse) and a system streams box (microphones, virtual mics, capture cards), then what it
//! gave so far, a waveform of the audio and, for sources with pictures, a video viewer. Its size
//! and which outputs show are its look.rhai: a new node is just the two boxes, outputs hidden. Web pages are
//! resolved with yt-dlp. Compiled into the `corroborate` program.

use super::*;
use corroborate::modules::sourcer::has_video;
use std::io::Read;

/// Direct media URLs ffmpeg opens itself; every other http(s) link is a page for yt-dlp.
const DIRECT: [&str; 12] = [".mp3", ".wav", ".flac", ".ogg", ".opus", ".m4a", ".aac", ".mp4", ".webm", ".mkv", ".m3u8", ".ts"];
/// A microphone is written as `mic:<device name>`.
const MIC: &str = "mic:";
/// Samples (16 kHz) per waveform column: 0.1 s.
const PEAK: usize = 1600;
/// Video frames are kept small and at most this many; past it every other one is dropped.
const FW: usize = 192;
const FH: usize = 108;
const MAX_FRAMES: usize = 600;

/// What the node opens at Run: a microphone, a URL if it has a scheme, otherwise a file.
pub fn input(text: &str) -> Option<Input> {
    let t = text.trim().trim_matches('"');
    if t.is_empty() {
        None
    } else if let Some(d) = t.strip_prefix(MIC) {
        (!d.trim().is_empty()).then(|| Input::Mic(d.trim().to_string()))
    } else if t.contains("://") {
        Some(Input::Url(t.to_string()))
    } else {
        Some(Input::File(t.to_string()))
    }
}

/// What kind of source the text is, for the node's label line.
fn kind_of(text: &str) -> &'static str {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        "file, YouTube link, stream URL or microphone"
    } else if t.starts_with(MIC) {
        "microphone (live)"
    } else if !t.contains("://") {
        if has_video(&t) { "video file (recording)" } else { "audio file (recording)" }
    } else if t.contains("youtube.com/") || t.contains("youtu.be/") {
        "YouTube: video read as a recording, live stream read live (yt-dlp)"
    } else if !t.starts_with("http") || DIRECT.iter().any(|e| t.split('?').next().unwrap_or("").ends_with(e)) {
        "stream URL (ffmpeg)"
    } else {
        "web page (yt-dlp)"
    }
}

/// A direct media URL, and whether it is live. Web pages go through yt-dlp, which says both;
/// direct URLs are live unless they end in a file extension (an mp3 on a server is a recording).
pub fn resolve(url: &str) -> Result<(String, bool), String> {
    if !kind_of(url).contains("yt-dlp") {
        let path = url.to_lowercase();
        let file = DIRECT.iter().filter(|e| **e != ".m3u8").any(|e| path.split('?').next().unwrap_or("").ends_with(e));
        return Ok((url.to_string(), !file));
    }
    let out = corroborate::no_window(&mut std::process::Command::new("yt-dlp"))
        .args(["--no-warnings", "--no-playlist", "-f", "ba/b", "--print", "is_live", "--print", "urls", url])
        .output()
        .map_err(|e| format!("could not run yt-dlp (is it installed?): {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let is_live = lines.next().map(str::trim) == Some("True");
    match lines.next() {
        Some(u) if out.status.success() && u.contains("://") && is_live => Ok((u.trim().to_string(), true)),
        // Not live: YouTube refuses its direct links to ffmpeg (403), so yt-dlp downloads the
        // video into recordings\ and it is read as a file. The copy stays there as evidence.
        Some(_) if out.status.success() => download(url).map(|p| (p, false)),
        _ => Err(format!("yt-dlp: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no URL returned"))),
    }
}

/// Download a video (one file with picture and sound, at most 480 lines; sound only when there is
/// none) with yt-dlp to `recordings\<site> <id>.<ext>`; reuses an earlier download.
fn download(url: &str) -> Result<String, String> {
    let dir = corroborate::home().join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let out = corroborate::no_window(&mut std::process::Command::new("yt-dlp"))
        .args(["--no-warnings", "--no-playlist", "-f", "b[height<=480]/ba/b", "--print", "after_move:filepath", "-o"])
        .arg(dir.join("%(extractor)s %(id)s.%(ext)s"))
        .arg(url)
        .output()
        .map_err(|e| format!("could not run yt-dlp (is it installed?): {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match text.lines().rev().find(|l| !l.trim().is_empty()) {
        Some(p) if out.status.success() && std::path::Path::new(p.trim()).exists() => Ok(p.trim().to_string()),
        _ => Err(format!("yt-dlp download: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no file written"))),
    }
}

// ---------------------------------------------------------------- what a run gives

/// One small video frame (RGB) at `t` seconds of the source.
pub struct Frame {
    pub t: f64,
    pub rgb: Vec<u8>,
}

/// What the SOURCER keeps from a run for its own view and its metadata output.
#[derive(Default)]
pub struct SourceState {
    /// Lowest and highest sample of every 0.1 s.
    pub peaks: Vec<[f32; 2]>,
    acc: (f32, f32, usize),
    /// Video frames, oldest first, at least `frame_step` seconds apart.
    pub frames: Vec<Frame>,
    pub frame_step: f64,
    /// About the pictures: "no video stream", an ffmpeg error, or empty.
    pub video_note: String,
    /// ffprobe's view of the source, plus what was opened: key, value.
    pub meta: Vec<(String, String)>,
    /// The audio itself (16 kHz), for sound at the playhead.
    pub audio: std::sync::Arc<std::sync::Mutex<Vec<i16>>>,
}

impl SourceState {
    pub fn push_audio(&mut self, x: &[f32]) {
        self.audio.lock().unwrap().extend(x.iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16));
        for &v in x {
            let (lo, hi, n) = &mut self.acc;
            if *n == 0 {
                (*lo, *hi) = (v, v);
            } else {
                *lo = lo.min(v);
                *hi = hi.max(v);
            }
            *n += 1;
            if *n == PEAK {
                self.peaks.push([*lo, *hi]);
                *n = 0;
            }
        }
    }

    fn push_frame(&mut self, t: f64, rgb: Vec<u8>) {
        if self.frames.last().is_some_and(|f| t < f.t + self.frame_step) {
            return;
        }
        self.frames.push(Frame { t, rgb });
        if self.frames.len() > MAX_FRAMES {
            let mut k = 0;
            self.frames.retain(|_| {
                k += 1;
                k % 2 == 1
            });
            self.frame_step = self.frame_step.max(1.0) * 2.0;
        }
    }

    /// The frame shown at `t`: the last one at or before it.
    fn frame_at(&self, t: f64) -> Option<(usize, &Frame)> {
        let i = self.frames.partition_point(|f| f.t <= t);
        (i > 0).then(|| (i - 1, &self.frames[i - 1]))
    }
}

/// Metadata and pictures of a source, beside the audio: ffprobe once, then ffmpeg giving one
/// small frame a second while the source has a video stream.
pub fn start_side(ff_input: String, typed: String, live: bool, st: Shared, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut meta = vec![("input".to_string(), typed), ("opened".to_string(), ff_input.clone()), ("live".to_string(), live.to_string())];
        let probe = corroborate::no_window(&mut std::process::Command::new("ffprobe"))
            .args(["-v", "error", "-show_format", "-show_streams", "-of", "flat", &ff_input])
            .output();
        let mut video = false;
        match probe {
            Ok(o) if o.status.success() => {
                for line in String::from_utf8_lossy(&o.stdout).lines() {
                    let Some((k, v)) = line.split_once('=') else { continue };
                    let v = v.trim_matches('"').to_string();
                    let keep = k.starts_with("format.tags.")
                        || ["format.format_long_name", "format.duration", "format.bit_rate"].contains(&k)
                        || ["codec_type", "codec_name", "width", "height", "sample_rate", "channels", "r_frame_rate"]
                            .iter()
                            .any(|f| k.starts_with("streams.stream.") && k.ends_with(&format!(".{f}")));
                    video |= k.ends_with(".codec_type") && v == "video";
                    if keep {
                        meta.push((k.trim_start_matches("streams.").to_string(), v));
                    }
                }
            }
            Ok(o) => meta.push(("ffprobe".into(), String::from_utf8_lossy(&o.stderr).trim().to_string())),
            Err(e) => meta.push(("ffprobe".into(), format!("could not run (is it on PATH?): {e}"))),
        }
        {
            let mut s = st.lock().unwrap();
            s.source.meta = meta;
            if !video {
                s.source.video_note = "no video stream".into();
            }
        }
        if video && !stop.load(Ordering::Relaxed) {
            if let Err(e) = frames(&ff_input, &st, &stop) {
                st.lock().unwrap().source.video_note = e;
            }
        }
    });
}

/// Read one frame a second, FW × FH RGB (letterboxed), into the state until the end or Stop.
fn frames(input: &str, st: &Shared, stop: &AtomicBool) -> Result<(), String> {
    let vf = format!("fps=1,scale={FW}:{FH}:force_original_aspect_ratio=decrease,pad={FW}:{FH}:(ow-iw)/2:(oh-ih)/2");
    let mut child = corroborate::no_window(&mut std::process::Command::new("ffmpeg"))
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i", input, "-an", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start ffmpeg for video: {e}"))?;
    let mut out = child.stdout.take().ok_or("no ffmpeg output")?;
    let mut n = 0u64;
    loop {
        let mut buf = vec![0u8; FW * FH * 3];
        if out.read_exact(&mut buf).is_err() || stop.load(Ordering::Relaxed) {
            break;
        }
        st.lock().unwrap().source.push_frame(n as f64, buf);
        n += 1;
    }
    let _ = child.kill();
    let _ = child.wait();
    if n == 0 && !stop.load(Ordering::Relaxed) {
        return Err("ffmpeg gave no video frames".into());
    }
    Ok(())
}

/// The metadata output in the result JSON.
pub fn json(s: &NodeState) -> String {
    let m: Vec<String> = s.source.meta.iter().map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v))).collect();
    format!(",\"audio_s\":{:.1},\"live\":{},\"metadata\":{{{}}}", s.audio_s, s.live, m.join(","))
}

impl App {
    /// The text a node gives to a text input. No node has a text output yet: a wired text input
    /// says so on the SOURCER.
    fn text_out(&self, _from: NodeId) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------- on the canvas

const WAVE_H: f32 = 64.0;
const VIDEO_H: f32 = 150.0;
const WAVE: Color32 = Color32::from_rgb(224, 164, 88);

/// The SOURCER on the canvas: the path or URL with Browse, the system streams box, its kind,
/// status and clock, the waveform, and for sources with pictures the video viewer (hover the
/// waveform to scrub it).
pub struct Look;

impl crate::look::NodeLook for Look {
    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let z = app.zoom;
        let st = app.nodes[i].state.clone();
        let Kind::Sourcer { input } = &app.nodes[i].kind else { return };
        let mut input = input.clone();
        // A wire into the text input replaces what is typed.
        let fed = app.graph.wires().iter().find(|w| w.to == i && w.to_port == 0).map(|w| w.from);
        let fed_text = fed.and_then(|f| app.text_out(f));
        if let Some(t) = &fed_text {
            input = t.clone();
        }
        let mut refresh = false;
        // The playhead on this source's clock (stronger when its group is the active one).
        let clock = app.clock_of(i).unwrap_or_default();
        let ph = app.playhead_on(&clock).map(|t| (t, app.is_active_clock(&clock)));
        let mut set_ph: Option<f64> = None;
        crate::look::area(ui, r, |ui| {
            ui.add_enabled_ui(fed_text.is_none(), |ui| {
                // Row 1: file, YouTube link or stream URL. The text input overrides this box.
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut input).hint_text("file, YouTube link or stream URL").desired_width(r.width() - 80.0 * z));
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("audio / video", &["mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "mp4", "mkv", "webm", "mov", "avi"])
                            .add_filter("any", &["*"])
                            .pick_file()
                        {
                            input = p.to_string_lossy().into_owned();
                        }
                    }
                });
                // Row 2: system streams. Every audio device ffmpeg lists: microphones, digital and
                // virtual mics, capture cards. Picking one writes `mic:<device>` into the source.
                let shown = match input.trim().strip_prefix(MIC) {
                    Some(d) => d.trim().to_string(),
                    None => "system stream: microphone, capture card…".to_string(),
                };
                let combo = egui::ComboBox::from_id_salt(format!("sys_stream_{i}"))
                    .selected_text(shown)
                    .width(r.width() - 8.0 * z)
                    .show_ui(ui, |ui| {
                        if app.mics.is_empty() {
                            ui.label("no audio devices found");
                        }
                        for m in &app.mics {
                            let current = input.trim().strip_prefix(MIC).is_some_and(|d| d.trim() == m.as_str());
                            if ui.selectable_label(current, m.as_str()).clicked() {
                                input = format!("{MIC}{m}");
                            }
                        }
                    });
                refresh = combo.response.clicked();
            });
            match (fed, &fed_text) {
                (Some(f), None) => crate::look::line(ui, format!("text input: {} gives no text, the field is used", app.nodes[f].label), WARN),
                (Some(f), Some(_)) => crate::look::line(ui, format!("text from {}", app.nodes[f].label), DIM),
                _ => crate::look::line(ui, kind_of(&input), DIM),
            }
            source_info(ui, &st);
            if !input.trim().is_empty() {
                ui.add_space(4.0 * z);
                let (cursor, set) = waveform(ui, &st, z, i, ph);
                set_ph = set;
                if has_video(&input) {
                    ui.add_space(4.0 * z);
                    video(app, ui, i, &st, cursor, z);
                }
            }
        });
        if let Some(t) = set_ph {
            app.set_playhead(&clock, t);
        }
        if refresh {
            match corroborate::ingest::list_microphones() {
                Ok(m) => app.mics = m,
                Err(e) => app.msg = e,
            }
        }
        if let Kind::Sourcer { input: d } = &mut app.nodes[i].kind {
            *d = input;
        }
    }
}

/// The audio so far, min/max per column: the whole recording, or the last 30 s when live.
/// Draws the playhead `ph` (time, active); a click or drag moves it. Returns the time under the
/// pointer and where the playhead was moved to.
fn waveform(ui: &mut egui::Ui, st: &Shared, z: f32, i: NodeId, ph: Option<(f64, bool)>) -> (Option<f64>, Option<f64>) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, WAVE_H * z), Sense::click_and_drag());
    ui.data_mut(|d| d.insert_temp(egui::Id::new(("wave-rect", i)), rect));
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0 * z, BG);
    let s = st.lock().unwrap();
    let peaks = &s.source.peaks;
    if peaks.is_empty() {
        p.text(rect.center(), Align2::CENTER_CENTER, "waveform appears at Run", FontId::proportional(10.5 * z), DIM);
        return (None, None);
    }
    let dt = PEAK as f64 / corroborate::INPUT_SR;
    let first = if s.live { peaks.len().saturating_sub((30.0 / dt) as usize) } else { 0 };
    let shown = &peaks[first..];
    let cols = rect.width().max(1.0) as usize;
    let per = (shown.len() as f32 / cols as f32).max(1.0);
    let gain = shown.iter().map(|q| q[0].abs().max(q[1].abs())).fold(1e-4f32, f32::max);
    let mid = rect.center().y;
    let half = rect.height() * 0.46;
    let mut mesh = egui::epaint::Mesh::default();
    let ncol = ((shown.len() as f32 / per).ceil() as usize).min(cols);
    let colw = rect.width() / (shown.len() as f32 / per).max(1.0);
    for c in 0..ncol {
        let a = (c as f32 * per) as usize;
        let b = (((c + 1) as f32 * per) as usize).clamp(a + 1, shown.len());
        let (lo, hi) = shown[a..b].iter().fold((f32::MAX, f32::MIN), |(l, h), q| (l.min(q[0]), h.max(q[1])));
        let x = rect.left() + c as f32 * colw;
        let r = Rect::from_min_max(Pos2::new(x, mid - hi / gain * half), Pos2::new(x + colw.max(1.0), mid - lo / gain * half + 1.0));
        mesh.add_colored_rect(r, WAVE);
    }
    p.add(egui::Shape::mesh(mesh));
    let t0 = first as f64 * dt;
    let t1 = peaks.len() as f64 * dt;
    let span_text = if s.live { format!("last {:.0} s, live", t1 - t0) } else { format!("0 – {}", fmt_secs(t1)) };
    p.text(rect.left_top() + Vec2::new(4.0, 2.0) * z, Align2::LEFT_TOP, span_text, FontId::monospace(9.5 * z), DIM);
    let len = shown.len().max(1) as f64 * dt;
    let t_at = |x: f32| t0 + ((x - rect.left()) / rect.width()) as f64 * len;
    let set = if resp.clicked() || resp.dragged() { resp.interact_pointer_pos().map(|h| t_at(h.x.clamp(rect.left(), rect.right()))) } else { None };
    if let Some((t, active)) = set.map(|t| (t, true)).or(ph) {
        let x = rect.left() + ((t - t0) / len) as f32 * rect.width();
        if (rect.left()..=rect.right()).contains(&x) {
            let c = Color32::from_rgb(255, 210, 90);
            p.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(if active { 2.0 } else { 1.0 }, if active { c } else { c.gamma_multiply(0.5) }));
        }
    }
    let Some(hover) = resp.hover_pos().filter(|h| rect.contains(*h)) else { return (None, set) };
    let t = t_at(hover.x);
    p.line_segment([Pos2::new(hover.x, rect.top()), Pos2::new(hover.x, rect.bottom())], Stroke::new(1.0, TEXT));
    p.text(Pos2::new(hover.x + 4.0 * z, rect.bottom() - 2.0 * z), Align2::LEFT_BOTTOM, fmt_secs(t), FontId::monospace(9.5 * z), TEXT);
    (Some(t), set)
}

/// The video frame at the waveform's pointer, or the newest one.
fn video(app: &mut App, ui: &mut egui::Ui, i: NodeId, st: &Shared, cursor: Option<f64>, z: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, VIDEO_H * z), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0 * z, Color32::BLACK);
    let s = st.lock().unwrap();
    let pick = match cursor {
        Some(t) => s.source.frame_at(t),
        None => s.source.frames.len().checked_sub(1).map(|k| (k, &s.source.frames[k])),
    };
    let Some((k, f)) = pick else {
        let note = if s.source.video_note.is_empty() { "video appears at Run" } else { s.source.video_note.as_str() };
        p.text(rect.center(), Align2::CENTER_CENTER, note, FontId::proportional(10.5 * z), DIM);
        return;
    };
    // One texture per node, rebuilt when the shown frame changes.
    let stamp = (k as u64) << 32 | f.t as u64;
    let tex = match app.textures.get_mut(&i) {
        Some((old, _, tex)) if *old == stamp => tex.id(),
        _ => {
            let img = ColorImage::from_rgb([FW, FH], &f.rgb);
            let tex = ui.ctx().load_texture(format!("video{i}"), img, TextureOptions::LINEAR);
            let id = tex.id();
            app.textures.insert(i, (stamp, f.t, tex));
            id
        }
    };
    let h = rect.height();
    let fw = (h * FW as f32 / FH as f32).min(rect.width());
    let fr = Rect::from_center_size(rect.center(), Vec2::new(fw, fw * FH as f32 / FW as f32));
    p.image(tex, fr, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    p.text(rect.left_bottom() + Vec2::new(4.0, -2.0) * z, Align2::LEFT_BOTTOM, fmt_secs(f.t), FontId::monospace(9.5 * z), TEXT);
    if !s.source.video_note.is_empty() {
        p.text(rect.right_bottom() + Vec2::new(-4.0, -2.0) * z, Align2::RIGHT_BOTTOM, &s.source.video_note, FontId::proportional(9.5 * z), WARN);
    }
}

/// Seconds as H:MM:SS (or M:SS under an hour).
pub fn fmt_secs(t: f64) -> String {
    let t = t.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}
