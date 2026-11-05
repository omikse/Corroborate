//! The Invert location node in the workbench: the area it gives and its window.
//! Compiled into the `corroborate` program.

use super::*;

impl App {
    /// The wired location flipped: what it added now subtracts, and the other way round.
    pub(crate) fn invert_area_out(&self, i: NodeId) -> Result<(Arc<Area>, String), String> {
        let (f, _) = self.graph.feeder(i, "location").ok_or("no location wired in")?;
        let (a, note) = self.area_from(f)?;
        Ok((Arc::new(a.inverted()), format!("inverted ({}): {note}", if a.add { "adds → subtracts" } else { "subtracts → adds" })))
    }

    pub(crate) fn invert_window(&mut self, ui: &mut egui::Ui, i: NodeId) {
        ui.colored_label(DIM, "Flips a location: a green (adds) area becomes red (subtracts) on the same cells, and the other way round.");
        match self.area_from(i) {
            Ok((_, note)) => {
                ui.label(note);
            }
            Err(e) => {
                ui.colored_label(DIM, e);
            }
        }
    }
}
