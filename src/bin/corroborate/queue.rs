//! The queue (docs/run-queue/plan.md, stage 2): one for the whole project. A job (a node that does
//! work, `jobs::is_job`) enters it when asked for: Run, ▶ on the node, or a change while Auto is
//! on. It starts once everything it needs is done and up to date, in the order asked; until then
//! it says what it waits for. A need that fails or is cancelled ends what waits for it at once.
//!
//! What a job needs comes from its wires:
//! - a SOURCER needs nothing; it downloads (a turn to download) and decodes (a turn to decode);
//! - ENF, Planes and plugins read a SOURCER's sound as it decodes: they ride on its run, so asking
//!   for one runs its SOURCER again (nothing is kept between runs yet: stage 3);
//! - a Cut detector needs the SOURCERs of its recording and of what it compares with;
//! - an Audio match needs the SOURCERs of its files and the Cut detectors whose pieces it takes.
//!
//! A job's key is everything that went into it: its own settings that change what it works out,
//! and the keys of what it needs. A job is up to date when its last run is done and was on the key
//! it has now; stale when it ran on another key. With Auto on (and not stopped), a job whose key
//! changed is asked for again half a second after the last change (typing): only what a change
//! touches runs again. What was cancelled is left alone until Run or ▶.

use crate::jobs::{End, JobState};
use crate::runtime::{self, Job};
use crate::{sourcer_node, App, Kind, NodeId};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The queue: what waits to start, and what each job last ran on.
pub struct Queue {
    /// Asked for and not started yet, in the order asked.
    pub waiting: Vec<NodeId>,
    pub recs: HashMap<NodeId, Rec>,
    /// Auto: a change queues what it touches (a switch in the top bar).
    pub auto: bool,
    /// Every job's key as last worked out (the dots show stale from it).
    pub keys: HashMap<NodeId, u64>,
    /// All keys together as last seen, and since when: a change waits half a second (typing).
    seen: Option<(u64, Instant)>,
    /// The programs' settings they last started with (programs join the queue in stage 4).
    programs: Option<u64>,
    asks: u64,
}

impl Default for Queue {
    fn default() -> Self {
        Queue { waiting: Vec::new(), recs: HashMap::new(), auto: true, keys: HashMap::new(), seen: None, programs: None, asks: 0 }
    }
}

/// One job's record in the queue.
#[derive(Default)]
pub struct Rec {
    /// The key it was last asked for on (Auto asks again when the key changes).
    pub tried: Option<u64>,
    /// The key its current or last run started on.
    pub ran: Option<u64>,
    /// Its run's stop (a rider shares its SOURCER's).
    pub stop: Option<Arc<AtomicBool>>,
    /// When it was last asked for, and its place in the order asked.
    pub asked: Option<Instant>,
    pub order: u64,
    /// Ended without starting: cancelled while queued, or a need failed or was cancelled.
    pub ended: Option<(JobState, String)>,
}

/// How a job runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    /// A SOURCER: downloads and decodes, feeding its riders as it goes.
    Source,
    /// ENF, Planes, a plugin: reads the sound of a SOURCER (None: not wired to one) as it decodes.
    Rider(Option<NodeId>),
    /// Cut detector, Audio match: starts once its needs are done.
    Work,
}

fn hash(x: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    x.hash(&mut h);
    h.finish()
}

/// Reads `settings\run.txt` (how many at once; written with the defaults when missing):
/// `downloads = 2`, `heavy = 0` (0: half the processor cores).
pub fn load_limits(dir: &std::path::Path) {
    let p = dir.join("run.txt");
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(_) => {
            let t = "# How many jobs of each kind work at once (docs/run-queue/plan.md). 0: the default.\r\n# downloads: yt-dlp downloads (default 2).\r\ndownloads = 2\r\n# heavy: decoding and heavy work (default half the processor cores).\r\nheavy = 0\r\n";
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(&p, t);
            t.to_string()
        }
    };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let Ok(n) = v.trim().parse::<usize>() else { continue };
        match k.trim() {
            "downloads" => corroborate::ingest::DOWNLOADS.set_limit(n),
            "heavy" => corroborate::ingest::HEAVY.set_limit(n),
            _ => {}
        }
    }
}

impl App {
    /// How job `i` runs (None: it does no work).
    pub(crate) fn role(&self, i: NodeId) -> Option<Role> {
        let n = &self.nodes[i];
        if n.hidden || !crate::jobs::is_job(&n.kind) {
            return None;
        }
        Some(match &n.kind {
            Kind::Sourcer { .. } => Role::Source,
            Kind::Planes | Kind::Enf { .. } | Kind::Plugin { .. } => Role::Rider(self.audio_source(i).filter(|(f, o)| *o == "audio" && matches!(self.nodes[*f].kind, Kind::Sourcer { .. })).map(|(f, _)| f)),
            _ => Role::Work,
        })
    }

    /// Whether SOURCER `f` reads a recording another job can read after it (a file or a link;
    /// not a microphone).
    fn readable(&self, f: NodeId) -> bool {
        match &self.nodes[f].kind {
            Kind::Sourcer { input } => matches!(sourcer_node::input(input), Some(runtime::Input::File(_) | runtime::Input::Url(_))),
            _ => false,
        }
    }

    /// The jobs job `i` needs done before it starts (a rider's SOURCER is not among them: it rides
    /// on its run).
    pub(crate) fn needs(&self, i: NodeId) -> Vec<NodeId> {
        let mut v = Vec::new();
        match &self.nodes[i].kind {
            Kind::CutDetector { .. } => {
                if let Ok(src) = self.cut_source(i) {
                    if self.readable(src) {
                        v.push(src);
                        v.extend(self.cut_compared(i).into_iter().map(|(_, f)| f).filter(|&f| self.readable(f)));
                    }
                }
            }
            Kind::AudioMatch { .. } => {
                for p in self.graph.nodes()[i].spec.inputs.iter().filter(|p| p.kind == crate::PortKind::Audio) {
                    let Some((f, _)) = self.graph.feeder(i, p.name) else { continue };
                    match &self.nodes[f].kind {
                        Kind::Sourcer { input } if matches!(sourcer_node::input(input), Some(runtime::Input::File(_))) => v.push(f),
                        Kind::CutDetector { .. } => v.push(f),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        v.sort_unstable();
        v.dedup();
        v.retain(|&n| !self.nodes[n].hidden);
        v
    }

    /// What job `i` itself is run on: the settings that change what it works out (not what is
    /// only shown, like the Cut detector's thresholds).
    fn own_key(&self, i: NodeId) -> String {
        let n = &self.nodes[i];
        let feeder = |port: &str| self.graph.feeder(i, port).map(|(f, o)| format!("{f}.{o}")).unwrap_or_default();
        match &n.kind {
            Kind::Sourcer { input } => format!("{input}|{}", self.clock_of(i).unwrap_or_default()),
            Kind::Planes => String::new(),
            Kind::Enf { grid } => {
                // A new grid log (the file or folder changes) makes the ENF stale.
                let m = std::fs::metadata(grid).ok().map(|m| (m.len(), m.modified().ok()));
                let val = |port: &str| self.graph.feeder(i, port).map(|(f, o)| format!("{:?}", self.value_from(f, o))).unwrap_or_default();
                format!("{grid}|{m:?}|{}|{}", val("time"), val("location"))
            }
            Kind::Plugin { path } => format!("{path}|{:?}", self.value_args_into(i)),
            Kind::CutDetector { .. } => {
                let compared: Vec<String> = self.cut_compared(i).into_iter().map(|(k, f)| format!("{k}<{f}")).collect();
                format!("{:?}|{}|{}|{compared:?}", self.cut_source(i), feeder("video"), feeder("audio"))
            }
            Kind::AudioMatch { .. } => {
                let mut s = String::new();
                for p in self.graph.nodes()[i].spec.inputs.iter().filter(|p| p.kind == crate::PortKind::Audio) {
                    let Some((f, o)) = self.graph.feeder(i, p.name) else { continue };
                    s.push_str(&format!("{}<{f}.{o}:{}", p.name, self.clock_of(f).unwrap_or_default()));
                    if let Some(pf) = self.piece_feed(f, o) {
                        s.push_str(&format!(":{:.3}-{:.3}:{}", pf.piece.a0, pf.piece.a1, pf.clock));
                    }
                    s.push(';');
                }
                s
            }
            _ => String::new(),
        }
    }

    /// Every job's key now.
    pub(crate) fn job_keys(&self) -> HashMap<NodeId, u64> {
        let mut keys = HashMap::new();
        for i in 0..self.nodes.len() {
            self.key_of(i, &mut keys);
        }
        keys
    }

    fn key_of(&self, i: NodeId, keys: &mut HashMap<NodeId, u64>) -> Option<u64> {
        if let Some(k) = keys.get(&i) {
            return Some(*k);
        }
        let role = self.role(i)?;
        let mut need_keys: Vec<u64> = self.needs(i).into_iter().filter_map(|n| self.key_of(n, keys)).collect();
        if let Role::Rider(Some(s)) = role {
            need_keys.extend(self.key_of(s, keys));
        }
        let k = hash((self.nodes[i].kind.spec().name, self.own_key(i), need_keys));
        keys.insert(i, k);
        Some(k)
    }

    fn rec(&mut self, i: NodeId) -> &mut Rec {
        self.queue.recs.entry(i).or_default()
    }

    /// Job `i`'s run was told to stop (it may still be ending).
    fn stopping(&self, i: NodeId) -> bool {
        self.queue.recs.get(&i).and_then(|r| r.stop.as_ref()).is_some_and(|s| s.load(Ordering::Relaxed))
    }

    /// Job `i` is queued, or working on a run not told to stop.
    pub(crate) fn in_flight(&self, i: NodeId) -> bool {
        self.queue.waiting.contains(&i) || (self.nodes[i].state.lock().unwrap().running && !self.stopping(i))
    }

    /// Job `i`'s last run is done, on the key it has now.
    pub(crate) fn up_to_date(&self, i: NodeId, keys: &HashMap<NodeId, u64>) -> bool {
        let Some(r) = self.queue.recs.get(&i) else { return false };
        if r.ended.is_some() || self.queue.waiting.contains(&i) || r.ran.is_none() || r.ran != keys.get(&i).copied() {
            return false;
        }
        let s = self.nodes[i].state.lock().unwrap();
        !s.running && crate::jobs::view(&s).state == JobState::Done
    }

    /// Job `i` ended badly (failed or cancelled) and is not asked for again: how, in words.
    fn ended_badly(&self, i: NodeId) -> Option<JobState> {
        if self.in_flight(i) {
            return None;
        }
        if let Some((st, _)) = self.queue.recs.get(&i).and_then(|r| r.ended.as_ref()) {
            return Some(*st);
        }
        let s = self.nodes[i].state.lock().unwrap();
        match crate::jobs::view(&s).state {
            st @ (JobState::Failed | JobState::Cancelled) => Some(st),
            _ => None,
        }
    }

    /// Ask for job `i`: it and every job it needs that is not up to date are queued. `force`: even
    /// when it is up to date itself (▶, Run again). A job working on an older key starts again.
    pub(crate) fn ask_job(&mut self, i: NodeId, force: bool, keys: &HashMap<NodeId, u64>) {
        let Some(role) = self.role(i) else { return };
        let key = keys.get(&i).copied();
        self.rec(i).tried = key;
        let running = self.nodes[i].state.lock().unwrap().running && !self.stopping(i);
        if running {
            let r = &self.queue.recs[&i];
            if r.ran == key && !force {
                return;
            }
            self.restart(i);
        }
        if self.queue.waiting.contains(&i) || (!force && self.up_to_date(i, keys)) {
            return;
        }
        for n in self.needs(i) {
            if !self.in_flight(n) && !self.up_to_date(n, keys) {
                self.ask_job(n, false, keys);
            }
        }
        // A rider needs a fresh run of its SOURCER's sound.
        if let Role::Rider(Some(s)) = role {
            if !self.queue.waiting.contains(&s) {
                self.ask_job(s, true, keys);
            }
        }
        self.push(i);
    }

    fn push(&mut self, i: NodeId) {
        if self.queue.waiting.contains(&i) {
            return;
        }
        self.queue.asks += 1;
        let order = self.queue.asks;
        let r = self.rec(i);
        r.ended = None;
        r.asked = Some(Instant::now());
        r.order = order;
        self.queue.waiting.push(i);
    }

    /// Stop job `i`'s run to start it again; a SOURCER's riders in that run are queued again
    /// with it.
    fn restart(&mut self, i: NodeId) {
        let Some(stop) = self.queue.recs.get(&i).and_then(|r| r.stop.clone()) else { return };
        let riders: Vec<NodeId> = if self.role(i) == Some(Role::Source) {
            self.queue.recs.iter().filter(|&(&r, rec)| r != i && rec.stop.as_ref().is_some_and(|s| Arc::ptr_eq(s, &stop)) && self.nodes[r].state.lock().unwrap().running).map(|(&r, _)| r).collect()
        } else {
            Vec::new()
        };
        stop.store(true, Ordering::Relaxed);
        self.push(i);
        for r in riders {
            self.push(r);
        }
    }

    /// Ends queued job `i` before it started: `st` (failed or cancelled), saying why.
    fn end_queued(&mut self, i: NodeId, st: JobState, why: String) {
        self.queue.waiting.retain(|&w| w != i);
        self.rec(i).ended = Some((st, why));
    }

    /// Starts what can start, in the order asked; ends at once what needs a job that failed or
    /// was cancelled.
    pub(crate) fn pump(&mut self, keys: &HashMap<NodeId, u64>) {
        let list = self.queue.waiting.clone();
        for i in list {
            if !self.queue.waiting.contains(&i) {
                continue;
            }
            let Some(role) = self.role(i) else {
                self.queue.waiting.retain(|&w| w != i);
                continue;
            };
            let gone = |a: &App, n: NodeId, st: JobState| format!("needs '{}', which {}", a.node_name(n), if st == JobState::Failed { "failed" } else { "was cancelled" });
            match role {
                Role::Rider(None) => self.end_queued(i, JobState::Failed, "wire a SOURCER's audio into it".into()),
                // Started by its SOURCER's run.
                Role::Rider(Some(s)) => {
                    if !self.queue.waiting.contains(&s) {
                        match self.ended_badly(s) {
                            Some(st) => {
                                let why = gone(self, s, st);
                                self.end_queued(i, st, why);
                            }
                            None => self.ask_job(s, true, keys),
                        }
                    }
                }
                Role::Source => self.start_job(i, keys),
                Role::Work => {
                    let needs = self.needs(i);
                    if let Some((n, st)) = needs.iter().find_map(|&n| self.ended_badly(n).map(|st| (n, st))) {
                        let why = gone(self, n, st);
                        self.end_queued(i, st, why);
                    } else if needs.iter().all(|&n| self.up_to_date(n, keys)) {
                        self.start_job(i, keys);
                    } else {
                        // A need that changed while this waited is asked for again.
                        for n in needs {
                            if !self.in_flight(n) && !self.up_to_date(n, keys) {
                                self.ask_job(n, false, keys);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Starts job `i` (its needs are done): a SOURCER with the riders queued on it.
    fn start_job(&mut self, i: NodeId, keys: &HashMap<NodeId, u64>) {
        self.queue.waiting.retain(|&w| w != i);
        let stop = Arc::new(AtomicBool::new(false));
        let key = keys.get(&i).copied();
        {
            let r = self.rec(i);
            r.ran = key;
            r.stop = Some(stop.clone());
            r.ended = None;
        }
        let state = self.nodes[i].state.clone();
        match &self.nodes[i].kind {
            Kind::Sourcer { input } => {
                let Some(input) = sourcer_node::input(input) else {
                    let mut s = state.lock().unwrap();
                    s.reset();
                    s.status = "nothing to open: set it in the node".into();
                    s.progress = crate::jobs::Progress::new(Vec::<String>::new());
                    let why = s.status.clone();
                    s.progress.finish(End::Failed(why));
                    return;
                };
                let dir = self.results_dir();
                let _ = std::fs::create_dir_all(&dir);
                let n = &self.nodes[i];
                let mut job = Job {
                    label: n.label.clone(),
                    input,
                    source: state,
                    planes: Vec::new(),
                    enfs: Vec::new(),
                    plugins: Vec::new(),
                    stop: stop.clone(),
                    work_dir: dir,
                    taps: n.taps.clone(),
                    buses: self.audio_buses(i),
                    clock: self.clock_of(i).unwrap_or_default(),
                    downloads: self.downloads_dir(),
                };
                let riders: Vec<NodeId> = self.queue.waiting.iter().copied().filter(|&r| self.role(r) == Some(Role::Rider(Some(i)))).collect();
                for &r in &riders {
                    match &self.nodes[r].kind {
                        Kind::Planes => job.planes.push(self.nodes[r].state.clone()),
                        Kind::Enf { .. } => job.enfs.push(self.enf_job(r)),
                        Kind::Plugin { path } => job.plugins.push(self.plugin_job(r, path)),
                        _ => {}
                    }
                }
                for r in riders {
                    self.queue.waiting.retain(|&w| w != r);
                    let k = keys.get(&r).copied();
                    let rec = self.rec(r);
                    rec.ran = k;
                    rec.stop = Some(stop.clone());
                    rec.ended = None;
                }
                runtime::start(job);
            }
            Kind::CutDetector { .. } => crate::cut_detector_node::start(state, self.cut_job(i), stop),
            Kind::AudioMatch { .. } => {
                let (files, other) = self.audio_match_inputs(i);
                crate::audio_match_node::start(state, files, other, stop);
            }
            _ => {}
        }
    }

    /// Cancel job `i`: queued, it leaves the queue; working, its run stops (a rider's stops its
    /// SOURCER's run). What needs it ends at once.
    pub(crate) fn cancel_job(&mut self, i: NodeId) {
        if self.queue.waiting.contains(&i) {
            self.end_queued(i, JobState::Cancelled, String::new());
        } else if let Some(s) = self.queue.recs.get(&i).and_then(|r| r.stop.clone()) {
            s.store(true, Ordering::Relaxed);
        }
        let keys = self.job_keys();
        self.pump(&keys);
        self.queue.keys = keys;
    }

    /// ▶ on node `i`: it, and every job it needs that is not up to date.
    pub(crate) fn run_node(&mut self, i: NodeId) {
        let keys = self.job_keys();
        self.ask_job(i, true, &keys);
        self.pump(&keys);
        self.queue.keys = keys;
    }

    /// Every job that is not up to date.
    pub(crate) fn ask_all(&mut self, force: bool) {
        let keys = self.job_keys();
        for i in 0..self.nodes.len() {
            self.ask_job(i, force, &keys);
        }
        self.pump(&keys);
        self.queue.keys = keys;
    }

    /// Cancel everything queued or working.
    pub(crate) fn cancel_all(&mut self) {
        for i in std::mem::take(&mut self.queue.waiting) {
            self.rec(i).ended = Some((JobState::Cancelled, String::new()));
        }
        for r in self.queue.recs.values() {
            if let Some(s) = &r.stop {
                s.store(true, Ordering::Relaxed);
            }
        }
    }

    /// What the programs on the data bus start with (they join the queue in stage 4; until then a
    /// change to them starts them again).
    fn programs_key(&self) -> u64 {
        let mut v = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if let (false, Kind::Module { settings, .. }) = (n.hidden, &n.kind) {
                v.push(format!("{i}:{settings:?}:{:?}", self.audio_source(i)));
            }
        }
        hash(v)
    }

    /// Starts the programs again, and the SOURCERs whose sound they hear (they get it as it
    /// decodes).
    pub(crate) fn restart_programs(&mut self, keys: &HashMap<NodeId, u64>) {
        self.runs += 1;
        self.start_programs();
        self.queue.programs = Some(self.programs_key());
        let heard: Vec<NodeId> = self.programs.keys().filter_map(|&p| self.audio_source(p)).filter(|(f, _)| matches!(self.nodes[*f].kind, Kind::Sourcer { .. })).map(|(f, _)| f).collect();
        for f in heard {
            self.ask_job(f, true, keys);
        }
    }

    /// Once a frame: Auto asks for what a change touched (half a second after the last change),
    /// and the queue starts what can start.
    pub(crate) fn keep_running(&mut self) {
        let keys = self.job_keys();
        let mut all: Vec<(NodeId, u64)> = keys.iter().map(|(a, b)| (*a, *b)).collect();
        all.sort_unstable();
        let all = hash((all, self.programs_key()));
        let first = self.queue.seen.is_none();
        if self.queue.seen.map(|s| s.0) != Some(all) {
            self.queue.seen = Some((all, Instant::now()));
        }
        let settled = first || self.queue.seen.is_some_and(|s| s.1.elapsed() > Duration::from_millis(500));
        if !self.stopped && self.queue.auto && settled {
            if self.queue.programs != Some(self.programs_key()) {
                self.restart_programs(&keys);
            }
            let mut ids: Vec<NodeId> = keys.keys().copied().collect();
            ids.sort_unstable();
            for i in ids {
                // What was cancelled (■, Stop, or a need cancelled) waits for Run or ▶.
                if self.job_view(i).is_some_and(|v| v.state == JobState::Cancelled) {
                    continue;
                }
                if self.queue.recs.get(&i).and_then(|r| r.tried) != keys.get(&i).copied() {
                    self.ask_job(i, false, &keys);
                }
            }
        }
        self.pump(&keys);
        self.queue.keys = keys;
    }

    /// Why queued job `i` waits: for which need (and how far it has got), or for its SOURCER.
    pub(crate) fn waits_text(&self, i: NodeId) -> String {
        let keys = &self.queue.keys;
        let on = match self.role(i) {
            Some(Role::Rider(Some(s))) => Some(s),
            _ => self.needs(i).into_iter().find(|&n| !self.up_to_date(n, keys)),
        };
        match on {
            Some(n) if self.queue.waiting.contains(&n) => format!("for '{}' (queued)", self.node_name(n)),
            Some(n) => crate::jobs::waits_for(&self.node_name(n), &self.nodes[n].state.lock().unwrap()),
            None => "to start".into(),
        }
    }
}
