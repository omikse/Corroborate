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

    app.ev.off.insert(an.key());
    app.update_clocks();
    assert_eq!(app.ev.clocks.get(&us).kind, Ck::None, "switched off");
    assert!(!app.ev.anchors[0].enabled);
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

/// Times on clock groups with nothing linking them are listed as not comparable; the result file
/// carries the clocks, the evidence and the events with their UTC.
#[test]
fn not_comparable_and_the_result_file() {
    use crate::evidence::not_comparable;
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

    let mut app = App::empty();
    let a = source(&mut app, 40.0);
    anchor(&app, a, 1000.0, 2.0);
    app.update_clocks();
    let j = app.result_json(&[a]);
    assert!(j.contains("\"clocks\":[{") && j.contains("\"kind\":\"absolute\"") && j.contains("\"type\":\"anchor\"") && j.contains("\"not_comparable\":[]"), "{j}");
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
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(460.0, 120.0));
    app.wire_many(a, "audio", m);
    app.wire_many(b, "audio", m);
    assert_eq!(app.graph.nodes()[m].spec.inputs.len(), 6, "rows of audio and time: two wired and one free");
    assert_eq!(app.graph.nodes()[m].spec.outputs.iter().filter(|p| p.name.starts_with("synced")).count(), 3, "a yellow output per row");
    app.wire_many(c, "audio", m);
    app.run();
    assert!(d.wait_until(30.0, |a| a.nodes[m].state.lock().unwrap().done), "the match ends");
    d.frames(5);
    d.shot("audio_match");
    // On the node: each row placed by audio once the match has ended. These files have no stamp.
    d.app().update_clocks();
    d.frames(2);
    let view = d.app().match_view(m);
    let wired: Vec<_> = view.rows.iter().filter(|r| r.wired).collect();
    assert_eq!(wired.len(), 3);
    assert!(wired.iter().all(|r| r.start.is_some() && r.meta.is_none()), "{wired:?}");
    assert_eq!(view.groups.iter().filter(|g| g.number > 0).count(), 1, "one synced group");
    assert_eq!(crate::audio_match_node::cell(&view, 0).0, crate::audio_match_node::Cell::Plain("no time metadata".into()));
    assert!(d.app().value_from(m, "synced 2").is_err(), "no anchor: nothing comes out");
    assert!(d.app().output_held(m, "synced 2").is_some(), "greyed");
    // A time on row 1, then the group anchored on it: every recording of the group has real time.
    let v = d.app().add(Kind::Value { ty: "time".into(), rows: vec![("utc".into(), "1970-01-01T00:16:40Z".into()), ("error_s".into(), "2".into())] }, Pos2::new(30.0, 800.0));
    d.app().wire(v, "value", m, "time 1");
    d.frames(2);
    let ub = uid(d.app(), b);
    assert_ne!(d.app().ev.clocks.get(&ub).kind, Ck::Absolute, "a time on a row is a claim only");
    d.app().toggle_anchor(m, 0);
    d.app().update_clocks();
    d.frames(3);
    assert_eq!(d.app().ev.clocks.get(&ub).kind, Ck::Absolute, "anchored: the linked recording has real time");
    let Ok(Data::Time { utc: Some(ub_utc), error_s, .. }) = d.app().value_from(m, "synced 2") else { panic!("no time out of row 2") };
    assert!((ub_utc - 1020.5).abs() < 0.01 && error_s >= 2.0, "b starts 20.5 s after a: {ub_utc} ± {error_s}");
    let Ok(Data::Time { utc: Some(ua_utc), .. }) = d.app().value_from(m, "synced 1") else { panic!("no time out of row 1") };
    assert!((ua_utc - 1000.0).abs() < 0.01, "{ua_utc}");
    let view = d.app().match_view(m);
    assert_eq!(crate::audio_match_node::cell(&view, 0).0, crate::audio_match_node::Cell::Anchor);
    assert!(d.app().project_text().contains(&format!("anchor {}", uid(d.app(), a))), "kept with the project");
    d.shot("audio_match_anchored");
    d.app().toggle_anchor(m, 0);
    d.app().graph.disconnect(m, "time 1");
    d.app().sync_many_ports();
    d.app().update_clocks();
    d.frames(2);
    assert_eq!(d.app().audio_match_inputs(m, &Default::default()).0.len(), 3, "the time input is not a recording");
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

/// A row's audio and time, and the wire out of its yellow output, survive save and load by name;
/// a project saved before rows had a time each gets each recording's time wired to its row, a Time
/// node where the stamp was read as the end, and the anchor on the recording whose time was wired.
#[test]
fn audio_match_rows_are_saved_and_old_projects_converted() {
    let mut app = App::empty();
    let (a, b) = (source(&mut app, 40.0), source(&mut app, 300.0));
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(460.0, 120.0));
    let tl = app.add(Kind::Planes, Pos2::new(1600.0, 120.0));
    app.wire_many(a, "audio", m);
    app.wire_many(b, "audio", m);
    app.wire(b, "time", m, "time 2");
    app.wire(m, "synced 2", tl, "time");
    app.sync_many_ports();
    let text = app.project_text();
    assert!(text.contains("format|2"));
    let mut back = App::empty();
    assert!(back.load_project_text(&text));
    assert_eq!(back.graph.feeder(m, "audio 1").map(|f| f.0), Some(a));
    assert_eq!(back.graph.feeder(m, "audio 2").map(|f| f.0), Some(b));
    assert_eq!(back.graph.feeder(m, "time 2"), Some((b, "time")));
    assert_eq!(back.graph.feeder(m, "time 1"), None, "not wired, not added");
    assert_eq!(back.graph.feeder(tl, "time"), Some((m, "synced 2")));

    let ids = [uid(&app, a), uid(&app, b)];
    let old = format!(
        "node|sourcer|30|40|one|a.mp4|id={}\nnode|sourcer|30|300|two|b.mp4|id={}\nnode|audio-match|460|120|Audio match||id=n9\nnext|10\nwire|0|audio|2|audio 1\nwire|1|audio|2|audio 2\nwire|1|time|2|time\nstamp-end|{}\n",
        ids[0], ids[1], ids[0]
    );
    let mut conv = App::empty();
    assert!(conv.load_project_text(&old));
    assert_eq!(conv.graph.feeder(2, "time 2"), Some((1, "time")), "a stamp read as the start: wired straight");
    let (t, o) = conv.graph.feeder(2, "time 1").expect("the end stamp goes through a Time node");
    assert!(matches!(conv.nodes[t].kind, Kind::Time { .. }) && o == "result");
    assert_eq!(conv.graph.feeder(t, "time"), Some((0, "time")));
    assert_eq!(conv.graph.feeder(t, "row 1"), Some((0, "length")));
    assert!(conv.nodes[2].kind.rows().iter().any(|(k, _)| *k == format!("anchor {}", ids[1])), "the old time input's recording holds the anchor");
    assert!(conv.ev.stamp_end.is_empty());
    assert!(!conv.project_text().contains("stamp-end"));
}

/// Dropping a SOURCER's audio on a row draws its time wire too, as a normal wire.
#[test]
fn dropping_audio_on_a_row_draws_its_time_wire() {
    let mut d = crate::drive::Drive::new();
    let app = d.app();
    let s = app.add(Kind::Sourcer { input: "clip.mp4".into() }, Pos2::new(30.0, 40.0));
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(520.0, 40.0));
    d.frames(3);
    let app = d.app();
    let o = app.canvas_origin;
    let (from, to) = (app.port_place(o, s, 0, true).0, app.port_place(o, m, 0, false).0);
    d.drag(from, to);
    let app = d.app();
    assert_eq!(app.graph.feeder(m, "audio 1"), Some((s, "audio")), "{}", app.msg);
    assert_eq!(app.graph.feeder(m, "time 1"), Some((s, "time")));
    assert_eq!(app.graph.nodes()[m].spec.inputs.len(), 4, "and a free row");
    let (a, t) = (app.port_place(o, m, 0, false).0, app.port_place(o, m, 1, false).0);
    assert!(a.y < t.y && (a.x - t.x).abs() < 0.01, "audio above time: {a:?} {t:?}");
}

/// The Time node: a wired time shows and locks; a row wired to the recording's length takes it
/// away, turning an end stamp into a start; the error box replaces the error; a date that does
/// not exist greys the output, and the Audio match row says so.
#[test]
fn time_node_corrects_a_time() {
    use corroborate::timebase::{parse_iso8601, FileFacts};
    let mut d = crate::drive::Drive::new();
    let t0 = parse_iso8601("2012-02-29T13:18:01Z").unwrap();
    d.app().facts.put("clip.mp4", FileFacts { stamp: Some(t0), len: Some(150.0), source: "the file's creation_time".into() });
    let app = d.app();
    let s = app.add(Kind::Sourcer { input: "clip.mp4".into() }, Pos2::new(30.0, 40.0));
    let t = app.add(Kind::Time { rows: corroborate::modules::time::defaults() }, Pos2::new(520.0, 40.0));
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(1080.0, 40.0));
    app.wire(s, "time", t, "time");
    app.wire(t, "result", m, "time 1");
    app.wire(s, "audio", m, "audio 1");
    app.sync_many_ports();
    d.frames(3);
    let out = |d: &mut crate::drive::Drive| d.app().value_from(t, "result");
    assert!(matches!(out(&mut d), Ok(Data::Time { utc: Some(u), error_s, .. }) if u == t0 && error_s == 0.0));
    d.click("+");
    assert_eq!(d.app().graph.nodes()[t].spec.inputs.len(), 2, "a row and its port");
    d.app().wire(s, "length", t, "row 1");
    d.frames(3);
    assert!(matches!(out(&mut d), Ok(Data::Time { utc: Some(u), .. }) if (u - (t0 - 150.0)).abs() < 1e-3));
    assert_eq!(d.app().match_view(m).rows[0].meta, Some(t0 - 150.0), "the row follows the edit");
    d.shot("time_node");
    // Two years back from 29 February is a date that does not exist.
    if let Kind::Time { rows } = &mut d.app().nodes[t].kind {
        corroborate::modules::time::add_row(rows);
        rows.push(("2.year".into(), "2".into()));
    }
    let inputs = d.app().nodes[t].kind.spec().inputs;
    d.app().graph.set_inputs_keep(t, inputs);
    d.app().graph.disconnect(t, "row 1");
    if let Kind::Time { rows } = &mut d.app().nodes[t].kind {
        corroborate::modules::time::remove_row(rows, 1);
    }
    let inputs = d.app().nodes[t].kind.spec().inputs;
    d.app().graph.set_inputs_keep(t, inputs);
    d.frames(3);
    let e = out(&mut d).unwrap_err();
    assert!(e.contains("2010-02-29"), "{e}");
    assert!(d.app().output_held(t, "result").is_some(), "greyed");
    let view = d.app().match_view(m);
    assert_eq!(crate::audio_match_node::cell(&view, 0).0, crate::audio_match_node::Cell::Bad("Time node error".into()));
    d.shot("time_node_error");
    // Saved and loaded with its rows and wires.
    let text = d.app().project_text();
    let mut back = App::empty();
    assert!(back.load_project_text(&text));
    assert_eq!(back.nodes[t].kind.rows(), d.app().nodes[t].kind.rows());
    assert_eq!(back.graph.nodes()[t].spec.inputs.len(), 2);
}

/// A clock with no real time keeps its own playhead, linked to nothing, even to a clock audio ties
/// it to. Once it has real time it is on the one real-time playhead, at the same moment.
#[test]
fn one_real_time_playhead() {
    use corroborate::clock::{Anchor as An, Link};
    let mut app = App::empty();
    let (a, b) = (source(&mut app, 40.0), source(&mut app, 300.0));
    let (ca, cb) = (uid(&app, a), uid(&app, b));
    app.ev.manual_links.push(Link { a: ca.clone(), t_a: 20.0, b: cb.clone(), t_b: 0.0, error_s: 0.01, from: "manual".into(), enabled: true });
    app.update_clocks();
    app.set_playhead(&ca, 12.0);
    assert_eq!(app.playhead_on(&ca), Some(12.0));
    assert_eq!(app.playhead_on(&cb), None, "linked by audio, but no real time: its own");
    app.ev.manual_anchors.push(An { clock: ca.clone(), t: 0.0, utc: 1000.0, error_s: 0.01, from: "manual".into(), enabled: true });
    app.update_clocks();
    assert!((app.ph.utc.unwrap() - 1012.0).abs() < 1e-6, "{:?}", app.ph.utc);
    assert!(app.ph.floating.is_empty());
    assert!((app.playhead_on(&cb).unwrap() + 8.0).abs() < 1e-3, "{:?}", app.playhead_on(&cb));
    app.set_utc_playhead(1030.0);
    assert!((app.playhead_on(&ca).unwrap() - 30.0).abs() < 1e-3);
    assert!((app.playhead_on(&cb).unwrap() - 10.0).abs() < 1e-3);
}

/// A clock's error is smallest at its evidence and grows with unknown drift.
#[test]
fn errors_grow_away_from_the_evidence() {
    use corroborate::clock::Anchor as An;
    let clocks = corroborate::clock::solve(&[An { clock: "a".into(), t: 0.0, utc: 1000.0, error_s: 0.01, from: "n".into(), enabled: true }], &[]);
    let c = clocks.get("a");
    assert!((c.error_at(0.0) - 0.01).abs() < 1e-6 && (c.error_at(3600.0) - 0.36).abs() < 0.01);
    assert!((c.back(c.at(1234.5)) - 1234.5).abs() < 1e-9);
}

/// The Timeline node: a bar at the time wired in, as long as the media, with margins from the
/// time's error; a row with no time has no bar; rows sort by time, ports following; a click on the
/// ruler moves the real-time playhead.
#[test]
fn timeline_node_draws_what_it_is_given() {
    use corroborate::timebase::FileFacts;
    let mut d = crate::drive::Drive::new();
    for (p, t) in [("a.mp4", 1000.0), ("b.mp4", 500.0)] {
        d.app().facts.put(p, FileFacts { stamp: Some(t), len: Some(100.0), source: "the file's creation_time".into() });
    }
    d.app().facts.put("c.mp4", FileFacts { stamp: None, len: Some(40.0), source: String::new() });
    let app = d.app();
    let (a, b, c) = (app.add(Kind::Sourcer { input: "a.mp4".into() }, Pos2::new(30.0, 40.0)), app.add(Kind::Sourcer { input: "b.mp4".into() }, Pos2::new(30.0, 330.0)), app.add(Kind::Sourcer { input: "c.mp4".into() }, Pos2::new(30.0, 620.0)));
    let t = app.add(Kind::Timeline { rows: Vec::new() }, Pos2::new(560.0, 40.0));
    for s in [a, b, c] {
        app.wire_many(s, "audio", t);
    }
    assert_eq!(app.graph.nodes()[t].spec.inputs.len(), 8, "rows of media and time: three wired, one free");
    app.wire(a, "time", t, "time 1");
    app.wire(b, "time", t, "time 2");
    app.wire(c, "time", t, "time 3");
    d.frames(3);
    let rows = d.app().timeline_rows(t);
    assert_eq!((rows[0].at, rows[1].at, rows[2].at), (Some((1000.0, 0.0)), Some((500.0, 0.0)), None));
    assert_eq!(rows[2].len, 40.0);
    let o = d.app().canvas_origin;
    let y1 = d.app().port_place(o, t, 0, false).0.y;
    d.shot("timeline_node");
    if let Kind::Timeline { rows } = &mut d.app().nodes[t].kind {
        rows.push(("sort by time".into(), "true".into()));
    }
    d.frames(3);
    assert!((d.app().port_place(o, t, 2, false).0.y - y1).abs() < 0.01 && d.app().port_place(o, t, 0, false).0.y > y1, "row 2 is earlier, so it is drawn first and its ports follow");
    let track = *d.app().match_ui.tracks.last().unwrap();
    d.click_at(track.center().x, track.top() + 8.0);
    let u = d.app().ph.utc.expect("a click on the ruler sets the real-time playhead");
    assert!(u > 500.0 && u < 1100.0, "{u}");
    d.shot("timeline_node_sorted");
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
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(30.0 + 470.0 * cols as f32 + 80.0, 40.0));
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

/// The time written in a file never anchors its clock, wired or not; a live source's own time
/// does. The file's time is on the output from before any Run, raw, with an error of 0, and a Run
/// does not clear it; so is the file's length.
#[test]
fn a_file_stamp_is_a_claim_only() {
    use corroborate::timebase::FileFacts;
    let mut app = App::empty();
    let s = app.add(Kind::Sourcer { input: "clip.mp4".into() }, Pos2::new(30.0, 40.0));
    let p = app.add(Kind::Planes, Pos2::new(300.0, 40.0));
    app.wire(s, "audio", p, "audio");
    assert_eq!(app.value_from(s, "time"), Err("still reading the file".to_string()));
    app.facts.put("clip.mp4", FileFacts { stamp: Some(1000.0), len: Some(60.0), source: "the file's creation_time".into() });
    let c = app.clock_of(s).unwrap();
    assert_eq!(app.value_from(s, "time"), Ok(Data::Time { clock: c, t: 0.0, utc: Some(1000.0), error_s: 0.0, source: "the file's creation_time".into() }));
    assert_eq!(app.value_from(s, "length"), Ok(Data::Custom { kind: "length", json: "{\"seconds\":60}".into() }));
    app.nodes[s].state.lock().unwrap().anchor = Some(Anchor { utc0: 1000.0, error_s: 60.0, source: "file".into() });
    app.update_clocks();
    assert!(app.ev.anchors.is_empty(), "unwired: not used {:?}", app.ev.anchors);
    app.wire(s, "time", p, "time");
    assert_eq!(app.graph.wires().len(), 2, "{}", app.msg);
    app.update_clocks();
    assert!(app.ev.anchors.is_empty(), "wired: still only a claim {:?}", app.ev.anchors);
    app.nodes[s].state.lock().unwrap().reset();
    assert!(app.value_from(s, "time").is_ok(), "a Run clears the state, not what the file says");
    // Not a file: its own time counts.
    let live = source(&mut app, 300.0);
    anchor(&app, live, 2000.0, 1.0);
    app.update_clocks();
    assert_eq!(app.ev.anchors.len(), 1);
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

/// Makes app\projects\Jiku group 6: the 12 recordings the audio match links to
/// GT_090912_0_1347196530871, found by matching all 50 videos in memory, and an Audio match, sources
/// collapsed, first in Open recent. The 50-video project is not touched.
/// Run: cargo test --profile fast --bin corroborate make_jiku_group6 -- --ignored --nocapture
#[test]
#[ignore]
fn make_jiku_group6() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let data = root.join("tests").join("data").join("jiku").join("GT_090912");
    let mut videos: Vec<PathBuf> = std::fs::read_dir(&data).expect("no Jiku data").filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "mp4")).collect();
    videos.sort();
    assert_eq!(videos.len(), 50);
    let mut d = crate::drive::Drive::new();
    let app = d.app();
    let sources: Vec<NodeId> = videos.iter().map(|v| app.add(Kind::Sourcer { input: v.to_string_lossy().into_owned() }, Pos2::ZERO)).collect();
    for &s in &sources {
        app.nodes[s].collapsed = true;
    }
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::ZERO);
    for &s in &sources {
        app.wire_many(s, "audio", m);
    }
    app.run();
    assert!(d.wait_until(300.0, |a| a.nodes[m].state.lock().unwrap().done), "the match ends");
    d.frames(3);
    let app = d.app();
    app.update_clocks();
    let group_of = |app: &App, s: NodeId| app.ev.clocks.get(&uid(app, s)).group;
    let key = videos.iter().position(|v| v.to_string_lossy().contains("GT_090912_0_1347196530871")).expect("the video");
    let group = group_of(app, sources[key]);
    let members: Vec<PathBuf> = (0..50).filter(|&k| group_of(app, sources[k]) == group).map(|k| videos[k].clone()).collect();
    app.stop_run();
    assert_eq!(members.len(), 12, "group 6 has 12 recordings");

    let mut e = crate::drive::Drive::new();
    let app = e.app();
    let cols = 3;
    let mut made = Vec::new();
    for (k, v) in members.iter().enumerate() {
        let s = app.add(Kind::Sourcer { input: v.to_string_lossy().into_owned() }, Pos2::new(30.0 + 290.0 * (k % cols) as f32, 40.0 + 190.0 * (k / cols) as f32));
        let stem = v.file_stem().unwrap().to_string_lossy().into_owned();
        let parts: Vec<&str> = stem.split('_').collect();
        app.nodes[s].label = format!("phone {} {}", parts[2], parts[3]);
        app.nodes[s].collapsed = true;
        made.push(s);
    }
    let a = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(30.0 + 290.0 * cols as f32 + 120.0, 40.0));
    for &s in &made {
        app.wire_many(s, "audio", a);
    }
    let file = project::project_place(&root.join("app").join("projects").join("Jiku group 6"));
    let _ = std::fs::remove_file(project::recovery_of(&file));
    app.save_project_as(file.clone());
    assert!(file.is_file());
    project::push_recent(&root.join("app").join("settings"), &file);
    println!("saved {}", file.display());
}

/// The Audio match rows on the 12 recordings, handled like a user would (a copy of the project in
/// target\drive, saved before rows had a time each, so converted on opening): times shown before
/// Run, the audio sync over them after, a group dragged, a group anchored by a click on a row's
/// anchor, thin rows, the track zoomed and panned, a row hovered. PNGs am_1 to am_7.
/// Run: cargo test --profile fast --bin corroborate audio_match_timeline -- --ignored --nocapture
#[test]
#[ignore]
fn audio_match_timeline() {
    use crate::audio_match_node::{cell, Cell};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let made = root.join("app").join("projects").join("Jiku group 6").join("Jiku group 6.txt");
    assert!(made.is_file(), "make it first: make_jiku_group6");
    let dir = crate::drive::out_dir().join("Jiku group 6 copy");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Jiku group 6 copy.txt");
    std::fs::copy(&made, &file).unwrap();
    let mut d = crate::drive::Drive::new();
    d.app().open_project(file);
    d.frames(3);
    let m = d.app().nodes.iter().position(|n| matches!(n.kind, Kind::AudioMatch { .. })).unwrap();
    assert!(d.app().stopped && !d.app().running(), "opens stopped");
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end && d.app().match_view(m).rows.iter().filter(|r| r.meta.is_some()).count() < 5 {
        d.h.step();
        std::thread::sleep(Duration::from_millis(100));
    }
    d.wait(3.0);
    d.app().pick_only(m);
    let area = d.app().canvas_rect;
    d.app().frame_nodes(area);
    d.app().pick_none();
    d.frames(4);
    let v = d.app().match_view(m);
    let stamped = v.rows.iter().filter(|x| x.meta.is_some()).count();
    println!("before Run: {} rows, {stamped} with a time", v.rows.iter().filter(|r| r.wired).count());
    assert_eq!(v.rows.iter().filter(|r| r.wired).count(), 12);
    assert!(v.rows.iter().all(|x| x.start.is_none()), "no audio result before Run");
    assert!(!d.app().running(), "reading the times decodes nothing");
    d.shot("am_1_before_run");

    d.click("▶ Run");
    d.wait(1.5);
    d.shot("am_2_running");
    assert!(d.wait_until(120.0, |a| a.nodes[m].state.lock().unwrap().done), "the match ends");
    d.wait(1.0);
    d.app().update_clocks();
    d.frames(3);
    let v = d.app().match_view(m);
    for (k, x) in v.rows.iter().enumerate().filter(|(_, r)| r.wired) {
        println!("{:28} {:?} meta {:?} start {:?} hint {}", x.name, cell(&v, k).0, x.meta.map(|b| b as i64), x.start.map(|b| b as i64), x.hint);
    }
    assert_eq!(v.rows.iter().filter(|x| x.start.is_some()).count(), 12, "every row has its audio position");
    assert_eq!(v.groups.iter().filter(|g| g.number > 0).count(), 1, "one synced group");
    d.shot("am_3_after_the_match");
    assert!(d.app().value_from(m, "synced 1").is_err(), "no anchor: nothing comes out");

    // A drag on an amber bar moves the group and changes no data.
    let track = d.app().match_ui.tracks[0];
    let k = v.rows.iter().position(|r| r.meta.is_some()).expect("a row with a time");
    let on = d.app().match_ui.anchor_at[&(m, k)];
    let (t0, links0) = (v.rows[k].start.unwrap(), d.app().ev.links.len());
    let (v0, v1) = d.app().match_ui.views.get(&m).copied().unwrap_or((0.0, 1.0));
    let _ = (v0, v1);
    let y = on.center().y;
    let bar_x = {
        let v = d.app().match_view(m);
        let bars: Vec<(f64, f64)> = v.rows.iter().flat_map(|r| [r.meta.map(|t| (t, t + r.len)), r.start.map(|t| (t, t + r.len))]).flatten().collect();
        let lo = bars.iter().map(|b| b.0).fold(f64::MAX, f64::min);
        let hi = bars.iter().map(|b| b.1).fold(f64::MIN, f64::max);
        let pad = ((hi - lo) * 0.05).max(1.0);
        let (a, b) = (lo - pad, hi + pad);
        let r = &v.rows[k];
        track.left() + (((r.start.unwrap() + r.len / 2.0 - a) / (b - a)) as f32) * track.width()
    };
    d.drag(Pos2::new(bar_x, y), Pos2::new(bar_x + 60.0, y));
    let v = d.app().match_view(m);
    println!("dragged the group by {:.1} s", v.rows[k].start.unwrap() - t0);
    assert!(v.rows[k].start.unwrap() > t0, "the group moved");
    assert_eq!(d.app().ev.links.len(), links0, "and no data did");
    d.shot("am_4_group_dragged");

    // A click on a row's anchor: the group has real time and the yellow outputs give it.
    d.click_at(on.center().x, on.center().y);
    d.frames(3);
    d.app().update_clocks();
    d.frames(3);
    let v = d.app().match_view(m);
    assert_eq!(cell(&v, k).0, Cell::Anchor, "{}", d.msg());
    assert!((v.rows[k].start.unwrap() - v.rows[k].meta.unwrap()).abs() < 0.01, "the audio's start snaps to the time");
    for r in v.rows.iter().filter(|r| r.wired) {
        assert!(d.app().value_from(m, &format!("synced {}", r.n)).is_ok(), "row {} gives a time", r.n);
    }
    assert!(d.app().ev.clocks.contradictions.is_empty(), "{:?}", d.app().ev.clocks.contradictions.len());
    for (k, x) in v.rows.iter().enumerate().filter(|(_, r)| r.wired) {
        println!("{:28} {:?}", x.name, cell(&v, k).0);
    }
    d.shot("am_5_anchored");

    // The track: wheel zooms it and not the canvas, a drag pans it.
    let zoom = d.app().zoom;
    let at = Pos2::new(track.center().x, track.top() + 80.0);
    d.scroll(at, 8.0);
    let w = d.app().match_ui.views.get(&m).copied().expect("zoomed");
    println!("zoomed to {:.0} s across", w.1 - w.0);
    assert_eq!(d.app().zoom, zoom, "the wheel over the track is the track's");
    d.shot("am_6_zoomed");
    d.h.hover_at(Pos2::new(track.center().x, y));
    d.frames(10);
    d.shot("am_7_hover");
    if let Kind::AudioMatch { rows } = &mut d.app().nodes[m].kind {
        rows.push(("row height".into(), "24".into()));
    }
    d.app().match_ui.views.remove(&m);
    d.frames(4);
    d.shot("am_8_thin_rows");
    // The yellow outputs into a Timeline node: every recording at its real time.
    let pos = d.app().nodes[m].pos + Vec2::new(1100.0, 0.0);
    let tl = d.app().add(Kind::Timeline { rows: Vec::new() }, pos);
    let wired: Vec<usize> = d.app().match_view(m).rows.iter().filter(|r| r.wired).map(|r| r.n).collect();
    for n in wired {
        let (s, _) = d.app().graph.feeder(m, &format!("audio {n}")).unwrap();
        d.app().wire_many(s, "audio", tl);
        d.app().wire(m, &format!("synced {n}"), tl, &format!("time {n}"));
    }
    d.frames(3);
    assert!(d.app().timeline_rows(tl).iter().filter(|r| r.wired).all(|r| r.at.is_some() && r.len > 0.0), "{:?}", d.app().timeline_rows(tl));
    let area = d.app().canvas_rect;
    d.app().frame_nodes(area);
    d.frames(4);
    d.shot("am_9_timeline_node");
    d.app().stop_run();
}

