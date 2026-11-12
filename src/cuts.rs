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
    /// Its marks make cuts on their own. A lane that does not (the background noise) only adds to
    /// another lane's cue at the same moment; alone its marks show, off, for a person to switch.
    pub alone: bool,
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
    Lane { key: "picture".into(), name: "picture".into(), role: Role::Picture, unit: "scene change".into(), curve: p.times.iter().copied().zip(p.score.iter().copied()).collect(), peaks, thr: Some(PICTURE_THR), alone: true, note }
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
    Lane { key: "keyframes".into(), name: "keyframes and gaps".into(), role: Role::Picture, unit: "mark".into(), curve: Vec::new(), peaks, thr: Some(0.5), alone: true, note }
}

// ---------------------------------------------------------------- sound

/// The sound lanes' audio rate (the audio match's, `audio_match::SR`).
pub const SOUND_SR: f64 = crate::audio_match::SR as f64;
/// The sound level: one value every `LEVEL_STEP_S`, compared over `LEVEL_SIDE_S` either side.
pub const LEVEL_STEP_S: f64 = 0.05;
pub const LEVEL_SIDE_S: f64 = 1.0;
/// Its default threshold, dB: on a 16 min phone recording of a crowd (Jiku) the level moved by
/// more than 12 dB between one second and the next 13 times, by more than 15 dB 5 times.
pub const LEVEL_THR: f32 = 15.0;
/// The background noise: one estimate for each `NOISE_WIN_S` window every `NOISE_STEP_S`.
pub const NOISE_WIN_S: f64 = 1.0;
pub const NOISE_STEP_S: f64 = 0.25;
/// Seconds either side of a moment the noise is compared over.
pub const NOISE_SIDE_S: f64 = 2.0;
/// Its default threshold, dB. The estimate is rough on real sound: on the same phone recording
/// it stepped by more than 6 dB 198 times in 16 min, at most by 23 dB; 15 dB keeps about a dozen.
/// A lane to read along with the others more than one that cuts alone.
pub const NOISE_THR: f32 = 15.0;
/// Lane marks smaller than this are left out, dB.
const SOUND_FLOOR: f32 = 2.0;

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// The steps of a curve `db` sampled every `step` seconds: at each moment, how far the mean of
/// `side` seconds after it is from the mean of `side` seconds before it (a mean, not a median: a
/// step then peaks on its moment instead of a plateau).
fn steps(db: &[f32], step: f64, side: f64) -> Vec<(f64, f32)> {
    let w = ((side / step).round() as usize).max(1);
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    (w..db.len().saturating_sub(w).max(w))
        .filter(|&k| k + w <= db.len())
        .map(|k| (k as f64 * step, (mean(&db[k..k + w]) - mean(&db[k - w..k])).abs()))
        .collect()
}

/// A lane from a curve of steps: its peaks over `SOUND_FLOOR`.
fn step_lane(key: &str, name: &str, unit: &str, curve: Vec<(f64, f32)>, thr: f32, alone: bool, err: f64, note: String, what: &str) -> Lane {
    let times: Vec<f64> = curve.iter().map(|c| c.0).collect();
    let vals: Vec<f32> = curve.iter().map(|c| c.1).collect();
    let peaks = peaks_of(&times, &vals, SOUND_FLOOR).into_iter().map(|k| Peak { t: times[k], value: vals[k], error_s: err, note: format!("{what} steps by {:.1} {unit}", vals[k]), lasting: None }).collect();
    Lane { key: key.into(), name: name.into(), role: Role::Sound, unit: unit.into(), curve, peaks, thr: Some(thr), alone, note }
}

/// The sound level lane: loudness (RMS, root mean square, in dB) every 50 ms, and where its
/// median over the next second differs from the one over the second before.
pub fn level_lane(audio: &[f32]) -> Lane {
    let n = (LEVEL_STEP_S * SOUND_SR) as usize;
    let db: Vec<f32> = audio.chunks(n).map(|c| (10.0 * ((c.iter().map(|&x| (x * x) as f64).sum::<f64>() / c.len() as f64).max(1e-12)).log10()) as f32).collect();
    let note = format!("loudness every {:.0} ms; a mark where the mean of the next {LEVEL_SIDE_S:.0} s differs from the second before", LEVEL_STEP_S * 1e3);
    step_lane("level", "sound level", "dB", steps(&db, LEVEL_STEP_S, LEVEL_SIDE_S), LEVEL_THR, true, LEVEL_STEP_S, note, "the level")
}

/// The background noise of one window (Pan, Zhang, Lyu, ICASSP 2012): band-pass channels of a
/// natural sound have about the same kurtosis, and noise added to them lowers each channel's
/// kurtosis by its share of the channel's variance. With √κ̃ₖ = √κ (1 − σ²/vₖ) for each channel's
/// kurtosis κ̃ₖ and variance vₖ, a straight line through (1/vₖ, √κ̃ₖ) gives σ², the noise
/// variance, in dB. Channels: 16 bands of a 256-point spectrum (31 Hz bins at 8 kHz) from 125 Hz
/// up.
pub fn noise_db(x: &[f32], planner: &mut realfft::RealFftPlanner<f32>) -> Option<f32> {
    const N: usize = 256;
    const BANDS: usize = 16;
    const PER: usize = 7;
    if x.len() < 4 * N {
        return None;
    }
    let fft = planner.plan_fft_forward(N);
    let win: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
    // Per band: Σ c², Σ c⁴, count, over every coefficient's real and imaginary part.
    let mut acc = vec![(0.0f64, 0.0f64, 0usize); BANDS];
    let mut buf = vec![0.0f32; N];
    let mut out = fft.make_output_vec();
    let mut s = 0;
    while s + N <= x.len() {
        for (b, (v, w)) in buf.iter_mut().zip(x[s..s + N].iter().zip(&win)) {
            *b = v * w;
        }
        fft.process(&mut buf, &mut out).ok()?;
        for (band, a) in acc.iter_mut().enumerate() {
            for c in &out[4 + band * PER..4 + (band + 1) * PER] {
                for v in [c.re as f64, c.im as f64] {
                    let q = v * v;
                    a.0 += q;
                    a.1 += q * q;
                    a.2 += 1;
                }
            }
        }
        s += N / 2;
    }
    let pts: Vec<(f64, f64)> = acc
        .iter()
        .filter(|a| a.0 > 0.0)
        .map(|a| {
            let v = a.0 / a.2 as f64;
            let kurt = (a.1 / a.2 as f64) / (v * v) - 3.0;
            (1.0 / v, kurt.max(0.0).sqrt())
        })
        .collect();
    let vmin = acc.iter().filter(|a| a.0 > 0.0).map(|a| a.0 / a.2 as f64).fold(f64::MAX, f64::min);
    if pts.len() < 4 || vmin <= 0.0 || vmin == f64::MAX {
        return None;
    }
    // a = u − w·b by least squares; σ² = w / u.
    let n = pts.len() as f64;
    let (mb, ma) = (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
    let sbb: f64 = pts.iter().map(|p| (p.0 - mb).powi(2)).sum();
    let sab: f64 = pts.iter().map(|p| (p.0 - mb) * (p.1 - ma)).sum();
    let var = if sbb > 0.0 {
        let slope = sab / sbb;
        let u = ma - slope * mb;
        let w = -slope;
        if u > 0.0 && w > 0.0 { (w / u).min(vmin) } else { vmin }
    } else {
        vmin
    };
    Some((10.0 * var.max(1e-20).log10()) as f32)
}

/// The background noise lane: the noise level of each second, every quarter of a second, and
/// where the second after a moment differs from the second before.
pub fn noise_lane(audio: &[f32]) -> Lane {
    let mut planner = realfft::RealFftPlanner::<f32>::new();
    let (w, h) = ((NOISE_WIN_S * SOUND_SR) as usize, (NOISE_STEP_S * SOUND_SR) as usize);
    let mut est: Vec<f32> = Vec::new();
    let mut s = 0;
    while s + w <= audio.len() {
        est.push(noise_db(&audio[s..s + w], &mut planner).unwrap_or(f32::NAN));
        s += h;
    }
    // The windows wholly in the `NOISE_SIDE_S` after a moment against those wholly in the ones
    // before: the median of each.
    let win = (NOISE_WIN_S / NOISE_STEP_S).round() as usize;
    let side = (NOISE_SIDE_S / NOISE_STEP_S).round() as usize;
    let med = |a: usize, b: usize| {
        let v: Vec<f32> = est[a..b].iter().copied().filter(|x| x.is_finite()).collect();
        (v.len() * 2 > b - a).then(|| median(v))
    };
    let curve: Vec<(f64, f32)> = (side..(est.len() + win).saturating_sub(side))
        .filter_map(|k| {
            let before = med(k - side, k + 1 - win)?;
            let after = med(k, (k + side + 1 - win).min(est.len()))?;
            Some((k as f64 * NOISE_STEP_S, (after - before).abs()))
        })
        .collect();
    let note = format!("background noise level of each second (Pan, Zhang, Lyu 2012: kurtosis of 16 band-pass channels); a mark where the {NOISE_SIDE_S:.0} s after differ from the {NOISE_SIDE_S:.0} s before (medians)");
    step_lane("noise", "background noise", "dB", curve, NOISE_THR, false, NOISE_STEP_S, note, "the background noise")
}

/// How well `ours` (a window of this recording) lines up with `theirs` at `lag` seconds (their
/// moment = ours + lag), allowing `slack` seconds either way: GCC-PHAT's peak-to-sidelobe ratio,
/// 0 when the peak is outside the slack.
fn fits(planner: &mut realfft::RealFftPlanner<f32>, ours: &[f32], theirs: &[f32], t0: f64, lag: f64, slack: f64) -> f64 {
    let sr = SOUND_SR;
    let a = (t0 + lag - slack) * sr;
    if a < 0.0 {
        return 0.0;
    }
    let (a, len) = (a as usize, ours.len() + (2.0 * slack * sr) as usize);
    let Some(seg) = theirs.get(a..a + len) else { return 0.0 };
    match crate::audio_match::gcc_phat(planner, ours, seg) {
        Some((at, psr)) if (at / sr - slack).abs() <= slack => psr,
        _ => 0.0,
    }
}

/// Windows of `w` seconds every `hop` from `lo` to `hi` (seconds in `ours`), each with its vote:
/// how much better it fits `theirs` at lag `lx` than at `ly` (GCC-PHAT's peak-to-sidelobe ratio
/// at each, `fits`), by the window's middle.
fn votes(planner: &mut realfft::RealFftPlanner<f32>, ours: &[f32], theirs: &[f32], lo: f64, hi: f64, lx: f64, ly: f64, w: f64, hop: f64) -> Vec<(f64, f64)> {
    const SLACK: f64 = 0.03;
    let sr = SOUND_SR;
    let mut out = Vec::new();
    let mut t = lo.max(0.0);
    while t + w <= hi {
        let (a, n) = ((t * sr).round() as usize, (w * sr) as usize);
        if let Some(o) = ours.get(a..a + n) {
            out.push((t + w / 2.0, fits(planner, o, theirs, t, lx, SLACK) - fits(planner, o, theirs, t, ly, SLACK)));
        }
        t += hop;
    }
    out
}

/// A vote counts for a lag when it is this much (peak-to-sidelobe ratio) better than the other;
/// a window between abstains (too quiet, or neither lag fits).
const VOTE: f64 = 1.0;

/// Where the votes change from the old lag to the new: each window counts +1 (old), −1 (new) or 0,
/// and the splits that agree with the most of them (a step fit) form a stretch: the windows in it
/// abstain. Returns its first and last split.
fn switch_over(votes: &[(f64, f64)]) -> Option<(f64, f64)> {
    if votes.len() < 2 {
        return None;
    }
    let score: Vec<f64> = votes.iter().map(|v| if v.1 > VOTE { 1.0 } else if v.1 < -VOTE { -1.0 } else { 0.0 }).collect();
    if !score.contains(&1.0) || !score.contains(&-1.0) {
        return None;
    }
    let total: f64 = score.iter().sum();
    let mut run = 0.0;
    let gains: Vec<(usize, f64)> = (1..votes.len())
        .map(|j| {
            run += score[j - 1];
            (j, run - (total - run))
        })
        .collect();
    let best = gains.iter().map(|g| g.1).fold(f64::MIN, f64::max);
    let at = |j: usize| (votes[j - 1].0 + votes[j].0) / 2.0;
    let first = gains.iter().find(|g| g.1 == best)?.0;
    let last = gains.iter().rev().find(|g| g.1 == best)?.0;
    Some((at(first), at(last)))
}

/// The split near `c` (within a second) where the second of `ours` before it fits `theirs` at lag
/// `lx` and the second after it at `ly` best (the sum of the two peak-to-sidelobe ratios, every
/// 10 ms). With the split, half the width of the splits within one of the best: its error.
fn boundary(planner: &mut realfft::RealFftPlanner<f32>, ours: &[f32], theirs: &[f32], c: f64, lx: f64, ly: f64) -> Option<(f64, f64)> {
    const W: f64 = 1.0;
    const SLACK: f64 = 0.03;
    let sr = SOUND_SR;
    let n = (W * sr) as usize;
    let win = |t: f64| -> Option<&[f32]> { (t >= 0.0).then(|| ours.get((t * sr).round() as usize..(t * sr).round() as usize + n)).flatten() };
    let mut scores = Vec::new();
    let mut s = c - 1.0;
    while s <= c + 1.0 {
        let before = win(s - W).map_or(0.0, |o| fits(planner, o, theirs, s - W, lx, SLACK));
        let after = win(s).map_or(0.0, |o| fits(planner, o, theirs, s, ly, SLACK));
        scores.push((s, before + after));
        s += 0.01;
    }
    let best = scores.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1))?;
    if best.1 < 2.0 * VOTE {
        return None;
    }
    let good: Vec<f64> = scores.iter().filter(|x| x.1 >= best.1 - 1.0).map(|x| x.0).collect();
    let (lo, hi) = (good.iter().copied().fold(f64::MAX, f64::min), good.iter().copied().fold(f64::MIN, f64::max));
    Some((best.0, ((hi - lo) / 2.0).max(0.01)))
}

/// The loudness of `x` (RMS, dB).
fn level_db(x: &[f32]) -> f64 {
    10.0 * (x.iter().map(|&v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).max(1e-12).log10()
}

/// The compare lane against another recording of the same event: where this recording's lag
/// against it jumps between two of the audio match's links (`audio_match::match_pair`, 15 s
/// apart), located by GCC-PHAT windows voting for the old lag or the new one (2 s windows every
/// half second, then 1 s windows every 20 ms round the answer, `switch_over`). Windows where the
/// sound gives too little to tell abstain; a long stretch of them between the old lag and the new
/// is either quiet (the cut is somewhere in it: one mark in its middle, its error half the
/// stretch) or sound the other recording did not hear (a piece from elsewhere: a mark at each
/// end). A jump forward: this recording lacks what the other heard (a cut here); backward: it has a
/// stretch the other lacks (a stretch moved here, or the other was paused).
pub fn compare_lane(key: &str, name: &str, ours: &crate::audio_match::Clip, theirs: &crate::audio_match::Clip) -> Lane {
    let mut planner = realfft::RealFftPlanner::<f32>::new();
    let pair = crate::audio_match::match_pair(theirs, ours, &mut planner);
    let mut hits = pair.hits.clone();
    // A second pass with the chunks half a chunk later: twice as many links to find jumps between.
    let shift = crate::audio_match::CHUNK_S / 2.0;
    let skip = (shift * SOUND_SR) as usize;
    let mut chunks = pair.chunks;
    if ours.audio.len() > skip {
        let later = crate::audio_match::Clip::new(ours.audio[skip..].to_vec());
        let p2 = crate::audio_match::match_pair(theirs, &later, &mut planner);
        chunks += p2.chunks;
        hits.extend(p2.hits.into_iter().map(|h| crate::audio_match::Hit { t_b: h.t_b + shift, ..h }));
    }
    hits.sort_by(|x, y| x.t_b.total_cmp(&y.t_b));
    let sr = SOUND_SR;
    let mut peaks = Vec::new();
    for w in hits.windows(2) {
        let (x, y) = (&w[0], &w[1]);
        let (lx, ly) = (x.t_a - x.t_b, y.t_a - y.t_b);
        let d = ly - lx;
        let err = x.error_s.hypot(y.error_s);
        if d.abs() < crate::audio_match::GAP_MIN_S.max(5.0 * err) {
            continue;
        }
        // Each link is the lag that wins over its 8 s window: the jump may be inside either
        // window, so the search runs from the start of the first to the end of the second.
        let half = crate::audio_match::WIN_S / 2.0;
        let (lo, hi) = (x.t_b - half, y.t_b + half);
        let what = if d > 0.0 { format!("lacks {d:.2} s that {name} heard: a cut here") } else { format!("has {:.2} s that {name} did not hear: a stretch from elsewhere, or {name} paused", -d) };
        let jump = format!("the lag against {name} jumps by {d:+.2} s");
        let coarse = votes(&mut planner, &ours.audio, &theirs.audio, lo, hi, lx, ly, 2.0, 0.5);
        let Some((a, b)) = switch_over(&coarse) else {
            peaks.push(Peak { t: (lo + hi) / 2.0, value: d.abs() as f32, error_s: (hi - lo) / 2.0, note: format!("{jump} somewhere between {lo:.0} and {hi:.0} s (not located): this recording {what}"), lasting: None });
            continue;
        };
        // Close in on one moment with shorter windows.
        let fine = |planner: &mut realfft::RealFftPlanner<f32>, c: f64| -> Option<(f64, f64)> {
            let (p, q) = switch_over(&votes(planner, &ours.audio, &theirs.audio, c - 2.5, c + 2.5, lx, ly, 1.0, 0.02))?;
            Some(((p + q) / 2.0, ((q - p) / 2.0).max(0.02)))
        };
        let mut mark = |t: f64, e: f64, note: String| peaks.push(Peak { t, value: d.abs() as f32, error_s: e, note, lasting: None });
        if b - a <= 2.0 {
            let (t, e) = fine(&mut planner, (a + b) / 2.0).unwrap_or(((a + b) / 2.0, ((b - a) / 2.0).max(0.25)));
            // Near the other recording's start or end, windows over the moment can vote one way
            // only: there the moment is where the second before fits the old lag and the second
            // after the new one. Moving the mark widens its error by half the move.
            let theirs_len = theirs.audio.len() as f64 / sr;
            let near_edge = [lx, ly].iter().any(|lag| t + lag - 2.5 < 0.0 || t + lag + 2.5 > theirs_len);
            let (t, e) = match near_edge.then(|| boundary(&mut planner, &ours.audio, &theirs.audio, t, lx, ly)).flatten() {
                Some((bt, be)) => (bt, be.max((bt - t).abs() / 2.0)),
                None => (t, e),
            };
            mark(t, e, format!("{jump}: this recording {what}"));
            continue;
        }
        // A long stretch that votes for neither: quiet, or unheard by the other.
        let slice = |p: f64, q: f64| &ours.audio[((p.max(0.0) * sr) as usize).min(ours.audio.len())..((q.max(0.0) * sr) as usize).min(ours.audio.len())];
        let inside = level_db(slice(a, b));
        let around = level_db(slice(lo, a)).max(level_db(slice(b, hi)));
        if inside < around - 10.0 {
            mark((a + b) / 2.0, (b - a) / 2.0, format!("{jump} within {:.1} s of quiet ({:.0} dB under the sound round it): this recording {what}", b - a, around - inside));
        } else {
            let gap = format!("{jump} across {:.1} s that do not line up with {name}: it did not hear them", b - a);
            let (pa, pb) = (fine(&mut planner, a).unwrap_or((a, 0.25)), fine(&mut planner, b).unwrap_or((b, 0.25)));
            mark(pa.0, pa.1, format!("{gap}; the old lag ends here"));
            mark(pb.0, pb.1, format!("{gap}; the new lag starts here"));
        }
    }
    let note = format!("{} links with {name} ({chunks} chunks of 15 s tried, every 7.5 s); a mark where the lag jumps", hits.len());
    Lane { key: key.into(), name: format!("compare: {name}"), role: Role::Sound, unit: "s".into(), curve: Vec::new(), peaks, thr: Some(crate::audio_match::GAP_MIN_S as f32), alone: true, note }
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

/// Marks of the same jump from different compared recordings this far apart are one cut, seconds.
pub const COMPARE_JOIN_S: f64 = 3.0;

fn is_compare(l: &Lane) -> bool {
    l.key.starts_with("compare")
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
        // Recordings compared with this one see the same jump (the same seconds missing) a
        // little apart: one cut.
        let same_jump = |cl: &Vec<&Cue>| is_compare(&lanes[c.lane]) && cl.iter().any(|q| is_compare(&lanes[q.lane]) && (q.value - c.value).abs() <= 0.1 && c.t - q.t <= COMPARE_JOIN_S);
        match clusters.last_mut() {
            Some(cl) if c.t - cl.last().unwrap().t <= JOIN_S || same_jump(cl) => cl.push(c),
            _ => clusters.push(vec![c]),
        }
    }
    let err_of = |c: &Cue| lanes[c.lane].peaks.iter().find(|p| p.t == c.t).map_or(frame_s, |p| p.error_s.max(frame_s / 2.0));
    let mut cuts: Vec<Cut> = clusters
        .into_iter()
        .map(|cl| {
            let real: Vec<&&Cue> = cl.iter().filter(|c| !c.flash).collect();
            // The surest cue sets the time (the smallest error), the strongest among equals.
            let best = |pick: &dyn Fn(Role) -> bool| real.iter().filter(|c| pick(lanes[c.lane].role)).max_by(|a, b| err_of(b).total_cmp(&err_of(a)).then(strength(a).total_cmp(&strength(b)))).map(|c| (c.t, err_of(c)));
            let pic = best(&moves_picture);
            // Several recordings compared: the mean of where each put the jump, its error half
            // their spread (each counts; none is dropped).
            let cmp: Vec<&&&Cue> = real.iter().filter(|c| is_compare(&lanes[c.lane])).collect();
            let snd = if cmp.len() >= 2 {
                let mean = cmp.iter().map(|c| c.t).sum::<f64>() / cmp.len() as f64;
                let (lo, hi) = cmp.iter().fold((f64::MAX, f64::MIN), |(lo, hi), c| (lo.min(c.t), hi.max(c.t)));
                let own = cmp.iter().map(|c| err_of(c)).fold(0.0, f64::max);
                Some((mean, ((hi - lo) / 2.0).max(own)))
            } else {
                best(&moves_sound)
            };
            let flash_t = cl.iter().max_by(|a, b| strength(a).total_cmp(&strength(b))).map(|c| c.t).unwrap_or(0.0);
            let (t_pic, error_s) = pic.or(snd).unwrap_or((flash_t, frame_s));
            let t_snd = snd.map_or(t_pic, |s| s.0);
            let error_s = if pic.is_some() { error_s } else { snd.map_or(error_s, |s| s.1) };
            let (lo, hi) = (cl.first().unwrap().t - JOIN_S, cl.last().unwrap().t + JOIN_S);
            // Every lane's marks round it, passing or not: what agrees, and by how much.
            let cues = all.iter().filter(|c| c.t >= lo && c.t <= hi).cloned().collect();
            // A lane that does not cut alone needs another lane's cue with it.
            let lanes_in: std::collections::BTreeSet<usize> = real.iter().map(|c| c.lane).collect();
            let auto = real.iter().any(|c| lanes[c.lane].alone) || lanes_in.len() >= 2;
            let why = if auto {
                String::new()
            } else if real.is_empty() {
                "a flash: the picture comes back".to_string()
            } else {
                format!("{} alone: shown, not cut (switch it on to cut)", lanes[real[0].lane].name)
            };
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
            alone: true,
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
    fn a_lane_that_does_not_cut_alone_needs_another() {
        let mut noise = lane("noise", Role::Sound, Some(15.0), &[(10.0, 20.0), (30.0, 20.0)]);
        noise.alone = false;
        let lanes = [lane("picture", Role::Picture, Some(10.0), &[(10.2, 30.0)]), noise];
        let cuts = find_cuts(&lanes, &[None, None], &[], 0.04);
        let on: Vec<(f64, bool)> = cuts.iter().map(|c| (c.t_pic, c.on())).collect();
        assert_eq!(on, [(10.2, true), (30.0, false)]);
        assert!(cuts[1].why.contains("alone"), "{}", cuts[1].why);
        // The picture sets the picture time, the noise the sound time.
        assert_eq!(cuts[0].t_snd, 10.0);
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
    /// Gaussian noise from a fixed seed.
    fn noise(n: usize, sigma: f32, seed: &mut u64) -> Vec<f32> {
        let mut u = || {
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((*seed >> 11) as f64 / (1u64 << 53) as f64).max(1e-12)
        };
        (0..n).map(|_| (sigma as f64 * (-2.0 * u().ln()).sqrt() * (2.0 * std::f64::consts::PI * u()).cos()) as f32).collect()
    }

    /// Clicks and short bursts, louder low than high as natural sounds are (a one-pole low-pass):
    /// high kurtosis in every band, the high bands quiet enough for noise to show.
    fn clicks(n: usize, seed: &mut u64) -> Vec<f32> {
        let raw = raw_clicks(n, seed);
        let mut y = 0.0f32;
        raw.iter().map(|&v| {
            y = 0.95 * y + 0.2 * v;
            y
        }).collect()
    }

    fn raw_clicks(n: usize, seed: &mut u64) -> Vec<f32> {
        let mut x = vec![0.0f32; n];
        let mut k = 0;
        while k < n {
            let burst = noise(40, 0.3, seed);
            for (j, b) in burst.iter().enumerate() {
                if let Some(v) = x.get_mut(k + j) {
                    *v += b * (1.0 - j as f32 / 40.0);
                }
            }
            k += 211 + (*seed % 300) as usize;
        }
        x
    }

    #[test]
    fn noise_under_a_sound_is_measured_by_its_kurtosis() {
        let mut seed = 7;
        let sr = SOUND_SR as usize;
        let sound = clicks(sr, &mut seed);
        let mut planner = realfft::RealFftPlanner::<f32>::new();
        let mut at = |sigma: f32, seed: &mut u64| {
            let x: Vec<f32> = sound.iter().zip(noise(sr, sigma, seed)).map(|(a, b)| a + b).collect();
            noise_db(&x, &mut planner).unwrap()
        };
        let (quiet, loud) = (at(0.003, &mut seed), at(0.0095, &mut seed));
        assert!(((loud - quiet) - 10.0).abs() < 2.5, "quiet {quiet:.1} dB, loud {loud:.1} dB");
    }

    #[test]
    fn the_sound_lanes_mark_a_splice() {
        let mut seed = 11;
        let sr = SOUND_SR as usize;
        // 6 s of clicks over quiet noise, then 6 s of the same over noise 20 dB louder.
        let sound = clicks(12 * sr, &mut seed);
        let mut x = sound.clone();
        for (k, v) in noise(12 * sr, 0.001, &mut seed).into_iter().enumerate() {
            x[k] += if k < 6 * sr { v } else { v * 10.0 };
        }
        let lane = noise_lane(&x);
        let over: Vec<f64> = lane.peaks.iter().filter(|p| p.value >= NOISE_THR).map(|p| p.t).collect();
        assert_eq!(over.len(), 1, "{:?}", lane.peaks.iter().map(|p| (p.t, p.value)).collect::<Vec<_>>());
        assert!((over[0] - 6.0).abs() <= 0.25, "{over:?}");
        // A tone that drops by 20 dB at 5 s: the level lane.
        let tone: Vec<f32> = (0..10 * sr).map(|k| (if k < 5 * sr { 0.1 } else { 0.01 }) * (k as f32 * 0.3).sin()).collect();
        let lane = level_lane(&tone);
        let over: Vec<(f64, f32)> = lane.peaks.iter().filter(|p| p.value >= LEVEL_THR).map(|p| (p.t, p.value)).collect();
        assert_eq!(over.len(), 1, "{over:?}");
        assert!((over[0].0 - 5.0).abs() <= 0.05 && (over[0].1 - 20.0).abs() < 1.0, "{over:?}");
    }
}
