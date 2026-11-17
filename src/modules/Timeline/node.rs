//! The Timeline node in the workbench: a row per recording, bars with their margins, one real-time
//! playhead. A recording whose start time is wired in follows that playhead (playhead.rs
//! `update_follow`); one that is not keeps its own. Compiled into the `corroborate` program.

use super::*;
use crate::audio_match_node::{fit_axis, row_slider, scale_button, tick_step, tick_text};
use corroborate::data::Data;
use corroborate::module_manifest::{PortLabel, PortLook, Side};

const NODE_W: f32 = 820.0;
const TOP: f32 = HEAD + 4.0;
const BAR_H: f32 = 30.0;
const RULER_H: f32 = 24.0;
const LEFT_W: f32 = 190.0;
const ROW_DEFAULT: f32 = 40.0;
const ROW_MIN: f32 = 20.0;
const ROW_MAX: f32 = 96.0;
const BAR: Color32 = Color32::from_rgb(125, 180, 230);
const TRACK: Color32 = Color32::from_rgb(29, 32, 37);
const SEAM: Color32 = Color32::from_rgb(16, 18, 21);
const GRID: Color32 = Color32::from_rgb(49, 53, 60);

fn row_height(kind: &Kind) -> f32 {
    kind.rows().iter().find(|(k, _)| k == "row height").and_then(|(_, v)| v.parse().ok()).unwrap_or(ROW_DEFAULT).clamp(ROW_MIN, ROW_MAX)
}

fn sorted(kind: &Kind) -> bool {
    kind.rows().iter().any(|(k, v)| k == "sort by time" && v == "true")
}

fn place_y(slot: usize, rh: f32) -> f32 {
    TOP + BAR_H + RULER_H + slot as f32 * rh
}

/// One recording's row.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub n: usize,
    pub name: String,
    /// A wire goes into its media or time port.
    pub wired: bool,
    pub len: f64,
    /// When it starts and that time's error.
    pub at: Option<(f64, f64)>,
    /// When it ends and that time's error, when wired.
    pub end: Option<(f64, f64)>,
    /// The recording (its playhead's clock): its media's, else the one its start time belongs to.
    pub clock: Option<String>,
}

impl App {
    /// Timeline node `i`'s rows, from what is wired in.
    pub(crate) fn timeline_rows(&self, i: NodeId) -> Vec<Row> {
        let spec = &self.graph.nodes()[i].spec;
        spec.inputs
            .chunks(3)
            .enumerate()
            .map(|(k, ports)| {
                let mut r = Row { n: k + 1, ..Default::default() };
                if let [media, time, end] = ports {
                    if let Some(pf) = self.graph.feeder(i, media.name).and_then(|(f, o)| self.piece_feed(f, o)) {
                        // A piece of a cut recording.
                        r.wired = true;
                        r.name = pf.name.clone();
                        r.len = pf.piece.len();
                        r.clock = Some(pf.clock.clone());
                    } else if let Some((f, _)) = self.graph.feeder(i, media.name) {
                        r.wired = true;
                        let path = self.source_file(f);
                        r.name = match path.as_deref() {
                            Some(p) if self.nodes[f].label == "SOURCER" => p.rsplit(['\\', '/']).next().unwrap_or(p).to_string(),
                            _ => self.nodes[f].label.clone(),
                        };
                        r.len = self.source_length(f).unwrap_or(0.0);
                        r.clock = self.clock_of(f);
                    }
                    let time_in = |port: &str| {
                        let (f, o) = self.graph.feeder(i, port)?;
                        match self.value_from(f, o) {
                            Ok(Data::Time { utc, error_s, recording, .. }) => Some(Some((recording, utc, error_s))),
                            _ => Some(None),
                        }
                    };
                    let start = time_in(time.name);
                    let stop = time_in(end.name);
                    r.wired |= start.is_some() || stop.is_some();
                    if let Some(Some((rec, u, e))) = start {
                        if r.clock.is_none() && !rec.is_empty() {
                            r.clock = Some(rec);
                        }
                        r.at = Some((u, e));
                    }
                    if let Some(Some((_, u, e))) = stop {
                        r.end = Some((u, e));
                    }
                }
                r
            })
            .collect()
    }
}

/// The node on the canvas.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, _standard: Vec2) -> Vec2 {
        let n = app.graph.nodes()[i].spec.inputs.len() / 2;
        Vec2::new(NODE_W, place_y(n, row_height(&app.nodes[i].kind)) + 12.0)
    }

    fn port_look(&self, app: &App, i: NodeId, name: &str) -> Option<PortLook> {
        let rh = row_height(&app.nodes[i].kind);
        let row = |n: usize| {
            let k = n.saturating_sub(1);
            place_y(app.match_ui.slots.get(&i).and_then(|s| s.get(k)).copied().unwrap_or(k), rh)
        };
        let number = |base: &str| name.strip_prefix(base).and_then(|n| n.parse::<usize>().ok());
        let at = |y: f32| Some(PortLook { side: Some(Side::Left), at: Some(y), label: Some(PortLabel::Hidden), dot_size: Some((rh * 0.12).clamp(2.5, 5.0)), ..Default::default() });
        if let Some(n) = number("media ") {
            return at(row(n) + rh * 0.22);
        }
        if let Some(n) = number("time ") {
            return at(row(n) + rh * 0.52);
        }
        at(row(number("end ")?) + rh * 0.82)
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let body = ui.max_rect();
        let z = app.zoom;
        let rows = app.timeline_rows(i);
        let rh = row_height(&app.nodes[i].kind);
        let by_time = sorted(&app.nodes[i].kind);
        let p = ui.painter().with_clip_rect(ui.clip_rect());
        let font = |s: f32| FontId::proportional(s * z);
        let mono = |s: f32| FontId::monospace(s * z);

        let mut order: Vec<usize> = (0..rows.len()).collect();
        if by_time {
            order.sort_by(|&a, &b| {
                let key = |k: usize| (!rows[k].wired, rows[k].at.is_none(), rows[k].at.map_or(0.0, |x| x.0));
                let (x, y) = (key(a), key(b));
                x.0.cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.total_cmp(&y.2)).then(a.cmp(&b))
            });
        }
        let mut slots = vec![0; order.len()];
        for (s, &k) in order.iter().enumerate() {
            slots[k] = s;
        }
        app.match_ui.slots.insert(i, slots);

        let rows_top = body.top() + (BAR_H + RULER_H) * z;
        let rows_bottom = rows_top + order.len() as f32 * rh * z;
        let track = Rect::from_min_max(Pos2::new(body.left() + LEFT_W * z, rows_top - RULER_H * z), Pos2::new(body.right(), rows_bottom));
        if track.width() <= 10.0 {
            return;
        }
        // A row's bar runs from its start to its wired end, else to start + length.
        let stop = |r: &Row, t: f64, e: f64| r.end.unwrap_or((t + r.len, e));
        let bars: Vec<(f64, f64)> = rows.iter().filter_map(|r| r.at.map(|(t, e)| (t - e, stop(r, t, e).0 + stop(r, t, e).1))).collect();
        let fit = fit_axis(&bars);
        let (v0, v1) = app.match_ui.views.get(&i).copied().unwrap_or(fit);
        let span = (v1 - v0).max(1e-3);
        let tx = |t: f64| track.left() + ((t - v0) / span) as f32 * track.width();

        // The bar along the top.
        let y = body.top() + BAR_H * 0.5 * z;
        let always = !app.match_ui.views.contains_key(&i);
        let button = |ui: &mut egui::Ui, x: f32, w: f32, text: &str, on: bool| {
            let b = egui::Button::new(egui::RichText::new(text).size(11.5 * z)).fill(if on { Color32::from_rgb(81, 88, 98) } else { Color32::from_rgb(58, 63, 71) });
            ui.put(Rect::from_min_size(Pos2::new(body.left() + x * z, y - 10.0 * z), Vec2::new(w * z, 20.0 * z)), b).clicked()
        };
        let playing = app.ph.playing;
        let play = button(ui, 4.0, 44.0, if playing { "Pause" } else { "Play" }, playing);
        let (do_fit, toggle_always) = scale_button(ui, i, Rect::from_min_size(Pos2::new(body.left() + 52.0 * z, y - 10.0 * z), Vec2::new(98.0 * z, 20.0 * z)), z, always);
        let toggle_sort = button(ui, 154.0, 90.0, "Sort by time", by_time);
        p.text(Pos2::new(body.left() + 258.0 * z, y), Align2::LEFT_CENTER, "row height", font(11.5), DIM);
        let mut new_rh = rh;
        row_slider(ui, Pos2::new(body.left() + 320.0 * z, y), z, &mut new_rh, ROW_MIN..=ROW_MAX);
        if let Some(u) = app.ph.utc {
            p.text(Pos2::new(body.right() - 6.0 * z, y), Align2::RIGHT_CENTER, format!("{} UTC", timebase::fmt_utc(u)), mono(11.5), PLAYHEAD);
        }

        // The ruler and grid.
        p.rect_filled(track, 0.0, TRACK);
        let step = tick_step(span);
        let mut q = (v0 / step).ceil();
        let mut ticks = 0;
        while q * step <= v1 && ticks < 30 {
            let x = tx(q * step);
            p.line_segment([Pos2::new(x, track.top()), Pos2::new(x, track.bottom())], Stroke::new(z.max(1.0), GRID));
            p.text(Pos2::new(x + 4.0 * z, track.top() + RULER_H * 0.5 * z), Align2::LEFT_CENTER, tick_text(q * step, step), mono(11.0), DIM);
            q += 1.0;
            ticks += 1;
        }

        // The rows.
        for (slot, &k) in order.iter().enumerate() {
            let r = &rows[k];
            let top = rows_top + slot as f32 * rh * z;
            p.rect_filled(Rect::from_min_max(Pos2::new(body.left(), top), Pos2::new(body.right(), top + z)), 0.0, SEAM);
            let bottom = top + rh * z;
            let cy = (top + bottom) * 0.5;
            let label_x = body.left() + 12.0 * z;
            if !r.wired {
                p.text(Pos2::new(label_x, cy), Align2::LEFT_CENTER, format!("audio or video {0}, start time {0}, end time {0}", r.n), font(11.5), alpha(DIM, 0.8));
                continue;
            }
            let dim = r.at.is_none();
            let name = if r.name.is_empty() { "no recording wired".to_string() } else { r.name.clone() };
            let shown = if name.chars().count() > 27 { format!("{}…", name.chars().take(26).collect::<String>()) } else { name };
            p.text(Pos2::new(label_x, cy), Align2::LEFT_CENTER, shown, mono(11.5), if dim { DIM } else { TEXT });
            let Some((t, e)) = r.at else {
                p.rect_filled(Rect::from_min_max(Pos2::new(track.left(), top + z), Pos2::new(track.right(), bottom)), 0.0, Color32::from_rgba_unmultiplied(70, 70, 70, 60));
                p.text(Pos2::new(track.left() + 8.0 * z, cy), Align2::LEFT_CENTER, "no time", mono(11.0), DIM);
                continue;
            };
            let band = |a: f64, b: f64| Rect::from_min_max(Pos2::new(tx(a).max(track.left()), top + 3.0 * z), Pos2::new(tx(b).min(track.right()), bottom - 2.0 * z));
            let vis = |a: f64, b: f64| b >= v0 && a <= v1;
            let (u1, e1) = stop(r, t, e);
            for (a, b, m) in [(t - e, t, e), (u1, u1 + e1, e1)] {
                if m > 0.0 && vis(a, b) {
                    p.rect_filled(band(a, b), 0.0, alpha(BAR, 0.3));
                }
            }
            if vis(t, u1) {
                let mut rect = band(t, u1);
                rect.set_right(rect.right().max(rect.left() + 2.0 * z));
                p.rect_filled(rect, 0.0, alpha(BAR, 0.88));
            } else {
                let (at, side, text) = if u1 < v0 { (track.left() + 6.0 * z, Align2::LEFT_CENTER, "<") } else { (track.right() - 6.0 * z, Align2::RIGHT_CENTER, ">") };
                p.text(Pos2::new(at, cy), side, text, mono(11.0), DIM);
            }
        }

        // The playhead.
        if let Some(u) = app.ph.utc.filter(|u| *u >= v0 && *u <= v1) {
            let x = tx(u);
            p.line_segment([Pos2::new(x, track.top()), Pos2::new(x, track.bottom())], Stroke::new(1.5 * z, PLAYHEAD));
        }

        // Wheel zooms, a drag pans, a click on the ruler moves the playhead.
        app.match_ui.tracks.push(track);
        let resp = ui.interact(track, egui::Id::new(("timeline-track", i)), Sense::click_and_drag());
        if let Some(h) = resp.hover_pos() {
            let scroll = ui.input(|k| k.smooth_scroll_delta.y + k.smooth_scroll_delta.x);
            if scroll != 0.0 {
                let f = (-scroll as f64 * 0.002).exp();
                let at = v0 + ((h.x - track.left()) / track.width()) as f64 * span;
                app.match_ui.views.insert(i, (at - (at - v0) * f, at + (v1 - at) * f));
            }
        }
        if resp.dragged() && !resp.drag_started() {
            let dt = (resp.drag_delta().x / track.width()) as f64 * span;
            app.match_ui.views.insert(i, (v0 - dt, v1 - dt));
        }
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                app.set_utc_playhead(v0 + ((pos.x - track.left()) / track.width()) as f64 * span);
            }
        }
        if (resp.double_clicked() || do_fit) && !always {
            app.match_ui.views.insert(i, fit);
        }
        if toggle_always {
            if always {
                app.match_ui.views.insert(i, (v0, v1));
            } else {
                app.match_ui.views.remove(&i);
            }
        }
        if play {
            app.ph.playing = !app.ph.playing;
        }
        let mut changed = false;
        if let Kind::Timeline { rows } = &mut app.nodes[i].kind {
            let set = |rows: &mut Vec<(String, String)>, k: &str, val: String| match rows.iter_mut().find(|(q, _)| q == k) {
                Some((_, x)) => *x = val,
                None => rows.push((k.to_string(), val)),
            };
            if toggle_sort {
                set(rows, "sort by time", (!by_time).to_string());
                changed = true;
            }
            if (new_rh - rh).abs() >= 0.5 {
                set(rows, "row height", format!("{new_rh:.0}"));
                changed = true;
            }
        }
        if changed {
            app.save_project();
        }
    }
}
