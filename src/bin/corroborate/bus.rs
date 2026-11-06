//! The data bus in the workbench. Every output of every node can be read as a value of its kind
//! (`corroborate::data::Data`) with `value_from`; any input of that kind can take it.
//!
//! - **Outboxes**: what a program sends on an output (`{"type":"out",…}`, protocol 2) is kept in
//!   its node's state, one value per output: lists (events, tracks) grow, the rest is replaced.
//! - **Built-in modules** are read through the same call: a location from `area_from`, a time from
//!   the node's UTC anchor, a plugin's findings and facts, a Value node's value.
//! - **Programs on protocol 2** (`Kind::Module`) start when the graph runs, with their settings as
//!   `--set name=value`; each wired input is sent at start and again whenever its value changes;
//!   audio comes from the source job when an audio output is wired in.
//! - **Settings** are the only way the GUI changes processing: each value is checked against the
//!   module.toml (type, range, choices) before it is used, every change is kept with its time for
//!   the result file, and the graph restarts half a second after the last change.

use super::*;
use corroborate::data::{Data, Event};
use corroborate::module_manifest::Manifest;
use corroborate::plugin::{self as protocol, Msg};
use std::io::{BufRead, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::SyncSender;
use std::sync::Mutex;

/// A running protocol-2 program.
pub struct Program {
    /// Lines for its stdin. The source job feeding it audio holds a clone.
    pub tx: SyncSender<String>,
    child: Arc<Mutex<Option<Child>>>,
    /// What each input was last given.
    sent: HashMap<&'static str, Data>,
}

impl Drop for Program {
    /// Stopping the graph stops the program.
    fn drop(&mut self) {
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// A setting changed on the canvas: kept for the result file (the evidence trail).
#[derive(Clone, Debug)]
pub struct SettingChange {
    pub utc: f64,
    /// The node's stable id.
    pub node: String,
    pub label: String,
    pub name: String,
    pub old: String,
    pub new: String,
}

/// Start module `m`'s program with `settings`. Its messages go into `st`: outputs into the
/// outbox, hello/status/log into the plugin part shown on the node.
/// `extra` are more arguments (the audio source's `--source`, `--file` or `--live`, `--clock`).
pub fn start(m: &Manifest, settings: &[(String, String)], st: Shared, dir: &std::path::Path, extra: &[String]) -> Result<Program, String> {
    let prog = m.program.as_ref().ok_or("its module.toml names no program")?;
    let path = nodes::program_path(m, prog);
    let mut args = extra.to_vec();
    for (k, v) in settings {
        let s = m.setting(k).ok_or(format!("no setting {k}"))?;
        args.extend(["--set".to_string(), format!("{k}={}", s.check(v)?)]);
    }
    {
        let mut s = st.lock().unwrap();
        s.reset();
        s.running = true;
        s.status = "running".into();
        s.plugin.state = "starting".into();
    }
    let spawned = corroborate::no_window(&mut Command::new(&path))
        .args(&args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let mut s = st.lock().unwrap();
            s.running = false;
            s.plugin.state = format!("could not start {}: {e}", path.display());
            return Err(s.plugin.state.clone());
        }
    };
    let (Some(mut stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return Err("no pipes to the program".into());
    };
    let child = Arc::new(Mutex::new(Some(child)));
    let (tx, rx) = std::sync::mpsc::sync_channel::<String>(64);
    std::thread::spawn(move || {
        for line in rx {
            if writeln!(stdin, "{line}").and_then(|_| stdin.flush()).is_err() {
                break;
            }
        }
    });
    let outputs = m.outputs.clone();
    {
        let (st, child) = (st.clone(), child.clone());
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                match protocol::parse_line(&line) {
                    Some(Msg::Out { port, value }) => {
                        let mut s = st.lock().unwrap();
                        let Some(p) = outputs.iter().find(|p| p.name == port) else {
                            s.plugin.log = format!("sent on \"{port}\", which its module.toml does not declare");
                            continue;
                        };
                        match Data::from_json(p.kind, &value) {
                            Ok(d) => {
                                match s.outbox.iter_mut().find(|(n, _)| *n == p.name) {
                                    Some((_, old)) => old.merge(d),
                                    None => s.outbox.push((p.name, d)),
                                }
                                s.outbox_rev += 1;
                            }
                            Err(e) => s.plugin.log = format!("output {port}: {e}"),
                        }
                    }
                    Some(m) => st.lock().unwrap().plugin.apply(m),
                    None => {}
                }
            }
            // Its output closed: it has ended (or been stopped).
            let ended = child.lock().unwrap().take().map(|mut c| c.wait());
            let mut s = st.lock().unwrap();
            s.running = false;
            s.done = true;
            if let Some(r) = ended {
                s.plugin.state = match r {
                    Ok(x) if x.success() => "finished".into(),
                    Ok(x) => format!("stopped ({x})"),
                    Err(e) => format!("stopped ({e})"),
                };
            }
        });
    }
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                st.lock().unwrap().plugin.log = line;
            }
        }
    });
    Ok(Program { tx, child, sent: HashMap::new() })
}

impl App {
    /// The value on output `out` of node `from`, or why there is none (yet).
    pub(crate) fn value_from(&self, from: NodeId, out: &str) -> Result<Data, String> {
        let kind = self.graph.nodes()[from].spec.outputs.iter().find(|p| p.name == out).map(|p| p.kind).ok_or(format!("{} has no output {out}", self.nodes[from].label))?;
        // Times a program sent without a clock are on the clock of the audio it heard.
        let clock = self.clock_of(from).unwrap_or_default();
        {
            let s = self.nodes[from].state.lock().unwrap();
            if let Some((_, d)) = s.outbox.iter().find(|(n, _)| *n == out) {
                let mut d = d.clone();
                match &mut d {
                    Data::Events(v) => v.iter_mut().filter(|e| e.clock.is_empty()).for_each(|e| e.clock = clock.clone()),
                    Data::Tracks(v) => v.iter_mut().filter(|p| p.clock.is_empty()).for_each(|p| p.clock = clock.clone()),
                    _ => {}
                }
                return Ok(d);
            }
        }
        let s = self.nodes[from].state.lock().unwrap();
        match (kind, &self.nodes[from].kind) {
            (_, Kind::Value { ty, rows }) => corroborate::modules::value::data(ty, rows),
            (PortKind::Location, _) => {
                drop(s);
                self.area_from(from).map(|(area, note)| Data::Location { area, note, site: None })
            }
            (_, Kind::Enf { .. }) => crate::enf_node::value(&s, out, &clock),
            (PortKind::Time, _) => s
                .anchor
                .as_ref()
                .map(|a| Data::Time { clock: clock.clone(), t: 0.0, utc: Some(a.utc0), error_s: a.error_s, source: a.source.clone() })
                .ok_or("no time yet".into()),
            (PortKind::Event, Kind::Plugin { .. }) => Ok(Data::Events(
                s.plugin.findings.iter().map(|f| Event { clock: clock.clone(), t_start: f.t_start, t_end: f.t_end, label: f.label.clone(), detail: f.detail.clone(), error_s: f.error_s }).collect(),
            )),
            // Planes' passes: each is its closest approach, with the fit's error of that time.
            (PortKind::Event, Kind::Planes) => Ok(Data::Events(
                s.planes
                    .passes
                    .iter()
                    .map(|e| Event {
                        clock: clock.clone(),
                        t_start: e.t_cpa,
                        t_end: e.t_cpa,
                        label: format!("pass #{}", e.id),
                        detail: format!("closest approach; heard {:.1}–{:.1} s, {:.0} m/s, {:.2} km, f0 {:.1} Hz", e.t_start, e.t_end, e.speed, e.dist / 1000.0, e.f0),
                        error_s: e.t_err,
                    })
                    .collect(),
            )),
            (PortKind::Json, Kind::Plugin { .. }) => Ok(Data::Json(json_object(&s.plugin.meta))),
            (PortKind::Audio | PortKind::Video, _) => Err("a stream, not a value".into()),
            _ => Err("nothing on this output yet".into()),
        }
    }

    /// Why output `out` of node `i` gives nothing although it is built: the node's signal is
    /// below what the output's use case needs (module.toml `min_quality`). None: not held.
    pub(crate) fn output_held(&self, i: NodeId, out: &str) -> Option<String> {
        match &self.nodes[i].kind {
            Kind::Enf { .. } => crate::enf_node::held(&*self.nodes[i].state.lock().ok()?, out),
            _ => None,
        }
    }

    /// What a dot's tooltip says: its type, what the type is, and the value on it now.
    pub(crate) fn port_hover(&self, i: NodeId, p: &corroborate::core::Port, out: bool) -> String {
        let fed = if out { None } else { self.graph.feeder(i, p.name) };
        let kind = match fed {
            Some((f, o)) => self.graph.nodes()[f].spec.outputs.iter().find(|x| x.name == o).map_or(p.kind, |x| x.kind),
            None => p.kind,
        };
        let t = corroborate::types::of(kind);
        let need = if out { "output" } else if p.required { "input, needed to run" } else { "input, optional" };
        let now = match (out, fed) {
            (true, _) => self.value_from(i, p.name),
            (false, Some((f, o))) => self.value_from(f, o),
            (false, None) => Err("not wired".into()),
        };
        let now = match now {
            Ok(d) => {
                let s = summary(&d);
                if s.chars().count() > 200 { format!("{}…", s.chars().take(199).collect::<String>()) } else { s }
            }
            Err(e) => e,
        };
        // What the output is for, as its module.toml says.
        let about = match self.manifest_of(i).and_then(|m| m.sockets.iter().find(|s| s.name == p.name).cloned()) {
            Some(s) if out => format!("\n{}", layout::socket_text(&s)),
            _ => String::new(),
        };
        format!("{} · {} ({}, {need})\n{}{about}\nnow: {now}", p.name, t.label, t.flow.name(), t.description)
    }

    /// A location a program sent (the area of a node whose module has no Rust code).
    pub(crate) fn outbox_area(&self, from: NodeId) -> Result<(Arc<Area>, String), String> {
        let s = self.nodes[from].state.lock().unwrap();
        s.outbox
            .iter()
            .find_map(|(_, d)| match d {
                Data::Location { area, note, .. } => Some((area.clone(), note.clone())),
                _ => None,
            })
            .ok_or("no location from it yet".into())
    }

    /// Start every protocol-2 module's program (the graph is starting).
    pub(crate) fn start_programs(&mut self) {
        self.programs.clear();
        let dir = results_dir();
        for i in 0..self.nodes.len() {
            let n = &self.nodes[i];
            let Kind::Module { m, settings } = &n.kind else { continue };
            if n.hidden {
                continue;
            }
            // The audio's source, for programs that read its files or need its clock.
            let mut extra: Vec<String> = Vec::new();
            if let Some((f, _)) = self.audio_source(i) {
                if let Kind::Sourcer { input } = &self.nodes[f].kind {
                    if let Some(inp) = sourcer_node::input(input) {
                        extra.extend(["--source".to_string(), inp.key(), if inp.live() { "--live" } else { "--file" }.to_string()]);
                    }
                }
                if let Some(c) = self.clock_of(f) {
                    extra.extend(["--clock".to_string(), c]);
                }
            }
            if let Ok(p) = start(m, settings, n.state.clone(), &dir, &extra) {
                self.programs.insert(i, p);
            }
        }
        self.bus_sent = Instant::now() - Duration::from_secs(1);
        self.feed_programs();
    }

    /// What feeds node `i`'s audio input: (node, output).
    pub(crate) fn audio_source(&self, i: NodeId) -> Option<(NodeId, &'static str)> {
        let p = self.graph.nodes()[i].spec.inputs.iter().find(|p| p.kind == PortKind::Audio)?;
        self.graph.feeder(i, p.name)
    }

    /// Where source `src`'s audio goes on the bus: the programs wired to its audio output.
    pub(crate) fn audio_buses(&self, src: NodeId) -> Vec<SyncSender<String>> {
        self.graph.targets(src, "audio").iter().filter_map(|(t, _)| self.programs.get(t).map(|p| p.tx.clone())).collect()
    }

    /// Send each program the inputs whose value changed (at most ten times a second).
    pub(crate) fn feed_programs(&mut self) {
        if self.programs.is_empty() || self.bus_sent.elapsed() < Duration::from_millis(100) {
            return;
        }
        self.bus_sent = Instant::now();
        // A program is live while the audio it hears is.
        for &i in self.programs.keys() {
            if let Some((f, _)) = self.audio_source(i) {
                let live = self.nodes[f].state.lock().unwrap().live;
                self.nodes[i].state.lock().unwrap().live = live;
            }
        }
        let mut send: Vec<(NodeId, &'static str, Data)> = Vec::new();
        for (&i, p) in &self.programs {
            for port in &self.graph.nodes()[i].spec.inputs {
                if matches!(port.kind, PortKind::Audio | PortKind::Video) {
                    continue;
                }
                let Some((from, out)) = self.graph.feeder(i, port.name) else { continue };
                if let Ok(d) = self.value_from(from, out) {
                    if p.sent.get(port.name) != Some(&d) {
                        send.push((i, port.name, d));
                    }
                }
            }
        }
        for (i, port, d) in send {
            if let Some(p) = self.programs.get_mut(&i) {
                if p.tx.try_send(protocol::input(port, &d.to_json())).is_ok() {
                    p.sent.insert(port, d);
                }
            }
        }
    }

    /// Set a module's setting from the canvas: checked against its module.toml first; a refused
    /// value changes nothing. A change is kept for the result file and saved with the project.
    pub(crate) fn set_setting(&mut self, i: NodeId, name: &str, value: &str) -> Result<(), String> {
        let label = self.nodes[i].label.clone();
        let m = self.manifest_of(i).ok_or(format!("{label} has no settings"))?;
        let v = m.setting(name).ok_or(format!("{} has no setting {name}", m.label))?.check(value)?;
        let old = self.nodes[i].kind.settings().into_iter().find(|(k, _)| k == name).map(|(_, x)| x).unwrap_or_default();
        if old == v {
            return Ok(());
        }
        self.nodes[i].kind.put_setting(name, v.clone());
        self.changes.push(SettingChange { utc: timebase::now_unix(), node: self.nodes[i].uid.clone(), label, name: name.to_string(), old, new: v });
        self.save_project();
        Ok(())
    }

    /// The module's part of the result file: its program, settings and what each output holds.
    pub(crate) fn module_json(&self, i: NodeId) -> String {
        let Kind::Module { m, settings } = &self.nodes[i].kind else { return String::new() };
        let s = self.nodes[i].state.lock().unwrap();
        let outs: Vec<String> = s.outbox.iter().map(|(n, d)| format!("\"{}\":{}", json_escape(n), d.to_json())).collect();
        format!(
            ",\"module\":\"{}\",\"program\":\"{}\",\"state\":\"{}\",\"settings\":{},\"outputs\":{{{}}}",
            m.id,
            json_escape(m.program.as_deref().unwrap_or("")),
            json_escape(&s.plugin.state),
            json_object(settings),
            outs.join(",")
        )
    }

    /// Changes made to the settings of nodes `ids`, oldest first, for the result file.
    pub(crate) fn changes_json(&self, ids: &[NodeId]) -> String {
        let uids: Vec<&str> = ids.iter().map(|&i| self.nodes[i].uid.as_str()).collect();
        let c: Vec<String> = self
            .changes
            .iter()
            .filter(|c| uids.contains(&c.node.as_str()))
            .map(|c| {
                format!(
                    "{{\"utc\":\"{}\",\"node\":\"{}\",\"label\":\"{}\",\"setting\":\"{}\",\"old\":\"{}\",\"new\":\"{}\"}}",
                    timebase::fmt_utc(c.utc),
                    c.node,
                    json_escape(&c.label),
                    json_escape(&c.name),
                    json_escape(&c.old),
                    json_escape(&c.new)
                )
            })
            .collect();
        format!("[{}]", c.join(","))
    }

    /// One editor per declared setting: a checkbox, a list of choices, or a text box whose value
    /// is used once it passes its checks (until then it is shown in red with the reason).
    fn settings_ui(&mut self, ui: &mut egui::Ui, i: NodeId) {
        let Kind::Module { m, settings } = &self.nodes[i].kind else { return };
        let (m, settings) = (m.clone(), settings.clone());
        let mut set: Option<(String, String)> = None;
        for s in &m.settings {
            let cur = settings.iter().find(|(k, _)| *k == s.name).map_or(s.default.clone(), |(_, v)| v.clone());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&s.name).small().color(DIM));
                match &s.ty {
                    SettingType::Bool => {
                        let mut b = cur == "true";
                        if ui.checkbox(&mut b, "").changed() {
                            set = Some((s.name.clone(), b.to_string()));
                        }
                    }
                    SettingType::Choice(choices) => {
                        egui::ComboBox::from_id_salt(("setting", i, &s.name)).selected_text(&cur).show_ui(ui, |ui| {
                            for c in choices {
                                if ui.selectable_label(*c == cur, c).clicked() {
                                    set = Some((s.name.clone(), c.clone()));
                                }
                            }
                        });
                    }
                    _ => {
                        let key = (i, s.name.clone());
                        let bad = self.setting_errors.contains_key(&key);
                        let draft = self.drafts.entry(key).or_insert_with(|| cur.clone());
                        let mut edit = egui::TextEdit::singleline(draft).desired_width(ui.available_width());
                        if bad {
                            edit = edit.text_color(WARN);
                        }
                        if ui.add(edit).changed() {
                            set = Some((s.name.clone(), draft.clone()));
                        }
                    }
                }
            });
            if let Some(e) = self.setting_errors.get(&(i, s.name.clone())) {
                look::line(ui, e, WARN);
            }
        }
        if let Some((name, v)) = set {
            match self.set_setting(i, &name, &v) {
                Ok(()) => {
                    self.setting_errors.remove(&(i, name));
                }
                Err(e) => {
                    self.setting_errors.insert((i, name), e);
                }
            }
        }
    }
}

/// `{"key":"value",…}`
fn json_object(kv: &[(String, String)]) -> String {
    let v: Vec<String> = kv.iter().map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v))).collect();
    format!("{{{}}}", v.join(","))
}

/// One line about an output's value, for the node.
pub(crate) fn summary(d: &Data) -> String {
    match d {
        Data::Time { utc: Some(u), error_s, .. } => format!("{} ± {error_s:.1} s", timebase::fmt_utc(*u)),
        Data::Time { clock, t, error_s, .. } => format!("{t:+.1} s on {clock} ± {error_s:.1} s"),
        Data::Location { area, note, .. } => format!("{} {note}", map_node::area_status(area)),
        Data::Events(v) => match v.last() {
            Some(e) => format!("{} events, last: {}", v.len(), e.label),
            None => "no events".into(),
        },
        Data::Tracks(v) => format!("{} points", v.len()),
        Data::Text(t) | Data::File(t) => t.clone(),
        Data::Json(j) | Data::Custom { json: j, .. } => j.clone(),
    }
}

/// A module on the data bus: its settings, then a line per output with what it holds, and the
/// program's state and last log line.
pub struct Look;

impl look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, standard: Vec2) -> Vec2 {
        let Kind::Module { m, .. } = &app.nodes[i].kind else { return standard };
        let lines = m.settings.len() as f32 * 24.0 + (m.outputs.len() + 2) as f32 * 14.0;
        Vec2::new(280.0, standard.y.max(HEAD + 8.0 + lines + 26.0))
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = look::content(app, ui, i);
        look::area(ui, r, |ui| {
            app.settings_ui(ui, i);
            let Kind::Module { m, .. } = &app.nodes[i].kind else { return };
            let s = app.nodes[i].state.lock().unwrap();
            for p in &m.outputs {
                match s.outbox.iter().find(|(n, _)| *n == p.name) {
                    Some((_, d)) => look::line(ui, format!("{}: {}", p.name, summary(d)), TEXT),
                    None => look::line(ui, format!("{}: —", p.name), DIM),
                }
            }
            if !s.plugin.log.is_empty() {
                look::line(ui, format!("log: {}", s.plugin.log), DIM);
            }
        });
    }
}

use corroborate::module_manifest::SettingType;
