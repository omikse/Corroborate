//! Running a graph: each source node decodes its audio (ffmpeg, 16 kHz mono) on its own
//! thread and feeds every node wired to its audio output. What each kind of node does with
//! the audio is in its module's `node.rs`; this file only decodes, hands out the audio, and
//! keeps the shared state the GUI reads.

use crate::enf_node::{self, EnfJob, EnfState};
use crate::jobs::{End, Progress, StepState};
use crate::planes_node::{self, PlanesState};
use crate::plugin_node::{self, PluginJob, PluginState};
use crate::sourcer_node;
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
    /// The run that owns this state now: a stopped run that ends late leaves it alone.
    pub run_id: u64,
    pub planes: PlanesState,
    pub enf: EnfState,
    pub plugin: PluginState,
    pub source: sourcer_node::SourceState,
    /// Audio match: how far each wired recording has got.
    pub matching: Vec<crate::audio_match_node::Stage>,
    /// Audio match: what the match found, its links among it, once it has ended.
    pub found: Option<std::sync::Arc<crate::audio_match_node::Found>>,
    /// Cut detector: what its run found in the recording.
    pub cut: Option<std::sync::Arc<crate::cut_detector_node::Scan>>,
    /// Every job's run: its steps and how far each has got, what it waits for, how it ended.
    pub progress: crate::jobs::Progress,
    /// What a program sent on each of its outputs (the data bus), and a count of the changes.
    pub outbox: Vec<(&'static str, corroborate::data::Data)>,
    pub outbox_rev: u64,
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
    pub fn live(&self) -> bool {
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
    /// WAV files recording this source's audio (Result record nodes add and remove them).
    pub taps: crate::result_record_node::Taps,
    /// Programs on the data bus wired to this source's audio: they get it as JSON lines.
    pub buses: Vec<std::sync::mpsc::SyncSender<String>>,
    /// The clock this source's times are on (its node's id, or the clock it shares).
    pub clock: String,
    /// Where a video from the web is downloaded: the project's recordings\ folder.
    pub downloads: std::path::PathBuf,
}

/// The steps of a source's run, and of the nodes reading its sound.
const DOWNLOAD: &str = "download";
const DECODE: &str = "decode";
const READ: &str = "read the sound";

static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Starts a source's run: its state and the states of what it feeds say so at once (queued),
/// then a thread of its own does the work.
pub fn start(job: Job) {
    let run_id = RUNS.fetch_add(1, Ordering::Relaxed);
    // A link is live only once it is known to be (yt-dlp says so, or a stream URL): while yt-dlp
    // checks it or downloads a video, nothing treats it as live (no pulsing wires, no playhead
    // following its audio).
    let live = matches!(job.input, Input::Mic(_));
    let key = job.input.key();
    // The steps: a web page's download, then the decode; the nodes it feeds read the sound as it
    // comes (they wait while it downloads or waits for its turn).
    let web = is_web(&job.input);
    for (k, s) in all_states(&job).into_iter().enumerate() {
        let mut s = s.lock().unwrap();
        s.reset();
        s.run_id = run_id;
        s.running = true;
        s.live = live;
        s.source_label = job.label.clone();
        s.source_key = key.clone();
        s.status = "starting".into();
        s.progress = if k == 0 { Progress::new(if web { vec![DOWNLOAD, DECODE] } else { vec![DECODE] }) } else { Progress::new([READ]) };
        s.progress.waits = match k {
            0 if live => None,
            0 if web => Some(crate::jobs::download_waits()),
            0 => Some(crate::jobs::turn_waits()),
            _ => Some(format!("for '{}'", job.label)),
        };
    }
    thread::spawn(move || run(job, run_id));
}

/// A web page yt-dlp resolves (and downloads, when it is not live).
fn is_web(input: &Input) -> bool {
    matches!(input, Input::Url(u) if sourcer_node::kind_of(u).contains("yt-dlp"))
}

fn all_states(job: &Job) -> Vec<Shared> {
    let mut v = vec![job.source.clone()];
    v.extend(job.planes.iter().cloned());
    v.extend(job.enfs.iter().map(|e| e.state.clone()));
    v.extend(job.plugins.iter().map(|p| p.state.clone()));
    v
}

fn run(job: Job, run_id: u64) {
    let mut live = matches!(job.input, Input::Mic(_));
    let key = job.input.key();
    let web = is_web(&job.input);
    let set_status = |text: &str| job.source.lock().unwrap().status = text.to_string();
    // Write into every state of this run (not one a newer run took over).
    let each = |f: &dyn Fn(usize, &mut NodeState)| {
        for (k, s) in all_states(&job).into_iter().enumerate() {
            let mut s = s.lock().unwrap();
            if s.run_id == run_id {
                f(k, &mut s);
            }
        }
    };
    // The source's step; what the others wait for follows it.
    let source_step = |name: &str, st: StepState, frac: f32| {
        each(&|k, s| {
            if k == 0 {
                s.progress.step(name, st, frac, "");
            } else if s.progress.waits.is_some() {
                s.progress.waits = Some(format!("for '{}' ({name}{})", job.label, if frac >= 0.0 { format!(" {:.0}%", frac * 100.0) } else { String::new() }));
            }
        })
    };

    let ff_input = match &job.input {
        Input::File(p) => p.clone(),
        Input::Mic(d) => format!("dshow:{d}"),
        Input::Url(u) => {
            // Only so many download at once (the network).
            let _turn = if web {
                set_status("waiting for a turn to download…");
                let w = crate::jobs::download_waits();
                each(&|k, s| s.progress.waits = Some(if k == 0 { w.clone() } else { format!("for '{}' ({w})", job.label) }));
                match ingest::DOWNLOADS.take(&job.stop) {
                    Some(t) => Some(t),
                    None => return finish(&job, run_id, Ok(0.0)),
                }
            } else {
                None
            };
            set_status("yt-dlp: checking the link, downloading the audio if it is not live…");
            if web {
                source_step(DOWNLOAD, StepState::Working, -1.0);
            }
            let report = |f: f32| {
                set_status(&format!("yt-dlp: downloading {:.0}%", f * 100.0));
                source_step(DOWNLOAD, StepState::Working, f);
            };
            match sourcer_node::resolve(u, &job.downloads, &report, &job.stop) {
                Ok((r, is_live)) => {
                    if web {
                        source_step(DOWNLOAD, StepState::Done, 1.0);
                    }
                    // A video that is not live is read as a recording: as fast as it decodes.
                    live = is_live;
                    for s in all_states(&job) {
                        s.lock().unwrap().live = live;
                    }
                    r
                }
                Err(e) => return finish(&job, run_id, Err(e)),
            }
        }
    };

    // Metadata and pictures for the SOURCER's own view and outputs (not for microphones).
    if !matches!(job.input, Input::Mic(_)) {
        sourcer_node::start_side(ff_input.clone(), key.clone(), live, job.source.clone(), job.stop.clone());
    }

    // UTC anchor: file metadata or our recording sidecar for files; for live sources the PC
    // clock at first audio, upgraded to the stream's own capture times when it has them.
    let anchor: SharedAnchor = Default::default();
    if let Input::File(p) = &job.input {
        *anchor.lock().unwrap() = timebase::file_anchor(p);
    }

    // A recording waits its turn: only so many decode at once (big projects).
    let _turn = if live {
        None
    } else {
        set_status(&format!("waiting: {} recordings decode at once", ingest::decoders_at_once()));
        let w = crate::jobs::turn_waits();
        each(&|k, s| s.progress.waits = Some(if k == 0 { w.clone() } else { format!("for '{}' ({w})", job.label) }));
        match ingest::decode_turn(&job.stop) {
            Some(t) => Some(t),
            None => return finish(&job, run_id, Ok(0.0)),
        }
    };
    let mut args = vec!["--source".to_string(), key.clone(), if live { "--live" } else { "--file" }.to_string(), "--clock".to_string(), job.clock.clone()];
    if let Some(a) = anchor.lock().unwrap().as_ref() {
        args.extend(["--utc0".to_string(), format!("{:.3}", a.utc0), "--utc-source".to_string(), a.source.clone(), "--utc-error".to_string(), format!("{:.3}", a.error_s)]);
    }
    let mut feeds: Vec<_> = job.plugins.iter().map(|p| plugin_node::start(p, &args, &job.work_dir)).collect();
    let mut planes: Vec<_> = job.planes.iter().map(|s| (planes_node::start(s), s.clone())).collect();
    let mut enfs: Vec<_> = job.enfs.iter().map(enf_node::start).collect();

    set_status(if live { "connecting…" } else { "decoding…" });
    // How long the recording is, for how far the decode has got.
    let total = if live || !std::path::Path::new(&ff_input).exists() { None } else { timebase::file_facts(&ff_input).len.filter(|l| *l > 0.0) };
    let part = |secs: f64| total.map_or(-1.0, |l| (secs / l).clamp(0.0, 1.0) as f32);
    each(&|k, s| s.progress.step(if k == 0 { DECODE } else { READ }, StepState::Working, part(0.0), ""));
    let mut shown = -1i32;
    ingest::LAST_ERROR.lock().unwrap().clear();
    let (mut child, rx, first) = match ingest::spawn_ffmpeg_logged(&ff_input, INPUT_SR as u32, None) {
        Ok(x) => x,
        Err(e) => return finish(&job, run_id, Err(e)),
    };
    if live {
        timebase::upgrade_from_hls(ff_input.clone(), first, anchor.clone(), 0.0);
    }

    let mut buses = job.buses.clone();
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
        // How far, every half percent.
        let f = part(audio_s);
        if f >= 0.0 && (f * 200.0) as i32 != shown {
            shown = (f * 200.0) as i32;
            each(&|k, s| s.progress.step(if k == 0 { DECODE } else { READ }, StepState::Working, f, ""));
        }

        for (i, feed) in feeds.iter_mut().enumerate() {
            if feed.as_ref().is_some_and(|tx| !plugin_node::feed(tx, &x, live, &job.plugins[i].state)) {
                *feed = None;
            }
        }
        if !buses.is_empty() {
            let line = corroborate::plugin::audio(&x);
            // Live audio never waits for a program; a file's does.
            buses.retain(|tx| if live { !matches!(tx.try_send(line.clone()), Err(std::sync::mpsc::TrySendError::Disconnected(_))) } else { tx.send(line.clone()).is_ok() });
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
        s.source.push_audio(&x);
        for (_, w) in job.taps.lock().unwrap().iter_mut() {
            w.push(&x);
        }
        s.status = if live { "live".into() } else { "decoding…".into() };
    }
    let _ = child.wait();
    if job.stop.load(Ordering::Relaxed) {
        // Stopped for a restart: the new run owns the shared state; only release what is ours.
        drop(feeds);
        return finish(&job, run_id, Ok(0.0));
    }
    // End of input: plugins report what they hold and exit.
    drop(feeds);
    for tx in &buses {
        let _ = tx.send(corroborate::plugin::END.to_string());
    }
    for (p, s) in planes.iter_mut() {
        planes_node::flush(p, s);
    }
    for r in enfs.iter_mut() {
        enf_node::flush(r);
    }
    if n == 0 && !job.stop.load(Ordering::Relaxed) {
        let e = ingest::LAST_ERROR.lock().unwrap().clone();
        return finish(&job, run_id, Err(format!("no audio came out of ffmpeg{}", if e.is_empty() { String::new() } else { format!(": {e}") })));
    }
    finish(&job, run_id, Ok(n as f64 / INPUT_SR));
}

fn finish(job: &Job, run_id: u64, r: Result<f64, String>) {
    let stopped = job.stop.load(Ordering::Relaxed);
    for (k, s) in all_states(job).into_iter().enumerate() {
        let mut s = s.lock().unwrap();
        if s.run_id == run_id {
            s.running = false;
            s.done = true;
            for st in &mut s.progress.steps {
                if st.state == StepState::Working {
                    st.state = if r.is_ok() && !stopped { StepState::Done } else { StepState::Failed };
                }
            }
            s.progress.finish(match &r {
                _ if stopped => End::Cancelled,
                Ok(_) => End::Done,
                Err(e) if k == 0 => End::Failed(e.clone()),
                Err(_) => End::Failed(format!("needs '{}', which failed", job.label)),
            });
        }
    }
    let mut s = job.source.lock().unwrap();
    if s.run_id != run_id {
        return;
    }
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
