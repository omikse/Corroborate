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

/// Timeline rows: absolute first (placed on UTC), then relative (on the group's reference clock),
/// then unanchored; a floating row starts under the first absolute event.
#[test]
fn timeline_rows_order_and_placement() {
    use crate::timeline::{build_rows, default_shift};
    use corroborate::clock::{Anchor as An, Link};
    use corroborate::data::Event;
    let clocks = corroborate::clock::solve(
        &[An { clock: "a".into(), t: 0.0, utc: 1000.0, error_s: 2.0, from: "n1".into(), enabled: true }],
        &[Link { a: "b".into(), t_a: 0.0, b: "c".into(), t_b: 10.0, error_s: 1.0, from: "manual".into(), enabled: true }],
    );
    let ev = |c: &str, t: f64, e: f64| (0usize, Event { clock: c.into(), t_start: t, t_end: t + 1.0, error_s: e, ..Default::default() });
    // Listed unanchored first, to show the order comes from the kind.
    let events = [ev("z", 5.0, 0.0), ev("c", 3.0, 0.0), ev("b", 4.0, 0.0), ev("a", 30.0, 0.5)];
    let rows = build_rows(&clocks, &events, &|c| format!("clip {c}"));
    let kinds: Vec<Ck> = rows.iter().map(|r| r.group.kind).collect();
    assert_eq!(kinds, vec![Ck::Absolute, Ck::Relative, Ck::None]);
    let a = &rows[0].subs[0].events[0];
    assert_eq!((a.t0, a.clock_err, a.own_err), (1030.0, 2.0, 0.5));
    assert!((a.err() - 2.0f64.hypot(0.5)).abs() < 1e-9);
    // b and c share a group; both placed on the reference clock (the smaller id, b).
    assert_eq!(rows[1].subs.len(), 2);
    let on_ref: Vec<f64> = rows[1].subs.iter().map(|s| s.events[0].t0).collect();
    assert!((on_ref[0] - 4.0).abs() < 1e-6 && (on_ref[1] - -7.0).abs() < 1e-6, "{on_ref:?}");
    assert_eq!(rows[2].subs[0].events[0].t0, 5.0, "unanchored: as it is");
    // The floating row's first event starts under the first absolute one.
    let s = default_shift(&rows, &rows[2], 0.0);
    assert!((rows[2].first().unwrap() + s - 1030.0).abs() < 1e-9);
    assert_eq!(default_shift(&rows, &rows[0], 0.0), 0.0);
}

#[test]
fn sync_list_entries_share_and_contradictions() {
    use corroborate::clock::{solve, Anchor, Link};
    let an = |t: f64, utc: f64, from: &str, enabled: bool| Anchor { clock: "a".into(), t, utc, error_s: 1.0, from: from.into(), enabled };
    let name = |c: &str| c.to_string();
    let who = |f: &str| f.to_string();

    // Two agreeing anchors share the error evenly; the link ties b to a.
    let anchors = vec![an(0.0, 1000.0, "n1", true), an(10.0, 1010.0, "n2", true)];
    let links = vec![Link { a: "a".into(), t_a: 5.0, b: "b".into(), t_b: 2.0, error_s: 1.0, from: "manual".into(), enabled: true }];
    let clocks = solve(&anchors, &links);
    let e = crate::timeline::sync_entries(&anchors, &links, &clocks, 2.0, &name, &who);
    assert_eq!(e.len(), 3);
    assert!(e.iter().all(|x| x.contradiction.is_none()));
    assert!((e[0].share - e[1].share).abs() < 1e-6 && e[0].share > 0.0);
    assert_eq!(e[2].from, "manual");
    assert!(e[2].title.starts_with("link"));

    // Disagreeing anchors are flagged; a switched-off one has no share.
    let anchors = vec![an(0.0, 1000.0, "n1", true), an(0.0, 1100.0, "n2", true), an(0.0, 5000.0, "n3", false)];
    let clocks = solve(&anchors, &[]);
    let e = crate::timeline::sync_entries(&anchors, &[], &clocks, 2.0, &name, &who);
    assert!(e[0].contradiction.is_some() && e[1].contradiction.is_some());
    assert!(!e[2].enabled && e[2].share == 0.0 && e[2].contradiction.is_none());
}

/// A dragged row's event meets the closest event on another row within the tolerance; an absolute
/// row gives an anchor in UTC, a floating one a link in its own clock's time.
#[test]
fn drop_match_finds_the_closest_pair() {
    use crate::timeline::{build_rows, drop_match};
    use corroborate::clock::Anchor as An;
    use corroborate::data::Event;
    let clocks = corroborate::clock::solve(&[An { clock: "a".into(), t: 0.0, utc: 1000.0, error_s: 1.0, from: "n1".into(), enabled: true }], &[]);
    let ev = |c: &str, t: f64| (0usize, Event { clock: c.into(), t_start: t, t_end: t, ..Default::default() });
    let rows = build_rows(&clocks, &[ev("a", 10.0), ev("a", 20.0), ev("z", 3.0)], &|c| c.to_string());
    let off = |c: &str| clocks.get(c).offset;
    // z's event at 3 is drawn at 3 + shift: 1018 is 2 s short of a's event at 20 s (UTC 1020).
    let m = drop_match(&rows, "z", &|r| if r.group.id == "z" { 1015.0 } else { 0.0 }, &off, 3.0).unwrap();
    assert!(m.absolute && m.a == "z" && m.t_a == 3.0 && m.t_b == 1020.0 && (m.gap - 2.0).abs() < 1e-9, "{m:?}");
    assert!(drop_match(&rows, "z", &|r| if r.group.id == "z" { 1015.0 } else { 0.0 }, &off, 1.0).is_none());
}

/// Dragging a floating row so its event meets another floating row's event asks for the error;
/// confirming makes a manual link and the row joins that group.
#[test]
fn drag_a_row_onto_an_event_links_it() {
    use corroborate::data::Event;
    let mut d = crate::drive::Drive::new();
    let ev = |c: &str, t: f64| (0usize, Event { clock: c.into(), t_start: t, t_end: t, label: c.into(), ..Default::default() });
    let app = d.app();
    app.tls.extra = vec![ev("c1", 10.0), ev("c2", 5.0)];
    app.follow = false;
    app.tl = (0.0, 60.0);
    d.frames(3);
    let (track, v0, span) = d.app().tls.geom.unwrap();
    let y = d.app().tls.row_y.iter().find(|(g, _)| g == "c2").unwrap().1;
    let x = |t: f64| track.left() + ((t - v0) / span) as f32 * track.width();
    let (from, to) = (Pos2::new(x(5.0), y), Pos2::new(x(10.0) + 2.0, y));
    d.h.hover_at(from);
    d.h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    d.frames(1);
    for k in 1..=6 {
        d.h.hover_at(from + (to - from) * (k as f32 / 6.0));
        d.frames(1);
    }
    d.h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    d.frames(3);
    let p = d.app().tls.pending.clone().expect("the drop asks for the link's error");
    assert!(!p.absolute && p.a == "c2" && p.t_a == 5.0 && p.b == "c1" && p.t_b == 10.0 && p.error_s > 0.0, "{p:?}");
    d.shot("timeline_link_box");
    d.click("Link");
    d.frames(3);
    let app = d.app();
    assert_eq!(app.ev.manual_links.len(), 1);
    assert_eq!(app.ev.manual_links[0].from, "manual");
    let (g1, g2) = (app.ev.clocks.get("c1").group.clone(), app.ev.clocks.get("c2").group.clone());
    assert_eq!(g1, g2, "c2 joined c1's group");
    assert!(app.tls.pending.is_none() && !app.tls.shift.contains_key("c2"));
    d.shot("timeline_linked");
}

/// Moving the playhead in SOURCER's waveform moves it on the timeline and in Planes, which is on
/// the same clock (its audio comes from the source).
#[test]
fn playhead_links_sourcer_planes_and_timeline() {
    let mut d = crate::drive::Drive::new();
    let s = d.app().add(Kind::Sourcer { input: String::new() }, Pos2::new(40.0, 40.0));
    let pl = d.app().add(Kind::Planes, Pos2::new(560.0, 40.0));
    d.app().wire(s, "audio", pl, "audio");
    d.frames(3);
    d.type_into(0, &crate::drive::tone_wav().to_string_lossy());
    assert!(d.wait_until(30.0, |app| app.nodes[s].state.lock().map(|st| st.audio_s > 2.0).unwrap_or(false)), "the file decoded");
    d.app().pick_only(s);
    d.frames(3);
    let r: Rect = d.h.ctx.data(|m| m.get_temp(egui::Id::new(("wave-rect", s)))).expect("waveform drawn");
    d.click_at(r.center().x, r.center().y);
    let app = d.app();
    let clock = app.clock_of(s).unwrap();
    let t = app.playhead_on(&clock).expect("the click set the playhead");
    assert!(t > 0.0);
    assert_eq!(app.clock_of(pl).as_deref(), Some(clock.as_str()));
    assert!(app.is_active_clock(&clock));
    d.frames(3);
    let end = crate::planes_node::view_end(&d.h.ctx, pl).expect("Planes follows the playhead");
    assert!((end - (t + 80.0)).abs() < 1e-6, "{end} vs {t}");
    d.shot("playhead_linked");
}

/// Playing moves the selected node's playhead; in a live project a live group's playhead stops
/// at the newest audio, and Live jumps back to it.
#[test]
fn play_and_live_playheads() {
    let mut app = App::empty();
    let a = source(&mut app, 40.0);
    anchor(&app, a, 1000.0, 1.0);
    app.update_clocks();
    app.pick_only(a);
    let c = uid(&app, a);
    assert!(!app.is_live());
    app.set_playhead(&c, 2.0);
    app.ph.playing = true;
    app.play_step(1.5);
    assert_eq!(app.playhead_on(&c), Some(3.5));
    assert_eq!(app.ph.utc, Some(1003.5));
    {
        let mut s = app.nodes[a].state.lock().unwrap();
        s.live = true;
        s.audio_s = 3.0;
    }
    assert!(app.is_live());
    app.play_step(1.0);
    assert_eq!(app.playhead_on(&c), Some(3.0), "stops at now");
    assert!(!app.ph.playing);
    app.set_playhead(&c, 0.5);
    app.go_live();
    assert_eq!(app.playhead_on(&c), Some(3.0));
}

/// While playing, the sound is the selected node's source audio at its playhead; Planes, fed by
/// the source, plays the source's audio; paused, nothing.
#[test]
fn sound_follows_the_selected_node() {
    let mut app = App::empty();
    let s = source(&mut app, 40.0);
    let pl = app.add(Kind::Planes, Pos2::new(600.0, 40.0));
    app.wire(s, "audio", pl, "audio");
    app.nodes[s].state.lock().unwrap().source.push_audio(&[0.5; 1600]);
    app.update_clocks();
    app.pick_only(pl);
    let c = app.clock_of(s).unwrap();
    app.set_playhead(&c, 0.05);
    assert!(app.sound_target().is_none(), "paused: no sound");
    app.ph.playing = true;
    let (a, t) = app.sound_target().unwrap();
    assert_eq!((a.lock().unwrap().len(), t), (1600, 0.05));
}
