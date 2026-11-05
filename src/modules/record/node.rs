//! The Result record node in the workbench: the location its inputs agree on, its window,
//! canvas status and its part of the result file. Compiled into the `corroborate` program.

use super::*;

impl App {
    /// The record's location: the whole world, AND each area wired into its location inputs.
    /// Steps: (node, how its area was reached, the fraction of Earth it allows or None while it has none).
    pub(crate) fn record_area(&self, rec: NodeId) -> (WorldMask, Vec<(String, String, Option<f64>)>) {
        let mut m = WorldMask::world();
        let mut steps = Vec::new();
        for inp in nodes::RECORD_LOCATIONS {
            let Some((from, _)) = self.graph.feeder(rec, inp) else { continue };
            let label = self.nodes[from].label.clone();
            match self.area_from(from) {
                Ok((a, note)) => {
                    let al = a.allowed();
                    m = m.and(&al);
                    steps.push((label, note, Some(al.fraction())));
                }
                Err(e) => steps.push((label, e, None)),
            }
        }
        (m, steps)
    }

    /// The area on the record's location output.
    pub(crate) fn record_area_out(&self, i: NodeId) -> Result<(Arc<Area>, String), String> {
        let (m, steps) = self.record_area(i);
        let k = steps.iter().filter(|s| s.2.is_some()).count();
        Ok((Arc::new(Area::add(m)), format!("the whole world cut by {k} area(s)")))
    }

    pub(crate) fn record_status(&self, i: NodeId) -> String {
        let (m, _) = self.record_area(i);
        let left = if m.is_empty() { "nothing left: contradiction".to_string() } else { format!("{:.1} % of Earth left", m.fraction() * 100.0) };
        format!("{left} · {}", self.saved_note)
    }

    pub(crate) fn record_window(&mut self, ui: &mut egui::Ui, i: NodeId) {
        let (m, steps) = self.record_area(i);
        if m.is_empty() {
            ui.colored_label(WARN, "Location: nothing left. The evidence contradicts itself: check the steps below.");
        } else {
            ui.label(format!("Location: {:.2} % of Earth's surface left ({:.1} million km²), 1° cells", m.fraction() * 100.0, m.area_km2() / 1e6));
        }
        ui.colored_label(DIM, "start: the whole world");
        for (n, note, f) in &steps {
            let f = f.map_or("—".to_string(), |f| format!("{:.2} %", f * 100.0));
            ui.colored_label(if note.starts_with("CONTRADICTION") { WARN } else { TEXT }, format!("∩ {n} ({f}): {note}"));
        }
        if steps.is_empty() {
            ui.colored_label(DIM, "wire areas (ENF location, Manual place) into the location inputs to subtract from the world");
        }
        map_node::draw(ui, self.earth.as_ref().map(|t| t.id()), Some(&Area::add(m)));
        ui.separator();
        ui.label("Keeps one result JSON for the current run: every node's findings and facts, and the wires (the evidence trail). Saved every 30 s while running and when the run ends.");
        match &self.result_file {
            Some(p) => {
                ui.colored_label(TEXT, p.display().to_string());
                ui.colored_label(DIM, &self.saved_note);
            }
            None => {
                ui.colored_label(DIM, "not running yet");
            }
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(self.result_file.is_some(), egui::Button::new("Save now")).clicked() {
                self.save_result();
            }
            if ui.button("Open folder").clicked() {
                let d = results_dir();
                let _ = std::fs::create_dir_all(&d);
                let _ = std::process::Command::new("explorer").arg(d).spawn();
            }
        });
    }

    pub(crate) fn record_json(&self, i: NodeId) -> String {
        let (m, steps) = self.record_area(i);
        let st: Vec<String> = steps
            .iter()
            .map(|(n, note, f)| {
                format!(
                    "{{\"node\":\"{}\",\"note\":\"{}\",\"fraction_of_earth\":{}}}",
                    json_escape(n),
                    json_escape(note),
                    f.map_or("null".to_string(), |f| format!("{f:.5}"))
                )
            })
            .collect();
        format!(
            ",\"location\":{{\"start\":\"whole world\",\"cell_deg\":1,\"remaining_fraction_of_earth\":{:.5},\"remaining_km2\":{:.0},\"contradiction\":{},\"steps\":[{}],\"cells_row_first_last\":{}}}",
            m.fraction(),
            m.area_km2(),
            m.is_empty(),
            st.join(","),
            m.runs_json()
        )
    }
}
