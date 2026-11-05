//! How a node looks and behaves on the canvas. Every node folder has a `Look` in its `node.rs`
//! implementing this trait; each method has a default, so an empty `impl NodeLook for Look {}`
//! is the standard node. Override a method in the node's own folder to change only that node:
//!
//! - `size`: the node's size on the canvas (unzoomed), given the standard size.
//! - `fill`: the node's background colour.
//! - `body`: draw inside the node, below its title and above its status line (the port dots and
//!   names, centred on the side edges, are drawn over it). `ui` is a child
//!   area there: paint with `ui.painter()`, or add controls (buttons, text fields, combo boxes);
//!   controls take clicks before the node's own drag. Multiply sizes by `app.zoom`.
//! - `status`: false hides the status line at the bottom of the node.
//! - `window`: false means double-click and "Open window" do nothing for this node.

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

    fn window(&self) -> bool {
        true
    }
}

/// The look of a node kind: its folder's `Look`.
pub fn of(kind: &Kind) -> &'static dyn NodeLook {
    match kind {
        Kind::File { .. } => &file_node::Look,
        Kind::Stream { .. } => &stream_node::Look,
        Kind::Mic { .. } => &mic_node::Look,
        Kind::Manual { .. } => &manual_node::Look,
        Kind::Planes => &planes_node::Look,
        Kind::Enf { .. } => &enf_node::Look,
        Kind::GridFrequencies { .. } => &grid_node::Look,
        Kind::Plugin { .. } => &plugin_node::Look,
        Kind::Record => &record_node::Look,
        Kind::Map => &map_node::Look,
        Kind::Combine { .. } => &combine_node::Look,
        Kind::Invert => &invert_node::Look,
    }
}
