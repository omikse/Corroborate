//! The Combine locations node in the workbench: one row per wired location with its sign (a
//! click turns it over), the mode for the + rows, and the area it gives. Compiled into the
//! `corroborate` program.

use super::*;
use corroborate::module_manifest::{PortLabel, PortLook, Side};
use corroborate::modules::combine_locations as combine;

const NODE_W: f32 = 340.0;
const TOP: f32 = HEAD + 4.0;
/// The line with the mode, then one row per input.
const MODE_H: f32 = 28.0;
const ROW_H: f32 = 26.0;
const NAME_X: f32 = 6.0;
const SIGN_X: f32 = 78.0;
const FAINT: Color32 = Color32::from_rgb(86, 92, 102);

fn row_y(place: usize) -> f32 {
    TOP + MODE_H + ROW_H * (place as f32 + 0.5)
}

impl App {
    /// The wired locations, each with whether it is taken away: (node it comes from, its area or
    /// why it has none, minus).
    fn combine_rows(&self, i: NodeId) -> Vec<(NodeId, Result<Arc<Area>, String>, bool)> {
        let Kind::CombineLocations { rows, .. } = &self.nodes[i].kind else { return Vec::new() };
        self.graph.nodes()[i]
            .spec
            .inputs
            .iter()
            .filter_map(|p| self.graph.feeder(i, p.name))
            .map(|(f, _)| (f, self.area_from(f).map(|(a, _)| a), combine::is_minus(rows, &self.nodes[f].uid)))
            .collect()
    }

    /// The + rows joined (all: AND, any: OR), then every − row taken away.
    pub(crate) fn combine_area_out(&self, i: NodeId, any: bool) -> Result<(Arc<Area>, String), String> {
        let rows = self.combine_rows(i);
        let have: Vec<(&Area, bool, String)> = rows.iter().filter_map(|(f, a, minus)| a.as_ref().ok().map(|a| (&**a, *minus, self.nodes[*f].label.clone()))).collect();
        if have.is_empty() {
            return Err("no location with an area wired in yet".into());
        }
        let plus: Vec<&WorldMask> = have.iter().filter(|h| !h.1).map(|h| &h.0.cells).collect();
        let minus: Vec<&WorldMask> = have.iter().filter(|h| h.1).map(|h| &h.0.cells).collect();
        let m = combine::combine(any, &plus, &minus);
        let names = |minus: bool| have.iter().filter(|h| h.1 == minus).map(|h| h.2.as_str()).collect::<Vec<_>>().join(", ");
        let how = match (plus.is_empty(), minus.is_empty()) {
            (true, _) => format!("the whole world without {}", names(true)),
            (false, true) => format!("{} of {}", combine::mode(any), names(false)),
            (false, false) => format!("{} of {}, without {}", combine::mode(any), names(false), names(true)),
        };
        let note = if m.is_empty() { format!("CONTRADICTION: nothing is left of {how}") } else { how };
        Ok((Arc::new(Area::new(m)), note))
    }
}

/// The Combine node on the canvas: the mode, then a row per input with its sign.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, _standard: Vec2) -> Vec2 {
        let n = app.graph.nodes()[i].spec.inputs.len().max(1);
        Vec2::new(NODE_W, row_y(n - 1) + ROW_H * 0.5 + 26.0)
    }

    fn port_look(&self, app: &App, i: NodeId, name: &str) -> Option<PortLook> {
        let place = app.graph.nodes()[i].spec.inputs.iter().position(|p| p.name == name)?;
        Some(PortLook { side: Some(Side::Left), at: Some(row_y(place)), label: Some(PortLabel::Hidden), ..Default::default() })
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let body = ui.max_rect();
        let z = app.zoom;
        let node_top = body.top() - TOP * z;
        let p = ui.painter().with_clip_rect(ui.clip_rect());
        let word = |s: f32| FontId::proportional(s * z);
        let Kind::CombineLocations { any, .. } = app.nodes[i].kind else { return };

        // The mode: how the + rows join.
        let mid = body.top() + MODE_H * 0.5 * z;
        p.text(Pos2::new(body.left() + NAME_X * z, mid), Align2::LEFT_CENTER, "+ rows", word(11.5), DIM);
        app.setting_widget(ui, i, "mode", Rect::from_min_size(Pos2::new(body.left() + SIGN_X * z, mid - 9.0 * z), Vec2::new(64.0 * z, 18.0 * z)), 2);
        let what = if any { "any (OR), then − taken away" } else { "all (AND), then − taken away" };
        p.text(Pos2::new(body.left() + (SIGN_X + 74.0) * z, mid), Align2::LEFT_CENTER, what, word(10.5), FAINT);

        // A row per input: its name, its sign, where it comes from and how much of Earth it keeps.
        let names: Vec<&'static str> = app.graph.nodes()[i].spec.inputs.iter().map(|p| p.name).collect();
        let rows = app.combine_rows(i);
        let mut flip = None;
        let mut fed = rows.iter();
        for (place, name) in names.iter().enumerate() {
            let y = node_top + row_y(place) * z;
            p.text(Pos2::new(body.left() + NAME_X * z, y), Align2::LEFT_CENTER, *name, word(11.0), DIM);
            let Some(_) = app.graph.feeder(i, name) else {
                p.text(Pos2::new(body.left() + SIGN_X * z, y), Align2::LEFT_CENTER, "wire a location in", word(10.5), FAINT);
                continue;
            };
            let Some((f, area, minus)) = fed.next() else { continue };
            let sr = Rect::from_center_size(Pos2::new(body.left() + (SIGN_X + 9.0) * z, y), Vec2::new(18.0 * z, 18.0 * z));
            let label = egui::RichText::new(if *minus { "−" } else { "+" }).size(14.0 * z).color(TEXT);
            let hover = if *minus { "taken away: click to add it instead" } else { "added: click to take it away instead" };
            if ui.put(sr, egui::Button::new(label).fill(Color32::from_rgb(48, 52, 59))).on_hover_text(hover).clicked() {
                flip = Some(app.nodes[*f].uid.clone());
            }
            let (text, col) = match area {
                Ok(a) => (format!("{} · {:.1} % of Earth", app.nodes[*f].label, a.cells.fraction() * 100.0), TEXT),
                Err(e) => (format!("{} · {e}", app.nodes[*f].label), WARN),
            };
            let tr = Rect::from_min_max(Pos2::new(sr.right() + 8.0 * z, y - 9.0 * z), Pos2::new(body.right(), y + 9.0 * z));
            p.with_clip_rect(tr.intersect(ui.clip_rect())).text(Pos2::new(tr.left(), y), Align2::LEFT_CENTER, text, word(11.0), col);
        }

        // Signs of nodes no longer wired in are forgotten.
        let wired: Vec<String> = rows.iter().map(|(f, _, _)| app.nodes[*f].uid.clone()).collect();
        if let Kind::CombineLocations { rows, .. } = &mut app.nodes[i].kind {
            rows.retain(|(k, _)| wired.iter().any(|u| *k == combine::minus_key(u)));
            if let Some(uid) = flip {
                combine::flip(rows, &uid);
            }
        }
    }
}
