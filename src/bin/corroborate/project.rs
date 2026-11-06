//! Which project file the workbench works on: File → New, Open…, Save as…. The untitled project
//! is settings\project.txt; a project opened or saved elsewhere is remembered in
//! settings\last_project.txt and opened again at the next start. Every project autosaves to its
//! own file (on changes and every 10 s).

use super::*;
use std::path::Path;

fn settings_dir() -> PathBuf {
    corroborate::home().join("settings")
}

/// Where Open and Save as start: projects\ next to the program.
fn projects_dir() -> PathBuf {
    corroborate::home().join("projects")
}

/// The project opened last, when it was not the untitled one and still exists.
pub(crate) fn last_used() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let p = PathBuf::from(std::fs::read_to_string(settings_dir().join("last_project.txt")).ok()?.trim());
    p.is_file().then_some(p)
}

pub(crate) fn remember(path: Option<&Path>) {
    if cfg!(test) {
        return;
    }
    let f = settings_dir().join("last_project.txt");
    match path {
        Some(p) => {
            let _ = std::fs::create_dir_all(settings_dir());
            let _ = std::fs::write(f, p.to_string_lossy().as_bytes());
        }
        None => {
            let _ = std::fs::remove_file(f);
        }
    }
}

impl App {
    /// The file this project saves to. Tests drive the workbench headless (see drive.rs) and
    /// never read or write the untitled project; they may set `project_path`.
    pub(crate) fn project_file(&self) -> Option<PathBuf> {
        if self.project_path.is_some() || cfg!(test) {
            return self.project_path.clone();
        }
        Some(settings_dir().join("project.txt"))
    }

    /// The project's name for the top bar: its file name, or "untitled".
    pub(crate) fn project_name(&self) -> String {
        match &self.project_path {
            Some(p) => p.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
            None => "untitled".into(),
        }
    }

    /// Saves the project, ends recordings, stops the graph and empties the canvas (the Earth
    /// image and the device list stay).
    fn close_project(&mut self) {
        self.save_project();
        let recording: Vec<NodeId> = self.recordings.keys().copied().collect();
        for i in recording {
            self.record_end(i);
        }
        self.stop_all();
        let (earth, mics, origin) = (self.earth.take(), std::mem::take(&mut self.mics), self.canvas_origin);
        *self = App::empty();
        (self.earth, self.mics, self.canvas_origin) = (earth, mics, origin);
    }

    /// File → New: an empty untitled project. A non-empty untitled project is kept as
    /// settings\project.bak.txt first.
    pub(crate) fn new_project(&mut self) {
        let untitled = self.project_path.is_none();
        let had_nodes = self.nodes.iter().any(|n| !n.hidden);
        self.close_project();
        if untitled && had_nodes && !cfg!(test) {
            let d = settings_dir();
            let _ = std::fs::copy(d.join("project.txt"), d.join("project.bak.txt"));
        }
        remember(None);
        self.save_project();
        self.msg = if untitled && had_nodes { "new project (the old one is in settings\\project.bak.txt)".into() } else { "new project".into() };
    }

    /// File → Open: saves this project, then opens `path` in its place.
    pub(crate) fn open_project(&mut self, path: PathBuf) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            self.msg = format!("cannot read {}", path.display());
            return;
        };
        self.close_project();
        self.load_project_text(&text);
        self.msg = format!("opened {}", path.display());
        remember(Some(&path));
        self.project_path = Some(path);
        self.last_project_save = Instant::now();
    }

    /// File → Save as: writes the project to `path`, which becomes this project's file.
    pub(crate) fn save_project_as(&mut self, path: PathBuf) {
        if !self.save_project_to(&path) {
            self.msg = format!("cannot write {}", path.display());
            return;
        }
        self.msg = format!("saved as {}", path.display());
        remember(Some(&path));
        self.project_path = Some(path);
        self.last_project_save = Instant::now();
    }

    fn dialog(&self) -> rfd::FileDialog {
        let start = match &self.project_path {
            Some(p) => p.parent().map(Path::to_path_buf).unwrap_or_else(projects_dir),
            None => projects_dir(),
        };
        let _ = std::fs::create_dir_all(&start);
        rfd::FileDialog::new().set_directory(start).add_filter("Corroborate project", &["txt"])
    }

    /// The File menu in the top bar, with the project's name beside it.
    pub(crate) fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui.button("New").clicked() {
                self.new_project();
                ui.close();
            }
            if ui.button("Open…").clicked() {
                ui.close();
                if let Some(p) = self.dialog().pick_file() {
                    self.open_project(p);
                }
            }
            if ui.button("Save as…").clicked() {
                ui.close();
                let name = format!("{}.txt", self.project_name());
                if let Some(p) = self.dialog().set_file_name(name).save_file() {
                    self.save_project_as(p);
                }
            }
        });
        let path = self.project_file().map(|p| p.display().to_string()).unwrap_or_default();
        ui.colored_label(TEXT, self.project_name()).on_hover_text(path);
        ui.separator();
    }
}
