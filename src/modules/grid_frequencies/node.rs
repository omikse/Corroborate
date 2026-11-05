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

pub fn window(ui: &mut egui::Ui, path: &mut String) {
    ui.horizontal(|ui| {
        ui.label("File");
        ui.add(egui::TextEdit::singleline(path).hint_text("empty: the bundled file").desired_width(300.0));
        if ui.button("Browse…").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                *path = p.to_string_lossy().into_owned();
            }
        }
    });
    match load(path) {
        Ok(g) => {
            ui.colored_label(DIM, format!("data: {}", g.source));
            for r in &g.regions {
                ui.label(format!("{:.0} Hz · {}", r.hz, r.id));
            }
        }
        Err(e) => {
            ui.colored_label(WARN, e);
        }
    }
    if path.trim().is_empty() {
        ui.colored_label(DIM, format!("bundled file: {}", bundled_file()));
    }
    ui.colored_label(DIM, "Swap the file to use other grid data. Read at each Run.");
}

pub fn json(path: &str) -> String {
    let r = match load(path) {
        Ok(g) => format!("\"regions\":{},\"data_source\":\"{}\"", g.regions.len(), json_escape(&g.source)),
        Err(e) => format!("\"error\":\"{}\"", json_escape(&e)),
    };
    format!(",\"file\":\"{}\",{r}", json_escape(path))
}

impl App {
    /// The grid map from the Grid frequencies node wired into `id`'s "grid" input.
    pub(crate) fn grid_into(&self, id: NodeId) -> Result<Arc<GridMap>, String> {
        let Some((from, _)) = self.graph.feeder(id, "grid") else { return Err("no grid data wired".into()) };
        match &self.nodes[from].kind {
            Kind::GridFrequencies { path } => load(path).map(Arc::new),
            _ => Err("grid input is not a Grid frequencies node".into()),
        }
    }
}
