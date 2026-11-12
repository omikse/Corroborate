//! Where a recording is cut, and into which pieces (the Cut detector node; plan and sources in
//! docs/cut-detector/plan.md). Each cue is a lane of marks along the recording's own time, with
//! a strength and a threshold; a cut is where a cue passes its threshold. Nothing is decided here
//! for good: every cut keeps the cues behind it, and a person switches any cut on or off, or adds
//! one by hand.
//!
//! Lanes:
//! - **picture**: ffmpeg's `scdet` filter (scene change score per frame, the mean absolute frame
//!   difference in percent, default threshold 10), on frames brought to 320 pixels wide first.
//!   A cut must last: the frame before it is compared with the frame a second after it, and the
//!   frame a second before with the frame after (the idea of PERSIST, Lin et al. 2026). Where the
//!   picture comes back (a flash, a camera passing in front) the cut starts off.
//! - **keyframes and gaps** (ffprobe's packets): packet times that jump (a stream that paused, or
//!   pieces joined without re-encoding), and keyframes off the file's regular rhythm (a piece
//!   cut without re-encoding starts on a keyframe).
//! - **sound level** and **background noise**, **compare with** and **events**: see `sound`,
//!   `compare` and `Lane`.
//!
//! A cut has a picture time and a sound time: in a J-cut or L-cut the sound changes before or
//! after the picture. Picture lanes set the first, sound lanes the second; with only one kind of
//! cue both are the same. Pieces split their video at the picture time and their audio at the
//! sound time.

use std::io::{BufRead, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

/// The filmstrip's pictures.
pub const THUMB_W: usize = 48;
pub const THUMB_H: usize = 27;
/// The pictures the lasting check compares.
const SMALL_W: usize = 16;
const SMALL_H: usize = 9;
/// scdet's own default threshold.
pub const PICTURE_THR: f32 = 10.0;
/// How far apart the frames the lasting check compares are, seconds.
pub const LASTING_S: f64 = 1.0;
/// A picture cue whose lasting score is below this part of the threshold, or below this part of
/// its own score, is a flash: its cut starts off. On the test clip a real cut lasts 0.8 to 1.1
/// times its score, a flash in a moving shot 0.15 times.
pub const FLASH: f32 = 0.5;
pub const FLASH_OF_SCORE: f32 = 1.0 / 3.0;
/// Peaks of one lane closer than this are one peak (the strongest), seconds.
pub const PEAK_S: f64 = 0.5;
/// Lanes' peaks smaller than this are not marked at all (the lane's curve still shows them).
const PICTURE_FLOOR: f32 = 3.0;
/// Cues of different lanes this close are one cut, seconds. A J-cut or L-cut moves the sound by
/// up to about this much.
pub const JOIN_S: f64 = 1.0;
/// A saved switch belongs to the cut whose picture or sound time is this close, seconds.
pub const SWITCH_S: f64 = 0.25;
/// Keyframe lane: more than this part of the keyframe intervals off the usual one means the file
/// has no regular rhythm, and the lane gives no cues.
const IRREGULAR: f64 = 0.3;

/// What a lane's cut moves: the picture, the sound, or both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    Picture,
    Sound,
    Both,
    /// Marks from another node: they confirm cuts, they do not make them.
    Events,
}

/// One mark on a lane.
#[derive(Clone, Debug, PartialEq)]
pub struct Peak {
    pub t: f64,
    /// Its strength, in the lane's unit.
    pub value: f32,
    /// How far the true moment may be, seconds (1 sigma).
    pub error_s: f64,
    /// What it is, in words.
    pub note: String,
    /// Picture lane: the lasting score (see the module docs).
    pub lasting: Option<f32>,
}

/// One cue along the recording.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    /// Its key in the saved rows (thresholds): `picture`, `keyframes`, `level`, `noise`,
    /// `compare 2`, `events 1`.
    pub key: String,
    pub name: String,
    pub role: Role,
    pub unit: String,
    /// The cue all along, for drawing (may be empty).
    pub curve: Vec<(f64, f32)>,
    pub peaks: Vec<Peak>,
    /// Its default threshold; None for an events lane (marks only).
    pub thr: Option<f32>,
    /// What the lane could not do, or how it was made.
    pub note: String,
}

/// A small picture, `THUMB_W` × `THUMB_H` RGB.
#[derive(Clone, Debug)]
pub struct Thumb {
    pub t: f64,
    pub rgb: Vec<u8>,
}

/// What the picture pass found.
#[derive(Clone, Debug, Default)]
pub struct PictureScan {
    /// Each frame's time, scdet score and lasting score.
    pub times: Vec<f64>,
    pub score: Vec<f32>,
    pub lasting: Vec<f32>,
    /// One picture a second, for the filmstrip.
    pub thumbs: Vec<Thumb>,
    /// Frozen and black stretches (ffmpeg's freezedetect and blackdetect).
    pub freeze: Vec<(f64, f64)>,
    pub black: Vec<(f64, f64)>,
    pub frame_s: f64,
}

/// What ffmpeg's frame metadata says, line by line.
#[derive(Default)]
struct Meta {
    times: Vec<f64>,
    score: Vec<f32>,
    freeze: Vec<(f64, f64)>,
    black: Vec<(f64, f64)>,
}

impl Meta {
    /// One line of ffmpeg's log; only the metadata filter's lines count.
    fn line(&mut self, line: &str) {
        if !line.starts_with("[Parsed_metadata") {
            return;
        }
        let Some((_, body)) = line.split_once("] ") else { return };
        let body = body.trim();
        if let Some(rest) = body.strip_prefix("frame:") {
            let t = rest.split_whitespace().find_map(|w| w.strip_prefix("pts_time:")).and_then(|v| v.parse::<f64>().ok());
            // A frame with no time (rare) is put one frame after the last.
            let t = t.unwrap_or_else(|| self.times.last().map_or(0.0, |l| l + 1.0 / 30.0));
            self.times.push(t);
            self.score.push(0.0);
            return;
        }
        let Some((k, v)) = body.split_once('=') else { return };
        let Ok(v) = v.trim().parse::<f64>() else { return };
        match k {
            "lavfi.scd.score" => {
                if let Some(s) = self.score.last_mut() {
                    *s = v as f32;
                }
            }
            "lavfi.freezedetect.freeze_start" => self.freeze.push((v, f64::NAN)),
            "lavfi.freezedetect.freeze_end" => {
                if let Some(f) = self.freeze.last_mut().filter(|f| f.1.is_nan()) {
                    f.1 = v;
                }
            }
            "lavfi.black_start" => self.black.push((v, f64::NAN)),
            "lavfi.black_end" => {
                if let Some(b) = self.black.last_mut().filter(|b| b.1.is_nan()) {
                    b.1 = v;
                }
            }
            _ => {}
        }
    }

    /// Stretches still open at the end close there.
    fn close(&mut self, end: f64) {
        for s in self.freeze.iter_mut().chain(self.black.iter_mut()) {
            if s.1.is_nan() {
                s.1 = end;
            }
        }
    }
}

/// Mean absolute difference of two small pictures, in percent of full scale.
fn mad(a: &[u8], b: &[u8]) -> f32 {
    let s: u32 = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as u32).sum();
    s as f32 / a.len().max(1) as f32 * 100.0 / 255.0
}

/// A thumbnail brought down to the lasting check's size, grey.
fn small(rgb: &[u8]) -> Vec<u8> {
    let (fx, fy) = (THUMB_W / SMALL_W, THUMB_H / SMALL_H);
    let mut out = Vec::with_capacity(SMALL_W * SMALL_H);
    for y in 0..SMALL_H {
        for x in 0..SMALL_W {
            let mut s = 0u32;
            for dy in 0..fy {
                for dx in 0..fx {
                    let p = ((y * fy + dy) * THUMB_W + x * fx + dx) * 3;
                    s += (rgb[p] as u32 * 299 + rgb[p + 1] as u32 * 587 + rgb[p + 2] as u32 * 114) / 1000;
                }
            }
            out.push((s / (fx * fy) as u32) as u8);
        }
    }
    out
}

/// The lasting score of each frame: how different the pictures stay a second (`LASTING_S`)
/// either side of it; the smaller of the two crossings.
fn lasting(small: &[Vec<u8>], frame_s: f64) -> Vec<f32> {
    let n = small.len();
    let l = ((LASTING_S / frame_s.max(1e-3)).round() as usize).max(1);
    (0..n)
        .map(|k| {
            if k == 0 {
                return 0.0;
            }
            let after = (k + l).min(n - 1);
            let before = k.saturating_sub(1 + l);
            mad(&small[k - 1], &small[after]).min(mad(&small[before], &small[k]))
        })
        .collect()
}

/// The middle of the sorted differences: one frame's length.
fn frame_length(times: &[f64]) -> f64 {
    let mut d: Vec<f64> = times.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0.0).collect();
    if d.is_empty() {
        return 1.0 / 30.0;
    }
    d.sort_by(f64::total_cmp);
    d[d.len() / 2]
}

/// Reads every frame of the file's first video stream once (ffmpeg): scdet scores, the lasting
/// check, frozen and black stretches, and the filmstrip. `progress` is told the seconds read.
pub fn scan_picture(path: &str, stop: &AtomicBool, progress: &dyn Fn(f64)) -> Result<PictureScan, String> {
    let filter = format!(
        "[0:v:0]scale=320:-2,scdet=t=100,freezedetect=d=0.5,blackdetect=d=0.1,metadata=print,scale={THUMB_W}:{THUMB_H},format=rgb24[o]"
    );
    let mut child = crate::no_window(&mut Command::new("ffmpeg"))
        .args(["-hide_banner", "-nostdin", "-loglevel", "info", "-i", path, "-an", "-sn", "-dn", "-fps_mode", "passthrough", "-filter_complex", &filter, "-map", "[o]", "-f", "rawvideo", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start ffmpeg (is it on PATH?): {e}"))?;
    let stderr = child.stderr.take().ok_or("no ffmpeg log")?;
    let log = std::thread::spawn(move || {
        let mut m = Meta::default();
        let mut last_error = String::new();
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.contains("rror") || line.contains("does not contain any stream") || line.contains("matches no streams") {
                last_error = line.trim().to_string();
            }
            m.line(&line);
        }
        (m, last_error)
    });
    let mut out = child.stdout.take().ok_or("no ffmpeg output")?;
    let size = THUMB_W * THUMB_H * 3;
    let mut buf = vec![0u8; size];
    let (mut smalls, mut thumbs): (Vec<Vec<u8>>, Vec<(usize, Vec<u8>)>) = (Vec::new(), Vec::new());
    let mut next_thumb = 0usize;
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("stopped".into());
        }
        if out.read_exact(&mut buf).is_err() {
            break;
        }
        let k = smalls.len();
        smalls.push(small(&buf));
        // A picture a second (at 30 frames a second); the times are known once the log is read.
        if k >= next_thumb {
            thumbs.push((k, buf.clone()));
            next_thumb = k + 30;
        }
        if k % 300 == 0 {
            progress(k as f64 / 30.0);
        }
    }
    let _ = child.wait();
    let (mut m, last_error) = log.join().map_err(|_| "ffmpeg's log reader crashed")?;
    if smalls.is_empty() {
        return Err(if last_error.is_empty() { "no video frames".into() } else { last_error });
    }
    // Frames and their log lines go one to one; a short log keeps the frames it names.
    let n = smalls.len().min(m.times.len());
    if n == 0 {
        return Err("ffmpeg gave frames without times".into());
    }
    smalls.truncate(n);
    m.times.truncate(n);
    m.score.truncate(n);
    let frame_s = frame_length(&m.times);
    let end = m.times[n - 1] + frame_s;
    m.close(end);
    let last = lasting(&smalls, frame_s);
    let thumbs = thumbs.into_iter().filter(|(k, _)| *k < n).map(|(k, rgb)| Thumb { t: m.times[k], rgb }).collect();
    Ok(PictureScan { times: m.times, score: m.score, lasting: last, thumbs, freeze: m.freeze, black: m.black, frame_s })
}

/// The strongest values of a curve: each at least `floor`, the largest within `PEAK_S` either
/// side. Returns indices.
fn peaks_of(times: &[f64], v: &[f32], floor: f32) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut k = 0;
    while k < v.len() {
        if v[k] >= floor {
            let (mut lo, mut hi) = (k, k);
            while lo > 0 && times[k] - times[lo - 1] <= PEAK_S {
                lo -= 1;
            }
            while hi + 1 < v.len() && times[hi + 1] - times[k] <= PEAK_S {
                hi += 1;
            }
            // The first of equal values wins.
            let best = (lo..=hi).max_by(|&a, &b| v[a].total_cmp(&v[b]).then(b.cmp(&a))).unwrap_or(k);
            if best == k {
                out.push(k);
            }
        }
        k += 1;
    }
    out
}

/// The picture lane from a picture scan.
pub fn picture_lane(p: &PictureScan) -> Lane {
    let peaks = peaks_of(&p.times, &p.score, PICTURE_FLOOR)
        .into_iter()
        .map(|k| {
            let l = p.lasting.get(k).copied();
            Peak { t: p.times[k], value: p.score[k], error_s: p.frame_s / 2.0, note: format!("scene change {:.1}, lasting {:.1}", p.score[k], l.unwrap_or(0.0)), lasting: l }
        })
        .collect();
    let mut note = format!("ffmpeg scdet on every frame ({:.2} frames a second), lasting checked a second either side", 1.0 / p.frame_s);
    let freeze: f64 = p.freeze.iter().map(|f| f.1 - f.0).sum();
    let black: f64 = p.black.iter().map(|b| b.1 - b.0).sum();
    if freeze > 0.0 || black > 0.0 {
        note.push_str(&format!("; frozen {freeze:.1} s, black {black:.1} s"));
    }
    Lane { key: "picture".into(), name: "picture".into(), role: Role::Picture, unit: "scene change".into(), curve: p.times.iter().copied().zip(p.score.iter().copied()).collect(), peaks, thr: Some(PICTURE_THR), note }
}

/// What ffprobe's video packets say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Packets {
    pub keyframes: Vec<f64>,
    /// Where packet times jump: the time after the jump and the seconds missing.
    pub gaps: Vec<(f64, f64)>,
    pub frame_s: f64,
    /// The usual keyframe interval, when the file has a regular one.
    pub gop_s: Option<f64>,
    /// Keyframes off that rhythm.
    pub off_rhythm: Vec<f64>,
}

/// Reads ffprobe's `pts_time,flags` lines (one per packet, in file order).
pub fn packets_of(csv: &str) -> Packets {
    let mut pk: Vec<(f64, bool)> = csv
        .lines()
        .filter_map(|l| {
            let mut f = l.split(',');
            let t = f.next()?.trim().parse::<f64>().ok()?;
            Some((t, f.next().unwrap_or("").contains('K')))
        })
        .collect();
    pk.sort_by(|a, b| a.0.total_cmp(&b.0));
    let times: Vec<f64> = pk.iter().map(|p| p.0).collect();
    let frame_s = frame_length(&times);
    let gaps = times.windows(2).filter(|w| w[1] - w[0] > 1.5 * frame_s).map(|w| (w[1], w[1] - w[0] - frame_s)).collect();
    let keyframes: Vec<f64> = pk.iter().filter(|p| p.1).map(|p| p.0).collect();
    // The usual interval, in whole frames.
    let frames = |d: f64| (d / frame_s).round() as i64;
    let mut count: std::collections::BTreeMap<i64, usize> = Default::default();
    for w in keyframes.windows(2) {
        *count.entry(frames(w[1] - w[0])).or_default() += 1;
    }
    let intervals = keyframes.len().saturating_sub(1);
    let usual = count.iter().max_by_key(|(_, c)| **c).map(|(f, _)| *f);
    // The last interval is often short (the file ends): it does not count.
    let off: Vec<f64> = match usual {
        Some(u) => keyframes.windows(2).take(intervals.saturating_sub(1)).filter(|w| frames(w[1] - w[0]) != u).map(|w| w[1]).collect(),
        None => Vec::new(),
    };
    let regular = intervals >= 3 && (off.len() as f64) <= IRREGULAR * intervals as f64;
    Packets { keyframes, gaps, frame_s, gop_s: if regular { usual.map(|u| u as f64 * frame_s) } else { None }, off_rhythm: if regular { off } else { Vec::new() } }
}

/// ffprobe's video packets of the file.
pub fn scan_packets(path: &str) -> Result<Packets, String> {
    let o = crate::no_window(&mut Command::new("ffprobe"))
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "packet=pts_time,flags", "-of", "csv=p=0", path])
        .output()
        .map_err(|e| format!("could not start ffprobe (is it on PATH?): {e}"))?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok(packets_of(&String::from_utf8_lossy(&o.stdout)))
}

/// The keyframes and gaps lane.
pub fn packets_lane(p: &Packets) -> Lane {
    let mut peaks: Vec<Peak> = p.gaps.iter().map(|&(t, s)| Peak { t, value: 1.0, error_s: p.frame_s / 2.0, note: format!("packet times jump: {s:.2} s missing"), lasting: None }).collect();
    peaks.extend(p.off_rhythm.iter().map(|&t| Peak { t, value: 1.0, error_s: p.frame_s / 2.0, note: "keyframe off the file's rhythm".into(), lasting: None }));
    peaks.sort_by(|a, b| a.t.total_cmp(&b.t));
    let note = match p.gop_s {
        Some(g) => format!("{} keyframes, every {g:.2} s as a rule; {} off it; {} jumps in packet times", p.keyframes.len(), p.off_rhythm.len(), p.gaps.len()),
        None => format!("{} keyframes with no regular rhythm (no cues from them); {} jumps in packet times", p.keyframes.len(), p.gaps.len()),
    };
    Lane { key: "keyframes".into(), name: "keyframes and gaps".into(), role: Role::Picture, unit: "mark".into(), curve: Vec::new(), peaks, thr: Some(0.5), note }
}

/// A person's choice about one cut, saved with the project.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Switch {
    On,
    Off,
    /// A cut where no cue reached: the person put it there.
    Hand,
}

impl Switch {
    pub fn word(self) -> &'static str {
        match self {
            Switch::On => "on",
            Switch::Off => "off",
            Switch::Hand => "hand",
        }
    }

    pub fn of(w: &str) -> Option<Switch> {
        match w.trim() {
            "on" => Some(Switch::On),
            "off" => Some(Switch::Off),
            "hand" => Some(Switch::Hand),
            _ => None,
        }
    }
}

/// One lane's mark at a cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub lane: usize,
    pub t: f64,
    pub value: f32,
    /// It reaches its lane's threshold.
    pub passes: bool,
    /// A picture cue that does not last (a flash).
    pub flash: bool,
}

/// An event from another node at a cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Confirm {
    pub lane: usize,
    pub t: f64,
    pub error_s: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// Where the picture changes and where the sound changes, seconds in the file.
    pub t_pic: f64,
    pub t_snd: f64,
    pub cues: Vec<Cue>,
    pub confirms: Vec<Confirm>,
    /// What the lanes say: a cue passes (and is not only a flash).
    pub auto: bool,
    pub switch: Option<Switch>,
    /// Why it is off by itself, when it is.
    pub why: String,
    /// How far its picture time may be from the true moment, seconds (1 sigma).
    pub error_s: f64,
}

impl Cut {
    pub fn on(&self) -> bool {
        match self.switch {
            Some(Switch::On | Switch::Hand) => true,
            Some(Switch::Off) => false,
            None => self.auto,
        }
    }

    pub fn hand(&self) -> bool {
        self.switch == Some(Switch::Hand) && self.cues.is_empty()
    }

    /// Its time where switches and the result file name it.
    pub fn t(&self) -> f64 {
        self.t_pic
    }

    /// The sound comes before the picture (J-cut, > 0) or after it (L-cut, < 0), seconds.
    pub fn lead(&self) -> f64 {
        self.t_pic - self.t_snd
    }
}

fn moves_picture(r: Role) -> bool {
    matches!(r, Role::Picture | Role::Both)
}

fn moves_sound(r: Role) -> bool {
    matches!(r, Role::Sound | Role::Both)
}

/// The cuts the lanes give at thresholds `thr` (one per lane; None: the lane's default), with a
/// person's `switches` (time, choice). `frame_s`: one frame, the least error of a cut.
pub fn find_cuts(lanes: &[Lane], thr: &[Option<f32>], switches: &[(f64, Switch)], frame_s: f64) -> Vec<Cut> {
    let thr_of = |k: usize| thr.get(k).copied().flatten().or(lanes[k].thr);
    // Every mark of a lane that has a threshold, passing or not.
    let mut all: Vec<Cue> = Vec::new();
    for (k, lane) in lanes.iter().enumerate() {
        let Some(th) = thr_of(k) else { continue };
        for p in &lane.peaks {
            let flash = lane.role == Role::Picture && p.lasting.is_some_and(|l| l < FLASH * th || l < p.value * FLASH_OF_SCORE);
            all.push(Cue { lane: k, t: p.t, value: p.value, passes: p.value >= th, flash });
        }
    }
    all.sort_by(|a, b| a.t.total_cmp(&b.t));
    let strength = |c: &Cue| c.value / thr_of(c.lane).unwrap_or(1.0).max(1e-6);
    // Passing cues within JOIN_S of each other are one cut.
    let passing: Vec<&Cue> = all.iter().filter(|c| c.passes).collect();
    let mut clusters: Vec<Vec<&Cue>> = Vec::new();
    for c in passing {
        match clusters.last_mut() {
            Some(cl) if c.t - cl.last().unwrap().t <= JOIN_S => cl.push(c),
            _ => clusters.push(vec![c]),
        }
    }
    let err_of = |c: &Cue| lanes[c.lane].peaks.iter().find(|p| p.t == c.t).map_or(frame_s, |p| p.error_s.max(frame_s / 2.0));
    let mut cuts: Vec<Cut> = clusters
        .into_iter()
        .map(|cl| {
            let real: Vec<&&Cue> = cl.iter().filter(|c| !c.flash).collect();
            let best = |pick: &dyn Fn(Role) -> bool| real.iter().filter(|c| pick(lanes[c.lane].role)).max_by(|a, b| strength(a).total_cmp(&strength(b))).map(|c| (c.t, err_of(c)));
            let pic = best(&moves_picture);
            let snd = best(&moves_sound);
            let flash_t = cl.iter().max_by(|a, b| strength(a).total_cmp(&strength(b))).map(|c| c.t).unwrap_or(0.0);
            let (t_pic, error_s) = pic.or(snd).unwrap_or((flash_t, frame_s));
            let t_snd = snd.map_or(t_pic, |s| s.0);
            let (lo, hi) = (cl.first().unwrap().t - JOIN_S, cl.last().unwrap().t + JOIN_S);
            // Every lane's marks round it, passing or not: what agrees, and by how much.
            let cues = all.iter().filter(|c| c.t >= lo && c.t <= hi).cloned().collect();
            let auto = !real.is_empty();
            let why = if auto { String::new() } else { "a flash: the picture comes back".to_string() };
            Cut { t_pic, t_snd, cues, confirms: Vec::new(), auto, switch: None, why, error_s }
        })
        .collect();
    for &(t, sw) in switches {
        let near = cuts.iter().position(|c| (c.t_pic - t).abs() <= SWITCH_S || (c.t_snd - t).abs() <= SWITCH_S);
        match (near, sw) {
            (Some(k), Switch::Hand) => cuts[k].switch = Some(Switch::On),
            (Some(k), s) => cuts[k].switch = Some(s),
            (None, Switch::Hand) => cuts.push(Cut { t_pic: t, t_snd: t, cues: Vec::new(), confirms: Vec::new(), auto: false, switch: Some(Switch::Hand), why: String::new(), error_s: frame_s }),
            // A switch for a cut the lanes no longer give: nothing to switch.
            (None, _) => {}
        }
    }
    cuts.sort_by(|a, b| a.t_pic.total_cmp(&b.t_pic));
    // Events from other nodes confirm the cuts they fall on.
    for (k, lane) in lanes.iter().enumerate().filter(|(_, l)| l.role == Role::Events) {
        for p in &lane.peaks {
            let within = p.error_s.max(frame_s);
            if let Some(c) = cuts.iter_mut().filter(|c| (c.t_pic - p.t).abs().min((c.t_snd - p.t).abs()) <= within).min_by(|a, b| (a.t_pic - p.t).abs().total_cmp(&(b.t_pic - p.t).abs())) {
                c.confirms.push(Confirm { lane: k, t: p.t, error_s: p.error_s });
            }
        }
    }
    cuts
}

/// One piece of the recording: its video from `v0` to `v1`, its audio from `a0` to `a1`,
/// seconds in the file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub n: usize,
    pub v0: f64,
    pub v1: f64,
    pub a0: f64,
    pub a1: f64,
}

impl Piece {
    /// Its length: its audio's (the time outputs follow the audio).
    pub fn len(&self) -> f64 {
        self.a1 - self.a0
    }
}

/// The pieces a recording of length `len` falls into at the cuts that are on.
pub fn pieces(len: f64, cuts: &[Cut]) -> Vec<Piece> {
    let mut on: Vec<&Cut> = cuts.iter().filter(|c| c.on() && c.t_pic > 0.0 && c.t_pic < len).collect();
    on.sort_by(|a, b| a.t_pic.total_cmp(&b.t_pic));
    let mut out = Vec::new();
    let (mut v0, mut a0) = (0.0, 0.0);
    for c in on {
        let a1 = c.t_snd.clamp(a0, len);
        out.push(Piece { n: out.len() + 1, v0, v1: c.t_pic, a0, a1 });
        (v0, a0) = (c.t_pic, a1);
    }
    out.push(Piece { n: out.len() + 1, v0, v1: len, a0, a1: len });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(key: &str, role: Role, thr: Option<f32>, peaks: &[(f64, f32)]) -> Lane {
        Lane {
            key: key.into(),
            name: key.into(),
            role,
            unit: String::new(),
            curve: Vec::new(),
            peaks: peaks.iter().map(|&(t, value)| Peak { t, value, error_s: 0.02, note: String::new(), lasting: None }).collect(),
            thr,
            note: String::new(),
        }
    }

    #[test]
    fn metadata_lines_give_frames_scores_and_stretches() {
        let mut m = Meta::default();
        for l in [
            "[Parsed_metadata_3 @ 0000] frame:0    pts:0       pts_time:0",
            "[Parsed_metadata_3 @ 0000] lavfi.scd.mafd=0.000",
            "[Parsed_metadata_3 @ 0000] lavfi.scd.score=0.000",
            "[Parsed_metadata_3 @ 0000] frame:1    pts:512     pts_time:0.04",
            "[Parsed_metadata_3 @ 0000] lavfi.scd.score=18.984",
            "[Parsed_metadata_3 @ 0000] lavfi.black_start=0.04",
            "[Parsed_freezedetect_2 @ 0000] lavfi.freezedetect.freeze_start: 3",
            "[Parsed_metadata_3 @ 0000] lavfi.freezedetect.freeze_start=0.04",
        ] {
            m.line(l);
        }
        m.close(2.0);
        assert_eq!(m.times, [0.0, 0.04]);
        assert_eq!(m.score, [0.0, 18.984]);
        assert_eq!(m.black, [(0.04, 2.0)]);
        assert_eq!(m.freeze, [(0.04, 2.0)]);
    }

    #[test]
    fn keyframes_off_the_rhythm_and_packet_jumps() {
        // 25 frames a second, a keyframe every second; at 3.48 s a piece starts (stream copy), and
        // at 6 s the packets jump by 2 s.
        let mut csv = String::new();
        let mut t = 0.0;
        let mut key_at = vec![0.0, 1.0, 2.0, 3.0, 3.48, 4.48, 5.48];
        key_at.extend([8.0, 9.0, 10.0, 11.0]);
        while t < 12.0 {
            if !(6.0..8.0).contains(&t) {
                let k = key_at.iter().any(|k: &f64| (k - t).abs() < 1e-6);
                csv.push_str(&format!("{t:.6},{}\n", if k { "K__" } else { "___" }));
            }
            t = ((t + 0.04) * 1000.0f64).round() / 1000.0;
        }
        let p = packets_of(&csv);
        assert!((p.frame_s - 0.04).abs() < 1e-9);
        assert_eq!(p.gaps.len(), 1);
        assert!((p.gaps[0].0 - 8.0).abs() < 1e-6 && (p.gaps[0].1 - 2.0).abs() < 1e-6, "{:?}", p.gaps);
        assert_eq!(p.gop_s.map(|g| (g * 100.0).round()), Some(100.0));
        // 3.48 (0.48 after 3) and 8.0 (2.52 after 5.48) are off; 4.48 and 5.48 keep the rhythm.
        assert_eq!(p.off_rhythm, [3.48, 8.0]);
        let l = packets_lane(&p);
        assert_eq!(l.peaks.len(), 3, "{:?}", l.peaks);
    }

    #[test]
    fn no_rhythm_no_keyframe_cues() {
        let csv: String = [0.0, 0.4, 1.9, 2.1, 3.7, 3.9, 5.5, 6.0].iter().map(|t| format!("{t:.6},K__\n")).collect();
        let p = packets_of(&csv);
        assert_eq!(p.gop_s, None);
        assert!(p.off_rhythm.is_empty());
    }

    #[test]
    fn cues_close_together_are_one_cut_with_a_picture_and_a_sound_time() {
        let lanes = [
            lane("picture", Role::Picture, Some(10.0), &[(10.0, 30.0), (40.0, 12.0), (70.0, 4.0)]),
            lane("level", Role::Sound, Some(4.0), &[(10.4, 6.0), (55.0, 9.0)]),
        ];
        let cuts = find_cuts(&lanes, &[None, None], &[], 1.0 / 25.0);
        let times: Vec<(f64, f64)> = cuts.iter().map(|c| (c.t_pic, c.t_snd)).collect();
        assert_eq!(times, [(10.0, 10.4), (40.0, 40.0), (55.0, 55.0)]);
        assert!(cuts.iter().all(|c| c.on()));
        // An L-cut: the sound changes 0.4 s after the picture.
        assert!((cuts[0].lead() + 0.4).abs() < 1e-9);
        // A higher picture threshold drops the weaker picture cut.
        let cuts = find_cuts(&lanes, &[Some(20.0), None], &[], 1.0 / 25.0);
        assert_eq!(cuts.len(), 2);
    }

    #[test]
    fn a_flash_starts_off_and_switches_win() {
        let mut lanes = vec![lane("picture", Role::Picture, Some(10.0), &[(10.0, 30.0), (20.0, 25.0)])];
        lanes[0].peaks[0].lasting = Some(30.0);
        lanes[0].peaks[1].lasting = Some(1.0);
        let cuts = find_cuts(&lanes, &[None], &[], 0.04);
        assert!(cuts[0].on() && !cuts[1].on());
        assert!(cuts[1].why.contains("flash"));
        let cuts = find_cuts(&lanes, &[None], &[(10.1, Switch::Off), (20.0, Switch::On), (30.0, Switch::Hand), (40.0, Switch::Off)], 0.04);
        let on: Vec<(f64, bool)> = cuts.iter().map(|c| (c.t_pic, c.on())).collect();
        assert_eq!(on, [(10.0, false), (20.0, true), (30.0, true)]);
        assert!(cuts[2].hand());
    }

    #[test]
    fn events_confirm_cuts_and_make_none() {
        let lanes = [
            lane("picture", Role::Picture, Some(10.0), &[(10.0, 30.0)]),
            Lane { peaks: vec![Peak { t: 10.3, value: 1.0, error_s: 0.5, note: String::new(), lasting: None }, Peak { t: 50.0, value: 1.0, error_s: 0.5, note: String::new(), lasting: None }], ..lane("events 1", Role::Events, None, &[]) },
        ];
        let cuts = find_cuts(&lanes, &[None, None], &[], 0.04);
        assert_eq!(cuts.len(), 1);
        assert_eq!(cuts[0].confirms.len(), 1);
        assert!((cuts[0].confirms[0].t - 10.3).abs() < 1e-9);
        // Outside its error it confirms nothing.
        let lanes = [lanes[0].clone(), Lane { peaks: vec![Peak { t: 10.6, value: 1.0, error_s: 0.5, note: String::new(), lasting: None }], ..lanes[1].clone() }];
        assert!(find_cuts(&lanes, &[None, None], &[], 0.04)[0].confirms.is_empty());
    }

    #[test]
    fn pieces_split_video_at_the_picture_and_audio_at_the_sound() {
        let lanes = [lane("picture", Role::Picture, Some(10.0), &[(10.0, 30.0), (40.0, 30.0)]), lane("level", Role::Sound, Some(4.0), &[(9.6, 6.0)])];
        let cuts = find_cuts(&lanes, &[None, None], &[], 0.04);
        let p = pieces(60.0, &cuts);
        assert_eq!(p.len(), 3);
        assert_eq!((p[0].v0, p[0].v1, p[0].a0, p[0].a1), (0.0, 10.0, 0.0, 9.6));
        assert_eq!((p[1].v0, p[1].v1, p[1].a0, p[1].a1), (10.0, 40.0, 9.6, 40.0));
        assert_eq!((p[2].v0, p[2].v1, p[2].a1), (40.0, 60.0, 60.0));
        assert!((p[1].len() - 30.4).abs() < 1e-9);
        // No cut on: one piece, the whole file.
        let off = find_cuts(&lanes, &[None, None], &[(10.0, Switch::Off), (40.0, Switch::Off)], 0.04);
        assert_eq!(pieces(60.0, &off), [Piece { n: 1, v0: 0.0, v1: 60.0, a0: 0.0, a1: 60.0 }]);
    }

    #[test]
    fn lasting_tells_a_cut_from_a_flash() {
        // 25 frames of grey 50, a flash of 3 white frames, 40 of grey 50, then grey 200 for good.
        let mut f: Vec<Vec<u8>> = Vec::new();
        f.extend((0..25).map(|_| vec![50u8; SMALL_W * SMALL_H]));
        f.extend((0..3).map(|_| vec![255u8; SMALL_W * SMALL_H]));
        f.extend((0..40).map(|_| vec![50u8; SMALL_W * SMALL_H]));
        f.extend((0..40).map(|_| vec![200u8; SMALL_W * SMALL_H]));
        let l = lasting(&f, 0.04);
        assert!(l[25] < 1.0, "flash {}", l[25]);
        assert!(l[68] > 50.0, "cut {}", l[68]);
    }
}
