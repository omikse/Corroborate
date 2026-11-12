//! The cut detector's lanes on clips made with ffmpeg, with cuts at known frames.
//!
//!     cargo test --profile fast --test cuts
//!
//! The clip: four shots of 3 s at 25 frames a second (ffmpeg's test pictures testsrc2,
//! mandelbrot, smptebars and life), cut at 3, 6 and 9 s, with a white flash of two frames at
//! 4.5 s inside the second shot. Encoded with x264, a keyframe every 2 s and at scene changes.

use corroborate::cuts::{self, Switch};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("cuts");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// The four-shot clip (made once).
pub fn four_shots() -> PathBuf {
    let p = out_dir().join("four_shots.mp4");
    if !p.exists() {
        let shot = |src: &str| format!("{src},trim=duration=3,setpts=PTS-STARTPTS,format=yuv420p,scale=320:240,setsar=1");
        let graph = format!(
            "{};{};{};{};[a][b][c][d]concat=n=4:v=1:a=0,drawbox=color=white:t=fill:enable='between(n,112,113)'[v]",
            shot("testsrc2=size=320x240:rate=25") + "[a]",
            shot("mandelbrot=size=320x240:rate=25") + "[b]",
            shot("smptebars=size=320x240:rate=25") + "[c]",
            shot("life=size=320x240:rate=25:mold=10:ratio=0.3:death_color=#c83232:life_color=#00ff00") + "[d]",
        );
        let ok = corroborate::no_window(&mut std::process::Command::new("ffmpeg"))
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=f=440:d=12", "-filter_complex", &graph, "-map", "[v]", "-map", "0:a"])
            .args(["-c:v", "libx264", "-g", "50", "-c:a", "aac", "-t", "12"])
            .arg(&p)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "ffmpeg made the clip");
    }
    p
}

#[test]
fn hard_cuts_are_found_on_their_frame_and_the_flash_starts_off() {
    let p = four_shots();
    let stop = AtomicBool::new(false);
    let pic = cuts::scan_picture(p.to_str().unwrap(), &stop, &|_| {}).unwrap();
    assert_eq!(pic.times.len(), 300, "every frame read");
    assert!((pic.frame_s - 0.04).abs() < 1e-6);
    assert!(pic.thumbs.len() >= 10);
    let packets = cuts::scan_packets(p.to_str().unwrap()).unwrap();
    let lanes = vec![cuts::picture_lane(&pic), cuts::packets_lane(&packets)];
    let found = cuts::find_cuts(&lanes, &[None, None], &[], pic.frame_s);
    let on: Vec<f64> = found.iter().filter(|c| c.on()).map(|c| c.t_pic).collect();
    for (c, l) in found.iter().zip(found.iter().map(|c| cuts::pieces(12.0, std::slice::from_ref(c)))) {
        println!("cut {:.3} on {} auto {} why {:?} cues {:?} ({} pieces)", c.t_pic, c.on(), c.auto, c.why, c.cues.iter().map(|q| (q.lane, q.t, q.value, q.passes, q.flash)).collect::<Vec<_>>(), l.len());
    }
    // Each cut on its first frame of the new shot.
    assert_eq!(on.len(), 3, "{on:?}");
    for (t, want) in on.iter().zip([3.0, 6.0, 9.0]) {
        assert!((t - want).abs() < 0.021, "cut at {t}, want {want}");
    }
    // The flash: found by the picture lane, off because the picture comes back.
    let flash = found.iter().find(|c| (c.t_pic - 4.48).abs() < 0.1).expect("the flash is a cut that is off");
    assert!(!flash.on() && flash.why.contains("flash"), "{flash:?}");
    // Switched on by hand, it makes a fifth piece.
    let with = cuts::find_cuts(&lanes, &[None, None], &[(flash.t_pic, Switch::On)], pic.frame_s);
    assert_eq!(cuts::pieces(12.0, &with).len(), 5);
    let ps = cuts::pieces(12.0, &found);
    assert_eq!(ps.len(), 4);
    assert!((ps[1].v0 - 3.0).abs() < 0.021 && (ps[1].len() - 3.0).abs() < 0.05);
}

/// A phone-like file (a keyframe every second, none at scene changes) cut without re-encoding:
/// 0 to 3.5 s, then from 6 s on, joined by ffmpeg's concat demuxer. The second piece starts on a
/// keyframe half a second after the last one: off the rhythm, a cue; the picture barely changes
/// (one moving test picture), a jump cut.
#[test]
fn a_piece_joined_without_re_encoding_starts_off_the_keyframe_rhythm() {
    let d = out_dir();
    let whole = d.join("phone.mp4");
    let run = |args: &[&str]| assert!(corroborate::no_window(&mut std::process::Command::new("ffmpeg")).args(["-v", "error", "-y"]).args(args).status().is_ok_and(|s| s.success()), "{args:?}");
    if !whole.exists() {
        run(&["-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25:duration=12", "-c:v", "libx264", "-g", "25", "-sc_threshold", "0", whole.to_str().unwrap()]);
    }
    let (a, b, list, joined) = (d.join("a.mp4"), d.join("b.mp4"), d.join("list.txt"), d.join("joined.mp4"));
    run(&["-i", whole.to_str().unwrap(), "-t", "3.5", "-c", "copy", a.to_str().unwrap()]);
    run(&["-ss", "6", "-i", whole.to_str().unwrap(), "-c", "copy", b.to_str().unwrap()]);
    let fwd = |p: &PathBuf| p.to_str().unwrap().replace('\\', "/");
    std::fs::write(&list, format!("file '{}'\nfile '{}'\n", fwd(&a), fwd(&b))).unwrap();
    run(&["-f", "concat", "-safe", "0", "-i", list.to_str().unwrap(), "-c", "copy", joined.to_str().unwrap()]);
    let p = cuts::scan_packets(joined.to_str().unwrap()).unwrap();
    println!("keyframes {:?} gop {:?} off {:?} gaps {:?}", p.keyframes, p.gop_s, p.off_rhythm, p.gaps);
    assert_eq!(p.gop_s.map(|g| (g * 100.0).round()), Some(100.0));
    assert_eq!(p.off_rhythm.len(), 1, "{:?}", p.off_rhythm);
    // A stream copy ends on whole packets: the first piece ran to 3.6 s, not 3.5.
    assert!((p.off_rhythm[0] - 3.5).abs() < 0.15, "{:?}", p.off_rhythm);
    let lane = cuts::packets_lane(&p);
    let found = cuts::find_cuts(&[lane], &[None], &[], p.frame_s);
    assert_eq!(found.iter().filter(|c| c.on()).count(), 1);
}
