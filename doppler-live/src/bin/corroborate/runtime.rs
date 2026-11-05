//! Running a graph: each source node decodes its audio (ffmpeg, 16 kHz mono) on its own
//! thread and feeds every node wired to its audio output. Planes and ENF run in that thread;
//! plugins are separate programs fed through a pipe, as in the old viewer. Every node's
//! results land in its own shared state, which the GUI reads.

use doppler_live::modules::planes::detect::{Config, Event};
use doppler_live::module::{Finding, Module};
use doppler_live::modules::enf::{Enf, EnfPoint, EnfSegment};
use doppler_live::modules::planes::Planes;
use doppler_live::plugin::{self, PluginFinding};
use doppler_live::timebase::{self, Anchor, SharedAnchor};
use doppler_live::area::{GridMap, WorldMask};
use doppler_live::{ingest, INPUT_SR};
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;

pub type Shared = Arc<Mutex<NodeState>>;

/// Spectrogram value range kept for display (whitened z-scores).
const Z_LO: f32 = -1.0;
const Z_HI: f32 = 6.0;

#[derive(Default, Clone)]
pub struct PluginState {
    pub name: String,
    pub about: String,
    pub color: Option<[u8; 3]>,
    pub status: String,
    /// Where the process stands: starting, running, finished, or what went wrong.
    pub state: String,
    /// Last line on stderr.
    pub log: String,
    pub tracks: Vec<(String, Vec<(f64, f64, bool)>)>,
    pub findings: Vec<PluginFinding>,
    pub meta: Vec<(String, String)>,
    /// Live audio the plugin could not keep up with, seconds.
    pub dropped_s: f64,
}

impl PluginState {
    fn apply(&mut self, m: plugin::Msg) {
        match m {
            plugin::Msg::Hello { name, about, color } => {
                if !name.is_empty() {
                    self.name = name;
                }
                self.about = about;
                self.color = color;
                self.state = "running".into();
            }
            plugin::Msg::Status(t) => self.status = t,
            plugin::Msg::View { .. } => {}
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
}

/// What one node has produced so far. Fields are used by the kinds they belong to.
#[derive(Default)]
pub struct NodeState {
    pub status: String,
    pub running: bool,
    pub done: bool,
    pub live: bool,
    /// The source feeding this node: label and path or URL.
    pub source_label: String,
    pub source_key: String,
    /// UTC of the source's relative time 0, and where it came from.
    pub anchor: Option<Anchor>,
    pub audio_s: f64,
    // Planes.
    pub freqs: Vec<f64>,
    pub hist: Vec<u8>,
    pub hist_t: Vec<f64>,
    /// Frames ever added: changes whenever the spectrogram changes.
    pub frames: u64,
    pub passes: Vec<Event>,
    pub pending: Vec<Event>,
    pub ridges: Vec<Vec<(f64, f64)>>,
    // ENF.
    pub enf_points: Vec<EnfPoint>,
    pub enf_segments: Vec<EnfSegment>,
    pub enf_nominal: Option<f64>,
    /// Where the measured grid frequency is used (AND the location wired in), once decided.
    pub enf_area: Option<Arc<WorldMask>>,
    /// How that area was reached: regions, data source, contradictions.
    pub enf_area_note: String,
    // Plugin.
    pub plugin: PluginState,
}

impl NodeState {
    /// Clear results before a new run.
    pub fn reset(&mut self) {
        *self = NodeState::default();
    }
}

pub enum Input {
    File(String),
    Url(String),
    Mic(String),
}

impl Input {
    pub fn key(&self) -> String {
        match self {
            Input::File(p) | Input::Url(p) => p.clone(),
            Input::Mic(d) => format!("dshow:{d}"),
        }
    }
    fn live(&self) -> bool {
        !matches!(self, Input::File(_))
    }
}

pub struct PluginJob {
    pub path: String,
    pub state: Shared,
    /// Manual data rows from a Manual node wired into the plugin.
    pub manual: Vec<(String, String)>,
}

pub struct Job {
    pub label: String,
    pub input: Input,
    pub source: Shared,
    pub planes: Vec<Shared>,
    pub enfs: Vec<EnfJob>,
    pub plugins: Vec<PluginJob>,
    pub stop: Arc<AtomicBool>,
    /// Working folder for plugins (adsb-check writes its flight log there).
    pub work_dir: std::path::PathBuf,
}

/// An ENF node: its state, the grid map wired into it, and the location wired into it (if any).
pub struct EnfJob {
    pub state: Shared,
    pub grid: Result<Arc<GridMap>, String>,
    pub prior: Option<(WorldMask, String)>,
}

pub fn start(job: Job) {
    thread::spawn(move || run(job));
}

fn quantise(z: f32) -> u8 {
    (((z - Z_LO) / (Z_HI - Z_LO)).clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Where a measured nominal frequency is used, AND the location wired into the node.
/// An empty result is kept: it says the hum contradicts that location.
fn enf_area(nominal: f64, j: &EnfJob) -> (Option<WorldMask>, String) {
    let g = match &j.grid {
        Ok(g) => g,
        Err(e) => return (None, format!("no area: {e}")),
    };
    let names: Vec<&str> = g.regions_for(nominal).iter().map(|r| r.id.as_str()).collect();
    let mut m = g.mask_for(nominal);
    let mut note = format!("{nominal:.0} Hz grid: {} (grid data: {})", names.join("; "), g.source);
    if let Some((p, label)) = &j.prior {
        m = m.and(p);
        if m.is_empty() {
            note = format!("CONTRADICTION: {nominal:.0} Hz hum is not used anywhere in {label}. {note}");
        } else {
            note = format!("{note}; limited to {label}");
        }
    }
    (Some(m), note)
}

/// YouTube page links need resolving to a direct stream URL (yt-dlp); others are used as-is.
fn resolve(url: &str) -> Result<String, String> {
    if !(url.contains("youtube.com/") || url.contains("youtu.be/")) {
        return Ok(url.to_string());
    }
    let out = doppler_live::no_window(&mut Command::new("yt-dlp"))
        .args(["--no-warnings", "-g", "-f", "ba/b", url])
        .output()
        .map_err(|e| format!("could not run yt-dlp (is it installed?): {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match text.lines().next() {
        Some(u) if out.status.success() && u.contains("://") => Ok(u.trim().to_string()),
        _ => Err(format!("yt-dlp: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no URL returned"))),
    }
}

fn all_states(job: &Job) -> Vec<Shared> {
    let mut v = vec![job.source.clone()];
    v.extend(job.planes.iter().cloned());
    v.extend(job.enfs.iter().map(|e| e.state.clone()));
    v.extend(job.plugins.iter().map(|p| p.state.clone()));
    v
}

fn run(job: Job) {
    let live = job.input.live();
    let key = job.input.key();
    for s in all_states(&job) {
        let mut s = s.lock().unwrap();
        s.reset();
        s.running = true;
        s.live = live;
        s.source_label = job.label.clone();
        s.source_key = key.clone();
        s.status = "starting".into();
    }
    let set_status = |text: &str| job.source.lock().unwrap().status = text.to_string();

    let ff_input = match &job.input {
        Input::File(p) => p.clone(),
        Input::Mic(d) => format!("dshow:{d}"),
        Input::Url(u) => {
            set_status("resolving the stream URL…");
            match resolve(u) {
                Ok(r) => r,
                Err(e) => return finish(&job, Err(e)),
            }
        }
    };

    // UTC anchor: file metadata or our recording sidecar for files; for live sources the PC
    // clock at first audio, upgraded to the stream's own capture times when it has them.
    let anchor: SharedAnchor = Default::default();
    if let Input::File(p) = &job.input {
        *anchor.lock().unwrap() = timebase::file_anchor(p);
    }

    let mut args = vec!["--source".to_string(), key.clone(), if live { "--live" } else { "--file" }.to_string()];
    if let Some(a) = anchor.lock().unwrap().as_ref() {
        args.extend(["--utc0".to_string(), format!("{:.3}", a.utc0), "--utc-source".to_string(), a.source.clone()]);
    }
    let mut feeds: Vec<Option<SyncSender<Vec<f32>>>> = job
        .plugins
        .iter()
        .map(|p| {
            let mut a = args.clone();
            for (k, v) in p.manual.iter().filter(|(k, v)| !k.trim().is_empty() && !v.trim().is_empty()) {
                a.extend(["--manual".to_string(), format!("{}={}", k.trim(), v.trim())]);
            }
            {
                let mut s = p.state.lock().unwrap();
                s.plugin.state = "starting".into();
                s.status = "running".into();
            }
            spawn_plugin(&p.path, p.state.clone(), &a, &job.work_dir)
        })
        .collect();

    let mut planes: Vec<(Planes, Shared)> = job
        .planes
        .iter()
        .map(|s| {
            let p = Planes::new(Config::default());
            s.lock().unwrap().freqs = p.detector().bin_freqs();
            (p, s.clone())
        })
        .collect();
    let mut enfs: Vec<(Enf, &EnfJob, Option<f64>)> = job.enfs.iter().map(|j| (Enf::new(), j, None)).collect();

    // Spawn doppler-view as a separate visualization process for Planes data.
    if !planes.is_empty() {
        let _ = Command::new(std::env::current_exe().unwrap_or_default().with_file_name(format!("doppler-view{}", std::env::consts::EXE_SUFFIX)))
            .arg(&ff_input)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    set_status(if live { "connecting…" } else { "decoding…" });
    let (mut child, rx, first) = match ingest::spawn_ffmpeg_logged(&ff_input, INPUT_SR as u32, None) {
        Ok(x) => x,
        Err(e) => return finish(&job, Err(e)),
    };
    if live {
        timebase::upgrade_from_hls(ff_input.clone(), first, anchor.clone(), 0.0);
    }

    let mut n: u64 = 0;
    for x in rx {
        if job.stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            break;
        }
        if live && n == 0 {
            let mut a = anchor.lock().unwrap();
            if a.is_none() {
                *a = Some(timebase::received_anchor(0.0));
            }
        }
        n += x.len() as u64;
        let an = anchor.lock().unwrap().clone();

        // Plugins: a file waits for them; live audio never does, and what a plugin misses is counted.
        for (i, feed) in feeds.iter_mut().enumerate() {
            if let Some(tx) = feed {
                let gone = if live {
                    match tx.try_send(x.clone()) {
                        Ok(()) => false,
                        Err(TrySendError::Full(_)) => {
                            job.plugins[i].state.lock().unwrap().plugin.dropped_s += x.len() as f64 / INPUT_SR;
                            false
                        }
                        Err(TrySendError::Disconnected(_)) => true,
                    }
                } else {
                    tx.send(x.clone()).is_err()
                };
                if gone {
                    *feed = None;
                }
            }
        }

        for (p, s) in planes.iter_mut() {
            let mut frames: Vec<(f64, Vec<u8>)> = Vec::new();
            let evs = p.push_with(&x, |t, z| frames.push((t, z.iter().map(|&v| quantise(v)).collect())));
            let mut s = s.lock().unwrap();
            for (t, q) in frames {
                s.hist.extend_from_slice(&q);
                s.hist_t.push(t);
                s.frames += 1;
            }
            s.ridges = p.detector().ridges().to_vec();
            s.pending = p.detector().pending().cloned().collect();
            s.passes.extend(evs);
            s.anchor = an.clone();
            s.audio_s = n as f64 / INPUT_SR;
            s.status = format!("{} passes", s.passes.len());
        }
        for (e, j, last) in enfs.iter_mut() {
            let found = e.push(&x);
            let pts = e.take_points();
            let mut s = j.state.lock().unwrap();
            s.enf_points.extend(pts);
            s.enf_segments.extend(found.into_iter().filter_map(|f| if let Finding::Enf(g) = f { Some(g) } else { None }));
            s.enf_nominal = e.nominal();
            s.anchor = an.clone();
            s.audio_s = n as f64 / INPUT_SR;
            if s.enf_nominal != *last {
                *last = s.enf_nominal;
                if let Some(f) = s.enf_nominal {
                    let (area, note) = enf_area(f, j);
                    s.enf_area = area.map(Arc::new);
                    s.enf_area_note = note;
                }
            }
            s.status = match s.enf_nominal {
                Some(f) => format!("{f:.0} Hz grid, {} runs", s.enf_segments.len()),
                None => "listening for hum".into(),
            };
        }
        for p in &job.plugins {
            p.state.lock().unwrap().anchor = an.clone();
        }
        let mut s = job.source.lock().unwrap();
        s.anchor = an;
        s.audio_s = n as f64 / INPUT_SR;
        s.status = if live { "live".into() } else { "decoding…".into() };
    }
    let _ = child.wait();
    // End of input: plugins report what they hold and exit.
    drop(feeds);
    for (p, s) in planes.iter_mut() {
        let evs = p.flush_events();
        let mut s = s.lock().unwrap();
        s.passes.extend(evs);
        s.pending.clear();
        s.status = format!("{} passes", s.passes.len());
    }
    for (e, j, _) in enfs.iter_mut() {
        let found = e.flush();
        let mut s = j.state.lock().unwrap();
        s.enf_segments.extend(found.into_iter().filter_map(|f| if let Finding::Enf(g) = f { Some(g) } else { None }));
    }
    finish(&job, Ok(n as f64 / INPUT_SR));
}

fn finish(job: &Job, r: Result<f64, String>) {
    let stopped = job.stop.load(Ordering::Relaxed);
    for s in all_states(job) {
        let mut s = s.lock().unwrap();
        s.running = false;
        s.done = true;
    }
    let mut s = job.source.lock().unwrap();
    s.status = match r {
        Ok(_) if stopped => "stopped".into(),
        Ok(secs) if s.live => format!("stream ended after {secs:.0} s"),
        Ok(secs) => format!("analysed {secs:.0} s of audio"),
        Err(e) => format!("error: {e}"),
    };
    // Detectors that never got audio show why.
    let text = s.status.clone();
    drop(s);
    for st in job.planes.iter().chain(job.enfs.iter().map(|e| &e.state)) {
        let mut st = st.lock().unwrap();
        if st.status == "starting" {
            st.status = text.clone();
        }
    }
}

/// Start a plugin program on this source. Returns the channel feeding its stdin; threads
/// read its messages (stdout) and its last log line (stderr) into the node's state.
fn spawn_plugin(path: &str, st: Shared, args: &[String], dir: &std::path::Path) -> Option<SyncSender<Vec<f32>>> {
    let started = doppler_live::no_window(&mut Command::new(path))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match started {
        Ok(c) => c,
        Err(e) => {
            st.lock().unwrap().plugin.state = format!("could not start: {e}");
            return None;
        }
    };
    let (Some(mut stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return None;
    };
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(64);
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
    {
        let st = st.clone();
        thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(m) = plugin::parse_line(&line) {
                    st.lock().unwrap().plugin.apply(m);
                }
            }
            let state = match child.wait() {
                Ok(s) if s.success() => "finished".to_string(),
                Ok(s) => format!("stopped ({s})"),
                Err(e) => format!("stopped ({e})"),
            };
            st.lock().unwrap().plugin.state = state;
        });
    }
    thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                st.lock().unwrap().plugin.log = line;
            }
        }
    });
    Some(tx)
}
