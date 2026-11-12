//! The queue (docs/run-queue/plan.md, stage 2): one for the whole project. A job (a node that does
//! work, `jobs::is_job`) enters it when asked for: Run, ▶ on the node, or a change while Auto is
//! on. It starts once everything it needs is done and up to date, in the order asked; until then
//! it says what it waits for. A need that fails or is cancelled ends what waits for it at once.
//!
//! What a job needs comes from its wires:
//! - a SOURCER needs nothing; it downloads (a turn to download) and decodes (a turn to decode);
//! - ENF, Planes and plugins read a SOURCER's sound as it decodes: they ride on its run, so asking
//!   for one runs its SOURCER again (from its kept sound when it has one, `kept.rs`);
//! - a Cut detector needs the SOURCERs of its recording and of what it compares with;
//! - an Audio match needs the SOURCERs of its files and the Cut detectors whose pieces it takes.
//!
//! A job's key is everything that went into it: its own settings that change what it works out,
//! and the keys of what it needs. A job is up to date when its last run is done and was on the key
//! it has now; stale when it ran on another key. With Auto on (and not stopped), a job whose key
//! changed is asked for again half a second after the last change (typing): only what a change
//! touches runs again. What was cancelled is left alone until Run or ▶.

use crate::jobs::{End, JobState};
use crate::kept;
use crate::runtime::{self, Job};
use crate::{sourcer_node, App, Kind, NodeId};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
    /// Jobs to work out anew, without their kept results (Run again), until they start.
    pub fresh: HashSet<NodeId>,
}

impl Default for Queue {
    fn default() -> Self {
        Queue { waiting: Vec::new(), recs: HashMap::new(), auto: true, keys: HashMap::new(), seen: None, programs: None, asks: 0, fresh: HashSet::new() }
    }
}

/// What a node's right-click menu asks of its job.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JobAct {
    Run,
    Again,
    Cancel,
    Forget,
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
    /// Where its run's result is kept once it is done (None: it is not kept).
    pub keep: Option<PathBuf>,
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
            // A file by its size and print too: a changed file makes it stale (read once per size
            // and modified time).
            Kind::Sourcer { input } => {
                let print = match sourcer_node::input(input) {
                    Some(runtime::Input::File(p)) => kept::print_of(&p).unwrap_or_default(),
                    _ => String::new(),
                };
                format!("{input}|{}|{print}", self.clock_of(i).unwrap_or_default())
            }
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
        !s.running && crate::jobs::view(&s).state.has_result()
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
        // A kept result is loaded instead. A Cut detector's or an ENF's kept key is known from
        // the files it reads; an Audio match's from its pieces, once what cuts them is there. A
        // SOURCER reads its kept sound when it starts (the ENF, Planes or plugins riding on it
        // read it then).
        let now = match role {
            Role::Source => false,
            Role::Rider(_) => true,
            Role::Work => matches!(self.nodes[i].kind, Kind::CutDetector { .. }) || self.needs(i).iter().all(|&n| self.up_to_date(n, keys)),
        };
        if now && self.try_kept(i, keys) {
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
        // Kept: loaded, not worked out (a SOURCER reads its kept sound below, for its riders).
        if self.role(i) != Some(Role::Source) && self.try_kept(i, keys) {
            return;
        }
        let fresh = self.queue.fresh.remove(&i);
        self.queue.waiting.retain(|&w| w != i);
        let stop = Arc::new(AtomicBool::new(false));
        let key = keys.get(&i).copied();
        let keep = self.kept_path(i);
        let from_kept = keep.clone().filter(|p| self.role(i) == Some(Role::Source) && !fresh && p.exists());
        {
            let r = self.rec(i);
            r.ran = key;
            r.stop = Some(stop.clone());
            r.ended = None;
            r.keep = if from_kept.is_some() { None } else { keep };
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
                    from_kept,
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
                    self.queue.fresh.remove(&r);
                    let k = keys.get(&r).copied();
                    let keep = self.kept_path(r);
                    let rec = self.rec(r);
                    rec.ran = k;
                    rec.stop = Some(stop.clone());
                    rec.ended = None;
                    rec.keep = keep;
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
        self.keep_results();
        self.queue.keys = keys;
    }

    // ---------------- kept results (stage 3, `kept.rs`)

    /// The project's `cache\` folder; an untitled project's is next to the program (in a test it
    /// keeps nothing).
    pub(crate) fn cache_dir(&self) -> Option<PathBuf> {
        match self.project_dir() {
            Some(d) => Some(d.join("cache")),
            None if cfg!(test) => None,
            None => Some(corroborate::home().join("cache")),
        }
    }

    /// Job `i`'s kept file's name now: from its kept key (`kept.rs`). None: it is not kept (a
    /// live source, Planes, plugins), or what it reads cannot be read.
    pub(crate) fn kept_name(&self, i: NodeId) -> Option<String> {
        let print = |p: &str| kept::print_of(p);
        match &self.nodes[i].kind {
            Kind::Sourcer { input } => {
                let what = match sourcer_node::input(input)? {
                    runtime::Input::File(p) => format!("file|{}", print(&p)?),
                    runtime::Input::Url(u) if sourcer_node::kind_of(&u).contains("yt-dlp") => format!("link|{u}"),
                    _ => return None,
                };
                Some(kept::name("sourcer", &format!("v{}|{what}", kept::SOURCER_V)))
            }
            Kind::CutDetector { .. } => {
                let j = self.cut_job(i).ok()?;
                let mut what = format!("v{}|video {}|audio {}|{}", kept::CUT_V, j.video, j.audio, print(&j.path)?);
                for o in &j.compare {
                    what.push_str(&format!("|{}<{}", o.key, print(&o.path)?));
                }
                Some(kept::name("cut", &what))
            }
            Kind::AudioMatch { .. } => {
                let (files, other) = self.audio_match_inputs(i);
                if files.len() < 2 {
                    return None;
                }
                let mut what = format!("v{}|{other:?}", kept::MATCH_V);
                for w in &files {
                    let range = w.range.map(|(a, b)| format!("{a:.3}-{b:.3}")).unwrap_or_default();
                    what.push_str(&format!("|{}|{}|{}|{range}", w.label, w.clock, print(&w.path)?));
                }
                Some(kept::name("match", &what))
            }
            Kind::Enf { .. } => {
                let Some(Role::Rider(Some(s))) = self.role(i) else { return None };
                let sound = self.kept_name(s)?;
                Some(kept::name("enf", &format!("v{}|{sound}|{}", kept::ENF_V, self.own_key(i))))
            }
            _ => None,
        }
    }

    /// Where job `i`'s result is kept (None: it is not kept).
    pub(crate) fn kept_path(&self, i: NodeId) -> Option<PathBuf> {
        Some(self.cache_dir()?.join(self.kept_name(i)?))
    }

    /// Loads job `i`'s kept result into its state, when there is one (not for a SOURCER: it reads
    /// its kept sound as it starts, `start_job`), and not when it is to be worked out anew.
    fn try_kept(&mut self, i: NodeId, keys: &HashMap<NodeId, u64>) -> bool {
        if self.queue.fresh.contains(&i) || self.role(i) == Some(Role::Source) {
            return false;
        }
        let Some(path) = self.kept_path(i) else { return false };
        if !path.exists() {
            return false;
        }
        let state = self.nodes[i].state.clone();
        let enf = matches!(self.nodes[i].kind, Kind::Enf { .. }).then(|| self.enf_job(i));
        let mut s = state.lock().unwrap();
        let rev = s.outbox_rev;
        let loaded = match &self.nodes[i].kind {
            Kind::CutDetector { .. } => kept::read(&path, "cut").and_then(|b| kept::get_scan(&b)).map(|(scan, status)| {
                s.reset();
                s.cut = Some(Arc::new(scan));
                s.status = status;
            }),
            Kind::AudioMatch { .. } => kept::read(&path, "match").and_then(|b| kept::get_match(&b)).map(|(found, stages, status)| {
                s.reset();
                s.found = Some(Arc::new(found));
                s.matching = stages;
                s.status = status;
            }),
            Kind::Enf { .. } => kept::read(&path, "enf").and_then(|b| {
                s.reset();
                kept::get_enf(&b, &mut s)?;
                crate::enf_node::restore(enf.as_ref()?, &mut s);
                Some(())
            }),
            _ => None,
        };
        if loaded.is_none() {
            // A kept file that does not read is worked out again (and written anew).
            return false;
        }
        s.run_id = kept::run_id();
        s.outbox_rev = rev + 1;
        s.done = true;
        s.progress = crate::jobs::Progress::new(Vec::<String>::new());
        s.progress.finish(End::Kept);
        drop(s);
        self.queue.waiting.retain(|&w| w != i);
        self.queue.asks += 1;
        let (order, key) = (self.queue.asks, keys.get(&i).copied());
        let r = self.rec(i);
        (r.ran, r.tried, r.stop, r.ended, r.keep) = (key, key, None, None, None);
        r.asked = Some(Instant::now());
        r.order = order;
        true
    }

    /// Keeps what ended done since the last frame: each job's result in its kept file (written on
    /// a thread of its own).
    fn keep_results(&mut self) {
        let ids: Vec<NodeId> = self.queue.recs.iter().filter(|(_, r)| r.keep.is_some()).map(|(&i, _)| i).collect();
        for i in ids {
            let state = self.nodes[i].state.clone();
            let s = state.lock().unwrap();
            if s.running {
                continue;
            }
            let Some(path) = self.queue.recs.get_mut(&i).and_then(|r| r.keep.take()) else { continue };
            if s.progress.end != Some(End::Done) || s.live {
                continue;
            }
            match &self.nodes[i].kind {
                Kind::Sourcer { .. } => {
                    let run = s.run_id;
                    drop(s);
                    std::thread::spawn(move || keep_sound(&state, run, &path));
                }
                Kind::CutDetector { .. } => {
                    if let Some(scan) = s.cut.clone() {
                        let status = s.status.clone();
                        std::thread::spawn(move || {
                            let _ = kept::write(&path, "cut", kept::put_scan(&scan, &status));
                        });
                    }
                }
                Kind::AudioMatch { .. } => {
                    if let Some(b) = kept::put_match(&s) {
                        spawn_write(path, "match", b);
                    }
                }
                Kind::Enf { .. } => spawn_write(path, "enf", kept::put_enf(&s)),
                _ => {}
            }
        }
    }

    /// Loads what is kept for this project (when it opens): SOURCERs read their kept sound (a
    /// thread each), Cut detectors, ENFs and then Audio matches load theirs. Nothing is worked
    /// out; the project stays stopped.
    pub(crate) fn load_kept(&mut self) {
        let Some(dir) = self.cache_dir() else { return };
        if !dir.exists() {
            return;
        }
        let keys = self.job_keys();
        let ids: Vec<NodeId> = (0..self.nodes.len()).filter(|&i| self.role(i).is_some()).collect();
        for &i in &ids {
            if self.role(i) == Some(Role::Source) && self.kept_path(i).is_some_and(|p| p.exists()) {
                self.queue.asks += 1;
                let order = self.queue.asks;
                let r = self.rec(i);
                r.order = order;
                r.tried = keys.get(&i).copied();
                self.start_job(i, &keys);
            }
        }
        for pass in [0, 1] {
            for &i in &ids {
                let first = matches!(self.nodes[i].kind, Kind::CutDetector { .. } | Kind::Enf { .. });
                if first == (pass == 0) && self.role(i) != Some(Role::Source) {
                    self.try_kept(i, &keys);
                }
            }
            // The pieces of the Cut detectors just loaded (the Audio matches' keys read them).
            self.cut_ui.cache.borrow_mut().clear();
            self.match_ui.cache.borrow_mut().clear();
        }
        self.queue.keys = keys;
    }

    /// Run again on one node: it is worked out anew, without its kept result.
    pub(crate) fn run_node_again(&mut self, i: NodeId) {
        self.queue.fresh.insert(i);
        if let Some(r) = self.queue.recs.get_mut(&i) {
            r.ran = None;
        }
        self.run_node(i);
    }

    /// Forget kept result: deletes job `i`'s kept file (what it shows stays until it runs again).
    pub(crate) fn forget_kept(&mut self, i: NodeId) {
        match self.kept_path(i) {
            Some(p) if p.exists() => {
                let _ = std::fs::remove_file(&p);
                self.msg = format!("forgot the kept result of '{}'", self.node_name(i));
            }
            _ => self.msg = format!("'{}' has no kept result", self.node_name(i)),
        }
    }

    /// Forget all kept: empties the project's cache\ folder.
    pub(crate) fn forget_all_kept(&mut self) {
        if let Some(d) = self.cache_dir() {
            kept::forget_all(&d);
            self.msg = format!("forgot everything kept in {}", d.display());
        }
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

/// Writes a kept result on a thread of its own.
fn spawn_write(path: PathBuf, kind: &'static str, body: Vec<u8>) {
    std::thread::spawn(move || {
        let _ = kept::write(&path, kind, body);
    });
}

/// Keeps SOURCER state `st`'s sound (its run `run`) in `path`, once ffprobe's view of the file is
/// there too (it comes beside the decode).
fn keep_sound(st: &runtime::Shared, run: u64, path: &std::path::Path) {
    for _ in 0..200 {
        if !st.lock().unwrap().source.meta.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let sound = {
        let s = st.lock().unwrap();
        if s.run_id != run {
            return;
        }
        let opened = s.source.meta.iter().find(|(k, _)| k == "opened").map(|(_, v)| v.clone()).unwrap_or_default();
        let audio = s.source.audio.lock().unwrap().clone();
        kept::Sound { opened, anchor: s.anchor.clone(), meta: s.source.meta.clone(), video: s.source.has_video(), audio }
    };
    if sound.opened.is_empty() || sound.audio.is_empty() {
        return;
    }
    let mut w = kept::W(Vec::new());
    kept::Part::put(&sound, &mut w);
    let _ = kept::write(path, "sourcer", w.0);
}
