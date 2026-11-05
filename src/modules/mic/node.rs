//! The Microphone node in the workbench: its window and what it opens at Run.
//! Compiled into the `corroborate` program.

use super::*;

/// What the node opens at Run, if a device is chosen.
pub fn input(device: &str) -> Option<Input> {
    (!device.trim().is_empty()).then(|| Input::Mic(device.trim().to_string()))
}

impl App {
    pub(crate) fn mic_window(&mut self, ui: &mut egui::Ui, i: NodeId) {
        let Kind::Mic { device } = &self.nodes[i].kind else { return };
        let mut device = device.clone();
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("mic", i)).selected_text(if device.is_empty() { "choose a microphone" } else { device.as_str() }).show_ui(ui, |ui| {
                for m in &self.mics {
                    ui.selectable_value(&mut device, m.clone(), m);
                }
            });
            if ui.button("Refresh").clicked() {
                match corroborate::ingest::list_microphones() {
                    Ok(m) => self.mics = m,
                    Err(e) => self.msg = e,
                }
            }
        });
        if let Kind::Mic { device: d } = &mut self.nodes[i].kind {
            *d = device;
        }
        source_info(ui, &self.nodes[i].state.clone());
    }
}
