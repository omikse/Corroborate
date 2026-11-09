//! The project: Project → New…, Open…, Save as… and Save a copy… (each picks a folder), Open
//! recent, Save, Revert to saved.
//!
//! A saved project is a folder, and the folder's name is the project's only name. In it are
//! `project.txt` (nodes, wires, clocks, evidence and its recordings, one per line), `results\`
//! for what its Result records write and `recordings\` for files copied into it (recordings.rs).
//! New and Save as pick (or make) the folder; a folder that already holds another project is
//! refused. Clicking the name in the top bar renames the folder. projects\ next to the program is
//! where the dialogs start. Recordings are kept with their path relative to the project file and
//! their absolute path, so a project folder can move.
//!
//! The project file changes only on Save (Ctrl+S). Changes go to `project.recovery.txt` beside it
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

/// Takes `p` out of `dir`\recent.txt.
pub(crate) fn drop_recent(dir: &Path, p: &Path) {
    let k = recordings::key(p);
    let text: String = recent_in(dir).iter().filter(|q| recordings::key(q) != k).map(|q| format!("{}\n", q.display())).collect();
    let _ = std::fs::write(dir.join("recent.txt"), text);
}

/// The recovery file of project file `p`: `project.recovery.txt` beside it.
pub(crate) fn recovery_of(p: &Path) -> PathBuf {
    p.with_file_name(format!("{}.recovery.txt", p.file_stem().unwrap_or_default().to_string_lossy()))
}

/// The project file of project folder `dir`.
pub(crate) fn project_in(dir: &Path) -> PathBuf {
    dir.join("project.txt")
}

/// The project in folder `dir`, for Open; None when it holds none.
pub(crate) fn project_file_of(dir: &Path) -> Option<PathBuf> {
    let file = project_in(dir);
    file.is_file().then_some(file)
}

/// The name of the project whose file is `file`: its folder's.
pub(crate) fn name_of(file: &Path) -> String {
    file.parent().and_then(Path::file_name).unwrap_or_default().to_string_lossy().into_owned()
}

/// Why a folder cannot be called `name` on Windows; None when it can.
pub(crate) fn bad_name(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("a project needs a name".into());
    }
    if let Some(c) = name.chars().find(|c| "<>:\"/\\|?*".contains(*c) || c.is_control()) {
        return Some(format!("a folder name cannot contain {c}"));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("a folder name cannot end with a dot or a space".into());
    }
    let device = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let numbered = |p: &str| device.strip_prefix(p).is_some_and(|d| matches!(d, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    if matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT") {
        return Some(format!("{device} is reserved by Windows"));
    }
    None
}

/// The project already in folder `dir` that writing a project there would replace; `own`, the
/// project being saved, does not count.
pub(crate) fn taken_by(dir: &Path, own: Option<&Path>) -> Option<PathBuf> {
    project_file_of(dir).filter(|p| own.is_none_or(|o| recordings::key(o) != recordings::key(p)))
}

/// What waits for an answer about unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Then {
    /// A new project in this folder.
    New(PathBuf),
    Open(PathBuf),
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

    /// The project's name for the top bar: its folder's, or "untitled".
    pub(crate) fn project_name(&self) -> String {
        self.project_path.as_deref().map_or("untitled".into(), name_of)
    }

    /// Renames the project: its folder takes the new name, what is inside moves with it, and
    /// recordings inside it are read from the new place. Unsaved changes stay unsaved. Windows
    /// refuses while a file in the folder is open, which is reported as it is.
    pub(crate) fn rename_project(&mut self, name: &str) -> bool {
        let name = name.trim();
        let Some(file) = self.project_path.clone() else { return false };
        let Some(old) = file.parent().map(Path::to_path_buf) else { return false };
        if name == name_of(&file) {
            return false;
        }
        if let Some(why) = bad_name(name) {
            self.msg = format!("not renamed: {why}");
            return false;
        }
        if self.copying() {
            self.msg = "not renamed: a recording is still being copied into the project".into();
            return false;
        }
        let new = old.with_file_name(name);
        // Only the letter case changing is the same folder, which does exist already.
        if new.exists() && recordings::key(&new) != recordings::key(&old) {
            self.msg = format!("not renamed: {} already exists", new.display());
            return false;
        }
        if let Err(e) = std::fs::rename(&old, &new) {
            self.msg = format!("not renamed: {e} (press Stop, and close whatever has a file in the folder open)");
            return false;
        }
        let unsaved = self.dirty();
        let new_file = project_in(&new);
        self.project_path = Some(new_file.clone());
        self.folder_moved(&old, &new);
        if !cfg!(test) {
            drop_recent(&settings_dir(), &file);
        }
        remember(Some(&new_file));
        // The file keeps the old absolute paths of recordings inside the folder until written.
        if unsaved {
            self.save_project();
        } else {
            self.save();
        }
        self.msg = format!("renamed to {}", new.display());
        true
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

    /// Project → Save: writes the project file. False while untitled (that needs Save as).
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

    /// Project → Save as: writes the project into folder `dir`, which becomes this project's; the
    /// old project's recovery file goes, as its changes are in the new one.
    pub(crate) fn save_project_as(&mut self, dir: PathBuf) {
        let path = project_in(&dir);
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

    /// Project → Save a copy: writes the project into folder `dir`; this one stays open as it is.
    pub(crate) fn save_copy(&mut self, dir: PathBuf) -> Option<PathBuf> {
        let path = project_in(&dir);
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

    /// Project → New: an empty project in folder `dir`, its project.txt written at once. An
    /// untitled project before it stays in settings\project.txt.
    pub(crate) fn new_project(&mut self, dir: &Path) {
        let file = project_in(dir);
        if !App::empty().save_project_to(&file) {
            self.msg = format!("cannot write {}", file.display());
            return;
        }
        self.close_project();
        remember(Some(&file));
        self.msg = format!("new project {}", file.display());
        self.project_path = Some(file);
        self.saved_text = self.project_text();
        self.last_project_save = Instant::now();
    }

    /// Project → Open: opens `path` in place of this project (Open asks about unsaved changes first).
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
                    self.stopped = true;
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
                    self.msg = format!("restored unsaved changes from {}: Save keeps them, Project -> Revert to saved drops them", rec.display());
                } else {
                    let _ = std::fs::remove_file(&rec);
                }
            }
        }
        remember(Some(&path));
        self.last_project_save = Instant::now();
        // A project opens stopped: Run starts it.
        self.stopped = true;
        self.msg = format!("{} (stopped: Run starts it)", self.msg);
    }

    /// Project → Revert to saved: drops the unsaved changes and opens the project file again.
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
                    match self.free_folder("Save the project in this folder", false) {
                        Some(d) => {
                            self.save_project_as(d);
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
            Then::New(d) => self.new_project(&d),
            Then::Open(p) => self.open_project(p),
            Then::Quit => self.close_now = true,
            Then::Revert => self.revert(),
        }
    }

    /// A folder picker starting in the folder holding this project's folder, so another project
    /// is one step away, or in projects\.
    fn folder_dialog(&self, title: &str) -> Option<PathBuf> {
        let start = match &self.project_path {
            Some(p) => p.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_else(projects_dir),
            None => projects_dir(),
        };
        let _ = std::fs::create_dir_all(&start);
        rfd::FileDialog::new().set_directory(start).set_title(title).pick_folder()
    }

    /// A folder for a project to be written in. One that holds another project is refused, as a
    /// folder picker gives no warning before its file would be written over; with `own_ok`, this
    /// project's own folder is fine.
    fn free_folder(&mut self, title: &str, own_ok: bool) -> Option<PathBuf> {
        let d = self.folder_dialog(title)?;
        let own = self.project_path.clone().filter(|_| own_ok);
        if let Some(p) = taken_by(&d, own.as_deref()) {
            self.msg = format!("{} already holds the project {}: pick another folder, or Open it", d.display(), name_of(&p));
            return None;
        }
        Some(d)
    }

    /// New: pick (or make) the new project's folder, then ask about unsaved changes.
    fn new_dialog(&mut self) {
        if let Some(d) = self.free_folder("Make the new project in this folder", false) {
            self.request(Then::New(d));
        }
    }

    /// Open: pick a project's folder, then ask about unsaved changes.
    fn open_dialog(&mut self) {
        let Some(d) = self.folder_dialog("Open the project in this folder") else { return };
        match project_file_of(&d) {
            Some(p) => self.request(Then::Open(p)),
            None => self.msg = format!("no project in {} (it holds no project.txt)", d.display()),
        }
    }

    /// Save as: pick (or make) the project's folder.
    fn save_as_dialog(&mut self) {
        if let Some(d) = self.free_folder("Save the project in this folder", true) {
            self.save_project_as(d);
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
            self.open_dialog();
        } else if hit(M::COMMAND, Key::N) {
            self.new_dialog();
        }
    }

    /// The Project menu in the top bar, with the project's name beside it (• while it has unsaved
    /// changes), then the Recordings menu.
    pub(crate) fn file_menu(&mut self, ui: &mut egui::Ui) {
        self.project_keys(&ui.ctx().clone());
        let named = self.project_path.is_some();
        let dirty = self.dirty();
        ui.menu_button("Project", |ui| {
            if ui.add(egui::Button::new("New…").shortcut_text("Ctrl+N")).on_hover_text("pick or make the new project's folder").clicked() {
                ui.close();
                self.new_dialog();
            }
            if ui.add(egui::Button::new("Open…").shortcut_text("Ctrl+O")).on_hover_text("pick a project's folder").clicked() {
                ui.close();
                self.open_dialog();
            }
            ui.menu_button("Open recent", |ui| {
                let list = recent();
                if list.is_empty() {
                    ui.colored_label(DIM, "none yet");
                }
                for p in list {
                    let there = p.is_file();
                    let b = ui.add_enabled(there, egui::Button::new(name_of(&p))).on_hover_text(p.display().to_string()).on_disabled_hover_text(format!("{} is not there", p.display()));
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
                if let Some(d) = self.free_folder("Save a copy of the project in this folder", false) {
                    self.save_copy(d);
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
        let hover = match (named, dirty) {
            (true, true) => format!("{path}\nunsaved changes (Ctrl+S saves; they are kept in {} until then)\nclick to rename the project and its folder", self.project_path.as_deref().map(recovery_of).unwrap_or_default().display()),
            (true, false) => format!("{path}\nclick to rename the project and its folder"),
            (false, true) => "untitled: kept in settings\\project.txt\nclick to give it a folder".to_string(),
            (false, false) => format!("{path}\nclick to give it a folder"),
        };
        self.name_box(ui, hover);
        if dirty {
            ui.colored_label(TEXT, "•");
        }
        ui.separator();
        self.recordings_menu(ui);
        ui.separator();
    }

    /// The project's name in the top bar. A click makes it a box to type a new name in: Enter or
    /// clicking away renames, Esc leaves it. An untitled project has no folder to rename, so the
    /// click gives it one.
    fn name_box(&mut self, ui: &mut egui::Ui, hover: String) {
        let Some(typed) = &mut self.renaming else {
            let r = ui.add(egui::Label::new(egui::RichText::new(self.project_name()).color(TEXT)).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::Text).on_hover_text(hover);
            if r.clicked() {
                if self.project_path.is_some() {
                    self.renaming = Some(self.project_name());
                    self.renaming_fresh = true;
                } else {
                    self.save_as_dialog();
                }
            }
            return;
        };
        let mut out = egui::TextEdit::singleline(typed).clip_text(false).desired_width(90.0).show(ui);
        if std::mem::take(&mut self.renaming_fresh) {
            out.response.request_focus();
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(typed.chars().count()));
            out.state.cursor.set_char_range(Some(all));
            out.state.store(ui.ctx(), out.response.id);
        } else if out.response.lost_focus() {
            let leave = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if let Some(name) = self.renaming.take().filter(|_| !leave) {
                self.rename_project(&name);
            }
        }
    }

    /// The question about unsaved changes, while one waits.
    pub(crate) fn ask_ui(&mut self, ctx: &egui::Context) {
        let Some(then) = self.ask.clone() else { return };
        let revert = then == Then::Revert;
        let mut a = None;
        egui::Window::new("Unsaved changes").collapsible(false).resizable(false).anchor(Align2::CENTER_TOP, [0.0, 90.0]).show(ctx, |ui| {
            let what = match &then {
                Then::New(_) => "before a new project",
                Then::Open(_) => "before opening another project",
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
