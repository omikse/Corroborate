//! Drives the workbench headless: no window opens. A test builds a `Drive`, adds nodes, clicks
//! and types by label like a user would, lets the graph run for real (ffmpeg, yt-dlp, plugins),
//! and saves what the screen shows as PNG in `target/drive/`.
//!
//! Run all scenarios:   cargo test --bin corroborate drive -- --nocapture
//! Run one:             cargo test --bin corroborate drive::sourcer -- --nocapture
//!
//! `dump()` prints every widget the screen exposes (role, label, value), which is what `click`
//! and `type_into` find things by. Tests run with no project file: nothing of yours is read or
//! written (see `project_file`).

use super::*;
use egui::accesskit::Role as A11y;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;

pub struct Drive {
    pub h: Harness<'static, App>,
}

/// Where screenshots and scenario files go.
pub fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("drive");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Tests run at the same time; the graphics device is set up and rendered to one at a time.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

// A toolkit: scenarios use what they need.
#[allow(dead_code)]
impl Drive {
    /// An empty workbench, 1400 × 780 like the real window.
    pub fn new() -> Self {
        Self::with(App::empty())
    }

    pub fn with(app: App) -> Self {
        let gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
        let h = Harness::builder().with_size(Vec2::new(1400.0, 780.0)).wgpu().build_ui_state(
            |ui, app: &mut App| {
                ui.ctx().set_visuals(egui::Visuals::dark());
                app.frame(ui);
            },
            app,
        );
        drop(gpu);
        let mut d = Self { h };
        d.frames(3);
        d
    }

    pub fn app(&mut self) -> &mut App {
        self.h.state_mut()
    }

    /// Steps `n` frames at once (no real time passes).
    pub fn frames(&mut self, n: usize) {
        self.h.run_steps(n);
    }

    /// Steps frames for `secs` of real time, so background jobs (decoding, streams, plugins)
    /// get on with their work between frames.
    pub fn wait(&mut self, secs: f64) {
        let end = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < end {
            self.h.step();
            std::thread::sleep(Duration::from_millis(50));
        }
        self.frames(2);
    }

    /// Steps until `done(app)` holds or `secs` pass; returns whether it held.
    pub fn wait_until(&mut self, secs: f64, mut done: impl FnMut(&App) -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < end {
            self.h.step();
            if done(self.h.state()) {
                self.frames(2);
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// Clicks the widget whose label is exactly `label` (the first if there are several).
    pub fn click(&mut self, label: &str) {
        self.h.get_all_by_label(label).next().unwrap().click();
        self.frames(2);
    }

    /// Clicks the first widget whose label contains `part`.
    pub fn click_contains(&mut self, part: &str) {
        self.h.get_all_by_label_contains(part).next().unwrap().click();
        self.frames(2);
    }

    /// Types into the `n`-th text box on screen (top to bottom as egui lays them out), replacing
    /// what was there.
    pub fn type_into(&mut self, n: usize, text: &str) {
        let boxes: Vec<_> = self.h.query_all_by_role(A11y::TextInput).collect();
        let b = boxes.get(n).unwrap_or_else(|| panic!("text box {n}: only {} on screen", boxes.len()));
        b.focus();
        self.h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        self.h.get_all_by_role(A11y::TextInput).nth(n).unwrap().type_text(text);
        self.frames(2);
    }

    /// Clicks at a point on screen (canvas coordinates as shown in the PNG).
    pub fn click_at(&mut self, x: f32, y: f32) {
        let p = Pos2::new(x, y);
        self.h.hover_at(p);
        self.h.drag_at(p);
        self.h.drop_at(p);
        self.frames(2);
    }

    /// Right-clicks at a point on screen (opens a context menu).
    pub fn right_click_at(&mut self, x: f32, y: f32) {
        let pos = Pos2::new(x, y);
        self.h.hover_at(pos);
        for pressed in [true, false] {
            self.h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
            self.frames(1);
        }
        self.frames(2);
    }

    /// Where node `i`'s title bar is on screen.
    pub fn node_title(&mut self, i: NodeId) -> Pos2 {
        let app = self.app();
        let r = app.node_rect(app.canvas_origin, i);
        Pos2::new(r.left() + 40.0, r.top() + 10.0)
    }

    /// Every widget the screen exposes: role, label, value and where it is.
    pub fn dump(&mut self) -> String {
        let mut s = String::new();
        for n in self.h.query_all_by(|_| true) {
            let a = n.accesskit_node();
            let (label, value) = (a.label().unwrap_or_default(), a.value().unwrap_or_default());
            if label.is_empty() && value.is_empty() && a.role() != A11y::TextInput {
                continue;
            }
            let r = n.rect();
            s.push_str(&format!(
                "{:?} \"{label}\"{} @ {:.0},{:.0} {:.0}x{:.0}\n",
                a.role(),
                if value.is_empty() { String::new() } else { format!(" = \"{value}\"") },
                r.min.x,
                r.min.y,
                r.width(),
                r.height()
            ));
        }
        s
    }

    /// Saves what the screen shows as `target/drive/<name>.png` and prints the path.
    pub fn shot(&mut self, name: &str) -> PathBuf {
        let img = {
            let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
            self.h.render().expect("render (wgpu)")
        };
        let p = out_dir().join(format!("{name}.png"));
        img.save(&p).expect("save png");
        println!("shot: {}", p.display());
        p
    }

    /// The status line under the top bar.
    pub fn msg(&mut self) -> String {
        self.app().msg.clone()
    }
}

/// A 3 s 440 Hz tone with 50 Hz hum, as a WAV file in target/drive/.
/// A Value node of type location: a circle of `radius_km` around a point.
pub fn place(lat: f64, lon: f64, radius_km: f64) -> Kind {
    let mut rows = corroborate::modules::value::defaults("location");
    for (k, v) in [("lat", lat), ("lon", lon), ("radius_km", radius_km)] {
        rows.iter_mut().find(|(q, _)| q == k).unwrap().1 = v.to_string();
    }
    Kind::Value { ty: "location".into(), rows }
}

pub fn tone_wav() -> PathBuf {
    let p = out_dir().join("tone.wav");
    let spec = hound::WavSpec { channels: 1, sample_rate: 16000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(&p, spec).unwrap();
    for i in 0..16000 * 3 {
        let t = i as f64 / 16000.0;
        let v = 0.4 * (2.0 * std::f64::consts::PI * 440.0 * t).sin() + 0.1 * (2.0 * std::f64::consts::PI * 50.0 * t).sin();
        w.write_sample((v * 32767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
    p
}

// ---------------------------------------------------------------- scenarios

/// SOURCER: a new node is two boxes with its outputs hidden (look.rhai); typing a file shows
/// audio, time and metadata (no video) and a waveform.
#[test]
fn sourcer() {
    let mut d = Drive::new();
    let s = d.app().add(Kind::Sourcer { input: String::new() }, Pos2::new(40.0, 40.0));
    d.frames(3);
    let shown = |a: &App| -> Vec<&'static str> {
        a.graph.nodes()[s].spec.outputs.iter().filter(|p| a.port_shown(s, p, true, &a.port_look(s, p))).map(|p| p.name).collect()
    };
    assert!(shown(d.app()).is_empty(), "a new SOURCER shows no outputs");
    println!("{}", d.dump());
    d.shot("sourcer_new");

    let wav = tone_wav();
    d.type_into(0, &wav.to_string_lossy());
    let ok = d.wait_until(30.0, |app| app.nodes[s].state.lock().map(|st| st.audio_s > 2.0).unwrap_or(false));
    d.shot("sourcer_file");
    assert_eq!(shown(d.app()), ["audio", "time", "metadata"], "outputs appear with a source");
    assert!(ok, "the file decoded");
}

/// One of every node kind; then the canvas editors: a Value field typed in, Combine switched
/// to OR (its look.rhai's list), which is a logged setting change.
#[test]
fn all_nodes() {
    let mut d = Drive::new();
    let a = d.app();
    a.add(Kind::Sourcer { input: String::new() }, Pos2::new(30.0, 40.0));
    a.add(Kind::Value { ty: "location".into(), rows: corroborate::modules::value::defaults("location") }, Pos2::new(30.0, 200.0));
    a.add(Kind::Enf { grid: String::new() }, Pos2::new(460.0, 40.0));
    a.add(Kind::ResultRecord { folder: String::new() }, Pos2::new(990.0, 40.0));
    a.add(Kind::Planes, Pos2::new(460.0, 700.0));
    a.add(Kind::Map, Pos2::new(560.0, 330.0));
    a.add(Kind::InvertLocation, Pos2::new(560.0, 560.0));
    let comb = a.add(Kind::CombineLocations { any: false }, Pos2::new(1050.0, 330.0));
    a.add(Kind::GridFrequencies { path: String::new() }, Pos2::new(1050.0, 500.0));
    a.add(Kind::Plugin { path: "my-plugin.exe".into() }, Pos2::new(30.0, 560.0));
    let man = a.nodes.iter().position(|n| matches!(n.kind, Kind::Value { .. })).unwrap();
    d.frames(5);
    d.shot("all_nodes");

    let note = |d: &mut Drive| match &d.app().nodes[man].kind {
        Kind::Value { rows, .. } => rows.iter().find(|(k, _)| k == "note").unwrap().1.clone(),
        _ => unreachable!(),
    };
    // The note is the last text box in the Value node.
    let r = { let a = d.app(); a.node_rect(a.canvas_origin, man) };
    let boxes: Vec<Rect> = d.h.query_all_by_role(A11y::TextInput).map(|n| n.rect()).collect();
    let last = (0..boxes.len()).filter(|&k| r.contains(boxes[k].center())).last().unwrap();
    d.type_into(last, "typed on the canvas");
    assert_eq!(note(&mut d), "typed on the canvas", "typing edits the field");

    let r = { let a = d.app(); a.node_rect(a.canvas_origin, comb) };
    d.h.query_all_by_role(A11y::ComboBox).find(|n| r.contains(n.rect().center())).expect("Combine's mode list").click();
    d.frames(2);
    d.click("any");
    assert!(matches!(d.app().nodes[comb].kind, Kind::CombineLocations { any: true }), "Combine switched to OR");
    let uid = d.app().nodes[comb].uid.clone();
    assert!(d.app().changes.iter().any(|c| c.node == uid && c.name == "mode" && c.new == "any"), "the change is logged");
    d.shot("all_nodes_edited");
}

/// Time to work out location areas with the per-frame cache cleared (a Value's circle, Invert of
/// it, the Record stacking it). Run: cargo test --release --bin corroborate area_timing -- --ignored --nocapture
#[test]
#[ignore]
fn area_timing() {
    let mut app = App::empty();
    let man = app.add(place(50.0, 15.0, 2500.0), Pos2::new(30.0, 200.0));
    let inv = app.add(Kind::InvertLocation, Pos2::new(250.0, 420.0));
    app.wire(man, "value", inv, "location");
    let rec = app.add(Kind::ResultRecord { folder: String::new() }, Pos2::new(990.0, 40.0));
    app.wire_many(man, "value", rec);
    for (name, id) in [("value", man), ("invert", inv), ("record", rec)] {
        let t = Instant::now();
        for _ in 0..20 {
            app.area_cache.borrow_mut().clear();
            let _ = std::hint::black_box(app.area_from(id).map(|(a, _)| a.allowed().fraction()));
        }
        println!("{name} {:?}: {:.2} ms", app.area_from(id).map(|(_, n)| n), t.elapsed().as_secs_f64() * 1000.0 / 20.0);
    }
}

/// Right-click → Collapse folds a node to its title and ports; wires stay; Expand brings it back.
#[test]
fn collapse_node() {
    let mut d = Drive::new();
    let a = d.app();
    let man = a.add(place(50.0, 15.0, 100.0), Pos2::new(30.0, 60.0));
    let comb = a.add(Kind::CombineLocations { any: false }, Pos2::new(400.0, 60.0));
    a.wire(man, "value", comb, "location 1");
    d.frames(3);
    let tall = { let a = d.app(); a.node_rect(a.canvas_origin, comb).height() };
    let p = d.node_title(comb);
    d.right_click_at(p.x, p.y);
    d.click("Collapse");
    let a = d.app();
    assert!(a.nodes[comb].collapsed, "collapsed");
    let short = a.node_rect(a.canvas_origin, comb).height();
    assert!(short < tall, "shorter: {short} < {tall}");
    assert_eq!(a.graph.wires().len(), 1, "the wire stays");
    assert!(a.project_text().contains("|collapsed"), "saved");
    d.shot("collapse_node");
    let p = d.node_title(comb);
    d.right_click_at(p.x, p.y);
    d.click("Expand");
    assert!(!d.app().nodes[comb].collapsed, "expanded again");
}

/// Add node, built from the modules' categories.
#[test]
fn add_node_menu() {
    let mut d = Drive::new();
    d.click("Add node");
    let text = d.dump();
    for want in ["SOURCES", "DETECTORS", "PLUGINS", "ENF (power hum)", "Metadata", "Combine locations (any)", "Plugin from file…"] {
        assert!(text.contains(want), "{want} not in the menu:\n{text}");
    }
    d.shot("add_node_menu");
}
