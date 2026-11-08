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

    // A node group holding a and b glows in both their groups' colours.
    let g = app.add(Kind::Group { def: "Group".into(), rows: Vec::new() }, Pos2::ZERO);
    app.nodes[a].parent = Some(g);
    app.nodes[b].parent = Some(g);
    let cg = app.canvas_groups();
    let mut both = [cg.of_node[a].clone(), cg.of_node[b].clone()].concat();
    both.sort_unstable();
    assert_eq!(cg.of_node[g], both);
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
    assert_eq!((a.t0, a.own_err), (1030.0, 0.5));
    // The clock's error grows away from the anchor (unknown drift): a hair over 2 s at 30-31 s.
    assert!((a.err0 - 2.0).abs() < 1e-4 && a.err1 > a.err0, "{} {}", a.err0, a.err1);
    assert!((a.err() - a.err1.hypot(0.5)).abs() < 1e-9);
    // b and c share a group; both placed on the reference clock (the smaller id, b), the one
    // that starts first first.
    assert_eq!(rows[1].subs.len(), 2);
    let on_ref: Vec<(&str, f64)> = rows[1].subs.iter().map(|s| (s.clock.as_str(), s.events[0].t0)).collect();
    assert!(on_ref[0].0 == "c" && (on_ref[0].1 - -7.0).abs() < 1e-6 && (on_ref[1].1 - 4.0).abs() < 1e-6, "{on_ref:?}");
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
    let off = |c: &str, s: f64| clocks.get(c).back(s);
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
    assert!(app.ph.playing, "playing on: following live");
    app.ph.playing = false;
    app.set_playhead(&c, 0.5);
    app.go_live();
    assert_eq!(app.playhead_on(&c), Some(3.0));
}

/// A recording plays to its end and pauses; Play on the SOURCER at the end starts it again from
/// the beginning, and a source with audio always has a playhead.
#[test]
fn recording_plays_to_its_end() {
    let mut app = App::empty();
    let a = source(&mut app, 40.0);
    app.nodes[a].state.lock().unwrap().audio_s = 10.0;
    app.update_clocks();
    let c = app.clock_of(a).unwrap();
    app.ensure_playheads();
    assert_eq!(app.playhead_on(&c), Some(0.0), "a playhead at the start");
    app.play_source(a);
    assert!(app.ph.playing && app.selected == Some(a));
    app.play_step(4.0);
    assert_eq!(app.playhead_on(&c), Some(4.0));
    app.play_source(a);
    assert!(!app.ph.playing, "paused");
    app.play_source(a);
    app.play_step(20.0);
    assert_eq!(app.playhead_on(&c), Some(10.0), "stops at the end");
    assert!(!app.ph.playing);
    app.play_source(a);
    assert_eq!(app.playhead_on(&c), Some(0.0), "from the start again");
    assert!(app.ph.playing);
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

/// Times on clock groups with nothing linking them are listed as not comparable and warned
/// about; the result file carries the clocks, the evidence and the events with their UTC.
#[test]
fn not_comparable_and_the_result_file() {
    use crate::evidence::{comparable_note, not_comparable};
    use corroborate::clock::{solve, Anchor as An, Link};
    let clocks = solve(
        &[An { clock: "a".into(), t: 0.0, utc: 1000.0, error_s: 1.0, from: "n1".into(), enabled: true }],
        &[Link { a: "b".into(), t_a: 0.0, b: "c".into(), t_b: 1.0, error_s: 1.0, from: "manual".into(), enabled: true }],
    );
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert!(not_comparable(&clocks, &s(&["b", "c", ""])).is_empty(), "linked: comparable");
    assert!(not_comparable(&clocks, &s(&["a", "utc"])).is_empty(), "both absolute");
    let nc = not_comparable(&clocks, &s(&["a", "b", "z"]));
    assert_eq!(nc.len(), 3);
    assert!(comparable_note(&nc).unwrap().contains("3 clock groups"));
    assert!(comparable_note(&[]).is_none());

    let mut app = App::empty();
    let a = source(&mut app, 40.0);
    anchor(&app, a, 1000.0, 2.0);
    app.update_clocks();
    let j = app.result_json(&[a]);
    assert!(j.contains("\"clocks\":[{") && j.contains("\"kind\":\"absolute\"") && j.contains("\"type\":\"anchor\"") && j.contains("\"not_comparable\":[]"), "{j}");
}

/// The canvas outlines each group and numbers it; the timeline gives each group its own lane and
/// ruler: UTC for a dated source, time on the reference clock for linked ones, none unanchored.
/// Media are square boxes with see-through margins either side.
#[test]
fn timeline_lanes_per_group() {
    use corroborate::clock::Link;
    let mut d = crate::drive::Drive::new();
    let app = d.app();
    let a = app.add(Kind::Sourcer { input: String::new() }, Pos2::new(30.0, 40.0));
    let enf = app.add(Kind::Enf { grid: String::new() }, Pos2::new(330.0, 40.0));
    app.wire(a, "audio", enf, "audio");
    let b = app.add(Kind::Sourcer { input: String::new() }, Pos2::new(30.0, 330.0));
    let c = app.add(Kind::Sourcer { input: String::new() }, Pos2::new(700.0, 330.0));
    let e = app.add(Kind::Sourcer { input: String::new() }, Pos2::new(1050.0, 60.0));
    let utc0 = 1_467_916_147.0;
    anchor(app, a, utc0, 20.0);
    for (i, s) in [(a, 300.0), (b, 200.0), (c, 150.0), (e, 120.0)] {
        app.nodes[i].state.lock().unwrap().audio_s = s;
    }
    d.frames(3);
    let app = d.app();
    let (cb, cc) = (app.clock_of(b).unwrap(), app.clock_of(c).unwrap());
    app.ev.manual_links.push(Link { a: cb, t_a: 30.0, b: cc, t_b: 0.0, error_s: 4.0, from: "manual".into(), enabled: true });
    app.update_clocks();
    app.follow = false;
    app.tl = (utc0 - 40.0, utc0 + 360.0);
    d.frames(4);
    let app = d.app();
    assert_eq!(app.tls.row_y.len(), 3, "one lane per group: dated, linked, unanchored");
    let ys: Vec<f32> = app.tls.row_y.iter().map(|r| r.1).collect();
    assert!(ys[1] - ys[0] > 40.0, "the dated lane has its own ruler above the lane: {ys:?}");
    d.shot("timeline_lanes");
}

/// Group numbers stay with their group: others leaving or a merge do not renumber the rest.
#[test]
fn group_numbers_stay_put() {
    use crate::clock_view::number_groups;
    let mut known = Default::default();
    assert_eq!(number_groups(&mut known, &["a", "b", "c"]), vec![1, 2, 3]);
    // b merged into a (the merge keeps a's id): c stays 3.
    assert_eq!(number_groups(&mut known, &["a", "c"]), vec![1, 3]);
    // A new group takes the smallest free number.
    assert_eq!(number_groups(&mut known, &["a", "c", "d"]), vec![1, 3, 2]);
}

/// Linking two groups keeps what the user watches: the active group's playhead moves into the
/// merged group at the same moment on its clock; the other group's playhead and shift go.
#[test]
fn playheads_carry_over_a_merge() {
    use corroborate::clock::Link;
    let mut d = crate::drive::Drive::new();
    let app = d.app();
    let b = source(app, 40.0);
    let c = source(app, 330.0);
    for i in [b, c] {
        app.nodes[i].state.lock().unwrap().audio_s = 100.0;
    }
    d.frames(3);
    let app = d.app();
    let (cb, cc) = (app.clock_of(b).unwrap(), app.clock_of(c).unwrap());
    app.set_playhead(&cb, 12.0);
    app.set_playhead(&cc, 3.0);
    app.tls.shift.insert(cc.clone(), 50.0);
    app.pick_only(c);
    // b 30 s is c 0 s.
    app.ev.manual_links.push(Link { a: cb.clone(), t_a: 30.0, b: cc.clone(), t_b: 0.0, error_s: 1.0, from: "manual".into(), enabled: true });
    app.update_clocks();
    let g = app.ev.clocks.get(&cc).group.clone();
    assert_eq!(app.ev.clocks.get(&cb).group, g, "one group now");
    let at = app.playhead_on(&cc).unwrap();
    assert!((at - 3.0).abs() < 1e-6, "the watched source is still at 3 s: {at}");
    assert_eq!(app.ph.floating.len(), 1, "one playhead for the one group");
    assert!(app.tls.shift.keys().all(|k| *k == g), "shifts of groups gone are dropped");
}

/// Two clocks linked at both ends of an hour, one running 300 ppm fast: the timeline places the
/// drifting clock's events drift-corrected, and a playhead set on it reads back the same moment.
#[test]
fn drifting_clocks_on_the_timeline_and_playheads() {
    use crate::timeline::build_rows;
    use corroborate::clock::Link;
    use corroborate::data::Event;
    let k = 1.0 + 300e-6;
    let link = |ta: f64| Link { a: "a".into(), t_a: ta, b: "b".into(), t_b: 20.0 + ta * k, error_s: 0.01, from: "m".into(), enabled: true };
    let clocks = corroborate::clock::solve(&[], &[link(0.0), link(3600.0)]);
    let ev = |c: &str, t: f64| (0usize, Event { clock: c.into(), t_start: t, t_end: t, ..Default::default() });
    let rows = build_rows(&clocks, &[ev("a", 3600.0), ev("b", 20.0 + 3600.0 * k)], &|c| c.to_string());
    let t: Vec<f64> = rows[0].subs.iter().map(|s| s.events[0].t0).collect();
    assert!((t[0] - t[1]).abs() < 0.05, "the same moment lines up despite 1.08 s of drift: {t:?}");
    let b = clocks.get("b");
    assert!((b.back(b.at(1234.5)) - 1234.5).abs() < 1e-9);

    let mut app = App::empty();
    app.ev.clocks = clocks;
    app.set_playhead("b", 2000.0);
    let back = app.playhead_on("b").unwrap();
    assert!((back - 2000.0).abs() < 1e-6, "{back}");
    let on_a = app.playhead_on("a").unwrap();
    assert!((on_a - (2000.0 - 20.0) / k).abs() < 0.05, "{on_a}");
}

/// One anchor at the start of an hour-long recording: the right margin (the clock's error at the
/// end) is wider than the left, by about 0.36 s at 100 ppm.
#[test]
fn margins_grow_away_from_the_evidence() {
    use crate::timeline::build_rows;
    use corroborate::clock::Anchor as An;
    use corroborate::data::Event;
    let clocks = corroborate::clock::solve(&[An { clock: "a".into(), t: 0.0, utc: 1000.0, error_s: 0.01, from: "n".into(), enabled: true }], &[]);
    let rows = build_rows(&clocks, &[(0usize, Event { clock: "a".into(), t_start: 0.0, t_end: 3600.0, ..Default::default() })], &|c| c.to_string());
    let e = &rows[0].subs[0].events[0];
    assert!((e.err0 - 0.01).abs() < 1e-6 && (e.err1 - 0.36).abs() < 0.01, "{} {}", e.err0, e.err1);
}

/// Headless: an hour-long span anchored at its start draws a thin margin at the start and a
/// wide one at the end (`target/drive/timeline_drift_start.png`, `timeline_drift_end.png`).
#[test]
fn drift_margins_drawn() {
    use corroborate::clock::Anchor as An;
    use corroborate::data::Event;
    let mut d = crate::drive::Drive::new();
    let app = d.app();
    app.tls.extra = vec![(0usize, Event { clock: "c".into(), t_start: 0.0, t_end: 3600.0, label: "an hour".into(), ..Default::default() })];
    app.ev.manual_anchors.push(An { clock: "c".into(), t: 0.0, utc: 1_700_000_000.0, error_s: 0.05, from: "manual".into(), enabled: true });
    app.update_clocks();
    app.follow = false;
    // The start, then the end, at the same zoom (6 s across).
    app.tl = (1_700_000_000.0 - 3.0, 1_700_000_000.0 + 3.0);
    d.frames(3);
    d.shot("timeline_drift_start");
    let e = d.app().ev.clocks.get("c");
    assert!(e.error_at(3600.0) > 5.0 * e.error_at(0.0));
    d.app().tl = (1_700_003_600.0 - 3.0, 1_700_003_600.0 + 3.0);
    d.frames(3);
    d.shot("timeline_drift_end");
}

/// Audio match: two files that heard one event (b starts 20.5 s into it, a at its start; b's
/// microphone hears an echo and its own noise) wired into an Audio match node give links naming
/// both clocks, and the clock solve puts b's 10 s at a's 30.5 s, in one group. A third file, a
/// quieter copy of a, is called the same recording as a (and b is not); a and its copy's links
/// span 45 s, enough for their drift (none).
#[test]
fn audio_match_links_two_recordings() {
    let ev = corroborate::synth::notes(90.0, 8000, 11);
    let write = |name: &str, x: &[f32]| {
        let p = crate::drive::out_dir().join(name);
        let spec = hound::WavSpec { channels: 1, sample_rate: 8000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&p, spec).unwrap();
        for &v in x {
            w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
        }
        w.finalize().unwrap();
        p.to_string_lossy().into_owned()
    };
    let s0 = (20.5 * 8000.0) as usize;
    let mut n = 3u32;
    let other: Vec<f32> = (s0..ev.len())
        .map(|i| {
            n = n.wrapping_mul(1_103_515_245).wrapping_add(12345);
            ev[i] + 0.8 * ev[i - 47] + 0.2 * ((n >> 16) as f32 / 32768.0 - 1.0)
        })
        .map(|v| 0.5 * v)
        .collect();
    let copy: Vec<f32> = ev[..60 * 8000].iter().map(|&v| 0.7 * v).collect();
    let (pa, pb, pc) = (write("match_a.wav", &ev[..60 * 8000]), write("match_b.wav", &other), write("match_c.wav", &copy));
    // Headless, drawn to target/drive/audio_match.png (no window).
    let mut d = crate::drive::Drive::new();
    let app = d.app();
    let a = app.add(Kind::Sourcer { input: pa }, Pos2::new(30.0, 40.0));
    let b = app.add(Kind::Sourcer { input: pb }, Pos2::new(30.0, 300.0));
    let c = app.add(Kind::Sourcer { input: pc }, Pos2::new(30.0, 560.0));
    let m = app.add(Kind::AudioMatch, Pos2::new(460.0, 120.0));
    app.wire_many(a, "audio", m);
    app.wire_many(b, "audio", m);
    assert_eq!(app.graph.nodes()[m].spec.inputs.len(), 3, "two wired and one free");
    app.wire_many(c, "audio", m);
    app.run();
    assert!(d.wait_until(30.0, |a| a.nodes[m].state.lock().unwrap().done), "the match ends");
    d.frames(5);
    d.shot("audio_match");
    let app = d.app();
    let st = app.nodes[m].state.lock().unwrap().status.clone();
    let Ok(Data::Custom { json, .. }) = app.value_from(m, "quality") else { panic!("no quality: {st}") };
    assert!(json.contains("\"level\":\"good\""), "{json}");
    app.update_clocks();
    let (ua, ub, uc) = (uid(app, a), uid(app, b), uid(app, c));
    let ab: Vec<_> = app.ev.links.iter().filter(|l| (l.a == ua && l.b == ub) || (l.a == ub && l.b == ua)).collect();
    assert!(ab.len() >= 2, "links between a and b: {} ({st})", ab.len());
    let Ok(Data::Events(same)) = app.value_from(m, "same recording") else { panic!("no same recording output: {st}") };
    let mut on: Vec<&str> = same.iter().map(|e| e.clock.as_str()).collect();
    on.sort();
    let mut want = vec![ua.as_str(), uc.as_str()];
    want.sort();
    assert_eq!(on, want, "a and its copy are one recording, b is not: {same:?}");
    assert!(same.iter().all(|e| e.t_end - e.t_start > 40.0), "{same:?}");
    let Ok(Data::Json(drift)) = app.value_from(m, "clock drift") else { panic!("no drift: {st}") };
    assert_eq!(drift.matches("\"ppm\"").count(), 1, "only a and its copy span 30 s of links: {drift}");
    assert!(matches!(app.value_from(m, "speed change"), Ok(Data::Events(v)) if v.is_empty()));
    assert!(matches!(app.value_from(m, "gaps"), Ok(Data::Events(v)) if v.is_empty()), "no gaps");
    let (ca, cb) = (app.ev.clocks.get(&ua), app.ev.clocks.get(&ub));
    assert_eq!((ca.kind, ca.group.clone()), (Ck::Relative, cb.group.clone()), "one group");
    let on_a = ca.back(cb.at(10.0));
    assert!((on_a - 30.5).abs() < 0.002, "b's 10 s is a's {on_a}");
    app.stop_all();
}

/// Makes the Jiku project in the app folder: app\projects\Jiku GT_090912\Jiku GT_090912.txt, one
/// SOURCER per video of event GT_090912 (the .mp4 files in tests\data\jiku, referenced where
/// they are, not copied: 13 GB), all wired into one Audio match node, and puts it first in Open
/// recent. Then runs it headless (no window) and checks the recordings link into groups, drawn
/// to target\drive\jiku_project.png. Fetch the videos first: python tools/jiku.py GT_090912.
/// Run: cargo test --profile fast --bin corroborate make_jiku_project -- --ignored --nocapture
#[test]
#[ignore]
fn make_jiku_project() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let data = root.join("tests").join("data").join("jiku").join("GT_090912");
    let mut videos: Vec<PathBuf> = std::fs::read_dir(&data).expect("no Jiku data: python tools/jiku.py GT_090912").filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "mp4")).collect();
    videos.sort();
    assert_eq!(videos.len(), 50, "the 50 videos of GT_090912");
    let mut d = crate::drive::Drive::new();
    let app = d.app();
    let cols = 5;
    let mut sources = Vec::new();
    for (k, v) in videos.iter().enumerate() {
        let (c, r) = (k % cols, k / cols);
        let s = app.add(Kind::Sourcer { input: v.to_string_lossy().into_owned() }, Pos2::new(30.0 + 470.0 * c as f32, 40.0 + 460.0 * r as f32));
        // GT_090912_<phone>_<phone clock at the start, ms>.
        let stem = v.file_stem().unwrap().to_string_lossy().into_owned();
        let parts: Vec<&str> = stem.split('_').collect();
        app.nodes[s].label = format!("phone {} {}", parts[2], parts[3]);
        // Collapsed: a big project starts small, and reads no pictures.
        app.nodes[s].collapsed = true;
        sources.push(s);
    }
    let m = app.add(Kind::AudioMatch, Pos2::new(30.0 + 470.0 * cols as f32 + 80.0, 40.0));
    for &s in &sources {
        app.wire_many(s, "audio", m);
    }
    assert_eq!(app.graph.wires().len(), 50, "every video wired in");
    let file = project::project_place(&root.join("app").join("projects").join("Jiku GT_090912"));
    // A fresh project: no unsaved changes of an earlier one restored over it.
    let _ = std::fs::remove_file(project::recovery_of(&file));
    app.save_project_as(file.clone());
    assert!(file.is_file(), "{}", file.display());
    project::push_recent(&root.join("app").join("settings"), &file);
    println!("saved {}", file.display());
    // Check it syncs: run it as the app would.
    app.run();
    let t0 = std::time::Instant::now();
    assert!(d.wait_until(600.0, |a| a.nodes[m].state.lock().unwrap().done), "the match ends");
    let app = d.app();
    let st = app.nodes[m].state.lock().unwrap().status.clone();
    println!("{st} ({:.0} s)", t0.elapsed().as_secs_f64());
    if let Ok(Data::Custom { json, .. }) = app.value_from(m, "quality") {
        let e = json.find("\"errors\"").unwrap_or(0);
        println!("{}", &json[e..json.len().min(e + 3000)]);
    }
    app.update_clocks();
    let groups: std::collections::BTreeSet<String> = sources.iter().map(|&s| app.ev.clocks.get(&uid(app, s)).group.clone()).collect();
    println!("{} recordings in {} groups", sources.len(), groups.len());
    d.frames(5);
    d.shot("jiku_project");
    let app = d.app();
    app.stop_all();
    assert!(st.contains("of 1225 pairs"), "all 50 matched: {st}");
}

/// A file's creation time (a camera clock perhaps set by hand) anchors its source's clock only
/// while the source's time output is wired; other anchors count wired or not.
#[test]
fn creation_time_anchors_only_when_wired() {
    let mut app = App::empty();
    let s = source(&mut app, 40.0);
    let p = app.add(Kind::Planes, Pos2::new(300.0, 40.0));
    app.wire(s, "audio", p, "audio");
    let stamp = Anchor { utc0: 1000.0, error_s: 60.0, source: "file creation_time (low confidence)".into() };
    assert!(stamp.is_creation_time());
    app.nodes[s].state.lock().unwrap().anchor = Some(stamp);
    app.update_clocks();
    assert!(app.ev.anchors.is_empty(), "unwired: not used {:?}", app.ev.anchors);
    app.wire(s, "time", p, "time");
    assert_eq!(app.graph.wires().len(), 2, "{}", app.msg);
    app.update_clocks();
    assert_eq!(app.ev.anchors.len(), 1, "wired: used");
    assert_eq!(app.ev.anchors[0].utc, 1000.0);
    // Our recorder's sidecar time counts unwired.
    anchor(&app, s, 2000.0, 1.0);
    app.graph.disconnect(p, "time");
    app.update_clocks();
    assert_eq!(app.ev.anchors.len(), 1, "a sidecar counts unwired");
}

/// Opens a copy of the Jiku project (made by make_jiku_project; the copy in target\drive, so
/// the project itself is never written) as the app would: it opens stopped, fits
/// on screen zoomed out, and after Run the frames stay quick while 50 videos decode: only
/// still frames, read for the sources on screen. PNGs jiku_opened, jiku_running, jiku_close.
/// Run: cargo test --profile fast --bin corroborate jiku_project_opens_stopped -- --ignored --nocapture
#[test]
#[ignore]
fn jiku_project_opens_stopped() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let made = root.join("app").join("projects").join("Jiku GT_090912").join("Jiku GT_090912.txt");
    assert!(made.is_file(), "make it first: make_jiku_project");
    let dir = crate::drive::out_dir().join("Jiku copy");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Jiku copy.txt");
    std::fs::copy(&made, &file).unwrap();
    let mut d = crate::drive::Drive::new();
    d.app().open_project(file);
    d.frames(5);
    assert_eq!(d.app().nodes.iter().filter(|n| !n.hidden).count(), 51);
    assert!(d.app().stopped && !d.app().running(), "opens stopped");
    d.wait(1.0);
    assert!(!d.app().running(), "and stays stopped");
    let area = Rect::from_min_size(d.app().canvas_origin, Vec2::new(1400.0, 800.0));
    d.app().frame_nodes(area);
    d.frames(3);
    println!("zoom to see it all: {:.2}", d.app().zoom);
    d.shot("jiku_opened");
    d.click("▶ Run");
    assert!(!d.app().stopped && d.app().running(), "Run starts it");
    let (mut worst, mut sum, mut n) = (0.0f64, 0.0f64, 0);
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        let t = Instant::now();
        d.h.step();
        let dt = t.elapsed().as_secs_f64();
        if dt > 0.3 {
            println!("slow frame {n} at {:.1} s: {:.0} ms", 30.0 - (end - Instant::now()).as_secs_f64(), dt * 1000.0);
        }
        (worst, sum, n) = (worst.max(dt), sum + dt, n + 1);
        std::thread::sleep(Duration::from_millis(30));
    }
    let asked = d.app().nodes.iter().filter(|n| n.state.lock().unwrap().source.frames_asked != sourcer_node::Asked::Nothing).count();
    println!("frames while running: {n}, mean {:.1} ms, worst {:.0} ms; still frames asked by {asked} sources", sum / n as f64 * 1000.0, worst * 1000.0);
    d.shot("jiku_running");
    // Close up: a still frame.
    d.app().zoom = 1.0;
    // The bottom row: in this layout each row's video lies under the next row's nodes.
    d.app().pan = Vec2::ZERO - d.app().nodes[46].pos.to_vec2() + Vec2::new(40.0, 0.0);
    d.wait(5.0);
    d.shot("jiku_close");
    d.app().stop_run();
    let t = Instant::now();
    d.wait_until(10.0, |a| !a.running());
    let running: Vec<String> = d.app().nodes.iter().filter(|n| n.state.lock().unwrap().running).map(|n| n.label.clone()).collect();
    println!("stopped in {:.1} s", t.elapsed().as_secs_f64());
    assert!(running.is_empty() && d.app().stopped, "Stop stops it: still running {running:?}");
    assert!(worst < 2.0, "no frame takes seconds: worst {worst:.1} s");
}

/// The timeline on a big project, handled like a user would (a copy of the Jiku project in
/// target\drive): the panel dragged taller, each group of several recordings one summary bar,
/// zoomed out with the wheel, panned by dragging empty space, a group opened by a click on its
/// bar, the rows scrolled, a floating row dragged along, a bar hovered, the group closed by a
/// click on its label. PNGs tl_1_closed … tl_7_closed_again.
/// Run: cargo test --profile fast --bin corroborate timeline_on_a_big_project -- --ignored --nocapture
#[test]
#[ignore]
fn timeline_on_a_big_project() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let made = root.join("app").join("projects").join("Jiku GT_090912").join("Jiku GT_090912.txt");
    assert!(made.is_file(), "make it first: make_jiku_project");
    let dir = crate::drive::out_dir().join("Jiku timeline");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Jiku timeline.txt");
    std::fs::copy(&made, &file).unwrap();
    let mut d = crate::drive::Drive::new();
    d.app().open_project(file);
    d.app().places.open = false;
    d.frames(3);
    assert!(d.app().nodes.iter().filter(|n| n.kind.is_source()).all(|n| n.collapsed), "the sources start collapsed");
    let m = d.app().nodes.iter().position(|n| matches!(n.kind, Kind::AudioMatch)).unwrap();
    d.click("▶ Run");
    assert!(d.wait_until(180.0, |a| a.nodes[m].state.lock().unwrap().done), "the match ends");
    d.wait(1.0);
    let n_pics = d.app().nodes.iter().filter(|n| n.state.lock().unwrap().source.frames_asked != sourcer_node::Asked::Nothing).count();
    assert_eq!(n_pics, 0, "collapsed sources read no pictures");

    // The panel: drag its top edge up.
    let (track, ..) = d.app().tls.geom.expect("timeline drawn");
    let edge = track.top() - 30.0;
    d.drag(Pos2::new(700.0, edge), Pos2::new(700.0, 260.0));
    d.frames(3);
    let (track, ..) = d.app().tls.geom.unwrap();
    println!("timeline track {track:?}");
    let bars = d.app().tls.bars.clone();
    println!("{} summary bars, {} rows", bars.len(), d.app().tls.row_y.len());
    assert!(!bars.is_empty(), "groups of several recordings are summary bars");
    d.shot("tl_1_closed");

    // Zoom out: wheel over the track.
    let span0 = d.app().tl.1 - d.app().tl.0;
    d.scroll(track.center(), -8.0);
    let span1 = d.app().tl.1 - d.app().tl.0;
    println!("span {span0:.0} s -> {span1:.0} s");
    assert!(span1 > span0 * 1.5, "the wheel zooms out");
    d.shot("tl_2_zoomed_out");

    // Pan: drag empty space under the rows.
    let (track, ..) = d.app().tls.geom.unwrap();
    let below = d.app().tls.row_y.iter().map(|r| r.1).fold(track.top(), f32::max) + 40.0;
    if below < track.bottom() - 10.0 {
        let v0 = d.app().tl.0;
        d.drag(Pos2::new(track.center().x, below), Pos2::new(track.center().x - 200.0, below));
        println!("pan: view start {v0:.0} -> {:.0}", d.app().tl.0);
        assert!(d.app().tl.0 > v0, "dragging empty space to the left moves the view later");
        assert!(!d.app().follow);
    } else {
        println!("no empty space under the rows to pan on");
    }

    // Open the first group: a click on its bar.
    let (g, bar) = d.app().tls.bars[0].clone();
    d.click_at(bar.center().x, bar.center().y);
    assert!(d.app().tls.expanded.contains(&g), "a click on the bar opens the group");
    d.frames(3);
    d.shot("tl_3_open");

    // Scroll the rows: wheel over the labels.
    let (track, ..) = d.app().tls.geom.unwrap();
    // The track is the rows' whole scrolled content: aim inside the panel (the screen is 900 high).
    let seen = |t: Rect| Rect::from_min_max(t.min, Pos2::new(t.right(), t.bottom().min(880.0)));
    let track = seen(track);
    let y_before = d.app().tls.row_y.last().map(|r| r.1);
    d.scroll(Pos2::new(track.left() - 100.0, track.center().y), -6.0);
    let y_after = d.app().tls.row_y.last().map(|r| r.1);
    println!("last row y {y_before:?} -> {y_after:?}");
    assert!(y_after < y_before, "the wheel over the labels scrolls the rows");
    d.shot("tl_4_scrolled");
    d.scroll(Pos2::new(track.left() - 100.0, track.center().y), 20.0);
    let (track, ..) = d.app().tls.geom.unwrap();
    let track = seen(track);

    // Drag a floating group row along by 150 px (the open one, the only one in sight).
    let (g2, y2) = d.app().tls.row_y.iter().find(|r| r.1 > track.top() + 20.0 && r.1 < track.bottom() - 10.0).cloned().expect("a group row in sight");
    let before = d.app().tls.shift.get(&g2).copied();
    let x = track.left() + 60.0;
    d.drag(Pos2::new(x, y2), Pos2::new(x + 150.0, y2));
    let (after, pending) = (d.app().tls.shift.get(&g2).copied(), d.app().tls.pending.is_some());
    println!("row {g2}: shift {before:?} -> {after:?}, pending link: {pending}");
    assert!(after != before, "the row moved");
    d.shot("tl_5_dragged");
    d.app().tls.pending = None;
    d.frames(2);

    // Hover a bar: what it holds.
    if let Some((_, bar)) = d.app().tls.bars.iter().find(|b| b.1.center().y < 880.0).cloned() {
        d.h.hover_at(bar.center() + Vec2::new(bar.width() * 0.2, 0.0));
        d.frames(10);
        d.shot("tl_6_hover");
    }

    // Close it again: a click on its label.
    let y = d.app().tls.row_y.iter().find(|r| r.0 == g).unwrap().1;
    d.click_at(track.left() - 120.0, y - 5.0);
    assert!(!d.app().tls.expanded.contains(&g), "a click on the label closes it");
    d.frames(3);
    d.shot("tl_7_closed_again");
    d.app().stop_run();
}

/// The timeline's rows scroll with the wheel over their labels when there are more than fit.
#[test]
fn timeline_rows_scroll() {
    let mut app = App::empty();
    app.places.open = false;
    app.tls.extra = (0..40).map(|k| (0usize, corroborate::data::Event { clock: format!("c{k:02}"), t_start: 0.0, t_end: 60.0, label: "x".into(), ..Default::default() })).collect();
    let mut d = crate::drive::Drive::with(app);
    d.frames(3);
    let (track, ..) = d.app().tls.geom.expect("drawn");
    let first = d.app().tls.row_y[0].1;
    d.scroll(Pos2::new(track.left() - 100.0, track.top() + 60.0), -5.0);
    let after = d.app().tls.row_y[0].1;
    d.shot("timeline_rows_scroll");
    assert!(after < first - 20.0, "rows scrolled up: {first} -> {after}");
}
