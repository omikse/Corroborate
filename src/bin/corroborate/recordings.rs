//! The project's recordings: every file a SOURCER reads belongs to the project, which keeps a list
//! of them (top bar → Recordings). Files stay where they are (Copy into project puts one in the
//! project's recordings\ folder); the project file keeps, for each, a line
//! `rec|<id>|<path relative to the project file>|<absolute path>|<bytes>|<modified, Unix seconds>|<print>`.
//!
//! Opening a project looks for each recording at its relative path first (a project folder moved
//! to another drive or computer), then at its absolute path, then by size and print in the
//! project folder and recordings\ next to the program; a SOURCER whose file was found elsewhere
//! is pointed there. The print is FNV-1a (Fowler–Noll–Vo, 64 bit) of the size and the first and
//! last 64 KiB: cheap on big videos, and enough to notice another or an edited file. It is not a
//! cryptographic hash. A recording whose size or print differs from what the project kept is
//! shown as changed until Accept change takes its new print.
//!
//! A recording stays in the project when its SOURCER is deleted (shown as not on the canvas),
//! until Remove. Only files are listed: web pages, stream URLs and microphones are not files.

use super::*;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path};
use std::sync::Mutex;

/// Bytes read from each end of a file for its print.
const END: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Not looked at since it was loaded.
    Unchecked,
    Ok,
    /// Not at its path, and not found elsewhere.
    Missing,
    /// There, but its size or print is not what the project kept.
    Changed,
    /// Found somewhere else than the project said (the old path); Save keeps the new one.
    Moved(PathBuf),
    /// Being copied into the project folder.
    Copying,
}

#[derive(Clone, Debug)]
pub struct Rec {
    pub id: String,
    /// Where it is now (absolute).
    pub path: PathBuf,
    pub size: u64,
    pub modified: u64,
    pub print: String,
    pub state: State,
    /// Size and modified time when the print was last compared, so a check reads the file only
    /// when either moved.
    seen: Option<(u64, u64)>,
}

type CopyDone = Arc<Mutex<Option<Result<PathBuf, String>>>>;

#[derive(Default)]
pub struct Library {
    pub list: Vec<Rec>,
    /// The number of the next id (`r<number>`).
    next: u64,
    copies: Vec<(String, CopyDone)>,
}

/// A path's key for comparing: absolute, `\` on Windows, any case there.
pub fn key(p: &Path) -> String {
    let a = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let s = a.to_string_lossy().into_owned();
    if cfg!(windows) {
        s.replace('/', "\\").to_lowercase()
    } else {
        s
    }
}

/// Size and modified time (Unix seconds) of a file; None when it is not a file.
fn stat(p: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(p).ok().filter(|m| m.is_file())?;
    let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    Some((m.len(), t))
}

/// The print of a file (see the top of this file), 16 hex digits.
pub fn print(p: &Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    let size = f.metadata().ok()?.len();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(&size.to_le_bytes());
    let mut buf = vec![0u8; END.min(size) as usize];
    f.read_exact(&mut buf).ok()?;
    eat(&buf);
    if size > END {
        let tail = END.min(size - END);
        f.seek(SeekFrom::End(-(tail as i64))).ok()?;
        let mut buf = vec![0u8; tail as usize];
        f.read_exact(&mut buf).ok()?;
        eat(&buf);
    }
    Some(format!("{h:016x}"))
}

/// `to` written relative to the folder `from` (`..\clips\a.wav`); None when they share no root
/// (another drive).
pub fn relative(from: &Path, to: &Path) -> Option<PathBuf> {
    let (from, to) = (std::path::absolute(from).ok()?, std::path::absolute(to).ok()?);
    let same = |a: &Component, b: &Component| {
        if cfg!(windows) {
            a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
        } else {
            a == b
        }
    };
    let (f, t): (Vec<Component>, Vec<Component>) = (from.components().collect(), to.components().collect());
    if f.is_empty() || t.is_empty() || !same(&f[0], &t[0]) {
        return None;
    }
    let common = f.iter().zip(&t).take_while(|(a, b)| same(a, b)).count();
    let mut r = PathBuf::new();
    for _ in common..f.len() {
        r.push("..");
    }
    for c in &t[common..] {
        r.push(c.as_os_str());
    }
    Some(r)
}

/// A file with `size` and `print` under `root` (at most 4 folders down), whatever its name.
fn find(root: &Path, size: u64, want: &str, depth: usize) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut dirs = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            dirs.push(p);
        } else if t.is_file() && stat(&p).is_some_and(|(s, _)| s == size) && print(&p).as_deref() == Some(want) {
            return Some(p);
        }
    }
    if depth == 0 {
        return None;
    }
    dirs.into_iter().find_map(|d| find(&d, size, want, depth - 1))
}

/// A free name for `name` in `dir`: itself, else `stem (2).ext`, `stem (3).ext`…
fn free_name(dir: &Path, name: &Path) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let ext = name.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|k| dir.join(format!("{stem} ({k}){ext}"))).find(|p| !p.exists()).unwrap()
}

impl Library {
    pub fn get(&self, id: &str) -> Option<&Rec> {
        self.list.iter().find(|r| r.id == id)
    }

    fn index_of_path(&self, p: &Path) -> Option<usize> {
        let k = key(p);
        self.list.iter().position(|r| key(&r.path) == k)
    }

    /// Adds the file at `p` (which exists), its size, time and print read now. Returns its index.
    fn add(&mut self, p: &Path) -> Option<usize> {
        let (size, modified) = stat(p)?;
        let print = print(p)?;
        self.next = self.next.max(1);
        let id = format!("r{}", self.next);
        self.next += 1;
        let path = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
        self.list.push(Rec { id, path, size, modified, print, state: State::Ok, seen: Some((size, modified)) });
        Some(self.list.len() - 1)
    }
}

impl Rec {
    /// Looks at the file again: missing, changed or fine. Reads it only when its size or time
    /// moved since the last look.
    pub fn check(&mut self) {
        if self.state == State::Copying {
            return;
        }
        let Some(now) = stat(&self.path) else {
            self.state = State::Missing;
            return;
        };
        if matches!(self.state, State::Moved(_)) && self.seen == Some(now) {
            return;
        }
        if self.seen != Some(now) {
            let same = now.0 == self.size && print(&self.path).is_some_and(|p| p == self.print);
            self.seen = Some(now);
            self.state = if same { State::Ok } else { State::Changed };
        } else if self.state == State::Unchecked || self.state == State::Missing {
            self.state = State::Ok;
        }
    }
}

impl App {
    /// The folder the project file is in (relative paths start there); None while untitled.
    pub(crate) fn project_dir(&self) -> Option<PathBuf> {
        self.project_path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf)
    }

    /// The file a SOURCER's text names, absolute; None for URLs, microphones and empty boxes.
    pub(crate) fn file_of(input: &str) -> Option<PathBuf> {
        match sourcer_node::input(input)? {
            Input::File(f) => Some(std::path::absolute(&f).unwrap_or_else(|_| PathBuf::from(f))),
            _ => None,
        }
    }

    /// Visible SOURCER nodes reading recording `r`.
    pub(crate) fn rec_users(&self, r: &Rec) -> Vec<NodeId> {
        let k = key(&r.path);
        (0..self.nodes.len())
            .filter(|&i| !self.nodes[i].hidden)
            .filter(|&i| matches!(&self.nodes[i].kind, Kind::Sourcer { input } if Self::file_of(input).is_some_and(|p| key(&p) == k)))
            .collect()
    }

    /// Each frame: a file a SOURCER reads that the project does not list yet joins it (once the
    /// file exists, so a path being typed does not). Finished copies are taken in.
    pub(crate) fn sync_recordings(&mut self) {
        let files: Vec<PathBuf> = self.nodes.iter().filter(|n| !n.hidden).filter_map(|n| match &n.kind {
            Kind::Sourcer { input } => Self::file_of(input),
            _ => None,
        }).collect();
        for f in files {
            if self.library.index_of_path(&f).is_none() && f.is_file() {
                self.library.add(&f);
            }
        }
        self.copy_tick();
    }

    /// The project lines for the recordings, paths relative to the folder `dir` (the project
    /// file's) when there is one.
    pub(crate) fn recordings_text(&self, dir: Option<&Path>) -> String {
        let mut t = String::new();
        for r in &self.library.list {
            let rel = dir.and_then(|d| relative(d, &r.path)).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            t.push_str(&format!("rec|{}|{}|{}|{}|{}|{}\n", clean(&r.id), clean(&rel), clean(&r.path.to_string_lossy()), r.size, r.modified, clean(&r.print)));
        }
        t
    }

    /// Reads one project line of a recording (kept as saved; `resolve_recordings` finds it).
    pub(crate) fn recording_line(&mut self, f: &[&str]) -> bool {
        let ["rec", id, rel, abs, size, modified, print] = f else { return false };
        let n = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
        if id.is_empty() || self.library.get(id).is_some() {
            return true;
        }
        if let Some(num) = id.strip_prefix('r').and_then(|n| n.parse::<u64>().ok()) {
            self.library.next = self.library.next.max(num + 1);
        }
        // Where it was saved; `rel` is kept in `seen`'s place until resolved.
        let path = PathBuf::from(abs);
        self.library.list.push(Rec { id: id.to_string(), path, size: n(size), modified: n(modified), print: print.to_string(), state: State::Unchecked, seen: None });
        self.rel_pending.push((id.to_string(), rel.to_string()));
        true
    }

    /// After loading: finds each recording (relative path, absolute path, then by size and print
    /// near the project), points the SOURCERs that read it at where it is, and checks it.
    pub(crate) fn resolve_recordings(&mut self) {
        let dir = self.project_dir();
        let pending = std::mem::take(&mut self.rel_pending);
        for k in 0..self.library.list.len() {
            let r = self.library.list[k].clone();
            let rel = pending.iter().find(|(id, _)| *id == r.id).map(|(_, rel)| rel.clone()).unwrap_or_default();
            let mut tries: Vec<PathBuf> = Vec::new();
            if let (Some(d), false) = (&dir, rel.is_empty()) {
                tries.push(d.join(&rel));
            }
            tries.push(r.path.clone());
            let fits = |p: &Path| stat(p).is_some_and(|(s, _)| s == r.size);
            let mut at = tries.iter().find(|p| fits(p)).cloned().or_else(|| tries.iter().find(|p| p.is_file()).cloned());
            if at.is_none() && !r.print.is_empty() {
                let roots = dir.iter().cloned().chain([corroborate::home().join("recordings")]);
                at = roots.into_iter().find_map(|root| find(&root, r.size, &r.print, 4));
            }
            match at {
                None => self.library.list[k].state = State::Missing,
                Some(p) => {
                    let p = std::path::absolute(&p).unwrap_or(p);
                    if key(&p) != key(&r.path) {
                        self.point_sources(&r.path, &p);
                        self.library.list[k].path = p;
                        self.library.list[k].check();
                        if self.library.list[k].state == State::Ok {
                            self.library.list[k].state = State::Moved(r.path.clone());
                        }
                    } else {
                        self.library.list[k].check();
                    }
                }
            }
        }
    }

    /// SOURCERs reading `old` read `new` instead.
    fn point_sources(&mut self, old: &Path, new: &Path) {
        let k = key(old);
        for n in self.nodes.iter_mut() {
            if let Kind::Sourcer { input } = &mut n.kind {
                if Self::file_of(input).is_some_and(|p| key(&p) == k) {
                    *input = new.to_string_lossy().into_owned();
                }
            }
        }
    }

    /// Recording `id` is now at `new` (Locate…, a finished copy): its SOURCERs follow.
    pub(crate) fn move_recording(&mut self, id: &str, new: PathBuf, state: State) {
        let Some(k) = self.library.list.iter().position(|r| r.id == id) else { return };
        let old = self.library.list[k].path.clone();
        let new = std::path::absolute(&new).unwrap_or(new);
        self.point_sources(&old, &new);
        let r = &mut self.library.list[k];
        r.path = new;
        r.seen = None;
        r.state = state;
        r.check();
        self.save_project();
    }

    /// Takes the file's size and print as they are now (after a change the user accepts).
    pub(crate) fn accept_change(&mut self, id: &str) {
        if let Some(r) = self.library.list.iter_mut().find(|r| r.id == id) {
            if let (Some((size, modified)), Some(p)) = (stat(&r.path), print(&r.path)) {
                (r.size, r.modified, r.print, r.seen, r.state) = (size, modified, p, Some((size, modified)), State::Ok);
            }
        }
        self.save_project();
    }

    /// Copies recording `id` into the project's recordings\ folder on its own thread; when it is
    /// done the recording and its SOURCERs use the copy. False while untitled.
    pub(crate) fn copy_into_project(&mut self, id: &str) -> bool {
        let Some(dir) = self.project_dir() else {
            self.msg = "save the project first: copies go into its folder".into();
            return false;
        };
        let Some(r) = self.library.list.iter_mut().find(|r| r.id == id) else { return false };
        let into = dir.join("recordings");
        if r.path.starts_with(&into) || matches!(r.state, State::Copying | State::Missing) {
            return false;
        }
        let Some(name) = r.path.file_name().map(PathBuf::from) else { return false };
        let from = r.path.clone();
        let from_name = name.display().to_string();
        r.state = State::Copying;
        let done: CopyDone = Default::default();
        self.library.copies.push((id.to_string(), done.clone()));
        let job = move || -> Result<PathBuf, String> {
            std::fs::create_dir_all(&into).map_err(|e| e.to_string())?;
            let to = free_name(&into, &name);
            let part = to.with_extension(format!("{}.part", to.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
            std::fs::copy(&from, &part).map_err(|e| e.to_string())?;
            std::fs::rename(&part, &to).map_err(|e| e.to_string())?;
            Ok(to)
        };
        std::thread::spawn(move || {
            let res = job();
            *done.lock().unwrap() = Some(res);
        });
        self.msg = format!("copying {} into the project", from_name);
        true
    }

    /// Takes in copies that finished.
    fn copy_tick(&mut self) {
        let mut done = Vec::new();
        self.library.copies.retain(|(id, d)| match d.lock().unwrap().take() {
            Some(res) => {
                done.push((id.clone(), res));
                false
            }
            None => true,
        });
        for (id, res) in done {
            match res {
                Ok(to) => {
                    self.msg = format!("copied into {}", to.display());
                    self.move_recording(&id, to, State::Unchecked);
                }
                Err(e) => {
                    self.msg = format!("copy failed: {e}");
                    if let Some(r) = self.library.list.iter_mut().find(|r| r.id == id) {
                        r.state = State::Unchecked;
                        r.check();
                    }
                }
            }
        }
    }

    /// Whether copies are still running.
    pub(crate) fn copying(&self) -> bool {
        !self.library.copies.is_empty()
    }

    /// Takes recording `id` out of the project (only one no SOURCER reads).
    pub(crate) fn remove_recording(&mut self, id: &str) {
        if let Some(k) = self.library.list.iter().position(|r| r.id == id) {
            if self.rec_users(&self.library.list[k]).is_empty() {
                self.library.list.remove(k);
                self.save_project();
            }
        }
    }

    /// Adds a SOURCER on each file (near the canvas's top-left corner, a step further for each).
    pub(crate) fn add_recordings(&mut self, files: Vec<PathBuf>) {
        for f in files {
            let k = self.nodes.iter().filter(|n| !n.hidden).count() as f32;
            let at = ((Vec2::splat(60.0 + 30.0 * (k % 6.0)) - self.pan) / self.zoom).to_pos2();
            let n = self.add(Kind::Sourcer { input: f.to_string_lossy().into_owned() }, at);
            self.pick_only(n);
        }
        self.sync_recordings();
        self.save_project();
    }

    /// Top bar → Recordings: each of the project's files with what is known of it, and what can
    /// be done with it.
    pub(crate) fn recordings_menu(&mut self, ui: &mut egui::Ui) {
        let n = self.library.list.len();
        let bad = self.library.list.iter().filter(|r| matches!(r.state, State::Missing | State::Changed)).count();
        let title = if bad > 0 { format!("Recordings ({n}, {bad} !)") } else { format!("Recordings ({n})") };
        let r = ui.menu_button(title, |ui| {
            ui.set_min_width(460.0);
            let into = self.project_dir().map(|d| d.join("recordings"));
            let mut act: Option<(String, &'static str)> = None;
            if self.library.list.is_empty() {
                ui.colored_label(DIM, "none yet: a file in a SOURCER joins the project");
            }
            let mut pick: Option<NodeId> = None;
            for r in &mut self.library.list {
                if r.state == State::Unchecked {
                    r.check();
                }
            }
            for k in 0..self.library.list.len() {
                let r = self.library.list[k].clone();
                let users = self.rec_users(&r);
                let inside = into.as_ref().is_some_and(|d| r.path.starts_with(d));
                let (state, col) = match &r.state {
                    State::Ok | State::Unchecked => (if users.is_empty() { "not on the canvas".to_string() } else { format!("{} source{}", users.len(), if users.len() == 1 { "" } else { "s" }) }, if users.is_empty() { DIM } else { GOOD }),
                    State::Missing => ("missing".into(), WARN),
                    State::Changed => ("changed since added".into(), WARN),
                    State::Moved(from) => (format!("found elsewhere (was {})", from.display()), SEL),
                    State::Copying => ("copying…".into(), SEL),
                };
                ui.horizontal(|ui| {
                    let name = r.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let hover = format!("{}\n{} bytes · print {} · id {}{}", r.path.display(), r.size, r.print, r.id, if inside { "\nin the project folder" } else { "" });
                    if ui.add(egui::Button::new(egui::RichText::new(&name).color(TEXT)).frame(false)).on_hover_text(hover).clicked() {
                        pick = users.first().copied();
                    }
                    ui.colored_label(col, state);
                    if inside {
                        ui.colored_label(DIM, "in project");
                    }
                    if r.state == State::Missing && ui.small_button("Locate…").clicked() {
                        act = Some((r.id.clone(), "locate"));
                    }
                    if r.state == State::Changed && ui.small_button("Accept change").on_hover_text("keep the file as it is now: its new size and print").clicked() {
                        act = Some((r.id.clone(), "accept"));
                    }
                    if !inside && !matches!(r.state, State::Missing | State::Copying) {
                        let b = ui.add_enabled(into.is_some(), egui::Button::new("Copy into project").small());
                        if b.on_disabled_hover_text("save the project first").clicked() {
                            act = Some((r.id.clone(), "copy"));
                        }
                    }
                    if r.state != State::Missing && ui.small_button("Show").on_hover_text("show the file in Explorer").clicked() {
                        act = Some((r.id.clone(), "show"));
                    }
                    if users.is_empty() && ui.small_button("Remove").on_hover_text("take it out of the project (the file stays)").clicked() {
                        act = Some((r.id.clone(), "remove"));
                    }
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Add recordings…").clicked() {
                    act = Some((String::new(), "add"));
                }
                let outside = self.library.list.iter().any(|r| !into.as_ref().is_some_and(|d| r.path.starts_with(d)) && r.state != State::Missing);
                if outside && ui.add_enabled(into.is_some(), egui::Button::new("Copy all into project")).on_disabled_hover_text("save the project first").clicked() {
                    act = Some((String::new(), "copy all"));
                }
            });
            if let Some(i) = pick {
                self.pick_only(i);
                ui.close();
            }
            if let Some((id, what)) = act {
                match what {
                    "locate" => {
                        if let Some(p) = rfd::FileDialog::new().pick_file() {
                            self.move_recording(&id, p, State::Unchecked);
                        }
                    }
                    "accept" => self.accept_change(&id),
                    "copy" => {
                        self.copy_into_project(&id);
                    }
                    "copy all" => {
                        let ids: Vec<String> = self.library.list.iter().map(|r| r.id.clone()).collect();
                        for id in ids {
                            self.copy_into_project(&id);
                        }
                    }
                    "show" => {
                        if let Some(r) = self.library.get(&id) {
                            let _ = std::process::Command::new("explorer").arg(format!("/select,{}", r.path.display())).spawn();
                        }
                    }
                    "remove" => self.remove_recording(&id),
                    "add" => {
                        let files = rfd::FileDialog::new()
                            .add_filter("audio / video", &["mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "mp4", "mkv", "webm", "mov", "avi"])
                            .add_filter("any", &["*"])
                            .pick_files()
                            .unwrap_or_default();
                        self.add_recordings(files);
                        ui.close();
                    }
                    _ => {}
                }
            }
        });
        if bad > 0 {
            r.response.on_hover_text(format!("{bad} recording{} missing or changed", if bad == 1 { "" } else { "s" }));
        }
    }
}
