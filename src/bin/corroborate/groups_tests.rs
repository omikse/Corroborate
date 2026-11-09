//! Node groups (groups.rs) driven like a user would: Ctrl+G, Tab in and out, wiring the Group
//! Input and Output, uses of one definition kept the same, Alt+G, saving, undo. Headless;
//! screenshots in target/drive/ (groups_*.png).

use super::drive::{self, Drive};
use super::*;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};

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
    d.app().wire(comb, "location", map, "location 1");
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
    assert_eq!(lines(app), ["Group.location > Map.location 1", "Value.value > Group.location", "Value.value > Group.location 2"]);
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

    // Moving a node inside one use moves it in the other, and no wire is remade (in order, as
    // they are kept: remaking them would restart a running graph).
    let order = |app: &App| app.graph.wires().to_vec();
    let before = order(d.app());
    d.app().nodes[inv].pos += Vec2::new(25.0, 10.0);
    d.frames(3);
    let app = d.app();
    assert_eq!(app.nodes[inv2].pos, app.nodes[inv].pos);
    assert_eq!(order(d.app()), before, "a move rewires nothing");

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
    let g = group_of(d.app());
    let all: Vec<NodeId> = (0..d.app().nodes.len()).collect();
    let json = d.app().result_json(&all);
    assert!(json.contains(&format!("\"in_group\":{g},\"group_member\":\"m1\"")), "the result file says which group a node is in: {json}");
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
    assert_eq!(lines(d.app()), ["Group.location > Map.location 1", "Value.value > Group.location", "Value.value > Group.location 2"]);
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

/// The Jiku project (50 videos into one Audio match; a copy in target\drive) all in one group:
/// Ctrl+A, Ctrl+G, then run it from outside and look inside. Frames stay quick, the match runs
/// through the group as before. PNGs groups_jiku_top, groups_jiku_inside.
/// Run: cargo test --profile fast --bin corroborate groups_on_a_big_project -- --ignored --nocapture
#[test]
#[ignore]
fn groups_on_a_big_project() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let made = root.join("app").join("projects").join("Jiku GT_090912").join("Jiku GT_090912.txt");
    assert!(made.is_file(), "make it first: make_jiku_project");
    let dir = drive::out_dir().join("Jiku groups");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Jiku groups.txt");
    std::fs::copy(&made, &file).unwrap();
    let mut d = Drive::new();
    d.app().open_project(file.clone());
    d.frames(3);
    let before = wires(d.app());
    key(&mut d, Modifiers::COMMAND, Key::A);
    let t = Instant::now();
    key(&mut d, Modifiers::COMMAND, Key::G);
    println!("Ctrl+G on 51 nodes: {:.0} ms (3 frames)", t.elapsed().as_secs_f64() * 1000.0);
    let g = group_of(d.app());
    assert_eq!(d.app().inside(g).len(), 53, "51 nodes and the two port nodes");
    assert_eq!(wires(d.app()), before);
    d.frames(2);
    d.shot("groups_jiku_top");
    let m = d.app().nodes.iter().position(|n| matches!(n.kind, Kind::AudioMatch { .. })).unwrap();
    d.click("▶ Run");
    let started = Instant::now();
    let (mut worst, mut n) = (0.0f64, 0);
    let end = Instant::now() + Duration::from_secs(180);
    while Instant::now() < end && !d.app().nodes[m].state.lock().unwrap().done {
        let t = Instant::now();
        d.h.step();
        worst = worst.max(t.elapsed().as_secs_f64());
        n += 1;
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(d.app().nodes[m].state.lock().unwrap().done, "the match inside the group ends");
    println!("{n} frames while it ran, worst {:.0} ms; the match took {:.1} s", worst * 1000.0, started.elapsed().as_secs_f64());
    d.frames(3);
    println!("{} links; {}", d.app().ev.links.len(), d.app().nodes[m].state.lock().unwrap().status);
    d.frames(3);
    d.shot("groups_jiku_top_matched");
    // Inside, framed.
    d.app().pick_only(g);
    let t = Instant::now();
    key(&mut d, Modifiers::NONE, Key::Tab);
    println!("Tab into 53 nodes: {:.0} ms (3 frames)", t.elapsed().as_secs_f64() * 1000.0);
    d.frames(3);
    d.shot("groups_jiku_inside");
    // Saved and opened again: still one group of 51.
    assert!(d.app().save());
    let mut e = Drive::new();
    e.app().open_project(file);
    e.frames(3);
    let g2 = group_of(e.app());
    assert_eq!(e.app().inside(g2).len(), 53);
    d.app().stop_run();
    d.wait_until(10.0, |a| !a.running());
    assert!(worst < 2.0, "no frame takes seconds: worst {worst:.1} s");
}

/// Add node → Groups adds another use of a group; right-click → Make single gives one use a
/// definition of its own, after which changes inside the first no longer reach it.
#[test]
fn add_use_from_menu_and_make_single() {
    let Scene { mut d, inv, comb, .. } = scene();
    d.app().sel.picked = [inv, comb].into_iter().collect();
    key(&mut d, Modifiers::COMMAND, Key::G);
    let g1 = group_of(d.app());
    // Right-click on empty canvas, low on the left, and pick the group under GROUPS.
    let o = d.app().canvas_origin;
    d.right_click_at(o.x + 300.0, o.y + 600.0);
    d.shot("groups_11_add_menu");
    // The menu's button (the group node shows its definition's name too).
    d.h.get_all_by_role(egui::accesskit::Role::Button).filter(|n| n.accesskit_node().label().as_deref() == Some("Group")).last().unwrap().click();
    d.frames(3);
    let uses: Vec<NodeId> = (0..d.app().nodes.len()).filter(|&i| !d.app().nodes[i].hidden && matches!(d.app().nodes[i].kind, Kind::Group { .. })).collect();
    assert_eq!(uses.len(), 2, "{}", d.msg());
    let g2 = uses[1];
    assert_eq!(d.app().group_uses(g1), 2);
    d.frames(3);
    // Make single, from the second use's menu.
    let t = d.node_title(g2);
    d.right_click_at(t.x, t.y);
    d.click("Make single");
    assert_eq!(d.app().group_uses(g1), 1);
    assert_eq!(d.app().group_uses(g2), 1);
    assert_eq!(d.app().nodes[g2].label, "Group 2");
    // A change inside the first stays there.
    d.app().nodes[inv].label = "Only here".into();
    d.frames(3);
    let inv2 = d.app().inside(g2).into_iter().find(|&x| matches!(d.app().nodes[x].kind, Kind::InvertLocation)).unwrap();
    assert_eq!(d.app().nodes[inv2].label, "Invert location");
    d.shot("groups_12_single");
}

/// By mouse only: a box dragged round two nodes, right-click → Group, a double-click opens it,
/// the path's Top goes back out, right-click → Ungroup.
#[test]
fn group_by_mouse() {
    let Scene { mut d, inv, comb, .. } = scene();
    let before = wires(d.app());
    // A box from empty canvas over Invert and Combine (right-drag, as box select does).
    let app = d.app();
    let (ri, rc) = (app.node_rect(app.canvas_origin, inv), app.node_rect(app.canvas_origin, comb));
    // From empty canvas right of Combine and above Invert, to inside Invert's left part.
    let from = Pos2::new(rc.right() + 20.0, ri.top() - 15.0);
    let to = Pos2::new(ri.center().x, rc.bottom() - 20.0);
    d.h.hover_at(from);
    d.h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    d.frames(1);
    for k in 1..=6 {
        d.h.hover_at(from + (to - from) * (k as f32 / 6.0));
        d.frames(1);
    }
    d.h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    d.frames(2);
    assert_eq!(d.app().picked(), vec![inv, comb], "the box picks the two");
    let t = d.node_title(inv);
    d.right_click_at(t.x, t.y);
    d.click("Group (Ctrl+G)");
    let g = group_of(d.app());
    assert_eq!(d.app().nodes[comb].parent, Some(g));
    // Double-click its title (a moment after the clicks before, or egui counts a triple click).
    d.frames(20);
    let t = d.node_title(g);
    d.h.hover_at(t);
    // The harness's clock moves 0.25 s a frame: egui's 0.3 s for a double-click is made 1.5 s
    // here, so a click a frame (press and release in one) is a double-click.
    d.h.ctx.options_mut(|o| o.input_options.max_double_click_delay = 1.5);
    for _ in 0..2 {
        for pressed in [true, false] {
            d.h.event(egui::Event::PointerButton { pos: t, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        }
        d.h.step();
    }
    d.frames(3);
    let (view, msg) = (d.app().view, d.msg());
    assert_eq!(view, Some(g), "a double-click opens the group: {msg}");
    d.click("Top");
    assert_eq!(d.app().view, None);
    let t = d.node_title(g);
    d.right_click_at(t.x, t.y);
    d.click("Ungroup (Alt+G)");
    let app = d.app();
    assert!(app.nodes[g].hidden && app.nodes[inv].parent.is_none());
    assert_eq!(wires(d.app()), before);
    d.shot("groups_14_by_mouse");
}

/// A port renamed in the group's menu: renamed in every use, each keeping its wires outside.
#[test]
fn rename_a_port() {
    let Scene { mut d, a, b, inv, comb, .. } = scene();
    d.app().sel.picked = [inv, comb].into_iter().collect();
    key(&mut d, Modifiers::COMMAND, Key::G);
    let g1 = group_of(d.app());
    d.app().pick_only(g1);
    key(&mut d, Modifiers::COMMAND, Key::D);
    let g2 = (0..d.app().nodes.len()).rfind(|&i| !d.app().nodes[i].hidden && matches!(d.app().nodes[i].kind, Kind::Group { .. })).unwrap();
    let out_b = port(&mut d, b, 0, true);
    let in1 = port(&mut d, g2, 0, false);
    d.drag(out_b, in1);
    let inv2 = d.app().inside(g2).into_iter().find(|&x| matches!(d.app().nodes[x].kind, Kind::InvertLocation)).unwrap();
    assert_eq!(d.app().graph.feeder(inv2, "location").map(|f| f.0), Some(b));
    // The menu lists the ports to rename.
    let t = d.node_title(g1);
    d.right_click_at(t.x, t.y);
    assert!(d.dump().contains("PORTS IN"), "{}", d.dump());
    d.shot("groups_13_port_names");
    d.h.key_press(Key::Escape);
    d.frames(2);
    d.app().rename_port(g1, false, 0, "area to flip");
    d.frames(3);
    let app = d.app();
    for g in [g1, g2] {
        assert_eq!(app.graph.nodes()[g].spec.inputs[0].name, "area to flip");
    }
    assert_eq!(app.graph.feeder(inv2, "location").map(|f| f.0), Some(b), "the other use keeps its wire");
    assert_eq!(app.graph.feeder(inv, "location").map(|f| f.0), Some(a));
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
