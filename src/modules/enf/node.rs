//! The Power hum (ENF) node in the workbench: what it keeps while running, the area its grid
//! frequency allows, its window, its timeline marks and its part of the result file.
//! Compiled into the `corroborate` program (see `src/bin/corroborate/main.rs`).

use super::*;
use corroborate::module::{Finding, Module};
use corroborate::modules::enf::{Enf, EnfPoint, EnfSegment};

/// What an ENF node has produced so far.
#[derive(Default)]
pub struct EnfState {
    pub points: Vec<EnfPoint>,
    pub segments: Vec<EnfSegment>,
    pub nominal: Option<f64>,
    /// Where the measured grid frequency is used (AND the location wired in), once decided.
    pub area: Option<Arc<WorldMask>>,
    /// How that area was reached: regions, data source, contradictions.
    pub area_note: String,
}

/// An ENF node in a run: its state, the grid map wired into it, and the location wired into it (if any).
pub struct EnfJob {
    pub state: Shared,
    pub grid: Result<Arc<GridMap>, String>,
    pub prior: Option<(WorldMask, String)>,
}

/// One ENF node running: the detector, its job, and the nominal frequency last decided.
pub struct EnfRun<'a> {
    enf: Enf,
    job: &'a EnfJob,
    last: Option<f64>,
}

pub fn start(job: &EnfJob) -> EnfRun<'_> {
    EnfRun { enf: Enf::new(), job, last: None }
}

/// Where a measured nominal frequency is used, AND the location wired into the node.
/// An empty result is kept: it says the hum contradicts that location.
fn area(nominal: f64, j: &EnfJob) -> (Option<WorldMask>, String) {
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

fn segments(found: Vec<Finding>) -> impl Iterator<Item = EnfSegment> {
    found.into_iter().filter_map(|f| if let Finding::Enf(g) = f { Some(g) } else { None })
}

/// Feed one block of the shared 16 kHz audio.
pub fn push(r: &mut EnfRun, x: &[f32], anchor: &Option<Anchor>, audio_s: f64) {
    let found = r.enf.push(x);
    let pts = r.enf.take_points();
    let mut s = r.job.state.lock().unwrap();
    s.enf.points.extend(pts);
    s.enf.segments.extend(segments(found));
    s.enf.nominal = r.enf.nominal();
    s.anchor = anchor.clone();
    s.audio_s = audio_s;
    if s.enf.nominal != r.last {
        r.last = s.enf.nominal;
        if let Some(f) = s.enf.nominal {
            let (a, note) = area(f, r.job);
            s.enf.area = a.map(Arc::new);
            s.enf.area_note = note;
        }
    }
    s.status = match s.enf.nominal {
        Some(f) => format!("{f:.0} Hz grid, {} runs", s.enf.segments.len()),
        None => "listening for hum".into(),
    };
}

/// End of input: runs still open are reported.
pub fn flush(r: &mut EnfRun) {
    let found = r.enf.flush();
    r.job.state.lock().unwrap().enf.segments.extend(segments(found));
}

/// The area on the node's location output.
pub fn area_out(st: &Shared) -> Result<(Arc<Area>, String), String> {
    let s = st.lock().unwrap();
    match &s.enf.area {
        Some(a) => Ok((Arc::new(Area::add((**a).clone())), s.enf.area_note.clone())),
        None if !s.enf.area_note.is_empty() => Err(s.enf.area_note.clone()),
        None => Err(format!("no area yet ({})", if s.status.is_empty() { "not run" } else { &s.status })),
    }
}

pub fn window(ui: &mut egui::Ui, st: &Shared) {
    let s = st.lock().unwrap();
    ui.label(format!("status: {}", s.status));
    let pts: Vec<(f64, f64, bool)> = s.enf.points.iter().map(|p| (p.t, p.hz / p.harmonic as f64, p.ok)).collect();
    drop(s);
    ui.colored_label(DIM, "grid frequency (Hz, folded to the fundamental) over source time; faded = unclear reading");
    plot_series(ui, &[(Color32::from_rgb(110, 170, 255), pts)], 220.0);
    let s = st.lock().unwrap();
    for g in s.enf.segments.iter().rev().take(20) {
        ui.label(format!("{:.0}–{:.0} s · {:.3} Hz ({:.3}–{:.3}) · h{}", g.t_start, g.t_end, g.mean_hz, g.min_hz, g.max_hz, g.harmonic));
    }
    let note = s.enf.area_note.clone();
    drop(s);
    if !note.is_empty() {
        ui.separator();
        ui.colored_label(if note.starts_with("CONTRADICTION") { WARN } else { TEXT }, format!("area: {note}"));
    }
}

pub fn marks(s: &NodeState) -> Vec<Mark> {
    let (to, err) = clock(&s.anchor);
    s.enf
        .segments
        .iter()
        .map(|g| Mark {
            t0: to(g.t_start),
            t1: to(g.t_end),
            err,
            label: format!("{:.3} Hz", g.mean_hz),
            detail: format!("hum {:.3} Hz ({:.3}–{:.3}), harmonic {}", g.mean_hz, g.min_hz, g.max_hz, g.harmonic),
            col: Color32::from_rgb(110, 170, 255),
        })
        .collect()
}

pub fn json(s: &NodeState) -> String {
    let p: Vec<String> = s.enf.segments.iter().map(|e| e.to_json(s.anchor.as_ref())).collect();
    let frac = s.enf.area.as_ref().map_or("null".to_string(), |a| format!("{:.5}", a.fraction()));
    format!(",\"hum_runs\":[{}],\"area_fraction_of_earth\":{frac},\"area_note\":\"{}\"", p.join(","), json_escape(&s.enf.area_note))
}

impl App {
    /// What an ENF node needs for a run: the grid map and the location wired into it.
    pub(crate) fn enf_job(&self, i: NodeId) -> EnfJob {
        EnfJob { state: self.nodes[i].state.clone(), grid: self.grid_into(i), prior: self.manual_area_into(i, "location") }
    }
}

/// How this node looks on the canvas: the standard look. Override `NodeLook` methods here to
/// change it (see `src/bin/corroborate/look.rs`).
pub struct Look;

impl crate::look::NodeLook for Look {}
