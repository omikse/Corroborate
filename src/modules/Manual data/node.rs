//! The Manual data node in the workbench: its rows edited on the canvas, the area its coordinates give, its
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

/// The area on the node's place output. Building the circle takes about 2 ms and the canvas asks
/// every frame, so the last few are kept, keyed by the coordinates and radius typed in.
pub fn area_out(rows: &[(String, String)]) -> Result<(Arc<Area>, String), String> {
    type Memo = Vec<(String, Result<(Arc<Area>, String), String>)>;
    static MEMO: std::sync::Mutex<Memo> = std::sync::Mutex::new(Vec::new());
    let val = |k: &str| rows.iter().find(|(q, _)| q.trim() == k).map_or("", |(_, v)| v.trim());
    let key = format!("{}|{}", val("coordinates"), val("radius_km"));
    let mut memo = MEMO.lock().unwrap();
    if let Some((_, r)) = memo.iter().find(|(k, _)| *k == key) {
        return r.clone();
    }
    let r = area(rows).map(|(a, l)| (Arc::new(Area::add(a)), l)).ok_or("no coordinates typed in".to_string());
    if memo.len() >= 8 {
        let _ = memo.remove(0);
    }
    memo.push((key, r.clone()));
    r
}

pub fn status(rows: &[(String, String)]) -> String {
    format!("{} facts", rows.iter().filter(|(_, v)| !v.trim().is_empty()).count())
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
    /// The rows of Manual nodes wired into `id`'s place and time inputs, each key once.
    pub(crate) fn manual_rows_into(&self, id: NodeId) -> Vec<(String, String)> {
        let mut rows = Vec::new();
        for inp in ["place", "time"] {
            if let Some((from, _)) = self.graph.feeder(id, inp) {
                if let Kind::ManualData { rows: r } = &self.nodes[from].kind {
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

/// Height of one row on the canvas, unzoomed.
const ROW_H: f32 = 22.0;

/// The Manual data node on the canvas: one row per fact (key, value, 🗙 to remove) and "+ field". Wired
/// into a plugin, the facts are passed to it when it starts (coordinates as lat, lon; elevation
/// in m).
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, standard: Vec2) -> Vec2 {
        let n = match &app.nodes[i].kind {
            Kind::ManualData { rows } => rows.len(),
            _ => 0,
        };
        Vec2::new(340.0, standard.y.max(HEAD + 4.0 + (n + 1) as f32 * ROW_H + 28.0))
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let z = app.zoom;
        let Kind::ManualData { rows } = &mut app.nodes[i].kind else { return };
        let key_w = r.width() * 0.36;
        let val_w = r.width() - key_w - 30.0 * z;
        let mut remove = None;
        crate::look::area(ui, r, |ui| {
            for (n, (k, v)) in rows.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(k).hint_text("key").desired_width(key_w));
                    ui.add(egui::TextEdit::singleline(v).hint_text("value").desired_width(val_w));
                    if ui.small_button("🗙").on_hover_text("remove this row").clicked() {
                        remove = Some(n);
                    }
                });
            }
            if ui.small_button("+ field").clicked() {
                rows.push((String::new(), String::new()));
            }
        });
        if let Some(n) = remove {
            rows.remove(n);
        }
    }
}
