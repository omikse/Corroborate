//! The data bus end to end: a program on protocol 2 (examples/bus_echo.rs, built by
//! `cargo test`) in a module folder, fed a location by Manual data; its event goes into a Result
//! record and its location on to a Map, through the same calls built-in modules use.
//! Run: cargo test --bin corroborate bus_tests -- --nocapture

use super::*;
use crate::drive::out_dir;
use corroborate::data::Data;
use corroborate::module_manifest as mm;

const MANIFEST: &str = r#"id = "bus-echo"
label = "Bus echo"
category = "Checks"
role = "transform"

[[input]]
name = "place"
kind = "location"

[[output]]
name = "seen"
kind = "event"

[[output]]
name = "place"
kind = "location"

[[setting]]
name = "label"
type = "text"
default = "got a place"

[[setting]]
name = "level"
type = "number"
default = 1
min = 0
max = 10

[processing]
program = "bus_echo.exe"
protocol = 2
"#;

/// The example program, copied into a module folder with its module.toml (one folder per test:
/// tests run at the same time, and a running program's file cannot be replaced).
fn echo_module(test: &str) -> mm::Manifest {
    let exe = std::env::current_exe().unwrap();
    let built = exe.parent().unwrap().parent().unwrap().join("examples").join("bus_echo.exe");
    assert!(built.exists(), "{} is missing: run `cargo test` (it builds the examples), or `cargo build --example bus_echo`", built.display());
    let dir = out_dir().join("bus_modules").join(test).join("Bus echo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(&built, dir.join("bus_echo.exe")).unwrap();
    std::fs::write(dir.join("module.toml"), MANIFEST).unwrap();
    let mut m = mm::parse(MANIFEST).unwrap();
    m.folder = Some(dir);
    m
}

/// Run frames of the bus (feeding the programs) until `done` or 10 s.
fn pump(app: &mut App, mut done: impl FnMut(&App) -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) {
        app.bus_sent = Instant::now() - Duration::from_secs(1);
        app.feed_programs();
        app.area_cache.borrow_mut().clear();
        if done(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn program_on_the_bus() {
    let mut app = App::empty();
    let rows = vec![("coordinates".to_string(), "50.08, 14.42".to_string()), ("radius_km".to_string(), "300".to_string())];
    let man = app.add(Kind::ManualData { rows }, Pos2::new(30.0, 40.0));
    let echo = app.add(Kind::module(echo_module("program_on_the_bus")).unwrap(), Pos2::new(300.0, 40.0));
    let map = app.add(Kind::Map, Pos2::new(600.0, 40.0));
    let rec = app.add(Kind::ResultRecord { folder: String::new() }, Pos2::new(600.0, 300.0));
    app.wire(man, "place", echo, "place");
    app.wire(echo, "place", map, "location");
    app.wire_record(echo, "seen", rec);
    assert_eq!(app.graph.wires().len(), 3, "the module's declared ports take wires");
    assert!(app.set_setting(echo, "label", "place seen").is_ok());

    app.run();
    assert!(app.programs.contains_key(&echo), "the program started: {}", app.nodes[echo].state.lock().unwrap().plugin.state);
    let got = pump(&mut app, |a| a.value_from(echo, "seen").is_ok_and(|d| matches!(d, Data::Events(v) if !v.is_empty())));
    let st = app.nodes[echo].state.lock().unwrap().plugin.clone();
    assert!(got, "an event came back: state {} log {}", st.state, st.log);

    // The event, labelled by the setting, is in the record's result file.
    let json = app.result_json(&[echo, rec]);
    assert!(json.contains("\"label\":\"place seen\""), "{json}");
    assert!(json.contains("\"kind\":\"events\""), "{json}");
    assert!(json.contains("\"setting\":\"label\",\"old\":\"got a place\",\"new\":\"place seen\""), "the change is kept: {json}");
    // The location it sent back reaches the map as the same area Manual data gives.
    let (sent_back, _) = app.area_from(echo).expect("a location from the program");
    let (from_manual, _) = app.area_from(man).unwrap();
    assert_eq!(sent_back.cells, from_manual.cells);
    assert!(app.map_status(map).contains('%') || !app.map_status(map).is_empty());

    // A changed input is sent again.
    if let Kind::ManualData { rows } = &mut app.nodes[man].kind {
        rows[1].1 = "1000".into();
    }
    app.area_cache.borrow_mut().clear();
    let bigger = app.area_from(man).unwrap().0.allowed().fraction();
    assert!(pump(&mut app, |a| a.area_from(echo).is_ok_and(|(x, _)| (x.allowed().fraction() - bigger).abs() < 1e-9)), "the new place went through");
    app.stop_all();
}

/// Settings: only declared ones, only values their checks pass; a refused value changes nothing.
#[test]
fn settings_are_checked() {
    let mut app = App::empty();
    let echo = app.add(Kind::module(echo_module("settings_are_checked")).unwrap(), Pos2::new(300.0, 40.0));
    assert!(app.set_setting(echo, "speed", "9").unwrap_err().contains("no setting speed"));
    assert!(app.set_setting(echo, "level", "11").unwrap_err().contains("outside"));
    assert!(app.set_setting(echo, "level", "loud").unwrap_err().contains("not a number"));
    assert!(app.changes.is_empty(), "refused values are not changes");
    assert!(app.set_setting(echo, "level", "7").is_ok());
    assert_eq!(app.nodes[echo].kind.rows(), [("label".to_string(), "got a place".to_string()), ("level".to_string(), "7".to_string())]);
    // Saved and read back; a bad value in the file keeps the default.
    let text = app.project_text().replace("|level|7", "|level|99");
    let mut back = App::empty();
    assert!(back.load_project_text(&app.project_text()));
    // The module is found by its id in the modules folder; here it is not there, so it is kept as missing with its rows.
    assert!(matches!(&back.nodes[0].kind, Kind::Missing { rows, .. } if rows.len() == 2), "{:?}", back.nodes[0].kind);
    assert_eq!(back.project_text(), app.project_text(), "a missing module's settings are saved back unchanged");
    let mut k = Kind::module(echo_module("settings_are_checked")).unwrap();
    for line in text.lines().filter(|l| l.starts_with("row|")) {
        let f: Vec<&str> = line.split('|').collect();
        k.add_row(f[2].into(), f[3].into());
    }
    assert_eq!(k.rows()[1], ("level".to_string(), "1".to_string()), "99 is refused on load: the default stays");
}

/// A setting typed in a burst restarts the graph once, half a second after the last change.
#[test]
fn restart_waits_for_the_last_change() {
    let mut app = App::empty();
    let echo = app.add(Kind::module(echo_module("restart_waits_for_the_last_change")).unwrap(), Pos2::new(300.0, 40.0));
    app.keep_running();
    assert_eq!(app.runs, 1, "the graph starts at once");
    for v in ["2", "3", "4"] {
        app.set_setting(echo, "level", v).unwrap();
        app.keep_running();
        std::thread::sleep(Duration::from_millis(100));
        app.keep_running();
    }
    assert_eq!(app.runs, 1, "no restart while typing");
    std::thread::sleep(Duration::from_millis(600));
    app.keep_running();
    assert_eq!(app.runs, 2, "one restart after the last change");
    app.keep_running();
    assert_eq!(app.runs, 2);
    app.stop_all();
}

/// On the canvas: one editor per setting; a value its checks refuse is shown in red with the
/// reason and not used.
#[test]
fn settings_on_the_canvas() {
    let mut d = crate::drive::Drive::new();
    let echo = d.app().add(Kind::module(echo_module("settings_on_the_canvas")).unwrap(), Pos2::new(60.0, 60.0));
    d.frames(3);
    // The text boxes are the two settings, top to bottom: label, level.
    d.type_into(1, "12");
    assert!(d.app().setting_errors.values().any(|e| e.contains("outside")), "refused: {:?}", d.app().setting_errors);
    assert_eq!(d.app().nodes[echo].kind.rows()[1].1, "1", "the refused value is not used");
    d.shot("bus_module_refused");
    d.type_into(1, "8");
    assert_eq!(d.app().nodes[echo].kind.rows()[1].1, "8");
    assert!(d.app().setting_errors.is_empty());
    d.app().stop_all();
}
