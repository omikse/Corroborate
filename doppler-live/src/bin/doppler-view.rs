//! doppler-view: live display of the detector.
//!
//! Waveform, scrolling whitened spectrogram (exactly what the detector searches), a spectrum
//! slice on the same frequency axis, raw ridges, held candidates and reported passes drawn as
//! their fitted Doppler curves.
//!
//! Raw by design:
//! - A recording (file) is analysed as fast as the machine allows, so its whole timeline is
//!   there to scroll at once. Audio plays from a playhead set by clicking the spectrogram.
//! - A live stream shows only the visible window. Press REC to keep the timeline from that
//!   moment (no cap) and write the stream audio to `recordings/rec-<UTC>.wav`.
//!
//! Audio is decoded at 16 kHz; the detector gets a 4 kHz copy. The window only reads shared
//! state written by the worker thread.

use doppler_live::fit::model;
use doppler_live::{ingest, Config, Detector, Event};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, ColorImage, FontFamily, FontId, Id, Key, Pos2, Rect, Sense, Shape, Stroke,
    TextureHandle, TextureOptions,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const USAGE: &str = "\
doppler-view: live view of the Doppler-curve detector

USAGE:
  doppler-view <file-or-url> [--start <s>]
  doppler-view --stdin            (raw f32le mono 16000 Hz on stdin)

  --start <s>   seek into a file before analysing (via ffmpeg)

  Files are analysed at full speed; scroll the whole timeline, click the spectrogram to set
  the playhead, tick [audio] to listen from there.
  Live streams show the current window; press REC to keep the timeline and save a WAV.
  [smooth] buffers live streams 4 s and scrolls between frames. End: back to live / follow.";

const WAVE_S: f64 = 5.0;
/// Decode/playback rate; the detector gets a 4 kHz copy.
const PLAY_SR: f64 = 16_000.0;
/// Seconds of audio held back on live streams (smooth on) to absorb network bursts.
const LIVE_BUFFER_S: f64 = 4.0;
const HOP_S: f64 = 0.25;
/// z range mapped onto one byte of history.
const Z_LO: f32 = -1.0;
const Z_HI: f32 = 6.0;

// Palette: near-black field, phosphor accents.
const BG: Color32 = Color32::from_rgb(8, 9, 11);
const PANEL: Color32 = Color32::from_rgb(13, 15, 18);
const FIELD: Color32 = Color32::from_rgb(4, 5, 8);
const GRID: Color32 = Color32::from_rgb(34, 38, 44);
const DIM: Color32 = Color32::from_rgb(110, 118, 128);
const TEXT: Color32 = Color32::from_rgb(198, 206, 214);
const WAVE: Color32 = Color32::from_rgb(120, 220, 160);
const RIDGE: Color32 = Color32::from_rgb(80, 200, 230);
const HIGH: Color32 = Color32::from_rgb(140, 255, 90);
const MED: Color32 = Color32::from_rgb(255, 190, 60);
const LOW: Color32 = Color32::from_rgb(150, 150, 150);
const REC: Color32 = Color32::from_rgb(255, 70, 70);
const HEAD: Color32 = Color32::from_rgb(240, 240, 240);

struct Shared {
    sr: f64,
    offset: f64,
    source: String,
    freqs: Vec<f64>,
    /// Last WAVE_S seconds of decoded audio at PLAY_SR, ring.
    wave: Vec<f32>,
    wave_head: usize,
    wave_n: u64,
    /// Frames shown across the spectrogram.
    cols: usize,
    /// Spectrogram timeline, one byte per bin (z quantised over Z_LO..Z_HI), and frame times.
    /// Files: everything. Live: the visible window, or everything since REC was pressed.
    hist: Vec<u8>,
    hist_t: Vec<f64>,
    /// Frames ever added (changes whenever the timeline changes).
    frames_total: u64,
    now: f64,
    /// When the latest frame was added: used to scroll between frames.
    frame_wall: Instant,
    ridges: Vec<Vec<(f64, f64)>>,
    pending: Vec<Event>,
    events: Vec<Event>,
    dsp_s: f64,
    audio_s: f64,
    wall_s: f64,
    done: bool,
    live: bool,
    /// Current recording, if any: file and seconds written.
    rec_path: Option<String>,
    rec_s: f64,
    status: String,
    c: f64,
}

fn quantise(z: f32) -> u8 {
    (((z - Z_LO) / (Z_HI - Z_LO)).clamp(0.0, 1.0) * 255.0).round() as u8
}

fn dequantise(q: u8) -> f32 {
    Z_LO + q as f32 / 255.0 * (Z_HI - Z_LO)
}

fn grade_color(g: &str) -> Color32 {
    match g {
        "high" => HIGH,
        "medium" => MED,
        _ => LOW,
    }
}

/// Byte â†’ colour: black, deep blue, violet, red, orange, pale yellow.
fn colormap_lut() -> [[u8; 4]; 256] {
    const STOPS: [(f32, [f32; 3]); 6] = [
        (0.00, [4.0, 5.0, 8.0]),
        (0.22, [18.0, 22.0, 60.0]),
        (0.45, [90.0, 30.0, 120.0]),
        (0.65, [200.0, 60.0, 70.0]),
        (0.85, [250.0, 160.0, 40.0]),
        (1.00, [255.0, 250.0, 200.0]),
    ];
    let mut lut = [[0u8; 4]; 256];
    for (q, px) in lut.iter_mut().enumerate() {
        let x = q as f32 / 255.0;
        let mut i = 0;
        while i + 1 < STOPS.len() - 1 && x > STOPS[i + 1].0 {
            i += 1;
        }
        let (a, ca) = STOPS[i];
        let (b, cb) = STOPS[i + 1];
        let u = ((x - a) / (b - a)).clamp(0.0, 1.0);
        *px = [
            (ca[0] + u * (cb[0] - ca[0])) as u8,
            (ca[1] + u * (cb[1] - ca[1])) as u8,
            (ca[2] + u * (cb[2] - ca[2])) as u8,
            255,
        ];
    }
    lut
}

fn fmt_clock(t: f64) -> String {
    let t = t.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}

/// UTC timestamp `YYYYMMDD-HHMMSS` for file names (civil-from-days, no date crate).
fn utc_stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, (rem / 60) % 60, rem % 60)
}

// ---------------------------------------------------------------- worker

/// Switches the window flips while the worker runs.
struct Controls {
    smooth: AtomicBool,
    /// Live only: feed the speakers from the stream.
    audio: AtomicBool,
    /// Live only: keep the timeline and write a WAV.
    record: AtomicBool,
}

/// Samples at PLAY_SR waiting for the audio output.
type Ring = Arc<Mutex<VecDeque<f32>>>;

#[derive(Clone)]
enum Source {
    Wav(String),
    Ffmpeg(String, Option<f64>),
    Stdin,
}

impl Source {
    fn is_live(&self) -> bool {
        match self {
            Source::Stdin => true,
            Source::Ffmpeg(p, _) => p.contains("://"),
            Source::Wav(_) => false,
        }
    }
}

type WavOut = hound::WavWriter<std::io::BufWriter<std::fs::File>>;

fn open_recording() -> Result<(WavOut, String), String> {
    std::fs::create_dir_all("recordings").map_err(|e| e.to_string())?;
    let path = format!("recordings/rec-{}.wav", utc_stamp());
    let spec = hound::WavSpec { channels: 1, sample_rate: PLAY_SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let w = hound::WavWriter::create(&path, spec).map_err(|e| format!("{path}: {e}"))?;
    Ok((w, path))
}

fn worker(sh: Arc<Mutex<Shared>>, src: Source, ctl: Arc<Controls>, ring: Ring) {
    let cfg = Config::default();
    let sr = cfg.sr;
    let c = cfg.c;
    let mut det = Detector::new(cfg);
    let mut rs = ingest::Resampler::new(PLAY_SR, sr);
    let mut det_in: Vec<f32> = Vec::with_capacity(1024);
    let mut n: u64 = 0;
    let live = src.is_live();
    let t0 = Instant::now();
    // Live + smooth: paced behind a jitter buffer. Live raw: processed as it arrives.
    // Files: as fast as possible, so the whole timeline is available.
    let mut base: Option<Instant> = None;
    let mut rec: Option<WavOut> = None;

    let mut handle = |x: &[f32], det: &mut Detector| {
        for piece in x.chunks((PLAY_SR / 20.0) as usize) {
            if live && ctl.smooth.load(Ordering::Relaxed) {
                let pos = Duration::from_secs_f64(n as f64 / PLAY_SR);
                let buffer = Duration::from_secs_f64(LIVE_BUFFER_S);
                // Anchor so the current sample is due one buffer from now.
                let b = *base.get_or_insert_with(|| {
                    let t = Instant::now() + buffer;
                    t.checked_sub(pos).unwrap_or(t)
                });
                let due = b + pos;
                let now = Instant::now();
                if due > now {
                    thread::sleep(due - now);
                } else if now - due > 2 * buffer {
                    // Stalled longer than the buffer: re-anchor rather than race to catch up.
                    base = None;
                }
            } else {
                base = None;
            }
            if live && ctl.audio.load(Ordering::Relaxed) {
                let mut q = ring.lock().unwrap();
                q.extend(piece.iter().copied());
                let cap = (PLAY_SR * 10.0) as usize;
                if q.len() > cap {
                    let over = q.len() - cap;
                    q.drain(..over);
                }
            }

            // Recording (live): open on REC, write every sample, close when REC is released.
            let recording = live && ctl.record.load(Ordering::Relaxed);
            let mut rec_event: Option<Result<String, String>> = None;
            if recording && rec.is_none() {
                match open_recording() {
                    Ok((w, p)) => {
                        rec = Some(w);
                        rec_event = Some(Ok(p));
                    }
                    Err(e) => {
                        ctl.record.store(false, Ordering::Relaxed);
                        rec_event = Some(Err(e));
                    }
                }
            }
            if let Some(w) = rec.as_mut() {
                if recording {
                    for &v in piece {
                        let _ = w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16);
                    }
                }
            }
            if !recording {
                if let Some(w) = rec.take() {
                    let _ = w.finalize();
                }
            }

            let tick = Instant::now();
            det_in.clear();
            rs.process(piece, &mut det_in);
            let mut frames: Vec<(f64, Vec<u8>)> = Vec::new();
            let evs = det.push_with(&det_in, |t, z| frames.push((t, z.iter().map(|&v| quantise(v)).collect())));
            let cost = tick.elapsed().as_secs_f64();
            n += piece.len() as u64;

            let mut s = sh.lock().unwrap();
            match rec_event {
                Some(Ok(p)) => {
                    s.rec_path = Some(p);
                    s.rec_s = 0.0;
                }
                Some(Err(e)) => s.status = format!("record: {e}"),
                None => {}
            }
            if recording {
                s.rec_s += piece.len() as f64 / PLAY_SR;
            } else {
                s.rec_path = None;
            }
            let wl = s.wave.len();
            for &v in piece {
                let h = s.wave_head;
                s.wave[h] = v;
                s.wave_head = (h + 1) % wl;
            }
            s.wave_n += piece.len() as u64;
            let nb = s.freqs.len();
            for (t, q) in frames {
                s.hist.extend_from_slice(&q);
                s.hist_t.push(t);
                s.frames_total += 1;
                s.now = t;
                s.frame_wall = Instant::now();
                s.ridges = det.ridges().to_vec();
                s.pending = det.pending().cloned().collect();
            }
            // Live and not recording: keep only the visible window (trimmed in batches).
            if live && !recording && s.hist_t.len() > 2 * s.cols {
                let k = s.hist_t.len() - s.cols;
                s.hist.drain(..k * nb);
                s.hist_t.drain(..k);
            }
            for e in evs {
                println!("{}", e.to_json(None));
                s.events.push(e);
            }
            s.dsp_s += cost;
            s.audio_s = n as f64 / PLAY_SR;
            s.wall_s = t0.elapsed().as_secs_f64();
            s.c = c;
        }
    };

    let r = match &src {
        Source::Wav(p) => ingest::read_wav(p, PLAY_SR, |x| handle(x, &mut det)),
        Source::Ffmpeg(p, start) => match ingest::spawn_ffmpeg_from(p, PLAY_SR as u32, *start) {
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
            for x in ingest::spawn_stdin((PLAY_SR / 20.0) as usize) {
                handle(&x, &mut det);
            }
            Ok(())
        }
    };
    if let Some(w) = rec.take() {
        let _ = w.finalize();
    }
    let evs = det.flush();
    let mut s = sh.lock().unwrap();
    for e in evs {
        println!("{}", e.to_json(None));
        s.events.push(e);
    }
    s.pending.clear();
    s.done = true;
    s.status = match r {
        Ok(()) if live => "stream ended".into(),
        Ok(()) => format!("analysed in {:.1} s", s.wall_s),
        Err(e) => format!("error: {e}"),
    };
}

// ---------------------------------------------------------------- audio out

/// Plays the ring on the default output device at PLAY_SR. When the ring runs dry the output
/// is silent, so gaps in the input are heard as gaps. `played` counts consumed samples.
struct Player {
    _stream: cpal::Stream,
}

fn start_player(ring: Ring, played: Arc<AtomicU64>) -> Result<Player, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let dev = cpal::default_host().default_output_device().ok_or("no audio output device")?;
    let sup = dev.default_output_config().map_err(|e| e.to_string())?;
    let ch = sup.channels() as usize;
    let step = PLAY_SR / sup.sample_rate() as f64;
    let cfg = sup.config();
    let err = |e| eprintln!("audio output: {e}");
    let stream = match sup.sample_format() {
        cpal::SampleFormat::F32 => dev.build_output_stream::<f32, _, _>(cfg, render(ring, played, ch, step, |v| v), err, None),
        cpal::SampleFormat::I16 => dev.build_output_stream::<i16, _, _>(
            cfg,
            render(ring, played, ch, step, |v| (v * 32767.0) as i16),
            err,
            None,
        ),
        cpal::SampleFormat::U16 => dev.build_output_stream::<u16, _, _>(
            cfg,
            render(ring, played, ch, step, |v| ((v * 32767.0) + 32768.0) as u16),
            err,
            None,
        ),
        f => return Err(format!("unsupported output sample format {f:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Player { _stream: stream })
}

/// Output callback: linear interpolation from the ring to the device rate, same value on
/// every channel.
fn render<T: Copy>(
    ring: Ring,
    played: Arc<AtomicU64>,
    ch: usize,
    step: f64,
    conv: impl Fn(f32) -> T + Send + 'static,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static {
    let (mut pos, mut a, mut b) = (0.0f64, 0.0f32, 0.0f32);
    move |data: &mut [T], _| {
        let mut q = ring.lock().unwrap();
        let mut took = 0u64;
        for frame in data.chunks_mut(ch) {
            pos += step;
            while pos >= 1.0 {
                pos -= 1.0;
                a = b;
                b = match q.pop_front() {
                    Some(v) => {
                        took += 1;
                        v
                    }
                    None => 0.0,
                };
            }
            let v = conv((a + (b - a) * pos as f32).clamp(-1.0, 1.0));
            for s in frame.iter_mut() {
                *s = v;
            }
        }
        played.fetch_add(took, Ordering::Relaxed);
    }
}

/// Files: decode from `from_s` (stream time) into the ring, keeping about one second queued.
fn spawn_file_audio(path: String, from_s: f64, ring: Ring, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let Ok((mut child, rx)) = ingest::spawn_ffmpeg_from(&path, PLAY_SR as u32, Some(from_s.max(0.0))) else {
            return;
        };
        'outer: for x in rx {
            loop {
                if stop.load(Ordering::Relaxed) {
                    break 'outer;
                }
                if ring.lock().unwrap().len() < PLAY_SR as usize {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
            ring.lock().unwrap().extend(x);
        }
        let _ = child.kill();
        let _ = child.wait();
    });
}

// ---------------------------------------------------------------- app

struct App {
    sh: Arc<Mutex<Shared>>,
    ctl: Arc<Controls>,
    ring: Ring,
    played: Arc<AtomicU64>,
    /// File path (for playback), or None for live sources.
    file: Option<String>,
    player: Option<Player>,
    /// Files: playback feeder's stop flag.
    feeder: Option<Arc<AtomicBool>>,
    smooth: bool,
    audio: bool,
    record: bool,
    audio_err: Option<String>,
    /// Files: playhead (s from the analysed start) where playback started.
    play_from: f64,
    /// Following live / the playhead, or parked with this time at the right edge.
    follow: bool,
    view_right: f64,
    selected: Option<u64>,
    lut: [[u8; 4]; 256],
    tex: Option<TextureHandle>,
    /// (frames_total, right-edge frame index) the texture was built for.
    tex_key: (u64, usize),
    img: Vec<u8>,
}

impl App {
    fn new(cc: &eframe::CreationContext, sh: Arc<Mutex<Shared>>, ctl: Arc<Controls>, ring: Ring, file: Option<String>) -> Self {
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
        let smooth = ctl.smooth.load(Ordering::Relaxed);
        Self {
            sh,
            ctl,
            ring,
            played: Arc::new(AtomicU64::new(0)),
            file,
            player: None,
            feeder: None,
            smooth,
            audio: false,
            record: false,
            audio_err: None,
            play_from: 0.0,
            follow: true,
            view_right: 0.0,
            selected: None,
            lut: colormap_lut(),
            tex: None,
            tex_key: (u64::MAX, usize::MAX),
            img: Vec::new(),
        }
    }

    /// Files: current playhead in stream seconds (from the analysed start).
    fn playhead(&self) -> f64 {
        let played = self.played.load(Ordering::Relaxed) as f64 / PLAY_SR;
        self.play_from + if self.audio { played } else { 0.0 }
    }

    fn stop_audio(&mut self) {
        if let Some(f) = self.feeder.take() {
            f.store(true, Ordering::Relaxed);
        }
        self.ctl.audio.store(false, Ordering::Relaxed);
        self.player = None;
        self.ring.lock().unwrap().clear();
    }

    fn start_audio(&mut self, offset: f64) -> Result<(), String> {
        self.ring.lock().unwrap().clear();
        self.played.store(0, Ordering::Relaxed);
        if let Some(path) = self.file.clone() {
            let stop = Arc::new(AtomicBool::new(false));
            spawn_file_audio(path, offset + self.play_from, self.ring.clone(), stop.clone());
            self.feeder = Some(stop);
        } else {
            self.ctl.audio.store(true, Ordering::Relaxed);
        }
        if self.player.is_none() {
            self.player = Some(start_player(self.ring.clone(), self.played.clone())?);
        }
        Ok(())
    }

    /// Files: move the playhead; restart playback there if audio is on.
    fn seek(&mut self, t: f64, offset: f64) {
        self.play_from = t.max(0.0);
        if self.audio {
            if let Some(f) = self.feeder.take() {
                f.store(true, Ordering::Relaxed);
            }
            if let Err(e) = self.start_audio(offset) {
                self.audio_err = Some(e);
            }
        }
    }

    fn apply_toggles(&mut self, smooth: bool, audio: bool, record: bool, offset: f64) {
        if smooth != self.smooth {
            self.smooth = smooth;
            self.ctl.smooth.store(smooth, Ordering::Relaxed);
        }
        if record != self.record {
            self.record = record;
            self.ctl.record.store(record, Ordering::Relaxed);
        }
        if audio != self.audio {
            if audio {
                match self.start_audio(offset) {
                    Ok(()) => {
                        self.audio = true;
                        self.audio_err = None;
                    }
                    Err(e) => {
                        self.stop_audio();
                        self.audio_err = Some(e);
                    }
                }
            } else {
                // Keep the playhead where playback got to.
                self.play_from = self.playhead();
                self.stop_audio();
                self.audio = false;
            }
        }
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
        let span = s.cols as f64 * HOP_S;
        let t_first = s.hist_t.first().copied().unwrap_or(0.0);
        let is_file = self.file.is_some();
        // Record only keeps the timeline for live sources; recording state comes back from the worker.
        if s.live && self.record && s.rec_path.is_none() && !s.status.is_empty() && s.status.starts_with("record:") {
            self.record = false;
        }

        if ctx.input(|i| i.key_pressed(Key::End)) {
            self.follow = true;
        }

        // View: right-edge time.
        let head = self.playhead().min(s.now);
        let t_right = if self.follow {
            if is_file && self.audio {
                // Follow the playhead, keeping it three quarters across.
                (head + span * 0.25).min(s.now).max((t_first + span).min(s.now))
            } else {
                let frac = if self.smooth && !s.done && s.live {
                    (s.frame_wall.elapsed().as_secs_f64() / HOP_S).min(1.0)
                } else {
                    0.0
                };
                s.now + frac * HOP_S
            }
        } else {
            self.view_right.clamp((t_first + span).min(s.now), s.now)
        };
        let t_left = t_right - span;

        // Frames covering the view: those with time â‰¤ t_right, the last `cols` of them.
        let idx_r = s.hist_t.partition_point(|&t| t <= t_right);
        let key = (if self.follow { s.frames_total } else { 0 }, idx_r);
        if key != self.tex_key || self.tex.is_none() {
            self.tex_key = key;
            let (w, h) = (s.cols, nb);
            self.img.resize(w * h * 4, 0);
            for col in 0..w {
                let fi = idx_r as isize - w as isize + col as isize;
                for row in 0..h {
                    let bin = h - 1 - row;
                    let px = if fi >= 0 { self.lut[s.hist[fi as usize * nb + bin] as usize] } else { [4, 5, 8, 255] };
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
        let t_img = if idx_r > 0 { s.hist_t[idx_r - 1] } else { t_right };

        // ------------------------------------------------ top bar
        let (mut smooth, mut audio, mut record) = (self.smooth, self.audio, self.record);
        let audio_err = self.audio_err.clone();
        egui::Panel::top("top").frame(egui::Frame::new().fill(BG).inner_margin(8.0)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("DOPPLER-LIVE").font(mono(15.0)).color(HIGH).strong());
                ui.add_space(12.0);
                ui.checkbox(&mut smooth, egui::RichText::new("smooth").font(mono(12.0))).on_hover_text(
                    "On: live streams play out through a 4 s buffer and the view scrolls between 0.25 s frames.\n\
                     Off (raw): data is shown exactly as it arrives, bursts and steps included.",
                );
                ui.checkbox(&mut audio, egui::RichText::new("audio").font(mono(12.0))).on_hover_text(if is_file {
                    "Play from the playhead (click the spectrogram to set it)."
                } else {
                    "Play the stream as received."
                });
                if s.live {
                    let label = if record {
                        egui::RichText::new(format!("â— REC {}", fmt_clock(s.rec_s))).font(mono(12.0)).color(REC).strong()
                    } else {
                        egui::RichText::new("â—‹ REC").font(mono(12.0)).color(DIM)
                    };
                    ui.toggle_value(&mut record, label).on_hover_text(
                        "Keep the timeline from now (scroll back, no limit) and save the stream audio to recordings/",
                    );
                    if let Some(p) = &s.rec_path {
                        ui.label(egui::RichText::new(format!("â†’ {p}")).font(mono(11.0)).color(DIM));
                    }
                }
                if let Some(e) = &audio_err {
                    ui.label(egui::RichText::new(format!("audio: {e}")).font(mono(11.0)).color(MED));
                }
                ui.add_space(12.0);
                ui.label(egui::RichText::new("â–¸").color(DIM));
                ui.label(egui::RichText::new(&s.source).color(DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let high = s.events.iter().filter(|e| e.grade() == "high").count();
                    let state = if s.done {
                        s.status.clone()
                    } else if s.live {
                        "â— LIVE".into()
                    } else {
                        format!("â–¶ ANALYSING {:.0}Ã— real time", s.audio_s / s.wall_s.max(1e-9))
                    };
                    ui.label(egui::RichText::new(state).color(if s.done { DIM } else { MED }));
                    let load = if s.live {
                        format!("DSP {:5.2}%", 100.0 * s.dsp_s / s.audio_s.max(1e-9))
                    } else {
                        format!("AUDIO {}", fmt_clock(s.audio_s))
                    };
                    ui.label(egui::RichText::new(format!(
                        "T {}   {load}   PASSES {} ({} high)",
                        fmt_clock(s.offset + if is_file { head } else { s.now }),
                        s.events.len(),
                        high
                    )));
                });
            });
        });

        // ------------------------------------------------ bottom status
        egui::Panel::bottom("bottom").frame(egui::Frame::new().fill(BG).inner_margin(6.0)).show(ui, |ui| {
            let df = s.freqs[1] - s.freqs[0];
            let hist = if is_file || self.record { fmt_clock(s.now - t_first) } else { "window only (REC to keep)".into() };
            ui.label(
                egui::RichText::new(format!(
                    "detector {:.0} Hz  â”‚  band {:.0}â€“{:.0} Hz  â”‚  Î”f {:.2} Hz  â”‚  hop 0.25 s  â”‚  view {:.0} s  â”‚  timeline {hist}  â”‚  \
                     ridge â”„ cyan  â”‚  candidate â”„ white  â”‚  pass: high / medium / low  â”‚  wheel/bar: scroll  End: {}",
                    s.sr,
                    fmin,
                    fmax,
                    df,
                    span,
                    if is_file { "follow" } else { "live" }
                ))
                .font(mono(11.0))
                .color(DIM),
            );
        });

        // ------------------------------------------------ event list
        let mut snap: Option<(u64, f64)> = None;
        egui::Panel::right("events").exact_size(330.0).frame(egui::Frame::new().fill(PANEL).inner_margin(10.0)).show(ui, |ui| {
            ui.label(egui::RichText::new("DETECTED PASSES").font(mono(13.0)).color(TEXT).strong());
            ui.label(egui::RichText::new("click a pass to jump to it").font(mono(10.5)).color(DIM));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(" #  T_CPA     f0     v     d    grade").font(mono(11.0)).color(DIM));
            ui.separator();
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for e in &s.events {
                    let sel = self.selected == Some(e.id);
                    let line = format!(
                        "{}{:>2}  {}  {:5.1}  {:3.0}  {:4.1}k  {}",
                        if sel { "â–¸" } else { " " },
                        e.id,
                        fmt_clock(s.offset + e.t_cpa),
                        e.f0,
                        e.speed,
                        e.dist / 1000.0,
                        e.grade()
                    );
                    let detail = format!("    {:.0}â†’{:.0} Hz  snr {:.2}  rms {:.2}  h{}", e.f_pre, e.f_post, e.snr, e.rms, e.harmonics);
                    let r = ui
                        .vertical(|ui| {
                            ui.label(egui::RichText::new(line).font(mono(12.0)).color(grade_color(e.grade())));
                            ui.label(egui::RichText::new(detail).font(mono(10.5)).color(DIM));
                        })
                        .response
                        .interact(Sense::click());
                    if r.hovered() {
                        ui.painter().rect_stroke(r.rect.expand(2.0), 2.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);
                    }
                    if r.clicked() {
                        snap = Some((e.id, e.t_cpa));
                    }
                }
                if s.events.is_empty() {
                    ui.label(egui::RichText::new(if s.done { "no passes found" } else { "listeningâ€¦" }).color(DIM));
                }
            });
        });

        // ------------------------------------------------ main area
        let mut new_view: Option<f64> = None;
        let mut seek_to: Option<f64> = None;
        let mut go_live = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(10.0)).show(ui, |ui| {
            let avail = ui.available_size();
            let wave_h = (avail.y * 0.16).clamp(70.0, 140.0);

            let (wrect, _) = ui.allocate_exact_size(vec2(avail.x, wave_h), Sense::hover());
            draw_wave(ui, wrect, &s, is_file && !self.audio);
            ui.add_space(8.0);

            let rest = ui.available_size();
            let slice_w = 120.0;
            let axis_l = 52.0;
            let axis_b = 20.0;
            let bar_h = 16.0;
            let (area, _) = ui.allocate_exact_size(rest, Sense::hover());
            let spec_rect = Rect::from_min_max(
                pos2(area.left() + axis_l, area.top()),
                pos2(area.right() - slice_w - 10.0, area.bottom() - axis_b - bar_h - 8.0),
            );
            let slice_rect = Rect::from_min_max(pos2(spec_rect.right() + 10.0, spec_rect.top()), pos2(area.right(), spec_rect.bottom()));
            let bar_rect = Rect::from_min_max(pos2(spec_rect.left(), area.bottom() - bar_h), pos2(spec_rect.right(), area.bottom()));
            let live_rect = Rect::from_min_max(pos2(slice_rect.left(), bar_rect.top()), pos2(slice_rect.right(), bar_rect.bottom()));

            let x_of = |t: f64| spec_rect.left() + ((t - t_left) / span) as f32 * spec_rect.width();
            let t_of = |x: f32| t_left + ((x - spec_rect.left()) / spec_rect.width()) as f64 * span;
            let y_of = |f: f64| spec_rect.bottom() - ((f - fmin) / (fmax - fmin)) as f32 * spec_rect.height();

            // Spectrogram: wheel pans in time; click (files) sets the playhead.
            let spec_resp = ui.interact(spec_rect, Id::unique("spec"), Sense::click());
            if spec_resp.hovered() {
                let d = ui.input(|i| i.smooth_scroll_delta);
                let dpx = if d.x.abs() > d.y.abs() { d.x } else { d.y };
                if dpx != 0.0 {
                    new_view = Some(t_right + dpx as f64 / spec_rect.width() as f64 * span);
                }
            }
            if is_file && spec_resp.clicked() {
                if let Some(pp) = spec_resp.interact_pointer_pos() {
                    seek_to = Some(t_of(pp.x).clamp(t_first, s.now));
                }
            }

            let p = ui.painter();
            p.rect_filled(spec_rect, 0.0, FIELD);
            if let Some(t) = &self.tex {
                let img_rect = Rect::from_min_max(pos2(x_of(t_img - span + HOP_S), spec_rect.top()), pos2(x_of(t_img + HOP_S), spec_rect.bottom()));
                p.with_clip_rect(spec_rect).image(t.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }

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

            // Time grid on absolute 20 s marks, so it moves with the data; labels are stream time.
            let mut tg = (t_left / 20.0).ceil() * 20.0;
            while tg <= t_right {
                let x = x_of(tg);
                p.line_segment([pos2(x, spec_rect.top()), pos2(x, spec_rect.bottom())], Stroke::new(0.5, GRID.gamma_multiply(0.6)));
                if x > spec_rect.left() + 24.0 && x < spec_rect.right() - 40.0 {
                    p.text(pos2(x, spec_rect.bottom() + 4.0), Align2::CENTER_TOP, fmt_clock(s.offset + tg), mono(10.5), DIM);
                }
                tg += 20.0;
            }
            let at_edge = (s.now - t_right).abs() < HOP_S * 1.5;
            let edge = if at_edge { "now".to_string() } else { format!("{} â—‚", fmt_clock(s.offset + t_right)) };
            p.text(pos2(spec_rect.right(), spec_rect.bottom() + 4.0), Align2::RIGHT_TOP, edge, mono(10.5), if at_edge { HIGH } else { MED });

            let clip = p.with_clip_rect(spec_rect);
            if at_edge && !s.done {
                // Raw ridges and held candidates belong to the latest search.
                for r in &s.ridges {
                    let pts: Vec<Pos2> = r.iter().step_by(2).map(|&(t, f)| pos2(x_of(t), y_of(f))).collect();
                    if pts.len() > 1 {
                        clip.extend(Shape::dashed_line(&pts, Stroke::new(1.0, RIDGE.gamma_multiply(0.7)), 3.0, 3.0));
                    }
                }
                for e in &s.pending {
                    let pts = curve(e, s.c, t_left, t_right, &x_of, &y_of);
                    clip.extend(Shape::dashed_line(&pts, Stroke::new(1.5, Color32::WHITE), 6.0, 4.0));
                }
            }
            for e in &s.events {
                if e.t_end + 40.0 < t_left || e.t_start - 40.0 > t_right {
                    continue;
                }
                let col = grade_color(e.grade());
                let w = if self.selected == Some(e.id) { 3.0 } else { 2.0 };
                let pts = curve(e, s.c, t_left, t_right, &x_of, &y_of);
                if pts.len() > 1 {
                    clip.add(Shape::line(pts, Stroke::new(w, col)));
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
            // Playhead (files).
            if is_file && head >= t_left && head <= t_right {
                let x = x_of(head);
                clip.line_segment([pos2(x, spec_rect.top()), pos2(x, spec_rect.bottom())], Stroke::new(1.0, HEAD));
                clip.text(pos2(x + 4.0, spec_rect.top() + 2.0), Align2::LEFT_TOP, if self.audio { "â–¶" } else { "â–®" }, mono(11.0), HEAD);
            }
            p.rect_stroke(spec_rect, 0.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);

            // Spectrum slice: the frame at the playhead (files) or right edge.
            p.rect_filled(slice_rect, 0.0, FIELD);
            let slice_t = if is_file { head } else { t_right };
            let si = s.hist_t.partition_point(|&t| t <= slice_t);
            if si > 0 {
                let row = &s.hist[(si - 1) * nb..si * nb];
                let x_of_z = |z: f32| slice_rect.left() + ((z - Z_LO) / (Z_HI - Z_LO + 1.0)).clamp(0.0, 1.0) * slice_rect.width();
                let pts: Vec<Pos2> = row.iter().enumerate().step_by(2).map(|(i, &q)| pos2(x_of_z(dequantise(q)), y_of(s.freqs[i]))).collect();
                p.with_clip_rect(slice_rect).add(Shape::line(pts, Stroke::new(1.0, WAVE)));
                let x0 = x_of_z(0.0);
                p.line_segment([pos2(x0, slice_rect.top()), pos2(x0, slice_rect.bottom())], Stroke::new(0.5, DIM));
            }
            p.rect_stroke(slice_rect, 0.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);
            p.text(
                pos2(slice_rect.center().x, slice_rect.bottom() + 4.0),
                Align2::CENTER_TOP,
                if is_file { "z at â–®" } else if at_edge { "z now" } else { "z at edge" },
                mono(10.5),
                DIM,
            );

            // Timeline bar: everything kept; the thumb is the visible window. Click or drag to move.
            let total = (s.now - t_first).max(span);
            let bx = |t: f64| bar_rect.left() + ((t - t_first) / total) as f32 * bar_rect.width();
            let bar = ui.interact(bar_rect, Id::unique("timeline"), Sense::click_and_drag());
            p.rect_filled(bar_rect, 2.0, FIELD);
            for e in &s.events {
                if e.t_cpa >= t_first {
                    let x = bx(e.t_cpa);
                    p.line_segment([pos2(x, bar_rect.top() + 2.0), pos2(x, bar_rect.bottom() - 2.0)], Stroke::new(2.0, grade_color(e.grade())));
                }
            }
            if is_file {
                let x = bx(head);
                p.line_segment([pos2(x, bar_rect.top()), pos2(x, bar_rect.bottom())], Stroke::new(1.0, HEAD));
            }
            let thumb = Rect::from_min_max(pos2(bx(t_left).max(bar_rect.left()), bar_rect.top()), pos2(bx(t_right).min(bar_rect.right()), bar_rect.bottom()));
            p.rect_stroke(thumb, 2.0, Stroke::new(1.5, if self.follow { HIGH } else { MED }), egui::StrokeKind::Inside);
            p.rect_stroke(bar_rect, 2.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);
            if let Some(pp) = bar.interact_pointer_pos() {
                if bar.clicked() || bar.dragged() {
                    let u = ((pp.x - bar_rect.left()) / bar_rect.width()).clamp(0.0, 1.0) as f64;
                    new_view = Some(t_first + u * total + span / 2.0);
                }
            }
            p.text(pos2(bar_rect.left() - 6.0, bar_rect.center().y), Align2::RIGHT_CENTER, "time", mono(10.0), DIM);

            // LIVE / FOLLOW button.
            let live_btn = ui.interact(live_rect, Id::unique("live"), Sense::click());
            let lc = if self.follow { HIGH } else if live_btn.hovered() { TEXT } else { MED };
            p.rect_stroke(live_rect, 2.0, Stroke::new(1.0, lc), egui::StrokeKind::Inside);
            let lbl = match (is_file, self.follow) {
                (false, true) => "â— LIVE",
                (false, false) => "âµ GO LIVE",
                (true, true) => "â— FOLLOW",
                (true, false) => "âµ FOLLOW",
            };
            p.text(live_rect.center(), Align2::CENTER_CENTER, lbl, mono(11.0), lc);
            if live_btn.clicked() {
                go_live = true;
            }
        });
        let (now, offset) = (s.now, s.offset);
        drop(s);

        if let Some((id, tc)) = snap {
            // Closest approach a little left of centre, so the approach and the fall both show.
            self.selected = Some(id);
            self.follow = false;
            self.view_right = tc + span * 0.6;
            if is_file {
                // Start listening a little before the closest approach.
                self.seek(tc - 20.0, offset);
            }
        }
        if let Some(t) = seek_to {
            self.seek(t, offset);
        }
        if go_live {
            self.follow = true;
        } else if let Some(v) = new_view {
            if v >= now - HOP_S {
                self.follow = true;
            } else {
                self.follow = false;
                self.view_right = v;
            }
        }
        self.apply_toggles(smooth, audio, record, offset);
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

/// Waveform of the most recent audio: what is being decoded (live) or analysed (files).
fn draw_wave(ui: &egui::Ui, rect: Rect, s: &Shared, analysis: bool) {
    let p = ui.painter();
    p.rect_filled(rect, 0.0, FIELD);
    let mid = rect.center().y;
    p.line_segment([pos2(rect.left(), mid), pos2(rect.right(), mid)], Stroke::new(0.5, GRID));
    let wl = s.wave.len();
    let n = (s.wave_n as usize).min(wl);
    if n > 0 {
        let at = |i: usize| s.wave[(s.wave_head + wl - n + i) % wl];
        let mut peak = 1e-6f32;
        for i in 0..n {
            peak = peak.max(at(i).abs());
        }
        let cols = rect.width().max(1.0) as usize;
        let per = (wl as f64 / cols as f64).max(1.0);
        let off = wl - n;
        let h = rect.height() * 0.46;
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
            shapes.push(Shape::line_segment([pos2(x, mid - hi / peak * h), pos2(x, mid - lo / peak * h)], Stroke::new(1.0, WAVE)));
        }
        p.extend(shapes);
        let rms = (0..n).map(|i| at(i) * at(i)).sum::<f32>() / n as f32;
        p.text(
            rect.left_top() + vec2(6.0, 4.0),
            Align2::LEFT_TOP,
            format!(
                "WAVEFORM  {}last {WAVE_S:.0} s @ {:.0} Hz   peak {:.3}   rms {:.1} dBFS   (auto-scaled)",
                if analysis { "analysis head Â· " } else { "" },
                PLAY_SR,
                peak,
                10.0 * rms.max(1e-12).log10()
            ),
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
    let mut stdin = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--start" => start = args.next().and_then(|v| v.parse().ok()),
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
            if name.chars().count() > 60 { format!("{}â€¦", name.chars().take(60).collect::<String>()) } else { name.to_string() }
        }
        Source::Stdin => "stdin".into(),
    };
    let live = src.is_live();
    let file = match &src {
        Source::Wav(p) | Source::Ffmpeg(p, _) if !live => Some(p.clone()),
        _ => None,
    };

    let cfg = Config::default();
    let freqs = Detector::new(cfg.clone()).bin_freqs();
    let cols = (cfg.window_s * cfg.sr / cfg.hop as f64) as usize;
    let sh = Arc::new(Mutex::new(Shared {
        sr: cfg.sr,
        offset: start.unwrap_or(0.0),
        source: label,
        freqs,
        wave: vec![0.0; (WAVE_S * PLAY_SR) as usize],
        wave_head: 0,
        wave_n: 0,
        cols,
        hist: Vec::new(),
        hist_t: Vec::new(),
        frames_total: 0,
        now: 0.0,
        frame_wall: Instant::now(),
        ridges: Vec::new(),
        pending: Vec::new(),
        events: Vec::new(),
        dsp_s: 0.0,
        audio_s: 0.0,
        wall_s: 0.0,
        done: false,
        live,
        rec_path: None,
        rec_s: 0.0,
        status: String::new(),
        c: cfg.c,
    }));
    let ctl = Arc::new(Controls { smooth: AtomicBool::new(true), audio: AtomicBool::new(false), record: AtomicBool::new(false) });
    let ring: Ring = Arc::new(Mutex::new(VecDeque::new()));

    {
        let (sh, ctl, ring) = (sh.clone(), ctl.clone(), ring.clone());
        thread::spawn(move || worker(sh, src, ctl, ring));
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1480.0, 920.0]).with_title("doppler-view"),
        ..Default::default()
    };
    eframe::run_native("doppler-view", options, Box::new(move |cc| Ok(Box::new(App::new(cc, sh, ctl, ring, file)))))
        .map_err(|e| e.to_string())
}
