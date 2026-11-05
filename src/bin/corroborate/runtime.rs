//! Running a graph: each source node decodes its audio (ffmpeg, 16 kHz mono) on its own
//! thread and feeds every node wired to its audio output. What each kind of node does with
//! the audio is in its module's `node.rs`; this file only decodes, hands out the audio, and
//! keeps the shared state the GUI reads.

use crate::enf_node::{self, EnfJob, EnfState};
use crate::planes_node::{self, PlanesState};
use crate::plugin_node::{self, PluginJob, PluginState};
use crate::stream_node;
use corroborate::timebase::{self, Anchor, SharedAnchor};
use corroborate::{ingest, INPUT_SR};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

pub type Shared = Arc<Mutex<NodeState>>;

/// What one node has produced so far: the part every node has, and each kind's own part.
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
    pub planes: PlanesState,
    pub enf: EnfState,
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

pub fn start(job: Job) {
    thread::spawn(move || run(job));
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
            match stream_node::resolve(u) {
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
    let mut feeds: Vec<_> = job.plugins.iter().map(|p| plugin_node::start(p, &args, &job.work_dir)).collect();
    let mut planes: Vec<_> = job.planes.iter().map(|s| (planes_node::start(s), s.clone())).collect();
    let mut enfs: Vec<_> = job.enfs.iter().map(enf_node::start).collect();
    if !planes.is_empty() {
        planes_node::open_viewer(&ff_input);
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
        let audio_s = n as f64 / INPUT_SR;

        for (i, feed) in feeds.iter_mut().enumerate() {
            if feed.as_ref().is_some_and(|tx| !plugin_node::feed(tx, &x, live, &job.plugins[i].state)) {
                *feed = None;
            }
        }
        for (p, s) in planes.iter_mut() {
            planes_node::push(p, s, &x, &an, audio_s);
        }
        for r in enfs.iter_mut() {
            enf_node::push(r, &x, &an, audio_s);
        }
        for p in &job.plugins {
            p.state.lock().unwrap().anchor = an.clone();
        }
        let mut s = job.source.lock().unwrap();
        s.anchor = an;
        s.audio_s = audio_s;
        s.status = if live { "live".into() } else { "decoding…".into() };
    }
    let _ = child.wait();
    // End of input: plugins report what they hold and exit.
    drop(feeds);
    for (p, s) in planes.iter_mut() {
        planes_node::flush(p, s);
    }
    for r in enfs.iter_mut() {
        enf_node::flush(r);
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
