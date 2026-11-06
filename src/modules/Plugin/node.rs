//! The Plugin node in the workbench: starting the program and feeding it audio, what it reports,
//! what it shows on the canvas, its status line, timeline marks and its part of the result file.
//! Compiled into the `corroborate` program (see `src/bin/corroborate/main.rs`).

use super::*;
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
    pub(crate) fn apply(&mut self, m: protocol::Msg) {
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
            // Values on the data bus: kept by the bus (protocol 2), not here.
            protocol::Msg::Out { .. } => {}
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
    /// What Value nodes wired into the plugin hold (name, value).
    pub manual: Vec<(String, String)>,
}

/// Start the plugin on a source with the source's arguments plus the values wired into it.
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
    /// What a plugin node needs for a run: its program and the values wired into it.
    pub(crate) fn plugin_job(&self, i: NodeId, path: &str) -> PluginJob {
        PluginJob { path: path.to_string(), state: self.nodes[i].state.clone(), manual: self.value_args_into(i) }
    }
}

/// Lines of the plugin's own status shown on the canvas.
const LINES: usize = 5;

/// The Plugin node on the canvas: its name and state, what it is about, missed live audio, its
/// own status lines, and how many findings, facts and tracks it gave (findings are on the
/// timeline and in the result file; tracks on the Planes spectrogram of the same source).
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, _app: &App, _i: NodeId, standard: Vec2) -> Vec2 {
        Vec2::new(300.0, standard.y.max(HEAD + 4.0 + (LINES + 3) as f32 * 13.0 + 28.0))
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let pl = app.nodes[i].state.lock().unwrap().plugin.clone();
        crate::look::area(ui, r, |ui| {
            let name = if pl.name.is_empty() { "plugin" } else { &pl.name };
            let state = if pl.state.is_empty() { "not run" } else { &pl.state };
            crate::look::line(ui, format!("{name} · {state}"), TEXT);
            if !pl.about.is_empty() {
                crate::look::line(ui, &pl.about, DIM);
            }
            if pl.dropped_s > 0.0 {
                crate::look::line(ui, format!("missed {:.0} s of live audio (could not keep up)", pl.dropped_s), WARN);
            }
            for l in pl.status.lines().take(LINES) {
                crate::look::line(ui, l, TEXT);
            }
            if !pl.findings.is_empty() || !pl.meta.is_empty() || !pl.tracks.is_empty() {
                crate::look::line(ui, format!("{} findings · {} facts · {} tracks", pl.findings.len(), pl.meta.len(), pl.tracks.len()), DIM);
            }
            if !pl.log.is_empty() {
                crate::look::line(ui, format!("log: {}", pl.log), DIM);
            }
        });
    }
}
