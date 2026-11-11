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
        let h = Harness::builder().with_size(Vec2::new(1800.0, 900.0)).wgpu().build_ui_state(
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

    /// Presses a key.
    pub fn press(&mut self, key: egui::Key) {
        self.h.key_press(key);
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

    /// Drags with the left button from `from` to `to`, in steps, like a hand would.
    pub fn drag(&mut self, from: Pos2, to: Pos2) {
        self.h.hover_at(from);
        self.h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        self.frames(1);
        for k in 1..=8 {
            self.h.hover_at(from + (to - from) * (k as f32 / 8.0));
            self.frames(1);
        }
        self.h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        self.frames(2);
    }

    /// Turns the mouse wheel at `at` by `notches` (up is positive), letting smooth scrolling end.
    pub fn scroll(&mut self, at: Pos2, notches: f32) {
        self.scroll_with(at, notches, egui::Modifiers::NONE);
    }

    /// `scroll` with keys held (Alt, Ctrl…).
    pub fn scroll_with(&mut self, at: Pos2, notches: f32, modifiers: egui::Modifiers) {
        self.h.hover_at(at);
        self.h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: Vec2::new(0.0, notches), phase: egui::TouchPhase::Move, modifiers });
        self.frames(20);
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
    assert_eq!(shown(d.app()), ["audio", "time", "length", "metadata"], "outputs appear with a source");
    assert!(ok, "the file decoded");
}

/// The ENF node after synthetic hum: clear (good enough to date), weak, and noise only (no hum).
/// The audio goes through the node's own run code (`enf_node::push`), as in a real run.
#[test]
fn enf_quality() {
    for (name, noise, dur) in [("enf_good", 0.1f64, 240.0), ("enf_weak", 1.5, 150.0), ("enf_none", 5.0, 120.0)] {
        let mut d = Drive::new();
        let e = d.app().add(Kind::Enf { grid: String::new() }, Pos2::new(40.0, 40.0));
        let job = d.app().enf_job(e);
        let mut r = crate::enf_node::start(&job);
        // 50 Hz hum drifting ±20 mHz a minute: harmonics 2, 3, 5 (and 1 when clear), in noise.
        let sr = 16000.0;
        let (mut ph, mut seed) = (0.0f64, 7u64);
        let mut gauss = || {
            let mut u = || {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((seed >> 11) as f64 + 0.5) / (1u64 << 53) as f64
            };
            let (a, b) = (u(), u());
            (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
        };
        let amps = if noise < 1.0 { [0.02, 0.03, 0.01, 0.0, 0.01] } else { [0.0, 0.02, 0.015, 0.0, 0.01] };
        let n = (dur * sr) as usize;
        let mut block = Vec::with_capacity(1600);
        for i in 0..n {
            let t = i as f64 / sr;
            ph += 2.0 * std::f64::consts::PI * (50.0 + 0.02 * (2.0 * std::f64::consts::PI * t / 60.0).sin()) / sr;
            let v: f64 = amps.iter().enumerate().map(|(k, a)| a * ((k + 1) as f64 * ph).sin()).sum();
            block.push((v + noise * gauss()) as f32);
            if block.len() == 1600 || i + 1 == n {
                crate::enf_node::push(&mut r, &block, &None, t);
                block.clear();
            }
        }
        crate::enf_node::flush(&mut r);
        job.state.lock().unwrap().done = true;
        d.frames(3);
        d.shot(name);
    }
}

/// A real recording's hum dated against the grid log store (CORROBORATE_GRID_LOGS). ENF_F32: raw
/// 16 kHz mono f32le audio. Optional, wired in as Value nodes: ENF_UTC, its claimed start (ISO
/// 8601); ENF_LAT and ENF_LON, the claimed place. Without them the match is blind. Writes
/// `enf_date.png`.
#[test]
#[ignore]
fn enf_date() {
    let var = |k: &str| std::env::var(k).ok();
    let mut d = Drive::new();
    let e = d.app().add(Kind::Enf { grid: String::new() }, Pos2::new(300.0, 40.0));
    if let Some(utc) = var("ENF_UTC") {
        let t = d.app().add(Kind::Value { ty: "time".into(), rows: vec![("utc".into(), utc), ("error_s".into(), "1".into())] }, Pos2::new(20.0, 60.0));
        d.app().wire(t, "value", e, "time");
    }
    if let (Some(lat), Some(lon)) = (var("ENF_LAT"), var("ENF_LON")) {
        let l = d.app().add(Kind::Value { ty: "location".into(), rows: vec![("lat".into(), lat), ("lon".into(), lon), ("radius_km".into(), "5".into())] }, Pos2::new(20.0, 260.0));
        d.app().wire(l, "value", e, "location");
    }
    let job = d.app().enf_job(e);
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
    let b = std::fs::read(var("ENF_F32")).unwrap();
    let x: Vec<f32> = b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let mut r = crate::enf_node::start(&job);
    for (k, block) in x.chunks(16_000).enumerate() {
        crate::enf_node::push(&mut r, block, &None, (k + 1) as f64);
    }
    let t = std::time::Instant::now();
    crate::enf_node::flush(&mut r);
    println!("match took {:.1} s", t.elapsed().as_secs_f64());
    job.state.lock().unwrap().done = true;
    {
        let s = job.state.lock().unwrap();
        match &s.enf.dated {
            Some(Ok(dt)) => println!("{}\n{}", dt.verdict(), dt.lines().join("\n")),
            other => println!("{other:?}"),
        }
        println!("area: {}", s.enf.area_note);
    }
    println!("audio start time: {:?}", d.app().value_from(e, "audio start time"));
    d.frames(3);
    d.shot("enf_date");
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
    d.click("+ Add node");
    let text = d.dump();
    for want in ["SOURCES", "DETECTORS", "PLUGINS", "ENF (power hum)", "Metadata", "Combine locations (any)", "Plugin from file…"] {
        assert!(text.contains(want), "{want} not in the menu:\n{text}");
    }
    d.shot("add_node_menu");
}

/// A 3 s clip (grey picture, 440 Hz tone) with GPS in its metadata as a phone writes it: London,
/// 10 m up (ISO 6709 in `location`).
pub fn gps_clip() -> PathBuf {
    let p = out_dir().join("gps.mp4");
    if !p.exists() {
        let ok = corroborate::no_window(&mut std::process::Command::new("ffmpeg"))
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=f=440:d=3", "-f", "lavfi", "-i", "color=c=gray:s=64x48:d=3", "-shortest"])
            .args(["-metadata", "location=+51.5074-000.1278+010.000/", "-c:v", "libx264", "-c:a", "aac"])
            .arg(&p)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "ffmpeg made the GPS clip");
    }
    p
}

/// The location panel: a file with GPS in its metadata puts a pin at its place with nothing
/// wired, framed by itself; an area (a typed place with a radius, through Combine) is tinted.
/// A Map node takes everything wired into it: a file's GPS as a pin, a typed place, a combined
/// area, and grows a port for each. The wheel over it zooms the map, not the canvas.
#[test]
fn map_from_metadata() {
    let mut d = Drive::new();
    let s = d.app().add(Kind::Sourcer { input: gps_clip().to_string_lossy().into_owned() }, Pos2::new(40.0, 40.0));
    let v = d.app().add(Kind::Value { ty: "location".into(), rows: vec![("lat".into(), "48.85".into()), ("lon".into(), "2.35".into()), ("radius_km".into(), "300".into())] }, Pos2::new(40.0, 520.0));
    let comb = d.app().add(Kind::CombineLocations { any: true }, Pos2::new(420.0, 520.0));
    d.app().wire(v, "value", comb, "location 1");
    let map = d.app().add(Kind::Map, Pos2::new(900.0, 40.0));
    assert_eq!(d.app().map_status(map), "no location wired");
    d.app().wire_many(s, "location", map);
    d.frames(3);
    d.app().wire_many(comb, "location", map);
    d.frames(3);
    assert!(d.app().graph.nodes()[map].spec.inputs.len() >= 3, "a free port is left");
    assert_eq!(d.app().graph.feeder(map, "location 2"), Some((comb, "location")));
    assert!(d.wait_until(30.0, |app| app.map_inputs(map).0.iter().any(|p| p.node == s && p.site.is_some())), "the clip's GPS reached the map: {}", d.app().map_status(map));
    let (pl, errors) = d.app().map_inputs(map);
    assert!(errors.is_empty(), "{errors:?}");
    let site = pl[0].site.unwrap();
    assert!((site.lat - 51.5074).abs() < 1e-6 && (site.lon + 0.1278).abs() < 1e-6 && site.elevation == Some(10.0), "{site:?}");
    assert!(pl[0].point && pl[1].node == comb && !pl[1].point, "the combined area is on it");
    assert_eq!(d.app().map_status(map), "1 point, 1 area");
    d.frames(4);
    d.shot("map_node");

    let uid = d.app().nodes[map].uid.clone();
    let (canvas, before) = (d.app().zoom, d.app().maps.views[&uid].memory.zoom());
    let over = d.app().maps.over[0].center();
    d.h.input_mut().events.push(egui::Event::PointerMoved(over));
    d.frames(2);
    for _ in 0..4 {
        d.h.input_mut().events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, 60.0), phase: egui::TouchPhase::Move, modifiers: Default::default() });
        d.frames(2);
    }
    assert_eq!(d.app().zoom, canvas, "the canvas stays");
    assert!(d.app().maps.views[&uid].memory.zoom() > before, "the map zooms");
}
