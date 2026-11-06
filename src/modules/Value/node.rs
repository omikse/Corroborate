//! The Value node in the workbench: a type picker and the type's fields edited on the canvas,
//! its status, its part of the result file, and what protocol-1 plugins wired to it are given.
//! Compiled into the `corroborate` program.

use super::*;
use corroborate::modules::value;
use corroborate::types::{Field, FieldType, Flow};

/// One line about the value, or what is wrong with it.
pub fn status(ty: &str, rows: &[(String, String)]) -> String {
    match value::data(ty, rows) {
        Ok(d) => bus::summary(&d),
        Err(e) => e,
    }
}

/// Its part of the result file: the type and the value (or the error).
pub fn json(ty: &str, rows: &[(String, String)]) -> String {
    match value::json(ty, rows) {
        Ok(j) => format!(",\"type\":\"{}\",\"value\":{j}", json_escape(ty)),
        Err(e) => format!(",\"type\":\"{}\",\"error\":\"{}\"", json_escape(ty), json_escape(&e)),
    }
}

impl App {
    /// What Value nodes wired into `id` hold, as `--manual name=value` arguments for a protocol-1
    /// plugin: every filled field, each name once; a latitude and longitude also as
    /// `coordinates=lat, lon`.
    pub(crate) fn value_args_into(&self, id: NodeId) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for inp in self.graph.connected_inputs(id) {
            let Some((from, _)) = self.graph.feeder(id, inp) else { continue };
            let Kind::Value { ty, rows } = &self.nodes[from].kind else { continue };
            if corroborate::types::lookup(ty).is_some_and(|t| t.flow == Flow::List) {
                continue;
            }
            let get = |k: &str| rows.iter().find(|(q, _)| q == k).map(|(_, v)| v.trim()).filter(|v| !v.is_empty());
            let mut add = |k: &str, v: String| {
                if !out.iter().any(|(q, _)| q == k) {
                    out.push((k.to_string(), v));
                }
            };
            if let (Some(lat), Some(lon)) = (get("lat"), get("lon")) {
                add("coordinates", format!("{lat}, {lon}"));
            }
            for (k, v) in rows.iter().filter(|(_, v)| !v.trim().is_empty()) {
                add(k, v.trim().to_string());
            }
        }
        out
    }

    /// Changes Value node `i`'s type (a logged setting): its fields start again and its output
    /// takes the new type; wires that no longer fit go.
    pub(crate) fn set_value_type(&mut self, i: NodeId, ty: &str) {
        if let Err(e) = self.set_setting(i, "type", ty) {
            self.msg = e;
        }
        let outputs = self.nodes[i].kind.spec().outputs;
        self.graph.set_outputs(i, outputs);
        self.sync_record_ports();
    }
}

/// Height of one field row on the canvas, unzoomed.
const ROW_H: f32 = 24.0;

/// One field's editor: its name, then a box, tick, list or file picker by its type.
fn field_ui(ui: &mut egui::Ui, f: &Field, v: &mut String, z: f32, w: f32) {
    ui.horizontal(|ui| {
        ui.add_sized([86.0 * z, 18.0 * z], egui::Label::new(egui::RichText::new(&f.label).small().color(DIM)).truncate());
        let w = (w - 92.0 * z).max(40.0 * z);
        match &f.ty {
            FieldType::Bool => {
                let mut b = v == "true";
                if ui.checkbox(&mut b, "").changed() {
                    *v = b.to_string();
                }
            }
            FieldType::Choice(c) => {
                egui::ComboBox::from_id_salt(ui.next_auto_id()).selected_text(v.as_str()).width(w).show_ui(ui, |ui| {
                    for x in c {
                        ui.selectable_value(v, x.clone(), x);
                    }
                });
            }
            FieldType::File => {
                ui.add(egui::TextEdit::singleline(v).hint_text("path").desired_width(w - 26.0 * z));
                if ui.small_button("…").on_hover_text("pick a file").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_file() {
                        *v = p.to_string_lossy().into_owned();
                    }
                }
            }
            ty => {
                let hint = match ty {
                    FieldType::Datetime => "2026-11-04 20:06:39 (UTC)".to_string(),
                    FieldType::Number { min: Some(a), max: Some(b) } => format!("{a} to {b}"),
                    FieldType::Number { min: Some(a), max: None } => format!("from {a}"),
                    _ if f.required => "needed".into(),
                    _ => String::new(),
                };
                let ok = f.check(v).is_ok();
                let unit_w = if f.unit.is_empty() { 0.0 } else { 22.0 * z };
                let mut t = egui::TextEdit::singleline(v).hint_text(hint).desired_width(w - unit_w);
                if !ok {
                    t = t.text_color(WARN);
                }
                ui.add(t).on_hover_text(f.check(v).err().unwrap_or_default());
                if !f.unit.is_empty() {
                    ui.colored_label(DIM, egui::RichText::new(&f.unit).small());
                }
            }
        }
    });
}

/// The Value node on the canvas: which type, then its fields (a list type: its items, each with
/// 🗙, and "+ item").
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, standard: Vec2) -> Vec2 {
        let Kind::Value { ty, rows } = &app.nodes[i].kind else { return standard };
        let list = corroborate::types::lookup(ty).is_some_and(|t| t.flow == Flow::List);
        let n = rows.len() + if list { value::items(rows).len() + 1 } else { 0 };
        Vec2::new(330.0, standard.y.max(HEAD + 8.0 + (n + 1) as f32 * ROW_H + 26.0))
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let z = app.zoom;
        let Kind::Value { ty, rows } = &mut app.nodes[i].kind else { return };
        let ty_now = ty.clone();
        let t = corroborate::types::lookup(&ty_now);
        let mut pick: Option<&'static str> = None;
        crate::look::area(ui, r, |ui| {
            let label = t.as_ref().map_or(ty_now.clone(), |t| t.label.clone());
            egui::ComboBox::from_id_salt(("value-type", i)).selected_text(label).width(r.width() - 8.0 * z).show_ui(ui, |ui| {
                for x in value::types() {
                    ui.horizontal(|ui| {
                        crate::types_menu::type_dot(ui, &x);
                        if ui.selectable_label(x.id == ty_now, &x.label).on_hover_text(&x.description).clicked() {
                            pick = Some(x.id);
                        }
                    });
                }
            });
            let Some(t) = &t else { return };
            let w = r.width() - 8.0 * z;
            if t.flow != Flow::List {
                for f in &t.fields {
                    if !rows.iter().any(|(k, _)| *k == f.name) {
                        rows.push((f.name.clone(), f.default.clone()));
                    }
                    let v = &mut rows.iter_mut().find(|(k, _)| *k == f.name).unwrap().1;
                    field_ui(ui, f, v, z, w);
                }
                return;
            }
            let mut remove = None;
            for (n, _) in value::items(rows) {
                ui.horizontal(|ui| {
                    ui.colored_label(DIM, egui::RichText::new(format!("item {n}")).small());
                    if ui.small_button("🗙").on_hover_text("remove this item").clicked() {
                        remove = Some(n);
                    }
                });
                for f in &t.fields {
                    let key = format!("{n}.{}", f.name);
                    if !rows.iter().any(|(k, _)| *k == key) {
                        rows.push((key.clone(), f.default.clone()));
                    }
                    let v = &mut rows.iter_mut().find(|(k, _)| *k == key).unwrap().1;
                    field_ui(ui, f, v, z, w);
                }
            }
            if let Some(n) = remove {
                let p = format!("{n}.");
                rows.retain(|(k, _)| !k.starts_with(&p));
            }
            if ui.small_button("+ item").clicked() {
                let next = value::items(rows).iter().map(|(n, _)| n + 1).max().unwrap_or(1);
                rows.extend(value::item_rows(t, next));
            }
        });
        if let Some(id) = pick.filter(|id| *id != ty_now) {
            app.set_value_type(i, id);
        }
    }
}
