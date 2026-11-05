//! The Microphone node in the workbench: on the canvas, the device and what it gave so far.
//! Compiled into the `corroborate` program.

use super::*;

/// What the node opens at Run, if a device is chosen.
pub fn input(device: &str) -> Option<Input> {
    (!device.trim().is_empty()).then(|| Input::Mic(device.trim().to_string()))
}

/// The Microphone node on the canvas: the device list with Refresh, then status, audio and clock.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, _app: &App, _i: NodeId, _standard: Vec2) -> Vec2 {
        Vec2::new(330.0, 150.0)
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let z = app.zoom;
        let st = app.nodes[i].state.clone();
        let Kind::Mic { device } = &app.nodes[i].kind else { return };
        let mut device = device.clone();
        let mut refresh = false;
        crate::look::area(ui, r, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("mic", i))
                    .width(r.width() - 80.0 * z)
                    .selected_text(if device.is_empty() { "choose a microphone" } else { device.as_str() })
                    .show_ui(ui, |ui| {
                        for m in &app.mics {
                            ui.selectable_value(&mut device, m.clone(), m);
                        }
                    });
                refresh = ui.button("Refresh").clicked();
            });
            source_info(ui, &st);
        });
        if refresh {
            match corroborate::ingest::list_microphones() {
                Ok(m) => app.mics = m,
                Err(e) => app.msg = e,
            }
        }
        if let Kind::Mic { device: d } = &mut app.nodes[i].kind {
            *d = device;
        }
    }

    fn status(&self) -> bool {
        false
    }
}
