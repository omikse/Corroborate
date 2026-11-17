//! The Invert location node in the workbench: the area it gives (its status line shows it).
//! Compiled into the `corroborate` program.

use super::*;

impl App {
    /// Everywhere except the wired location.
    pub(crate) fn invert_area_out(&self, i: NodeId) -> Result<(Arc<Area>, String), String> {
        let (f, _) = self.graph.feeder(i, "location").ok_or("no location wired in")?;
        let (a, note) = self.area_from(f)?;
        Ok((Arc::new(a.inverted()), format!("everywhere but: {note}")))
    }
}

/// How this node looks on the canvas: the standard look. Override `NodeLook` methods here to
/// change it (see `src/bin/corroborate/look.rs`).
pub struct Look;

impl crate::look::NodeLook for Look {}
