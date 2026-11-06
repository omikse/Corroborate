//! The project pipeline: nodes made on the canvas, saved to the project file, read back.
//! Run: cargo test --bin corroborate project_tests -- --nocapture

use super::*;
use crate::drive::{out_dir, Drive};

/// Everything a project should keep, in a form two workbenches can be compared by: each visible
/// node (kind, setting, label, position, Manual rows) and each wire by node and port names.
fn describe_project(app: &App) -> Vec<String> {
    let keep: Vec<NodeId> = (0..app.nodes.len()).filter(|&i| !app.nodes[i].hidden).collect();
    let idx = |i: NodeId| keep.iter().position(|&k| k == i).unwrap();
    let mut v: Vec<String> = keep
        .iter()
        .map(|&i| {
            let n = &app.nodes[i];
            let rows = match &n.kind {
                Kind::Manual { rows } => format!(" rows={rows:?}"),
                _ => String::new(),
            };
            format!("node {} {} {:?} label={:?} at {:.0},{:.0}{rows}", idx(i), n.kind.spec().name, n.kind.saved_param(), n.label, n.pos.x, n.pos.y)
        })
        .collect();
    let g = app.graph.nodes();
    let mut wires: Vec<String> = app
        .graph
        .wires()
        .iter()
        .map(|w| format!("wire {}.{} -> {}.{}", idx(w.from), g[w.from].spec.outputs[w.from_port].name, idx(w.to), g[w.to].spec.inputs[w.to_port].name))
        .collect();
    wires.sort();
    v.extend(wires);
    v
}

fn reload(app: &App) -> App {
    let mut back = App::empty();
    assert!(back.load_project_text(&app.project_text()), "the saved project loads");
    back
}

/// A project with one of every node kind, unusual settings and labels, and every kind of wire.
fn full_project() -> App {
    let mut a = App::empty();
    let src = a.add(Kind::Source { input: r"C:\Media\field recording 03.wav".into() }, Pos2::new(30.0, 40.0));
    let vid = a.add(Kind::Source { input: "https://www.youtube.com/watch?v=x10vL6_47Dw".into() }, Pos2::new(30.0, 200.0));
    let mic = a.add(Kind::Source { input: "mic:Microphone (USB Audio Device)".into() }, Pos2::new(30.0, 360.0));
    let rows = vec![
        ("coordinates".to_string(), "50.08, 14.42".to_string()),
        ("radius_km".to_string(), "25".to_string()),
        ("notes".to_string(), "Příbram, ½ km from the road: 30 °C".to_string()),
        ("empty".to_string(), String::new()),
    ];
    let man = a.add(Kind::Manual { rows }, Pos2::new(30.0, 520.0));
    let planes = a.add(Kind::Planes, Pos2::new(500.0, 700.0));
    let enf = a.add(Kind::Enf { grid: r"D:\grids\eu.json".into() }, Pos2::new(500.0, 40.0));
    a.add(Kind::GridFrequencies { path: r"D:\grids\eu.json".into() }, Pos2::new(500.0, 400.0));
    let plug = a.add(Kind::Plugin { path: r"C:\Tools\my plugin\detect.exe".into() }, Pos2::new(30.0, 760.0));
    let inv = a.add(Kind::Invert, Pos2::new(1050.0, 400.0));
    let map = a.add(Kind::Map, Pos2::new(1300.0, 400.0));
    let comb = a.add(Kind::Combine { any: true }, Pos2::new(1050.0, 600.0));
    let rec = a.add(Kind::Record { folder: r"E:\Evidence\case 12".into() }, Pos2::new(1050.0, 40.0));
    a.nodes[src].label = "Field recorder — north gate".into();
    a.nodes[mic].label = "Mic 2".into();

    a.wire(src, "audio", planes, "audio");
    a.wire(src, "audio", enf, "audio");
    a.wire(vid, "audio", plug, "audio");
    a.wire(vid, "time", plug, "time");
    a.wire(man, "place", plug, "place");
    a.wire(enf, "location", inv, "location");
    a.wire(inv, "location", map, "location");
    a.wire(man, "place", comb, "location 1");
    a.wire(enf, "location", comb, "location 2");
    a.wire_record(planes, "aircrafts", rec);
    a.wire_record(comb, "location", rec);
    a.wire_record(man, "facts", rec);
    a.wire_record(mic, "audio", rec);
    a
}

/// Every node kind, setting, label, position, Manual row and wire survives save and load, and
/// saving the loaded project gives the same file.
#[test]
fn round_trip_every_kind() {
    let a = full_project();
    let wires = a.graph.wires().len();
    assert_eq!(wires, 13, "all test wires were made: {:#?}", describe_project(&a));
    let b = reload(&a);
    assert_eq!(describe_project(&a), describe_project(&b));
    assert_eq!(a.project_text(), b.project_text(), "saving a loaded project changes nothing");
}

/// Through a real file: written beside the target and renamed over it, nothing left behind,
/// an older project replaced whole.
#[test]
fn saves_through_a_file() {
    let dir = out_dir().join("project_file");
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("settings").join("project.txt");
    let mut small = App::empty();
    small.add(Kind::Map, Pos2::new(10.0, 10.0));
    assert!(small.save_project_to(&path), "first save (creates the folder)");
    let a = full_project();
    assert!(a.save_project_to(&path), "save over an older project");
    let names: Vec<String> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["project.txt"], "no temporary file left");
    let mut b = App::empty();
    assert!(b.load_project_text(&std::fs::read_to_string(&path).unwrap()));
    assert_eq!(describe_project(&a), describe_project(&b));
}

/// The file keeps one field per `|`: a label or value holding `|` or a line break is saved with
/// spaces there, and nothing after it shifts.
#[test]
fn separators_in_text_do_not_shift_fields() {
    let mut a = App::empty();
    let man = a.add(Kind::Manual { rows: vec![("a|b".into(), "line 1\nline 2".into()), ("after".into(), "kept".into())] }, Pos2::new(30.0, 40.0));
    a.nodes[man].label = "two\r\nlines | here".into();
    let src = a.add(Kind::Source { input: "C:\\x|y.wav".into() }, Pos2::new(400.0, 40.0));
    let b = reload(&a);
    assert_eq!(b.nodes.len(), 2);
    assert_eq!(b.nodes[man].label, "two  lines   here");
    match &b.nodes[man].kind {
        Kind::Manual { rows } => assert_eq!(rows, &[("a b".to_string(), "line 1 line 2".to_string()), ("after".to_string(), "kept".to_string())]),
        k => panic!("{k:?}"),
    }
    assert!(matches!(&b.nodes[src].kind, Kind::Source { input } if input == "C:\\x y.wav"));
}

/// Deleted nodes are not saved; the others are renumbered and keep their wires.
#[test]
fn deleted_nodes_are_left_out() {
    let mut a = App::empty();
    let src = a.add(Kind::Source { input: "x.wav".into() }, Pos2::new(30.0, 40.0));
    let gone = a.add(Kind::Planes, Pos2::new(400.0, 40.0));
    let enf = a.add(Kind::Enf { grid: String::new() }, Pos2::new(400.0, 300.0));
    a.wire(src, "audio", gone, "audio");
    a.wire(src, "audio", enf, "audio");
    a.graph.isolate(gone);
    a.nodes[gone].hidden = true;
    let b = reload(&a);
    assert_eq!(b.nodes.len(), 2, "{:#?}", describe_project(&b));
    assert_eq!(describe_project(&a), describe_project(&b));
}

/// A SOURCER whose box was emptied keeps its outputs and wires on the canvas (they come back to
/// life with the next source). Saving and loading keeps them too.
#[test]
fn emptied_source_keeps_its_wires() {
    let mut a = App::empty();
    let src = a.add(Kind::Source { input: "x.wav".into() }, Pos2::new(30.0, 40.0));
    let enf = a.add(Kind::Enf { grid: String::new() }, Pos2::new(400.0, 40.0));
    a.wire(src, "audio", enf, "audio");
    let clip = a.add(Kind::Source { input: "clip.mkv".into() }, Pos2::new(30.0, 300.0));
    let rec = a.add(Kind::Record { folder: String::new() }, Pos2::new(400.0, 300.0));
    a.wire_record(clip, "video", rec);
    for s in [src, clip] {
        if let Kind::Source { input } = &mut a.nodes[s].kind {
            input.clear();
        }
    }
    assert_eq!(a.graph.wires().len(), 2);
    let b = reload(&a);
    assert_eq!(describe_project(&a), describe_project(&b));
}

/// A broken or cut-off file loads what it can: unknown lines and kinds, bad numbers, wires to
/// nodes that are not there, and a last line cut in half are skipped without a panic.
#[test]
fn damaged_files_load_what_they_can() {
    let text = "# corroborate project\n\
                node|source|30|40|My source|x.wav\n\
                node|teleporter|0|0||\n\
                node|enf|abc|-|ENF|\n\
                garbage line\n\
                row|7|k|v\n\
                wire|0|audio|1|audio\n\
                wire|0|audio|9|audio\n\
                wire|0|nonsense|1|audio\n\
                node|map|10";
    let mut b = App::empty();
    assert!(b.load_project_text(text));
    let d = describe_project(&b);
    assert_eq!(d.len(), 3, "{d:#?}");
    assert!(d[0].contains("source") && d[0].contains("My source"), "{d:#?}");
    assert!(d[1].contains("enf") && d[1].contains("at 40,40"), "bad numbers fall back: {d:#?}");
    assert_eq!(d[2], "wire 0.audio -> 1.audio");
    assert!(!App::empty().load_project_text(""), "an empty file is no project");
    assert!(!App::empty().load_project_text("garbage\n"), "no nodes is no project");
}

/// On the canvas: nodes made with the Add node menu and edited in place are what the file keeps.
#[test]
fn made_on_the_canvas() {
    let mut d = Drive::new();
    for label in [Kind::Map.default_label(), Kind::Manual { rows: vec![] }.default_label()] {
        d.click("Add node");
        d.click(&label);
    }
    assert_eq!(d.app().nodes.len(), 2, "two nodes added from the menu");
    d.click("+ field");
    d.type_into(0, "coordinates");
    let back = reload(d.app());
    let want = describe_project(d.app());
    assert_eq!(want, describe_project(&back));
    assert!(want.iter().any(|l| l.contains("\"coordinates\"")), "{want:#?}");
    d.shot("project_made_on_canvas");
}
