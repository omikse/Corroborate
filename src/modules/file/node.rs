//! The File node in the workbench: its window and what it opens at Run.
//! Compiled into the `corroborate` program.

use super::*;

/// What the node opens at Run, if a file is set.
pub fn input(path: &str) -> Option<Input> {
    (!path.trim().is_empty()).then(|| Input::File(path.trim().to_string()))
}

pub fn window(ui: &mut egui::Ui, path: &mut String, st: &Shared) {
    ui.horizontal(|ui| {
        ui.label("File");
        ui.add(egui::TextEdit::singleline(path).desired_width(300.0));
        if ui.button("Browse…").clicked() {
            if let Some(p) = rfd::FileDialog::new().pick_file() {
                *path = p.to_string_lossy().into_owned();
            }
        }
    });
    source_info(ui, st);
}

/// How this node looks on the canvas: the standard look. Override `NodeLook` methods here to
/// change it (see `src/bin/corroborate/look.rs`).
pub struct Look;

impl crate::look::NodeLook for Look {}
