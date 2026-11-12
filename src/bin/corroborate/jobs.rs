//! Jobs: the nodes that do work (SOURCER, Cut detector, Audio match, ENF, Planes, plugins). One
//! kind of progress for all of them, in the node's state: named steps with how far each has got,
//! what the job waits for, and how it ended. From it, the state shown on the node's dot (before its
//! name) and in the queue list (Queue in the top bar).
//!
//! docs/run-queue/plan.md, stage 1; what starts a job, and when, is the queue (queue.rs).

use crate::runtime::NodeState;
use crate::{App, Kind, NodeId, DIM, TEXT};
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use std::time::Instant;

/// Where a step of a run is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepState {
    Waiting,
    Working,
    Done,
    Failed,
}

/// One step of a run: its name, where it is, how far (0 to 1; below 0 when not known) and what
/// went wrong.
#[derive(Clone, Debug)]
pub struct Step {
    pub name: String,
    pub state: StepState,
    pub frac: f32,
    pub note: String,
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq)]
pub enum End {
    Done,
    /// Loaded from its kept result (`kept.rs`), not worked out again.
    Kept,
    Failed(String),
    Cancelled,
}

/// A run's steps, what it waits for, when it started, how it ended and how long it took.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub steps: Vec<Step>,
    pub started: Option<Instant>,
    pub took: Option<f64>,
    /// While queued: what it waits for (another job, a turn), in words: "for a turn to decode (4
    /// of 4 in use)".
    pub waits: Option<String>,
    pub end: Option<End>,
}

impl Progress {
    /// A run starting now with these steps, all waiting.
    pub fn new<S: Into<String>>(names: impl IntoIterator<Item = S>) -> Self {
        let steps = names.into_iter().map(|n| Step { name: n.into(), state: StepState::Waiting, frac: -1.0, note: String::new() }).collect();
        Progress { steps, started: Some(Instant::now()), ..Default::default() }
    }

    /// Step `name`'s state, how far it has got and what went wrong. A step working ends the wait.
    pub fn step(&mut self, name: &str, state: StepState, frac: f32, note: &str) {
        if let Some(k) = self.steps.iter_mut().find(|k| k.name == name) {
            k.state = state;
            k.frac = frac;
            k.note = note.to_string();
        }
        if state == StepState::Working {
            self.waits = None;
        }
    }

    /// The run ended: how, and how long it took.
    pub fn finish(&mut self, end: End) {
        self.waits = None;
        self.took = self.started.map(|t| t.elapsed().as_secs_f64());
        self.end = Some(end);
    }

    /// The step working now: its name and how far (below 0: not known).
    pub fn working(&self) -> Option<(String, f32)> {
        self.steps.iter().find(|k| k.state == StepState::Working).map(|k| (k.name.clone(), k.frac))
    }
}

/// "for a turn to decode (4 of 4 in use)".
pub fn turn_waits() -> String {
    let n = corroborate::ingest::decoders_at_once();
    format!("for a turn to decode ({} of {n} in use)", corroborate::ingest::decoding_now().min(n))
}

/// "for a turn to download (2 of 2 in use)".
pub fn download_waits() -> String {
    let d = &corroborate::ingest::DOWNLOADS;
    format!("for a turn to download ({} of {} in use)", d.now().min(d.limit()), d.limit())
}

/// What waiting for job `label` (whose state is `s`) is waiting for: "for SOURCER 'phone 3'
/// (download 34%)".
pub fn waits_for(label: &str, s: &NodeState) -> String {
    let v = view(s);
    let how = match (v.state, &v.step) {
        (JobState::Working, Some((step, f))) if *f >= 0.0 => format!(" ({step} {:.0}%)", f * 100.0),
        (JobState::Working, Some((step, _))) => format!(" ({step})"),
        (JobState::NotRun, _) => String::new(),
        (st, _) => format!(" ({})", st.word()),
    };
    format!("for '{label}'{how}")
}

/// What a job is doing, as its dot and the queue list show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    NotRun,
    /// Its result is from before a change to what went into it.
    Stale,
    Queued,
    Working,
    Done,
    /// Loaded from its kept result, not worked out again.
    Kept,
    Failed,
    Cancelled,
    Live,
}

impl JobState {
    /// Done, in this session or before (kept): its result is there.
    pub fn has_result(self) -> bool {
        matches!(self, JobState::Done | JobState::Kept)
    }

    pub fn word(self) -> &'static str {
        match self {
            JobState::NotRun => "not run",
            JobState::Stale => "stale",
            JobState::Queued => "queued",
            JobState::Working => "working",
            JobState::Done => "done",
            JobState::Kept => "kept",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
            JobState::Live => "live",
        }
    }

    /// Each state's colour (the mockup's).
    pub fn color(self) -> Color32 {
        match self {
            JobState::NotRun => Color32::from_rgb(0x56, 0x5c, 0x66),
            JobState::Stale => Color32::from_rgb(0xe0, 0xa4, 0x58),
            JobState::Queued => Color32::from_rgb(0x8a, 0x90, 0x99),
            JobState::Working => Color32::from_rgb(0xe0, 0xc8, 0x5a),
            JobState::Done => Color32::from_rgb(0x8c, 0xdc, 0x6e),
            JobState::Kept => Color32::from_rgb(0x6c, 0xc0, 0xa8),
            JobState::Failed => Color32::from_rgb(0xe0, 0x52, 0x4f),
            JobState::Cancelled => Color32::from_rgb(0x9a, 0x7a, 0x7a),
            JobState::Live => Color32::from_rgb(0xff, 0x5a, 0x5a),
        }
    }
}

/// A job's state, step and how far, what it waits for or why it failed, how long it took.
#[derive(Clone, Debug)]
pub struct JobView {
    pub state: JobState,
    pub step: Option<(String, f32)>,
    /// Queued: what it waits for. Failed: why. Live: how much came in.
    pub why: String,
    pub took: Option<f64>,
    pub started: Option<Instant>,
    /// Its place in the order asked (the queue list's order).
    pub order: Option<u64>,
}

impl JobView {
    /// One line: "working: decode 34%", "queued: for a turn to decode (4 of 4 in use)", "done in
    /// 4.2 s", "failed: …".
    pub fn text(&self) -> String {
        let w = self.state.word();
        match self.state {
            JobState::Working => match &self.step {
                Some((s, f)) if *f >= 0.0 => format!("{w}: {s} {:.0}%", f * 100.0),
                Some((s, _)) => format!("{w}: {s}"),
                None => w.into(),
            },
            JobState::Done => match self.took {
                Some(t) => format!("done in {}", took_text(t)),
                None => w.into(),
            },
            JobState::Kept => "kept: loaded from cache\\, not worked out again".into(),
            JobState::NotRun => "not run: Run or ▶ starts it".into(),
            JobState::Stale => "stale: changed since it ran".into(),
            _ if self.why.is_empty() => w.into(),
            _ => format!("{w}: {}", self.why),
        }
    }
}

/// How long a run took: seconds with a tenth under a minute, else minutes and seconds.
pub fn took_text(s: f64) -> String {
    if s < 60.0 { format!("{s:.1} s") } else { crate::sourcer_node::fmt_secs(s) }
}

/// The state of the job whose node state is `s`.
pub fn view(s: &NodeState) -> JobView {
    let p = &s.progress;
    let state = if s.running && s.live {
        JobState::Live
    } else if s.running {
        if p.waits.is_some() { JobState::Queued } else { JobState::Working }
    } else {
        match &p.end {
            Some(End::Done) => JobState::Done,
            Some(End::Kept) => JobState::Kept,
            Some(End::Failed(_)) => JobState::Failed,
            Some(End::Cancelled) => JobState::Cancelled,
            None if s.done => JobState::Done,
            None => JobState::NotRun,
        }
    };
    let why = match (&p.end, state) {
        (_, JobState::Queued) => p.waits.clone().unwrap_or_default(),
        (Some(End::Failed(e)), _) => e.clone(),
        (_, JobState::Live) => format!("{:.0} s in", s.audio_s),
        _ => String::new(),
    };
    JobView { state, step: p.working(), why, took: p.took, started: p.started, order: None }
}

/// Does node kind `k` do work (a job, with a state dot)?
pub fn is_job(k: &Kind) -> bool {
    matches!(k, Kind::Sourcer { .. } | Kind::CutDetector { .. } | Kind::AudioMatch { .. } | Kind::Enf { .. } | Kind::Planes | Kind::Plugin { .. })
}

/// The dot's radius, before zoom.
pub const DOT_R: f32 = 4.5;

/// The state dot at `c`: a dot in the state's colour; queued or working, a turning ring in it
/// (slowly while queued); live, a pulsing dot. True while it moves (the canvas draws again).
pub fn draw_dot(p: &egui::Painter, c: Pos2, r: f32, st: JobState, now: f64) -> bool {
    let col = st.color();
    match st {
        JobState::Queued | JobState::Working => {
            let turns = if st == JobState::Queued { 0.35 } else { 1.2 };
            let a0 = (now * turns * std::f64::consts::TAU) as f32;
            let pts: Vec<Pos2> = (0..=20).map(|k| a0 + k as f32 / 20.0 * 4.7).map(|a| c + Vec2::angled(a) * r).collect();
            p.circle_stroke(c, r, Stroke::new(r * 0.32, col.gamma_multiply(0.25)));
            p.add(egui::Shape::line(pts, Stroke::new(r * 0.38, col)));
            true
        }
        JobState::Live => {
            let k = 0.6 + 0.4 * (now * 4.0).sin() as f32;
            p.circle_filled(c, r * (0.75 + 0.25 * k), col.gamma_multiply(k));
            true
        }
        _ => {
            p.circle_filled(c, r, col);
            false
        }
    }
}

impl App {
    /// Node `i`'s job state, or None when it does no work: its run's, or the queue's while it
    /// waits there or ended there; stale when it ran on what has changed since.
    pub(crate) fn job_view(&self, i: NodeId) -> Option<JobView> {
        let n = &self.nodes[i];
        if n.hidden || !is_job(&n.kind) {
            return None;
        }
        let mut v = view(&n.state.lock().unwrap());
        let Some(r) = self.queue.recs.get(&i) else { return Some(v) };
        v.order = Some(r.order);
        let stopping = r.stop.as_ref().is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed));
        if self.queue.waiting.contains(&i) {
            v.state = JobState::Queued;
            v.step = None;
            v.why = self.waits_text(i);
            v.started = r.asked;
        } else if let Some((st, why)) = &r.ended {
            v.state = *st;
            v.why = why.clone();
            v.step = None;
            v.took = None;
        } else if stopping && matches!(v.state, JobState::Queued | JobState::Working | JobState::Live) {
            // Told to stop: it is ending.
            v.state = JobState::Cancelled;
            v.why.clear();
        } else if matches!(v.state, JobState::Done | JobState::Kept | JobState::Failed | JobState::Cancelled) && r.ran.is_some() && r.ran != self.queue.keys.get(&i).copied() && self.queue.keys.contains_key(&i) {
            v.state = JobState::Stale;
        }
        Some(v)
    }

    /// Every job on the canvas's levels, in the order asked (never asked last).
    pub(crate) fn jobs(&self) -> Vec<(NodeId, JobView)> {
        let mut v: Vec<(NodeId, JobView)> = (0..self.nodes.len()).filter_map(|i| self.job_view(i).map(|j| (i, j))).collect();
        v.sort_by_key(|(i, j)| (j.order.is_none(), j.order, *i));
        v
    }

    /// The counts on the Queue button: "2 working · 3 queued", else how the jobs ended.
    pub(crate) fn queue_counts(&self) -> String {
        let jobs = self.jobs();
        let count = |s: JobState| jobs.iter().filter(|(_, j)| j.state == s).count();
        let order = [JobState::Live, JobState::Working, JobState::Queued, JobState::Stale, JobState::Failed, JobState::Cancelled, JobState::Done, JobState::Kept];
        let parts: Vec<String> = order.iter().filter_map(|&s| (count(s) > 0).then(|| format!("{} {}", count(s), s.word()))).collect();
        if parts.is_empty() { "nothing run".into() } else { parts.join(" · ") }
    }

    /// The queue list: every job, in the order asked, with its state, step and how far, what it
    /// waits for, how long it took or why it failed; ■ cancels one queued or working, ▶ runs one.
    pub(crate) fn queue_window(&mut self, ctx: &egui::Context) {
        if !self.queue_open {
            return;
        }
        let jobs = self.jobs();
        let now = ctx.input(|i| i.time);
        let mut open = true;
        let mut moving = false;
        let mut act: Option<(NodeId, bool)> = None;
        let cache = self.cache_dir();
        let mut forget = false;
        egui::Window::new("Queue").open(&mut open).resizable(true).default_width(420.0).default_pos(Pos2::new(220.0, 60.0)).show(ctx, |ui| {
            if jobs.is_empty() {
                ui.label(egui::RichText::new("no nodes do work here").color(DIM));
                return;
            }
            egui::Grid::new("queue grid").num_columns(4).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
                for (i, j) in &jobs {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
                    moving |= draw_dot(ui.painter(), r.center(), DOT_R, j.state, now);
                    ui.label(egui::RichText::new(&self.nodes[*i].label).color(TEXT));
                    let text = j.text();
                    if let (JobState::Working, Some((_, f))) = (j.state, &j.step) {
                        ui.horizontal(|ui| {
                            ui.label(&text);
                            if *f >= 0.0 {
                                ui.add(egui::ProgressBar::new(*f).desired_width(80.0));
                            }
                        });
                    } else {
                        // A long reason (yt-dlp's errors) is cut short; hover for all of it.
                        let short = if text.chars().count() > 90 { format!("{}…", text.chars().take(89).collect::<String>()) } else { text.clone() };
                        let l = ui.label(egui::RichText::new(short).color(if j.state == JobState::Failed { j.state.color() } else { DIM }));
                        if text.chars().count() > 90 {
                            l.on_hover_text(&text);
                        }
                    }
                    let busy = matches!(j.state, JobState::Queued | JobState::Working | JobState::Live);
                    let b = ui.small_button(if busy { "■" } else { "▶" }).on_hover_text(if busy { "cancel it (and what needs it)" } else { "run it and what it needs" });
                    if b.clicked() {
                        act = Some((*i, !busy));
                    }
                    ui.end_row();
                }
            });
            // How much is kept in the project's cache\ folder.
            if let Some(dir) = &cache {
                ui.separator();
                ui.horizontal(|ui| {
                    let n = crate::kept::size(dir);
                    ui.label(egui::RichText::new(format!("kept: {}", crate::kept::size_text(n))).color(DIM)).on_hover_text(dir.display().to_string());
                    if n > 0 && ui.small_button("Forget all kept").on_hover_text("delete every kept result of this project (cache\\): they are worked out again when they run").clicked() {
                        forget = true;
                    }
                });
            }
        });
        if forget {
            self.forget_all_kept();
        }
        if moving {
            ctx.request_repaint();
        }
        match act {
            Some((i, true)) => self.run_node(i),
            Some((i, false)) => self.cancel_job(i),
            None => {}
        }
        self.queue_open = open;
    }
}
