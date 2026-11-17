//! Kept results (docs/run-queue/plan.md, stage 3): what a job worked out, on disk in the project's
//! `cache\` folder, one file per result, named by its kept key. Opening the project loads them;
//! a job whose kept key has a file is loaded from it instead of worked out again.
//!
//! A kept key is everything the result depends on, in words that stay the same between sessions:
//! the job's kind and method version (raised when its method changes what it works out), the
//! settings that change what it works out, and what it reads: a file by its size and print (as
//! the recordings list has them), a web video by its link. It is hashed (FNV-1a, 64 bit) into the
//! file's name. Deleting the folder only costs the time to work things out again.
//!
//! What is kept: the SOURCER's decoded sound (16 kHz, 16 bit) with what it found out about the
//! file; the Cut detector's scan; the Audio match's result; the ENF's hum track and its dating.
//! Planes and plugins are not kept: they read their SOURCER's sound again (from its kept sound,
//! which is quick). A live source is never kept.

use crate::cut_detector_node::Scan;
use crate::runtime::NodeState;
use corroborate::cuts::{self, Lane, Peak, Thumb};
use corroborate::timebase::Anchor;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// Each kind's method version: raise it when the method changes what it works out (the build
/// number would throw everything away at every build).
pub const SOURCER_V: u32 = 1;
pub const CUT_V: u32 = 1;
pub const MATCH_V: u32 = 1;
pub const ENF_V: u32 = 1;

/// The first bytes of every kept file.
const MAGIC: &[u8; 8] = b"CORRKEPT";

/// FNV-1a, 64 bit.
pub fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The kept file's name for a result of `kind` on `what` (its kept key in words).
pub fn name(kind: &str, what: &str) -> String {
    format!("{kind}-{:016x}.bin", fnv(what))
}

/// A file as a kept key sees it: its size and print, or None when it cannot be read. Read once
/// per size and modified time.
pub fn print_of(path: &str) -> Option<String> {
    static SEEN: Mutex<Option<HashMap<String, ((u64, Option<std::time::SystemTime>), String)>>> = Mutex::new(None);
    let m = std::fs::metadata(path).ok()?;
    let stat = (m.len(), m.modified().ok());
    if let Some((s, p)) = SEEN.lock().unwrap().get_or_insert_with(HashMap::new).get(path) {
        if *s == stat {
            return Some(p.clone());
        }
    }
    let p = format!("{}:{}", stat.0, crate::recordings::print(Path::new(path))?);
    SEEN.lock().unwrap().get_or_insert_with(HashMap::new).insert(path.to_string(), (stat, p.clone()));
    Some(p)
}

/// Writes `bytes` (a kept result of `kind`) to `path`, through a temporary file so a reader never
/// sees half of it.
pub fn write(path: &Path, kind: &str, body: Vec<u8>) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut w = W(Vec::with_capacity(body.len() + 32));
    w.0.extend_from_slice(MAGIC);
    kind.to_string().put(&mut w);
    w.0.extend_from_slice(&body);
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, &w.0)?;
    std::fs::rename(&tmp, path)
}

/// Reads kept file `path` of `kind`: its body after the header.
pub fn read(path: &Path, kind: &str) -> Option<Vec<u8>> {
    let b = std::fs::read(path).ok()?;
    let mut r = R { b: &b, p: 0 };
    if r.take(MAGIC.len())? != MAGIC || String::get(&mut r)? != kind {
        return None;
    }
    Some(b[r.p..].to_vec())
}

/// How many bytes are kept in `dir`.
pub fn size(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map(|d| d.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum()).unwrap_or(0)
}

/// Forgets everything kept in `dir` (only the files kept results are written as).
pub fn forget_all(dir: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "bin" || x == "part") {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// "85.2 MB".
pub fn size_text(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} kB", n as f64 / (1u64 << 10) as f64),
        n => format!("{n} bytes"),
    }
}

// ---- Writing and reading the parts of a result (little-endian, lengths before lists). ----

pub struct W(pub Vec<u8>);
pub struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        R { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
}

/// A part of a kept result.
pub trait Part: Sized {
    fn put(&self, w: &mut W);
    fn get(r: &mut R) -> Option<Self>;
}

macro_rules! num {
    ($($t:ty),*) => {$(
        impl Part for $t {
            fn put(&self, w: &mut W) {
                w.0.extend_from_slice(&self.to_le_bytes());
            }
            fn get(r: &mut R) -> Option<Self> {
                Some(<$t>::from_le_bytes(r.take(std::mem::size_of::<$t>())?.try_into().ok()?))
            }
        }
    )*};
}
num!(u8, u32, u64, i64, f32, f64);

impl Part for usize {
    fn put(&self, w: &mut W) {
        (*self as u64).put(w)
    }
    fn get(r: &mut R) -> Option<Self> {
        u64::get(r).map(|v| v as usize)
    }
}

impl Part for bool {
    fn put(&self, w: &mut W) {
        (*self as u8).put(w)
    }
    fn get(r: &mut R) -> Option<Self> {
        Some(u8::get(r)? != 0)
    }
}

impl Part for String {
    fn put(&self, w: &mut W) {
        self.len().put(w);
        w.0.extend_from_slice(self.as_bytes());
    }
    fn get(r: &mut R) -> Option<Self> {
        let n = usize::get(r)?;
        String::from_utf8(r.take(n)?.to_vec()).ok()
    }
}

impl<T: Part> Part for Vec<T> {
    fn put(&self, w: &mut W) {
        self.len().put(w);
        for x in self {
            x.put(w);
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        let n = usize::get(r)?;
        // A broken file cannot ask for more than it holds.
        if n > r.b.len() - r.p {
            return None;
        }
        (0..n).map(|_| T::get(r)).collect()
    }
}

impl<T: Part> Part for Option<T> {
    fn put(&self, w: &mut W) {
        match self {
            Some(x) => {
                true.put(w);
                x.put(w);
            }
            None => false.put(w),
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        Some(if bool::get(r)? { Some(T::get(r)?) } else { None })
    }
}

impl<A: Part, B: Part> Part for (A, B) {
    fn put(&self, w: &mut W) {
        self.0.put(w);
        self.1.put(w);
    }
    fn get(r: &mut R) -> Option<Self> {
        Some((A::get(r)?, B::get(r)?))
    }
}

impl<T: Part, E: Part> Part for Result<T, E> {
    fn put(&self, w: &mut W) {
        match self {
            Ok(x) => {
                true.put(w);
                x.put(w);
            }
            Err(e) => {
                false.put(w);
                e.put(w);
            }
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        Some(if bool::get(r)? { Ok(T::get(r)?) } else { Err(E::get(r)?) })
    }
}

/// Writes and reads a struct field by field.
macro_rules! fields {
    ($t:path { $($f:ident),* }) => {
        impl Part for $t {
            fn put(&self, w: &mut W) {
                $(self.$f.put(w);)*
            }
            fn get(r: &mut R) -> Option<Self> {
                Some(Self { $($f: Part::get(r)?),* })
            }
        }
    };
}

fields!(Anchor { utc0, error_s, source });
fields!(Peak { t, value, error_s, note, lasting });
fields!(Thumb { t, rgb });
fields!(Lane { key, name, role, unit, curve, peaks, thr, alone, note });
fields!(corroborate::clock::Link { a, t_a, b, t_b, error_s, from, enabled });
fields!(corroborate::data::Event { clock, t_start, t_end, label, detail, error_s });
fields!(crate::audio_match_node::Found { links, links_json, same, speed, gaps, quality, drift });
fields!(corroborate::modules::enf::EnfPoint { t, hz, nominal, harmonic, snr_db, err_hz, used, ok, reject });
fields!(corroborate::modules::enf::EnfSegment { id, nominal, harmonic, t_start, t_end, mean_hz, min_hz, max_hz, mean_snr_db, n_points });
fields!(corroborate::modules::enf::log_match::Candidate { grid, start, rms_hz, offset_hz });
fields!(corroborate::modules::enf::log_match::Dated { claimed, best, runner_up, searched, readings, confirmed, error_s });

impl Part for cuts::Role {
    fn put(&self, w: &mut W) {
        (match self {
            cuts::Role::Picture => 0u8,
            cuts::Role::Sound => 1,
            cuts::Role::Both => 2,
            cuts::Role::Events => 3,
        })
        .put(w)
    }
    fn get(r: &mut R) -> Option<Self> {
        Some(match u8::get(r)? {
            0 => cuts::Role::Picture,
            1 => cuts::Role::Sound,
            2 => cuts::Role::Both,
            3 => cuts::Role::Events,
            _ => return None,
        })
    }
}

impl Part for corroborate::modules::enf::Reject {
    fn put(&self, w: &mut W) {
        use corroborate::modules::enf::Reject::*;
        (match self {
            Weak => 0u8,
            Undecided => 1,
            Slope => 2,
            Section => 3,
        })
        .put(w)
    }
    fn get(r: &mut R) -> Option<Self> {
        use corroborate::modules::enf::Reject::*;
        Some(match u8::get(r)? {
            0 => Weak,
            1 => Undecided,
            2 => Slope,
            3 => Section,
            _ => return None,
        })
    }
}

impl Part for crate::audio_match_node::Stage {
    fn put(&self, w: &mut W) {
        use crate::audio_match_node::Stage::*;
        match self {
            Waiting(s) => {
                0u8.put(w);
                s.put(w)
            }
            Fingerprinting => 1u8.put(w),
            Ready(n) => {
                2u8.put(w);
                n.put(w)
            }
            Failed(e) => {
                3u8.put(w);
                e.put(w)
            }
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        use crate::audio_match_node::Stage::*;
        Some(match u8::get(r)? {
            0 => Waiting(f64::get(r)?),
            1 => Fingerprinting,
            2 => Ready(usize::get(r)?),
            3 => Failed(String::get(r)?),
            _ => return None,
        })
    }
}

// ---- Each job's kept result. ----

/// A SOURCER's: the file it opened (a web video's downloaded copy), its time anchor, ffprobe's
/// view of it, whether it has pictures, and its sound.
pub struct Sound {
    pub opened: String,
    pub anchor: Option<Anchor>,
    pub meta: Vec<(String, String)>,
    pub video: bool,
    pub audio: Vec<i16>,
}

impl Part for Sound {
    fn put(&self, w: &mut W) {
        self.opened.put(w);
        self.anchor.put(w);
        self.meta.put(w);
        self.video.put(w);
        // The sound as it is in memory, without a length per sample.
        self.audio.len().put(w);
        w.0.reserve(self.audio.len() * 2);
        for v in &self.audio {
            w.0.extend_from_slice(&v.to_le_bytes());
        }
    }
    fn get(r: &mut R) -> Option<Self> {
        let (opened, anchor, meta, video) = (Part::get(r)?, Part::get(r)?, Part::get(r)?, Part::get(r)?);
        let n = usize::get(r)?;
        let b = r.take(n.checked_mul(2)?)?;
        let audio = b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        Some(Sound { opened, anchor, meta, video, audio })
    }
}

/// A Cut detector's: its scan and the line its node shows.
pub fn put_scan(scan: &Scan, status: &str) -> Vec<u8> {
    let mut w = W(Vec::new());
    (scan.path.clone(), (scan.len, scan.frame_s)).put(&mut w);
    scan.lanes.put(&mut w);
    scan.thumbs.put(&mut w);
    scan.notes.put(&mut w);
    status.to_string().put(&mut w);
    w.0
}

pub fn get_scan(b: &[u8]) -> Option<(Scan, String)> {
    let mut r = R::new(b);
    let (path, (len, frame_s)) = <(String, (f64, f64))>::get(&mut r)?;
    let scan = Scan { path, len, frame_s, lanes: Part::get(&mut r)?, thumbs: Part::get(&mut r)?, notes: Part::get(&mut r)? };
    Some((scan, String::get(&mut r)?))
}

/// An Audio match's: what it found, each recording's stage, its line.
pub fn put_match(s: &NodeState) -> Option<Vec<u8>> {
    let mut w = W(Vec::new());
    s.found.as_deref()?.put(&mut w);
    s.matching.put(&mut w);
    s.status.put(&mut w);
    Some(w.0)
}

pub fn get_match(b: &[u8]) -> Option<(crate::audio_match_node::Found, Vec<crate::audio_match_node::Stage>, String)> {
    let mut r = R::new(b);
    Some((Part::get(&mut r)?, Part::get(&mut r)?, Part::get(&mut r)?))
}

/// An ENF's: its hum track (readings, runs, the grid's frequency), the time anchor and length of
/// the sound it read, what it read, the dating against the grid logs, its line.
pub fn put_enf(s: &NodeState) -> Vec<u8> {
    let mut w = W(Vec::new());
    s.enf.points.put(&mut w);
    s.enf.segments.put(&mut w);
    s.enf.nominal.put(&mut w);
    s.enf.dated.put(&mut w);
    s.anchor.put(&mut w);
    s.audio_s.put(&mut w);
    (s.source_label.clone(), s.source_key.clone()).put(&mut w);
    s.status.put(&mut w);
    w.0
}

/// Fills `s` from an ENF's kept result (the area is worked out again from the grid's frequency:
/// `enf_node::restore`).
pub fn get_enf(b: &[u8], s: &mut NodeState) -> Option<()> {
    let mut r = R::new(b);
    s.enf.points = Part::get(&mut r)?;
    s.enf.segments = Part::get(&mut r)?;
    s.enf.nominal = Part::get(&mut r)?;
    s.enf.dated = Part::get(&mut r)?;
    s.anchor = Part::get(&mut r)?;
    s.audio_s = Part::get(&mut r)?;
    (s.source_label, s.source_key) = Part::get(&mut r)?;
    s.status = Part::get(&mut r)?;
    Some(())
}

/// Run ids for states loaded from kept results: counted down from the top, so they are never a
/// run's (each kind counts its runs up from 1).
pub fn run_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
    NEXT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed)
}

