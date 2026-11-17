//! The project's clocks (evidence.rs, playhead.rs): which clock each node's times are on, Same
//! clock, old evidence lines dropped, which recordings follow the real-time playhead (their times
//! wired into a Timeline), and Audio match.
//! Run: cargo test --bin corroborate clock_tests

use super::*;
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

/// One recording puts a source in another's recording, saved with the project (not with pasted nodes);
/// Separate recordings undoes it. An old project's evidence lines (anchors, links, switches) are read,
/// dropped, and counted in a message.
#[test]
fn sources_share_clocks_and_old_evidence_is_dropped() {
    let mut app = App::empty();
    let (a, b) = (source(&mut app, 40.0), source(&mut app, 300.0));
    let (ua, ub) = (uid(&app, a), uid(&app, b));
    app.same_clock(&[a, b]);
    assert_eq!(app.clock_of(b), Some(ua.clone()), "b is on a's clock");
    assert!(app.shares_clock(a) && app.shares_clock(b));
    let mut back = App::empty();
    assert!(back.load_project_text(&app.project_text()));
    assert_eq!(back.ev.share, app.ev.share);
    assert!(back.msg.is_empty() || !back.msg.contains("dropped"), "{}", back.msg);
    // Pasting the nodes does not bring the project's clocks.
    let mut pasted = App::empty();
    pasted.append_text(&app.project_text(), false);
    assert!(pasted.ev.share.is_empty());

    let old = format!("{}anchor|{ub}|3|2000|0.5\nanchor|{ua}|0|1000|1\nlink|{ua}|1|n7|2|0.25\noff|anchor n9 n1 0.000\nstamp-end|{ua}\n", app.project_text());
    let mut back = App::empty();
    assert!(back.load_project_text(&old));
    assert!(back.msg.contains("2 anchors, 1 link, 1 evidence switch"), "{}", back.msg);
    let again = back.project_text();
    assert!(!again.contains("anchor|") && !again.contains("link|") && !again.contains("off|") && !again.contains("stamp-end"), "{again}");
    assert_eq!(back.ev.share, app.ev.share, "the clock lines are kept");

    app.separate_clocks(&[b]);
    assert_eq!(app.clock_of(b), Some(ub), "b has its own clock again");
    assert!(app.ev.share.is_empty(), "a shares with no one: back to its own");
}

/// No time reaches a recording without a wire: a time wired into Planes is an input like any
/// other, and the recording keeps its own playhead. Wired into a Timeline with the recording, the
/// recording follows the real-time playhead.
#[test]
fn no_time_reaches_a_recording_without_a_wire() {
    let mut app = App::empty();
    let s = source(&mut app, 40.0);
    let p = app.add(Kind::Planes, Pos2::new(300.0, 40.0));
    let v = app.add(Kind::Value { ty: "time".into(), rows: vec![("utc".into(), "2026-11-06T12:00:00Z".into()), ("error_s".into(), "0.5".into())] }, Pos2::new(30.0, 300.0));
    app.wire(s, "audio", p, "audio");
    app.wire(v, "value", p, "time");
    assert_eq!(app.graph.wires().len(), 2, "{}", app.msg);
    app.update_follow();
    assert!(app.ph.follow.is_empty(), "wired into Planes only: its own playhead");
    let tl = app.add(Kind::Timeline { rows: Vec::new() }, Pos2::new(700.0, 40.0));
    app.wire_many(s, "audio", tl);
    app.wire(v, "value", tl, "time 1");
    app.update_follow();
    let f = app.ph.follow.get(&uid(&app, s)).copied().expect("wired into a Timeline: it follows");
    assert_eq!((f.t0, f.k), (0.0, 1.0));
    assert!((f.u0 - 1_793_966_400.0).abs() < 1e-6, "{}", f.u0);
}

/// A manifest can no longer feed a pool: the `anchors` key and the anchor and link types are gone.
#[test]
fn the_pool_types_are_retired() {
    let head = "id = \"x\"\nlabel = \"X\"\ncategory = \"Checks\"\n[[input]]\nname = \"audio\"\nkind = \"audio\"\n";
    let e = mm::parse(&format!("{head}[[input]]\nname = \"time\"\nkind = \"time\"\nanchors = \"audio\"\n")).unwrap_err();
    assert!(e.contains("retired"), "{e}");
    for k in ["anchor", "link"] {
        assert!(mm::parse(&format!("{head}[[output]]\nname = \"out\"\nkind = \"{k}\"\n")).is_err(), "{k} is no type");
    }
    assert!(mm::parse(&format!("{head}[[output]]\nname = \"audio start time\"\nkind = \"time\"\n")).is_ok());
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
    let tl = app.add(Kind::Timeline { rows: Vec::new() }, Pos2::new(700.0, 40.0));
    let v = app.add(Kind::Value { ty: "time".into(), rows: vec![("utc".into(), "1970-01-01 00:16:40".into())] }, Pos2::new(30.0, 300.0));
    app.wire_many(a, "audio", tl);
    app.wire(v, "value", tl, "time 1");
    app.update_follow();
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
    app.pick_only(pl);
    let c = app.clock_of(s).unwrap();
    app.set_playhead(&c, 0.05);
    assert!(app.sound_target().is_none(), "paused: no sound");
    app.ph.playing = true;
    let (a, t) = app.sound_target().unwrap();
    assert_eq!((a.lock().unwrap().len(), t), (1600, 0.05));
}

/// The result file carries no solved clocks or evidence pool any more: each time carries its own
/// trace.
#[test]
fn the_result_file_has_no_pool() {
    let mut app = App::empty();
    let a = source(&mut app, 40.0);
    let j = app.result_json(&[a]);
    assert!(!j.contains("\"clocks\":[") && !j.contains("\"evidence\":[") && !j.contains("not_comparable"), "{j}");
    assert!(j.trim_end().ends_with('}'), "{j}");
}

/// Audio match: two files that heard one event (b starts 20.5 s into it, a at its start; b's
/// microphone hears an echo and its own noise) wired into an Audio match node give links naming
/// both clocks, and the node's own fit puts b's 10 s at a's 30.5 s, in one group. A third file, a
/// quieter copy of a, is marked the same recording as a (and b is not); a and its copy's links
/// span 45 s, enough for their drift (none). The anchor's end wired 0.4 s past start + length
/// moves every end by the anchor's real speed. PNGs audio_match_drift and audio_match_marks.
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
    assert_eq!(app.graph.nodes()[m].spec.inputs.len(), 9, "rows of audio, start time and end time: two wired and one free");
    assert_eq!(app.graph.nodes()[m].spec.outputs.iter().filter(|p| p.name.starts_with("synced")).count(), 3, "a start time output per row");
    assert_eq!(app.graph.nodes()[m].spec.outputs.iter().filter(|p| p.name.starts_with("end ")).count(), 3, "an end time output per row");
    app.wire_many(c, "audio", m);
    app.run();
    assert!(d.wait_until(30.0, |a| a.nodes[m].state.lock().unwrap().done), "the match ends");
    d.frames(5);
    d.shot("audio_match");
    // On the node: each row placed by audio once the match has ended. These files have no stamp.
    d.frames(2);
    let view = d.app().match_view(m);
    let wired: Vec<_> = view.rows.iter().filter(|r| r.wired).collect();
    assert_eq!(wired.len(), 3);
    assert!(wired.iter().all(|r| r.start.is_some() && r.meta.is_none()), "{wired:?}");
    assert_eq!(view.groups.iter().filter(|g| g.number > 0).count(), 1, "one synced group");
    assert_eq!(crate::audio_match_node::cell(&view, 0).0, crate::audio_match_node::Cell::Plain("no time metadata".into()));
    assert!(d.app().value_from(m, "synced 2").is_err(), "no time metadata in the group: nothing comes out");
    assert!(d.app().output_held(m, "synced 2").is_some(), "greyed");
    // A time on row 1: unanchored, the group is placed by its metadata and its times come out.
    let v = d.app().add(Kind::Value { ty: "time".into(), rows: vec![("utc".into(), "1970-01-01T00:16:40Z".into()), ("error_s".into(), "2".into())] }, Pos2::new(30.0, 800.0));
    d.app().wire(v, "value", m, "time 1");
    d.frames(3);
    let Ok(Data::Time { utc: free_b, source, .. }) = d.app().value_from(m, "synced 2") else { panic!("no unanchored time out of row 2") };
    assert!((free_b - 1020.5).abs() < 0.01 && source.contains("unanchored: the average of 1 phone's stamps"), "{free_b} {source}");
    let len_b = d.app().match_view(m).rows[1].len;
    let Ok(Data::Time { utc: end_b, .. }) = d.app().value_from(m, "end 2") else { panic!("no end time out of row 2") };
    assert!((end_b - free_b - len_b).abs() < 1e-6, "the end is the start plus the length");
    d.shot("audio_match_unanchored");

    // A drag on row 1's amber bar moves the group: saved with the project, logged, undone.
    d.wait(0.5);
    let track = d.app().match_ui.tracks.last().copied().unwrap();
    let y = d.app().match_ui.anchor_at[&(m, 0)].center().y;
    let bar_x = {
        let v = d.app().match_view(m);
        let bars: Vec<(f64, f64)> = v.rows.iter().flat_map(|r| [r.meta.map(|t| (t, t + r.len)), r.start.map(|t| (t, t + r.len))]).flatten().collect();
        let (lo, hi) = crate::audio_match_node::fit_axis(&bars);
        let r = &v.rows[0];
        track.left() + (((r.start.unwrap() + r.len / 2.0 - lo) / (hi - lo)) as f32) * track.width()
    };
    let logged = d.app().changes.len();
    d.drag(Pos2::new(bar_x, y), Pos2::new(bar_x + 60.0, y));
    d.frames(2);
    let Ok(Data::Time { utc: moved_b, source, .. }) = d.app().value_from(m, "synced 2") else { panic!("no time after the drag") };
    assert!(moved_b > free_b + 1.0 && source.contains("moved by hand"), "the output moved: {free_b} -> {moved_b} {source}");
    assert_eq!(d.app().changes.len(), logged + 1, "the drag is logged");
    assert!(d.app().changes.last().unwrap().name.starts_with("drag of the group"));
    let text = d.app().project_text();
    assert!(text.contains("shift "), "kept with the project");
    let mut back = App::empty();
    assert!(back.load_project_text(&text));
    assert!(back.nodes[m].kind.rows().iter().any(|(k, v)| k.starts_with("shift ") && v.parse::<f64>().is_ok_and(|s| s > 1.0)), "and read back");
    d.shot("audio_match_dragged");
    d.wait(0.5);
    d.h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    d.frames(3);
    let Ok(Data::Time { utc: undone_b, .. }) = d.app().value_from(m, "synced 2") else { panic!("no time after undo") };
    assert!((undone_b - free_b).abs() < 0.01, "Ctrl+Z puts the group back: {undone_b}");

    // The group anchored on row 1's time: every recording of the group has real time.
    d.app().toggle_anchor(m, 0);
    d.frames(3);
    let Ok(Data::Time { utc: ub_utc, error_s, .. }) = d.app().value_from(m, "synced 2") else { panic!("no time out of row 2") };
    assert!((ub_utc - 1020.5).abs() < 0.01 && error_s >= 2.0, "b starts 20.5 s after a: {ub_utc} ± {error_s}");
    let Ok(Data::Time { utc: ua_utc, .. }) = d.app().value_from(m, "synced 1") else { panic!("no time out of row 1") };
    assert!((ua_utc - 1000.0).abs() < 0.01, "{ua_utc}");
    let view = d.app().match_view(m);
    assert_eq!(crate::audio_match_node::cell(&view, 0).0, crate::audio_match_node::Cell::Anchor);
    assert!(d.app().project_text().contains(&format!("anchor {}", uid(d.app(), a))), "kept with the project");
    // Row 2's metadata 2.5 s later than audio puts it, row 3's 2 s earlier: the Status cell is
    // split at zero, later filling the right half, earlier the left.
    let t2 = d.app().add(Kind::Value { ty: "time".into(), rows: vec![("utc".into(), "1970-01-01T00:17:03Z".into()), ("error_s".into(), "1".into())] }, Pos2::new(30.0, 900.0));
    d.app().wire(t2, "value", m, "time 2");
    let t3 = d.app().add(Kind::Value { ty: "time".into(), rows: vec![("utc".into(), "1970-01-01T00:16:38Z".into()), ("error_s".into(), "1".into())] }, Pos2::new(30.0, 1000.0));
    d.app().wire(t3, "value", m, "time 3");
    d.frames(3);
    let view = d.app().match_view(m);
    let off = |k: usize| match crate::audio_match_node::cell(&view, k).0 {
        crate::audio_match_node::Cell::Off(d, _) => d,
        c => panic!("row {}: {c:?}", k + 1),
    };
    assert!((off(1) - 2.5).abs() < 0.01 && (off(2) + 2.0).abs() < 0.01, "{} {}", off(1), off(2));
    d.shot("audio_match_anchored");
    // The Order menu: open, then by how far off.
    d.click_contains("Order:");
    d.frames(2);
    d.shot("audio_match_order_menu");
    d.click("Group › how far off");
    d.frames(2);
    assert!(d.app().nodes[m].kind.rows().iter().any(|(k, v)| k == "order" && v == "group-off"), "the order is kept");
    assert_eq!(d.app().match_ui.slots[&m][..3], [0, 2, 1], "the anchor, then 2 s off, then 2.5 s off");
    d.shot("audio_match_by_how_far_off");
    // The Drift column: every row's drift against the group's average clock, the anchor's too,
    // near 0.
    use crate::audio_match_node::{drift_cell, DriftCell, DriftFrom};
    assert_eq!(view.groups[view.rows[0].group].drift, DriftFrom::Average);
    for k in [0, 1, 2] {
        let DriftCell::Speed(ppm, _, None) = drift_cell(&view, k) else { panic!("row {}: {:?}", k + 1, drift_cell(&view, k)) };
        assert!(ppm.abs() < 300.0, "row {}: {ppm} ppm", k + 1);
    }
    let Ok(Data::Time { source, .. }) = d.app().value_from(m, "end 2") else { panic!("no end out of row 2") };
    assert!(source.contains("measured drift") && source.contains("s/h against the group's average clock") && source.contains("fitted on"), "{source}");
    d.shot("audio_match_drift");
    // The anchor's end wired 0.4 s past start + length: its clock runs slow against real time, so
    // every row's seconds are longer, and the anchor's Drift cell gives its real drift.
    let (len_a, (s2, e2)) = {
        let v = d.app().match_view(m);
        (v.rows[0].len, (v.rows[1].start.unwrap(), v.rows[1].end.unwrap()))
    };
    let end_a = d.app().add(Kind::Value { ty: "time".into(), rows: vec![("utc".into(), timebase::fmt_utc(1000.0 + len_a + 0.4)), ("error_s".into(), "0.1".into())] }, Pos2::new(30.0, 1100.0));
    d.app().wire(end_a, "value", m, "end in 1");
    d.frames(3);
    let view = d.app().match_view(m);
    let k = (len_a + 0.4) / len_a;
    assert_eq!(view.groups[view.rows[0].group].drift, DriftFrom::AnchorEnd(0));
    let DriftCell::Speed(real, _, Some(wired)) = drift_cell(&view, 0) else { panic!("{:?}", drift_cell(&view, 0)) };
    assert!((real - (1.0 / k - 1.0) * 1e6).abs() < 1.0 && wired.abs() < 1e-6, "the anchor runs {real} ppm against real time, its end on its wired end");
    // Every row's seconds grow by one factor, the anchor's real speed against the average clock.
    let (s2k, e2k) = (view.rows[1].start.unwrap(), view.rows[1].end.unwrap());
    let grew = (s2k - 1000.0) / (s2 - 1000.0);
    assert!(((e2k - s2k) / (e2 - s2) - grew).abs() < 1e-6 && (grew / k - 1.0).abs() < 300e-6, "row 2 moves by the same speed: {s2} {e2} -> {s2k} {e2k}");
    let Ok(Data::Time { utc: end1, .. }) = d.app().value_from(m, "end 1") else { panic!("no end out of row 1") };
    assert!((end1 - (1000.0 + len_a + 0.4)).abs() < 1e-6, "the anchor's end is its wired end: {end1}");
    d.shot("audio_match_anchor_end");
    d.app().graph.disconnect(m, "end in 1");
    d.frames(2);

    // Phone ids, typed on the name strips: a and its copy are one phone (one drift, one vote).
    for k in [0, 2] {
        let at = d.app().match_ui.phone_at[&(m, k)].center();
        d.click_at(at.x, at.y);
        d.h.event(egui::Event::Text("Rode".into()));
        d.frames(1);
        d.press(egui::Key::Enter);
    }
    d.frames(2);
    let view = d.app().match_view(m);
    assert_eq!((view.rows[0].phone.as_str(), view.rows[1].phone.as_str(), view.rows[2].phone.as_str()), ("Rode", "", "Rode"));
    assert!(d.app().changes.last().unwrap().name.starts_with("phone of"), "logged");
    assert!(d.app().project_text().contains("phone "), "kept with the project");
    let (p0, p2) = (view.rows[0].speed.unwrap().0, view.rows[2].speed.unwrap().0);
    assert!((p0 - p2).abs() < 1e-6, "one phone, one drift: {p0} {p2}");
    d.shot("audio_match_phones");
    // The average anchor on the group's tab: it takes the place of row 1's anchor. Each phone
    // votes once for where a starts: Rode with a (1000 ± 2) and its copy (998 ± 1), 998.4 ± 1.26;
    // b's stamp, 20.5 s after a, says 1002.5 ± 1.
    let gi = view.rows[0].group;
    d.app().toggle_average(m, gi);
    d.frames(3);
    let view = d.app().match_view(m);
    let g = &view.groups[gi];
    let avg = g.average.expect("anchored on the average");
    assert!(g.anchor.is_none() && avg.phones == 2, "{g:?}");
    let want = (998.4 / 1.6 + 1002.5) / (1.0 / 1.6 + 1.0);
    assert!((view.rows[0].start.unwrap() - want).abs() < 0.01, "a starts at the phones' average: {} against {want}", view.rows[0].start.unwrap());
    let Ok(Data::Time { source, .. }) = d.app().value_from(m, "synced 2") else { panic!("no time out of row 2") };
    assert!(source.contains("anchored on the average of 2 phones' stamps"), "{source}");
    assert!(view.anchored(&view.rows[1]), "the outputs carry the anchor mark");
    assert!(d.app().changes.last().unwrap().new.contains("average"), "logged");
    d.shot("audio_match_average");
    // Row 1's anchor takes the average's place again.
    d.app().toggle_anchor(m, 0);
    d.frames(2);
    let view = d.app().match_view(m);
    assert!(view.groups[gi].average.is_none() && view.groups[gi].anchor == Some(0));
    // The speed reference: b's clock runs true; the others are measured against it.
    d.app().toggle_speed_ref(m, 1);
    d.frames(3);
    let view = d.app().match_view(m);
    assert_eq!(view.groups[gi].drift, DriftFrom::File(1));
    assert_eq!(drift_cell(&view, 1), DriftCell::True);
    assert!(view.rows[1].speed.unwrap().0.abs() < 1e-6);
    let Ok(Data::Time { source, .. }) = d.app().value_from(m, "end 1") else { panic!("no end out of row 1") };
    assert!(source.contains("(speed reference)"), "{source}");
    d.shot("audio_match_speed_reference");
    d.app().toggle_speed_ref(m, 1);
    d.frames(2);
    assert_eq!(d.app().match_view(m).groups[gi].drift, DriftFrom::Average);
    d.app().graph.disconnect(m, "time 2");
    d.app().graph.disconnect(m, "time 3");
    d.app().toggle_anchor(m, 0);
    d.app().graph.disconnect(m, "time 1");
    d.app().sync_many_ports();
    d.frames(2);
    assert_eq!(d.app().audio_match_inputs(m).0.len(), 3, "the time input is not a recording");
    let app = d.app();
    let st = app.nodes[m].state.lock().unwrap().status.clone();
    let found = app.nodes[m].state.lock().unwrap().found.clone().expect("what the match found");
    assert!(found.quality.contains("\"level\":\"good\""), "{}", found.quality);
    let (ua, ub, uc) = (uid(app, a), uid(app, b), uid(app, c));
    let ab: Vec<_> = found.links.iter().filter(|l| (l.a == ua && l.b == ub) || (l.a == ub && l.b == ua)).collect();
    assert!(ab.len() >= 2, "links between a and b: {} ({st})", ab.len());
    let mut on: Vec<&str> = found.same.iter().map(|e| e.clock.as_str()).collect();
    on.sort();
    let mut want = vec![ua.as_str(), uc.as_str()];
    want.sort();
    assert_eq!(on, want, "a and its copy are one recording, b is not: {:?}", found.same);
    assert!(found.same.iter().all(|e| e.t_end - e.t_start > 40.0), "{:?}", found.same);
    assert_eq!(found.drift.matches("\"ppm\"").count(), 1, "only a and its copy span 30 s of links: {}", found.drift);
    assert!(found.speed.is_empty() && found.gaps.is_empty(), "no speed change, no gaps");
    // Same recording is a mark in the Status cells of a and its copy, with the words on hover.
    let view = app.match_view(m);
    let marks = |k: usize| view.rows[k].marks.iter().map(|x| x.kind).collect::<Vec<_>>();
    use crate::audio_match_node::MarkKind;
    assert_eq!((marks(0), marks(1), marks(2)), (vec![MarkKind::Same], vec![], vec![MarkKind::Same]));
    assert!(view.rows[0].marks[0].text.contains("same recording as"), "{:?}", view.rows[0].marks);
    assert!(app.value_from(m, "gaps").is_err() && app.value_from(m, "quality").is_err(), "the outputs went");
    assert!(view.leftover.is_some_and(|l| l < 0.05), "{:?}", view.leftover);
    // The node's own fit: b's 10 s is a's 30.5 s.
    let fits = crate::audio_match_node::fit_groups(&found.links, &[ua.clone()].into(), &Default::default());
    let on_a = corroborate::clock::read_in(&fits.clocks[&ub], &fits.clocks[&ua]).at(10.0);
    assert!((on_a - 30.5).abs() < 0.002, "b's 10 s is a's {on_a}");
    let json = crate::audio_match_node::json(app, m);
    assert!(json.contains("\"same_recording\":[") && json.contains("\"leftover_s\"") && json.contains("\"s_per_h\":") && json.contains("\"clock_speed\":"), "{json}");
    app.stop_all();
    d.frames(2);
    d.shot("audio_match_marks");
    // A project wiring an output that went: the wire is dropped, and the message says so.
    let mut text = d.app().project_text();
    let id = format!("id={}", uid(d.app(), m));
    let mi = text.lines().filter(|l| l.starts_with("node|")).position(|l| l.split('|').any(|f| f == id)).expect("the node's line");
    text.push_str(&format!("wire|{mi}|gaps|{mi}|audio 9\n"));
    let mut back = App::empty();
    assert!(back.load_project_text(&text));
    assert!(back.msg.contains("1 wire dropped"), "{}", back.msg);
}

/// A row's audio and time, and the wire out of its yellow output, survive save and load by name;
/// a project saved before rows had a time each is not converted any more: its old time wire is
/// dropped and named, and so is a wire from the retired links output.
#[test]
fn audio_match_rows_are_saved_and_old_wires_dropped() {
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
        "node|sourcer|30|40|one|a.mp4|id={}\nnode|sourcer|30|300|two|b.mp4|id={}\nnode|audio-match|460|120|Audio match||id=n9\nnext|10\nwire|0|audio|2|audio 1\nwire|1|audio|2|audio 2\nwire|1|time|2|time\nwire|2|links|1|audio\nstamp-end|{}\n",
        ids[0], ids[1], ids[0]
    );
    let mut conv = App::empty();
    assert!(conv.load_project_text(&old));
    assert_eq!(conv.graph.feeder(2, "audio 2"), Some((1, "audio")));
    assert_eq!(conv.graph.feeder(2, "time 1"), None, "no longer converted");
    assert!(conv.msg.contains("1 wire into its old time input dropped"), "{}", conv.msg);
    assert!(conv.msg.contains("Audio match › links"), "{}", conv.msg);
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
    assert_eq!(app.graph.nodes()[m].spec.inputs.len(), 6, "and a free row (audio, start time, end time)");
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
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(1200.0, 40.0));
    app.wire(s, "time", t, "time");
    app.wire(t, "result", m, "time 1");
    app.wire(s, "audio", m, "audio 1");
    app.sync_many_ports();
    d.frames(3);
    let out = |d: &mut crate::drive::Drive| d.app().value_from(t, "result");
    assert!(matches!(out(&mut d), Ok(Data::Time { utc: u, error_s, .. }) if u == t0 && error_s == 0.0));
    d.click("+ row");
    assert_eq!(d.app().graph.nodes()[t].spec.inputs.len(), 3, "a row and its port, and the waiting port");
    d.app().wire(s, "length", t, "row 1");
    d.frames(3);
    assert!(matches!(out(&mut d), Ok(Data::Time { utc: u, .. }) if (u - (t0 - 150.0)).abs() < 1e-3));
    assert_eq!(d.app().match_view(m).rows[0].meta, Some(t0 - 150.0), "the row follows the edit");
    // The error stack: a row of zeros with only a ± is a judgement, whatever its sign.
    let judge = |d: &mut crate::drive::Drive, sign: &str, e: &str| {
        if let Kind::Time { rows } = &mut d.app().nodes[t].kind {
            if !corroborate::modules::time::numbers(rows).contains(&2) {
                corroborate::modules::time::add_row(rows);
            }
            rows.retain(|(k, _)| k != "2.sign" && k != "2.error_s");
            rows.push(("2.sign".into(), sign.into()));
            rows.push(("2.error_s".into(), e.into()));
        }
        let inputs = d.app().nodes[t].kind.spec().inputs;
        d.app().graph.set_inputs_keep(t, inputs);
        d.frames(2);
    };
    judge(&mut d, "+", "1");
    assert!(matches!(out(&mut d), Ok(Data::Time { utc: u, error_s, .. }) if (u - (t0 - 150.0)).abs() < 1e-3 && (error_s - 1.0).abs() < 1e-9), "{:?}", out(&mut d));
    judge(&mut d, "-", "1");
    assert!(matches!(out(&mut d), Ok(Data::Time { error_s, .. }) if (error_s - 1.0).abs() < 1e-9), "a row's sign does not change its ±");
    // The trace of SOURCER → Time → Audio match names the three nodes in order.
    let names: Vec<String> = d.app().trace(m, "synced 1").into_iter().map(|s| s.node).collect();
    let want: Vec<String> = [s, t, m].iter().map(|&n| d.app().nodes[n].label.clone()).collect();
    assert_eq!(names, want);
    let steps = d.app().trace(t, "result");
    assert!(steps[1].did.contains("− 2 min 30 s (wired length)") && steps[1].did.contains("root sum of squares"), "{:?}", steps[1]);
    assert!(steps[0].did.contains("creation_time"), "{:?}", steps[0]);
    let results = d.app().result_json(&[s, t, m]);
    assert!(results.contains("\"traces\":{\"result\":[{\"node\""), "the result file writes the trace");
    d.shot("time_node");
    // The trace on a hover of the Time node's output.
    let o = d.app().canvas_origin;
    let idx = d.app().graph.nodes()[t].spec.outputs.iter().position(|p| p.name == "result").unwrap();
    let at = d.app().port_place(o, t, idx, true).0;
    d.h.hover_at(at);
    d.wait(1.0);
    d.shot("time_trace_hover");
    if let Kind::Time { rows } = &mut d.app().nodes[t].kind {
        corroborate::modules::time::remove_row(rows, 2);
    }
    // Two years back from 29 February is a date that does not exist.
    if let Kind::Time { rows } = &mut d.app().nodes[t].kind {
        let n = corroborate::modules::time::add_row(rows);
        rows.retain(|(k, _)| *k != format!("{n}.year"));
        rows.push((format!("{n}.year"), "2".into()));
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
    // A file with no time in its metadata through a Time node: no time metadata, not the Time
    // node's error, and the hover says why.
    let bare = d.app().add(Kind::Sourcer { input: "bare.mp4".into() }, Pos2::new(30.0, 400.0));
    d.app().facts.put("bare.mp4", FileFacts { stamp: None, len: Some(90.0), source: "no time in the file's metadata".into() });
    let t2 = d.app().add(Kind::Time { rows: corroborate::modules::time::defaults() }, Pos2::new(520.0, 400.0));
    d.app().wire(bare, "time", t2, "time");
    d.app().wire(t2, "result", m, "time 2");
    d.app().wire(bare, "audio", m, "audio 2");
    d.app().sync_many_ports();
    d.frames(3);
    let view = d.app().match_view(m);
    assert!(d.app().value_from(t2, "result").is_err());
    assert_eq!(view.rows[1].time_err, None);
    assert!(matches!(crate::audio_match_node::cell(&view, 1).0, crate::audio_match_node::Cell::Plain(ref s) if s.contains("no time metadata")), "{:?}", crate::audio_match_node::cell(&view, 1).0);
    assert!(view.rows[1].meta_source.contains("metadata"), "{}", view.rows[1].meta_source);
    // A length wired into the waiting port by "+ row" adds a row, and the wire moves onto it.
    d.app().wire(bare, "length", t2, corroborate::modules::time::NEXT);
    d.frames(3);
    let Kind::Time { rows } = &d.app().nodes[t2].kind else { panic!() };
    assert_eq!(corroborate::modules::time::numbers(rows), [1]);
    assert_eq!(d.app().graph.feeder(t2, "row 1"), Some((bare, "length")));
    assert_eq!(d.app().graph.feeder(t2, corroborate::modules::time::NEXT), None, "the port waits again");
    // Saved and loaded with its rows and wires.
    let text = d.app().project_text();
    let mut back = App::empty();
    assert!(back.load_project_text(&text));
    assert_eq!(back.nodes[t].kind.rows(), d.app().nodes[t].kind.rows());
    assert_eq!(back.graph.nodes()[t].spec.inputs.len(), 3);
}

/// A project saved when a Time node had one error margin: it becomes a row of zeros with that ±,
/// so the result reads the same date and ± as before.
#[test]
fn an_old_time_node_reads_the_same() {
    let mut app = App::empty();
    let typed = [("in.year", "2012"), ("in.month", "9"), ("in.day", "9"), ("in.hours", "12"), ("in.min", "20"), ("in.sec", "41"), ("1.sign", "-"), ("1.min", "2"), ("1.sec", "46"), ("1.ms", "200"), ("error_s", "1.5")];
    let t = app.add(Kind::Time { rows: typed.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }, Pos2::new(0.0, 0.0));
    let text = app.project_text();
    assert!(text.contains("error_s"), "{text}");
    let mut back = App::empty();
    assert!(back.load_project_text(&text));
    let Ok(Data::Time { utc: u, error_s, .. }) = back.value_from(t, "result") else { panic!("{:?}", back.value_from(t, "result")) };
    assert_eq!(corroborate::timebase::fmt_utc(u), corroborate::timebase::fmt_utc(corroborate::timebase::parse_iso8601("2012-09-09T12:17:54.800Z").unwrap()));
    assert!((error_s - 1.5).abs() < 1e-9, "{error_s}");
    let Kind::Time { rows } = &back.nodes[t].kind else { panic!() };
    assert_eq!(corroborate::modules::time::numbers(rows), [1, 2], "the margin is a row of its own");
    assert!(!rows.iter().any(|(k, _)| k == "error_s"));
}

/// A recording follows the real-time playhead only while its start time is wired into a Timeline;
/// with its end wired too, at the speed they give. One that starts following moves its playhead
/// over at the same moment, one that stops keeps its place; a recording only in another row's
/// media, with no time, keeps its own.
#[test]
fn one_real_time_playhead() {
    use corroborate::timebase::FileFacts;
    let mut app = App::empty();
    app.facts.put("a.mp4", FileFacts { stamp: None, len: Some(100.0), source: String::new() });
    let a = app.add(Kind::Sourcer { input: "a.mp4".into() }, Pos2::new(30.0, 40.0));
    let b = source(&mut app, 300.0);
    let (ca, cb) = (uid(&app, a), uid(&app, b));
    let time = |utc: &str| Kind::Value { ty: "time".into(), rows: vec![("utc".into(), utc.into())] };
    let (start, end) = (app.add(time("1970-01-01 00:16:40"), Pos2::new(30.0, 600.0)), app.add(time("1970-01-01 00:18:21"), Pos2::new(30.0, 800.0)));
    let tl = app.add(Kind::Timeline { rows: Vec::new() }, Pos2::new(700.0, 40.0));
    app.wire_many(a, "audio", tl);
    app.wire_many(b, "audio", tl);
    app.set_playhead(&ca, 12.0);
    assert_eq!(app.playhead_on(&ca), Some(12.0));
    assert_eq!(app.playhead_on(&cb), None, "its own: none yet");
    app.wire(start, "value", tl, "time 1");
    app.wire(end, "value", tl, "end 1");
    app.update_follow();
    // 100 s of file in 101 real seconds.
    assert!((app.ph.utc.unwrap() - 1012.12).abs() < 1e-6, "{:?}", app.ph.utc);
    assert!(app.ph.own.is_empty());
    assert!((app.playhead_on(&ca).unwrap() - 12.0).abs() < 1e-9);
    assert_eq!(app.playhead_on(&cb), None, "b has no time wired");
    app.set_utc_playhead(1050.5);
    assert!((app.playhead_on(&ca).unwrap() - 50.0).abs() < 1e-9, "{:?}", app.playhead_on(&ca));
    app.pick_only(a);
    assert!(app.is_active_clock(&ca) && !app.is_active_clock(&cb));
    app.graph.disconnect(tl, "time 1");
    app.update_follow();
    assert!(app.ph.follow.is_empty());
    assert!((app.playhead_on(&ca).unwrap() - 50.0).abs() < 1e-9, "it keeps its place on its own clock");
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
    assert_eq!(app.graph.nodes()[t].spec.inputs.len(), 12, "rows of media, start and end: three wired, one free");
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
    assert!((d.app().port_place(o, t, 3, false).0.y - y1).abs() < 0.01 && d.app().port_place(o, t, 0, false).0.y > y1, "row 2 is earlier, so it is drawn first and its ports follow");
    let track = *d.app().match_ui.tracks.last().unwrap();
    d.click_at(track.center().x, track.top() + 8.0);
    let u = d.app().ph.utc.expect("a click on the ruler sets the real-time playhead");
    assert!(u > 500.0 && u < 1100.0, "{u}");
    // The click moves the playheads of the recordings whose times are wired in, and only those.
    d.frames(1);
    let (ua, uc) = (uid(d.app(), a), uid(d.app(), c));
    let on_a = d.app().playhead_on(&ua).expect("a follows");
    assert!((on_a - (u - 1000.0)).abs() < 1e-6, "{on_a}");
    assert!(!d.app().ph.follow.contains_key(&uc), "c's time has no date: it keeps its own");
    d.shot("timeline_node_sorted");
}

/// Makes the Jiku project in the app folder: app\projects\Jiku GT_090912\project.txt, one
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
    let dir = root.join("app").join("projects").join("Jiku GT_090912");
    let file = project::project_in(&dir);
    // A fresh project: no unsaved changes of an earlier one restored over it.
    let _ = std::fs::remove_file(project::recovery_of(&file));
    app.save_project_as(dir);
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
    if let Some(json) = app.nodes[m].state.lock().unwrap().found.as_ref().map(|f| f.quality.clone()) {
        let e = json.find("\"errors\"").unwrap_or(0);
        println!("{}", &json[e..json.len().min(e + 3000)]);
    }
    let v = app.match_view(m);
    println!("{} recordings in {} synced groups", sources.len(), v.groups.iter().filter(|g| g.number > 0).count());
    d.frames(5);
    d.shot("jiku_project");
    let app = d.app();
    app.stop_all();
    assert!(st.contains("of 1225 pairs"), "all 50 matched: {st}");
}

/// Makes app\projects\Jiku GT_090912 inverted from the saved Jiku project: each video's time goes
/// through a Time node taking away the video's length, as the time in these files marks the end of
/// the recording. It first measures that against the file names, which hold the phone's clock at
/// the start, and builds nothing if most videos do not agree. Then runs it.
/// Run: cargo test --profile fast --bin corroborate make_jiku_inverted_project -- --ignored --nocapture
#[test]
#[ignore]
fn make_jiku_inverted_project() {
    use corroborate::modules::time;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let projects = root.join("app").join("projects");
    let text = std::fs::read_to_string(projects.join("Jiku GT_090912").join("project.txt")).expect("the Jiku project");
    let mut d = crate::drive::Drive::new();
    d.app().load_project_text(&text);
    d.app().resolve_recordings();
    d.app().sync_recordings();
    d.app().stop_run();
    let app = d.app();
    // Hand-made Time nodes in the original: every video gets the same one here.
    let earlier: Vec<NodeId> = (0..app.nodes.len()).filter(|&i| !app.nodes[i].hidden && matches!(app.nodes[i].kind, Kind::Time { .. })).collect();
    app.delete_nodes(&earlier);
    let m = (0..app.nodes.len()).find(|&i| matches!(app.nodes[i].kind, Kind::AudioMatch { .. })).expect("an Audio match node");
    let pairs: Vec<(usize, NodeId)> = (1..=60).filter_map(|n| app.graph.feeder(m, &format!("audio {n}")).map(|(s, _)| (n, s))).collect();
    assert_eq!(pairs.len(), 50, "the 50 videos");
    let reading = |r: Result<Data, String>| r.is_err_and(|e| e.contains("still reading"));
    let read = d.wait_until(120.0, |a| pairs.iter().all(|&(_, s)| !reading(a.value_from(s, "time")) && !reading(a.value_from(s, "length"))));
    println!("every file read: {read}");
    let app = d.app();
    let (mut at_end, mut at_start, mut neither) = (0, 0, 0);
    for &(n, s) in &pairs {
        let (Ok(Data::Time { utc: stamp, .. }), Ok(Data::Custom { json, .. })) = (app.value_from(s, "time"), app.value_from(s, "length")) else {
            println!("{:32} {n}: time {:?}, length {:?}", app.nodes[s].label, app.value_from(s, "time"), app.value_from(s, "length"));
            continue;
        };
        let len: f64 = json.trim_start_matches("{\"seconds\":").trim_end_matches('}').parse().unwrap();
        let named: f64 = app.nodes[s].label.rsplit(' ').next().unwrap().parse::<f64>().unwrap() / 1000.0;
        let (start, end) = ((stamp - named).abs(), (stamp - len - named).abs());
        println!("{:32} length {len:8.1}  stamp - name {:9.1}  stamp - length - name {:9.1}", app.nodes[s].label, stamp - named, stamp - len - named);
        match () {
            _ if end < 10.0 && end < start => at_end += 1,
            _ if start < 10.0 => at_start += 1,
            _ => neither += 1,
        }
    }
    println!("the file's time marks: the end {at_end}, the start {at_start}, neither (the phone's clock or file name off) {neither}");
    assert!(at_end > at_start, "the file's time does not mark the end: nothing built");
    for (k, &(n, s)) in pairs.iter().enumerate() {
        let mut rows = time::defaults();
        time::add_row(&mut rows);
        let t = app.add(Kind::Time { rows }, Pos2::new(0.0, 300.0 * k as f32));
        app.nodes[t].label = format!("Time: end of {}", app.nodes[s].label);
        let inputs = app.nodes[t].kind.spec().inputs;
        app.graph.set_inputs_keep(t, inputs);
        app.graph.disconnect(m, &format!("time {n}"));
        app.wire(s, "time", t, "time");
        app.wire(s, "length", t, "row 1");
        app.wire(t, "result", m, &format!("time {n}"));
    }
    d.frames(3);
    d.click("Arrange");
    d.frames(3);
    let dir = projects.join("Jiku GT_090912 inverted");
    let file = project::project_in(&dir);
    let _ = std::fs::remove_file(project::recovery_of(&file));
    d.app().save_project_as(dir);
    project::push_recent(&root.join("app").join("settings"), &file);
    println!("saved {}", file.display());
    let mut back = App::empty();
    back.open_project(file.clone());
    assert_eq!(back.nodes.iter().filter(|n| !n.hidden && matches!(n.kind, Kind::Time { .. })).count(), 50, "fifty Time nodes saved");
    assert!(!back.dirty(), "it opens with nothing unsaved");
    drop(back);
    d.shot("jiku_inverted");
    let app = d.app();
    app.run();
    let t0 = std::time::Instant::now();
    assert!(d.wait_until(1200.0, |a| a.nodes[m].state.lock().unwrap().done), "the match ends");
    let app = d.app();
    println!("{} ({:.0} s)", app.nodes[m].state.lock().unwrap().status, t0.elapsed().as_secs_f64());
    d.frames(5);
    d.shot("jiku_inverted_run");
    d.app().stop_all();
}

/// Makes app\projects\Hillingdon meetings: the two Hillingdon recordings in app\recordings, each
/// dated by its mains hum with nothing but the place wired in, and shown on a Timeline. Then runs
/// it. Needs the grid logs: set CORROBORATE_GRID_LOGS to app\grid logs.
/// Run: cargo test --profile fast --bin corroborate make_hillingdon_project -- --ignored --nocapture
#[test]
#[ignore]
fn make_hillingdon_project() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let recordings = root.join("app").join("recordings");
    let meetings = [("youtube qJ4EtDtDAr0.mp4", "Cabinet, 22 October 2026"), ("youtube pRX5rSk_j_M.mp4", "Council, 7 July 2016")];
    let mut d = crate::drive::Drive::new();
    let app = d.app();
    // Uxbridge Civic Centre, where they meet.
    let place = app.add(crate::drive::place(51.5447, -0.4787, 10.0), Pos2::new(40.0, 40.0));
    app.nodes[place].label = "Hillingdon".into();
    let tl = app.add(Kind::Timeline { rows: Vec::new() }, Pos2::new(1200.0, 40.0));
    let mut enfs = Vec::new();
    for (k, (file, title)) in meetings.iter().enumerate() {
        let f = recordings.join(file);
        assert!(f.is_file(), "{}", f.display());
        let s = app.add(Kind::Sourcer { input: f.to_string_lossy().into_owned() }, Pos2::new(40.0, 300.0 + 400.0 * k as f32));
        app.nodes[s].label = title.to_string();
        let e = app.add(Kind::Enf { grid: String::new() }, Pos2::new(600.0, 300.0 + 400.0 * k as f32));
        app.wire(s, "audio", e, "audio");
        app.wire(place, "value", e, "location");
        app.wire_many(s, "audio", tl);
        app.wire(e, "audio start time", tl, &format!("time {}", k + 1));
        enfs.push((*title, e));
    }
    assert_eq!(app.graph.wires().len(), 8, "{}", app.msg);
    d.frames(3);
    d.click("Arrange");
    d.frames(3);
    let dir = root.join("app").join("projects").join("Hillingdon meetings");
    let file = project::project_in(&dir);
    let _ = std::fs::remove_file(project::recovery_of(&file));
    d.app().save_project_as(dir);
    project::push_recent(&root.join("app").join("settings"), &file);
    println!("saved {}", file.display());
    d.shot("hillingdon_project");
    let app = d.app();
    app.run();
    let t0 = std::time::Instant::now();
    let ended = d.wait_until(1800.0, |a| enfs.iter().all(|&(_, e)| a.nodes[e].state.lock().unwrap().done));
    println!("ended: {ended} ({:.0} s)", t0.elapsed().as_secs_f64());
    let app = d.app();
    for &(title, e) in &enfs {
        let status = app.nodes[e].state.lock().unwrap().status.clone();
        println!("{title}: {status} -> {:?}", app.value_from(e, "audio start time"));
    }
    d.frames(5);
    d.shot("hillingdon_run");
    d.app().stop_all();
    assert!(ended, "both files ended");
}

/// The time written in a file is a claim only, wired or not; a live source's own time is on its
/// time output too. The file's time is on the output from before any Run, raw, with an error of 0, and a Run
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
    assert_eq!(app.value_from(s, "time"), Ok(Data::Time { utc: 1000.0, error_s: 0.0, source: "the file's creation_time".into(), recording: c }));
    assert_eq!(app.value_from(s, "length"), Ok(Data::Custom { kind: "length", json: "{\"seconds\":60}".into() }));
    app.wire(s, "time", p, "time");
    assert_eq!(app.graph.wires().len(), 2, "{}", app.msg);
    app.update_follow();
    assert!(app.ph.follow.is_empty(), "wired into Planes: still only a claim");
    app.nodes[s].state.lock().unwrap().reset();
    assert!(app.value_from(s, "time").is_ok(), "a Run clears the state, not what the file says");
    // Not a file: its own time is on its time output.
    let live = source(&mut app, 300.0);
    anchor(&app, live, 2000.0, 1.0);
    let Ok(Data::Time { utc, error_s, .. }) = app.value_from(live, "time") else { panic!("no time from the live source") };
    assert_eq!((utc, error_s), (2000.0, 1.0));
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
    let made = root.join("app").join("projects").join("Jiku GT_090912").join("project.txt");
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
    let v = app.match_view(m);
    let group_of = |s: NodeId| v.rows.iter().find(|r| r.uid.as_deref() == Some(app.nodes[s].uid.as_str())).map(|r| r.group);
    let key = videos.iter().position(|v| v.to_string_lossy().contains("GT_090912_0_1347196530871")).expect("the video");
    let group = group_of(sources[key]);
    let members: Vec<PathBuf> = (0..50).filter(|&k| group_of(sources[k]) == group).map(|k| videos[k].clone()).collect();
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
    let dir = root.join("app").join("projects").join("Jiku group 6");
    let file = project::project_in(&dir);
    let _ = std::fs::remove_file(project::recovery_of(&file));
    app.save_project_as(dir);
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
    let made = root.join("app").join("projects").join("Jiku group 6").join("project.txt");
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
    d.frames(3);
    let v = d.app().match_view(m);
    for (k, x) in v.rows.iter().enumerate().filter(|(_, r)| r.wired) {
        println!("{:28} {:?} meta {:?} start {:?} hint {}", x.name, cell(&v, k).0, x.meta.map(|b| b as i64), x.start.map(|b| b as i64), x.hint);
    }
    assert_eq!(v.rows.iter().filter(|x| x.start.is_some()).count(), 12, "every row has its audio position");
    assert_eq!(v.groups.iter().filter(|g| g.number > 0).count(), 1, "one synced group");
    d.shot("am_3_after_the_match");
    let Ok(Data::Time { source, .. }) = d.app().value_from(m, "synced 1") else { panic!("unanchored: a time all the same") };
    assert!(source.contains("unanchored"), "{source}");

    // A drag on an amber bar moves the group (saved in the node) and changes no evidence.
    let track = d.app().match_ui.tracks[0];
    let k = v.rows.iter().position(|r| r.meta.is_some()).expect("a row with a time");
    let on = d.app().match_ui.anchor_at[&(m, k)];
    let links_of = |app: &App| app.nodes[m].state.lock().unwrap().found.as_ref().map_or(0, |f| f.links.len());
    let (t0, links0) = (v.rows[k].start.unwrap(), links_of(d.app()));
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
    assert_eq!(links_of(d.app()), links0, "and no data did");
    d.shot("am_4_group_dragged");

    // A click on a row's anchor: the group has real time and the yellow outputs give it.
    d.click_at(on.center().x, on.center().y);
    d.frames(6);
    let v = d.app().match_view(m);
    assert_eq!(cell(&v, k).0, Cell::Anchor, "{}", d.msg());
    assert!((v.rows[k].start.unwrap() - v.rows[k].meta.unwrap()).abs() < 0.01, "the audio's start snaps to the time");
    for r in v.rows.iter().filter(|r| r.wired) {
        assert!(d.app().value_from(m, &format!("synced {}", r.n)).is_ok(), "row {} gives a time", r.n);
    }
    for (k, x) in v.rows.iter().enumerate().filter(|(_, r)| r.wired) {
        println!("{:28} {:?} {:?}", x.name, cell(&v, k).0, crate::audio_match_node::drift_cell(&v, k));
    }
    // The node's own fit of its group against the clock solve of all its links at once (what the
    // global solve did), anchored on the same time: each row's start from the anchor's, in real
    // seconds. Both count the group's average clock as running true.
    let a = v.groups.iter().find_map(|g| g.anchor).expect("anchored");
    let meta_a = v.rows[a].meta.unwrap();
    let links = d.app().nodes[m].state.lock().unwrap().found.as_ref().unwrap().links.clone();
    let ca = v.rows[a].clock.clone().unwrap();
    let solved = corroborate::clock::solve(&[corroborate::clock::Anchor { clock: ca.clone(), t: 0.0, utc: meta_a, error_s: 1.0, from: "test".into(), enabled: true }], &links);
    let ga = solved.get(&ca);
    let mut worst = 0.0f64;
    for r in v.rows.iter().filter(|r| r.wired) {
        let gr = solved.get(r.clock.as_deref().unwrap());
        let (own, global) = (r.start.unwrap() - meta_a, gr.at(0.0) - ga.at(0.0));
        println!("{:28} {:?} own fit {own:+.4} s, global {global:+.4} s from the anchor's start: {:+.3} ms", r.name, crate::audio_match_node::drift_cell(&v, v.rows.iter().position(|x| x.n == r.n).unwrap()), (own - global) * 1000.0);
        worst = worst.max((own - global).abs());
    }
    println!("per-group fit against the global solve: worst {:.3} ms, median leftover {:.2} ms", worst * 1000.0, v.leftover.unwrap_or(f64::NAN) * 1000.0);
    assert!(worst < 0.001, "the per-group fit agrees with the global solve to within 1 ms: {:.3} ms", worst * 1000.0);
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


