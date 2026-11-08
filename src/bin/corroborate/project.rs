//! The project: File → New, Open…, Open recent, Save, Save as…, Save a copy…, Revert to saved.
//!
//! A saved project is a folder named after it: `<name>\<name>.txt` (nodes, wires, clocks,
//! evidence and its recordings, one per line), with `results\` for what its Result records write
//! and `recordings\` for files copied into it (recordings.rs). Save as makes the folder (unless
//! the chosen file already sits in a folder of its name); projects\ next to the program is where
//! the dialogs start. Recordings are kept with their path relative to the project file and their
//! absolute path, so a project folder can move.
//!
//! The project file changes only on Save (Ctrl+S). Changes go to `<name>.recovery.txt` beside it
//! (on each change and every 10 s), and the next Open restores them when it is newer. The top bar
//! marks unsaved changes with •; New, Open and closing the window ask Save, Don't save or Cancel.
//!
//! The untitled project is settings\project.txt, written on every change and opened again at the
//! next start. The project open at the end is settings\last_project.txt; the ten opened last are
//! settings\recent.txt.

use super::*;
use std::path::Path;

fn settings_dir() -> PathBuf {
    corroborate::home().join("settings")
}

/// Where Open and Save as start: projects\ next to the program.
fn projects_dir() -> PathBuf {
    corroborate::home().join("projects")
}

/// How many projects Open recent keeps.
const RECENT: usize = 10;

/// The project opened last, when it was not the untitled one and still exists.
pub(crate) fn last_used() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let p = PathBuf::from(std::fs::read_to_string(settings_dir().join("last_project.txt")).ok()?.trim());
    p.is_file().then_some(p)
}

/// The projects in `dir`\recent.txt, newest first.
pub(crate) fn recent_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(dir.join("recent.txt")).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

/// Puts `p` first in `dir`\recent.txt (once, at most `RECENT`).
pub(crate) fn push_recent(dir: &Path, p: &Path) {
    let k = recordings::key(p);
    let mut list = recent_in(dir);
    list.retain(|q| recordings::key(q) != k);
    list.insert(0, p.to_path_buf());
    list.truncate(RECENT);
    let text: String = list.iter().map(|q| format!("{}\n", q.display())).collect();
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join("recent.txt"), text);
}

fn recent() -> Vec<PathBuf> {
    if cfg!(test) {
        return Vec::new();
    }
    recent_in(&settings_dir())
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
            push_recent(&settings_dir(), p);
        }
        None => {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// The recovery file of project file `p`: `<name>.recovery.txt` beside it.
pub(crate) fn recovery_of(p: &Path) -> PathBuf {
    p.with_file_name(format!("{}.recovery.txt", p.file_stem().unwrap_or_default().to_string_lossy()))
}

/// Where Save as puts a project chosen as `p`: in a folder named after it, `<name>\<name>.txt`,
/// unless `p` already is in one.
pub(crate) fn project_place(p: &Path) -> PathBuf {
    let stem = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let stem = stem.strip_suffix(".recovery").unwrap_or(&stem).to_string();
    let file = format!("{stem}.txt");
    let parent = p.parent().map(Path::to_path_buf).unwrap_or_default();
    if parent.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&stem)) {
        parent.join(file)
    } else {
        parent.join(&stem).join(file)
    }
}

/// What waits for an answer about unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Then {
    New,
    Open(PathBuf),
    OpenDialog,
    Quit,
    Revert,
}

/// The answer to "unsaved changes".
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Answer {
    Save,
    Discard,
    Cancel,
}

impl App {
    /// The file this project is: the one chosen with Open or Save as, or settings\project.txt
    /// while untitled. Tests drive the workbench headless (see drive.rs) and never read or write
    /// the untitled project; they may set `project_path`.
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

    /// Where results go: results\ beside the project file, or next to the program while untitled.
    pub(crate) fn results_dir(&self) -> PathBuf {
        match self.project_dir() {
            Some(d) => d.join("results"),
            None => corroborate::home().join("results"),
        }
    }

    /// Whether there is something Save would keep: a saved project whose text differs from its
    /// file, or an untitled project with nodes.
    pub(crate) fn dirty(&self) -> bool {
        match &self.project_path {
            None => self.nodes.iter().any(|n| !n.hidden),
            Some(_) => self.project_text() != self.saved_text,
        }
    }

    /// Called on every change and every 10 s: the untitled project writes settings\project.txt;
    /// a saved one writes its recovery file while it has unsaved changes (and drops it when not).
    pub(crate) fn save_project(&mut self) {
        self.last_project_save = Instant::now();
        match self.project_path.clone() {
            None => {
                if let Some(path) = self.project_file() {
                    self.save_project_to(&path);
                }
            }
            Some(p) => {
                let r = recovery_of(&p);
                if self.dirty() {
                    self.save_project_to(&r);
                } else if r.exists() {
                    let _ = std::fs::remove_file(r);
                }
            }
        }
    }

    /// Writes the project to `path`: first beside it as `<name>.tmp`, then renamed over it, so a
    /// crash while saving leaves the previous project whole, never half a file. Recording paths
    /// are written relative to `path`'s folder.
    pub(crate) fn save_project_to(&self, path: &Path) -> bool {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.project_text_in(path.parent())).is_ok() && std::fs::rename(&tmp, path).is_ok()
    }

    /// File → Save: writes the project file. False while untitled (that needs Save as).
    pub(crate) fn save(&mut self) -> bool {
        let Some(p) = self.project_path.clone() else { return false };
        self.sync_recordings();
        if !self.save_project_to(&p) {
            self.msg = format!("cannot write {}", p.display());
            return false;
        }
        self.saved_text = self.project_text();
        let _ = std::fs::remove_file(recovery_of(&p));
        self.last_project_save = Instant::now();
        self.msg = format!("saved {}", p.display());
        true
    }

    /// File → Save as: writes the project into its folder (see `project_place`), which becomes
    /// this project's; the old project's recovery file goes, as its changes are in the new one.
    pub(crate) fn save_project_as(&mut self, path: PathBuf) {
        let path = project_place(&path);
        self.sync_recordings();
        if !self.save_project_to(&path) {
            self.msg = format!("cannot write {}", path.display());
            return;
        }
        if let Some(old) = &self.project_path {
            if recordings::key(old) != recordings::key(&path) {
                let _ = std::fs::remove_file(recovery_of(old));
            }
        }
        let _ = std::fs::remove_file(recovery_of(&path));
        self.msg = format!("saved as {}", path.display());
        remember(Some(&path));
        self.project_path = Some(path);
        self.saved_text = self.project_text();
        self.last_project_save = Instant::now();
    }

    /// File → Save a copy: writes the project into another folder; this one stays open as it is.
    pub(crate) fn save_copy(&mut self, path: PathBuf) -> Option<PathBuf> {
        let path = project_place(&path);
        self.sync_recordings();
        if !self.save_project_to(&path) {
            self.msg = format!("cannot write {}", path.display());
            return None;
        }
        self.msg = format!("saved a copy as {}", path.display());
        Some(path)
    }

    /// Ends recordings, stops the graph and empties the canvas (the Earth image and the device
    /// list stay). The untitled project is written first; a saved one is left as its file and
    /// recovery file have it.
    fn close_project(&mut self) {
        if self.project_path.is_none() {
            self.save_project();
        }
        let recording: Vec<NodeId> = self.recordings.keys().copied().collect();
        for i in recording {
            self.record_end(i);
        }
        self.stop_all();
        let (earth, mics, origin) = (self.earth.take(), std::mem::take(&mut self.mics), self.canvas_origin);
        *self = App::empty();
        (self.earth, self.mics, self.canvas_origin) = (earth, mics, origin);
    }

    /// File → New: an empty untitled project. The untitled project before it is kept as
    /// settings\project.bak.txt.
    pub(crate) fn new_project(&mut self) {
        let had_nodes = self.project_path.is_none() && self.nodes.iter().any(|n| !n.hidden);
        self.close_project();
        if !cfg!(test) {
            let d = settings_dir();
            if std::fs::read_to_string(d.join("project.txt")).is_ok_and(|t| t.lines().any(|l| l.starts_with("node|"))) {
                let _ = std::fs::copy(d.join("project.txt"), d.join("project.bak.txt"));
            }
        }
        remember(None);
        self.save_project();
        self.msg = if had_nodes { "new project (the old one is in settings\\project.bak.txt)".into() } else { "new project".into() };
    }

    /// File → Open: opens `path` in place of this project (Open asks about unsaved changes first).
    pub(crate) fn open_project(&mut self, path: PathBuf) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            self.msg = format!("cannot read {}", path.display());
            return;
        };
        self.close_project();
        self.load_into(path, &text, true);
    }

    /// At start: the project open at the end, or the untitled one.
    pub(crate) fn open_at_start(&mut self) {
        match last_used().and_then(|p| Some((std::fs::read_to_string(&p).ok()?, p))) {
            Some((text, p)) => self.load_into(p, &text, true),
            None => {
                if let Some(text) = self.project_file().and_then(|p| std::fs::read_to_string(p).ok()) {
                    self.load_project_text(&text);
                    self.resolve_recordings();
                    self.sync_recordings();
                }
            }
        }
    }

    /// Loads project file `path` (its text `text`) into the emptied workbench: finds its
    /// recordings, and with `restore` takes in its recovery file when that is newer.
    fn load_into(&mut self, path: PathBuf, text: &str, restore: bool) {
        self.project_path = Some(path.clone());
        self.load_project_text(text);
        self.resolve_recordings();
        self.sync_recordings();
        self.saved_text = self.project_text();
        self.msg = format!("opened {}", path.display());
        let rec = recovery_of(&path);
        let newer = |a: &Path, b: &Path| match (std::fs::metadata(a).and_then(|m| m.modified()), std::fs::metadata(b).and_then(|m| m.modified())) {
            (Ok(a), Ok(b)) => a >= b,
            _ => false,
        };
        if restore && newer(&rec, &path) {
            if let Ok(rt) = std::fs::read_to_string(&rec) {
                let saved = std::mem::take(&mut self.saved_text);
                self.close_project();
                self.project_path = Some(path.clone());
                self.load_project_text(&rt);
                self.resolve_recordings();
                self.sync_recordings();
                self.saved_text = saved;
                if self.dirty() {
                    self.msg = format!("restored unsaved changes from {}: Save keeps them, File → Revert to saved drops them", rec.display());
                } else {
                    let _ = std::fs::remove_file(&rec);
                }
            }
        }
        remember(Some(&path));
        self.last_project_save = Instant::now();
    }

    /// File → Revert to saved: drops the unsaved changes and opens the project file again.
    pub(crate) fn revert(&mut self) {
        let Some(p) = self.project_path.clone() else { return };
        let Ok(text) = std::fs::read_to_string(&p) else {
            self.msg = format!("cannot read {}", p.display());
            return;
        };
        let _ = std::fs::remove_file(recovery_of(&p));
        self.close_project();
        self.load_into(p, &text, false);
        self.msg = "reverted to the saved project".into();
    }

    /// New, Open, Revert and closing the window go through here: with unsaved changes they wait
    /// for an answer (`answer`), else they happen now. Closing asks only for a saved project (the
    /// untitled one is kept in settings\ as it is).
    pub(crate) fn request(&mut self, then: Then) {
        let ask = match then {
            Then::Quit => self.project_path.is_some() && self.dirty(),
            Then::Revert => self.project_path.is_some() && self.dirty(),
            _ => self.dirty(),
        };
        if ask {
            self.ask = Some(then);
        } else {
            self.carry_out(then);
        }
    }

    /// The answer to the waiting question: Save (Save as while untitled) and go on, go on without
    /// saving, or stay.
    pub(crate) fn answer(&mut self, a: Answer) {
        let Some(then) = self.ask.take() else { return };
        match a {
            Answer::Cancel => {}
            Answer::Save => {
                let saved = if self.project_path.is_some() {
                    self.save()
                } else {
                    match self.dialog().set_file_name("untitled.txt").save_file() {
                        Some(p) => {
                            self.save_project_as(p);
                            self.project_path.is_some()
                        }
                        None => false,
                    }
                };
                if saved {
                    self.carry_out(then);
                }
            }
            Answer::Discard => {
                if let Some(p) = &self.project_path {
                    let _ = std::fs::remove_file(recovery_of(p));
                    // Nothing left to keep: the 10 s autosave would write the recovery again.
                    self.saved_text = self.project_text();
                }
                self.carry_out(then);
            }
        }
    }

    fn carry_out(&mut self, then: Then) {
        match then {
            Then::New => self.new_project(),
            Then::Open(p) => self.open_project(p),
            Then::OpenDialog => {
                if let Some(p) = self.dialog().pick_file() {
                    self.open_project(p);
                }
            }
            Then::Quit => self.close_now = true,
            Then::Revert => self.revert(),
        }
    }

    fn dialog(&self) -> rfd::FileDialog {
        let start = match &self.project_path {
            // The folder holding the project's folder, so another project is one step away.
            Some(p) => p.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_else(projects_dir),
            None => projects_dir(),
        };
        let _ = std::fs::create_dir_all(&start);
        rfd::FileDialog::new().set_directory(start).add_filter("Corroborate project", &["txt"])
    }

    fn save_as_dialog(&mut self) {
        let name = format!("{}.txt", self.project_name());
        if let Some(p) = self.dialog().set_file_name(name).save_file() {
            self.save_project_as(p);
        }
    }

    /// Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S.
    fn project_keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut as K, Modifiers as M};
        let hit = |m: M, k: Key| ctx.input_mut(|i| i.consume_shortcut(&K::new(m, k)));
        if hit(M::COMMAND | M::SHIFT, Key::S) {
            self.save_as_dialog();
        } else if hit(M::COMMAND, Key::S) {
            if !self.save() {
                self.save_as_dialog();
            }
        } else if hit(M::COMMAND, Key::O) {
            self.request(Then::OpenDialog);
        } else if hit(M::COMMAND, Key::N) {
            self.request(Then::New);
        }
    }

    /// The File menu in the top bar, with the project's name beside it (• while it has unsaved
    /// changes), then the Recordings menu.
    pub(crate) fn file_menu(&mut self, ui: &mut egui::Ui) {
        self.project_keys(&ui.ctx().clone());
        let named = self.project_path.is_some();
        let dirty = self.dirty();
        ui.menu_button("File", |ui| {
            if ui.add(egui::Button::new("New").shortcut_text("Ctrl+N")).clicked() {
                ui.close();
                self.request(Then::New);
            }
            if ui.add(egui::Button::new("Open…").shortcut_text("Ctrl+O")).clicked() {
                ui.close();
                self.request(Then::OpenDialog);
            }
            ui.menu_button("Open recent", |ui| {
                let list = recent();
                if list.is_empty() {
                    ui.colored_label(DIM, "none yet");
                }
                for p in list {
                    let there = p.is_file();
                    let name = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                    let b = ui.add_enabled(there, egui::Button::new(name)).on_hover_text(p.display().to_string()).on_disabled_hover_text(format!("{} is not there", p.display()));
                    if b.clicked() {
                        ui.close();
                        self.request(Then::Open(p));
                    }
                }
                if !recent().is_empty() && ui.button("Clear recent").clicked() {
                    let _ = std::fs::remove_file(settings_dir().join("recent.txt"));
                    ui.close();
                }
            });
            ui.separator();
            if ui.add(egui::Button::new("Save").shortcut_text("Ctrl+S")).clicked() {
                ui.close();
                if !self.save() {
                    self.save_as_dialog();
                }
            }
            if ui.add(egui::Button::new("Save as…").shortcut_text("Ctrl+Shift+S")).clicked() {
                ui.close();
                self.save_as_dialog();
            }
            if ui.button("Save a copy…").on_hover_text("write the project into another folder; this one stays open").clicked() {
                ui.close();
                let name = format!("{} copy.txt", self.project_name());
                if let Some(p) = self.dialog().set_file_name(name).save_file() {
                    self.save_copy(p);
                }
            }
            if ui.add_enabled(named && dirty, egui::Button::new("Revert to saved")).clicked() {
                ui.close();
                self.request(Then::Revert);
            }
            ui.separator();
            if ui.add_enabled(named, egui::Button::new("Show project folder")).on_disabled_hover_text("save the project first").clicked() {
                ui.close();
                if let Some(d) = self.project_dir() {
                    let _ = std::process::Command::new("explorer").arg(d).spawn();
                }
            }
        });
        let path = self.project_file().map(|p| p.display().to_string()).unwrap_or_else(|| "not saved".into());
        let (name, hover) = match (named, dirty) {
            (true, true) => (format!("{} •", self.project_name()), format!("{path}\nunsaved changes (Ctrl+S saves; they are kept in {} until then)", self.project_path.as_deref().map(recovery_of).unwrap_or_default().display())),
            (false, true) => (format!("{} •", self.project_name()), "untitled: kept in settings\\project.txt; Save as gives it its own folder".to_string()),
            _ => (self.project_name(), path),
        };
        ui.colored_label(TEXT, name).on_hover_text(hover);
        ui.separator();
        self.recordings_menu(ui);
        ui.separator();
    }

    /// The question about unsaved changes, while one waits.
    pub(crate) fn ask_ui(&mut self, ctx: &egui::Context) {
        let Some(then) = self.ask.clone() else { return };
        let revert = then == Then::Revert;
        let mut a = None;
        egui::Window::new("Unsaved changes").collapsible(false).resizable(false).anchor(Align2::CENTER_TOP, [0.0, 90.0]).show(ctx, |ui| {
            let what = match &then {
                Then::New => "before a new project",
                Then::Open(_) | Then::OpenDialog => "before opening another project",
                Then::Quit => "before closing",
                Then::Revert => "they will be lost",
            };
            ui.label(format!("{} has changes that are not saved: {what}.", self.project_name()));
            ui.horizontal(|ui| {
                if !revert && ui.button("Save").clicked() {
                    a = Some(Answer::Save);
                }
                if ui.button(if revert { "Revert to saved" } else { "Don't save" }).clicked() {
                    a = Some(Answer::Discard);
                }
                if ui.button("Cancel").clicked() {
                    a = Some(Answer::Cancel);
                }
            });
        });
        if let Some(a) = a {
            self.answer(a);
        }
    }

    /// Closing the window: with unsaved changes it waits for the answer; the window title names
    /// the project.
    pub(crate) fn window_tick(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            if self.project_path.is_some() && self.dirty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.ask = Some(Then::Quit);
            } else {
                self.save_project();
            }
        }
        if self.close_now {
            self.close_now = false;
            self.quitting = true;
            self.save_project();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let title = format!("{}{} — Corroborate", self.project_name(), if self.dirty() { " •" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}
