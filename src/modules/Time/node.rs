//! The Time node in the workbench: the rows of boxes on the canvas, what comes out, its part of the
//! result file. Compiled into the `corroborate` program.

use super::*;
use corroborate::data::Data;
use corroborate::module_manifest::{PortLabel, PortLook, Side};
use corroborate::modules::time::{self, BOXES};

const NODE_W: f32 = 500.0;
const TOP: f32 = HEAD + 4.0;
const NAMES_H: f32 = 16.0;
const ROW_H: f32 = 30.0;
const PORT_W: f32 = 12.0;
const RIGHT_W: f32 = 84.0;
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

fn seconds_of(d: &Data) -> Option<f64> {
    let Data::Custom { json, .. } = d else { return None };
    corroborate::plugin::parse_json(json)?.get("seconds")?.num()
}

impl App {
    /// What is wired into the time input: the moment and where it comes from, or why there is none.
    fn time_in(&self, i: NodeId) -> Option<Result<(f64, String), String>> {
        let (f, o) = self.graph.feeder(i, "time")?;
        Some(match self.value_from(f, o) {
            Ok(Data::Time { utc: Some(u), source, .. }) => Ok((u, source)),
            Ok(_) => Err("the wired time has no date".to_string()),
            Err(e) => Err(format!("the wired time: {e}")),
        })
    }

    /// The length wired into modification row `n`, if one is.
    fn row_length(&self, i: NodeId, n: usize) -> Option<Result<f64, String>> {
        let (f, o) = self.graph.feeder(i, time::row_port(n))?;
        Some(self.value_from(f, o).and_then(|d| seconds_of(&d).ok_or("the wire gives no length".to_string())))
    }

    /// The result of Time node `i`: the input with every row applied from the top down.
    pub(crate) fn time_out(&self, i: NodeId) -> Result<Data, String> {
        let Kind::Time { rows } = &self.nodes[i].kind else { return Err("not a Time node".into()) };
        let (mut t, from) = match self.time_in(i) {
            Some(w) => w?,
            None => (time::typed(rows)?, "typed".to_string()),
        };
        for (k, n) in time::numbers(rows).into_iter().enumerate() {
            let wired = self.row_length(i, n).transpose().map_err(|e| format!("Modification row {}: {e}", k + 1))?;
            t = time::apply(t, rows, n, wired, k + 1)?;
        }
        let error_s = time::error_s(rows)?;
        Ok(Data::Time { clock: corroborate::clock::UTC.to_string(), t, utc: Some(t), error_s, source: format!("Time node {}, from {from}", self.nodes[i].label) })
    }
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

/// A box of a row.
fn text_box(ui: &mut egui::Ui, r: Rect, v: &mut String, locked: bool, hint: &str, salt: (NodeId, &str)) {
    let t = egui::TextEdit::singleline(v).horizontal_align(egui::Align::Center).margin(egui::Margin::ZERO).hint_text(egui::RichText::new(hint).color(alpha(DIM, 0.4))).interactive(!locked).id_salt(salt);
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
        Vec2::new(NODE_W, row_y(n + 3) + ROW_H * 0.5 + MSG_H + 8.0)
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
        let (x0, x1) = (body.left() + PORT_W * z, body.right() - RIGHT_W * z);
        let word = |s: f32| FontId::proportional(s * z);
        let Kind::Time { rows } = &app.nodes[i].kind else { return };
        let nums = time::numbers(rows);
        let incoming = app.time_in(i);
        let lengths: Vec<Option<Result<f64, String>>> = nums.iter().map(|&n| app.row_length(i, n)).collect();
        let out = app.time_out(i);
        let mut remove = None;
        let mut add = false;

        for (r, name) in box_rects(x0, x1, body.top() + NAMES_H * 0.5 * z, z).iter().zip(BOXES) {
            p.text(r.center(), Align2::CENTER_CENTER, name, word(10.5), DIM);
        }
        let Kind::Time { rows } = &mut app.nodes[i].kind else { return };

        // The input row.
        let y = y_of(0);
        let shown: Option<[String; 7]> = match &incoming {
            Some(Ok((t, _))) => Some(time::time_boxes(*t)),
            Some(Err(_)) => Some(Default::default()),
            None => None,
        };
        for (k, (r, name)) in box_rects(x0, x1, y, z).iter().zip(BOXES).enumerate() {
            match &shown {
                Some(s) => text_box(ui, *r, &mut s[k].clone(), true, "", (i, name)),
                None => text_box(ui, *r, slot(rows, &format!("in.{name}")), false, "", (i, name)),
            }
        }
        p.text(Pos2::new(body.right() - 6.0 * z, y), Align2::RIGHT_CENTER, "time", word(11.5), DIM);

        // The modification rows.
        for (place, (&n, len)) in nums.iter().zip(&lengths).enumerate() {
            let y = y_of(place + 1);
            let wired = len.is_some();
            let filled = match len {
                Some(Ok(l)) => time::length_boxes(*l),
                _ => Default::default(),
            };
            for (k, (r, name)) in box_rects(x0, x1, y, z).iter().zip(BOXES).enumerate() {
                let key = format!("{n}.{name}");
                if wired {
                    text_box(ui, *r, &mut filled[k].clone(), true, "", (n, name));
                } else {
                    text_box(ui, *r, slot(rows, &key), false, "", (n, name));
                }
            }
            let sign = slot(rows, &format!("{n}.sign"));
            let bw = 24.0 * z;
            let bx = |k: f32| Rect::from_min_size(Pos2::new(x1 + 6.0 * z + k * (bw + 2.0 * z), y - 11.0 * z), Vec2::new(bw, 22.0 * z));
            let on = Color32::from_rgb(125, 180, 230);
            let off = Color32::from_rgb(58, 63, 71);
            let text = |t: &str, picked: bool| egui::RichText::new(t.to_string()).size(13.0 * z).color(if picked { Color32::from_rgb(16, 18, 20) } else { DIM });
            if ui.put(bx(0.0), egui::Button::new(text("-", *sign != "+")).fill(if *sign != "+" { on } else { off })).on_hover_text("take this row away").clicked() {
                *sign = "-".to_string();
            }
            if ui.put(bx(1.0), egui::Button::new(text("+", *sign == "+")).fill(if *sign == "+" { on } else { off })).on_hover_text("add this row").clicked() {
                *sign = "+".to_string();
            }
            if ui.put(bx(2.0), egui::Button::new(text("×", false)).fill(off)).on_hover_text("remove this row").clicked() {
                remove = Some(n);
            }
        }

        // The shaded row that adds one.
        let y = y_of(nums.len() + 1);
        let r = Rect::from_min_max(Pos2::new(x0, y - 11.0 * z), Pos2::new(body.right() - 2.0 * z, y + 11.0 * z));
        let mut x = r.left();
        while x < r.right() {
            p.rect_filled(Rect::from_min_max(Pos2::new(x, r.top()), Pos2::new((x + 5.0 * z).min(r.right()), r.bottom())), 0.0, Color32::from_rgb(58, 63, 71));
            x += 10.0 * z;
        }
        if ui.put(r, egui::Button::new(egui::RichText::new("+").size(14.0 * z)).fill(Color32::TRANSPARENT)).on_hover_text("add a modification row").clicked() {
            add = true;
        }

        // Result, read-only.
        let y = y_of(nums.len() + 2);
        p.line_segment([Pos2::new(body.left(), y - 15.0 * z), Pos2::new(body.right(), y - 15.0 * z)], Stroke::new(z, LINE));
        let res: [String; 7] = match &out {
            Ok(Data::Time { utc: Some(u), .. }) => time::time_boxes(*u),
            _ => Default::default(),
        };
        for (k, (r, name)) in box_rects(x0, x1, y, z).iter().zip(BOXES).enumerate() {
            text_box(ui, *r, &mut res[k].clone(), true, "", (i + 1_000_000, name));
        }
        p.text(Pos2::new(body.right() - 6.0 * z, y), Align2::RIGHT_CENTER, "time", word(11.5), if out.is_ok() { TEXT } else { DIM });

        // The error margin, and what is wrong.
        let y = y_of(nums.len() + 3);
        p.text(Pos2::new(x0, y), Align2::LEFT_CENTER, "error margin ± (s)", word(11.5), DIM);
        text_box(ui, Rect::from_min_size(Pos2::new(x0 + 120.0 * z, y - 11.0 * z), Vec2::new(90.0 * z, 22.0 * z)), slot(rows, "error_s"), false, "", (i, "error_s"));
        if let Err(e) = &out {
            p.text(Pos2::new(x0, y + (ROW_H * 0.5 + MSG_H * 0.5) * z), Align2::LEFT_CENTER, e, word(11.5), Color32::from_rgb(224, 82, 79));
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
