//! The Combine locations node in the workbench: the area it gives and its window.
//! Compiled into the `corroborate` program.

use super::*;

impl App {
    /// What each wired location still allows, combined: all must hold (AND) or any may (OR).
    pub(crate) fn combine_area_out(&self, i: NodeId, any: bool) -> Result<(Arc<Area>, String), String> {
        let mut acc: Option<WorldMask> = None;
        let mut used = Vec::new();
        for inp in nodes::COMBINE_INPUTS {
            let Some((f, _)) = self.graph.feeder(i, inp) else { continue };
            if let Ok((a, _)) = self.area_from(f) {
                let al = a.allowed();
                acc = Some(match acc {
                    None => al,
                    Some(m) if any => m.or(&al),
                    Some(m) => m.and(&al),
                });
                used.push(self.nodes[f].label.clone());
            }
        }
        let m = acc.ok_or("no location with an area wired in yet")?;
        let how = if any { "any of" } else { "all of" };
        let note = if m.is_empty() {
            format!("CONTRADICTION: nothing is possible under {how} {}", used.join(", "))
        } else {
            format!("possible under {how} {}", used.join(", "))
        };
        Ok((Arc::new(Area::add(m)), note))
    }

    pub(crate) fn combine_window(&mut self, ui: &mut egui::Ui, i: NodeId) {
        if let Kind::Combine { any } = &mut self.nodes[i].kind {
            ui.horizontal(|ui| {
                ui.radio_value(any, false, "all must hold (AND)");
                ui.radio_value(any, true, "any may hold (OR)");
            });
        }
        ui.colored_label(DIM, "Takes what each wired location still allows and combines it. The result adds (green).");
        match self.area_from(i) {
            Ok((_, note)) => {
                ui.colored_label(if note.contains("CONTRADICTION") { WARN } else { TEXT }, note);
            }
            Err(e) => {
                ui.colored_label(DIM, e);
            }
        }
    }
}

/// How this node looks on the canvas: the standard look. Override `NodeLook` methods here to
/// change it (see `src/bin/corroborate/look.rs`).
pub struct Look;

impl crate::look::NodeLook for Look {}
