//! The Time node in the workbench: the rows of boxes on the canvas, what comes out, its part of the
//! result file. Compiled into the `corroborate` program.

use super::*;
use corroborate::data::Data;
use corroborate::module_manifest::{PortLabel, PortLook, Side};
use corroborate::modules::time::{self, BOXES};

const NODE_W: f32 = 560.0;
const TOP: f32 = HEAD + 4.0;
const NAMES_H: f32 = 16.0;
const ROW_H: f32 = 30.0;
/// The row's name ("input", "row 1", "Result") from the node's edge, then its sign.
const NAME_X: f32 = 6.0;
const NAME_W: f32 = 50.0;
const SIGN_W: f32 = 22.0;
/// The gap before the error margin column (the root sign's hook sits in it), the ± box, its ².
const ROOT_W: f32 = 28.0;
const PM_W: f32 = 64.0;
const SQ_W: f32 = 10.0;
/// The remove button, and the output's name on Result's row.
const RIGHT_W: f32 = 46.0;
const MSG_H: f32 = 18.0;
/// The widths of the boxes of a row, in shares, and the gaps between them with what they show:
/// the date (year-mo-day), a wider gap, then the clock (h:min:s.ms).
const WIDTHS: [f32; 7] = [50.0, 30.0, 30.0, 30.0, 30.0, 30.0, 42.0];
const GAPS: [(f32, &str); 6] = [(9.0, "-"), (9.0, "-"), (14.0, ""), (9.0, ":"), (9.0, ":"), (9.0, ".")];
/// The boxes' names over the columns.
const SHORT: [&str; 7] = ["year", "mo", "day", "h", "min", "s", "ms"];
/// How far under the row's middle a wire runs into its frame: under the name, never through it.
const LOW: f32 = 10.0;
const FAINT: Color32 = Color32::from_rgb(86, 92, 102);
const BOX_LOCKED: Color32 = Color32::from_rgb(32, 35, 40);
const ADD: Color32 = Color32::from_rgb(58, 63, 71);

fn row_y(place: usize) -> f32 {
    TOP + NAMES_H + ROW_H * (place as f32 + 0.5)
}

/// The rects of a row's boxes, centred on `y`, between `x0` and `x1`.
fn box_rects(x0: f32, x1: f32, y: f32, z: f32) -> [Rect; 7] {
    let gaps: f32 = GAPS.iter().map(|g| g.0).sum::<f32>() * z;
    let unit = (x1 - x0 - gaps) / WIDTHS.iter().sum::<f32>();
    let mut x = x0;
    let mut k = 0;
    WIDTHS.map(|w| {
        let r = Rect::from_min_size(Pos2::new(x, y - 11.0 * z), Vec2::new(w * unit, 22.0 * z));
        x += w * unit + GAPS.get(k).map_or(0.0, |g| g.0) * z;
        k += 1;
        r
    })
}

/// The separators between a row's boxes.
fn separators(p: &egui::Painter, rects: &[Rect; 7], z: f32) {
    for (k, (_, s)) in GAPS.iter().enumerate() {
        let at = Pos2::new((rects[k].right() + rects[k + 1].left()) * 0.5, rects[k].center().y);
        p.text(at, Align2::CENTER_CENTER, s, FontId::monospace(11.5 * z), FAINT);
    }
}

/// The frame round a row's boxes (and Result's ±): the colour of what feeds or leaves it.
fn frame(p: &egui::Painter, r: Rect, c: Color32, z: f32) -> Rect {
    let f = r.expand(3.5 * z);
    p.rect_stroke(f, 3.0 * z, Stroke::new(1.5 * z, c), egui::StrokeKind::Middle);
    f
}

/// A wired length: its seconds, and its ± (0 when it gives none).
fn length_of(d: &Data) -> Option<(f64, f64)> {
    let Data::Custom { json, .. } = d else { return None };
    let v = corroborate::plugin::parse_json(json)?;
    Some((v.get("seconds")?.num()?, v.get("error_s").and_then(|e| e.num()).unwrap_or(0.0)))
}

/// Time node `i` worked through: the result, every row's ± from the top (the input's first), what
/// each modification row did in words, and where the input came from.
pub(crate) struct Stacks {
    pub t: f64,
    pub errs: Vec<f64>,
    pub did: Vec<String>,
    pub from: String,
}

impl App {
    /// What is wired into the time input: the moment, where it comes from and its ±, or why
    /// there is none.
    pub(crate) fn time_in(&self, i: NodeId) -> Option<Result<(f64, String, f64), String>> {
        let (f, o) = self.graph.feeder(i, "time")?;
        Some(match self.value_from(f, o) {
            Ok(Data::Time { utc, source, error_s, .. }) => Ok((utc, source, error_s)),
            Ok(_) => Err("the wire gives no time".to_string()),
            Err(e) => Err(format!("the wired time: {e}")),
        })
    }

    /// The length wired into modification row `n`, with its ±, if one is.
    fn row_length(&self, i: NodeId, n: usize) -> Option<Result<(f64, f64), String>> {
        let (f, o) = self.graph.feeder(i, time::row_port(n))?;
        Some(self.value_from(f, o).and_then(|d| length_of(&d).ok_or("the wire gives no length".to_string())))
    }

    /// The date stack and the error stack of Time node `i`, from the top down.
    pub(crate) fn time_stacks(&self, i: NodeId) -> Result<Stacks, String> {
        let Kind::Time { rows } = &self.nodes[i].kind else { return Err("not a Time node".into()) };
        let (mut t, from, in_err) = match self.time_in(i) {
            Some(w) => w?,
            None => (time::typed(rows)?, "typed".to_string(), time::row_error(rows, "in", 0)?),
        };
        let mut errs = vec![in_err];
        let mut did = Vec::new();
        for (k, n) in time::numbers(rows).into_iter().enumerate() {
            let wired = self.row_length(i, n).transpose().map_err(|e| format!("Modification row {}: {e}", k + 1))?;
            t = time::apply(t, rows, n, wired.map(|w| w.0), k + 1)?;
            let e = match wired {
                Some((_, e)) => e,
                None => time::row_error(rows, &n.to_string(), k + 1)?,
            };
            errs.push(e);
            let len = if wired.is_some() { " (wired length)" } else { "" };
            let pm = if e > 0.0 { format!(" ±{} s", pm_text(e)) } else { String::new() };
            did.push(format!("{}{len}{pm}", time::row_words(rows, n, wired.map(|w| w.0))));
        }
        Ok(Stacks { t, errs, did, from })
    }

    /// What Time node `i` did to the time, in a line: its rows, and its ± from the error stack.
    pub(crate) fn time_did(&self, i: NodeId) -> String {
        match self.time_stacks(i) {
            Ok(s) => did_line(&s),
            Err(e) => e,
        }
    }

    /// The result of Time node `i`: the input with every row applied from the top down, and the
    /// root sum of squares of every row's ±.
    pub(crate) fn time_out(&self, i: NodeId) -> Result<Data, String> {
        let s = self.time_stacks(i)?;
        let error_s = time::root_sum_squares(&s.errs);
        let source = format!("Time node {} ({}), from {}", self.nodes[i].label, did_line(&s), s.from);
        Ok(Data::Time { utc: s.t, error_s, source, recording: String::new() })
    }
}

fn did_line(s: &Stacks) -> String {
    let rows = if s.did.is_empty() { "no rows".to_string() } else { s.did.join(", ") };
    format!("{rows}; ±{:.3} s, the root sum of squares of {} ±", time::root_sum_squares(&s.errs), s.errs.len())
}

/// A ± as a short figure: 0.02, 1, 0.005.
fn pm_text(e: f64) -> String {
    let s = format!("{e:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Its part of the result file: its rows, and what it gives.
pub fn json(app: &App, i: NodeId) -> String {
    let Kind::Time { rows } = &app.nodes[i].kind else { return String::new() };
    let r: Vec<String> = rows.iter().map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v))).collect();
    let out = match app.time_out(i) {
        Ok(d) => d.to_json(),
        Err(e) => format!("{{\"error\":\"{}\"}}", json_escape(&e)),
    };
    format!(",\"rows\":{{{}}},\"result\":{out}", r.join(","))
}

/// How a box looks: typed into, filled from a wire (grey, locked), or Result's (read-only).
#[derive(Clone, Copy, PartialEq)]
enum Shade {
    Typed,
    Wired,
    Result,
}

/// A box of a row. An empty box reads 0 and shows it (`hint`); a box at 0 is dimmed, so the real
/// figures stand out.
fn text_box(ui: &mut egui::Ui, r: Rect, v: &mut String, shade: Shade, hint: &str, salt: (NodeId, &str)) {
    let zero = v.trim().trim_start_matches('0').trim_start_matches('.').trim_start_matches('0').is_empty();
    let color = match shade {
        _ if zero => FAINT,
        Shade::Wired => DIM,
        _ => TEXT,
    };
    let mut t = egui::TextEdit::singleline(v).horizontal_align(egui::Align::Center).margin(egui::Margin::ZERO).hint_text(egui::RichText::new(hint).color(FAINT)).text_color(color).interactive(shade == Shade::Typed).id_salt(salt);
    if shade == Shade::Wired {
        t = t.background_color(BOX_LOCKED);
    }
    ui.put(r, t);
}

fn slot<'a>(rows: &'a mut Vec<(String, String)>, key: &str) -> &'a mut String {
    if !rows.iter().any(|(k, _)| k == key) {
        rows.push((key.to_string(), String::new()));
    }
    &mut rows.iter_mut().find(|(k, _)| k == key).unwrap().1
}

/// The node on the canvas.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, _standard: Vec2) -> Vec2 {
        let n = match &app.nodes[i].kind {
            Kind::Time { rows } => time::numbers(rows).len(),
            _ => 0,
        };
        Vec2::new(NODE_W, row_y(n + 2) + ROW_H * 0.5 + MSG_H + 8.0)
    }

    fn port_look(&self, app: &App, i: NodeId, name: &str) -> Option<PortLook> {
        let Kind::Time { rows } = &app.nodes[i].kind else { return None };
        let nums = time::numbers(rows);
        let at = |side: Side, place: usize| Some(PortLook { side: Some(side), at: Some(row_y(place)), label: Some(PortLabel::Hidden), ..Default::default() });
        match name {
            "time" => at(Side::Left, 0),
            "result" => at(Side::Right, nums.len() + 2),
            time::NEXT => at(Side::Left, nums.len() + 1),
            _ => {
                let n: usize = name.strip_prefix("row ")?.parse().ok()?;
                at(Side::Left, 1 + nums.iter().position(|m| *m == n)?)
            }
        }
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let body = ui.max_rect();
        let z = app.zoom;
        let node_top = body.top() - TOP * z;
        let y_of = |place: usize| node_top + row_y(place) * z;
        let p = ui.painter().with_clip_rect(ui.clip_rect());
        // The wires inside run from the node's edge, past the body's margin.
        let edge = (body.left() - 6.0 * z, body.right() + 6.0 * z);
        let wp = ui.painter().with_clip_rect(ui.clip_rect().expand2(Vec2::new(7.0 * z, 0.0)));
        let name_x = body.left() + NAME_X * z;
        let sign_x = name_x + NAME_W * z;
        let x0 = sign_x + SIGN_W * z + 6.0 * z;
        let rx = body.right() - (RIGHT_W + SQ_W + PM_W + ROOT_W) * z;
        let x1 = rx - 6.0 * z;
        let pm_x = rx + ROOT_W * z;
        let pm_rect = |y: f32| Rect::from_min_size(Pos2::new(pm_x, y - 11.0 * z), Vec2::new(PM_W * z, 22.0 * z));
        let right_x = pm_x + (PM_W + SQ_W) * z;
        let word = |s: f32| FontId::proportional(s * z);
        let time_c = kind_color(PortKind::Time);
        let length_c = kind_color(PortKind::Custom("length"));
        // A wire from the node's edge into a frame: it drops under the row's name, then runs in.
        let wire_in = |y: f32, to: f32, c: Color32| {
            let pts = vec![Pos2::new(edge.0, y), Pos2::new(edge.0 + 8.0 * z, y + LOW * z), Pos2::new(to, y + LOW * z)];
            wp.add(egui::Shape::line(pts, Stroke::new(1.6 * z, c)));
        };
        let Kind::Time { rows } = &app.nodes[i].kind else { return };
        let nums = time::numbers(rows);
        let incoming = app.time_in(i);
        let lengths: Vec<Option<Result<(f64, f64), String>>> = nums.iter().map(|&n| app.row_length(i, n)).collect();
        let stacks = app.time_stacks(i);
        let next = app.graph.feeder(i, time::NEXT);
        let mut remove = None;
        let mut add = false;

        let head = box_rects(x0, x1, body.top() + NAMES_H * 0.5 * z, z);
        for (r, name) in head.iter().zip(SHORT) {
            p.text(r.center(), Align2::CENTER_CENTER, name, FontId::monospace(10.0 * z), FAINT);
        }
        p.text(pm_rect(body.top() + NAMES_H * 0.5 * z).center(), Align2::CENTER_CENTER, "error margin ±", FontId::monospace(9.5 * z), FAINT);
        let Kind::Time { rows } = &mut app.nodes[i].kind else { return };
        let square = |r: Rect| {
            p.text(Pos2::new(r.right() + 1.5 * z, r.top() - 1.0 * z), Align2::LEFT_TOP, "2", word(8.5), DIM);
        };

        // The input row: wired, a frame in the time's colour round its locked boxes.
        let y = y_of(0);
        let rects = box_rects(x0, x1, y, z);
        let shown: Option<([String; 7], String)> = match &incoming {
            Some(Ok((t, _, e))) => Some((time::time_boxes(*t), pm_text(*e))),
            Some(Err(_)) => Some(Default::default()),
            None => None,
        };
        if shown.is_some() {
            let f = frame(&p, rects[0].union(rects[6]), time_c, z);
            wire_in(y, f.left(), time_c);
        }
        p.text(Pos2::new(name_x, y), Align2::LEFT_CENTER, "input", word(11.5), DIM);
        separators(&p, &rects, z);
        for (k, (r, name)) in rects.iter().zip(BOXES).enumerate() {
            match &shown {
                Some((s, _)) => text_box(ui, *r, &mut s[k].clone(), Shade::Wired, "", (i, name)),
                None => text_box(ui, *r, slot(rows, &format!("in.{name}")), Shade::Typed, "", (i, name)),
            }
        }
        match &shown {
            Some((_, e)) => text_box(ui, pm_rect(y), &mut e.clone(), Shade::Wired, "", (i, "in.error_s")),
            None => text_box(ui, pm_rect(y), slot(rows, "in.error_s"), Shade::Typed, "0", (i, "in.error_s")),
        }
        square(pm_rect(y));

        // The modification rows: a wired one has a frame in the length's colour, its boxes and ±
        // grey and locked; a typed one has no frame.
        for (place, (&n, len)) in nums.iter().zip(&lengths).enumerate() {
            let y = y_of(place + 1);
            let rects = box_rects(x0, x1, y, z);
            let wired = len.is_some();
            if wired {
                let f = frame(&p, rects[0].union(rects[6]), length_c, z);
                wire_in(y, f.left(), length_c);
            }
            p.text(Pos2::new(name_x, y), Align2::LEFT_CENTER, format!("row {}", place + 1), word(11.5), DIM);
            separators(&p, &rects, z);
            let (filled, err): ([String; 7], String) = match len {
                Some(Ok((l, e))) => (time::length_boxes(*l), pm_text(*e)),
                _ => Default::default(),
            };
            for (k, (r, name)) in rects.iter().zip(BOXES).enumerate() {
                let key = format!("{n}.{name}");
                if wired {
                    text_box(ui, *r, &mut filled[k].clone(), Shade::Wired, "", (n, name));
                } else {
                    text_box(ui, *r, slot(rows, &key), Shade::Typed, "0", (n, name));
                }
            }
            if wired {
                text_box(ui, pm_rect(y), &mut err.clone(), Shade::Wired, "", (n, "error_s"));
            } else {
                text_box(ui, pm_rect(y), slot(rows, &format!("{n}.error_s")), Shade::Typed, "0", (n, "error_s"));
            }
            square(pm_rect(y));
            // Its sign, wired or not: a click turns it over.
            let sign = slot(rows, &format!("{n}.sign"));
            let plus = *sign == "+";
            let sr = Rect::from_center_size(Pos2::new(sign_x + SIGN_W * 0.5 * z, y), Vec2::new(18.0 * z, 18.0 * z));
            let label = egui::RichText::new(if plus { "+" } else { "−" }).size(14.0 * z).color(TEXT);
            let hover = if plus { "added: click to take it away instead" } else { "taken away: click to add it instead" };
            if ui.put(sr, egui::Button::new(label).fill(Color32::from_rgb(48, 52, 59))).on_hover_text(hover).clicked() {
                *sign = if plus { "-" } else { "+" }.to_string();
            }
            let xr = Rect::from_min_size(Pos2::new(right_x + 4.0 * z, y - 11.0 * z), Vec2::new(22.0 * z, 22.0 * z));
            if ui.put(xr, egui::Button::new(egui::RichText::new("×").size(13.0 * z).color(FAINT)).frame(false)).on_hover_text("remove this row").clicked() {
                remove = Some(n);
            }
        }

        // The error stack, drawn as its formula: a root sign over every row's ±, its hook in the
        // gap before the column and its bar across the top. Result's ± sits below it, outside.
        let (top, low) = (y_of(0) - 14.0 * z, y_of(nums.len()) + 11.0 * z);
        let hx = pm_x - 16.0 * z;
        let ink = Stroke::new(1.5 * z, TEXT);
        let hook = vec![Pos2::new(hx, low - 9.0 * z), Pos2::new(hx + 3.5 * z, low - 11.0 * z), Pos2::new(hx + 8.0 * z, low), Pos2::new(hx + 13.0 * z, top), Pos2::new(right_x, top)];
        p.add(egui::Shape::line(hook, ink));
        let root = Rect::from_min_max(Pos2::new(hx, top - 3.0 * z), Pos2::new(pm_x, low));
        ui.interact(root, ui.id().with(("root", i)), egui::Sense::hover())
            .on_hover_text("root sum of squares: every row's ± squared, added up, and the root taken. A row's sign does not matter here: an uncertainty is never subtracted.");

        // The row that adds one: a grey strip as wide as the boxes; its port waits on the left.
        let y = y_of(nums.len() + 1);
        let r = Rect::from_min_max(Pos2::new(x0, y - 11.0 * z), Pos2::new(x1, y + 11.0 * z));
        p.rect_filled(r, 3.0 * z, ADD);
        if ui.put(r, egui::Button::new(egui::RichText::new("+ row").size(12.0 * z).color(TEXT)).frame(false)).on_hover_text("add a modification row (or wire a length into the port on the left)").clicked() {
            add = true;
        }

        // Result, read-only: the date stack's sum and the error stack's, in one frame, since the
        // output carries both. The output's line leaves the frame and runs under "time" to the port.
        let y = y_of(nums.len() + 2);
        let rects = box_rects(x0, x1, y, z);
        let out_c = if stacks.is_ok() { time_c } else { FAINT };
        let f = frame(&p, rects[0].union(pm_rect(y)), out_c, z);
        let out = vec![Pos2::new(f.right(), y + LOW * z), Pos2::new(edge.1 - 10.0 * z, y + LOW * z), Pos2::new(edge.1, y)];
        wp.add(egui::Shape::line(out.clone(), Stroke::new(6.0 * z, alpha(out_c, 0.12))));
        wp.add(egui::Shape::line(out, Stroke::new(2.0 * z, out_c)));
        p.text(Pos2::new(name_x, y), Align2::LEFT_CENTER, "Result", word(11.5), TEXT);
        separators(&p, &rects, z);
        let (res, err): ([String; 7], String) = match &stacks {
            Ok(s) => (time::time_boxes(s.t), format!("{:.3}", time::root_sum_squares(&s.errs))),
            _ => Default::default(),
        };
        for (k, (r, name)) in rects.iter().zip(BOXES).enumerate() {
            text_box(ui, *r, &mut res[k].clone(), Shade::Result, "", (i + 1_000_000, name));
        }
        text_box(ui, pm_rect(y), &mut err.clone(), Shade::Result, "", (i + 1_000_000, "error_s"));
        p.text(Pos2::new(body.right() - 8.0 * z, y), Align2::RIGHT_CENTER, "time", word(11.5), if stacks.is_ok() { TEXT } else { DIM });

        // What is wrong.
        if let Err(e) = &stacks {
            p.text(Pos2::new(name_x, y + (ROW_H * 0.5 + MSG_H * 0.5) * z), Align2::LEFT_CENTER, e, word(11.5), Color32::from_rgb(224, 82, 79));
        }

        // A length wired into the waiting port: a new row, and the wire moves onto its port.
        let mut grown = None;
        if add || next.is_some() {
            grown = Some(time::add_row(rows));
        }
        if let Some(n) = remove {
            time::remove_row(rows, n);
        }
        if grown.is_some() || remove.is_some() {
            let inputs = app.nodes[i].kind.spec().inputs;
            app.graph.set_inputs_keep(i, inputs);
            if let (Some((f, o)), Some(n)) = (next, grown) {
                app.graph.disconnect(i, time::NEXT);
                if let Err(e) = app.graph.connect(f, o, i, time::row_port(n)) {
                    app.msg = format!("could not move the wire onto row {n}: {e:?}");
                }
            }
            app.save_project();
        }
    }
}
