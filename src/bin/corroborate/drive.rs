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

impl Drive {
    /// An empty workbench, 1400 × 780 like the real window.
    pub fn new() -> Self {
        Self::with(App::empty())
    }

    /// The default project (source, Planes, ENF, Manual, Record and bundled plugins).
    pub fn default_project() -> Self {
        let mut app = App::empty();
        app.default_project();
        Self::with(app)
    }

    pub fn with(app: App) -> Self {
        let h = Harness::builder().with_size(Vec2::new(1400.0, 780.0)).wgpu().build_ui_state(
            |ui, app: &mut App| {
                ui.ctx().set_visuals(egui::Visuals::dark());
                app.frame(ui);
            },
            app,
        );
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
        let img = self.h.render().expect("render (wgpu)");
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

/// SOURCER: a new node is two boxes with no outputs; typing a file gives outputs and a waveform.
#[test]
fn sourcer() {
    let mut d = Drive::new();
    let s = d.app().add(Kind::Source { input: String::new() }, Pos2::new(40.0, 40.0));
    d.frames(3);
    assert!(d.app().graph.nodes()[s].spec.outputs.is_empty(), "a new SOURCER has no outputs");
    println!("{}", d.dump());
    d.shot("sourcer_new");

    let wav = tone_wav();
    d.type_into(0, &wav.to_string_lossy());
    let ok = d.wait_until(30.0, |app| app.nodes[s].state.lock().map(|st| st.audio_s > 2.0).unwrap_or(false));
    d.shot("sourcer_file");
    assert!(!d.app().graph.nodes()[s].spec.outputs.is_empty(), "outputs appear with a source");
    assert!(ok, "the file decoded");
}
