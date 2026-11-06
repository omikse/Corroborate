//! The project's clocks (evidence.rs): which clock each node's times are on, the evidence nodes
//! give, the solve, Same clock, the on/off switches, and how they are saved.
//! Run: cargo test --bin corroborate clock_tests

use super::*;
use corroborate::clock::Kind as Ck;
use corroborate::data::Data;
use corroborate::module_manifest as mm;

fn source(app: &mut App, y: f32) -> NodeId {
    app.add(Kind::Sourcer { input: String::new() }, Pos2::new(30.0, y))
}

fn anchor(app: &App, i: NodeId, utc0: f64, error_s: f64) {
    app.nodes[i].state.lock().unwrap().anchor = Some(Anchor { utc0, error_s, source: "test".into() });
}

fn uid(app: &App, i: NodeId) -> String {
    app.nodes[i].uid.clone()
}

/// A source's anchor makes its clock absolute; Same clock puts another source on it, and it is
/// saved with the project; Separate clocks undoes it.
#[test]
fn sources_anchor_and_share_clocks() {
    let mut app = App::empty();
    let (a, b) = (source(&mut app, 40.0), source(&mut app, 300.0));
    anchor(&app, a, 1000.0, 2.0);
    app.update_clocks();
    assert_eq!(app.ev.anchors.len(), 1);
    let (ua, ub) = (uid(&app, a), uid(&app, b));
    let (utc, err) = app.ev.clocks.to_utc(&ua, 5.0).expect("absolute");
    assert!((utc - 1005.0).abs() < 1e-6 && (err - 2.0).abs() < 1e-6, "{utc} ± {err}");
    assert_eq!(app.ev.clocks.get(&ub).kind, Ck::None);

    app.same_clock(&[a, b]);
    assert_eq!(app.clock_of(b), Some(ua.clone()), "b is on a's clock");
    assert!(app.shares_clock(a) && app.shares_clock(b));
    app.update_clocks();
    assert!(app.ev.clocks.to_utc(&app.clock_of(b).unwrap(), 5.0).is_some(), "b's times are on UTC now");

    app.ev.off.insert("anchor n9 n1 0.000".into());
    app.ev.manual_anchors.push(corroborate::clock::Anchor { clock: ub.clone(), t: 3.0, utc: 2000.0, error_s: 0.5, from: "manual".into(), enabled: true });
    app.ev.manual_links.push(corroborate::clock::Link { a: ua.clone(), t_a: 1.0, b: "n7".into(), t_b: 2.0, error_s: 0.25, from: "manual".into(), enabled: true });
    let mut back = App::empty();
    assert!(back.load_project_text(&app.project_text()));
    assert_eq!(back.ev.share, app.ev.share);
    assert_eq!(back.ev.off, app.ev.off);
    assert_eq!(back.ev.manual_anchors, app.ev.manual_anchors);
    assert_eq!(back.ev.manual_links, app.ev.manual_links);
    // Pasting the nodes does not bring the project's clocks.
    let mut pasted = App::empty();
    pasted.append_text(&app.project_text(), false);
    assert!(pasted.ev.share.is_empty() && pasted.ev.manual_anchors.is_empty());

    app.separate_clocks(&[b]);
    assert_eq!(app.clock_of(b), Some(ub), "b has its own clock again");
    assert!(app.ev.share.is_empty(), "a shares with no one: back to its own");
}

/// A time wired into Planes' time input anchors the clock of its audio; switching that evidence
/// off unanchors it again, and the switch is logged.
#[test]
fn a_wired_time_anchors_the_audio() {
    let mut app = App::empty();
    let s = source(&mut app, 40.0);
    let p = app.add(Kind::Planes, Pos2::new(300.0, 40.0));
    let v = app.add(Kind::Value { ty: "time".into(), rows: vec![("utc".into(), "2026-11-06T12:00:00Z".into()), ("error_s".into(), "0.5".into())] }, Pos2::new(30.0, 300.0));
    app.wire(s, "audio", p, "audio");
    app.wire(v, "value", p, "time");
    assert_eq!(app.graph.wires().len(), 2, "{}", app.msg);
    app.update_clocks();
    let us = uid(&app, s);
    assert_eq!(app.ev.anchors.len(), 1, "{:?}", app.ev.anchors);
    let an = app.ev.anchors[0].clone();
    assert_eq!((an.clock.as_str(), an.t, an.error_s, an.from.clone()), (us.as_str(), 0.0, 0.5, uid(&app, p)));
    assert_eq!(app.ev.clocks.get(&us).kind, Ck::Absolute);

    app.set_evidence(&an.key(), false);
    app.update_clocks();
    assert_eq!(app.ev.clocks.get(&us).kind, Ck::None, "switched off");
    assert!(!app.ev.anchors[0].enabled);
    assert_eq!(app.changes.last().map(|c| (c.name.clone(), c.new.clone())), Some((an.key(), "off".into())));
}

/// Planes' passes are readable on its output, on the clock of its audio, with the fit's error.
#[test]
fn planes_passes_on_the_bus() {
    let mut app = App::empty();
    let s = source(&mut app, 40.0);
    let p = app.add(Kind::Planes, Pos2::new(300.0, 40.0));
    app.wire(s, "audio", p, "audio");
    let pass = corroborate::modules::planes::detect::Event {
        id: 4,
        t_cpa: 80.0,
        t_err: 0.3,
        t_start: 40.0,
        t_end: 120.0,
        f0: 112.0,
        f_pre: 130.0,
        f_post: 96.0,
        speed: 62.0,
        dist: 900.0,
        snr: 10.0,
        rms: 0.2,
        harmonics: 0,
        n_points: 300,
        points: Vec::new(),
    };
    app.nodes[p].state.lock().unwrap().planes.passes.push(pass);
    let Ok(Data::Events(v)) = app.value_from(p, "aircrafts") else { panic!("no passes") };
    assert_eq!((v[0].clock.clone(), v[0].t_start, v[0].error_s), (uid(&app, s), 80.0, 0.3));
}

/// A module's anchor output is evidence, wired or not; an anchor without a clock is on the
/// clock of the module's audio.
#[test]
fn anchor_outputs_are_evidence() {
    let m = mm::parse(
        r#"id = "anchorer"
label = "Anchorer"
category = "Checks"

[[input]]
name = "audio"
kind = "audio"

[[output]]
name = "anchors"
kind = "anchor"

[processing]
program = "none.exe"
protocol = 2
"#,
    )
    .unwrap();
    let mut app = App::empty();
    let s = source(&mut app, 40.0);
    let k = app.add(Kind::module(m).unwrap(), Pos2::new(300.0, 40.0));
    app.wire(s, "audio", k, "audio");
    app.nodes[k].state.lock().unwrap().outbox.push(("anchors", Data::Custom { kind: "anchor", json: r#"[{"t":3,"utc":500,"error_s":0.2}]"#.into() }));
    app.update_clocks();
    let (utc, err) = app.ev.clocks.to_utc(&uid(&app, s), 3.0).expect("anchored by the module");
    assert!((utc - 500.0).abs() < 1e-6 && (err - 0.2).abs() < 1e-6, "{utc} ± {err}");
    assert_eq!(app.ev.anchors[0].from, uid(&app, k));
}

/// Groups on the canvas: absolute first with the first colour, then relative (dashed), then
/// unanchored (faint); linked sources share one group.
#[test]
fn canvas_groups_colours_and_styles() {
    use crate::clock_view::{glow_style, PALETTE};
    let mut app = App::empty();
    let (a, b, c) = (source(&mut app, 40.0), source(&mut app, 300.0), source(&mut app, 560.0));
    anchor(&app, a, 1000.0, 1.0);
    let (ub, uc) = (uid(&app, b), uid(&app, c));
    app.update_clocks();
    let cg = app.canvas_groups();
    assert_eq!(cg.groups.len(), 3);
    assert_eq!((cg.groups[0].kind, cg.groups[0].color), (Ck::Absolute, PALETTE[0]));
    assert_eq!(cg.groups[1].kind, Ck::None);
    assert_ne!(cg.groups[1].color, cg.groups[2].color);
    assert!(glow_style(Ck::Absolute).0 > glow_style(Ck::None).0);
    assert!(glow_style(Ck::Relative).1 && !glow_style(Ck::Absolute).1);

    app.ev.manual_links.push(corroborate::clock::Link { a: ub, t_a: 0.0, b: uc, t_b: 5.0, error_s: 0.5, from: "manual".into(), enabled: true });
    app.update_clocks();
    let cg = app.canvas_groups();
    assert_eq!(cg.groups.iter().map(|g| g.kind).collect::<Vec<_>>(), vec![Ck::Absolute, Ck::Relative]);
    assert_eq!(cg.of_node[b], cg.of_node[c]);
    assert_ne!(cg.of_node[a], cg.of_node[b]);
}

/// The wire box: UTC time, count and span, error at each confidence, red over the maximum.
#[test]
fn wire_box_text_and_confidence() {
    use crate::clock_view::{wire_text, Confidence as Cf};
    use corroborate::clock::Anchor as An;
    use corroborate::data::Event;
    let clocks = corroborate::clock::solve(&[An { clock: "s".into(), t: 0.0, utc: 45_000.0, error_s: 2.0, from: "n1".into(), enabled: true }], &[]);
    let name = |c: &str| format!("clip {c}");
    let ev = |t: f64, e: f64| Event { clock: "s".into(), t_start: t, t_end: t, error_s: e, ..Default::default() };
    let events = Data::Events(vec![ev(60.0, 1.5), ev(540.0, 0.5)]);
    let tx = |d: &Data, cf, max| wire_text(&clocks, d, cf, max, &name).unwrap();

    let t = tx(&events, Cf::P95, None);
    assert_eq!(t.top, "2 events, 12:31–12:39");
    assert_eq!(t.bottom, "± 5.00 s"); // 2 sigma of sqrt(2² + 1.5²) = 2.5 s
    assert_eq!(tx(&events, Cf::P68, None).bottom, "± 2.50 s");
    assert_eq!(tx(&events, Cf::P997, None).bottom, "± 7.50 s");
    assert!(!t.over && tx(&events, Cf::P95, Some(4.0)).over && !tx(&events, Cf::P95, Some(6.0)).over);

    let rel = Data::Time { clock: "z".into(), t: 41.2, utc: None, error_s: 0.1, source: String::new() };
    assert_eq!(tx(&rel, Cf::P68, None).top, "+00:41.2 on clip z");
    let abs = Data::Time { clock: "s".into(), t: 0.0, utc: Some(45_003.0), error_s: 1.0, source: String::new() };
    assert_eq!((tx(&abs, Cf::P68, None).top.as_str(), tx(&abs, Cf::P68, None).bottom.as_str()), ("12:30:03 UTC", "± 1.00 s"));
}
