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
            let rows = if n.kind.rows().is_empty() { String::new() } else { format!(" rows={:?}", n.kind.rows()) };
            let folded = if n.collapsed { " collapsed" } else { "" };
            format!("node {} {} {:?} label={:?} at {:.0},{:.0}{rows}{folded}", idx(i), n.kind.spec().name, n.kind.saved_param(), n.label, n.pos.x, n.pos.y)
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
    let src = a.add(Kind::Sourcer { input: r"C:\Media\field recording 03.wav".into() }, Pos2::new(30.0, 40.0));
    let vid = a.add(Kind::Sourcer { input: "https://www.youtube.com/watch?v=x10vL6_47Dw".into() }, Pos2::new(30.0, 200.0));
    let mic = a.add(Kind::Sourcer { input: "mic:Microphone (USB Audio Device)".into() }, Pos2::new(30.0, 360.0));
    let mut man = crate::drive::place(50.08, 14.42, 25.0);
    if let Kind::Value { rows, .. } = &mut man {
        rows.iter_mut().find(|(k, _)| k == "note").unwrap().1 = "Příbram, ½ km from the road: 30 °C".into();
    }
    let man = a.add(man, Pos2::new(30.0, 520.0));
    let planes = a.add(Kind::Planes, Pos2::new(500.0, 700.0));
    let enf = a.add(Kind::Enf { grid: r"D:\grids\eu.json".into() }, Pos2::new(500.0, 40.0));
    a.add(Kind::GridFrequencies { path: r"D:\grids\eu.json".into() }, Pos2::new(500.0, 400.0));
    let plug = a.add(Kind::Plugin { path: r"C:\Tools\my plugin\detect.exe".into() }, Pos2::new(30.0, 760.0));
    let inv = a.add(Kind::InvertLocation, Pos2::new(1050.0, 400.0));
    let map = a.add(Kind::Map, Pos2::new(1300.0, 400.0));
    let comb = a.add(Kind::CombineLocations { any: true }, Pos2::new(1050.0, 600.0));
    let rec = a.add(Kind::ResultRecord { folder: r"E:\Evidence\case 12".into() }, Pos2::new(1050.0, 40.0));
    a.nodes[src].label = "Field recorder — north gate".into();
    a.nodes[mic].label = "Mic 2".into();
    a.nodes[inv].collapsed = true;
    a.nodes[comb].collapsed = true;

    a.wire(src, "audio", planes, "audio");
    a.wire(src, "audio", enf, "audio");
    a.wire(vid, "audio", plug, "audio");
    a.wire(vid, "time", plug, "time");
    a.wire(man, "value", plug, "place");
    a.wire(enf, "location", inv, "location");
    a.wire(inv, "location", map, "location");
    a.wire(man, "value", comb, "location 1");
    a.wire(enf, "location", comb, "location 2");
    a.wire_record(planes, "aircrafts", rec);
    a.wire_record(comb, "location", rec);
    a.wire_record(man, "value", rec);
    a.wire_record(mic, "audio", rec);
    a
}

/// Every node kind, setting, label, position, Value field and wire survives save and load, and
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
    let man = a.add(Kind::Value { ty: "text".into(), rows: vec![("a|b".into(), "line 1\nline 2".into()), ("after".into(), "kept".into())] }, Pos2::new(30.0, 40.0));
    a.nodes[man].label = "two\r\nlines | here".into();
    let src = a.add(Kind::Sourcer { input: "C:\\x|y.wav".into() }, Pos2::new(400.0, 40.0));
    let b = reload(&a);
    assert_eq!(b.nodes.len(), 2);
    assert_eq!(b.nodes[man].label, "two  lines   here");
    match &b.nodes[man].kind {
        Kind::Value { rows, .. } => assert_eq!(rows, &[("a b".to_string(), "line 1 line 2".to_string()), ("after".to_string(), "kept".to_string())]),
        k => panic!("{k:?}"),
    }
    assert!(matches!(&b.nodes[src].kind, Kind::Sourcer { input } if input == "C:\\x y.wav"));
}

/// Deleted nodes are not saved; the others are renumbered and keep their wires.
#[test]
fn deleted_nodes_are_left_out() {
    let mut a = App::empty();
    let src = a.add(Kind::Sourcer { input: "x.wav".into() }, Pos2::new(30.0, 40.0));
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
    let src = a.add(Kind::Sourcer { input: "x.wav".into() }, Pos2::new(30.0, 40.0));
    let enf = a.add(Kind::Enf { grid: String::new() }, Pos2::new(400.0, 40.0));
    a.wire(src, "audio", enf, "audio");
    let clip = a.add(Kind::Sourcer { input: "clip.mkv".into() }, Pos2::new(30.0, 300.0));
    let rec = a.add(Kind::ResultRecord { folder: String::new() }, Pos2::new(400.0, 300.0));
    a.wire_record(clip, "video", rec);
    for s in [src, clip] {
        if let Kind::Sourcer { input } = &mut a.nodes[s].kind {
            input.clear();
        }
    }
    assert_eq!(a.graph.wires().len(), 2);
    let b = reload(&a);
    assert_eq!(describe_project(&a), describe_project(&b));
}

/// A broken or cut-off file loads what it can without a panic: unknown lines, bad numbers, wires
/// to nodes that are not there and a last line cut in half are skipped; a module this tool lacks
/// stays as a placeholder, so the wires after it still reach the nodes they were saved to.
#[test]
fn damaged_files_load_what_they_can() {
    let text = "# corroborate project\n\
                node|source|30|40|My source|x.wav\n\
                node|teleporter|0|0||\n\
                node|enf|abc|-|ENF|\n\
                garbage line\n\
                row|7|k|v\n\
                wire|0|audio|2|audio\n\
                wire|0|audio|9|audio\n\
                wire|0|nonsense|2|audio\n\
                node|map|10";
    let mut b = App::empty();
    assert!(b.load_project_text(text));
    let d = describe_project(&b);
    assert_eq!(d.len(), 4, "{d:#?}");
    assert!(d[0].contains("sourcer") && d[0].contains("My source"), "{d:#?}");
    assert!(d[1].contains("teleporter"), "kept as a placeholder: {d:#?}");
    assert!(d[2].contains("enf") && d[2].contains("at 40,40"), "bad numbers fall back: {d:#?}");
    assert_eq!(d[3], "wire 0.audio -> 2.audio");
    assert!(!App::empty().load_project_text(""), "an empty file is no project");
    assert!(!App::empty().load_project_text("garbage\n"), "no nodes is no project");
}

/// On the canvas: nodes made with the Add node menu and edited in place are what the file keeps.
#[test]
fn made_on_the_canvas() {
    let mut d = Drive::new();
    for label in [Kind::Map.default_label(), Kind::Value { ty: String::new(), rows: vec![] }.default_label()] {
        d.click("Add node");
        d.click(&label);
    }
    assert_eq!(d.app().nodes.len(), 2, "two nodes added from the menu");
    d.type_into(0, "49.5");
    let back = reload(d.app());
    let want = describe_project(d.app());
    assert_eq!(want, describe_project(&back));
    assert!(want.iter().any(|l| l.contains("\"lat\"") && l.contains("49.5")), "{want:#?}");
    d.shot("project_made_on_canvas");
}

/// A project using a module this tool lacks loads it as a grey placeholder that keeps its
/// setting and wires, and saves back exactly as it was.
#[test]
fn missing_module_keeps_its_place() {
    let text = "# corroborate project\n\
                node|sourcer|30|40|SOURCER|x.wav\n\
                node|teleporter|400|40|Teleporter|speed=9\n\
                node|invert-location|700|40|Invert location|\n\
                wire|0|audio|1|sound\n\
                wire|1|where|2|location\n";
    let mut a = App::empty();
    assert!(a.load_project_text(text));
    assert!(matches!(&a.nodes[1].kind, Kind::Missing { name, param, .. } if name == "teleporter" && param == "speed=9"));
    assert_eq!(a.graph.wires().len(), 2, "{:#?}", describe_project(&a));
    let again = a.project_text();
    let mut b = App::empty();
    assert!(b.load_project_text(&again));
    assert_eq!(describe_project(&a), describe_project(&b));
    assert!(again.contains("node|teleporter|400|40|Teleporter|speed=9"), "{again}");
}

/// Files written before modules had one name each load as today's modules and save with ids.
#[test]
fn old_names_are_saved_as_ids() {
    let text = "node|source|30|40|SOURCER|x.wav\nnode|record|400|40|Result record|\nnode|combine|400|300||any\nwire|0|audio|1|aircrafts\n";
    let mut a = App::empty();
    assert!(a.load_project_text(text));
    let t = a.project_text();
    for want in ["node|sourcer|", "node|result-record|", "node|combine-locations|400|300|Combine locations (any)|any"] {
        assert!(t.contains(want), "{want} not in\n{t}");
    }
}

/// File → Save as, New, Open: the project moves to its own file, New empties the canvas, Open
/// brings the saved project back, and changes autosave to the opened file.
#[test]
fn new_open_save_as() {
    let dir = out_dir().join("project_menu");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("case one.txt");
    let _ = std::fs::remove_file(&file);
    let mut d = Drive::new();
    let inv = d.app().add(Kind::InvertLocation, Pos2::new(40.0, 60.0));
    let map = d.app().add(Kind::Map, Pos2::new(400.0, 60.0));
    d.app().wire(inv, "location", map, "location");
    d.frames(3);
    assert_eq!(d.app().project_name(), "untitled");
    let before = describe_project(d.app());
    d.app().save_project_as(file.clone());
    assert_eq!(d.app().project_name(), "case one");
    assert!(std::fs::read_to_string(&file).unwrap().contains("wire|0|location|1|location"));
    d.app().new_project();
    d.frames(3);
    assert!(d.app().nodes.is_empty(), "New empties the canvas");
    assert_eq!(d.app().project_name(), "untitled");
    d.shot("project_new");
    d.app().open_project(file.clone());
    d.frames(3);
    assert_eq!(describe_project(d.app()), before, "Open brings it back");
    assert_eq!(d.app().project_name(), "case one");
    d.app().add(Kind::Map, Pos2::new(400.0, 400.0));
    d.app().save_project();
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().filter(|l| l.starts_with("node|")).count(), 3, "saves go to the opened file");
    d.shot("project_opened");
}

#[test]
fn stable_ids_survive_save_delete_and_load() {
    let mut a = App::empty();
    let n: Vec<NodeId> = (0..3).map(|k| a.add(Kind::Map, Pos2::new(40.0, 40.0 + 200.0 * k as f32))).collect();
    let uids: Vec<String> = n.iter().map(|&i| a.nodes[i].uid.clone()).collect();
    assert_eq!(uids, ["n1", "n2", "n3"]);
    a.delete_nodes(&[n[0]]);
    let mut b = reload(&a);
    let kept: Vec<&str> = b.nodes.iter().map(|n| n.uid.as_str()).collect();
    assert_eq!(kept, ["n2", "n3"], "the ids stay with their nodes, not their places");
    let c = b.add(Kind::Map, Pos2::ZERO);
    assert_eq!(b.nodes[c].uid, "n4", "an id is never given out again");
    // Delete the newest and reload: its id is still not reused.
    b.delete_nodes(&[c]);
    let mut d = reload(&b);
    let e = d.add(Kind::Map, Pos2::ZERO);
    assert_eq!(d.nodes[e].uid, "n5");
}

#[test]
fn pasted_nodes_get_new_ids() {
    let mut a = App::empty();
    let m = a.add(Kind::Map, Pos2::ZERO);
    let text = a.nodes_text(&[m]);
    let p = a.paste(&text, Pos2::new(300.0, 0.0));
    assert_eq!(p.len(), 1);
    assert_ne!(a.nodes[p[0]].uid, a.nodes[m].uid);
    let mut uids: Vec<&str> = a.nodes.iter().map(|n| n.uid.as_str()).collect();
    uids.dedup();
    assert_eq!(uids.len(), 2);
}

#[test]
fn old_files_without_ids_get_new_ones() {
    let mut a = App::empty();
    assert!(a.load_project_text("node|map|0|0||\nnode|map|0|200||\n"));
    let uids: Vec<&str> = a.nodes.iter().map(|n| n.uid.as_str()).collect();
    assert_eq!(uids, ["n1", "n2"]);
    assert!(a.project_text().contains("|id=n2"));
}
