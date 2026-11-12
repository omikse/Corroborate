//! The Cut detector node (src/modules/Cut detector): a clip with known cuts read for real, its
//! pieces as outputs, switches and hand cuts, saved with the project.
//! Run: cargo test --profile fast --bin corroborate cut_tests -- --nocapture

use super::*;
use crate::drive::{out_dir, Drive};
use corroborate::data::Data;

/// Four shots of 3 s at 25 frames a second, cut at 3, 6 and 9 s, a two-frame white flash at
/// 4.48 s, a 440 Hz tone all through, and a creation time in its metadata.
pub fn four_shots() -> PathBuf {
    let p = out_dir().join("cut_four_shots.mp4");
    if !p.exists() {
        let shot = |src: &str, name: &str| format!("{src},trim=duration=3,setpts=PTS-STARTPTS,format=yuv420p,scale=320:240,setsar=1[{name}]");
        let graph = format!(
            "{};{};{};{};[a][b][c][d]concat=n=4:v=1:a=0,drawbox=color=white:t=fill:enable='between(n,112,113)'[v]",
            shot("testsrc2=size=320x240:rate=25", "a"),
            shot("mandelbrot=size=320x240:rate=25", "b"),
            shot("smptebars=size=320x240:rate=25", "c"),
            shot("life=size=320x240:rate=25:mold=10:ratio=0.3:death_color=#c83232:life_color=#00ff00", "d"),
        );
        let ok = corroborate::no_window(&mut std::process::Command::new("ffmpeg"))
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=f=440:d=12", "-filter_complex", &graph, "-map", "[v]", "-map", "0:a"])
            .args(["-c:v", "libx264", "-g", "50", "-c:a", "aac", "-t", "12", "-metadata", "creation_time=2026-11-12T01:00:00Z"])
            .arg(&p)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "ffmpeg made the clip");
    }
    p
}

fn utc_of(d: Result<Data, String>) -> (f64, f64, String) {
    match d {
        Ok(Data::Time { utc: Some(u), error_s, clock, .. }) => (u, error_s, clock),
        other => panic!("no time: {other:?}"),
    }
}

/// The node reads the clip, finds the three cuts on their frames (the flash stays off), and gives
/// four pieces: each with its time as played and its length, on clocks of their own after the
/// first. A click's worth of switches: a cut off makes three pieces, a hand cut five; both are
/// saved with the project and come back with their outputs and wires.
#[test]
fn a_clip_falls_into_its_pieces() {
    let clip = four_shots();
    let mut d = Drive::new();
    let app = d.app();
    let s = app.add(Kind::Sourcer { input: clip.to_string_lossy().into_owned() }, Pos2::new(20.0, 40.0));
    let c = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(300.0, 40.0));
    app.wire(s, "video", c, "video");
    app.wire(s, "audio", c, "audio");
    app.wire(s, "time", c, "time");
    app.run();
    assert!(d.wait_until(300.0, |a| a.nodes[c].state.lock().unwrap().cut.is_some()), "the clip was read: {}", d.app().nodes[c].state.lock().unwrap().status);
    d.wait_until(30.0, |a| a.source_length(s).is_some());
    d.frames(3);
    let app = d.app();
    let v = app.cut_view(c);
    let on: Vec<f64> = v.cuts.iter().filter(|x| x.on()).map(|x| x.t_pic).collect();
    println!("cuts {:?}", v.cuts.iter().map(|x| (x.t_pic, x.on(), x.why.clone())).collect::<Vec<_>>());
    assert_eq!(on.len(), 3, "{on:?}");
    for (t, want) in on.iter().zip([3.0, 6.0, 9.0]) {
        assert!((t - want).abs() < 0.021, "{t} {want}");
    }
    assert_eq!(v.pieces.len(), 4);
    let outs: Vec<&str> = app.graph.nodes()[c].spec.outputs.iter().map(|p| p.name).collect();
    assert_eq!(outs.len(), 1 + 4 * 4, "{outs:?}");
    // Each piece's time: the file's start plus where the piece sits, with the file's error.
    let (t0, e0, k0) = utc_of(app.value_from(c, "time 1"));
    let (t2, _, k2) = utc_of(app.value_from(c, "time 2"));
    assert!((t2 - t0 - 3.0).abs() < 0.03, "{t0} {t2}");
    assert_eq!(e0, 0.0);
    assert_eq!(k0, app.clock_of(s).unwrap(), "the first piece is on the file's clock");
    assert_ne!(k2, k0, "a later piece has a clock of its own");
    match app.value_from(c, "length 2") {
        Ok(Data::Custom { json, .. }) => assert!(json.contains("\"seconds\":3") || json.contains("\"seconds\":2.9"), "{json}"),
        other => panic!("{other:?}"),
    }
    match app.value_from(c, "cuts") {
        Ok(Data::Events(e)) => {
            assert_eq!(e.len(), 3);
            assert!(e[0].detail.contains("picture"), "{}", e[0].detail);
        }
        other => panic!("{other:?}"),
    }
    d.shot("cut_detector");

    // A wire from the fourth piece, the second cut switched off: three pieces, the wire on the
    // fourth piece's output goes with it.
    let app = d.app();
    let tl = app.add(Kind::Timeline { rows: Vec::new() }, Pos2::new(1400.0, 40.0));
    app.wire_many(c, "audio 3", tl);
    let second = app.cut_view(c).cuts.iter().find(|x| (x.t_pic - 6.0).abs() < 0.1).cloned().unwrap();
    app.toggle_cut(c, &second);
    d.frames(3);
    let app = d.app();
    assert_eq!(app.cut_view(c).pieces.len(), 3);
    assert_eq!(app.graph.nodes()[c].spec.outputs.len(), 1 + 3 * 4);
    // And a hand cut at 7.5 s: four again.
    app.hand_cut(c, 7.5);
    d.frames(3);
    let app = d.app();
    let v = app.cut_view(c);
    assert_eq!(v.pieces.len(), 4);
    assert!(v.cuts.iter().any(|x| x.hand() && (x.t_pic - 7.5).abs() < 1e-9));
    assert!(app.changes.iter().any(|ch| ch.name.contains("cut at")), "switches go into the result file");
    // Saved and opened again: the switches, the outputs and the wire are back before any run.
    let text = app.project_text();
    let mut back = App::empty();
    assert!(back.load_project_text(&text));
    let c2 = back.nodes.iter().position(|n| matches!(n.kind, Kind::CutDetector { .. })).unwrap();
    assert_eq!(back.graph.nodes()[c2].spec.outputs.len(), 1 + 4 * 4);
    let rows = back.nodes[c2].kind.rows().to_vec();
    assert_eq!(corroborate::modules::cut_detector::switches(&rows).len(), 2, "{rows:?}");
    assert!(back.graph.wires().iter().any(|w| w.from == c2), "the wire from a piece came back");
    let json = cut_detector_node::json(d.app(), c);
    assert!(json.contains("\"pieces\":[") && json.contains("by_lanes"), "{json}");
}

/// A minute of a wandering tone over noise, at one loudness (an "original"), and a copy with 25 to 28 s taken out
/// (a jump cut), both WAV.
fn jump_cut_pair() -> (PathBuf, PathBuf) {
    let (orig, cut) = (out_dir().join("cut_orig.wav"), out_dir().join("cut_jump.wav"));
    let ff = |args: &[&str]| assert!(corroborate::no_window(&mut std::process::Command::new("ffmpeg")).args(["-v", "error", "-y"]).args(args).status().is_ok_and(|s| s.success()), "{args:?}");
    if !orig.exists() {
        let expr = "0.3*sin(2*PI*(500*t+80*sin(t*1.3)+40*sin(t*3.7)))+0.05*(random(0)-0.5)";
        ff(&["-f", "lavfi", "-i", &format!("aevalsrc={expr}:s=16000:d=60"), orig.to_str().unwrap()]);
    }
    if !cut.exists() {
        ff(&["-i", orig.to_str().unwrap(), "-filter_complex", "[0:a]atrim=0:25,asetpts=PTS-STARTPTS[a];[0:a]atrim=28:60,asetpts=PTS-STARTPTS[b];[a][b]concat=n=2:v=0:a=1[o]", "-map", "[o]", cut.to_str().unwrap()]);
    }
    (orig, cut)
}

/// Stage 2 in the node: a sound with 3 s cut out, compared with the original: the compare lane
/// finds the cut on its own (no picture), the sound lanes run, the pieces follow.
#[test]
fn a_jump_cut_is_found_by_comparing() {
    let (orig, cut) = jump_cut_pair();
    let mut d = Drive::new();
    let app = d.app();
    let s = app.add(Kind::Sourcer { input: cut.to_string_lossy().into_owned() }, Pos2::new(20.0, 40.0));
    let o = app.add(Kind::Sourcer { input: orig.to_string_lossy().into_owned() }, Pos2::new(20.0, 500.0));
    let c = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(300.0, 40.0));
    app.wire(s, "audio", c, "audio");
    app.wire_many(o, "audio", c);
    let inputs: Vec<&str> = app.graph.nodes()[c].spec.inputs.iter().map(|p| p.name).collect();
    assert!(app.graph.feeder(c, "compare with 1").is_some(), "{inputs:?}");
    app.run();
    assert!(d.wait_until(300.0, |a| a.nodes[c].state.lock().unwrap().cut.is_some()), "read: {}", d.app().nodes[c].state.lock().unwrap().status);
    d.frames(3);
    let app = d.app();
    let v = app.cut_view(c);
    let keys: Vec<&str> = v.lanes.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(keys, ["level", "noise", "compare 1"]);
    println!("{}", v.lanes[2].note);
    for l in &v.lanes {
        println!("{}: {:?}", l.name, l.peaks.iter().map(|p| (p.t, p.value)).collect::<Vec<_>>());
    }
    let on: Vec<&corroborate::cuts::Cut> = v.cuts.iter().filter(|x| x.on()).collect();
    assert_eq!(on.len(), 1, "{:?}", v.cuts);
    assert!((on[0].t_snd - 25.0).abs() <= 0.1, "{}", on[0].t_snd);
    assert_eq!(v.pieces.len(), 2);
    assert!((v.pieces[1].a0 - 25.0).abs() <= 0.1);
    d.shot("cut_detector_compare");
}
