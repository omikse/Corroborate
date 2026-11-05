// Release builds are a Windows GUI program: no console window. Debug builds keep the
// console for diagnostics.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
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
use doppler_live::timebase::{self, Anchor, SharedAnchor};
use doppler_live::plugin::{self, PluginFinding};
use doppler_live::modules::planes::Planes;
use doppler_live::spectro::Frontend;
use doppler_live::{ingest, Config, Detector, Event, Module};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, ColorImage, FontFamily, FontId, Id, Key, Pos2, Rect, Sense, Shape, Stroke,
    TextureHandle, TextureOptions,
};
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
/// Full-band display layer: 1024-point FFT at 16 kHz (15.6 Hz bins, 0–8 kHz).
const FULL_NFFT: usize = 1024;
/// z range mapped onto one byte of history.
const Z_LO: f32 = -1.0;
const Z_HI: f32 = 6.0;

// Palette. Plot areas (spectrogram, waveform, slice, timeline) are always a dark "screen"
// with phosphor accents; the window chrome around them follows the light/dark setting.
static LIGHT: AtomicBool = AtomicBool::new(false);

fn light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}
fn bg() -> Color32 {
    if light() { Color32::from_rgb(236, 238, 241) } else { Color32::from_rgb(8, 9, 11) }
}
fn panel() -> Color32 {
    if light() { Color32::from_rgb(246, 247, 249) } else { Color32::from_rgb(13, 15, 18) }
}
fn text() -> Color32 {
    if light() { Color32::from_rgb(28, 32, 38) } else { Color32::from_rgb(198, 206, 214) }
}
/// Accent for chrome text (title, live state): bright on dark, deep on light.
fn accent() -> Color32 {
    if light() { Color32::from_rgb(30, 120, 40) } else { HIGH }
}
/// Grade colours for text on the chrome (pass list); plot overlays use `grade_color`.
fn grade_text(g: &str) -> Color32 {
    if !light() {
        return grade_color(g);
    }
    match g {
        "high" => Color32::from_rgb(30, 120, 40),
        "medium" => Color32::from_rgb(170, 100, 0),
        _ => Color32::from_rgb(110, 110, 110),
    }
}
const FIELD: Color32 = Color32::from_rgb(4, 5, 8);

// ---------------------------------------------------------------- settings

/// How REC saves a stream.
#[derive(Clone)]
struct RecSettings {
    /// Output folder (absolute).
    folder: String,
    /// true: raw copy of the stream (original codec); false: WAV of the decoded audio.
    raw: bool,
    /// File names: `<prefix>-<UTC>.mka|wav` plus `.json`.
    prefix: String,
}

/// User settings, kept between runs in a small `key=value` text file.
struct Settings {
    light: bool,
    /// Show the whole-band minimap while zoomed in.
    minimap: bool,
    /// Show the waveform above the spectrogram.
    show_wave: bool,
    /// Show the spectrum slice to the right of the spectrogram.
    show_slice: bool,
    /// Imported plugins: executable path, on or off.
    plugins: Vec<(String, bool)>,
    rec: RecSettings,
}

impl Settings {
    fn file() -> Option<std::path::PathBuf> {
        let base = std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
        Some(base.join("doppler-view").join("settings.txt"))
    }

    fn load() -> Self {
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut s = Settings {
            light: true,
            minimap: true,
            show_wave: true,
            show_slice: true,
            plugins: Vec::new(),
            // Default: straight into the folder the app runs from, ready to record at once.
            rec: RecSettings { folder: cwd.to_string_lossy().into_owned(), raw: true, prefix: "doppler".into() },
        };
        let old_default_folder = cwd.join("recordings").to_string_lossy().into_owned();
        if let Some(text) = Self::file().and_then(|p| std::fs::read_to_string(p).ok()) {
            for line in text.lines() {
                let Some((k, v)) = line.split_once('=') else { continue };
                match k.trim() {
                    "theme" => s.light = v.trim() != "dark",
                    "minimap" => s.minimap = v.trim() != "off",
                    "show_wave" => s.show_wave = v.trim() != "off",
                    "show_slice" => s.show_slice = v.trim() != "off",
                    "plugin" => {
                        if let Some((state, path)) = v.split_once('|') {
                            s.plugins.push((path.trim().to_string(), state.trim() != "off"));
                        }
                    }
                    // Earlier builds saved their defaults (recordings/, "rec"): treat those as unset.
                    "rec_folder" if !v.trim().is_empty() && v.trim() != old_default_folder => {
                        s.rec.folder = v.trim().to_string()
                    }
                    "rec_format" => s.rec.raw = v.trim() != "wav",
                    "rec_prefix" if !v.trim().is_empty() && v.trim() != "rec" => s.rec.prefix = v.trim().to_string(),
                    _ => {}
                }
            }
        }
        s
    }

    fn save(&self) {
        let Some(p) = Self::file() else { return };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut text = format!(
            "theme={}\nminimap={}\nshow_wave={}\nshow_slice={}\nrec_folder={}\nrec_format={}\nrec_prefix={}\n",
            if self.light { "light" } else { "dark" },
            if self.minimap { "on" } else { "off" },
            if self.show_wave { "on" } else { "off" },
            if self.show_slice { "on" } else { "off" },
            self.rec.folder,
            if self.rec.raw { "raw" } else { "wav" },
            self.rec.prefix
        );
        for (path, on) in &self.plugins {
            text.push_str(&format!("plugin={}|{}\n", if *on { "on" } else { "off" }, path));
        }
        let _ = std::fs::write(p, text);
    }

    /// Paths of the plugins that are switched on.
    fn enabled_plugins(&self) -> Vec<String> {
        self.plugins.iter().filter(|(_, on)| *on).map(|(p, _)| p.clone()).collect()
    }
}

/// Keep file-name prefixes to safe characters.
fn clean_prefix(p: &str) -> String {
    let s: String = p.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect();
    if s.is_empty() { "doppler".into() } else { s }
}
const GRID: Color32 = Color32::from_rgb(34, 38, 44);
const DIM: Color32 = Color32::from_rgb(110, 118, 128);
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
    /// Full-band display layer (0–8 kHz, coarse), same byte encoding, own frame times.
    full_freqs: Vec<f64>,
    full_hist: Vec<u8>,
    full_t: Vec<f64>,
    /// Plugins running on this source (separate programs), in the order they were started.
    plugins: Vec<PluginView>,
    /// Manual data for this source (key, value rows), as typed in the viewer.
    manual: Vec<(String, String)>,
    /// What each module does to the stream (for display and the recording's JSON).
    module_desc: Vec<String>,
    dsp_s: f64,
    audio_s: f64,
    wall_s: f64,
    done: bool,
    live: bool,
    /// Current recording, if any: file and seconds written.
    rec_path: Option<String>,
    rec_s: f64,
    rec_format: String,
    /// UTC (Coordinated Universal Time) of relative time 0, and its source, if known.
    anchor: Option<Anchor>,
    status: String,
    c: f64,
}

/// A plugin's output, as the viewer shows it.
struct PluginView {
    /// Executable path: the plugin's identity (settings, eye toggle).
    path: String,
    name: String,
    about: String,
    color: Color32,
    /// Latest status text from the plugin ("\n" separates lines).
    status: String,
    /// Where the process stands: starting, running, finished, or what went wrong.
    state: String,
    /// Last line the plugin wrote on stderr.
    log: String,
    /// Measured frequency lines, by track id: (t, Hz, ok).
    tracks: Vec<(String, Vec<(f64, f64, bool)>)>,
    findings: Vec<PluginFinding>,
    /// The band the plugin's view should show.
    view: Option<(f64, f64)>,
    /// Live audio the plugin could not keep up with, in seconds.
    dropped_s: f64,
    /// Facts for the result JSON, in the order first reported.
    meta: Vec<(String, String)>,
}

impl PluginView {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            name: plugin_name(path),
            about: String::new(),
            color: Color32::from_rgb(200, 200, 200),
            status: String::new(),
            state: "starting".into(),
            log: String::new(),
            tracks: Vec::new(),
            findings: Vec::new(),
            view: None,
            dropped_s: 0.0,
            meta: Vec::new(),
        }
    }

    fn apply(&mut self, m: plugin::Msg) {
        match m {
            plugin::Msg::Hello { name, about, color } => {
                if !name.is_empty() {
                    self.name = name;
                }
                self.about = about;
                if let Some([r, g, b]) = color {
                    self.color = Color32::from_rgb(r, g, b);
                }
                self.state = "running".into();
            }
            plugin::Msg::Status(t) => self.status = t,
            plugin::Msg::View { f_lo, f_hi } => self.view = Some((f_lo, f_hi)),
            plugin::Msg::Track { id, t, hz, ok } => match self.tracks.iter_mut().find(|(i, _)| *i == id) {
                Some((_, v)) => v.push((t, hz, ok)),
                None => self.tracks.push((id, vec![(t, hz, ok)])),
            },
            plugin::Msg::Finding(f) => self.findings.push(f),
            plugin::Msg::Meta { key, value } => match self.meta.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => *v = value,
                None => self.meta.push((key, value)),
            },
        }
    }

    /// The plugin's colour for text on the window chrome (darker in the light theme).
    fn text_color(&self) -> Color32 {
        if light() {
            let [r, g, b, _] = self.color.to_array();
            Color32::from_rgb(r / 2, g / 2, b / 2)
        } else {
            self.color
        }
    }
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

/// Byte → colour: black, deep blue, violet, red, orange, pale yellow.
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

/// Clock text for relative time `rel`: UTC time of day when an anchor is known, else time
/// from the start of the source.
fn stamp(s: &Shared, rel: f64) -> String {
    match &s.anchor {
        Some(a) => timebase::fmt_utc_clock(a.utc(rel)),
        None => fmt_clock(s.offset + rel),
    }
}

fn fmt_clock(t: f64) -> String {
    let t = t.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}

// ---------------------------------------------------------------- worker

/// Switches the window flips while the worker runs.
struct Controls {
    smooth: AtomicBool,
    /// Live only: feed the speakers from the stream.
    audio: AtomicBool,
    /// Live only: keep the timeline and record the stream.
    record: AtomicBool,
    /// Ask the worker to finish (source switched or closed).
    stop: AtomicBool,
    /// Where and how REC saves; read when REC starts.
    rec: Mutex<RecSettings>,
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
            // A microphone ("dshow:<device>") is live, like a stream.
            Source::Ffmpeg(p, _) => p.contains("://") || p.starts_with("dshow:"),
            Source::Wav(_) => false,
        }
    }
}

type WavOut = hound::WavWriter<std::io::BufWriter<std::fs::File>>;

fn open_wav(path: &str) -> Result<WavOut, String> {
    let spec = hound::WavSpec { channels: 1, sample_rate: PLAY_SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    hound::WavWriter::create(path, spec).map_err(|e| format!("{path}: {e}"))
}

fn host_of(url: &str) -> String {
    url.split_once("://").map_or(url, |x| x.1).split('/').next().unwrap_or("").to_string()
}

/// One REC session: a raw copy of the stream when it can be copied (original codec,
/// untouched), else a WAV of the decoded audio; plus `<name>.json` saying what the file is
/// and how its times map to UTC (Coordinated Universal Time).
struct Recorder {
    stem: String,
    file: String,
    format: String,
    raw: Option<ingest::RawCopy>,
    wav: Option<WavOut>,
    /// Analysis (relative) time when REC was pressed.
    rel_start: f64,
    /// UTC of the recorded file's first sample, and its source.
    anchor: SharedAnchor,
    source_host: String,
    last_sidecar: Instant,
    secs: f64,
}

impl Recorder {
    fn start(src: &Source, rel_start: f64, analysis: Option<&Anchor>, rs: &RecSettings) -> Result<Self, String> {
        // One folder per recording, `<prefix>-<UTC>/`, holding `<prefix>-<UTC>.mka|wav` and `.json`
        // (same name inside, so the files still describe themselves if moved out).
        let name = format!("{}-{}", clean_prefix(&rs.prefix), timebase::utc_stamp(timebase::now_unix()));
        let dir = std::path::Path::new(&rs.folder).join(&name);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let stem = dir.join(&name).to_string_lossy().into_owned();
        let anchor: SharedAnchor = Default::default();
        let mut r = Self {
            stem: stem.clone(),
            file: String::new(),
            format: String::new(),
            raw: None,
            wav: None,
            rel_start,
            anchor: anchor.clone(),
            source_host: String::new(),
            last_sidecar: Instant::now(),
            secs: 0.0,
        };
        if let Source::Ffmpeg(url, _) = src {
            if url.contains("://") && rs.raw {
                let file = format!("{stem}.mka");
                let raw = ingest::RawCopy::start(url, &file)?;
                // Separate connection: approximate until the stream's own capture times resolve.
                *anchor.lock().unwrap() = Some(Anchor {
                    utc0: timebase::now_unix(),
                    source: "PC clock when REC was pressed (approximate: the copy uses its own connection)".into(),
                });
                timebase::upgrade_from_hls(url.clone(), raw.first_segment.clone(), anchor, 0.0);
                r.file = file;
                r.format = "raw stream copy: original codec and sample rate, untouched (Matroska audio container)".into();
                r.raw = Some(raw);
                r.source_host = host_of(url);
                return Ok(r);
            }
        }
        let why = if rs.raw { "this source cannot be copied raw" } else { "chosen in recording settings" };
        r.start_wav(analysis, &format!("WAV 16 kHz 16-bit mono of the decoded audio ({why})"))?;
        r.source_host = match src {
            Source::Stdin => "stdin".into(),
            Source::Ffmpeg(p, _) | Source::Wav(p) => host_of(p),
        };
        Ok(r)
    }

    fn start_wav(&mut self, analysis: Option<&Anchor>, format: &str) -> Result<(), String> {
        let file = format!("{}.wav", self.stem);
        self.wav = Some(open_wav(&file)?);
        self.file = file;
        self.format = format.into();
        // The WAV is written from the analysed audio, so it shares the analysis timeline.
        *self.anchor.lock().unwrap() = analysis.map(|a| Anchor {
            utc0: a.utc(self.rel_start + self.secs),
            source: format!("{} (WAV written from the analysed audio)", a.source),
        });
        // A fallback WAV starts now; earlier seconds went to the (failed) raw copy.
        self.rel_start += self.secs;
        self.secs = 0.0;
        Ok(())
    }

    fn write(&mut self, piece: &[f32], analysis: Option<&Anchor>) {
        if let Some(raw) = self.raw.as_mut() {
            if !raw.alive() {
                // ffmpeg could not (or no longer) copy the stream: fall back to the decoded audio.
                self.raw = None;
                let _ = self.start_wav(analysis, "WAV 16 kHz 16-bit mono of the decoded audio (raw copy failed)");
            }
        }
        if let Some(w) = self.wav.as_mut() {
            for &v in piece {
                let _ = w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16);
            }
        }
        self.secs += piece.len() as f64 / PLAY_SR;
    }

    /// Write `<name>.json`: what the file is, its UTC start and source, and the passes
    /// detected while recording with their time inside the file.
    fn sidecar(&mut self, analysis: Option<&Anchor>, s: &Shared) {
        let events = &s.events;
        self.last_sidecar = Instant::now();
        let rec = self.anchor.lock().unwrap().clone();
        let stream_exact = |a: &Anchor| a.source.starts_with("stream PROGRAM-DATE-TIME");
        let from_analysis = self.wav.is_some();
        let rs = self.rel_start;
        let (accuracy, map): (&str, Box<dyn Fn(f64) -> f64>) = match (&rec, analysis) {
            _ if from_analysis => ("exact (same audio as the analysis)", Box::new(move |t| t - rs)),
            (Some(r), Some(a)) if stream_exact(r) && stream_exact(a) => {
                let (ru, a) = (r.utc0, a.clone());
                ("exact (both anchored to the stream's capture times)", Box::new(move |t| a.utc(t) - ru))
            }
            (Some(r), Some(a)) => {
                let (ru, a) = (r.utc0, a.clone());
                ("approximate, about ±5 s (no stream capture times; two connections)", Box::new(move |t| a.utc(t) - ru))
            }
            _ => ("approximate, about ±5 s", Box::new(move |t| t - rs)),
        };
        let mut j = String::from("{\n");
        j.push_str(&format!("  \"file\": \"{}\",\n", timebase::json_escape(self.file.rsplit('/').next().unwrap_or(&self.file))));
        j.push_str(&format!("  \"format\": \"{}\",\n", timebase::json_escape(&self.format)));
        j.push_str(&format!("  \"source_host\": \"{}\",\n", timebase::json_escape(&self.source_host)));
        j.push_str("  \"recorded_by\": \"doppler-view\",\n");
        match &rec {
            Some(r) => {
                j.push_str(&format!("  \"utc_start\": \"{}\",\n", timebase::fmt_utc(r.utc0)));
                j.push_str(&format!("  \"utc_source\": \"{}\",\n", timebase::json_escape(&r.source)));
            }
            None => j.push_str("  \"utc_start\": null,\n  \"utc_source\": \"none\",\n"),
        }
        j.push_str(&format!("  \"duration_s\": {:.2},\n", self.secs));
        j.push_str(&format!("  \"analysis_rel_at_rec_start_s\": {:.3},\n", self.rel_start));
        j.push_str("  \"modules\": [");
        for (i, d) in s.module_desc.iter().enumerate() {
            j.push_str(&format!("{}\n    \"{}\"", if i == 0 { "" } else { "," }, timebase::json_escape(d)));
        }
        j.push_str("\n  ],\n");
        j.push_str(&manual_json(&s.manual));
        j.push_str(",\n");
        j.push_str(&format!("  \"event_time_in_file_accuracy\": \"{accuracy}\",\n"));
        j.push_str("  \"events\": [");
        let mut first = true;
        for e in events.iter().filter(|e| e.t_cpa >= self.rel_start) {
            let mut ej = e.to_json(analysis);
            ej.pop();
            ej.push_str(&format!(",\"t_in_file\":{:.2}}}", map(e.t_cpa)));
            j.push_str(if first { "\n    " } else { ",\n    " });
            j.push_str(&ej);
            first = false;
        }
        j.push_str(if first { "],\n" } else { "\n  ],\n" });
        // Plugins: their facts, and findings overlapping the recording.
        j.push_str(&plugins_json(s, analysis, self.rel_start));
        j.push_str("\n}\n");
        let _ = std::fs::write(format!("{}.json", self.stem), j);
    }

    fn stop(mut self, analysis: Option<&Anchor>, s: &Shared) {
        if let Some(w) = self.wav.take() {
            let _ = w.finalize();
        }
        self.sidecar(analysis, s);
        if let Some(raw) = self.raw.take() {
            // ffmpeg needs a moment to close the file cleanly; do not hold up the analysis.
            thread::spawn(move || raw.stop());
        }
    }
}

fn worker(sh: Arc<Mutex<Shared>>, src: Source, ctl: Arc<Controls>, ring: Ring, plugins: Vec<String>) {
    let cfg = Config::default();
    let c = cfg.c;
    // Modules, all reading the same 16 kHz chunks; plus a coarse full-band spectrogram for display.
    let mut planes = Planes::new(cfg);
    let mut full = Frontend::new(PLAY_SR, FULL_NFFT, (HOP_S * PLAY_SR) as usize, 0.0, PLAY_SR / 2.0);
    let mut full_frame = vec![0.0f32; full.nb];
    {
        let mut desc = vec![planes.describe()];
        desc.extend(plugins.iter().map(|p| format!("plugin {p}: separate program, fed the same 16 kHz audio through a pipe")));
        sh.lock().unwrap().module_desc = desc;
    }
    // Plugins: started now, fed every chunk below; None once a plugin has gone.
    let mut feeds: Vec<Option<std::sync::mpsc::SyncSender<Vec<f32>>>> =
        plugins.iter().enumerate().map(|(i, p)| spawn_plugin(p, i, sh.clone(), &source_key(&src), src.is_live())).collect();
    let mut n: u64 = 0;
    let live = src.is_live();
    let t0 = Instant::now();
    // Live + smooth: paced behind a jitter buffer. Live raw: processed as it arrives.
    // Files: as fast as possible, so the whole timeline is available.
    let mut base: Option<Instant> = None;
    let mut rec: Option<Recorder> = None;
    // UTC of relative time 0 for the analysis, and where it came from. Files: our sidecar or
    // creation_time (shifted by --start). Live: PC clock at first audio, upgraded to the
    // stream's capture times when the playlist has them.
    let anchor: SharedAnchor = Default::default();
    if let Source::Wav(p) | Source::Ffmpeg(p, _) = &src {
        if !live {
            let start = if let Source::Ffmpeg(_, Some(s)) = &src { *s } else { 0.0 };
            *anchor.lock().unwrap() =
                timebase::file_anchor(p).map(|a| Anchor { utc0: a.utc0 + start, source: a.source });
        }
    }

    let mut handle = |x: &[f32]| {
        if live && n == 0 {
            let mut a = anchor.lock().unwrap();
            if a.is_none() {
                *a = Some(timebase::received_anchor(0.0));
            }
        }
        for piece in x.chunks((PLAY_SR / 20.0) as usize) {
            if ctl.stop.load(Ordering::Relaxed) {
                return;
            }
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

            // Recording (live): start on REC, stop when REC is released.
            let recording = live && ctl.record.load(Ordering::Relaxed);
            let an = anchor.lock().unwrap().clone();
            let mut rec_error: Option<String> = None;
            if recording && rec.is_none() {
                let rs = ctl.rec.lock().unwrap().clone();
                match Recorder::start(&src, n as f64 / PLAY_SR, an.as_ref(), &rs) {
                    Ok(r) => rec = Some(r),
                    Err(e) => {
                        ctl.record.store(false, Ordering::Relaxed);
                        rec_error = Some(e);
                    }
                }
            }
            if recording {
                if let Some(r) = rec.as_mut() {
                    r.write(piece, an.as_ref());
                }
            } else if let Some(r) = rec.take() {
                r.stop(an.as_ref(), &sh.lock().unwrap());
            }

            // Plugins get the same chunk. A file waits for them; live audio never does: a plugin
            // that falls behind loses audio, and the loss is shown in its box.
            let mut dropped: Vec<usize> = Vec::new();
            for (i, feed) in feeds.iter_mut().enumerate() {
                if let Some(tx) = feed {
                    let gone = if live {
                        match tx.try_send(piece.to_vec()) {
                            Ok(()) => false,
                            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                                dropped.push(i);
                                false
                            }
                            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => true,
                        }
                    } else {
                        tx.send(piece.to_vec()).is_err()
                    };
                    if gone {
                        *feed = None;
                    }
                }
            }

            let tick = Instant::now();
            let mut frames: Vec<(f64, Vec<u8>)> = Vec::new();
            let evs = planes.push_with(piece, |t, z| frames.push((t, z.iter().map(|&v| quantise(v)).collect())));
            full.feed(piece);
            let mut full_frames: Vec<(f64, Vec<u8>)> = Vec::new();
            while let Some(t) = full.next_frame(&mut full_frame) {
                full_frames.push((t, full_frame.iter().map(|&v| quantise(v)).collect()));
            }
            let cost = tick.elapsed().as_secs_f64();
            n += piece.len() as u64;

            let mut s = sh.lock().unwrap();
            if let Some(e) = rec_error {
                s.status = format!("record: {e}");
            }
            s.anchor = an.clone();
            match &rec {
                Some(r) => {
                    s.rec_path = Some(r.file.clone());
                    s.rec_s = r.secs;
                    s.rec_format = r.format.clone();
                }
                None => s.rec_path = None,
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
                s.ridges = planes.detector().ridges().to_vec();
                s.pending = planes.detector().pending().cloned().collect();
            }
            for (t, q) in full_frames {
                s.full_hist.extend_from_slice(&q);
                s.full_t.push(t);
            }
            for i in dropped {
                if let Some(pv) = s.plugins.get_mut(i) {
                    pv.dropped_s += piece.len() as f64 / PLAY_SR;
                }
            }
            // Live and not recording: keep only the visible window (trimmed in batches).
            if live && !recording && s.hist_t.len() > 2 * s.cols {
                let k = s.hist_t.len() - s.cols;
                s.hist.drain(..k * nb);
                s.hist_t.drain(..k);
                let fnb = s.full_freqs.len();
                let kf = s.full_t.len().saturating_sub(s.cols);
                s.full_hist.drain(..kf * fnb);
                s.full_t.drain(..kf);
                let t_keep = s.hist_t.first().copied().unwrap_or(0.0);
                for pv in s.plugins.iter_mut() {
                    for (_, track) in pv.tracks.iter_mut() {
                        let k = track.partition_point(|q| q.0 < t_keep);
                        track.drain(..k);
                    }
                }
            }
            let new_events = !evs.is_empty();
            for e in evs {
                println!("{}", e.to_json(an.as_ref()));
                s.events.push(e);
            }
            // Keep the sidecar current: on new passes, and every few seconds (anchor upgrades).
            if let Some(r) = rec.as_mut() {
                if new_events || r.last_sidecar.elapsed() > Duration::from_secs(3) {
                    r.sidecar(an.as_ref(), &s);
                }
            }
            s.dsp_s += cost;
            s.audio_s = n as f64 / PLAY_SR;
            s.wall_s = t0.elapsed().as_secs_f64();
            s.c = c;
        }
    };

    let r = match &src {
        Source::Wav(p) => ingest::read_wav(p, PLAY_SR, |x| handle(x)),
        Source::Ffmpeg(p, start) => match ingest::spawn_ffmpeg_logged(p, PLAY_SR as u32, *start) {
            Ok((mut child, rx, first)) => {
                if live {
                    timebase::upgrade_from_hls(p.clone(), first, anchor.clone(), 0.0);
                }
                for x in rx {
                    if ctl.stop.load(Ordering::Relaxed) {
                        let _ = child.kill();
                        break;
                    }
                    handle(&x);
                }
                let _ = child.wait();
                Ok(())
            }
            Err(e) => Err(e),
        },
        Source::Stdin => {
            for x in ingest::spawn_stdin((PLAY_SR / 20.0) as usize) {
                if ctl.stop.load(Ordering::Relaxed) {
                    break;
                }
                handle(&x);
            }
            Ok(())
        }
    };
    drop(handle);
    // End of input for the plugins: they report what they still hold and exit.
    drop(feeds);
    let evs = planes.flush_events();
    let an = anchor.lock().unwrap().clone();
    let mut s = sh.lock().unwrap();
    for e in evs {
        println!("{}", e.to_json(an.as_ref()));
        s.events.push(e);
    }
    if let Some(r) = rec.take() {
        r.stop(an.as_ref(), &s);
    }
    s.pending.clear();
    s.done = true;
    s.status = match r {
        Ok(()) if live => "stream ended".into(),
        Ok(()) => format!("analysed in {:.1} s", s.wall_s),
        Err(e) => format!("error: {e}"),
    };
}

// ---------------------------------------------------------------- results

/// The source's identity: its path or URL.
fn source_key(src: &Source) -> String {
    match src {
        Source::Wav(p) | Source::Ffmpeg(p, _) => p.clone(),
        Source::Stdin => "stdin".into(),
    }
}

/// Manual data fields every source starts with, and what to type in them.
const MANUAL_KEYS: [(&str, &str); 5] = [
    ("date", "YYYY-MM-DD"),
    ("time", "HH:MM, with the time zone"),
    ("place", "town, region, country"),
    ("coordinates", "lat, lon"),
    ("notes", "anything else"),
];
const MANUAL_C: Color32 = Color32::from_rgb(230, 190, 110);

fn manual_defaults() -> Vec<(String, String)> {
    MANUAL_KEYS.iter().map(|(k, _)| (k.to_string(), String::new())).collect()
}

/// Manual data for a source: the standard fields, filled from the file's result JSON when an
/// earlier session wrote one.
fn load_manual(src: &Source) -> Vec<(String, String)> {
    let mut m = manual_defaults();
    if let Source::Wav(p) | Source::Ffmpeg(p, _) = src {
        if !src.is_live() {
            let doc = std::fs::read_to_string(format!("{p}.doppler.json")).unwrap_or_default();
            for (k, v) in plugin::get_object(&doc, "manual").unwrap_or_default() {
                if let plugin::Value::Str(v) = v {
                    match m.iter_mut().find(|(mk, _)| *mk == k) {
                        Some((_, mv)) => *mv = v,
                        None => m.push((k, v)),
                    }
                }
            }
        }
    }
    m
}

/// `"manual": {…}`: the rows with both a name and a value.
fn manual_json(manual: &[(String, String)]) -> String {
    let esc = timebase::json_escape;
    let items: Vec<String> = manual
        .iter()
        .filter(|(k, v)| !k.trim().is_empty() && !v.trim().is_empty())
        .map(|(k, v)| format!("\"{}\": \"{}\"", esc(k.trim()), esc(v.trim())))
        .collect();
    format!("  \"manual\": {{{}}}", items.join(", "))
}

/// `"plugins": […]`: each plugin with its state, facts and findings. Findings ending before
/// `from_t` are left out; times are in the analysis timeline, plus UTC when it is known.
fn plugins_json(s: &Shared, analysis: Option<&Anchor>, from_t: f64) -> String {
    let esc = timebase::json_escape;
    let mut j = String::from("  \"plugins\": [");
    for (i, pv) in s.plugins.iter().enumerate() {
        j.push_str(if i == 0 { "\n    {" } else { ",\n    {" });
        j.push_str(&format!(
            "\"plugin\": \"{}\", \"path\": \"{}\", \"state\": \"{}\",\n      \"meta\": {{",
            esc(&pv.name),
            esc(&pv.path),
            esc(&pv.state)
        ));
        for (k, (key, v)) in pv.meta.iter().enumerate() {
            j.push_str(&format!("{}\"{}\": \"{}\"", if k == 0 { "" } else { ", " }, esc(key), esc(v)));
        }
        j.push_str("},\n      \"findings\": [");
        let mut first = true;
        for f in pv.findings.iter().filter(|f| f.t_end >= from_t) {
            let utc = analysis.map_or(String::new(), |a| format!(", \"utc_start\": \"{}\"", timebase::fmt_utc(a.utc(f.t_start))));
            j.push_str(if first { "\n        " } else { ",\n        " });
            j.push_str(&format!(
                "{{\"id\": {}, \"t_start\": {:.2}, \"t_end\": {:.2}, \"f_lo_hz\": {:.4}, \"f_hi_hz\": {:.4}, \"label\": \"{}\", \"detail\": \"{}\"{utc}}}",
                f.id,
                f.t_start,
                f.t_end,
                f.f_lo,
                f.f_hi,
                esc(&f.label),
                esc(&f.detail)
            ));
            first = false;
        }
        j.push_str(if first { "]}" } else { "\n      ]}" });
    }
    j.push_str(if s.plugins.is_empty() { "]" } else { "\n  ]" });
    j
}

/// The result JSON for an analysed file, kept next to it as `<file>.doppler.json`: what was
/// analysed and how, the manual data, the passes, and every plugin's facts and findings.
fn file_result_json(s: &Shared, path: &str, manual: &[(String, String)]) -> String {
    let esc = timebase::json_escape;
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let mut j = String::from("{\n");
    j.push_str(&format!("  \"file\": \"{}\",\n  \"path\": \"{}\",\n", esc(name), esc(path)));
    j.push_str("  \"analysed_by\": \"doppler-view\",\n");
    j.push_str(&format!("  \"analysed_utc\": \"{}\",\n", timebase::fmt_utc(timebase::now_unix())));
    match &s.anchor {
        Some(a) => j.push_str(&format!("  \"utc_start\": \"{}\",\n  \"utc_source\": \"{}\",\n", timebase::fmt_utc(a.utc0), esc(&a.source))),
        None => j.push_str("  \"utc_start\": null,\n  \"utc_source\": \"none\",\n"),
    }
    j.push_str(&format!("  \"start_offset_s\": {:.3},\n", s.offset));
    j.push_str(&format!("  \"duration_analysed_s\": {:.2},\n", s.audio_s));
    j.push_str("  \"modules\": [");
    for (i, d) in s.module_desc.iter().enumerate() {
        j.push_str(&format!("{}\n    \"{}\"", if i == 0 { "" } else { "," }, esc(d)));
    }
    j.push_str("\n  ],\n");
    j.push_str(&manual_json(manual));
    j.push_str(",\n  \"events\": [");
    for (i, e) in s.events.iter().enumerate() {
        j.push_str(if i == 0 { "\n    " } else { ",\n    " });
        j.push_str(&e.to_json(s.anchor.as_ref()));
    }
    j.push_str(if s.events.is_empty() { "],\n" } else { "\n  ],\n" });
    j.push_str(&plugins_json(s, s.anchor.as_ref(), f64::NEG_INFINITY));
    j.push_str("\n}\n");
    j
}

// ---------------------------------------------------------------- plugins

/// Display name of a plugin: its file name without the extension.
fn plugin_name(path: &str) -> String {
    std::path::Path::new(path).file_stem().map_or(path.to_string(), |s| s.to_string_lossy().into_owned())
}

/// Start plugin `idx` (an executable) on this source. Returns the channel feeding its stdin;
/// threads read its stdout (messages, into `Shared.plugins[idx]`) and its stderr (last line).
fn spawn_plugin(
    path: &str,
    idx: usize,
    sh: Arc<Mutex<Shared>>,
    source: &str,
    live: bool,
) -> Option<std::sync::mpsc::SyncSender<Vec<f32>>> {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};
    let started = doppler_live::no_window(&mut Command::new(path))
        .args(["--source", source, if live { "--live" } else { "--file" }])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match started {
        Ok(c) => c,
        Err(e) => {
            if let Some(pv) = sh.lock().unwrap().plugins.get_mut(idx) {
                pv.state = format!("could not start: {e}");
            }
            return None;
        }
    };
    let (Some(mut stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return None;
    };
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(64);
    // Audio in: raw f32 little-endian. When the channel closes, stdin drops: end of input.
    thread::spawn(move || {
        let mut bytes = Vec::new();
        for x in rx {
            bytes.clear();
            for v in x {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            if stdin.write_all(&bytes).is_err() {
                break;
            }
        }
    });
    // Messages out, then how the process ended.
    {
        let sh = sh.clone();
        thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(m) = plugin::parse_line(&line) {
                    if let Some(pv) = sh.lock().unwrap().plugins.get_mut(idx) {
                        pv.apply(m);
                    }
                }
            }
            let state = match child.wait() {
                Ok(st) if st.success() => "finished".to_string(),
                Ok(st) => format!("stopped ({st})"),
                Err(e) => format!("stopped ({e})"),
            };
            if let Some(pv) = sh.lock().unwrap().plugins.get_mut(idx) {
                pv.state = state;
            }
        });
    }
    // Text for people.
    thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                if let Some(pv) = sh.lock().unwrap().plugins.get_mut(idx) {
                    pv.log = line;
                }
            }
        }
    });
    Some(tx)
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

/// Menu choices, applied after the frame is drawn.
enum Action {
    OpenFile,
    OpenStream,
    Theme(bool),
    FullView,
    OpenMic,
    ImportPlugin,
    TogglePlugin(usize),
    RemovePlugin(usize),
}

/// A stream link being resolved by yt-dlp in the background: (link, result when done).
type Resolving = (String, Arc<Mutex<Option<Result<String, String>>>>);

struct App {
    sess: Session,
    played: Arc<AtomicU64>,
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
    tex_key: (u64, usize, usize),
    img: Vec<u8>,
    /// Open-stream dialog: shown, link typed, link being resolved, last error.
    stream_dialog: bool,
    stream_url: String,
    resolving: Option<Resolving>,
    open_error: Option<String>,
    /// Saved settings and the recording-settings window.
    settings: Settings,
    rec_dialog: bool,
    /// Eye toggles: planes shown, and plugins hidden (by executable path), on the graph,
    /// markers and bars.
    show_planes: bool,
    hidden_plugins: Vec<String>,
    /// The open source, so plugin changes can restart it; and a note when they cannot.
    current: Option<(Source, String)>,
    plugin_note: Option<String>,
    /// Manual data for the open source (key, value rows), written to its result JSON.
    manual: Vec<(String, String)>,
    /// Files: fingerprint of what the result JSON last held, and where it went (or why not).
    result_key: u64,
    result_note: Option<String>,
    /// Microphone dialog: shown, device names found, the one picked, last error.
    mic_dialog: bool,
    mic_devices: Vec<String>,
    mic_pick: usize,
    mic_error: Option<String>,
    /// Turn audio on once a source is open (on by default; kept across source switches).
    autostart_audio: bool,
    /// Full-band layer texture.
    tex_full: Option<TextureHandle>,
    tex_full_key: (u64, usize, usize),
    img_full: Vec<u8>,
    /// Frequency view: span (zoom) and centre (position), Hz.
    fspan: f64,
    fcenter: f64,
    /// Visible time span (s): Ctrl+wheel zooms it.
    tspan: f64,
    /// Frequency bar drag in progress: what is being dragged.
    fbar_drag: Option<FbarDrag>,
}

/// What a drag on the frequency bar moves.
#[derive(Clone, Copy)]
enum FbarDrag {
    /// The whole view: where it was grabbed above its bottom edge and its height, both in bar units (0–1).
    Move(f32, f32),
    /// The top edge (upper frequency).
    Top,
    /// The bottom edge (lower frequency).
    Bottom,
}

/// Fonts and chrome colours for the current light/dark setting.
fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(if light() { egui::Theme::Light } else { egui::Theme::Dark });
    ctx.all_styles_mut(|style| {
        for (_, f) in style.text_styles.iter_mut() {
            f.family = FontFamily::Monospace;
        }
        style.visuals.panel_fill = panel();
        style.visuals.window_fill = panel();
        style.visuals.extreme_bg_color = bg();
        style.visuals.override_text_color = Some(text());
    });
}

impl App {
    fn new(cc: &eframe::CreationContext, sess: Session, current: Option<(Source, String)>) -> Self {
        apply_theme(&cc.egui_ctx);
        let smooth = sess.ctl.smooth.load(Ordering::Relaxed);
        let manual = current.as_ref().map_or_else(manual_defaults, |(src, _)| load_manual(src));
        let app = Self {
            sess,
            played: Arc::new(AtomicU64::new(0)),
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
            tex_key: (u64::MAX, usize::MAX, 0),
            img: Vec::new(),
            stream_dialog: false,
            stream_url: String::new(),
            resolving: None,
            open_error: None,
            settings: Settings::load(),
            rec_dialog: false,
            show_planes: true,
            hidden_plugins: Vec::new(),
            current,
            plugin_note: None,
            mic_dialog: false,
            mic_devices: Vec::new(),
            mic_pick: 0,
            mic_error: None,
            autostart_audio: true,
            tex_full: None,
            tex_full_key: (u64::MAX, usize::MAX, 0),
            img_full: Vec::new(),
            fspan: 410.0,
            fcenter: 245.0,
            tspan: 160.0,
            fbar_drag: None,
            manual,
            result_key: 0,
            result_note: None,
        };
        app.sess.sh.lock().unwrap().manual = app.manual.clone();
        app
    }

    /// The plane band, 40–450 Hz, on the spectrogram.
    fn view_planes(&mut self) {
        self.fspan = 410.0;
        self.fcenter = 245.0;
    }

    /// A plugin's view: the band it asks for, or around one of its findings.
    fn view_plugin(&mut self, s: &Shared, i: usize, f: Option<&PluginFinding>) {
        let Some(pv) = s.plugins.get(i) else { return };
        let (lo, hi) = match f {
            Some(f) => {
                let span = ((f.f_hi - f.f_lo) * 3.0).max(pv.view.map_or(1.0, |v| v.1 - v.0));
                let c = 0.5 * (f.f_lo + f.f_hi);
                (c - span / 2.0, c + span / 2.0)
            }
            None => match pv.view {
                Some(v) => v,
                None => return,
            },
        };
        self.fspan = (hi - lo).max(MIN_FSPAN);
        self.fcenter = 0.5 * (lo + hi);
    }

    /// Plugins were added, removed or switched: save, and restart the open source so the new set
    /// runs on it from the first sample. Not while recording: a restart would cut the recording.
    fn plugins_changed(&mut self) {
        self.settings.save();
        if self.record {
            self.plugin_note = Some("Recording: plugin changes apply to the next source you open.".into());
        } else if let Some((src, label)) = self.current.clone() {
            self.plugin_note = None;
            self.switch(src, label);
        }
    }

    /// Hand the current recording settings to the session (read when REC starts).
    fn push_rec_settings(&self) {
        *self.sess.ctl.rec.lock().unwrap() = self.settings.rec.clone();
    }

    /// Close the current source and start analysing `src`.
    fn switch(&mut self, src: Source, label: String) {
        // Audio stays as the user left it: restarted on the new source if it was on.
        self.autostart_audio = self.audio || self.autostart_audio;
        if self.audio {
            self.stop_audio();
            self.audio = false;
        }
        self.record = false;
        self.sess.close();
        // Manual data belongs to the source: kept when the same source restarts (plugin changes),
        // else loaded from the file's result JSON, or empty.
        if !self.current.as_ref().is_some_and(|(c, _)| source_key(c) == source_key(&src)) {
            self.manual = load_manual(&src);
        }
        self.sess = Session::open(src.clone(), label.clone(), self.settings.enabled_plugins());
        self.sess.sh.lock().unwrap().manual = self.manual.clone();
        self.current = Some((src, label));
        self.result_key = 0;
        self.result_note = None;
        self.sess.ctl.smooth.store(self.smooth, Ordering::Relaxed);
        self.push_rec_settings();
        self.play_from = 0.0;
        self.follow = true;
        self.view_right = 0.0;
        self.selected = None;
        self.audio_err = None;
        self.open_error = None;
        self.tex_key = (u64::MAX, usize::MAX, 0);
        self.tex_full_key = (u64::MAX, usize::MAX, 0);
    }

    fn open_file_dialog(&mut self) {
        let pick = rfd::FileDialog::new()
            .set_title("Open recording")
            .add_filter("audio / video", &["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "mka", "mkv", "webm", "mp4", "mov"])
            .add_filter("all files", &["*"])
            .pick_file();
        if let Some(p) = pick {
            let p = p.to_string_lossy().into_owned();
            self.switch(source_for(&p, None), short_label(&p));
        }
    }

    /// Start a typed stream link: YouTube pages via yt-dlp in the background, others directly.
    fn open_stream(&mut self) {
        let link = self.stream_url.trim().to_string();
        if link.is_empty() {
            return;
        }
        if needs_resolving(&link) {
            let slot: Arc<Mutex<Option<Result<String, String>>>> = Default::default();
            let (l, s) = (link.clone(), slot.clone());
            thread::spawn(move || *s.lock().unwrap() = Some(resolve_with_ytdlp(&l)));
            self.resolving = Some((link, slot));
        } else {
            self.stream_dialog = false;
            self.switch(Source::Ffmpeg(link.clone(), None), short_label(&link));
        }
    }

    /// Check on a yt-dlp resolve; open the stream when it is ready.
    fn poll_resolving(&mut self) {
        let done = match &self.resolving {
            Some((link, slot)) => slot.lock().unwrap().take().map(|r| (link.clone(), r)),
            None => None,
        };
        if let Some((link, r)) = done {
            self.resolving = None;
            match r {
                Ok(url) => {
                    self.stream_dialog = false;
                    self.switch(Source::Ffmpeg(url, None), short_label(&link));
                }
                Err(e) => self.open_error = Some(e),
            }
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
        self.sess.ctl.audio.store(false, Ordering::Relaxed);
        self.player = None;
        self.sess.ring.lock().unwrap().clear();
    }

    fn start_audio(&mut self, offset: f64) -> Result<(), String> {
        self.sess.ring.lock().unwrap().clear();
        self.played.store(0, Ordering::Relaxed);
        if let Some(path) = self.sess.file.clone() {
            let stop = Arc::new(AtomicBool::new(false));
            spawn_file_audio(path, offset + self.play_from, self.sess.ring.clone(), stop.clone());
            self.feeder = Some(stop);
        } else {
            self.sess.ctl.audio.store(true, Ordering::Relaxed);
        }
        if self.player.is_none() {
            self.player = Some(start_player(self.sess.ring.clone(), self.played.clone())?);
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
            self.sess.ctl.smooth.store(smooth, Ordering::Relaxed);
        }
        if record != self.record {
            self.record = record;
            self.sess.ctl.record.store(record, Ordering::Relaxed);
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
        self.poll_resolving();

        // ------------------------------------------------ menu bar
        let mut action: Option<Action> = None;
        let (mut show_wave, mut show_slice) = (self.settings.show_wave, self.settings.show_slice);
        egui::Panel::top("menu").frame(egui::Frame::new().fill(panel()).inner_margin(egui::Margin::symmetric(6, 2))).show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Source", |ui| {
                    if ui.button("Open file…").clicked() {
                        action = Some(Action::OpenFile);
                        ui.close();
                    }
                    if ui.button("Open stream…").clicked() {
                        action = Some(Action::OpenStream);
                        ui.close();
                    }
                    if ui.button("Microphone…").clicked() {
                        action = Some(Action::OpenMic);
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Full spectrogram").clicked() {
                        action = Some(Action::FullView);
                        ui.close();
                    }
                    ui.separator();
                    ui.checkbox(&mut show_wave, "Waveform");
                    ui.checkbox(&mut show_slice, "Spectrum slice");
                });
                ui.menu_button("Settings", |ui| {
                    if ui.button("Recording…").clicked() {
                        self.rec_dialog = true;
                        ui.close();
                    }
                    ui.menu_button("Theme", |ui| {
                        if ui.radio(!light(), "Dark").clicked() {
                            action = Some(Action::Theme(false));
                            ui.close();
                        }
                        if ui.radio(light(), "Light").clicked() {
                            action = Some(Action::Theme(true));
                            ui.close();
                        }
                    });
                });
                ui.menu_button("Plugins", |ui| {
                    if ui.button("Import plugin…").clicked() {
                        action = Some(Action::ImportPlugin);
                        ui.close();
                    }
                    ui.separator();
                    if self.settings.plugins.is_empty() {
                        ui.label(egui::RichText::new("no plugins imported").color(DIM));
                    }
                    for (i, (path, on)) in self.settings.plugins.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let mut v = *on;
                            if ui.checkbox(&mut v, plugin_name(path)).on_hover_text(path.as_str()).changed() {
                                action = Some(Action::TogglePlugin(i));
                            }
                            if ui.small_button("Remove").clicked() {
                                action = Some(Action::RemovePlugin(i));
                            }
                        });
                    }
                    ui.separator();
                    ui.label(
                        egui::RichText::new("A plugin is a program: audio in on stdin,\nfindings out as JSON lines. Changes restart\nthe current source (not while recording).")
                            .font(mono(10.5))
                            .color(DIM),
                    );
                    if let Some(n) = &self.plugin_note {
                        ui.colored_label(MED, n);
                    }
                });
            });
        });

        if (show_wave, show_slice) != (self.settings.show_wave, self.settings.show_slice) {
            self.settings.show_wave = show_wave;
            self.settings.show_slice = show_slice;
            self.settings.save();
        }

        // ------------------------------------------------ recording settings
        if self.rec_dialog {
            let mut open = true;
            let mut changed = false;
            let mut browse = false;
            let rs = &mut self.settings.rec;
            egui::Window::new("Recording settings")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    egui::Grid::new("rec_grid").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                        ui.label("Folder");
                        ui.horizontal(|ui| {
                            changed |= ui.add(egui::TextEdit::singleline(&mut rs.folder).desired_width(380.0)).changed();
                            browse = ui.button("Browse…").clicked();
                        });
                        ui.end_row();

                        ui.label("Format");
                        ui.vertical(|ui| {
                            changed |= ui
                                .radio_value(&mut rs.raw, true, "Raw stream copy (.mka): original audio, untouched")
                                .changed();
                            changed |= ui
                                .radio_value(&mut rs.raw, false, "WAV (.wav): decoded 16 kHz mono, as analysed")
                                .changed();
                            ui.label(
                                egui::RichText::new("Sources that cannot be copied raw are always saved as WAV.")
                                    .font(mono(10.5))
                                    .color(DIM),
                            );
                        });
                        ui.end_row();

                        ui.label("File name");
                        ui.horizontal(|ui| {
                            changed |= ui.add(egui::TextEdit::singleline(&mut rs.prefix).desired_width(120.0)).changed();
                            ui.label(
                                egui::RichText::new(format!(
                                    "folder {0}-YYYYMMDD-HHMMSS (UTC) with {0}-….mka/.wav + .json",
                                    clean_prefix(&rs.prefix)
                                ))
                                    .font(mono(11.0))
                                    .color(DIM),
                            );
                        });
                        ui.end_row();
                    });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Changes apply to the next recording.").font(mono(10.5)).color(DIM),
                    );
                });
            if browse {
                if let Some(d) = rfd::FileDialog::new().set_title("Recording folder").set_directory(&rs.folder).pick_folder() {
                    rs.folder = d.to_string_lossy().into_owned();
                    changed = true;
                }
            }
            if changed {
                self.settings.save();
                self.push_rec_settings();
            }
            if !open {
                self.rec_dialog = false;
            }
        }

        // ------------------------------------------------ microphone dialog
        if self.mic_dialog {
            let mut open = true;
            let mut start: Option<String> = None;
            egui::Window::new("Microphone")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    if self.mic_devices.is_empty() {
                        ui.label("No microphone found.");
                    } else {
                        egui::ComboBox::from_label("Device")
                            .width(420.0)
                            .selected_text(self.mic_devices[self.mic_pick].as_str())
                            .show_ui(ui, |ui| {
                                for (i, d) in self.mic_devices.iter().enumerate() {
                                    ui.selectable_value(&mut self.mic_pick, i, d.as_str());
                                }
                            });
                        if ui.button("Start listening").clicked() {
                            start = Some(self.mic_devices[self.mic_pick].clone());
                        }
                    }
                    ui.label(
                        egui::RichText::new("Captured through Windows (DirectShow) and decoded to 16 kHz mono.
Time is the PC clock when the audio arrives.")
                            .font(mono(10.5))
                            .color(DIM),
                    );
                    if let Some(e) = &self.mic_error {
                        ui.colored_label(MED, e);
                    }
                });
            if let Some(dev) = start {
                self.mic_dialog = false;
                self.switch(Source::Ffmpeg(format!("dshow:{dev}"), None), format!("microphone: {dev}"));
            }
            if !open {
                self.mic_dialog = false;
            }
        }

        // ------------------------------------------------ open-stream dialog
        if self.stream_dialog {
            let mut open = true;
            let mut go = false;
            egui::Window::new("Open stream")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label("Stream URL: YouTube live link, HLS (.m3u8), Icecast, RTSP…");
                    let r = ui.add(egui::TextEdit::singleline(&mut self.stream_url).desired_width(520.0).hint_text("https://"));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        go = true;
                    }
                    ui.horizontal(|ui| {
                        let busy = self.resolving.is_some();
                        if ui.add_enabled(!busy, egui::Button::new("Open")).clicked() {
                            go = true;
                        }
                        if busy {
                            ui.spinner();
                            ui.label("resolving with yt-dlp…");
                        }
                    });
                    if let Some(e) = &self.open_error {
                        ui.colored_label(MED, e);
                    }
                });
            if go && self.resolving.is_none() {
                self.open_error = None;
                self.open_stream();
            }
            if !open {
                self.stream_dialog = false;
                self.resolving = None;
            }
        }

        let sh = self.sess.sh.clone();
        let s = sh.lock().unwrap();
        let nb = s.freqs.len();
        let (fmin, fmax) = (s.freqs[0], s.freqs[nb - 1]);
        let window = s.cols as f64 * HOP_S;
        let t_first = s.hist_t.first().copied().unwrap_or(0.0);
        let is_file = self.sess.file.is_some();
        // Time zoom: 10 s up to everything kept (files, REC), or the live window (live keeps only that).
        let t_max = if is_file || self.record { (s.now - t_first).max(window) } else { window };
        self.tspan = self.tspan.clamp(10.0, t_max);
        let span = self.tspan;
        // Record only keeps the timeline for live sources; recording state comes back from the worker.
        if s.live && self.record && s.rec_path.is_none() && !s.status.is_empty() && s.status.starts_with("record:") {
            self.record = false;
        }

        // Keys (not while typing in a text field): Home/0 start, End live or end,
        // Page Up/Down move the frequency view by most of its height.
        // Space: play or pause (the audio switch). Left/right arrows: 5 s (30 s with Shift).
        let mut key_nudge = 0.0f64;
        let mut space_pressed = false;
        if !ctx.egui_wants_keyboard_input() {
            // Taken here, before any widget sees them: a focused button (REC after a click)
            // would otherwise also be pressed by Space, and a recording must never be cut by it.
            let take = |m: egui::Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
            space_pressed = take(egui::Modifiers::NONE, Key::Space);
            for (m, step) in [(egui::Modifiers::NONE, 5.0), (egui::Modifiers::SHIFT, 30.0)] {
                if take(m, Key::ArrowLeft) {
                    key_nudge -= step;
                }
                if take(m, Key::ArrowRight) {
                    key_nudge += step;
                }
            }
            let (home, end, pg_up, pg_dn) = ctx.input(|i| {
                (
                    i.key_pressed(Key::Home) || i.key_pressed(Key::Num0),
                    i.key_pressed(Key::End),
                    i.key_pressed(Key::PageUp),
                    i.key_pressed(Key::PageDown),
                )
            });
            if home {
                self.follow = false;
                self.view_right = t_first + span;
            }
            if end {
                self.follow = true;
            }
            if pg_up {
                self.fcenter += self.fspan * 0.9;
            }
            if pg_dn {
                self.fcenter -= self.fspan * 0.9;
            }
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

        // Frequency view: [f_lo, f_hi] from zoom (span) and position (centre).
        self.fspan = self.fspan.clamp(MIN_FSPAN, FULL_MAX);
        self.fcenter = self.fcenter.clamp(self.fspan / 2.0, FULL_MAX - self.fspan / 2.0);
        let (f_lo, f_hi) = (self.fcenter - self.fspan / 2.0, self.fcenter + self.fspan / 2.0);
        let zoomed = self.fspan < FULL_MAX * 0.95;

        // Two layers, each the frames of the visible span up to t_right: the planes module's fine
        // spectrogram (40–450 Hz, 0.49 Hz bins) and the coarse full band (0–8 kHz, 15.6 Hz bins).
        // Zoomed out past MAX_TEX_W frames, each column is the maximum of `stride` frames, so short
        // bright tones still show. Groups are aligned to fixed frame indices so columns do not shimmer.
        let frames_view = ((span / HOP_S).ceil() as usize).max(1);
        let stride = frames_view.div_ceil(MAX_TEX_W).max(1);
        let ncol = frames_view.div_ceil(stride);
        let tex_frames = (ncol * stride) as f64;
        // Live without REC trims old frames, so indices shift: key on the frame count too.
        let follow_key = if self.follow && stride == 1 { s.frames_total } else { 0 };
        let idx_r = s.hist_t.partition_point(|&t| t <= t_right);
        let idx_r = idx_r - idx_r % stride;
        if (follow_key, idx_r, ncol * stride) != self.tex_key || self.tex.is_none() {
            self.tex_key = (follow_key, idx_r, ncol * stride);
            fill_image(&mut self.img, &self.lut, &s.hist, nb, ncol, stride, idx_r);
            let image = ColorImage::from_rgba_unmultiplied([ncol, nb], &self.img);
            match &mut self.tex {
                Some(t) => t.set(image, TextureOptions::LINEAR),
                None => self.tex = Some(ctx.load_texture("spec", image, TextureOptions::LINEAR)),
            }
        }
        let t_img = if idx_r > 0 { s.hist_t[idx_r - 1] } else { t_right };
        let fnb = s.full_freqs.len();
        let idx_full = s.full_t.partition_point(|&t| t <= t_right);
        let idx_full = idx_full - idx_full % stride;
        if (follow_key, idx_full, ncol * stride) != self.tex_full_key || self.tex_full.is_none() {
            self.tex_full_key = (follow_key, idx_full, ncol * stride);
            fill_image(&mut self.img_full, &self.lut, &s.full_hist, fnb, ncol, stride, idx_full);
            let image = ColorImage::from_rgba_unmultiplied([ncol, fnb], &self.img_full);
            match &mut self.tex_full {
                Some(t) => t.set(image, TextureOptions::LINEAR),
                None => self.tex_full = Some(ctx.load_texture("spec_full", image, TextureOptions::LINEAR)),
            }
        }
        let t_img_full = if idx_full > 0 { s.full_t[idx_full - 1] } else { t_right };

        // Blink for the live and REC dots: on for half of each second.
        let blink_on = (ctx.input(|i| i.time) * 2.0).fract() < 0.5;

        // ------------------------------------------------ top bar
        let (mut smooth, mut audio, mut record) = (self.smooth, self.audio, self.record);
        let mut minimap = self.settings.minimap;
        let audio_err = self.audio_err.clone();
        egui::Panel::top("top").frame(egui::Frame::new().fill(bg()).inner_margin(8.0)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("DOPPLER-LIVE").font(mono(15.0)).color(accent()).strong());
                ui.add_space(12.0);
                // Source: what is open, as a file or live, then its name.
                ui.label(egui::RichText::new("Source:").font(mono(12.0)).color(DIM));
                if is_file {
                    file_icon(ui, text());
                } else if s.live {
                    live_dot(ui, blink_on);
                }
                ui.label(egui::RichText::new(&s.source).font(mono(12.0)).color(text()));
                ui.label(egui::RichText::new("|").font(mono(12.0)).color(DIM));
                ui.checkbox(&mut smooth, egui::RichText::new("smooth").font(mono(12.0))).on_hover_text(
                    "On: live streams play out through a 4 s buffer and the view scrolls between 0.25 s frames.\n\
                     Off (raw): data is shown exactly as it arrives, bursts and steps included.",
                );
                ui.checkbox(&mut minimap, egui::RichText::new("minimap").font(mono(12.0)))
                    .on_hover_text("While zoomed in, show the whole 0–8 kHz band in the corner, with your view framed.");
                ui.checkbox(&mut audio, egui::RichText::new("audio").font(mono(12.0))).on_hover_text(if is_file {
                    "Play from the playhead (click the spectrogram to set it)."
                } else {
                    "Play the stream as received."
                });
                if s.live {
                    let label: egui::WidgetText = if record {
                        // The dot blinks while recording; the word stays red.
                        let mut job = egui::text::LayoutJob::default();
                        let dot = if blink_on { REC } else { REC.gamma_multiply(0.2) };
                        job.append("●", 0.0, egui::TextFormat::simple(mono(12.0), dot));
                        job.append(&format!(" REC {}", fmt_clock(s.rec_s)), 0.0, egui::TextFormat::simple(mono(12.0), REC));
                        job.into()
                    } else {
                        egui::RichText::new("○ REC").font(mono(12.0)).color(DIM).into()
                    };
                    ui.toggle_value(&mut record, label).on_hover_text(format!(
                        "Keep the timeline from now (scroll back, no limit) and save the stream to\n{}\n(Settings ▸ Recording… to change)",
                        self.settings.rec.folder
                    ));
                    if let Some(p) = &s.rec_path {
                        ui.label(egui::RichText::new(format!("→ {p}")).font(mono(11.0)).color(DIM))
                            .on_hover_text(format!("{}\nplus {} describing it", s.rec_format, p.rsplit_once('.').map_or(p.as_str(), |x| x.0).to_string() + ".json"));
                    }
                }
                if let Some(e) = &audio_err {
                    ui.label(egui::RichText::new(format!("audio: {e}")).font(mono(11.0)).color(MED));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let high = s.events.iter().filter(|e| e.grade() == "high").count();
                    // Live state is shown once, on the LIVE button under the spectrogram.
                    let state = if s.done {
                        s.status.clone()
                    } else if s.live {
                        String::new()
                    } else {
                        format!("▶ ANALYSING {:.0}× real time", s.audio_s / s.wall_s.max(1e-9))
                    };
                    if !state.is_empty() {
                        ui.label(egui::RichText::new(state).color(if s.done { DIM } else { MED }));
                    }
                    let load = if s.live {
                        format!("DSP {:5.2}%", 100.0 * s.dsp_s / s.audio_s.max(1e-9))
                    } else {
                        format!("AUDIO {}", fmt_clock(s.audio_s))
                    };
                    ui.label(egui::RichText::new(format!("{load}   PASSES {} ({} high)", s.events.len(), high)));
                    // Clock: UTC when anchored (source on hover), else time from the start.
                    let (tag, col, why) = match &s.anchor {
                        Some(a) if a.source.starts_with("stream") => ("UTC", HIGH, a.source.clone()),
                        Some(a) => ("UTC≈", MED, a.source.clone()),
                        None => ("T+", DIM, "no UTC source: time from the start of the file".to_string()),
                    };
                    ui.label(
                        egui::RichText::new(format!("{tag} {}", stamp(&s, if is_file { head } else { s.now })))
                            .font(mono(13.0))
                            .color(col),
                    )
                    .on_hover_text(why);
                });
            });
        });

        // ------------------------------------------------ bottom status
        egui::Panel::bottom("bottom").frame(egui::Frame::new().fill(bg()).inner_margin(6.0)).show(ui, |ui| {
            let df = s.freqs[1] - s.freqs[0];
            let hist = if is_file || self.record { fmt_clock(s.now - t_first) } else { "window only (REC to keep)".into() };
            let basis = match &s.anchor {
                Some(a) => format!("UTC from {}", a.source),
                None => "relative time (no UTC source)".into(),
            };
            ui.label(
                egui::RichText::new(format!(
                    "time: {basis}  │  detector {:.0} Hz  │  band {:.0}–{:.0} Hz  │  Δf {:.2} Hz  │  view {:.0} s  │  timeline {hist}  │  \
                     ridge ┄ cyan  │  candidate ┄ white  │  wheel: zoom Hz  Alt: move Hz  Ctrl: zoom time  Shift: scroll time  PgUp/PgDn  Home/0  End: {}",
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
            if let Some(n) = &self.result_note {
                ui.label(egui::RichText::new(n).font(mono(11.0)).color(if n.starts_with("result →") { DIM } else { MED }));
            }
        });

        // ------------------------------------------------ modules panel
        let manual_before = self.manual.clone();
        let mut eye_planes = false;
        let mut eye_plugin: Option<String> = None;
        let mut snap: Option<(u64, f64)> = None;
        let mut snap_plugin: Option<(usize, PluginFinding)> = None;
        let mut view_act: Option<ViewAct> = None;
        egui::Panel::right("modules").exact_size(340.0).frame(egui::Frame::new().fill(panel()).inner_margin(10.0)).show(ui, |ui| {
            // One framed box per module (planes, then each plugin), sharing the height equally.
            let gap = 10.0;
            // Manual data takes what its rows need; planes and the plugins share the rest equally.
            let manual_h = 34.0 + 16.0 + 26.0 * self.manual.len() as f32 + 30.0;
            let n_boxes = 1 + s.plugins.len();
            let box_h = ((ui.available_height() - manual_h - gap * n_boxes as f32) / n_boxes as f32).max(140.0);
            let w = ui.available_width();
            let planes_what = "aircraft Doppler curves · 40–450 Hz";
            let (v_p, e_p) = module_box(ui, vec2(w, box_h), "PLANES", planes_what, HIGH, Some(self.show_planes), |ui| {
                ui.label(egui::RichText::new("click a pass to jump to it").font(mono(10.5)).color(DIM));
                ui.label(egui::RichText::new(" #  T_CPA     f0     v     d    grade").font(mono(11.0)).color(DIM));
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("planes_list")
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for e in &s.events {
                            let sel = self.selected == Some(e.id);
                            let line = format!(
                                "{}{:>2}  {}  {:5.1}  {:3.0}  {:4.1}k  {}",
                                if sel { "▸" } else { " " },
                                e.id,
                                stamp(&s, e.t_cpa),
                                e.f0,
                                e.speed,
                                e.dist / 1000.0,
                                e.grade()
                            );
                            let detail = format!("    {:.0}→{:.0} Hz  snr {:.2}  rms {:.2}  h{}", e.f_pre, e.f_post, e.snr, e.rms, e.harmonics);
                            let r = ui
                                .vertical(|ui| {
                                    ui.label(egui::RichText::new(line).font(mono(12.0)).color(grade_text(e.grade())));
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
                            ui.label(egui::RichText::new(if s.done { "no passes found" } else { "listening…" }).color(DIM));
                        }
                    });
            });
            if v_p {
                view_act = Some(ViewAct::Planes);
            }
            eye_planes = e_p;

            // Manual data: what you know about this source, for its result JSON.
            ui.add_space(gap);
            module_box(ui, vec2(w, manual_h), "MANUAL DATA", "what you know · goes into the result JSON", MANUAL_C, None, |ui| {
                let mut remove: Option<usize> = None;
                egui::Grid::new("manual_grid").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                    for (i, (k, v)) in self.manual.iter_mut().enumerate() {
                        let standard = MANUAL_KEYS.iter().find(|(mk, _)| *mk == k.as_str());
                        match standard {
                            Some(_) => {
                                ui.label(egui::RichText::new(k.as_str()).font(mono(11.0)).color(DIM));
                            }
                            None => {
                                ui.add(egui::TextEdit::singleline(k).desired_width(78.0).hint_text("field").font(mono(11.0)));
                            }
                        }
                        let hint = standard.map_or("value", |(_, h)| *h);
                        ui.add(egui::TextEdit::singleline(v).desired_width(180.0).hint_text(hint).font(mono(11.0)));
                        if standard.is_none() && ui.small_button("×").on_hover_text("remove this field").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
                if let Some(i) = remove {
                    self.manual.remove(i);
                }
                if ui.small_button("+ field").on_hover_text("add a field of your own").clicked() {
                    self.manual.push((String::new(), String::new()));
                }
            });

            // Plugins: what each reports, as it reports it.
            for (i, pv) in s.plugins.iter().enumerate() {
                ui.add_space(gap);
                let title = pv.name.to_uppercase();
                let about = if pv.about.is_empty() { format!("plugin · {}", plugin_name(&pv.path)) } else { pv.about.clone() };
                let visible = !self.hidden_plugins.contains(&pv.path);
                let (v, e) = module_box(ui, vec2(w, box_h), &title, &about, pv.color, Some(visible), |ui| {
                    let tc = pv.text_color();
                    for line in pv.status.lines() {
                        ui.label(egui::RichText::new(line).font(mono(11.0)).color(tc));
                    }
                    if pv.state != "running" {
                        let fine = pv.state == "starting" || pv.state == "finished";
                        ui.label(egui::RichText::new(format!("plugin {}", pv.state)).font(mono(10.5)).color(if fine { DIM } else { MED }));
                    }
                    if !pv.log.is_empty() {
                        ui.label(egui::RichText::new(&pv.log).font(mono(10.5)).color(DIM));
                    }
                    if pv.dropped_s > 0.0 {
                        ui.label(
                            egui::RichText::new(format!("fell behind: {:.1} s of audio not analysed", pv.dropped_s)).font(mono(10.5)).color(MED),
                        );
                    }
                    if !pv.findings.is_empty() || pv.meta.is_empty() {
                        ui.label(egui::RichText::new(" #  start      dur   finding").font(mono(11.0)).color(DIM));
                    }
                    ui.separator();
                    egui::ScrollArea::vertical().id_salt(("plugin_list", i)).stick_to_bottom(true).show(ui, |ui| {
                        for f in &pv.findings {
                            let line = format!("{:>2}  {}  {:4.0}s  {}", f.id, stamp(&s, f.t_start), f.t_end - f.t_start, f.label);
                            let mut r = ui.add(egui::Label::new(egui::RichText::new(line).font(mono(12.0)).color(tc)).sense(Sense::click()));
                            if !f.detail.is_empty() {
                                r = r.on_hover_text(f.detail.as_str());
                            }
                            if r.clicked() {
                                snap_plugin = Some((i, f.clone()));
                            }
                        }
                        if pv.findings.is_empty() && pv.meta.is_empty() {
                            ui.label(egui::RichText::new("no findings yet").font(mono(10.5)).color(DIM));
                        }
                        for (k, v) in &pv.meta {
                            ui.label(egui::RichText::new(format!("{k}: {v}")).font(mono(11.0)).color(text()));
                        }
                    });
                });
                if v {
                    view_act = Some(ViewAct::Plugin(i));
                }
                if e {
                    eye_plugin = Some(pv.path.clone());
                }
            }
            if s.plugins.is_empty() {
                ui.add_space(gap);
                ui.label(egui::RichText::new("More modules: Plugins ▸ Import plugin…").font(mono(10.5)).color(DIM));
            }
        });

        if eye_planes {
            self.show_planes = !self.show_planes;
        }
        if let Some(path) = eye_plugin {
            match self.hidden_plugins.iter().position(|p| *p == path) {
                Some(k) => {
                    self.hidden_plugins.remove(k);
                }
                None => self.hidden_plugins.push(path),
            }
        }

        // ------------------------------------------------ main area
        let mut new_view: Option<f64> = None;
        let mut seek_to: Option<f64> = None;
        let mut go_live = false;
        let mut new_tspan: Option<f64> = None;
        let (mut fcenter, mut fspan) = (self.fcenter, self.fspan);
        let mut fbar_drag = self.fbar_drag;
        let show_minimap = self.settings.minimap && zoomed;
        let (show_wave, show_slice) = (self.settings.show_wave, self.settings.show_slice);
        let show_planes = self.show_planes;
        let hidden_plugins = self.hidden_plugins.clone();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(bg()).inner_margin(10.0)).show(ui, |ui| {
            let avail = ui.available_size();
            let wave_h = (avail.y * 0.14).clamp(60.0, 130.0);

            if show_wave {
                let (wrect, _) = ui.allocate_exact_size(vec2(avail.x, wave_h), Sense::hover());
                draw_wave(ui, wrect, &s, is_file && !self.audio);
                ui.add_space(6.0);
            }

            // Toolbar: visible time span and what the layers are.
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(egui::RichText::new(format!("time {}", fmt_span(span))).font(mono(11.0)))
                    .on_hover_text("Visible time span. Ctrl+wheel zooms it; the box on the timeline bar shows it.");
                ui.add_space(12.0);
                let layer = if fine_visible(fspan, f_lo, f_hi, &s.freqs) {
                    format!("{:.0}–{:.0} Hz · planes layer 0.49 Hz bins inside 40–450 Hz, full layer 15.6 Hz bins elsewhere", f_lo, f_hi)
                } else {
                    format!("{:.0}–{:.0} Hz · full layer 15.6 Hz bins", f_lo, f_hi)
                };
                ui.label(egui::RichText::new(layer).font(mono(10.5)).color(DIM));
            });
            ui.add_space(2.0);

            let rest = ui.available_size();
            let slice_w = 120.0;
            let fbar_lbl = 30.0;
            let fbar_w = 18.0;
            let pos_w = fbar_lbl + fbar_w + 6.0;
            let axis_l = 56.0;
            let axis_b = 20.0;
            let bar_h = 16.0;
            let (area, _) = ui.allocate_exact_size(rest, Sense::hover());
            let right_gap = if show_slice { slice_w + 10.0 } else { 0.0 };
            let plot_rect = Rect::from_min_max(
                pos2(area.left() + pos_w + axis_l, area.top()),
                pos2(area.right() - right_gap, area.bottom() - axis_b - bar_h - 8.0),
            );
            let spec_rect = plot_rect;
            let fbar_rect = Rect::from_min_max(
                pos2(area.left() + fbar_lbl, spec_rect.top()),
                pos2(area.left() + fbar_lbl + fbar_w, spec_rect.bottom()),
            );
            let slice_rect = Rect::from_min_max(pos2(spec_rect.right() + 10.0, spec_rect.top()), pos2(area.right(), spec_rect.bottom()));
            // The timeline bar runs under the spectrogram; LIVE sits under the slice column, or in
            // the bottom-right corner when the slice is hidden.
            let bar_right = if show_slice { spec_rect.right() } else { area.right() - slice_w - 10.0 };
            let bar_rect = Rect::from_min_max(pos2(spec_rect.left(), area.bottom() - bar_h), pos2(bar_right, area.bottom()));
            let live_left = if show_slice { slice_rect.left() } else { area.right() - slice_w };
            let live_rect = Rect::from_min_max(pos2(live_left, bar_rect.top()), pos2(area.right(), bar_rect.bottom()));

            let x_of = |t: f64| spec_rect.left() + ((t - t_left) / span) as f32 * spec_rect.width();
            let t_of = |x: f32| t_left + ((x - spec_rect.left()) / spec_rect.width()) as f64 * span;

            // Frequency bar (left): the whole 0–8 kHz band on a log scale, findings marked, the box is
            // the visible band. Drag the box to move, its top or bottom edge to zoom, click to centre.
            let fb_y = |f: f64| fbar_rect.bottom() - fbar_u(f) as f32 * fbar_rect.height();
            let fb_f = |y: f32| fbar_f(((fbar_rect.bottom() - y) / fbar_rect.height()) as f64);
            let fb_u = |y: f32| ((fbar_rect.bottom() - y) / fbar_rect.height()) as f64;
            let fb = ui.interact(fbar_rect, Id::unique("fbar"), Sense::click_and_drag());
            let (box_top, box_bot) = (fb_y(f_hi), fb_y(f_lo));
            // Edge grips: 5 px outside the box, up to a third of the box inside it.
            let zone = ((box_bot - box_top) / 3.0).min(5.0);
            let grip = |y: f32| -> Option<FbarDrag> {
                if y >= box_top - 5.0 && y <= box_top + zone {
                    Some(FbarDrag::Top)
                } else if y >= box_bot - zone && y <= box_bot + 5.0 {
                    Some(FbarDrag::Bottom)
                } else if y > box_top && y < box_bot {
                    let u_lo = fbar_u(f_lo);
                    Some(FbarDrag::Move((fb_u(y) - u_lo) as f32, (fbar_u(f_hi) - u_lo) as f32))
                } else {
                    None
                }
            };
            if let Some(hp) = fb.hover_pos() {
                if fb.dragged() {
                    ui.ctx().set_cursor_icon(match fbar_drag {
                        Some(FbarDrag::Move(..)) => egui::CursorIcon::Grabbing,
                        _ => egui::CursorIcon::ResizeVertical,
                    });
                } else {
                    ui.ctx().set_cursor_icon(match grip(hp.y) {
                        Some(FbarDrag::Top) | Some(FbarDrag::Bottom) => egui::CursorIcon::ResizeVertical,
                        Some(FbarDrag::Move(..)) => egui::CursorIcon::Grab,
                        None => egui::CursorIcon::PointingHand,
                    });
                }
            }
            if fb.drag_started() {
                // Decide from where the button went down, not where the pointer is now.
                let y0 = ui.input(|i| i.pointer.press_origin()).map_or(box_top, |p| p.y);
                fbar_drag = Some(grip(y0).unwrap_or_else(|| {
                    // Outside the box: jump there, then move it.
                    let wu = (fbar_u(f_hi) - fbar_u(f_lo)) as f32;
                    FbarDrag::Move(wu / 2.0, wu)
                }));
            }
            if let (true, Some(pp)) = (fb.dragged(), fb.interact_pointer_pos()) {
                let f = fb_f(pp.y);
                match fbar_drag {
                    Some(FbarDrag::Top) => {
                        let hi = f.max(f_lo + MIN_FSPAN);
                        fspan = hi - f_lo;
                        fcenter = f_lo + fspan / 2.0;
                    }
                    Some(FbarDrag::Bottom) => {
                        let lo = f.min(f_hi - MIN_FSPAN);
                        fspan = f_hi - lo;
                        fcenter = lo + fspan / 2.0;
                    }
                    Some(FbarDrag::Move(du, wu)) => {
                        // The box keeps its size on the bar, so it stays under the pointer.
                        let u_lo = (fb_u(pp.y) - du as f64).clamp(0.0, 1.0 - wu as f64);
                        let (lo, hi) = (fbar_f(u_lo), fbar_f(u_lo + wu as f64));
                        fspan = (hi - lo).clamp(MIN_FSPAN, FULL_MAX);
                        fcenter = (lo + hi) / 2.0;
                    }
                    None => {}
                }
            }
            if fb.clicked() {
                if let Some(pp) = fb.interact_pointer_pos() {
                    if grip(pp.y).is_none() {
                        fcenter = fb_f(pp.y);
                    }
                }
            }
            if fb.drag_stopped() {
                fbar_drag = None;
            }
            {
                let p = ui.painter();
                let dim = 1.0;
                p.rect_filled(fbar_rect, 2.0, FIELD);
                // The planes module's band, faint.
                let pb = Rect::from_min_max(pos2(fbar_rect.left(), fb_y(s.freqs[nb - 1])), pos2(fbar_rect.right(), fb_y(s.freqs[0])));
                p.rect_filled(pb, 0.0, GRID.gamma_multiply(0.6 * dim));
                for pv in s.plugins.iter().filter(|pv| !hidden_plugins.contains(&pv.path)) {
                    for f in &pv.findings {
                        let y = fb_y(0.5 * (f.f_lo + f.f_hi));
                        p.line_segment([pos2(fbar_rect.left() + 2.0, y), pos2(fbar_rect.right() - 2.0, y)], Stroke::new(2.0, pv.color.gamma_multiply(dim)));
                    }
                }
                for e in s.events.iter().filter(|_| show_planes) {
                    let (y0, y1) = (fb_y(e.f_pre), fb_y(e.f_post));
                    let r = Rect::from_min_max(pos2(fbar_rect.left() + 3.0, y0.min(y1 - 2.0)), pos2(fbar_rect.right() - 3.0, y1));
                    p.rect_filled(r, 0.0, grade_color(e.grade()).gamma_multiply(dim));
                }
                for f in [20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0] {
                    let y = fb_y(f);
                    p.line_segment([pos2(fbar_rect.left() - 3.0, y), pos2(fbar_rect.left(), y)], Stroke::new(1.0, DIM));
                    let lbl = if f >= 1000.0 { format!("{:.0}k", f / 1000.0) } else { format!("{f:.0}") };
                    p.text(pos2(fbar_rect.left() - 5.0, y), Align2::RIGHT_CENTER, lbl, mono(9.5), DIM);
                }
                let bx = Rect::from_min_max(pos2(fbar_rect.left(), box_top), pos2(fbar_rect.right(), box_bot.max(box_top + 3.0)));
                p.rect_stroke(bx, 1.0, Stroke::new(1.5, HEAD), egui::StrokeKind::Inside);
                p.rect_stroke(fbar_rect, 2.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);
                p.text(pos2(fbar_rect.center().x, fbar_rect.top() - 2.0), Align2::CENTER_BOTTOM, "Hz", mono(9.5), DIM);
            }
            let fb_hover = fb.hovered();

            // Wheel over the plot or the bar: wheel = frequency zoom (at the pointer), Alt+wheel =
            // frequency up/down, Ctrl+wheel = time span, Shift+wheel or sideways = time scroll.
            // Click (files) sets the playhead.
            let spec_resp = ui.interact(spec_rect, Id::unique("spec"), Sense::click());
            if spec_resp.hovered() || fb_hover {
                let (d, zoom, alt, ptr) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers.alt, i.pointer.hover_pos()));
                let over_spec = spec_resp.hovered();
                let over_plot = over_spec;
                // Ctrl+wheel arrives as a zoom factor (> 1 zooms in).
                if zoom != 1.0 {
                    let new = (span / zoom as f64).clamp(10.0, t_max);
                    new_tspan = Some(new);
                    if !self.follow {
                        // Keep the time under the pointer where it is.
                        let tc = match ptr {
                            Some(pp) if over_plot => t_of(pp.x),
                            _ => t_right - span / 2.0,
                        };
                        new_view = Some(tc + (t_right - tc) * new / span);
                    }
                }
                if d.x != 0.0 {
                    new_view = Some(t_right - d.x as f64 / spec_rect.width() as f64 * span);
                }
                if d.y != 0.0 {
                    if alt {
                        fcenter += d.y as f64 / spec_rect.height() as f64 * fspan;
                    } else {
                        let new = (fspan * (-(d.y as f64) * 0.004).exp()).clamp(MIN_FSPAN, FULL_MAX);
                        // Keep the frequency under the pointer where it is.
                        let fc = match ptr {
                            Some(pp) if over_spec => f_lo + ((spec_rect.bottom() - pp.y) / spec_rect.height()) as f64 * fspan,
                            _ => fcenter,
                        };
                        fcenter = fc + (fcenter - fc) * new / fspan;
                        fspan = new;
                    }
                }
            }
            if is_file && spec_resp.clicked() {
                if let Some(pp) = spec_resp.interact_pointer_pos() {
                    seek_to = Some(t_of(pp.x).clamp(t_first, s.now));
                }
            }

            let p = ui.painter();
            p.rect_filled(spec_rect, 0.0, FIELD);
            let clip = p.with_clip_rect(spec_rect);
            let at_edge = (s.now - t_right).abs() < HOP_S * 1.5;

            {
                let y_of = |f: f64| spec_rect.bottom() - ((f - f_lo) / (f_hi - f_lo)) as f32 * spec_rect.height();
                // Coarse full band first, then the fine planes layer where it is sharper.
                if let Some(t) = &self.tex_full {
                    let v = |f: f64| (1.0 - f / FULL_MAX) as f32;
                    let rect = Rect::from_min_max(pos2(x_of(t_img_full - tex_frames * HOP_S + HOP_S), spec_rect.top()), pos2(x_of(t_img_full + HOP_S), spec_rect.bottom()));
                    clip.image(t.id(), rect, Rect::from_min_max(pos2(0.0, v(f_hi)), pos2(1.0, v(f_lo))), Color32::WHITE);
                }
                let (ff_lo, ff_hi) = (s.freqs[0], s.freqs[nb - 1]);
                if fine_visible(fspan, f_lo, f_hi, &s.freqs) {
                    if let Some(t) = &self.tex {
                        let (a, b) = (f_lo.max(ff_lo), f_hi.min(ff_hi));
                        let v = |f: f64| ((ff_hi - f) / (ff_hi - ff_lo)) as f32;
                        let rect = Rect::from_min_max(pos2(x_of(t_img - tex_frames * HOP_S + HOP_S), y_of(b)), pos2(x_of(t_img + HOP_S), y_of(a)));
                        clip.image(t.id(), rect, Rect::from_min_max(pos2(0.0, v(b)), pos2(1.0, v(a))), Color32::WHITE);
                    }
                }

                // Frequency grid and labels at a step that suits the zoom.
                let step = nice_step(f_hi - f_lo, 8.0);
                let mut f = (f_lo / step).ceil() * step;
                while f <= f_hi {
                    let y = y_of(f);
                    p.line_segment([pos2(spec_rect.left(), y), pos2(spec_rect.right(), y)], Stroke::new(0.5, GRID.gamma_multiply(0.6)));
                    p.line_segment([pos2(slice_rect.left(), y), pos2(slice_rect.right(), y)], Stroke::new(0.5, GRID));
                    let dec = (-step.log10().floor()).max(0.0) as usize;
                    let lbl = if f >= 1000.0 { format!("{:.1}k", f / 1000.0) } else { format!("{:.*}", dec, f) };
                    p.text(pos2(spec_rect.left() - 6.0, y), Align2::RIGHT_CENTER, lbl, mono(10.5), DIM);
                    f += step;
                }
                p.text(pos2(spec_rect.left() - 6.0, spec_rect.top() - 2.0), Align2::RIGHT_BOTTOM, "Hz", mono(10.5), DIM);

                if at_edge && !s.done && show_planes {
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
                for e in s.events.iter().filter(|_| show_planes) {
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

                // Plugins: their measured frequency lines, dashed over the spectrogram at every zoom,
                // so the raw spectrogram shows between the dashes. Gaps over 2.5 s break the line.
                for pv in s.plugins.iter().filter(|pv| !hidden_plugins.contains(&pv.path)) {
                    let stroke = Stroke::new(1.5, pv.color);
                    for (_, track) in &pv.tracks {
                        let mut line: Vec<Pos2> = Vec::new();
                        let mut last_t: Option<f64> = None;
                        let a = track.partition_point(|q| q.0 < t_left - 2.0);
                        for &(t, f, ok) in &track[a..] {
                            if t > t_right + 2.0 {
                                break;
                            }
                            if !ok {
                                continue;
                            }
                            if last_t.is_some_and(|lt| t - lt > 2.5) {
                                if line.len() > 1 {
                                    clip.extend(Shape::dashed_line(&line, stroke, 5.0, 5.0));
                                }
                                line.clear();
                            }
                            if f >= f_lo && f <= f_hi {
                                line.push(pos2(x_of(t), y_of(f)));
                            }
                            last_t = Some(t);
                        }
                        if line.len() > 1 {
                            clip.extend(Shape::dashed_line(&line, stroke, 5.0, 5.0));
                        }
                    }
                }

                // Findings outside the visible band: arrows pinned to the top or bottom edge.
                let mut marks: Vec<(f32, bool, Color32, ViewAct, f64)> = Vec::new();
                for e in s.events.iter().filter(|_| show_planes) {
                    if e.t_cpa < t_left || e.t_cpa > t_right {
                        continue;
                    }
                    if e.f_post > f_hi {
                        marks.push((x_of(e.t_cpa), true, grade_color(e.grade()), ViewAct::Planes, e.t_cpa));
                    } else if e.f_pre < f_lo {
                        marks.push((x_of(e.t_cpa), false, grade_color(e.grade()), ViewAct::Planes, e.t_cpa));
                    }
                }
                for (pi, pv) in s.plugins.iter().enumerate().filter(|(_, pv)| !hidden_plugins.contains(&pv.path)) {
                    for f in &pv.findings {
                        if f.t_end < t_left || f.t_start > t_right {
                            continue;
                        }
                        let x = x_of(0.5 * (f.t_start.max(t_left) + f.t_end.min(t_right)));
                        let tm = 0.5 * (f.t_start + f.t_end);
                        if f.f_lo > f_hi {
                            marks.push((x, true, pv.color, ViewAct::Plugin(pi), tm));
                        } else if f.f_hi < f_lo {
                            marks.push((x, false, pv.color, ViewAct::Plugin(pi), tm));
                        }
                    }
                }
                for (i, (x, top, col, act, _t)) in marks.iter().enumerate() {
                    let y = if *top { spec_rect.top() + 2.0 } else { spec_rect.bottom() - 2.0 };
                    let (tip, base) = if *top { (y, y + 9.0) } else { (y, y - 9.0) };
                    let tri = vec![pos2(*x, tip), pos2(*x - 6.0, base), pos2(*x + 6.0, base)];
                    clip.add(Shape::convex_polygon(tri, *col, Stroke::new(1.0, Color32::BLACK)));
                    let hit = Rect::from_center_size(pos2(*x, (tip + base) / 2.0), vec2(16.0, 14.0));
                    let r = ui.interact(hit, Id::unique(("edge", i)), Sense::click());
                    let what = match act {
                        ViewAct::Planes => "plane pass".to_string(),
                        ViewAct::Plugin(k) => format!("{} finding", s.plugins.get(*k).map_or("plugin", |pv| pv.name.as_str())),
                    };
                    if r.on_hover_text(format!("{what} {} the view: click to show it", if *top { "above" } else { "below" })).clicked() {
                        view_act = Some(*act);
                    }
                }
                let (n_up, n_dn) = (marks.iter().filter(|m| m.1).count(), marks.iter().filter(|m| !m.1).count());
                if n_up > 0 {
                    p.text(pos2(spec_rect.right() - 4.0, spec_rect.top() + 2.0), Align2::RIGHT_TOP, format!("▲ {n_up} above"), mono(10.5), MED);
                }
                if n_dn > 0 {
                    p.text(pos2(spec_rect.right() - 4.0, spec_rect.bottom() - 2.0), Align2::RIGHT_BOTTOM, format!("▼ {n_dn} below"), mono(10.5), MED);
                }

                // Spectrum slice: the visible layer at the playhead (files) or right edge.
                if show_slice {
                p.rect_filled(slice_rect, 0.0, FIELD);
                let slice_t = if is_file { head } else { t_right };
                let x_of_z = |z: f32| slice_rect.left() + ((z - Z_LO) / (Z_HI - Z_LO + 1.0)).clamp(0.0, 1.0) * slice_rect.width();
                let mut pts: Vec<Pos2> = Vec::new();
                let fi = s.full_t.partition_point(|&t| t <= slice_t);
                if fi > 0 {
                    let row = &s.full_hist[(fi - 1) * fnb..fi * fnb];
                    pts.extend(row.iter().enumerate().filter(|(i, _)| {
                        let f = s.full_freqs[*i];
                        f >= f_lo && f <= f_hi && !(fine_visible(fspan, f_lo, f_hi, &s.freqs) && f >= ff_lo && f <= ff_hi)
                    }).map(|(i, &q)| pos2(x_of_z(dequantise(q)), y_of(s.full_freqs[i]))));
                }
                if fine_visible(fspan, f_lo, f_hi, &s.freqs) {
                    let si = s.hist_t.partition_point(|&t| t <= slice_t);
                    if si > 0 {
                        let row = &s.hist[(si - 1) * nb..si * nb];
                        let mut fine: Vec<Pos2> = row
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| s.freqs[*i] >= f_lo && s.freqs[*i] <= f_hi)
                            .map(|(i, &q)| pos2(x_of_z(dequantise(q)), y_of(s.freqs[i])))
                            .collect();
                        pts.append(&mut fine);
                    }
                }
                pts.sort_by(|a, b| b.y.total_cmp(&a.y));
                if pts.len() > 1 {
                    p.with_clip_rect(slice_rect).add(Shape::line(pts, Stroke::new(1.0, WAVE)));
                }
                }
                if show_slice {
                    let x0 = slice_rect.left() + (-Z_LO / (Z_HI - Z_LO + 1.0)) * slice_rect.width();
                    p.line_segment([pos2(x0, slice_rect.top()), pos2(x0, slice_rect.bottom())], Stroke::new(0.5, DIM));
                }

                // Minimap: the whole band for the same time span; frame = visible band; click/drag to move.
                if show_minimap {
                    let mini = Rect::from_min_size(spec_rect.left_top() + vec2(6.0, 6.0), vec2(170.0, 120.0));
                    p.rect_filled(mini, 2.0, FIELD);
                    if let Some(t) = &self.tex_full {
                        p.image(t.id(), mini, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    let my = |f: f64| mini.bottom() - (f / FULL_MAX) as f32 * mini.height();
                    let mx = |t: f64| mini.left() + ((t - t_left) / span) as f32 * mini.width();
                    for e in s.events.iter().filter(|_| show_planes) {
                        if e.t_cpa >= t_left && e.t_cpa <= t_right {
                            p.circle_filled(pos2(mx(e.t_cpa), my(e.f0)), 2.5, grade_color(e.grade()));
                        }
                    }
                    for pv in s.plugins.iter().filter(|pv| !hidden_plugins.contains(&pv.path)) {
                        for f in &pv.findings {
                            if f.t_end >= t_left && f.t_start <= t_right {
                                let y = my(0.5 * (f.f_lo + f.f_hi));
                                p.line_segment([pos2(mx(f.t_start.max(t_left)), y), pos2(mx(f.t_end.min(t_right)), y)], Stroke::new(2.0, pv.color));
                            }
                        }
                    }
                    let frame = Rect::from_min_max(pos2(mini.left(), my(f_hi)), pos2(mini.right(), my(f_lo)));
                    p.rect_stroke(frame, 0.0, Stroke::new(1.5, HEAD), egui::StrokeKind::Inside);
                    p.rect_stroke(mini, 2.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);
                    p.text(mini.left_bottom() + vec2(3.0, -2.0), Align2::LEFT_BOTTOM, "0–8 kHz", mono(9.5), DIM);
                    let r = ui.interact(mini, Id::unique("minimap"), Sense::click_and_drag());
                    if let Some(pp) = r.interact_pointer_pos() {
                        if r.clicked() || r.dragged() {
                            fcenter = ((mini.bottom() - pp.y) / mini.height()) as f64 * FULL_MAX;
                        }
                    }
                }
            }

            // Time grid on absolute marks, so it moves with the data; labels are stream time.
            let tstep = time_step(span);
            let mut tg = (t_left / tstep).ceil() * tstep;
            while tg <= t_right {
                let x = x_of(tg);
                p.line_segment([pos2(x, spec_rect.top()), pos2(x, spec_rect.bottom())], Stroke::new(0.5, GRID.gamma_multiply(0.6)));
                if x > plot_rect.left() + 24.0 && x < plot_rect.right() - 40.0 {
                    p.text(pos2(x, plot_rect.bottom() + 4.0), Align2::CENTER_TOP, stamp(&s, tg), mono(10.5), DIM);
                }
                tg += tstep;
            }
            let edge = if at_edge { "now".to_string() } else { format!("{} ◂", stamp(&s, t_right)) };
            p.text(pos2(plot_rect.right(), plot_rect.bottom() + 4.0), Align2::RIGHT_TOP, edge, mono(10.5), if at_edge { HIGH } else { MED });

            // Playhead (files).
            if is_file && head >= t_left && head <= t_right {
                let x = x_of(head);
                clip.line_segment([pos2(x, spec_rect.top()), pos2(x, spec_rect.bottom())], Stroke::new(1.0, HEAD));
                clip.text(pos2(x + 4.0, spec_rect.top() + 2.0), Align2::LEFT_TOP, if self.audio { "▶" } else { "▮" }, mono(11.0), HEAD);
            }
            p.rect_stroke(spec_rect, 0.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);
            if show_slice {
                p.rect_stroke(slice_rect, 0.0, Stroke::new(1.0, GRID), egui::StrokeKind::Outside);
                p.text(
                    pos2(slice_rect.center().x, slice_rect.bottom() + 4.0),
                    Align2::CENTER_TOP,
                    if is_file { "z at ▮" } else if at_edge { "z now" } else { "z at edge" },
                    mono(10.5),
                    DIM,
                );
            }

            // Timeline bar: everything kept; the thumb is the visible window. Click or drag to move.
            let total = (s.now - t_first).max(span);
            let bx = |t: f64| bar_rect.left() + ((t - t_first) / total) as f32 * bar_rect.width();
            let bar = ui.interact(bar_rect, Id::unique("timeline"), Sense::click_and_drag());
            p.rect_filled(bar_rect, 2.0, FIELD);
            for pv in s.plugins.iter().filter(|pv| !hidden_plugins.contains(&pv.path)) {
                for f in &pv.findings {
                    if f.t_end >= t_first {
                        let (a, b) = (bx(f.t_start.max(t_first)), bx(f.t_end));
                        p.line_segment([pos2(a, bar_rect.bottom() - 2.0), pos2(b, bar_rect.bottom() - 2.0)], Stroke::new(2.0, pv.color));
                    }
                }
            }
            for e in s.events.iter().filter(|_| show_planes) {
                if e.t_cpa >= t_first {
                    let x = bx(e.t_cpa);
                    p.line_segment([pos2(x, bar_rect.top() + 2.0), pos2(x, bar_rect.bottom() - 4.0)], Stroke::new(2.0, grade_color(e.grade())));
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
            let lc = if self.follow { HIGH } else if live_btn.hovered() { text() } else { MED };
            p.rect_stroke(live_rect, 2.0, Stroke::new(1.0, lc), egui::StrokeKind::Inside);
            let lbl = match (is_file, self.follow) {
                (false, true) => "● LIVE",
                (false, false) => "⏵ GO LIVE",
                (true, true) => "● FOLLOW",
                (true, false) => "⏵ FOLLOW",
            };
            p.text(live_rect.center(), Align2::CENTER_CENTER, lbl, mono(11.0), lc);
            if live_btn.clicked() {
                go_live = true;
            }
        });
        self.fcenter = fcenter;
        self.fspan = fspan;
        self.fbar_drag = fbar_drag;
        if let Some(v) = new_tspan {
            self.tspan = v;
        }
        match view_act {
            Some(ViewAct::Planes) => self.view_planes(),
            Some(ViewAct::Plugin(i)) => self.view_plugin(&s, i, None),
            None => {}
        }
        if let Some((i, f)) = snap_plugin {
            self.view_plugin(&s, i, Some(&f));
            self.follow = false;
            self.view_right = 0.5 * (f.t_start + f.t_end) + span * 0.5;
        }
        // Files: keep `<file>.doppler.json` current: once the analysis ends, and again whenever a
        // plugin or the manual data adds to it.
        if let (Some(path), true) = (self.sess.file.clone(), s.done) {
            let mut h = std::hash::DefaultHasher::new();
            s.events.len().hash(&mut h);
            for pv in &s.plugins {
                (pv.findings.len(), pv.meta.len(), &pv.state).hash(&mut h);
            }
            self.manual.hash(&mut h);
            let key = h.finish();
            if key != self.result_key {
                self.result_key = key;
                let out = format!("{path}.doppler.json");
                self.result_note = Some(match std::fs::write(&out, file_result_json(&s, &path, &self.manual)) {
                    Ok(()) => format!("result → {}", out.rsplit(['/', '\\']).next().unwrap_or(&out)),
                    Err(e) => format!("result not written: {out}: {e}"),
                });
            }
        }
        let (now, offset) = (s.now, s.offset);
        let s_has_source = s.live || is_file;
        drop(s);
        if self.manual != manual_before {
            // Recordings read it from the session when they write their JSON.
            self.sess.sh.lock().unwrap().manual = self.manual.clone();
        }

        if let Some((id, tc)) = snap {
            // Closest approach a little left of centre, so the approach and the fall both show.
            self.selected = Some(id);
            self.follow = false;
            self.view_right = tc + span * 0.6;
            self.view_planes();
            if is_file {
                // Start listening a little before the closest approach.
                self.seek(tc - 20.0, offset);
            }
        }
        // Arrow keys move the playhead (files) or the view (streams: back into the kept timeline).
        if key_nudge != 0.0 {
            if is_file {
                seek_to = Some((head + key_nudge).clamp(t_first, now));
            } else {
                new_view = Some((t_right + key_nudge).clamp(t_first, now));
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
        // Audio on by default once there is something to hear.
        let has_source = s_has_source;
        let audio = if self.autostart_audio && has_source {
            self.autostart_audio = false;
            true
        } else {
            audio
        };
        let audio = if space_pressed { !audio } else { audio };
        if !audio && self.audio {
            // User turned it off: do not turn it back on at the next source.
            self.autostart_audio = false;
        }
        self.apply_toggles(smooth, audio, record, offset);
        if minimap != self.settings.minimap {
            self.settings.minimap = minimap;
            self.settings.save();
        }
        match action {
            Some(Action::OpenFile) => self.open_file_dialog(),
            Some(Action::OpenStream) => {
                self.stream_dialog = true;
                self.open_error = None;
            }
            Some(Action::OpenMic) => {
                match ingest::list_microphones() {
                    Ok(names) => {
                        self.mic_devices = names;
                        self.mic_pick = 0;
                        self.mic_error = None;
                    }
                    Err(e) => {
                        self.mic_devices.clear();
                        self.mic_error = Some(e);
                    }
                }
                self.mic_dialog = true;
            }
            Some(Action::ImportPlugin) => {
                let pick = rfd::FileDialog::new()
                    .set_title("Import plugin: a program that reads audio on stdin")
                    .add_filter("program", &["exe"])
                    .add_filter("all files", &["*"])
                    .pick_file();
                if let Some(path) = pick {
                    let path = path.to_string_lossy().into_owned();
                    if !self.settings.plugins.iter().any(|(q, _)| *q == path) {
                        self.settings.plugins.push((path, true));
                    }
                    self.plugins_changed();
                }
            }
            Some(Action::TogglePlugin(i)) => {
                if let Some(e) = self.settings.plugins.get_mut(i) {
                    e.1 = !e.1;
                }
                self.plugins_changed();
            }
            Some(Action::RemovePlugin(i)) => {
                if i < self.settings.plugins.len() {
                    self.settings.plugins.remove(i);
                }
                self.plugins_changed();
            }
            Some(Action::FullView) => {
                self.fspan = FULL_MAX;
                self.fcenter = FULL_MAX / 2.0;
            }
            Some(Action::Theme(l)) => {
                LIGHT.store(l, Ordering::Relaxed);
                apply_theme(ctx);
                self.settings.light = l;
                self.settings.save();
            }
            None => {}
        }
    }
}

// ---------------------------------------------------------------- view helpers

/// Quick views the panel, toolbar and edge arrows can ask for.
#[derive(Clone, Copy, PartialEq)]
enum ViewAct {
    Planes,
    /// A plugin's view, by its index in the session.
    Plugin(usize),
}

/// Top of the full-band layer (Nyquist frequency of the 16 kHz stream).
const FULL_MAX: f64 = 8000.0;
/// Narrowest frequency view (Hz).
const MIN_FSPAN: f64 = 0.2;
/// Widest spectrogram texture (columns); wider time views pool frames.
const MAX_TEX_W: usize = 2048;
/// The fine planes layer is drawn when zoomed in enough for its detail to matter and when
/// its band (40–450 Hz) is in view.
fn fine_visible(fspan: f64, f_lo: f64, f_hi: f64, freqs: &[f64]) -> bool {
    fspan <= 2000.0 && f_hi > freqs[0] && f_lo < freqs[freqs.len() - 1]
}

/// Source icon for a file: a page with a folded corner and two text lines.
fn file_icon(ui: &mut egui::Ui, col: Color32) {
    let (r, resp) = ui.allocate_exact_size(vec2(11.0, 14.0), Sense::hover());
    resp.on_hover_text("File: a recording opened from disk");
    let p = ui.painter();
    let f = 4.0;
    let page = vec![r.left_top(), pos2(r.right() - f, r.top()), pos2(r.right(), r.top() + f), r.right_bottom(), r.left_bottom(), r.left_top()];
    p.add(Shape::line(page, Stroke::new(1.2, col)));
    p.add(Shape::line(
        vec![pos2(r.right() - f, r.top()), pos2(r.right() - f, r.top() + f), pos2(r.right(), r.top() + f)],
        Stroke::new(1.0, col),
    ));
    for y in [r.top() + 7.0, r.top() + 10.5] {
        p.line_segment([pos2(r.left() + 2.5, y), pos2(r.right() - 2.5, y)], Stroke::new(1.0, col));
    }
}

/// Source icon for live audio: a red dot that blinks.
fn live_dot(ui: &mut egui::Ui, on: bool) {
    let (r, resp) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
    resp.on_hover_text("Live: audio arriving now");
    let col = if on { REC } else { REC.gamma_multiply(0.2) };
    ui.painter().circle_filled(r.center(), 4.0, col);
}

/// A module's box in the right panel: a coloured header strip (name and what it finds) above its
/// contents. Clicking the header shows the module's view. Returns true when it was clicked.
fn module_box(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    title: &str,
    what: &str,
    col: Color32,
    eye: Option<bool>,
    body: impl FnOnce(&mut egui::Ui),
) -> (bool, bool) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter().clone();
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), 34.0));
    let hr = ui.interact(head, Id::unique(("module_head", title)), Sense::click());
    // Boxes without an eye (manual data) have no view to show.
    let hover = hr.hovered() && eye.is_some();
    p.rect_filled(rect, 3.0, bg());
    p.rect_filled(head, 3.0, col.gamma_multiply(if hover { 0.28 } else { 0.14 }));
    p.rect_filled(Rect::from_min_size(head.min, vec2(4.0, head.height())), 0.0, col);
    p.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, GRID));
    p.text(head.min + vec2(12.0, 4.0), Align2::LEFT_TOP, title, mono(13.0), text());
    p.text(head.min + vec2(12.0, 20.0), Align2::LEFT_TOP, what, mono(10.0), DIM);
    let clicked = eye.is_some() && hr.on_hover_text(format!("Click to show the {title} view")).clicked();
    // Eye: hides the module on the graph, bars and minimap; its list stays.
    let eye_clicked = match eye {
        Some(visible) => {
            let eye_rect = Rect::from_center_size(pos2(head.right() - 16.0, head.center().y), vec2(22.0, 16.0));
            let er = ui.interact(eye_rect, Id::unique(("module_eye", title)), Sense::click());
            let eye_col = if !visible { DIM } else if er.hovered() { text() } else { col };
            eye_icon(&p, eye_rect.center(), visible, eye_col);
            er.on_hover_text(if visible { format!("Hide {title} on the graph") } else { format!("Show {title} on the graph") })
                .clicked()
        }
        None => false,
    };
    let inner = Rect::from_min_max(pos2(rect.left() + 8.0, head.bottom() + 6.0), pos2(rect.right() - 8.0, rect.bottom() - 6.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(inner);
    body(&mut child);
    p.rect_stroke(rect, 3.0, Stroke::new(1.0, GRID), egui::StrokeKind::Inside);
    (clicked, eye_clicked)
}

/// An eye: an almond outline with a pupil when open, with a slash when shut.
fn eye_icon(p: &egui::Painter, c: Pos2, open: bool, col: Color32) {
    let (w, h) = (7.0f32, 4.5f32);
    let arc = |sign: f32| (0..=16).map(move |i| {
        let u = -1.0 + 2.0 * i as f32 / 16.0;
        pos2(c.x + u * w, c.y + sign * h * (1.0 - u * u))
    });
    let mut outline: Vec<Pos2> = arc(-1.0).collect();
    outline.extend(arc(1.0).collect::<Vec<_>>().into_iter().rev());
    outline.push(outline[0]);
    p.add(Shape::line(outline, Stroke::new(1.5, col)));
    if open {
        p.circle_filled(c, 2.2, col);
    } else {
        p.line_segment([pos2(c.x - w, c.y + h + 2.0), pos2(c.x + w, c.y - h - 2.0)], Stroke::new(1.5, col));
    }
}

/// Frequency bar scale: log from 10 Hz to 8 kHz, as a 0–1 position.
fn fbar_u(f: f64) -> f64 {
    ((f.max(10.0) / 10.0).ln() / (FULL_MAX / 10.0).ln()).clamp(0.0, 1.0)
}

/// Inverse of `fbar_u`.
fn fbar_f(u: f64) -> f64 {
    10.0 * ((FULL_MAX / 10.0).ln() * u.clamp(0.0, 1.0)).exp()
}

/// Time grid step (s) giving at most about 10 lines over `span`.
fn time_step(span: f64) -> f64 {
    const STEPS: [f64; 13] = [1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1200.0, 1800.0, 3600.0];
    STEPS.iter().copied().find(|st| span / st <= 10.0).unwrap_or(7200.0)
}

/// A time span as "45 s", "8.0 min" or "1.50 h".
fn fmt_span(t: f64) -> String {
    if t < 120.0 {
        format!("{t:.0} s")
    } else if t < 7200.0 {
        format!("{:.1} min", t / 60.0)
    } else {
        format!("{:.2} h", t / 3600.0)
    }
}

/// A 1–2–5 step giving about `n` grid lines over `span`.
fn nice_step(span: f64, n: f64) -> f64 {
    let raw = (span / n).max(1e-9);
    let mag = 10f64.powf(raw.log10().floor());
    let r = raw / mag;
    let m = if r < 1.5 {
        1.0
    } else if r < 3.5 {
        2.0
    } else if r < 7.5 {
        5.0
    } else {
        10.0
    };
    m * mag
}

/// Fill `img` (RGBA, `cols` × `nb`, high frequencies on top) from the `cols × stride` frames of
/// `hist` ending before index `idx_r`. Each column is the maximum of its `stride` frames;
/// frames before the start are left dark.
fn fill_image(img: &mut Vec<u8>, lut: &[[u8; 4]; 256], hist: &[u8], nb: usize, cols: usize, stride: usize, idx_r: usize) {
    img.resize(cols * nb * 4, 0);
    let mut col_max = vec![0u8; nb];
    for col in 0..cols {
        let f0 = idx_r as isize - ((cols - col) * stride) as isize;
        let mut any = false;
        col_max.fill(0);
        for k in 0..stride as isize {
            let fi = f0 + k;
            if fi < 0 {
                continue;
            }
            any = true;
            let row = &hist[fi as usize * nb..(fi as usize + 1) * nb];
            for (m, &v) in col_max.iter_mut().zip(row) {
                *m = (*m).max(v);
            }
        }
        for row in 0..nb {
            let px = if any { lut[col_max[nb - 1 - row] as usize] } else { [4, 5, 8, 255] };
            let i = (row * cols + col) * 4;
            img[i..i + 4].copy_from_slice(&px);
        }
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
                if analysis { "analysis head · " } else { "" },
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
    LIGHT.store(Settings::load().light, Ordering::Relaxed);
    let current = if stdin {
        Some((Source::Stdin, "stdin".to_string()))
    } else {
        input.map(|i| (source_for(&i, start), short_label(&i)))
    };
    let session = match &current {
        Some((src, label)) => Session::open(src.clone(), label.clone(), Settings::load().enabled_plugins()),
        None => Session::idle(),
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1480.0, 920.0]).with_title("doppler-view"),
        ..Default::default()
    };
    eframe::run_native("doppler-view", options, Box::new(move |cc| Ok(Box::new(App::new(cc, session, current)))))
        .map_err(|e| e.to_string())
}

/// A path or URL as a source: WAV read directly, anything else through ffmpeg.
fn source_for(input: &str, start: Option<f64>) -> Source {
    if input.to_ascii_lowercase().ends_with(".wav") && start.is_none() && !input.contains("://") {
        Source::Wav(input.to_string())
    } else {
        Source::Ffmpeg(input.to_string(), start)
    }
}

/// File name, or the URL shortened, for the top bar.
fn short_label(input: &str) -> String {
    let name = if input.contains("://") { input } else { input.rsplit(['/', '\\']).next().unwrap_or(input) };
    if name.chars().count() > 60 { format!("{}…", name.chars().take(60).collect::<String>()) } else { name.to_string() }
}

/// One source being analysed: shared state, controls, audio ring, and the worker thread.
struct Session {
    sh: Arc<Mutex<Shared>>,
    ctl: Arc<Controls>,
    ring: Ring,
    /// File path (for playback), or None for live sources and the idle screen.
    file: Option<String>,
}

impl Session {
    fn shared(label: String, live: bool, offset: f64) -> Shared {
        let cfg = Config::default();
        Shared {
            sr: cfg.sr,
            offset,
            source: label,
            freqs: Detector::new(cfg.clone()).bin_freqs(),
            wave: vec![0.0; (WAVE_S * PLAY_SR) as usize],
            wave_head: 0,
            wave_n: 0,
            cols: (cfg.window_s * cfg.sr / cfg.hop as f64) as usize,
            hist: Vec::new(),
            hist_t: Vec::new(),
            frames_total: 0,
            now: 0.0,
            frame_wall: Instant::now(),
            ridges: Vec::new(),
            pending: Vec::new(),
            events: Vec::new(),
            full_freqs: (0..=FULL_NFFT / 2).map(|i| i as f64 * PLAY_SR / FULL_NFFT as f64).collect(),
            full_hist: Vec::new(),
            full_t: Vec::new(),
            plugins: Vec::new(),
            manual: Vec::new(),
            module_desc: Vec::new(),
            dsp_s: 0.0,
            audio_s: 0.0,
            wall_s: 0.0,
            done: false,
            live,
            rec_path: None,
            rec_s: 0.0,
            rec_format: String::new(),
            anchor: None,
            status: String::new(),
            c: cfg.c,
        }
    }

    fn controls() -> Arc<Controls> {
        Arc::new(Controls {
            smooth: AtomicBool::new(true),
            audio: AtomicBool::new(false),
            record: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            // Replaced by the app's saved settings as soon as the session is handed over.
            rec: Mutex::new(Settings::load().rec),
        })
    }

    /// Nothing open yet.
    fn idle() -> Self {
        let mut s = Self::shared("no source".into(), false, 0.0);
        s.done = true;
        s.status = "open a source: Source ▸ Open file… / Open stream…".into();
        Self { sh: Arc::new(Mutex::new(s)), ctl: Self::controls(), ring: Default::default(), file: None }
    }

    fn open(src: Source, label: String, plugins: Vec<String>) -> Self {
        let live = src.is_live();
        let offset = if let Source::Ffmpeg(_, Some(s)) = &src { *s } else { 0.0 };
        let file = match &src {
            Source::Wav(p) | Source::Ffmpeg(p, _) if !live => Some(p.clone()),
            _ => None,
        };
        let mut shared = Self::shared(label, live, offset);
        shared.plugins = plugins.iter().map(|p| PluginView::new(p)).collect();
        let sh = Arc::new(Mutex::new(shared));
        let ctl = Self::controls();
        let ring: Ring = Default::default();
        {
            let (sh, ctl, ring) = (sh.clone(), ctl.clone(), ring.clone());
            thread::spawn(move || worker(sh, src, ctl, ring, plugins));
        }
        Self { sh, ctl, ring, file }
    }

    /// Tell the worker to finish; it closes ffmpeg and any recording.
    fn close(&self) {
        self.ctl.record.store(false, Ordering::Relaxed);
        self.ctl.audio.store(false, Ordering::Relaxed);
        self.ctl.stop.store(true, Ordering::Relaxed);
    }
}

/// YouTube page links need resolving to a direct stream URL (yt-dlp); others are used as-is.
fn needs_resolving(url: &str) -> bool {
    url.contains("youtube.com/") || url.contains("youtu.be/")
}

fn resolve_with_ytdlp(url: &str) -> Result<String, String> {
    let out = doppler_live::no_window(&mut std::process::Command::new("yt-dlp"))
        .args(["--no-warnings", "-g", "-f", "ba/b", url])
        .output()
        .map_err(|e| format!("could not run yt-dlp (is it installed?): {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match text.lines().next() {
        Some(u) if out.status.success() && u.contains("://") => Ok(u.trim().to_string()),
        _ => Err(format!("yt-dlp: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no URL returned"))),
    }
}
