//! The Types menu in the top bar: every data type (src/types.rs) with its dot, label and
//! description. Hovering one lists the modules that give and take it; clicking one adds a Value
//! node of it.

use super::*;
use corroborate::module_manifest::{kind_id, DotShape, PortLook, Side};
use corroborate::types::{Flow, TypeDef};

/// The dot of a type's flow: value circle, list square, stream diamond.
pub(crate) fn flow_shape(f: Flow) -> DotShape {
    match f {
        Flow::Value => DotShape::Circle,
        Flow::List => DotShape::Square,
        Flow::Stream => DotShape::Diamond,
    }
}

/// A type's dot, open, as it is drawn on outputs.
pub(crate) fn type_dot(ui: &mut egui::Ui, t: &TypeDef) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    let [red, g, b] = t.color;
    layout::draw_port(ui.painter(), r.center(), Side::Right, &PortLook::default(), Color32::from_rgb(red, g, b), flow_shape(t.flow), false, "", TEXT, 1.0);
}

/// Which modules give (outputs) and take (inputs) type `id`, by label.
pub(crate) fn users(id: &str) -> (Vec<String>, Vec<String>) {
    let (mut give, mut take) = (Vec::new(), Vec::new());
    for m in nodes::modules() {
        if m.outputs.iter().any(|p| kind_id(p.kind) == id) {
            give.push(m.label.clone());
        }
        if m.inputs.iter().any(|p| kind_id(p.kind) == id) {
            take.push(m.label.clone());
        }
    }
    (give, take)
}

impl App {
    /// The menu's entries. Returns the type picked, if one was clicked.
    pub(crate) fn types_menu(&mut self, ui: &mut egui::Ui) -> Option<&'static str> {
        let mut pick = None;
        for (id, places) in corroborate::types::conflicts() {
            ui.colored_label(WARN, format!("{id}: defined differently in {}", places.join(" and ")));
        }
        for t in corroborate::types::all() {
            let row = ui.horizontal(|ui| {
                type_dot(ui, &t);
                let b = ui.add(egui::Button::new(egui::RichText::new(&t.label).color(TEXT)).frame(false));
                ui.colored_label(DIM, egui::RichText::new(format!("{} · {}", t.flow.name(), t.origin)).small());
                b
            });
            let (give, take) = users(t.id);
            let list = |v: &[String]| if v.is_empty() { "none".to_string() } else { v.join(", ") };
            let usable = !matches!(t.flow, Flow::Stream) && t.id != "any";
            let hint = if usable { "click: add a Value node of it" } else { "made by modules only" };
            let resp = row.inner.on_hover_text(format!("{}\n\ngiven by: {}\ntaken by: {}\n\n{hint}", t.description, list(&give), list(&take)));
            if resp.clicked() && usable {
                pick = Some(t.id);
                ui.close();
            }
        }
        pick
    }
}
