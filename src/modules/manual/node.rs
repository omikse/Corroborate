//! The Manual data node in the workbench: its window, the area its coordinates give, its
//! canvas status and its part of the result file. Compiled into the `corroborate` program.

use super::*;

/// The coordinates typed in, as an area: within `radius_km` of the point (a row in the node),
/// or the 1° cell holding it when no radius is given.
pub fn area(rows: &[(String, String)]) -> Option<(WorldMask, String)> {
    let val = |k: &str| rows.iter().find(|(q, _)| q.trim() == k).map(|(_, v)| v.trim()).filter(|v| !v.is_empty());
    let c: Vec<f64> = val("coordinates")?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    let (lat, lon) = (*c.first()?, *c.get(1)?);
    match val("radius_km").and_then(|r| r.parse::<f64>().ok()) {
        Some(r) => Some((WorldMask::circle(lat, lon, r), format!("{r:.0} km around {lat:.4}, {lon:.4} (manual)"))),
        None => Some((WorldMask::circle(lat, lon, 0.0), format!("the 1° cell holding {lat:.4}, {lon:.4} (manual, no radius_km given)"))),
    }
}

/// The area on the node's place output.
pub fn area_out(rows: &[(String, String)]) -> Result<(Arc<Area>, String), String> {
    area(rows).map(|(a, l)| (Arc::new(Area::add(a)), l)).ok_or("no coordinates typed in".to_string())
}

pub fn status(rows: &[(String, String)]) -> String {
    format!("{} facts", rows.iter().filter(|(_, v)| !v.trim().is_empty()).count())
}

pub fn window(ui: &mut egui::Ui, i: NodeId, rows: &mut Vec<(String, String)>) {
    ui.colored_label(DIM, "Facts you know. Wired into a plugin, they are passed to it at Run (coordinates as lat, lon; elevation in m).");
    let mut remove = None;
    egui::Grid::new(("manual", i)).num_columns(3).show(ui, |ui| {
        for (r, (k, v)) in rows.iter_mut().enumerate() {
            ui.add(egui::TextEdit::singleline(k).desired_width(110.0));
            ui.add(egui::TextEdit::singleline(v).desired_width(240.0));
            if ui.small_button("✕").clicked() {
                remove = Some(r);
            }
            ui.end_row();
        }
    });
    if let Some(r) = remove {
        rows.remove(r);
    }
    if ui.button("+ field").clicked() {
        rows.push((String::new(), String::new()));
    }
}

pub fn json(rows: &[(String, String)]) -> String {
    let m: Vec<String> = rows
        .iter()
        .filter(|(k, v)| !k.trim().is_empty() && !v.trim().is_empty())
        .map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k.trim()), json_escape(v.trim())))
        .collect();
    format!(",\"facts\":{{{}}}", m.join(","))
}

impl App {
    /// A Manual node's coordinates wired into `id`'s input, as an area.
    pub(crate) fn manual_area_into(&self, id: NodeId, inp: &str) -> Option<(WorldMask, String)> {
        let (from, _) = self.graph.feeder(id, inp)?;
        let Kind::Manual { rows } = &self.nodes[from].kind else { return None };
        area(rows)
    }

    /// The rows of Manual nodes wired into `id`'s place and time inputs, each key once.
    pub(crate) fn manual_rows_into(&self, id: NodeId) -> Vec<(String, String)> {
        let mut rows = Vec::new();
        for inp in ["place", "time"] {
            if let Some((from, _)) = self.graph.feeder(id, inp) {
                if let Kind::Manual { rows: r } = &self.nodes[from].kind {
                    for (k, v) in r {
                        if !rows.iter().any(|(q, _): &(String, String)| q == k) {
                            rows.push((k.clone(), v.clone()));
                        }
                    }
                }
            }
        }
        rows
    }
}
