//! The Plugin node in the workbench: starting the program and feeding it audio, what it reports,
//! its window, canvas status, timeline marks and its part of the result file.
//! Compiled into the `corroborate` program (see `src/bin/corroborate/main.rs`).

use super::*;
use corroborate::modules::plugin::BUNDLED;
use corroborate::plugin::{self as protocol, PluginFinding};
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{SyncSender, TrySendError};

/// What a plugin has reported so far.
#[derive(Default, Clone)]
pub struct PluginState {
    pub name: String,
    pub about: String,
    pub color: Option<[u8; 3]>,
    pub status: String,
    /// Where the process stands: starting, running, finished, or what went wrong.
    pub state: String,
    /// Last line on stderr.
    pub log: String,
    pub tracks: Vec<(String, Vec<(f64, f64, bool)>)>,
    pub findings: Vec<PluginFinding>,
    pub meta: Vec<(String, String)>,
    /// Live audio the plugin could not keep up with, seconds.
    pub dropped_s: f64,
}

impl PluginState {
    fn apply(&mut self, m: protocol::Msg) {
        match m {
            protocol::Msg::Hello { name, about, color } => {
                if !name.is_empty() {
                    self.name = name;
                }
                self.about = about;
                self.color = color;
                self.state = "running".into();
            }
            protocol::Msg::Status(t) => self.status = t,
            protocol::Msg::View { .. } => {}
            protocol::Msg::Track { id, t, hz, ok } => match self.tracks.iter_mut().find(|(i, _)| *i == id) {
                Some((_, v)) => v.push((t, hz, ok)),
                None => self.tracks.push((id, vec![(t, hz, ok)])),
            },
            protocol::Msg::Finding(f) => self.findings.push(f),
            protocol::Msg::Meta { key, value } => match self.meta.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => *v = value,
                None => self.meta.push((key, value)),
            },
        }
    }
}

/// A plugin node in a run.
pub struct PluginJob {
    pub path: String,
    pub state: Shared,
    /// Manual data rows from a Manual node wired into the plugin.
    pub manual: Vec<(String, String)>,
}

/// Programs next to this one that speak the plugin protocol (bundled plugins).
pub fn bundled() -> Vec<String> {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) else {
        return Vec::new();
    };
    let mut found: Vec<String> = BUNDLED
        .iter()
        .map(|(exe, _)| dir.join(format!("{exe}{}", std::env::consts::EXE_SUFFIX)))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

/// Start the plugin on a source with the source's arguments plus the Manual data wired into it.
/// Returns the channel feeding its stdin.
pub fn start(p: &PluginJob, source_args: &[String], dir: &std::path::Path) -> Option<SyncSender<Vec<f32>>> {
    let mut a = source_args.to_vec();
    for (k, v) in p.manual.iter().filter(|(k, v)| !k.trim().is_empty() && !v.trim().is_empty()) {
        a.extend(["--manual".to_string(), format!("{}={}", k.trim(), v.trim())]);
    }
    {
        let mut s = p.state.lock().unwrap();
        s.plugin.state = "starting".into();
        s.status = "running".into();
    }
    spawn(&p.path, p.state.clone(), &a, dir)
}

/// Feed one block of audio. A file waits for the plugin; live audio never does, and what the
/// plugin misses is counted. Returns false once the plugin has gone.
pub fn feed(tx: &SyncSender<Vec<f32>>, x: &[f32], live: bool, st: &Shared) -> bool {
    if live {
        match tx.try_send(x.to_vec()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                st.lock().unwrap().plugin.dropped_s += x.len() as f64 / corroborate::INPUT_SR;
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    } else {
        tx.send(x.to_vec()).is_ok()
    }
}

/// Start a plugin program. Threads read its messages (stdout) and its last log line (stderr)
/// into the node's state.
fn spawn(path: &str, st: Shared, args: &[String], dir: &std::path::Path) -> Option<SyncSender<Vec<f32>>> {
    let started = corroborate::no_window(&mut Command::new(path))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match started {
        Ok(c) => c,
        Err(e) => {
            st.lock().unwrap().plugin.state = format!("could not start: {e}");
            return None;
        }
    };
    let (Some(mut stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return None;
    };
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(64);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        for x in rx {
            bytes.clear();
            for v in x {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            if stdin.write_all(&bytes).is_err() {
                break;
            }
        }
    });
    {
        let st = st.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(m) = protocol::parse_line(&line) {
                    st.lock().unwrap().plugin.apply(m);
                }
            }
            let state = match child.wait() {
                Ok(s) if s.success() => "finished".to_string(),
                Ok(s) => format!("stopped ({s})"),
                Err(e) => format!("stopped ({e})"),
            };
            st.lock().unwrap().plugin.state = state;
        });
    }
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                st.lock().unwrap().plugin.log = line;
            }
        }
    });
    Some(tx)
}

/// The canvas status line: the plugin's own first status line, else where its process stands.
pub fn status(s: &NodeState) -> Option<String> {
    if s.plugin.state.is_empty() {
        return None;
    }
    let first = s.plugin.status.lines().next().unwrap_or("");
    Some(if first.is_empty() { s.plugin.state.clone() } else { first.to_string() })
}

pub fn window(ui: &mut egui::Ui, path: &str, st: &Shared) {
    ui.colored_label(DIM, format!("program: {path}"));
    let pl = st.lock().unwrap().plugin.clone();
    ui.label(format!("{} · {}", if pl.name.is_empty() { "plugin" } else { &pl.name }, pl.state));
    if !pl.about.is_empty() {
        ui.colored_label(DIM, &pl.about);
    }
    if pl.dropped_s > 0.0 {
        ui.colored_label(WARN, format!("missed {:.0} s of live audio (could not keep up)", pl.dropped_s));
    }
    if !pl.status.is_empty() {
        ui.separator();
        for line in pl.status.lines() {
            ui.label(line);
        }
    }
    if !pl.log.is_empty() {
        ui.colored_label(DIM, format!("log: {}", pl.log));
    }
    if !pl.tracks.is_empty() {
        let c = pl.color.map_or(Color32::WHITE, |[r, g, b]| Color32::from_rgb(r, g, b));
        let all: Vec<(f64, f64, bool)> = pl.tracks.iter().flat_map(|(_, v)| v.iter().copied()).collect();
        ui.colored_label(DIM, format!("{} tracks (also drawn on the Planes spectrogram of the same source)", pl.tracks.len()));
        plot_series(ui, &[(c, all)], 140.0);
    }
    if !pl.findings.is_empty() {
        ui.separator();
        ui.label(egui::RichText::new(format!("FINDINGS ({})", pl.findings.len())).small().color(DIM));
        for f in pl.findings.iter().rev().take(40) {
            ui.label(format!("{:.0}–{:.0} s · {}", f.t_start, f.t_end, f.label)).on_hover_text(&f.detail);
        }
    }
    if !pl.meta.is_empty() {
        ui.separator();
        ui.label(egui::RichText::new("FACTS").small().color(DIM));
        egui::Grid::new(ui.next_auto_id()).num_columns(2).striped(true).show(ui, |ui| {
            for (k, v) in &pl.meta {
                ui.colored_label(DIM, k);
                ui.label(v);
                ui.end_row();
            }
        });
    }
}

pub fn marks(s: &NodeState) -> Vec<Mark> {
    let (to, err) = clock(&s.anchor);
    let col = s.plugin.color.map_or(Color32::from_rgb(200, 200, 200), |[r, g, b]| Color32::from_rgb(r, g, b));
    s.plugin
        .findings
        .iter()
        .map(|f| Mark { t0: to(f.t_start), t1: to(f.t_end), err, label: f.label.clone(), detail: f.detail.clone(), col })
        .collect()
}

pub fn json(path: &str, s: &NodeState) -> String {
    let anchor = s.anchor.as_ref();
    let f: Vec<String> = s
        .plugin
        .findings
        .iter()
        .map(|f| {
            let utc = anchor.map_or("null".to_string(), |a| format!("\"{}\"", timebase::fmt_utc(a.utc(f.t_start))));
            format!(
                "{{\"t_start\":{:.2},\"t_end\":{:.2},\"utc_start\":{utc},\"label\":\"{}\",\"detail\":\"{}\"}}",
                f.t_start,
                f.t_end,
                json_escape(&f.label),
                json_escape(&f.detail)
            )
        })
        .collect();
    let m: Vec<String> = s.plugin.meta.iter().map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v))).collect();
    format!(
        ",\"program\":\"{}\",\"state\":\"{}\",\"plugin_status\":\"{}\",\"dropped_s\":{:.1},\"findings\":[{}],\"facts\":{{{}}}",
        json_escape(path),
        json_escape(&s.plugin.state),
        json_escape(&s.plugin.status),
        s.plugin.dropped_s,
        f.join(","),
        m.join(",")
    )
}

impl App {
    /// What a plugin node needs for a run: its program and the Manual data wired into it.
    pub(crate) fn plugin_job(&self, i: NodeId, path: &str) -> PluginJob {
        PluginJob { path: path.to_string(), state: self.nodes[i].state.clone(), manual: self.manual_rows_into(i) }
    }
}

/// How this node looks on the canvas: the standard look. Override `NodeLook` methods here to
/// change it (see `src/bin/corroborate/look.rs`).
pub struct Look;

impl crate::look::NodeLook for Look {}
