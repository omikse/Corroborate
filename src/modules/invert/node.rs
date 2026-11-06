//! The Invert location node in the workbench: the area it gives (its status line shows it).
//! Compiled into the `corroborate` program.

use super::*;

impl App {
    /// The wired location flipped: what it added now subtracts, and the other way round.
    pub(crate) fn invert_area_out(&self, i: NodeId) -> Result<(Arc<Area>, String), String> {
        let (f, _) = self.graph.feeder(i, "location").ok_or("no location wired in")?;
        let (a, note) = self.area_from(f)?;
        Ok((Arc::new(a.inverted()), format!("inverted ({}): {note}", if a.add { "adds → subtracts" } else { "subtracts → adds" })))
    }
}

/// How this node looks on the canvas: the standard look. Override `NodeLook` methods here to
/// change it (see `src/bin/corroborate/look.rs`).
pub struct Look;

impl crate::look::NodeLook for Look {}
