//! Canvas editing driven like a user would (canvas.rs): box select, Shift-click, moving several
//! nodes, middle-drag pan, Delete, copy/cut/paste/duplicate, undo/redo, frame all. Headless;
//! screenshots in target/drive/.

use super::drive::Drive;
use super::*;
use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable;

/// Three nodes: Invert location → Combine locations → Map.
fn three() -> (Drive, NodeId, NodeId, NodeId) {
    let mut d = Drive::new();
    let inv = d.app().add(Kind::InvertLocation, Pos2::new(40.0, 60.0));
    let comb = d.app().add(Kind::CombineLocations { any: false }, Pos2::new(40.0, 330.0));
    let map = d.app().add(Kind::Map, Pos2::new(700.0, 60.0));
    d.app().wire(inv, "location", comb, "location 1");
    d.app().wire(comb, "location", map, "location");
    d.frames(3);
    (d, inv, comb, map)
}

/// Presses `button` at `from`, moves to `to` in steps, releases.
fn drag(d: &mut Drive, from: Pos2, to: Pos2, button: PointerButton, modifiers: Modifiers) {
    d.h.hover_at(from);
    d.h.event(Event::PointerButton { pos: from, button, pressed: true, modifiers });
    d.frames(1);
    for k in 1..=6 {
        d.h.hover_at(from + (to - from) * (k as f32 / 6.0));
        d.frames(1);
    }
    d.h.event(Event::PointerButton { pos: to, button, pressed: false, modifiers });
    d.frames(2);
}

fn key(d: &mut Drive, m: Modifiers, k: Key) {
    d.h.key_press_modifiers(m, k);
    d.frames(2);
}

/// Lets a change settle into an undo step.
fn settle(d: &mut Drive) {
    d.wait(0.5);
}

fn visible(d: &mut Drive) -> usize {
    d.app().nodes.iter().filter(|n| !n.hidden).count()
}

#[test]
fn box_select_delete_undo_redo() {
    let (mut d, inv, comb, map) = three();
    settle(&mut d);
    // A box over the two nodes on the left, starting on empty canvas.
    let o = d.app().canvas_origin;
    drag(&mut d, o + Vec2::new(10.0, 20.0), o + Vec2::new(300.0, 500.0), PointerButton::Primary, Modifiers::NONE);
    assert_eq!(d.app().picked(), vec![inv, comb], "box picks the two it touches");
    assert_eq!(d.app().pan, Vec2::ZERO, "a plain drag on empty canvas does not pan");
    d.shot("canvas_box_selected");
    key(&mut d, Modifiers::NONE, Key::Delete);
    assert_eq!(visible(&mut d), 1, "Delete removes both");
    assert!(d.app().graph.wires().is_empty(), "their wires go with them");
    key(&mut d, Modifiers::COMMAND, Key::Z);
    assert_eq!(visible(&mut d), 3, "undo brings them back");
    assert_eq!(d.app().graph.wires().len(), 2, "with their wires");
    key(&mut d, Modifiers::COMMAND, Key::Y);
    assert_eq!(visible(&mut d), 1, "redo deletes again");
    key(&mut d, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    assert_eq!(d.msg(), "nothing to redo");
    key(&mut d, Modifiers::COMMAND, Key::Z);
    assert!(!d.app().nodes[map].hidden && !d.app().nodes[inv].hidden);
}

#[test]
fn shift_click_and_move_together() {
    let (mut d, inv, comb, map) = three();
    settle(&mut d);
    let a = d.node_title(inv);
    d.click_at(a.x, a.y);
    let b = d.node_title(map);
    d.h.hover_at(b);
    d.frames(1);
    for pressed in [true, false] {
        d.h.event_modifiers(Event::PointerButton { pos: b, button: PointerButton::Primary, pressed, modifiers: Modifiers::SHIFT }, Modifiers::SHIFT);
        d.frames(1);
    }
    d.frames(2);
    assert_eq!(d.app().picked(), vec![inv, map], "Shift-click adds");
    let (p_inv, p_map, p_comb) = (d.app().nodes[inv].pos, d.app().nodes[map].pos, d.app().nodes[comb].pos);
    drag(&mut d, a, a + Vec2::new(60.0, 40.0), PointerButton::Primary, Modifiers::NONE);
    let moved = d.app().nodes[inv].pos - p_inv;
    assert!(moved.length() > 50.0, "dragged node moved {moved:?}");
    assert_eq!(d.app().nodes[map].pos - p_map, moved, "the other selected node moved the same");
    assert_eq!(d.app().nodes[comb].pos, p_comb, "the unselected node stayed");
    // Moving is one undo step.
    settle(&mut d);
    key(&mut d, Modifiers::COMMAND, Key::Z);
    assert_eq!((d.app().nodes[inv].pos, d.app().nodes[map].pos), (p_inv, p_map), "undo puts both back");
}

#[test]
fn middle_drag_pans() {
    let (mut d, ..) = three();
    let o = d.app().canvas_origin;
    drag(&mut d, o + Vec2::new(400.0, 600.0), o + Vec2::new(450.0, 630.0), PointerButton::Middle, Modifiers::NONE);
    let pan = d.app().pan;
    assert!((pan - Vec2::new(50.0, 30.0)).length() < 2.0, "pan {pan:?}");
    assert!(d.app().picked().is_empty(), "no box from a middle drag");
}

#[test]
fn copy_paste_cut_duplicate() {
    let (mut d, inv, comb, _map) = three();
    d.app().set_setting(comb, "mode", "any").unwrap();
    d.app().nodes[comb].label = "my combine".into();
    d.app().sel.picked = [inv, comb].into();
    d.h.event(Event::Copy);
    d.frames(2);
    let clip = d.app().sel.clip.clone();
    assert!(clip.starts_with("# corroborate project"), "{clip}");
    let o = d.app().canvas_origin;
    d.h.hover_at(o + Vec2::new(900.0, 400.0));
    d.h.event(Event::Paste(clip.clone()));
    d.frames(2);
    assert_eq!(d.app().nodes.len(), 5, "two pasted");
    let (pi, pc) = (3, 4);
    assert!(matches!(d.app().nodes[pi].kind, Kind::InvertLocation));
    assert!(matches!(d.app().nodes[pc].kind, Kind::CombineLocations { any: true }), "settings come along");
    assert_eq!(d.app().nodes[pc].label, "my combine");
    assert_eq!(d.app().graph.feeder(pc, "location 1"), Some((pi, "location")), "the wire between them comes along");
    assert_eq!(d.app().graph.feeder(pi, "location"), None, "wires to nodes not copied do not");
    assert_eq!(d.app().picked(), vec![pi, pc], "pasted nodes are selected");
    let x = d.app().nodes[pi].pos.x;
    assert!((880.0..920.0).contains(&x), "pasted where the pointer is: {x}");
    // Text from elsewhere on the clipboard: the last copy pastes.
    d.h.event(Event::Paste("hello".into()));
    d.frames(2);
    assert_eq!(visible(&mut d), 7, "plain text pastes the last copy");
    // Duplicate and cut.
    key(&mut d, Modifiers::COMMAND, Key::D);
    assert_eq!(visible(&mut d), 9);
    d.h.event(Event::Cut);
    d.frames(2);
    assert_eq!(visible(&mut d), 7, "cut removes the selection");
    d.shot("canvas_pasted");
}

#[test]
fn undo_of_a_setting_is_logged() {
    let (mut d, _inv, comb, _map) = three();
    settle(&mut d);
    d.app().set_setting(comb, "mode", "any").unwrap();
    settle(&mut d);
    let logged = d.app().changes.len();
    key(&mut d, Modifiers::COMMAND, Key::Z);
    assert!(matches!(d.app().nodes[comb].kind, Kind::CombineLocations { any: false }), "undo puts the setting back");
    let c = d.app().changes.last().unwrap();
    assert_eq!((c.name.as_str(), c.old.as_str(), c.new.as_str()), ("mode", "any", "all"));
    assert_eq!(d.app().changes.len(), logged + 1);
}

#[test]
fn keys_leave_text_boxes_alone() {
    let mut d = Drive::new();
    let man = d.app().add(Kind::Value { ty: "text".into(), rows: vec![("value".into(), "abc".into())] }, Pos2::new(40.0, 60.0));
    d.frames(3);
    d.app().pick_only(man);
    d.type_into(0, "xyz");
    key(&mut d, Modifiers::NONE, Key::Backspace);
    assert_eq!(visible(&mut d), 1, "Backspace in a text box deletes text, not the node");
}

#[test]
fn select_all_and_frame() {
    let (mut d, ..) = three();
    key(&mut d, Modifiers::COMMAND, Key::A);
    assert_eq!(d.app().picked().len(), 3);
    key(&mut d, Modifiers::NONE, Key::Escape);
    assert!(d.app().picked().is_empty());
    d.app().pan = Vec2::new(-3000.0, -2000.0);
    key(&mut d, Modifiers::NONE, Key::F);
    let area = Rect::from_min_size(d.app().canvas_origin, Vec2::new(1400.0, 500.0));
    let o = d.app().canvas_origin;
    let r = d.app().node_rect(o, 2);
    assert!(area.expand(400.0).contains_rect(r), "F brings the nodes back on screen: {r:?}");
    d.shot("canvas_framed");
}

#[test]
fn a_dot_says_its_type() {
    let (mut d, inv, comb, _map) = three();
    let p = d.app().graph.nodes()[comb].spec.inputs[0].clone();
    let t = d.app().port_hover(comb, &p, false);
    assert!(t.starts_with("location 1 · Location (value, input, optional)"), "{t}");
    assert!(t.contains("An area of the Earth") && t.contains("now: no location wired in"), "{t}");
    let o = d.app().graph.nodes()[inv].spec.outputs[0].clone();
    assert!(d.app().port_hover(inv, &o, true).contains("(value, output)"));
    let (c, _) = { let a = d.app(); a.port_place(a.canvas_origin, comb, 0, false) };
    d.h.hover_at(c);
    d.frames(30);
    d.shot("canvas_dot_hover");
}

#[test]
fn types_menu() {
    let (mut d, ..) = three();
    d.click("Types");
    for t in ["Audio", "Location", "Event", "JSON"] {
        assert!(d.h.query_by_label(t).is_some(), "{t} is listed");
    }
    let loc = d.h.get_by_label("Location").rect().center();
    d.h.hover_at(loc);
    d.frames(30);
    d.shot("types_menu");
    let (give, take) = types_menu::users("location");
    assert!(give.iter().any(|m| m == "Combine locations") && take.iter().any(|m| m == "Map"), "{give:?} {take:?}");
}

/// A Value node of type location draws its circle on a Map; one of type time goes into the
/// Result record's file; the Types menu adds one of the type clicked; changing its type starts
/// its fields again and drops wires that no longer fit.
#[test]
fn value_nodes() {
    let mut d = Drive::new();
    let place = d.app().add(crate::drive::place(48.85, 2.35, 800.0), Pos2::new(30.0, 40.0));
    let map = d.app().add(Kind::Map, Pos2::new(420.0, 40.0));
    d.app().wire(place, "value", map, "location");
    let mut time = Kind::Value { ty: "time".into(), rows: corroborate::modules::value::defaults("time") };
    if let Kind::Value { rows, .. } = &mut time {
        rows[0].1 = "2026-11-04 20:06:39".into();
    }
    let time = d.app().add(time, Pos2::new(30.0, 330.0));
    let rec = d.app().add(Kind::ResultRecord { folder: String::new() }, Pos2::new(800.0, 330.0));
    d.app().wire_record(time, "value", rec);
    d.frames(5);
    assert!(d.app().map_status(map).contains('%'), "the map shows the place: {}", d.app().map_status(map));
    let json = d.app().result_json(&[time, rec]);
    assert!(json.contains("\"utc0\":1793822799"), "{json}");
    assert!(json.contains("\"type\":\"time\",\"value\":{\"utc0\": 1793822799"), "{json}");
    d.shot("value_place_and_time");

    // Types → Event adds a Value node of events.
    d.click("Types");
    d.click("Event");
    let ev = d.app().nodes.len() - 1;
    assert!(matches!(&d.app().nodes[ev].kind, Kind::Value { ty, .. } if ty == "event"));
    assert_eq!(d.app().graph.nodes()[ev].spec.outputs[0].kind, PortKind::Event);
    d.click("+ item");
    let items = |d: &mut Drive| match &d.app().nodes[ev].kind {
        Kind::Value { rows, .. } => corroborate::modules::value::items(rows).len(),
        _ => 0,
    };
    assert_eq!(items(&mut d), 2, "+ item adds an item");
    d.shot("value_events");

    // The place becomes a text: its fields start again and the wire to the map goes.
    d.app().set_value_type(place, "text");
    assert!(matches!(&d.app().nodes[place].kind, Kind::Value { ty, rows } if ty == "text" && rows == &[("value".to_string(), String::new())]));
    assert_eq!(d.app().graph.feeder(map, "location"), None, "a text does not fit the map");
    assert!(d.app().changes.iter().any(|c| c.node == place && c.name == "type" && c.old == "location" && c.new == "text"), "logged");
}

/// A wire dragged from a dot onto empty canvas offers the modules that fit its type; the one
/// picked is added there and wired. From an input, a Value node of its type comes first. A wire
/// dragged from an input onto an output wires them too.
#[test]
fn drag_from_a_dot_to_empty_canvas() {
    let mut d = Drive::new();
    let map = d.app().add(Kind::Map, Pos2::new(600.0, 60.0));
    d.frames(3);
    let (c, _) = { let a = d.app(); a.port_place(a.canvas_origin, map, 0, false) };
    drag(&mut d, c, c - Vec2::new(250.0, -150.0), PointerButton::Primary, Modifiers::NONE);
    let choices: Vec<String> = d.app().drop_choices(crate::drop_menu::End::In(map, 0)).into_iter().map(|(l, _)| l).collect();
    assert_eq!(choices[0], "Value (Location)", "{choices:?}");
    assert!(choices.iter().any(|l| l == "Combine locations (all)") && !choices.iter().any(|l| l == "Map"), "{choices:?}");
    d.shot("drop_menu_from_input");
    d.click("Value (Location)");
    let v = d.app().nodes.len() - 1;
    assert!(matches!(&d.app().nodes[v].kind, Kind::Value { ty, .. } if ty == "location"));
    assert_eq!(d.app().graph.feeder(map, "location"), Some((v, "value")), "the new Value feeds the map");
    assert!(d.app().nodes[v].pos.x < d.app().nodes[map].pos.x, "it sits left of the map");

    // From the Value's output onto empty canvas: Invert location takes a location.
    d.frames(3);
    let (c, _) = { let a = d.app(); a.port_place(a.canvas_origin, v, 0, true) };
    drag(&mut d, c, c + Vec2::new(200.0, 120.0), PointerButton::Primary, Modifiers::NONE);
    d.shot("drop_menu_from_output");
    d.click("Invert location");
    let inv = d.app().nodes.len() - 1;
    assert!(matches!(d.app().nodes[inv].kind, Kind::InvertLocation));
    assert_eq!(d.app().graph.feeder(inv, "location"), Some((v, "value")));

    // From the map's input back onto Invert's output: rewired.
    d.frames(3);
    let (a, b) = { let x = d.app(); (x.port_place(x.canvas_origin, map, 0, false).0, x.port_place(x.canvas_origin, inv, 0, true).0) };
    drag(&mut d, a, b, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(d.app().graph.feeder(map, "location"), Some((inv, "location")), "{}", d.msg());
    d.shot("drop_menu_wired");
}
