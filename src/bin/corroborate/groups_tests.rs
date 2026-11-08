//! Node groups (groups.rs) driven like a user would: Ctrl+G, Tab in and out, wiring the Group
//! Input and Output, uses of one definition kept the same, Alt+G, saving, undo. Headless;
//! screenshots in target/drive/ (groups_*.png).

use super::drive::{self, Drive};
use super::*;
use egui::{Key, Modifiers};

/// Two places → Invert → Combine (all) ← the second place; Combine → Map.
struct Scene {
    d: Drive,
    a: NodeId,
    b: NodeId,
    inv: NodeId,
    comb: NodeId,
    map: NodeId,
}

fn scene() -> Scene {
    let mut d = Drive::new();
    let a = d.app().add(drive::place(52.0, 21.0, 300.0), Pos2::new(40.0, 40.0));
    let b = d.app().add(drive::place(50.0, 20.0, 200.0), Pos2::new(40.0, 330.0));
    let inv = d.app().add(Kind::InvertLocation, Pos2::new(330.0, 40.0));
    let comb = d.app().add(Kind::CombineLocations { any: false }, Pos2::new(600.0, 180.0));
    let map = d.app().add(Kind::Map, Pos2::new(1000.0, 180.0));
    d.app().wire(a, "value", inv, "location");
    d.app().wire(inv, "location", comb, "location 1");
    d.app().sync_many_ports();
    d.app().wire(b, "value", comb, "location 2");
    d.app().sync_many_ports();
    d.app().wire(comb, "location", map, "location");
    d.frames(3);
    Scene { d, a, b, inv, comb, map }
}

fn key(d: &mut Drive, m: Modifiers, k: Key) {
    d.h.key_press_modifiers(m, k);
    d.frames(3);
}

/// The real wires, as (from, out, to, in), sorted.
fn wires(app: &App) -> Vec<(NodeId, String, NodeId, String)> {
    let g = app.graph.nodes();
    let mut v: Vec<_> = app.graph.wires().iter().map(|w| (w.from, g[w.from].spec.outputs[w.from_port].name.to_string(), w.to, g[w.to].spec.inputs[w.to_port].name.to_string())).collect();
    v.sort();
    v
}

/// The lines the canvas shows now, as (from label, out, to label, in), sorted.
fn lines(app: &App) -> Vec<String> {
    let g = app.graph.nodes();
    let mut v: Vec<String> = app
        .shown_lines()
        .iter()
        .map(|s| format!("{}.{} > {}.{}", app.nodes[s.from].label, g[s.from].spec.outputs[s.from_port].name, app.nodes[s.to].label, g[s.to].spec.inputs[s.to_port].name))
        .collect();
    v.sort();
    v
}

fn group_of(app: &App) -> NodeId {
    app.nodes.iter().position(|n| !n.hidden && matches!(n.kind, Kind::Group { .. })).expect("a group")
}

/// Where port `k` of node `i` is on screen (an output when `out`).
fn port(d: &mut Drive, i: NodeId, k: usize, out: bool) -> Pos2 {
    let app = d.app();
    app.port_place(app.canvas_origin, i, k, out).0
}

#[test]
fn group_open_wire_and_ungroup() {
    let Scene { mut d, a, b, inv, comb, map } = scene();
    let before = wires(d.app());
    d.shot("groups_1_before");
    // Ctrl+G on Invert and Combine.
    d.app().sel.picked = [inv, comb].into_iter().collect();
    key(&mut d, Modifiers::COMMAND, Key::G);
    let g = group_of(d.app());
    assert_eq!(wires(d.app()), before, "grouping changes no wire underneath");
    let app = d.app();
    assert_eq!(app.nodes[inv].parent, Some(g));
    assert_eq!(app.nodes[comb].parent, Some(g));
    let spec = &app.graph.nodes()[g].spec;
    let names = |ps: &[corroborate::core::Port]| ps.iter().map(|p| p.name).collect::<Vec<_>>();
    assert_eq!(names(&spec.inputs), ["location", "location 2"], "one input per wire coming in");
    assert_eq!(names(&spec.outputs), ["location"], "one output per wire going out");
    assert_eq!(lines(app), ["Group.location > Map.location", "Value.value > Group.location", "Value.value > Group.location 2"]);
    assert!(app.on_canvas(g) && !app.on_canvas(inv), "outside, the group stands for its nodes");
    d.shot("groups_2_grouped");

    // Tab: inside, between Group Input and Group Output.
    key(&mut d, Modifiers::NONE, Key::Tab);
    assert_eq!(d.app().view, Some(g));
    assert!(d.app().on_canvas(inv) && !d.app().on_canvas(map));
    assert_eq!(
        lines(d.app()),
        ["Combine locations (all).location > Group Output.location", "Group Input.location 2 > Combine locations (all).location 2", "Group Input.location > Invert location.location", "Invert location.location > Combine locations (all).location 1"]
    );
    d.shot("groups_3_inside");

    // Dragging the Group Input's empty port onto the free third input of Combine adds a port.
    let gi = d.app().nodes.iter().position(|n| !n.hidden && matches!(n.kind, Kind::GroupInput)).unwrap();
    let plus = port(&mut d, gi, 2, true);
    let free = port(&mut d, comb, 2, false);
    d.drag(plus, free);
    d.frames(3);
    let names_now: Vec<&str> = d.app().graph.nodes()[g].spec.inputs.iter().map(|p| p.name).collect();
    assert_eq!(names_now, ["location", "location 2", "location 3"], "{}", d.msg());
    d.shot("groups_4_new_port");

    // Back out: the new port is on the group, wired to nothing yet. Wire the first place into it.
    key(&mut d, Modifiers::NONE, Key::Tab);
    assert_eq!(d.app().view, None);
    let out_a = port(&mut d, a, 0, true);
    let in3 = port(&mut d, g, 2, false);
    d.drag(out_a, in3);
    assert!(wires(d.app()).contains(&(a, "value".into(), comb, "location 3".into())), "{}", d.msg());
    d.shot("groups_5_wired_outside");

    // Alt+G: the nodes come back, the wires are those made.
    d.app().pick_only(g);
    key(&mut d, Modifiers::ALT, Key::G);
    let app = d.app();
    assert!(app.nodes[g].hidden);
    assert_eq!(app.nodes[inv].parent, None);
    assert!(app.on_canvas(inv) && app.on_canvas(comb));
    assert_eq!(app.nodes.iter().filter(|n| !n.hidden).count(), 5, "Group Input and Output went with the group");
    assert!(wires(app).contains(&(b, "value".into(), comb, "location 2".into())));
    d.shot("groups_6_ungrouped");
}

#[test]
fn uses_of_one_group_stay_the_same() {
    let Scene { mut d, a, b, inv, comb, .. } = scene();
    d.app().sel.picked = [inv, comb].into_iter().collect();
    key(&mut d, Modifiers::COMMAND, Key::G);
    let g1 = group_of(d.app());
    // A second use: Ctrl+D.
    d.app().pick_only(g1);
    key(&mut d, Modifiers::COMMAND, Key::D);
    let uses: Vec<NodeId> = (0..d.app().nodes.len()).filter(|&i| !d.app().nodes[i].hidden && matches!(d.app().nodes[i].kind, Kind::Group { .. })).collect();
    assert_eq!(uses.len(), 2);
    let g2 = uses[1];
    assert_eq!(d.app().inside(g2).len(), 4, "the copy holds copies of the two nodes and the port nodes");
    assert_eq!(d.app().graph.feeder(inv, "location").map(|f| f.0), Some(a), "copying keeps the first use's wires");
    // Its own wire outside: b into its first input.
    let out_b = port(&mut d, b, 0, true);
    let in1 = port(&mut d, g2, 0, false);
    d.drag(out_b, in1);
    d.shot("groups_7_two_uses");
    assert_eq!(d.app().graph.feeder(inv, "location").map(|f| f.0), Some(a), "wiring the second use leaves the first: {}", d.msg());
    let inv2 = d.app().inside(g2).into_iter().find(|&x| matches!(d.app().nodes[x].kind, Kind::InvertLocation)).unwrap();
    assert_eq!(d.app().graph.feeder(inv2, "location").map(|f| f.0), Some(b));

    // Inside the first, a change: Combine becomes "any" (via its setting) and Invert is renamed.
    d.app().pick_only(g1);
    key(&mut d, Modifiers::NONE, Key::Tab);
    d.app().nodes[comb].kind.put_setting("mode", "any".into());
    d.app().nodes[inv].label = "Not here".into();
    d.frames(3);
    key(&mut d, Modifiers::NONE, Key::Tab);
    let app = d.app();
    let comb2 = app.inside(g2).into_iter().find(|&x| matches!(app.nodes[x].kind, Kind::CombineLocations { .. })).unwrap();
    assert!(matches!(app.nodes[comb2].kind, Kind::CombineLocations { any: true }), "the other use follows");
    assert_eq!(app.nodes[inv2].label, "Not here");
    assert_eq!(app.graph.feeder(inv2, "location").map(|f| f.0), Some(b), "and keeps its own wire outside");
    assert_eq!(app.graph.feeder(inv, "location").map(|f| f.0), Some(a));
    let _ = a;

    // A node added inside the second use shows up in the first.
    d.app().pick_only(g2);
    key(&mut d, Modifiers::NONE, Key::Tab);
    d.app().add(Kind::Map, Pos2::new(900.0, 400.0));
    d.frames(3);
    key(&mut d, Modifiers::NONE, Key::Tab);
    let maps_in = |app: &App, g: NodeId| app.inside(g).into_iter().filter(|&x| matches!(app.nodes[x].kind, Kind::Map)).count();
    assert_eq!(maps_in(d.app(), g1), 1, "added in one use, added in both");
    assert_eq!(maps_in(d.app(), g2), 1);
    d.shot("groups_8_follow");
}

#[test]
fn groups_survive_save_load_and_undo() {
    let Scene { mut d, inv, comb, .. } = scene();
    d.wait(0.5);
    d.app().sel.picked = [inv, comb].into_iter().collect();
    key(&mut d, Modifiers::COMMAND, Key::G);
    let text = d.app().project_text();
    assert!(text.contains("node|group|"), "{text}");
    assert!(text.contains("|in=") && text.contains("|m=m1"), "{text}");
    let want = lines(d.app());
    let wires_before = wires(d.app());

    let mut e = Drive::new();
    assert!(e.app().load_project_text(&text));
    e.frames(3);
    assert_eq!(lines(e.app()), want, "the same lines after loading");
    assert_eq!(wires(e.app()), wires_before);
    let g = group_of(e.app());
    assert_eq!(e.app().inside(g).len(), 4);

    // Undo the grouping.
    d.wait(0.6);
    assert!(d.app().undo());
    d.frames(3);
    let app = d.app();
    assert_eq!(app.nodes[inv].parent, None, "undo takes the group away");
    assert!(app.nodes.iter().all(|n| n.hidden || !matches!(n.kind, Kind::Group { .. } | Kind::GroupInput | Kind::GroupOutput)));
    assert_eq!(wires(app), wires_before);
    assert!(d.app().redo());
    d.frames(3);
    assert!(d.app().nodes[inv].parent.is_some(), "redo brings it back");
}

/// A group inside a group: the outer one's ports still reach the nodes inside the inner one, and
/// the lines at each level are drawn to the node standing for them there.
#[test]
fn nested_groups() {
    let Scene { mut d, a, b, inv, comb, map } = scene();
    let before = wires(d.app());
    d.app().sel.picked = [inv, comb].into_iter().collect();
    key(&mut d, Modifiers::COMMAND, Key::G);
    let outer = group_of(d.app());
    d.app().pick_only(outer);
    key(&mut d, Modifiers::NONE, Key::Tab);
    // Inside: Invert alone into a group of its own.
    d.app().pick_only(inv);
    key(&mut d, Modifiers::COMMAND, Key::G);
    let inner = d.app().nodes[inv].parent.unwrap();
    assert_ne!(inner, outer);
    assert_eq!(d.app().nodes[inner].parent, Some(outer));
    assert_eq!(wires(d.app()), before);
    assert_eq!(
        lines(d.app()),
        ["Combine locations (all).location > Group Output.location", "Group 2.location > Combine locations (all).location 1", "Group Input.location 2 > Combine locations (all).location 2", "Group Input.location > Group 2.location"]
    );
    d.shot("groups_9_nested");
    // Two levels in, then the path's "Top" goes straight out.
    d.app().pick_only(inner);
    key(&mut d, Modifiers::NONE, Key::Tab);
    assert_eq!(d.app().view, Some(inner));
    assert!(lines(d.app()).contains(&"Group Input.location > Invert location.location".to_string()), "{:?}", lines(d.app()));
    d.shot("groups_10_two_deep");
    d.click("Top");
    assert_eq!(d.app().view, None);
    assert_eq!(lines(d.app()), ["Group.location > Map.location", "Value.value > Group.location", "Value.value > Group.location 2"]);
    // Saved and opened again, the same.
    let text = d.app().project_text();
    let mut e = Drive::new();
    e.app().load_project_text(&text);
    e.frames(3);
    assert_eq!(wires(e.app()), wires(d.app()));
    let _ = (a, b, map);
}

/// Picking a node inside a group elsewhere (the timeline, the recordings list) shows its group;
/// deleting a group deletes what is in it; its Group Input cannot be deleted alone.
#[test]
fn pick_inside_and_delete() {
    let Scene { mut d, inv, comb, map, .. } = scene();
    d.app().sel.picked = [inv, comb].into_iter().collect();
    key(&mut d, Modifiers::COMMAND, Key::G);
    let g = group_of(d.app());
    d.app().pick_only(comb);
    d.frames(2);
    assert_eq!(d.app().view, Some(g), "the canvas shows the group Combine is in");
    let gi = d.app().nodes.iter().position(|n| !n.hidden && matches!(n.kind, Kind::GroupInput)).unwrap();
    d.app().pick_only(gi);
    key(&mut d, Modifiers::NONE, Key::Delete);
    assert!(!d.app().nodes[gi].hidden, "a group keeps its Group Input");
    key(&mut d, Modifiers::NONE, Key::Tab);
    assert_eq!(d.app().view, None);
    d.app().pick_only(g);
    key(&mut d, Modifiers::NONE, Key::Delete);
    let app = d.app();
    assert!(app.nodes[g].hidden && app.nodes[inv].hidden && app.nodes[comb].hidden && app.nodes[gi].hidden);
    assert!(app.graph.feeder(map, "location").is_none());
}

/// Right-click on a Group Output port removes the port and its wires outside; on a group's
/// input, the wire into it goes and the port stays.
#[test]
fn unwire_group_ports() {
    let Scene { mut d, b, inv, comb, map, .. } = scene();
    d.app().sel.picked = [inv, comb].into_iter().collect();
    key(&mut d, Modifiers::COMMAND, Key::G);
    let g = group_of(d.app());
    let p = port(&mut d, g, 1, false);
    d.right_click_at(p.x, p.y);
    assert!(d.app().graph.feeder(comb, "location 2").is_none(), "{}", d.msg());
    assert_eq!(d.app().graph.nodes()[g].spec.inputs.len(), 2, "the port stays while it is linked inside");
    // Wire it again by dragging onto the port.
    let out_b = port(&mut d, b, 0, true);
    let p = port(&mut d, g, 1, false);
    d.drag(out_b, p);
    assert_eq!(d.app().graph.feeder(comb, "location 2").map(|f| f.0), Some(b));
    // Inside: right-click the Group Output's port.
    d.app().pick_only(g);
    key(&mut d, Modifiers::NONE, Key::Tab);
    let go = d.app().nodes.iter().position(|n| !n.hidden && matches!(n.kind, Kind::GroupOutput)).unwrap();
    let p = port(&mut d, go, 0, false);
    d.right_click_at(p.x, p.y);
    d.frames(2);
    assert!(d.app().graph.feeder(map, "location").is_none(), "the wire outside went with the port");
    assert!(d.app().graph.nodes()[g].spec.outputs.is_empty());
}
