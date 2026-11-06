//! The Power hum (ENF) node in the workbench: what it keeps while running, the area its grid
//! frequency allows, its window, its timeline marks and its part of the result file.
//! Compiled into the `corroborate` program (see `src/bin/corroborate/main.rs`).

use super::*;
use corroborate::module::{Finding, Module};
use corroborate::modules::enf::{quality, sections, Enf, EnfPoint, EnfSegment, Level, Quality};

/// What an ENF node has produced so far.
#[derive(Default)]
pub struct EnfState {
    pub points: Vec<EnfPoint>,
    pub segments: Vec<EnfSegment>,
    pub nominal: Option<f64>,
    /// The signal quality verdict over all readings so far.
    pub quality: Option<Quality>,
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
    if !pts.is_empty() {
        s.enf.points.extend(pts);
        s.enf.quality = Some(quality(&s.enf.points));
    }
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
    let mut s = r.job.state.lock().unwrap();
    s.enf.segments.extend(segments(found));
    if let Some(f) = s.enf.nominal {
        s.status = format!("{f:.0} Hz grid, {} runs", s.enf.segments.len());
    }
}

/// The area on the node's location output.
pub fn area_out(st: &Shared) -> Result<(Arc<Area>, String), String> {
    let s = st.lock().unwrap();
    // The output follows the verdict: no area while no reading counts, even if the grid vote
    // already settled on 50 or 60 Hz.
    if let Some(q) = &s.enf.quality {
        if q.level == Level::None && s.enf.area.is_some() {
            return Err(format!("no hum counted yet ({}): no area", q.verdict()));
        }
    }
    match &s.enf.area {
        Some(a) => Ok((Arc::new(Area::add((**a).clone())), s.enf.area_note.clone())),
        None if !s.enf.area_note.is_empty() => Err(s.enf.area_note.clone()),
        None => Err(format!("no area yet ({})", if s.status.is_empty() { "not run" } else { &s.status })),
    }
}

pub fn json(s: &NodeState) -> String {
    let p: Vec<String> = s.enf.segments.iter().map(|e| e.to_json(s.anchor.as_ref())).collect();
    let frac = s.enf.area.as_ref().map_or("null".to_string(), |a| format!("{:.5}", a.fraction()));
    let q = s.enf.quality.as_ref().map_or("null".to_string(), |q| {
        let num = |v: f64| if v.is_finite() { format!("{v:.5}") } else { "null".into() };
        let rej: Vec<String> = q.rejected.iter().map(|(r, n)| format!("\"{}\":{n}", r.label())).collect();
        format!(
            "{{\"level\":\"{}\",\"verdict\":\"{}\",\"kept\":{},\"total\":{},\"score_db\":{},\"longest_run_s\":{:.1},\"err_hz\":{},\"harmonics\":{:?},\"steady\":{},\"rejected\":{{{}}}}}",
            q.level.word(),
            q.verdict(),
            q.kept,
            q.total,
            num(q.score_db),
            q.longest_run_s,
            num(q.err_hz),
            q.harmonics,
            q.steady,
            rej.join(",")
        )
    });
    format!(
        ",\"hum_runs\":[{}],\"quality\":{q},\"area_fraction_of_earth\":{frac},\"area_note\":\"{}\"",
        p.join(","),
        json_escape(&s.enf.area_note)
    )
}

impl App {
    /// What an ENF node needs for a run: the grid map chosen in the node.
    pub(crate) fn enf_job(&self, i: NodeId) -> EnfJob {
        let path = match &self.nodes[i].kind {
            Kind::Enf { grid } => grid.clone(),
            _ => String::new(),
        };
        EnfJob { state: self.nodes[i].state.clone(), grid: grid_frequencies_node::load(&path).map(Arc::new), prior: None }
    }
}

// ---------------------------------------------------------------- on the canvas
//
//  ┌ ENF · Electric grid hum ───────────────────────────┐
//  │        ┌──────── Earth map ────────┐  STATUS        │
//  ● audio  │  (where the grid is used) │  …             │ location ●
//  │        ├──── hum frequency trace ──┤                │
//  │        grid data [file……] [Browse]                  │
//  └─────────────────────────────────────────────────────┘

/// Size on the canvas (unzoomed).
const NODE_W: f32 = 520.0;
const NODE_H: f32 = 340.0;
/// Room left at the sides for the port names, and the status column's width.
const LEFT: f32 = 42.0;
const RIGHT: f32 = 60.0;
const STATUS_W: f32 = 128.0;
const TRACE_H: f32 = 58.0;
const HUM: Color32 = Color32::from_rgb(110, 170, 255);

/// The grid file's summary (regions and source), read once per path: drawing must not read the
/// file every frame. The run reads the file itself at each Run.
fn grid_summary(path: &str) -> Result<String, String> {
    static CACHE: std::sync::Mutex<Option<HashMap<String, Result<String, String>>>> = std::sync::Mutex::new(None);
    let mut c = CACHE.lock().unwrap();
    c.get_or_insert_with(HashMap::new)
        .entry(path.to_string())
        .or_insert_with(|| grid_frequencies_node::load(path).map(|g| format!("{} regions · {}", g.regions.len(), g.source)))
        .clone()
}

/// The hum frequency over source time (folded to the fundamental); faded where unclear.
fn trace(p: &egui::Painter, rect: Rect, z: f32, pts: &[(f64, f64, bool)], ran: bool) {
    p.rect_filled(rect, 4.0 * z, BG);
    let font = FontId::proportional(10.0 * z);
    p.text(rect.left_top() + Vec2::new(6.0, 4.0) * z, Align2::LEFT_TOP, "hum frequency", font.clone(), DIM);
    let (mut t0, mut t1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    let clear = pts.iter().any(|q| q.2);
    for &(t, y, ok) in pts {
        t0 = t0.min(t);
        t1 = t1.max(t);
        // With no clear reading yet, scale to the unclear ones so they still show.
        if ok || !clear {
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
    }
    if t0 >= t1 || y0 > y1 {
        let note = if ran { "listening: no hum reading yet" } else { "no hum yet: waiting for audio" };
        p.text(rect.center(), Align2::CENTER_CENTER, note, font, DIM);
        return;
    }
    if !clear {
        p.text(rect.center(), Align2::CENTER_CENTER, format!("{} readings, none clear: no steady hum", pts.len()), font.clone(), WARN);
    }
    if y1 - y0 < 0.02 {
        let m = (y0 + y1) / 2.0;
        (y0, y1) = (m - 0.01, m + 0.01);
    }
    let plot = Rect::from_min_max(rect.min + Vec2::new(6.0, 16.0) * z, rect.max - Vec2::new(6.0, 4.0) * z);
    let x = |t: f64| plot.left() + ((t - t0) / (t1 - t0)) as f32 * plot.width();
    let y = |v: f64| plot.bottom() - ((v - y0) / (y1 - y0)).clamp(0.0, 1.0) as f32 * plot.height();
    let line: Vec<Pos2> = pts.iter().filter(|q| q.2).map(|&(t, v, _)| Pos2::new(x(t), y(v))).collect();
    p.add(egui::Shape::line(line, Stroke::new(1.3 * z, HUM)));
    for &(t, v, ok) in pts {
        if !ok {
            p.circle_filled(Pos2::new(x(t), y(v)), 1.2 * z, alpha(HUM, 0.25));
        }
    }
    let last = pts.iter().rev().find(|q| q.2).map_or(String::new(), |q| format!("{:.3} Hz", q.1));
    p.text(rect.right_top() + Vec2::new(-6.0, 4.0) * z, Align2::RIGHT_TOP, last, font, TEXT);
}

/// The ENF node as sketched: the map of where its grid frequency is used, the hum trace under
/// it, the grid data chosen right here, and a status column. No separate window.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, _app: &App, _i: NodeId, _standard: Vec2) -> Vec2 {
        Vec2::new(NODE_W, NODE_H)
    }

    fn fill(&self, _app: &App, _i: NodeId) -> Color32 {
        Color32::from_rgb(34, 40, 50)
    }

    fn status(&self) -> bool {
        false
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let z = app.zoom;
        let r = ui.max_rect();
        let content = Rect::from_min_max(Pos2::new(r.left() + LEFT * z, r.top()), Pos2::new(r.right() - RIGHT * z, r.bottom()));
        let status = Rect::from_min_max(Pos2::new(content.right() - STATUS_W * z, content.top()), content.max);
        let main_w = status.left() - 8.0 * z - content.left();
        let map = Rect::from_min_size(content.min, Vec2::new(main_w, main_w / ROBINSON_ASPECT as f32));
        let band = Rect::from_min_size(map.left_bottom() + Vec2::new(0.0, 6.0 * z), Vec2::new(main_w, TRACE_H * z));
        let grid_row = Rect::from_min_max(band.left_bottom() + Vec2::new(0.0, 8.0 * z), Pos2::new(map.right(), content.bottom()));

        let st = app.nodes[i].state.clone();
        let p = ui.painter().clone();

        // Earth map: where the measured grid frequency is used.
        let area = area_out(&st).ok().map(|(a, _)| a);
        map_node::paint(&p.with_clip_rect(map), map, app.earth.as_ref().map(|t| t.id()), area.as_deref());
        p.rect_stroke(map, 6.0 * z, Stroke::new(1.0 * z, LINE), StrokeKind::Outside);

        // Hum frequency over time: the last 10 minutes of readings.
        let (pts, ran) = {
            let s = st.lock().unwrap();
            let from = s.enf.points.len().saturating_sub(600);
            // The section rule applied, so the line shows only readings that count.
            (sections(&s.enf.points[from..]).iter().map(|q| (q.t, q.hz, q.ok)).collect::<Vec<_>>(), s.running || s.done)
        };
        trace(&p, band, z, &pts, ran);

        // Grid data, changed here.
        if let Kind::Enf { grid } = &mut app.nodes[i].kind {
            ui.scope_builder(egui::UiBuilder::new().max_rect(grid_row), |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(DIM, "grid data");
                    ui.add(egui::TextEdit::singleline(grid).hint_text("bundled file").desired_width(main_w - 150.0 * z));
                    if ui.button("Browse…").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                            *grid = f.to_string_lossy().into_owned();
                        }
                    }
                });
                match grid_summary(grid) {
                    Ok(t) => ui.add(egui::Label::new(egui::RichText::new(t).small().color(DIM)).truncate()),
                    Err(e) => ui.add(egui::Label::new(egui::RichText::new(e).small().color(WARN)).truncate()),
                };
            });
        }

        // Status data.
        p.line_segment([status.left_top() - Vec2::new(4.0 * z, 0.0), status.left_bottom() - Vec2::new(4.0 * z, 0.0)], Stroke::new(1.0 * z, LINE));
        let s = st.lock().unwrap();
        let lines: Vec<(String, Color32)> = vec![
            (if s.running { "● running".into() } else { s.status.clone() }, if s.running { LIVE } else { TEXT }),
            (s.enf.nominal.map_or("grid: deciding".into(), |f| format!("grid: {f:.0} Hz")), TEXT),
            (format!("hum runs: {}", s.enf.segments.len()), TEXT),
            (format!("audio: {:.0} s", s.audio_s), DIM),
            (
                match (&s.enf.area, s.enf.quality.as_ref().map(|q| q.level)) {
                    (Some(_), Some(Level::None)) => "area: none (no hum counted)".into(),
                    (Some(a), _) => format!("area: {:.1} % of Earth", a.fraction() * 100.0),
                    (None, _) => "area: none yet".into(),
                },
                TEXT,
            ),
            (s.enf.area_note.clone(), if s.enf.area_note.starts_with("CONTRADICTION") { WARN } else { DIM }),
        ];
        let q = s.enf.quality.clone();
        drop(s);
        ui.scope_builder(egui::UiBuilder::new().max_rect(status), |ui| {
            // The quality verdict first: is the hum good enough, and why.
            ui.label(egui::RichText::new("QUALITY").small().color(DIM));
            match &q {
                Some(q) => {
                    let c = match q.level {
                        Level::Good => GOOD,
                        Level::Weak => WARN,
                        Level::None => DIM,
                    };
                    ui.add(egui::Label::new(egui::RichText::new(q.verdict()).color(c)).wrap());
                    for l in q.lines() {
                        ui.add(egui::Label::new(egui::RichText::new(l).small().color(TEXT)).wrap());
                    }
                }
                None => {
                    ui.label(egui::RichText::new("no readings yet").small().color(DIM));
                }
            }
            ui.add_space(4.0 * z);
            ui.label(egui::RichText::new("STATUS").small().color(DIM));
            for (t, c) in lines.into_iter().filter(|(t, _)| !t.is_empty()) {
                ui.add(egui::Label::new(egui::RichText::new(t).small().color(c)).wrap());
            }
        });
    }
}
