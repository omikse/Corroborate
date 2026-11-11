//! The Time node in the workbench: the rows of boxes on the canvas, what comes out, its part of the
//! result file. Compiled into the `corroborate` program.

use super::*;
use corroborate::data::Data;
use corroborate::module_manifest::{PortLabel, PortLook, Side};
use corroborate::modules::time::{self, BOXES};

const NODE_W: f32 = 600.0;
const TOP: f32 = HEAD + 4.0;
const NAMES_H: f32 = 16.0;
const ROW_H: f32 = 30.0;
const PORT_W: f32 = 12.0;
/// The row's name ("input", "row 1", "Result"), then its sign.
const NAME_W: f32 = 50.0;
const SIGN_W: f32 = 22.0;
/// The root sign's hook, then the ± box, then its ².
const ROOT_W: f32 = 16.0;
const PM_W: f32 = 58.0;
const SQ_W: f32 = 10.0;
/// The remove button, and the output's name on Result's row.
const RIGHT_W: f32 = 52.0;
const MSG_H: f32 = 18.0;
/// The widths of the boxes of a row, in shares.
const WIDTHS: [f32; 7] = [1.5, 1.0, 1.0, 1.0, 1.0, 1.0, 1.2];

fn row_y(place: usize) -> f32 {
    TOP + NAMES_H + ROW_H * (place as f32 + 0.5)
}

/// The rects of a row's boxes, centred on `y`, between `x0` and `x1`.
fn box_rects(x0: f32, x1: f32, y: f32, z: f32) -> [Rect; 7] {
    let gap = 4.0 * z;
    let unit = (x1 - x0 - gap * 6.0) / WIDTHS.iter().sum::<f32>();
    let mut x = x0;
    WIDTHS.map(|w| {
        let r = Rect::from_min_size(Pos2::new(x, y - 11.0 * z), Vec2::new(w * unit, 22.0 * z));
        x += w * unit + gap;
        r
    })
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
            Ok(Data::Time { utc: Some(u), source, error_s, .. }) => Ok((u, source, error_s)),
            Ok(_) => Err("the wired time has no date".to_string()),
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
        Ok(Data::Time { clock: corroborate::clock::UTC.to_string(), t: s.t, utc: Some(s.t), error_s, source })
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

/// A box of a row. An empty box reads 0 and shows it (`hint`).
fn text_box(ui: &mut egui::Ui, r: Rect, v: &mut String, locked: bool, hint: &str, salt: (NodeId, &str)) {
    let t = egui::TextEdit::singleline(v).horizontal_align(egui::Align::Center).margin(egui::Margin::ZERO).hint_text(egui::RichText::new(hint).color(alpha(DIM, 0.7))).interactive(!locked).id_salt(salt);
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
        let n = app.graph.nodes()[i].spec.inputs.len().saturating_sub(1);
        Vec2::new(NODE_W, row_y(n + 2) + ROW_H * 0.5 + MSG_H + 8.0)
    }

    fn port_look(&self, app: &App, i: NodeId, name: &str) -> Option<PortLook> {
        let Kind::Time { rows } = &app.nodes[i].kind else { return None };
        let nums = time::numbers(rows);
        let at = |side: Side, place: usize| Some(PortLook { side: Some(side), at: Some(row_y(place)), label: Some(PortLabel::Hidden), ..Default::default() });
        match name {
            "time" => at(Side::Left, 0),
            "result" => at(Side::Right, nums.len() + 2),
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
        let name_x = body.left() + PORT_W * z;
        let sign_x = name_x + NAME_W * z;
        let x0 = sign_x + SIGN_W * z;
        let rx = body.right() - (RIGHT_W + SQ_W + PM_W + ROOT_W) * z;
        let x1 = rx - 6.0 * z;
        let pm_x = rx + ROOT_W * z;
        let pm_rect = |y: f32| Rect::from_min_size(Pos2::new(pm_x, y - 11.0 * z), Vec2::new(PM_W * z, 22.0 * z));
        let right_x = pm_x + (PM_W + SQ_W) * z;
        let word = |s: f32| FontId::proportional(s * z);
        let Kind::Time { rows } = &app.nodes[i].kind else { return };
        let nums = time::numbers(rows);
        let incoming = app.time_in(i);
        let lengths: Vec<Option<Result<(f64, f64), String>>> = nums.iter().map(|&n| app.row_length(i, n)).collect();
        let stacks = app.time_stacks(i);
        let mut remove = None;
        let mut add = false;

        for (r, name) in box_rects(x0, x1, body.top() + NAMES_H * 0.5 * z, z).iter().zip(BOXES) {
            p.text(r.center(), Align2::CENTER_CENTER, name, word(10.5), DIM);
        }
        p.text(pm_rect(body.top() + NAMES_H * 0.5 * z).center(), Align2::CENTER_CENTER, "± s", word(10.5), DIM);
        let Kind::Time { rows } = &mut app.nodes[i].kind else { return };
        let square = |r: Rect| {
            p.text(Pos2::new(r.right() + 1.5 * z, r.top() - 1.0 * z), Align2::LEFT_TOP, "2", word(8.5), DIM);
        };

        // The input row.
        let y = y_of(0);
        p.text(Pos2::new(name_x, y), Align2::LEFT_CENTER, "input", word(11.5), DIM);
        let shown: Option<([String; 7], String)> = match &incoming {
            Some(Ok((t, _, e))) => Some((time::time_boxes(*t), pm_text(*e))),
            Some(Err(_)) => Some(Default::default()),
            None => None,
        };
        for (k, (r, name)) in box_rects(x0, x1, y, z).iter().zip(BOXES).enumerate() {
            match &shown {
                Some((s, _)) => text_box(ui, *r, &mut s[k].clone(), true, "", (i, name)),
                None => text_box(ui, *r, slot(rows, &format!("in.{name}")), false, "", (i, name)),
            }
        }
        match &shown {
            Some((_, e)) => text_box(ui, pm_rect(y), &mut e.clone(), true, "", (i, "in.error_s")),
            None => text_box(ui, pm_rect(y), slot(rows, "in.error_s"), false, "0", (i, "in.error_s")),
        }
        square(pm_rect(y));

        // The modification rows.
        for (place, (&n, len)) in nums.iter().zip(&lengths).enumerate() {
            let y = y_of(place + 1);
            let wired = len.is_some();
            p.text(Pos2::new(name_x, y), Align2::LEFT_CENTER, format!("row {}", place + 1), word(11.5), DIM);
            let (filled, err): ([String; 7], String) = match len {
                Some(Ok((l, e))) => (time::length_boxes(*l), pm_text(*e)),
                _ => Default::default(),
            };
            for (k, (r, name)) in box_rects(x0, x1, y, z).iter().zip(BOXES).enumerate() {
                let key = format!("{n}.{name}");
                if wired {
                    text_box(ui, *r, &mut filled[k].clone(), true, "", (n, name));
                } else {
                    text_box(ui, *r, slot(rows, &key), false, "0", (n, name));
                }
            }
            if wired {
                text_box(ui, pm_rect(y), &mut err.clone(), true, "", (n, "error_s"));
            } else {
                text_box(ui, pm_rect(y), slot(rows, &format!("{n}.error_s")), false, "0", (n, "error_s"));
            }
            square(pm_rect(y));
            // Its sign, on its left: a click turns it over.
            let sign = slot(rows, &format!("{n}.sign"));
            let plus = *sign == "+";
            let sr = Rect::from_center_size(Pos2::new(sign_x + (SIGN_W * 0.5 - 2.0) * z, y), Vec2::new(18.0 * z, 22.0 * z));
            let label = egui::RichText::new(if plus { "+" } else { "−" }).size(15.0 * z).color(TEXT);
            let hover = if plus { "added: click to take it away instead" } else { "taken away: click to add it instead" };
            if ui.put(sr, egui::Button::new(label).fill(Color32::from_rgb(58, 63, 71))).on_hover_text(hover).clicked() {
                *sign = if plus { "-" } else { "+" }.to_string();
            }
            let xr = Rect::from_min_size(Pos2::new(right_x + 4.0 * z, y - 11.0 * z), Vec2::new(22.0 * z, 22.0 * z));
            if ui.put(xr, egui::Button::new(egui::RichText::new("×").size(13.0 * z).color(DIM)).fill(Color32::from_rgb(58, 63, 71))).on_hover_text("remove this row").clicked() {
                remove = Some(n);
            }
        }

        // The error stack, drawn as its formula: a root sign over every row's ±, its hook beside
        // the last row and its bar across the top. Result's ± sits below it, outside.
        let (top, low) = (y_of(0) - 14.0 * z, y_of(nums.len()) + 11.0 * z);
        let ink = Stroke::new(1.6 * z, TEXT);
        let hook = vec![Pos2::new(rx, low - 9.0 * z), Pos2::new(rx + 3.5 * z, low - 11.0 * z), Pos2::new(rx + 8.0 * z, low), Pos2::new(rx + 13.0 * z, top), Pos2::new(right_x, top)];
        p.add(egui::Shape::line(hook, ink));
        let root = Rect::from_min_max(Pos2::new(rx, top - 3.0 * z), Pos2::new(pm_x, low));
        ui.interact(root, ui.id().with(("root", i)), egui::Sense::hover())
            .on_hover_text("root sum of squares: every row's ± squared, added up, and the root taken. A row's sign does not matter here: an uncertainty is never subtracted.");

        // The shaded row that adds one.
        let y = y_of(nums.len() + 1);
        let r = Rect::from_min_max(Pos2::new(x0, y - 11.0 * z), Pos2::new(x1, y + 11.0 * z));
        let mut x = r.left();
        while x < r.right() {
            p.rect_filled(Rect::from_min_max(Pos2::new(x, r.top()), Pos2::new((x + 5.0 * z).min(r.right()), r.bottom())), 0.0, Color32::from_rgb(58, 63, 71));
            x += 10.0 * z;
        }
        if ui.put(r, egui::Button::new(egui::RichText::new("+ row").size(13.0 * z)).fill(Color32::TRANSPARENT)).on_hover_text("add a modification row").clicked() {
            add = true;
        }

        // Result, read-only: the date stack's sum, and the error stack's below the root.
        let y = y_of(nums.len() + 2);
        p.line_segment([Pos2::new(body.left(), y - 15.0 * z), Pos2::new(body.right(), y - 15.0 * z)], Stroke::new(z, LINE));
        p.text(Pos2::new(name_x, y), Align2::LEFT_CENTER, "Result", word(11.5), TEXT);
        let (res, err): ([String; 7], String) = match &stacks {
            Ok(s) => (time::time_boxes(s.t), format!("{:.3}", time::root_sum_squares(&s.errs))),
            _ => Default::default(),
        };
        for (k, (r, name)) in box_rects(x0, x1, y, z).iter().zip(BOXES).enumerate() {
            text_box(ui, *r, &mut res[k].clone(), true, "", (i + 1_000_000, name));
        }
        text_box(ui, pm_rect(y), &mut err.clone(), true, "", (i + 1_000_000, "error_s"));
        p.text(Pos2::new(body.right() - 6.0 * z, y), Align2::RIGHT_CENTER, "time", word(11.5), if stacks.is_ok() { TEXT } else { DIM });

        // What is wrong.
        if let Err(e) = &stacks {
            p.text(Pos2::new(name_x, y + (ROW_H * 0.5 + MSG_H * 0.5) * z), Align2::LEFT_CENTER, e, word(11.5), Color32::from_rgb(224, 82, 79));
        }

        if add {
            time::add_row(rows);
        }
        if let Some(n) = remove {
            time::remove_row(rows, n);
        }
        if add || remove.is_some() {
            let inputs = app.nodes[i].kind.spec().inputs;
            app.graph.set_inputs_keep(i, inputs);
            app.save_project();
        }
    }
}
