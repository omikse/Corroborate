//! The Grid frequencies node in the workbench: finding and reading the grid file, its window
//! and its part of the result file. Compiled into the `corroborate` program.

use super::*;
use corroborate::modules::grid_frequencies::{BUNDLED_FILE, SOURCE_DIR};

/// The grid-frequency file that ships with the program: next to it, or in the source tree it was built from.
pub fn bundled_file() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let in_source = |root: PathBuf| root.join(SOURCE_DIR).join(BUNDLED_FILE);
    [dir.join(BUNDLED_FILE), dir.join("data").join(BUNDLED_FILE), in_source(dir.join("../..")), in_source(env!("CARGO_MANIFEST_DIR").into())]
        .into_iter()
        .find(|p| p.exists())
        .map_or(String::new(), |p| p.canonicalize().unwrap_or(p).to_string_lossy().trim_start_matches(r"\\?\").to_string())
}

/// An empty path means the grid file that ships with the program (found relative to it).
pub fn load(path: &str) -> Result<GridMap, String> {
    let bundled;
    let path = if path.trim().is_empty() {
        bundled = bundled_file();
        if bundled.is_empty() {
            return Err("no grid file next to the program: choose one".into());
        }
        bundled.as_str()
    } else {
        path
    };
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    GridMap::parse(&text).map_err(|e| format!("{path}: {e}"))
}

pub fn json(path: &str) -> String {
    let r = match load(path) {
        Ok(g) => format!("\"regions\":{},\"data_source\":\"{}\"", g.regions.len(), json_escape(&g.source)),
        Err(e) => format!("\"error\":\"{}\"", json_escape(&e)),
    };
    format!(",\"file\":\"{}\",{r}", json_escape(path))
}

/// The Grid frequencies node on the canvas: the data file (empty: the bundled one) with Browse.
/// Read when the graph (re)starts.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, _app: &App, _i: NodeId, standard: Vec2) -> Vec2 {
        Vec2::new(300.0, standard.y + 24.0)
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let z = app.zoom;
        let Kind::GridFrequencies { path } = &mut app.nodes[i].kind else { return };
        crate::look::area(ui, r, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(path).hint_text("bundled file").desired_width(r.width() - 70.0 * z));
                if ui.button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                        *path = p.to_string_lossy().into_owned();
                    }
                }
            });
        });
    }
}
