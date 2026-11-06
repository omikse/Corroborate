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
