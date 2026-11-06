//! Looks from module.toml and look.rhai: ports moved by a manifest, a port a script hides until a
//! setting is filled, broken and endless scripts that only mark their own node, and scripts that
//! do not run while the canvas pans and zooms.
//! Run: cargo test --bin corroborate look_tests -- --nocapture

use super::*;
use crate::drive::Drive;
use corroborate::module_manifest::{self as mm, Side};

/// A module on the bus (it needs no program for its look) with `extra` module.toml lines and
/// `script` as its look.rhai.
fn module(id: &str, extra: &str, script: Option<&str>) -> Kind {
    let text = format!(
        "id = \"{id}\"\nlabel = \"{id}\"\nrole = \"transform\"\n\
         [[input]]\nname = \"audio\"\nkind = \"audio\"\n\
         [[input]]\nname = \"place\"\nkind = \"location\"\n\
         [[output]]\nname = \"seen\"\nkind = \"event\"\n\
         [[setting]]\nname = \"label\"\ntype = \"text\"\n\
         [[setting]]\nname = \"loud\"\ntype = \"bool\"\n\
         [[setting]]\nname = \"mode\"\ntype = \"choice\"\nchoices = [\"fast\", \"slow\"]\n\
         {extra}\n[processing]\nprogram = \"no-such-program.exe\"\nprotocol = 2\n"
    );
    let mut m = mm::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    m.script = script.map(str::to_string);
    Kind::module(m).unwrap()
}

fn rect_of(d: &mut Drive, i: NodeId) -> Rect {
    let a = d.app();
    a.node_rect(a.canvas_origin, i)
}

/// module.toml moves `audio` to the bottom edge, 40 px from the left, with a square dot; the
/// wire into it leaves downwards.
#[test]
fn manifest_moves_a_port() {
    let mut d = Drive::new();
    let a = d.app();
    let src = a.add(Kind::Sourcer { input: crate::drive::tone_wav().to_string_lossy().into_owned() }, Pos2::new(30.0, 40.0));
    let n = a.add(module("bottom-audio", "[look]\nfill = \"#203040\"\nradius = 14\n[look.ports.audio]\nside = \"bottom\"\nat = 40\ndot = \"square\"\nlabel = \"outside\"", None), Pos2::new(560.0, 60.0));
    assert!(d.wait_until(30.0, |a| !a.graph.nodes()[src].spec.outputs.is_empty()), "the source opened");
    d.app().wire(src, "audio", n, "audio");
    assert_eq!(d.app().graph.wires().len(), 1);
    d.frames(3);
    let r = rect_of(&mut d, n);
    let a = d.app();
    let (p, side) = a.port_place(a.canvas_origin, n, 0, false);
    assert_eq!(side, Side::Bottom);
    assert!((p.y - r.bottom()).abs() < 0.01 && (p.x - (r.left() + 40.0)).abs() < 0.01, "{p:?} on {r:?}");
    // The other input keeps its place: alone on the left edge, in the middle.
    let (q, side) = a.port_place(a.canvas_origin, n, 1, false);
    assert_eq!(side, Side::Left);
    assert!((q.y - r.center().y).abs() < 0.01);
    d.shot("look_port_bottom");
}

const HIDE_UNTIL_LABEL: &str = r##"
// The "seen" output shows once a label is set.
fn layout(node) {
    #{ ports: #{ seen: #{ hidden: node.settings.label == "" } } }
}
fn draw(node) {
    [
        text(4, 2, "label:", #{ size: 11, color: "#9aa3ad" }),
        field("label", 50, 0, 120),
        checkbox("loud", 4, 24),
        dropdown("mode", 50, 24, 90),
        rect(4, 50, node.width - 8, 6, #{ fill: "#3a4860", radius: 3 }),
        circle(10, 70, 5, #{ fill: "#78aaff" }),
        line(20, 70, 120, 70, #{ color: "#78aaff", width: 2 }),
        text(130, 64, if node.settings.label == "" { "no label" } else { node.settings.label }, #{}),
    ]
}
"##;

/// A look.rhai hides an output until its setting is filled; the fields it draws edit settings
/// through the same checks as everywhere.
#[test]
fn script_hides_a_port_until_a_field_is_filled() {
    let mut d = Drive::new();
    let n = d.app().add(module("hide-until", "[look]\nsize = [300, 150]", Some(HIDE_UNTIL_LABEL)), Pos2::new(60.0, 60.0));
    d.frames(3);
    let shown = |d: &mut Drive| {
        let a = d.app();
        let p = a.graph.nodes()[n].spec.outputs[0].clone();
        let l = a.port_look(n, &p);
        a.port_shown(n, &p, true, &l)
    };
    assert!(!shown(&mut d), "hidden while the label is empty");
    assert_eq!(rect_of(&mut d, n).size(), Vec2::new(300.0, 150.0), "[look] size");
    d.shot("look_script_hidden");
    d.type_into(0, "claps");
    d.frames(2);
    assert_eq!(d.app().nodes[n].kind.rows()[0].1, "claps", "the script's field set the setting");
    assert!(shown(&mut d), "shown once the label is set");
    d.shot("look_script_shown");
}

/// A script that does not compile, and one that never ends, each show an error on their own
/// node; the node beside them draws as usual.
#[test]
fn bad_scripts_mark_only_their_node() {
    let mut d = Drive::new();
    let a = d.app();
    let broken = a.add(module("broken", "", Some("fn draw(node) { [ text(1, 2, \"x\" ]")), Pos2::new(30.0, 60.0));
    let endless = a.add(module("endless", "", Some("fn draw(node) { let x = 0; loop { x += 1; } }")), Pos2::new(400.0, 60.0));
    let good = a.add(module("good", "", Some(HIDE_UNTIL_LABEL)), Pos2::new(30.0, 320.0));
    let t = Instant::now();
    d.frames(3);
    assert!(t.elapsed() < Duration::from_secs(20), "the endless script was stopped");
    let err = |d: &mut Drive, i| d.app().scripts.by_node.get(&i).and_then(|o| o.error.clone());
    let e = err(&mut d, broken).expect("broken shows an error");
    assert!(e.contains("line"), "with where: {e}");
    let e = err(&mut d, endless).expect("endless shows an error");
    assert!(e.to_lowercase().contains("operations"), "{e}");
    assert_eq!(err(&mut d, good), None);
    assert!(d.app().scripts.by_node[&good].items.as_ref().is_some_and(|v| v.len() == 8));
    d.shot("look_bad_scripts");
}

/// Twenty scripted nodes: once drawn, panning and zooming the canvas runs no script at all; a
/// change to one node runs only that node's script.
#[test]
fn panning_runs_no_scripts() {
    let mut d = Drive::new();
    let mut ids = Vec::new();
    for k in 0..20 {
        let n = d.app().add(module("pan-test", "", Some(HIDE_UNTIL_LABEL)), Pos2::new(30.0 + (k % 5) as f32 * 260.0, 40.0 + (k / 5) as f32 * 150.0));
        ids.push(n);
    }
    // The graph restarts 0.5 s after nodes are added (their status changes, so their scripts
    // run again): let it settle first.
    d.wait(0.8);
    d.frames(2);
    let before = d.app().scripts.calls;
    assert_eq!(d.app().scripts.by_node.len(), 20, "each node's script ran");
    for f in 0..20 {
        let a = d.app();
        a.pan += Vec2::new(3.0, -2.0);
        a.zoom = 0.8 + (f % 5) as f32 * 0.05;
        d.frames(1);
    }
    assert_eq!(d.app().scripts.calls, before, "pan and zoom ran no script");
    d.app().set_setting(ids[3], "label", "x").unwrap();
    d.frames(2);
    assert_eq!(d.app().scripts.calls, before + 2, "one node changed: its layout() and draw() ran once");
}

/// The cost of scripts: a frame panning over 20 scripted nodes, and a frame where all 20 changed.
/// Run: cargo test --release --bin corroborate script_timing -- --ignored --nocapture
#[test]
#[ignore]
fn script_timing() {
    let ctx = egui::Context::default();
    let mut app = App::empty();
    let mut ids = Vec::new();
    for k in 0..20 {
        ids.push(app.add(module("timing", "", Some(HIDE_UNTIL_LABEL)), Pos2::new(30.0 + (k % 5) as f32 * 260.0, 40.0 + (k / 5) as f32 * 150.0)));
    }
    let input = || egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 780.0))), ..Default::default() };
    let frame = |app: &mut App| {
        let out = ctx.run_ui(input(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| app.graph_view(ui));
        });
        std::hint::black_box(ctx.tessellate(out.shapes, out.pixels_per_point).len());
    };
    for _ in 0..3 {
        frame(&mut app);
    }
    let (calls, frames) = (app.scripts.calls, 30);
    let t = Instant::now();
    for f in 0..frames {
        app.zoom = 0.6 + (f % 10) as f32 * 0.08;
        app.pan += Vec2::new(2.0, 1.0);
        frame(&mut app);
    }
    println!("pan/zoom frame, 20 scripted nodes: {:.2} ms, script calls {}", t.elapsed().as_secs_f64() * 1000.0 / frames as f64, app.scripts.calls - calls);
    let spent = app.scripts.time;
    for f in 0..frames {
        for &i in &ids {
            app.nodes[i].state.lock().unwrap().outbox_rev += 1;
        }
        let _ = f;
        frame(&mut app);
    }
    println!("all 20 changed: {:.3} ms of script per frame", (app.scripts.time - spent).as_secs_f64() * 1000.0 / frames as f64);
}

/// A look.rhai changed while the program runs is compiled again at the next refresh (each
/// second) and the node drawn from the new one.
#[test]
fn edited_script_is_picked_up() {
    let mut d = Drive::new();
    let n = d.app().add(module("hot", "", Some("fn draw(node) { [ text(4, 4, \"first\") ] }")), Pos2::new(60.0, 60.0));
    d.frames(3);
    let text = |d: &mut Drive| match d.app().scripts.by_node.get(&n).and_then(|o| o.items.clone()).as_deref() {
        Some([script::Item::Text { text, .. }]) => text.clone(),
        other => format!("{other:?}"),
    };
    assert_eq!(text(&mut d), "first");
    let a = d.app();
    if let Kind::Module { m, .. } = &mut a.nodes[n].kind {
        let mut new = (**m).clone();
        new.script = Some("fn draw(node) { [ text(4, 4, \"second\") ] }".into());
        *m = Arc::new(new);
    }
    a.looks.remove("hot");
    d.frames(2);
    assert_eq!(text(&mut d), "second");
}
