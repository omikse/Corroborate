//! How a node looks on the canvas. Nodes have no separate windows: everything a node shows and
//! every control it has is drawn in the node itself. Every node folder has a `Look` in its
//! `node.rs` implementing this trait; each method has a default. Override a method in the node's
//! own folder to change only that node:
//!
//! - `size`: the node's size on the canvas (unzoomed), given the standard size.
//! - `fill`: the node's background colour.
//! - `body`: draw inside the node, below its title and above its status line (the port dots and
//!   names, centred on the side edges, are drawn over it). `ui` is a child area there, with text
//!   and spacing already scaled to the zoom: paint with `ui.painter()`, or add controls (buttons,
//!   text fields, combo boxes); controls take clicks before the node's own drag. Use `content`
//!   for the part clear of the port names, and multiply your own sizes by `app.zoom`.
//! - `status`: false hides the status line at the bottom of the node.

use super::*;

pub trait NodeLook: Sync {
    fn size(&self, _app: &App, _i: NodeId, standard: Vec2) -> Vec2 {
        standard
    }

    fn fill(&self, _app: &App, _i: NodeId) -> Color32 {
        NODE
    }

    fn body(&self, _app: &mut App, _ui: &mut egui::Ui, _i: NodeId) {}

    fn status(&self) -> bool {
        true
    }
}

/// The look of a node kind: its folder's `Look`.
pub fn of(kind: &Kind) -> &'static dyn NodeLook {
    match kind {
        Kind::Sourcer { .. } => &sourcer_node::Look,
        Kind::ManualData { .. } => &manual_data_node::Look,
        Kind::Planes => &planes_node::Look,
        Kind::Enf { .. } => &enf_node::Look,
        Kind::GridFrequencies { .. } => &grid_frequencies_node::Look,
        Kind::Plugin { .. } => &plugin_node::Look,
        Kind::ResultRecord { .. } => &result_record_node::Look,
        Kind::Map => &map_node::Look,
        Kind::CombineLocations { .. } => &combine_locations_node::Look,
        Kind::InvertLocation => &invert_location_node::Look,
        Kind::Missing { .. } => &Missing,
    }
}

/// A node whose module this tool does not have: grey, nothing inside, its saved wires kept.
struct Missing;

impl NodeLook for Missing {
    fn fill(&self, _app: &App, _i: NodeId) -> Color32 {
        Color32::from_rgb(52, 52, 52)
    }
}

/// Text, spacing and controls in a node scaled with the canvas zoom (called before `body`).
pub fn scale(ui: &mut egui::Ui, z: f32) {
    let st = ui.style_mut();
    for f in st.text_styles.values_mut() {
        f.size *= z;
    }
    st.spacing.item_spacing *= z;
    st.spacing.button_padding *= z;
    st.spacing.interact_size *= z;
    st.spacing.icon_width *= z;
    st.spacing.icon_width_inner *= z;
    st.spacing.icon_spacing *= z;
}

/// Width a port name takes beside its dot, unzoomed (11.5 px text, about 6.3 px a character).
fn names_w(ports: &[corroborate::core::Port]) -> f32 {
    // An input that takes anything shows what is wired into it: room for about 16 characters.
    ports.iter().map(|p| if p.kind == corroborate::core::PortKind::Any { 16 } else { p.name.chars().count() }).max().map_or(0.0, |n| n as f32 * 6.3 + 16.0)
}

/// The body area clear of the port names on the left (inputs) and right (outputs).
pub fn content(app: &App, ui: &egui::Ui, i: NodeId) -> Rect {
    let spec = &app.graph.nodes()[i].spec;
    let r = ui.max_rect();
    Rect::from_min_max(Pos2::new(r.left() + names_w(&spec.inputs) * app.zoom, r.top()), Pos2::new(r.right() - names_w(&spec.outputs) * app.zoom, r.bottom()))
}

/// Run `f` in a child area at `rect`.
pub fn area<R>(ui: &mut egui::Ui, rect: Rect, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), f).inner
}

/// Small text in one colour, cut with … when it does not fit.
pub fn line(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text.into()).small().color(color)).truncate());
}

