//! The Cut detector node (src/modules/Cut detector): a clip with known cuts read for real, its
//! pieces as outputs, switches and hand cuts, saved with the project.
//! Run: cargo test --profile fast --bin corroborate cut_tests -- --nocapture

use super::*;
use crate::drive::{out_dir, Drive};
use corroborate::data::Data;
use std::path::Path;

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

/// A long recording's filmstrip fits one texture: a picture a second of 44 minutes (2645
/// pictures, 127 000 pixels in one row, which the graphics card refuses) goes in rows; past what
/// rows hold, every so many pictures is kept.
#[test]
fn a_long_filmstrip_fits_a_texture() {
    use corroborate::cuts::{THUMB_H, THUMB_W};
    for (n, side) in [(2645, 8192), (2645, 16384), (1, 8192), (60_000, 8192), (500_000, 2048)] {
        let (cols, rows, step) = cut_detector_node::strip_layout(n, side);
        assert!(cols * THUMB_W <= side && rows * THUMB_H <= side, "{n} pictures in {cols} x {rows} fit {side}");
        assert!(cols * rows >= n.div_ceil(step), "{n}: every kept picture has a place");
        if n <= (side / THUMB_W) * (side / THUMB_H) {
            assert_eq!(step, 1, "{n} pictures all kept in {side}");
        }
    }
}

/// The player: a cut plays from 2 s before it to 2 s after, picture and sound, and pauses there.
/// The pictures it reads are on the same times as the run's (the cut's frame is among them), and
/// the run's steps all end done, with how long it took.
#[test]
fn the_player_plays_a_cut() {
    let clip = four_shots();
    let mut d = Drive::new();
    let app = d.app();
    let s = app.add(Kind::Sourcer { input: clip.to_string_lossy().into_owned() }, Pos2::new(20.0, 40.0));
    let c = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(300.0, 40.0));
    app.wire(s, "video", c, "video");
    app.wire(s, "audio", c, "audio");
    app.run();
    assert!(d.wait_until(300.0, |a| a.nodes[c].state.lock().unwrap().cut.is_some()), "read");
    let prog = d.app().nodes[c].state.lock().unwrap().progress.clone();
    println!("steps {:?}", prog.steps.iter().map(|s| (s.name.clone(), s.state)).collect::<Vec<_>>());
    assert!(prog.steps.len() >= 4 && prog.steps.iter().all(|s| s.state == cut_detector_node::StepState::Done), "{prog:?}");
    assert!(prog.took.is_some());
    d.wait_until(30.0, |a| a.source_length(s).is_some());
    d.frames(3);
    let app = d.app();
    let v = app.cut_view(c);
    let k = v.cuts.iter().position(|x| x.on()).expect("a cut");
    let t_cut = v.cuts[k].t_pic;
    app.play_cut(c, k);
    let clock = app.clock_of(c).unwrap();
    assert_eq!(app.selected, Some(c));
    assert!(app.ph.playing);
    assert!((app.playhead_on(&clock).unwrap() - (t_cut - 2.0)).abs() < 1e-9);
    assert!(d.wait_until(60.0, |a| a.cut_player_times(c).iter().any(|t| (t - t_cut).abs() < 0.001)), "the cut's frame read: {:?}", d.app().cut_player_times(c));
    d.shot("cut_detector_player");
    assert!(d.wait_until(30.0, |a| !a.ph.playing), "the play pauses at its end");
    let end = d.app().playhead_on(&clock).unwrap();
    assert!(end >= t_cut + 2.0 - 1e-6 && end < t_cut + 2.5, "paused at {end}");
    d.shot("cut_detector_player_end");
}

/// A YouTube link in the SOURCER: the detector reads the copy the SOURCER downloaded into the
/// project's recordings\ (listed in its downloads.txt); with none, it cannot start (the queue runs
/// the SOURCER first).
/// A copy left in recordings\ next to the program is copied into the project first.
#[test]
fn a_downloaded_video_is_read() {
    let clip = four_shots();
    let dir = out_dir().join("cut_url_project");
    let _ = std::fs::remove_dir_all(&dir);
    let rec = dir.join("recordings");
    std::fs::create_dir_all(&rec).unwrap();
    let url = "https://www.youtube.com/watch?v=cuttest0001";
    let mut d = Drive::new();
    let app = d.app();
    app.project_path = Some(dir.join("project.txt"));
    assert_eq!(recordings::key(&app.downloads_dir()), recordings::key(&rec));
    let s = app.add(Kind::Sourcer { input: url.into() }, Pos2::new(20.0, 40.0));
    let c = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(300.0, 40.0));
    app.wire(s, "video", c, "video");
    assert_eq!(app.needs(c), vec![s], "it needs its SOURCER's download");
    let e = app.cut_job(c).err().unwrap_or_default();
    assert!(e.contains("kept no copy"), "no copy yet: {e}");
    // The copy listed by its name in the project's list.
    std::fs::copy(&clip, rec.join("youtube cuttest0001.mp4")).unwrap();
    std::fs::write(rec.join("downloads.txt"), format!("{url}\tyoutube cuttest0001.mp4\n")).unwrap();
    let job = app.cut_job(c).unwrap();
    assert!(recordings::key(Path::new(&job.path)) == recordings::key(&rec.join("youtube cuttest0001.mp4")), "{}", job.path);
    // One in recordings\ next to the program only: copied into the project.
    let other = "https://youtu.be/cuttest0002";
    let shared = corroborate::home().join("recordings");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::copy(&clip, shared.join("youtube cuttest0002.mp4")).unwrap();
    std::fs::write(shared.join("downloads.txt"), format!("{other}\t{}\n", shared.join("youtube cuttest0002.mp4").display())).unwrap();
    let p = sourcer_node::downloaded("https://www.youtube.com/watch?v=cuttest0002", &rec).expect("found next to the program");
    assert_eq!(recordings::key(Path::new(&p)), recordings::key(&rec.join("youtube cuttest0002.mp4")));
    assert!(std::fs::read_to_string(rec.join("downloads.txt")).unwrap().contains("cuttest0002\tyoutube cuttest0002.mp4"));
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

/// Stage 3: the original's 60 s cut into three and joined as 2 1 3 (a shuffled copy). The Cut
/// detector, comparing the copy with the original, finds the two joins; its three pieces go into an
/// Audio match with the original, which places each where it was (20, 0 and 40 s into the
/// original), as one phone; the placed strip reads their true order back.
#[test]
fn shuffled_pieces_go_back_in_order() {
    let (orig, _) = jump_cut_pair();
    let shuffled = out_dir().join("cut_shuffled.wav");
    if !shuffled.exists() {
        let ok = corroborate::no_window(&mut std::process::Command::new("ffmpeg"))
            .args(["-v", "error", "-y", "-i", orig.to_str().unwrap(), "-filter_complex"])
            .arg("[0:a]atrim=20:40,asetpts=PTS-STARTPTS[a];[0:a]atrim=0:20,asetpts=PTS-STARTPTS[b];[0:a]atrim=40:60,asetpts=PTS-STARTPTS[c];[a][b][c]concat=n=3:v=0:a=1[o]")
            .args(["-map", "[o]"])
            .arg(&shuffled)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok);
    }
    let mut d = Drive::new();
    let app = d.app();
    let s = app.add(Kind::Sourcer { input: shuffled.to_string_lossy().into_owned() }, Pos2::new(20.0, 40.0));
    let o = app.add(Kind::Sourcer { input: orig.to_string_lossy().into_owned() }, Pos2::new(20.0, 500.0));
    // Three pieces expected: their outputs are there before the run (as in a saved project).
    let c = app.add(Kind::CutDetector { rows: vec![("pieces".into(), "3".into())] }, Pos2::new(300.0, 40.0));
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(1400.0, 40.0));
    app.wire(s, "audio", c, "audio");
    app.wire_many(o, "audio", c);
    app.wire_many(o, "audio", m);
    for n in 1..=3 {
        app.wire_many(c, &format!("audio {n}"), m);
    }
    app.run();
    assert!(d.wait_until(300.0, |a| a.nodes[m].state.lock().unwrap().found.is_some()), "matched: {}", d.app().nodes[m].state.lock().unwrap().status);
    d.frames(3);
    let app = d.app();
    let v = app.cut_view(c);
    let on: Vec<f64> = v.cuts.iter().filter(|x| x.on()).map(|x| x.t_snd).collect();
    println!("cuts {on:?}");
    for l in &v.lanes {
        println!("{}: {:?}", l.name, l.peaks.iter().map(|p| (p.t, p.value, p.error_s, p.note.clone())).collect::<Vec<_>>());
    }
    assert_eq!(on.len(), 2, "{on:?}");
    assert!((on[0] - 20.0).abs() <= 0.1 && (on[1] - 40.0).abs() <= 0.1, "{on:?}");
    let mv = app.match_view(m);
    for r in &mv.rows {
        println!("row {}: {} start {:?} phone {:?} group {}", r.n, r.name, r.start, r.phone, r.group);
    }
    let start = |n: usize| mv.rows.iter().find(|r| r.n == n).and_then(|r| r.start).unwrap_or(f64::NAN);
    let o0 = start(1);
    for (row, want) in [(2, 20.0), (3, 0.0), (4, 40.0)] {
        let got = start(row) - o0;
        assert!((got - want).abs() <= 0.1, "row {row} at {got:.3}, want {want}");
    }
    let phones: Vec<&str> = mv.rows.iter().filter(|r| r.name.contains("piece")).map(|r| r.phone.as_str()).collect();
    assert!(phones.len() == 3 && phones.iter().all(|p| *p == phones[0] && !p.is_empty()), "{phones:?}");
    let placed: Vec<usize> = app.placed(c).iter().map(|p| p.n).collect();
    assert_eq!(placed, [2, 1, 3]);
    d.shot("cut_detector_placed");
}

/// The plan's stage 3 test on the Jiku Mobile Video Dataset (GT_090912; fetch it first:
/// `python tools/jiku.py GT_090912`): phone 4's recording 1347196751640, 190 to 370 s, cut into
/// four pieces of 45 s and joined as 3 1 4 2, read by a Cut detector comparing it with the three
/// phones that heard all of it; its four pieces wired into an Audio match with those phones. Each
/// piece must land where the ground truth says it was (against the first phone), and the placed
/// strip must read their true order.
///     cargo test --profile fast --bin corroborate cut_tests::jiku -- --ignored --nocapture
#[test]
#[ignore]
fn jiku_shuffled_pieces_land_in_audio_match() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join("jiku").join("GT_090912");
    let xml = std::fs::read_to_string(dir.join("GT_090912.xml")).expect("fetch the Jiku data first");
    let truth = |name: &str| -> (f64, f64) {
        let l = xml.lines().find(|l| l.contains(&format!("name=\"{name}\""))).unwrap();
        let attr = |k: &str| l.split(&format!("{k}=\"")).nth(1).unwrap().split('"').next().unwrap().to_string();
        let f: Vec<f64> = attr("offset").split(':').map(|x| x.parse().unwrap()).collect();
        (f[0] * 86400.0 + f[1] * 3600.0 + f[2] * 60.0 + f[3], attr("speed").parse().unwrap())
    };
    let ours = "GT_090912_4_1347196751640";
    let phones = ["GT_090912_2_1347196671407", "GT_090912_3_1347196795193", "GT_090912_0_1347196688274"];
    let (lo, piece, order) = (190.0, 45.0, [2usize, 0, 3, 1]);
    let joined = out_dir().join("jiku_shuffled.wav");
    if !joined.exists() {
        let (mut child, rx) = corroborate::ingest::spawn_ffmpeg(dir.join(format!("{ours}.m4a")).to_str().unwrap(), 16000).unwrap();
        let audio: Vec<f32> = rx.into_iter().flatten().collect();
        let _ = child.wait();
        let mut w = hound::WavWriter::create(&joined, hound::WavSpec { channels: 1, sample_rate: 16000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
        for &k in &order {
            let (a, b) = (((lo + k as f64 * piece) * 16000.0) as usize, ((lo + (k + 1) as f64 * piece) * 16000.0) as usize);
            for &v in &audio[a..b] {
                w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
            }
        }
        w.finalize().unwrap();
    }
    let mut d = Drive::new();
    let app = d.app();
    let s = app.add(Kind::Sourcer { input: joined.to_string_lossy().into_owned() }, Pos2::new(20.0, 40.0));
    let c = app.add(Kind::CutDetector { rows: vec![("pieces".into(), "4".into())] }, Pos2::new(300.0, 40.0));
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(1400.0, 40.0));
    app.wire(s, "audio", c, "audio");
    let mut ph = Vec::new();
    for (k, p) in phones.iter().enumerate() {
        let n = app.add(Kind::Sourcer { input: dir.join(format!("{p}.m4a")).to_string_lossy().into_owned() }, Pos2::new(20.0, 500.0 + 300.0 * k as f32));
        app.wire_many(n, "audio", c);
        app.wire_many(n, "audio", m);
        ph.push(n);
    }
    for n in 1..=4 {
        app.wire_many(c, &format!("audio {n}"), m);
    }
    app.run();
    assert!(d.wait_until(900.0, |a| a.nodes[m].state.lock().unwrap().found.is_some()), "matched: {}", d.app().nodes[m].state.lock().unwrap().status);
    d.frames(3);
    let app = d.app();
    let v = app.cut_view(c);
    let on: Vec<(f64, f64)> = v.cuts.iter().filter(|x| x.on()).map(|x| (x.t_snd, x.error_s)).collect();
    println!("cuts (time, error) {on:?}; true 45, 90, 135");
    assert_eq!(on.len(), 3);
    for (&(t, e), want) in on.iter().zip([45.0, 90.0, 135.0]) {
        assert!((t - want).abs() <= (3.0 * e).max(0.1), "{t} ± {e}, true {want}");
    }
    let mv = app.match_view(m);
    let row_of = |name: &str| mv.rows.iter().find(|r| r.name.contains(name)).unwrap();
    let (o_ours, sp) = truth(ours);
    let (o_ref, sp_ref) = truth(phones[0]);
    let reference = row_of(phones[0]).start.unwrap();
    for (k, p) in v.pieces.iter().enumerate() {
        let r = row_of(&format!("piece {}", p.n));
        let join = [0.0, 45.0, 90.0, 135.0][k];
        // Where the piece found starts in the recording: its true start, moved as far as its
        // join was found off.
        let true_start = lo + order[k] as f64 * piece + (p.a0 - join);
        // On the reference phone's clock (Audio match's times are clock times; the ground truth's
        // timeline runs 266 ppm apart from these phones').
        let want = (o_ours + true_start * sp - o_ref) / sp_ref;
        let got = r.start.unwrap() - reference;
        println!("piece {}: {:.3} s after {} (truth {:.3}), off {:.1} ms; phone {:?}", p.n, got, phones[0], want, (got - want) * 1e3, r.phone);
        assert!((got - want).abs() <= 0.05, "piece {} off by {:.3} s", p.n, got - want);
    }
    let placed: Vec<usize> = app.placed(c).iter().map(|p| p.n).collect();
    assert_eq!(placed, [2, 4, 1, 3]);
    d.shot("cut_detector_jiku");
}

/// A 44 minute YouTube video (the copy in app\recordings, if it is there): read to the end
/// without a crash (its filmstrip of 2645 pictures goes in rows), the progress row while it runs,
/// and the player on its first cut.
/// Run: cargo test --profile fast --bin corroborate cut_tests::a_long_video -- --ignored --nocapture
#[test]
#[ignore = "needs the video youtube 5wypCQp11wo.mp4 in app recordings"]
fn a_long_video() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("app").join("recordings").join("youtube 5wypCQp11wo.mp4");
    assert!(file.is_file(), "{}", file.display());
    let mut d = Drive::new();
    let app = d.app();
    let s = app.add(Kind::Sourcer { input: file.to_string_lossy().into_owned() }, Pos2::new(20.0, 40.0));
    let c = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(300.0, 40.0));
    app.wire(s, "video", c, "video");
    app.wire(s, "audio", c, "audio");
    app.run();
    let t0 = Instant::now();
    d.wait_until(20.0, |a| a.nodes[c].state.lock().unwrap().progress.steps.iter().any(|s| s.frac > 0.05));
    d.shot("cut_long_running");
    assert!(d.wait_until(1800.0, |a| a.nodes[c].state.lock().unwrap().cut.is_some()), "read: {}", d.app().nodes[c].state.lock().unwrap().status);
    println!("read in {:.0} s", t0.elapsed().as_secs_f64());
    d.wait_until(120.0, |a| a.source_length(s).is_some());
    d.frames(3);
    let v = d.app().cut_view(c);
    println!("{} cuts, {} on, {} thumbs", v.cuts.len(), v.cuts.iter().filter(|x| x.on()).count(), v.scan.as_ref().unwrap().thumbs.len());
    for x in v.cuts.iter().take(12) {
        println!("  {:8.2} {} {}", x.t_pic, if x.on() { "on " } else { "off" }, x.why);
    }
    d.shot("cut_long_done");
    if let Some(k) = v.cuts.iter().position(|x| x.on()) {
        d.app().play_cut(c, k);
        d.wait_until(60.0, |a| !a.cut_player_times(c).is_empty());
        d.frames(3);
        d.shot("cut_long_player");
    }
}
