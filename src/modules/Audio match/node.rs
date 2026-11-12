//! The Audio match node in the workbench: the time module. Each row takes one recording's audio and
//! the time its metadata claims, and gives a better start and end time on its outputs.
//!
//! Once the graph runs it takes the audio each wired source decodes (16 kHz, kept by the SOURCER) as
//! soon as that source is done, brings it to 8 kHz (the fingerprints' rate, `audio_match::halve`), and
//! fingerprints it, a core per recording; a source that gave no audio has its file decoded here
//! instead. Then it matches every pair (`corroborate::audio_match`) on its own thread and keeps
//! the links (with both clocks named) with what else it found (`Found`: same recording, speed
//! change, gaps, the quality) in the node's state; the links go to the result file, not to a wire. Files only for
//! now: a stream or microphone wired in is named in the status and left out. Compiled into the
//! `corroborate` program.
//!
//! On the node, one row per recording: blue is where its time input (audio start time) puts it,
//! amber where audio puts it. Recordings linked by audio are a synced group, which exists only here.
//! Each group is fitted from this node's links alone (`fit_groups`): every recording's offset and
//! drift in the group's average clock, one vote per phone (rows with the same phone id share a
//! drift). A speed reference row's clock, or real time from the anchor row's wired end, can run
//! true instead. Audio says only how a group's recordings lie against each other, so until the
//! group has an anchor it sits where its stamps put it (one vote per phone, weighted by ±), and
//! the person may drag it (saved with the project, logged); those times come out marked "?". A
//! person anchors it on one row's time (the audio's start snaps to it, every other row follows
//! along the fit) or on the average of its stamps (its number tab's button). Each row gives its
//! audio start time and audio end time: start + length at the measured drift. A row with no time
//! metadata that audio linked to nothing has nothing to place it by: it is greyed out and gives
//! nothing. Nothing is ever hidden or left out for being far off: a time 200 years away stays in
//! view, and counts in the average, until a person corrects it in a Time node.

use super::*;
use corroborate::audio_match::{self, Clip, Placed};
use corroborate::clock::Link;
use corroborate::data::{Data, Event};
use corroborate::sync::Fit;
use corroborate::module_manifest::{PortLabel, PortLook, Side};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::thread;

/// How far one wired recording has got in a run.
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// Waiting for its source: seconds decoded so far.
    Waiting(f64),
    Fingerprinting,
    Ready(usize),
    Failed(String),
}

/// One wired recording: its source's label, clock and file, and the source's state with the run
/// it was on before this run started (its audio is this run's once its run differs and is done).
pub struct Wired {
    pub label: String,
    pub clock: String,
    pub path: String,
    pub source: Option<(Shared, u64)>,
    /// A piece of the file (from a Cut detector): its audio from and to, seconds in the file.
    pub range: Option<(f64, f64)>,
}

/// The part of `audio` (at the match's rate) a piece covers, or all of it.
fn piece_of(audio: Vec<f32>, range: Option<(f64, f64)>) -> Vec<f32> {
    match range {
        Some((a, b)) => {
            let sr = audio_match::SR as f64;
            let (i, j) = (((a * sr) as usize).min(audio.len()), ((b * sr) as usize).min(audio.len()));
            audio[i..j.max(i)].to_vec()
        }
        None => audio,
    }
}

static RUNS: AtomicU64 = AtomicU64::new(1);

/// One recording's row. Times are seconds since 1970 when `MatchView::real`.
#[derive(Clone, Debug, Default)]
pub struct Row {
    /// Its number: the row of ports `audio n` and `time n`.
    pub n: usize,
    pub name: String,
    /// The source's stable id and clock, when audio is wired in.
    pub uid: Option<String>,
    pub clock: Option<String>,
    pub file: bool,
    /// A wire goes into its audio or time port (else: the free row at the end).
    pub wired: bool,
    /// The media's length, seconds.
    pub len: f64,
    /// What its time input says: the moment, its error and where it comes from.
    pub meta: Option<f64>,
    pub meta_err: f64,
    pub meta_source: String,
    /// The time input is wired to a Time node that has an error, and the error.
    pub time_err: Option<String>,
    pub time_wired: bool,
    /// Its optional end input (audio end time) is wired, and what it says.
    pub end_wired: bool,
    pub end_in: Option<f64>,
    /// Where audio puts its start (shown after a match), and how far from there the time input is.
    pub start: Option<f64>,
    pub off: Option<f64>,
    /// Where audio puts its end: the start plus its length at the speed the fit measured.
    pub end: Option<f64>,
    /// The errors (1 sigma) of `start` and `end`, seconds.
    pub start_err: f64,
    pub end_err: f64,
    /// How fast its clock runs (its drift), ppm (negative: slow), and the error: against its
    /// group's average clock, or the speed reference's, or real time (`Group::drift`). None for a
    /// row audio did not link.
    pub speed: Option<(f64, f64)>,
    /// Its phone id, as a person typed or accepted it: rows with the same text are one phone (one
    /// drift, one vote). Empty: its own phone.
    pub phone: String,
    /// What its metadata says the device is (make, model, serial): only a suggestion.
    pub phone_hint: String,
    /// Its speed reference button is on: its clock runs true.
    pub speed_ref: bool,
    /// This node's links on its clock, and how far they sit from the fit (root mean square), s.
    pub links: usize,
    pub leftover: f64,
    /// What the match found on it: same recording, speed change, gaps.
    pub marks: Vec<Mark>,
    /// In `MatchView::groups`.
    pub group: usize,
    pub stage: Option<Stage>,
    /// The offset is about the recording's own length.
    pub hint: bool,
    /// It was in an anchored group before this run and is not any more.
    pub lost: bool,
}

/// Recordings audio links together, or one alone.
#[derive(Clone, Debug, Default)]
pub struct Group {
    /// What names it while it lives: the smallest clock id in it.
    pub key: String,
    /// Shown on its border; 0: one recording alone, which has no border.
    pub number: usize,
    pub rows: Vec<usize>,
    /// The row holding its anchor.
    pub anchor: Option<usize>,
    /// Unanchored, and dragged by hand away from where its metadata puts it.
    pub moved: bool,
    /// The row whose clock the fit's errors are measured against: the anchor's, else the speed
    /// reference's, else the longest recording's.
    pub reference: Option<usize>,
    /// What its drifts are measured against.
    pub drift: DriftFrom,
    /// Anchored on the average of its stamps (its group tab's button): where its reference
    /// recording starts, the error, and how many phones voted.
    pub average: Option<Average>,
    /// Where its stamps put it, one vote per phone (where it sits unanchored, before a drag).
    pub stamps: Option<Average>,
}

/// What a group's drifts are measured against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DriftFrom {
    /// Its average clock: one vote per phone, weighted by how well each drift is measured.
    #[default]
    Average,
    /// Row k's clock runs true (its speed reference button).
    File(usize),
    /// Real time: the anchor row k's wired end.
    AnchorEnd(usize),
}

/// Where a group's stamps put it: each phone's vote for where its reference recording starts,
/// weighted by its ±; the error; how many phones voted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Average {
    pub at: f64,
    pub error: f64,
    pub phones: usize,
}

/// The average of the stamps' votes `(phone, where it puts the group, ±)`: one vote per phone
/// (its stamps' mean, weighted by their ±), the phones' votes weighted by their ±. Nothing is
/// dropped for being far off: a stamp 12 years off counts like any other, and shows on the
/// timeline. The error is the larger of the votes' weighted spread ÷ √phones and what their ±
/// give (the spread of one phone's vote is 0).
pub fn stamp_average(votes: &[(String, f64, f64)]) -> Option<Average> {
    let w = |e: f64| 1.0 / e.max(1e-3).powi(2);
    let mut phones: BTreeMap<&str, (f64, f64, usize)> = BTreeMap::new();
    for (p, v, e) in votes {
        let s = phones.entry(p.as_str()).or_default();
        s.0 += v * w(*e);
        s.1 += w(*e);
        s.2 += 1;
    }
    // Each phone: its stamps' weighted mean, and their typical ± (one phone's stamps share its
    // clock's error, so more of them do not make it surer).
    let each: Vec<(f64, f64)> = phones.values().map(|&(sv, sw, n)| (sv / sw, (n as f64 / sw).sqrt())).collect();
    let sw: f64 = each.iter().map(|&(_, e)| w(e)).sum();
    if each.is_empty() || sw <= 0.0 {
        return None;
    }
    let at = each.iter().map(|&(v, e)| v * w(e)).sum::<f64>() / sw;
    let spread = (each.iter().map(|&(v, e)| w(e) * (v - at).powi(2)).sum::<f64>() / sw).sqrt();
    let n = each.len();
    Some(Average { at, error: (spread / (n as f64).sqrt()).max(sw.recip().sqrt()), phones: n })
}

/// What the match found on a recording besides its links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Same,
    Speed,
    Gap,
}

/// A finding on a row: drawn in its Status cell, the words on hover.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    pub text: String,
}

/// What a match found, kept with the node's state once it ends.
#[derive(Clone, Debug, Default)]
pub struct Found {
    /// Every link, both clocks named, and the same as JSON (with each link's source) for the
    /// result file.
    pub links: Vec<Link>,
    pub links_json: String,
    pub same: Vec<Event>,
    pub speed: Vec<Event>,
    pub gaps: Vec<Event>,
    /// For the result file: the quality type's JSON, and each pair's own clock drift.
    pub quality: String,
    pub drift: String,
}

/// One fit of a node's links: each clock in its group's average clock.
#[derive(Debug)]
pub struct Fits {
    /// Moment t on a clock is `at(t)` seconds of its group's average clock from the start of the
    /// group's reference recording (`clock::fit_against`).
    pub clocks: BTreeMap<String, Fit>,
    /// Each link's leftover: how far apart its two moments land on the fit, seconds.
    pub leftover: Vec<f64>,
}

/// Fits `links` (`clock::fit_against`): each group of linked clocks in its average clock,
/// measured from the start of the clock in `refs` it holds; clocks of one `phone` share a drift.
pub fn fit_groups(links: &[Link], refs: &BTreeSet<String>, phone: &BTreeMap<String, String>) -> Fits {
    let (clocks, leftover) = corroborate::clock::fit_against(links, refs, phone);
    Fits { clocks, leftover }
}

/// How fast a clock fitted `f` runs: its seconds per real second, minus 1, in ppm, and the error;
/// `k` is real seconds per second of the fit's clock.
fn speed_ppm(f: &Fit, k: f64) -> (f64, f64) {
    ((1.0 / (k * (1.0 + f.rate)) - 1.0) * 1e6, f.var_rate.sqrt() * 1e6)
}

/// Where a recording fitted `f` starts and ends: its moments 0 and `len` on the fit's clock, into
/// real time by where the fit's 0 is (`at`) and its speed `k` (real seconds per second).
pub fn place(f: &Fit, len: f64, at: f64, k: f64) -> (f64, f64) {
    (at + k * f.at(0.0), at + k * f.at(len))
}

/// A clock with no links: its own time.
const SAME_CLOCK: Fit = Fit { offset: 0.0, rate: 0.0, center: 0.0, var_offset: 0.0, var_rate: 0.0, cov: 0.0 };

/// Seconds an hour a drift of `ppm` gains (+) or loses (−).
pub fn s_per_h(ppm: f64) -> f64 {
    ppm * 3600e-6
}

/// A drift in seconds an hour: `-0.90`, `+12.6`, `0`.
pub fn drift_text(ppm: f64) -> String {
    let s = s_per_h(ppm);
    if s.abs() < 0.005 {
        "0".to_string()
    } else if s.abs() < 10.0 {
        format!("{s:+.2}")
    } else {
        format!("{s:+.1}")
    }
}

/// A drift's exact scale, in words: the clock's speed (its seconds per real second), and what a
/// length on it is multiplied by to give real seconds.
pub fn scale_words(ppm: f64) -> String {
    let speed = 1.0 + ppm * 1e-6;
    format!("clock speed ×{speed:.7} (file seconds per real second); real length = file length ×{:.7}", 1.0 / speed)
}

/// What a group's drifts are against, in words.
fn drift_against(v: &MatchView, g: &Group) -> String {
    match g.drift {
        DriftFrom::Average => "the group's average clock".to_string(),
        DriftFrom::File(k) => format!("the clock of {} (speed reference)", v.rows[k].name),
        DriftFrom::AnchorEnd(k) => format!("real time ({}'s wired end)", v.rows[k].name),
    }
}

/// The fit, in words, for a row's time: "; fitted on 31 links, leftover 4.2 ms, drift …", or
/// nothing for a row audio did not link.
fn fit_words(v: &MatchView, r: &Row) -> String {
    if !v.linked(r) || r.links == 0 {
        return String::new();
    }
    let g = &v.groups[r.group];
    let speed = match r.speed {
        Some((p, e)) => format!(", drift {} ± {:.2} s/h against {}", drift_text(p), s_per_h(e), drift_against(v, g)),
        None => String::new(),
    };
    format!("; fitted on {} links, leftover {:.1} ms{speed}", r.links, r.leftover * 1000.0)
}

/// `+47 ms`, `-1.30 s`.
pub(crate) fn ms_text(d: f64) -> String {
    if d.abs() < 0.0005 {
        "0 ms".to_string()
    } else if d.abs() < 1.0 {
        format!("{:+.0} ms", d * 1000.0)
    } else {
        format!("{d:+.2} s")
    }
}

/// What a node's rows are this frame.
#[derive(Debug, Default)]
pub struct MatchView {
    pub rows: Vec<Row>,
    pub groups: Vec<Group>,
    pub done: bool,
    pub running: bool,
    /// Its axis is real time: some row has a time.
    pub real: bool,
    /// The stamps of the group the hint is about agree better read as ends: how far they disagree
    /// as starts and as ends, by group.
    pub spreads: Vec<Option<(f64, f64)>>,
    /// How far the links sit from their groups' fits, the median, seconds.
    pub leftover: Option<f64>,
}

impl MatchView {
    /// Its group is anchored: on a row's time, or on the average of its stamps.
    pub fn anchored(&self, r: &Row) -> bool {
        self.groups.get(r.group).is_some_and(|g| g.anchor.is_some() || g.average.is_some())
    }

    /// Audio linked it to another recording, and placed it.
    pub fn linked(&self, r: &Row) -> bool {
        self.done && r.start.is_some() && self.groups.get(r.group).is_some_and(|g| g.rows.iter().filter(|&&k| self.rows[k].clock.is_some()).count() >= 2)
    }
}

/// How close to the recording's own length an offset must be to hint at it, seconds (5 s for now).
const HINT_S: f64 = 5.0;

/// A node's saved rows: which row holds an anchor is kept by the recording's id.
fn anchor_key(uid: &str) -> String {
    format!("anchor {uid}")
}

/// A node's saved rows: how far an unanchored group was dragged, kept by the group's key.
fn shift_key(group: &str) -> String {
    format!("shift {group}")
}

/// A node's saved rows: a group anchored on the average of its stamps, kept by the group's key.
fn average_key(group: &str) -> String {
    format!("average {group}")
}

/// A node's saved rows: a recording's phone id.
fn phone_key(uid: &str) -> String {
    format!("phone {uid}")
}

/// A node's saved rows: the recording whose clock runs true, kept by its id.
fn speed_ref_key(uid: &str) -> String {
    format!("speed reference {uid}")
}

/// A saved row's value.
fn kept<'a>(kind: &'a Kind, key: &str) -> Option<&'a str> {
    kind.rows().iter().find(|(q, _)| q == key).map(|(_, v)| v.as_str())
}

/// What a source's metadata says its device is: make (or manufacturer), model, serial, from its
/// tags (`com.apple.quicktime.make`, `com.android.model`, …). Only ever a suggestion.
pub fn phone_hint(meta: &[(String, String)]) -> String {
    let tag = |names: &[&str]| {
        meta.iter().filter(|(k, _)| k.starts_with("format.tags.")).find_map(|(k, v)| {
            let last = k.rsplit(['.', ':']).next().unwrap_or(k).to_ascii_lowercase();
            names.iter().any(|n| last == *n).then(|| v.trim().to_string()).filter(|v| !v.is_empty())
        })
    };
    let mut parts: Vec<String> = Vec::new();
    for names in [&["make", "manufacturer"][..], &["model"], &["serial", "serialnumber", "serial_number"]] {
        if let Some(v) = tag(names) {
            // A model often repeats the make ("Apple" "Apple iPhone 12").
            if let Some(last) = parts.last_mut().filter(|l| v.to_lowercase().starts_with(&l.to_lowercase())) {
                *last = v;
            } else {
                parts.push(v);
            }
        }
    }
    parts.join(" ")
}

/// A phone id's colour: the same text, the same colour.
pub fn phone_color(id: &str) -> Color32 {
    let h = id.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    hsl((h % 360) as f32, 0.55, 0.64)
}

/// Sets saved row `k` to `val`, adding it when missing.
fn set_row(rows: &mut Vec<(String, String)>, k: &str, val: String) {
    match rows.iter_mut().find(|(q, _)| q == k) {
        Some((_, x)) => *x = val,
        None => rows.push((k.to_string(), val)),
    }
}

/// How far group `group` of a node of kind `kind` was dragged, seconds.
fn saved_shift(kind: &Kind, group: &str) -> f64 {
    let k = shift_key(group);
    kind.rows().iter().find(|(q, _)| *q == k).and_then(|(_, v)| v.parse().ok()).unwrap_or(0.0)
}

/// How many inputs make a row: audio, audio start time ("time n"), audio end time ("end in n").
const PER_ROW: usize = 3;

/// What a port is called on the node and in its hover; the saved names stay as they were, so old
/// wires hold.
pub(crate) fn port_title(name: &str) -> Option<String> {
    let (base, n) = name.rsplit_once(' ')?;
    n.parse::<usize>().ok()?;
    let t = match base {
        "audio" => "audio",
        "time" | "synced" => "audio start time",
        "end in" => "audio end time (optional)",
        "end" => "audio end time",
        _ => return None,
    };
    Some(format!("{t}, row {n}"))
}

/// The middle of a recording.
fn mid(t: f64, len: f64) -> f64 {
    t + len / 2.0
}

/// Which phone a row is: its phone id, else its own clock.
fn phone_of(r: &Row) -> String {
    if r.phone.is_empty() { format!("\u{1}{}", r.clock.as_deref().or(r.uid.as_deref()).unwrap_or_default()) } else { r.phone.clone() }
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// Its file's name, when the node still has the module's name.
fn row_name(label: &str, path: Option<&str>) -> String {
    match path {
        Some(p) if label == "SOURCER" => p.rsplit(['\\', '/']).next().unwrap_or(p).to_string(),
        _ => label.to_string(),
    }
}

impl App {
    /// What node `i`'s rows are now. Worked out once a frame (the outputs, the node and the clock
    /// evidence all ask).
    pub(crate) fn match_view(&self, i: NodeId) -> Rc<MatchView> {
        #[cfg(not(test))]
        if let Some(v) = self.match_ui.cache.borrow().get(&i) {
            return v.clone();
        }
        let v = Rc::new(self.match_view_now(i));
        #[cfg(not(test))]
        self.match_ui.cache.borrow_mut().insert(i, v.clone());
        v
    }

    /// The fit of node `i`'s links against references `refs`, clocks of one phone sharing a
    /// drift, worked out once per run, choice of references and phones (`run`: the state's run
    /// and outbox revision).
    fn match_fits(&self, i: NodeId, run: (u64, u64), links: &[Link], refs: BTreeSet<String>, phone: BTreeMap<String, String>) -> Rc<Fits> {
        let key = (run.0, run.1, refs, phone);
        if let Some((k, f)) = self.match_ui.fits.borrow().get(&i) {
            if *k == key {
                return f.clone();
            }
        }
        let f = Rc::new(fit_groups(links, &key.2, &key.3));
        self.match_ui.fits.borrow_mut().insert(i, (key, f.clone()));
        f
    }

    fn match_view_now(&self, i: NodeId) -> MatchView {
        let (done, running, stages, found, run) = {
            let s = self.nodes[i].state.lock().unwrap();
            (s.done, s.running, s.matching.clone(), s.found.clone(), (s.run_id, s.outbox_rev))
        };
        let links: &[Link] = found.as_deref().map_or(&[], |f| &f.links);
        let saved: BTreeSet<&str> = self.nodes[i].kind.rows().iter().filter_map(|(k, _)| k.strip_prefix("anchor ")).collect();
        let spec = &self.graph.nodes()[i].spec;
        let mut rows: Vec<Row> = Vec::new();
        let mut file_k = 0;
        for (k, ports) in spec.inputs.chunks(PER_ROW).enumerate() {
            let mut r = Row { n: k + 1, ..Default::default() };
            let (Some(audio), Some(time)) = (ports.first(), ports.get(1)) else { continue };
            if let Some(pf) = self.graph.feeder(i, audio.name).and_then(|(f, o)| self.piece_feed(f, o)) {
                // A piece of a cut recording: a recording of its own, on its own clock, of the
                // same device as the file's other pieces (one phone id, so one clock speed).
                r.name = pf.name.clone();
                r.phone = kept(&self.nodes[i].kind, &phone_key(&pf.uid)).unwrap_or_default().trim().to_string();
                if r.phone.is_empty() {
                    r.phone = pf.phone.clone();
                }
                r.speed_ref = kept(&self.nodes[i].kind, &speed_ref_key(&pf.uid)).is_some();
                r.phone_hint = phone_hint(&self.nodes[pf.src].state.lock().unwrap().source.meta);
                r.uid = Some(pf.uid.clone());
                r.clock = Some(pf.clock.clone());
                r.file = self.source_file(pf.src).is_some();
                r.len = pf.piece.len();
                if r.file {
                    r.stage = stages.get(file_k).cloned();
                    file_k += 1;
                }
                r.wired = true;
            } else if let Some((f, _)) = self.graph.feeder(i, audio.name) {
                let path = self.source_file(f);
                r.name = row_name(&self.nodes[f].label, path.as_deref());
                let uid = self.nodes[f].uid.clone();
                r.phone = kept(&self.nodes[i].kind, &phone_key(&uid)).unwrap_or_default().trim().to_string();
                r.speed_ref = kept(&self.nodes[i].kind, &speed_ref_key(&uid)).is_some();
                r.phone_hint = phone_hint(&self.nodes[f].state.lock().unwrap().source.meta);
                r.uid = Some(uid);
                r.clock = self.clock_of(f);
                r.file = path.is_some() && !matches!(self.nodes[f].kind, Kind::CutDetector { .. });
                r.len = self.source_length(f).unwrap_or_else(|| self.nodes[f].state.lock().unwrap().audio_s);
                if r.file {
                    r.stage = stages.get(file_k).cloned();
                    file_k += 1;
                }
                r.wired = true;
            }
            if let Some((f, o)) = self.graph.feeder(i, time.name) {
                r.time_wired = true;
                r.wired = true;
                match self.value_from(f, o) {
                    Ok(Data::Time { utc: Some(u), error_s, source, .. }) => (r.meta, r.meta_err, r.meta_source) = (Some(u), error_s, source),
                    Ok(_) => {}
                    // Nothing came into the Time node (a file with no time in its metadata): no
                    // time metadata, not the Time node's error.
                    Err(e) if matches!(self.nodes[f].kind, Kind::Time { .. }) && !matches!(self.time_in(f), Some(Err(_))) => r.time_err = Some(e),
                    Err(e) => r.meta_source = e,
                }
            }
            if let Some((f, o)) = ports.get(2).and_then(|e| self.graph.feeder(i, e.name)) {
                r.end_wired = true;
                r.wired = true;
                if let Ok(Data::Time { utc: Some(u), .. }) = self.value_from(f, o) {
                    r.end_in = Some(u);
                }
            }
            rows.push(r);
        }
        // Synced groups: rows on one clock, and rows this node's links join.
        let mut parent: Vec<usize> = (0..rows.len()).collect();
        fn root(p: &mut [usize], mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        let by_clock: BTreeMap<String, usize> = rows.iter().enumerate().rev().filter_map(|(k, r)| Some((r.clock.clone()?, k))).collect();
        for (k, r) in rows.iter().enumerate() {
            if let Some(&o) = r.clock.as_ref().and_then(|c| by_clock.get(c)) {
                let (a, b) = (root(&mut parent, k), root(&mut parent, o));
                parent[a.max(b)] = a.min(b);
            }
        }
        if done {
            for l in links {
                if let (Some(&a), Some(&b)) = (by_clock.get(l.a.as_str()), by_clock.get(l.b.as_str())) {
                    let (a, b) = (root(&mut parent, a), root(&mut parent, b));
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
        let mut groups: Vec<Group> = Vec::new();
        let mut at_root: BTreeMap<usize, usize> = BTreeMap::new();
        for k in 0..rows.len() {
            let g = *at_root.entry(root(&mut parent, k)).or_insert_with(|| {
                groups.push(Group::default());
                groups.len() - 1
            });
            groups[g].rows.push(k);
            rows[k].group = g;
        }
        let mut number = 0;
        for g in &mut groups {
            g.key = g.rows.iter().filter_map(|&k| rows[k].clock.clone()).min().unwrap_or_default();
            if g.rows.iter().filter(|&&k| rows[k].wired).count() >= 2 && done {
                number += 1;
                g.number = number;
            }
            g.anchor = g.rows.iter().copied().find(|&k| rows[k].meta.is_some() && rows[k].uid.as_deref().is_some_and(|u| saved.contains(u)));
            let speed_ref = g.rows.iter().copied().find(|&k| rows[k].speed_ref && rows[k].clock.is_some());
            if let Some(s) = speed_ref {
                g.drift = DriftFrom::File(s);
            }
            // The fit's errors are measured against the anchor's recording, else the speed
            // reference's, else the group's longest.
            g.reference = g.anchor.or(speed_ref).or_else(|| g.rows.iter().copied().filter(|&k| rows[k].clock.is_some()).max_by(|&a, &b| rows[a].len.total_cmp(&rows[b].len).then(b.cmp(&a))));
        }
        let refs: BTreeSet<String> = groups.iter().filter_map(|g| rows[g.reference?].clock.clone()).collect();
        // Clocks of one phone share a drift.
        let phones: BTreeMap<String, String> = rows.iter().filter(|r| !r.phone.is_empty()).filter_map(|r| Some((r.clock.clone()?, r.phone.clone()))).rev().collect();
        let fits = (done && !links.is_empty()).then(|| self.match_fits(i, run, links, refs, phones));
        let fit_of = |r: &Row| r.clock.as_ref().and_then(|c| fits.as_ref()?.clocks.get(c)).copied().unwrap_or(SAME_CLOCK);
        let shift_of = |g: &Group| saved_shift(&self.nodes[i].kind, &g.key);
        let on_average = |g: &Group| kept(&self.nodes[i].kind, &average_key(&g.key)).is_some();
        for g in &mut groups {
            g.moved = g.anchor.is_none() && !on_average(g) && shift_of(g) != 0.0;
        }
        let real = rows.iter().any(|r| r.meta.is_some());
        // The middle of every time there is: where a group with no time at all is shown.
        let all_mids: Vec<f64> = rows.iter().filter_map(|r| r.meta.map(|t| mid(t, r.len))).collect();
        let mut spreads = vec![None; groups.len()];
        let mut leftover = None;
        if done {
            for (gi, g) in groups.iter_mut().enumerate() {
                let audio_rows: Vec<usize> = g.rows.iter().copied().filter(|&k| rows[k].clock.is_some()).collect();
                let timed: Vec<usize> = audio_rows.iter().copied().filter(|&k| rows[k].meta.is_some()).collect();
                let linked = audio_rows.len() >= 2;
                if !linked && timed.is_empty() {
                    continue; // alone, with no time: nowhere to show it
                }
                // Real seconds per second of the fit's clock: the group's average clock runs true,
                // unless a speed reference's clock does, unless the anchor's wired end says.
                let rate = |k: usize| fit_of(&rows[k]).rate;
                let anchor_end = g.anchor.and_then(|a| {
                    let meta = rows[a].meta?;
                    rows[a].end_in.filter(|_| rows[a].len > 0.0).map(|e| (e - meta) / rows[a].len).filter(|q| *q > 0.0).map(|q| (a, q / (1.0 + rate(a))))
                });
                let k_real = match (anchor_end, g.drift) {
                    (Some((a, k)), _) => {
                        g.drift = DriftFrom::AnchorEnd(a);
                        k
                    }
                    (None, DriftFrom::File(s)) => 1.0 / (1.0 + rate(s)),
                    _ => 1.0,
                };
                // Where the fit's 0 is in real time.
                let start = |k: usize| k_real * fit_of(&rows[k]).at(0.0);
                // Each stamp's vote for it, one vote per phone.
                let votes: Vec<(String, f64, f64)> = timed.iter().map(|&k| (phone_of(&rows[k]), rows[k].meta.unwrap_or(0.0) - start(k), rows[k].meta_err)).collect();
                g.stamps = stamp_average(&votes);
                if g.anchor.is_none() && on_average(g) {
                    g.average = g.stamps;
                }
                let at = match (g.anchor, g.average, g.stamps) {
                    (Some(a), _, _) => rows[a].meta.unwrap_or(0.0) - start(a),
                    (None, Some(avg), _) => avg.at,
                    (None, None, Some(avg)) => avg.at + shift_of(g),
                    // No time in the group: shown at the middle of every time there is.
                    (None, None, None) => {
                        let audios: Vec<f64> = audio_rows.iter().map(|&k| mid(start(k), rows[k].len)).collect();
                        mean(&all_mids).unwrap_or(0.0) - mean(&audios).unwrap_or(0.0) + shift_of(g)
                    }
                };
                // Unanchored, each time keeps its own metadata error; a row with none the group's largest.
                let largest = timed.iter().map(|&k| rows[k].meta_err).fold(0.0, f64::max);
                let anchor_err = g.anchor.map(|a| rows[a].meta_err).or(g.average.map(|a| a.error));
                for &k in &audio_rows {
                    let f = fit_of(&rows[k]);
                    let (s0, s1) = place(&f, rows[k].len, at, k_real);
                    let r = &mut rows[k];
                    r.start = Some(s0);
                    r.end = Some(s1);
                    r.off = r.meta.map(|t| t - s0);
                    r.start_err = match anchor_err {
                        Some(e) => e.hypot(f.error_at(0.0)),
                        None => r.meta.map_or(largest, |_| r.meta_err),
                    };
                    r.end_err = r.start_err.hypot(f.var_rate.sqrt() * r.len);
                    r.speed = linked.then(|| speed_ppm(&f, k_real));
                }
                if linked && g.anchor.is_none() {
                    let recs: Vec<Placed> = audio_rows.iter().map(|&k| Placed { stamp: rows[k].meta, len: rows[k].len, start: fit_of(&rows[k]).at(0.0) }).collect();
                    spreads[gi] = audio_match::stamp_spread(&recs, false).zip(audio_match::stamp_spread(&recs, true));
                }
                // How far off is about its own length: compared directly once anchored; before an
                // anchor the average has absorbed it, so the stamps' agreement decides.
                let as_ends = spreads[gi].is_some_and(|(s, e): (f64, f64)| s - e > 1.0 && e * 2.0 < s);
                for &k in &audio_rows {
                    rows[k].hint = rows[k].meta.is_some() && if g.anchor.is_some() { rows[k].off.is_some_and(|d| (d.abs() - rows[k].len).abs() <= HINT_S) } else { as_ends };
                }
            }
            // Each row's links and how far they sit from the fit; the marks the match left on it.
            if let Some(fits) = &fits {
                let mut per: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
                for (l, d) in links.iter().zip(&fits.leftover).filter(|(_, d)| d.is_finite()) {
                    for c in [l.a.as_str(), l.b.as_str()] {
                        let e = per.entry(c).or_default();
                        e.0 += 1;
                        e.1 += d * d;
                    }
                }
                for r in rows.iter_mut() {
                    if let Some(&(n, sq)) = r.clock.as_deref().and_then(|c| per.get(c)) {
                        (r.links, r.leftover) = (n, (sq / n as f64).sqrt());
                    }
                }
                let mut all: Vec<f64> = fits.leftover.iter().filter(|d| d.is_finite()).map(|d| d.abs()).collect();
                all.sort_by(f64::total_cmp);
                leftover = all.get(all.len() / 2).copied();
            }
            if let Some(f) = &found {
                for r in rows.iter_mut() {
                    for (kind, evs) in [(MarkKind::Same, &f.same), (MarkKind::Speed, &f.speed), (MarkKind::Gap, &f.gaps)] {
                        for e in evs.iter().filter(|e| r.clock.as_deref() == Some(e.clock.as_str())) {
                            r.marks.push(Mark { kind, text: format!("{}: {}", e.label, e.detail) });
                        }
                    }
                }
            }
            if let Some(lost) = self.match_ui.lost.get(&i) {
                for r in rows.iter_mut() {
                    r.lost = r.uid.as_ref().is_some_and(|u| lost.contains(u));
                }
            }
        }
        MatchView { rows, groups, done, running, real, spreads, leftover }
    }

    /// The better time on output `out`: "synced 3" is row 3's audio start time, "end 3" its audio
    /// end time. Anchored, from the anchor's time along the group's own fit; unanchored, where the
    /// group's metadata (or a drag) puts it, with the row's own metadata error. The end is the
    /// start plus the length at the speed the fit measured; a row audio did not link passes its
    /// wired end straight through, or gives start + length.
    pub(crate) fn synced_time(&self, i: NodeId, out: &str) -> Result<Data, String> {
        let (end, n) = match (out.strip_prefix("synced "), out.strip_prefix("end ")) {
            (Some(n), _) => (false, n),
            (_, Some(n)) => (true, n),
            _ => return Err("no such output".into()),
        };
        let n: usize = n.parse().map_err(|_| "no such output")?;
        let v = self.match_view(i);
        let r = v.rows.get(n - 1).ok_or("no such row")?;
        if end && !v.linked(r) {
            if let Some((f, o)) = self.graph.feeder(i, &format!("end in {n}")) {
                return self.value_from(f, o);
            }
        }
        if cell(&v, n - 1).1 {
            return Err("no time metadata, and audio linked it to nothing: nothing to place it by".into());
        }
        let clock = r.clock.clone().ok_or("no audio wired into this row")?;
        let g = &v.groups[r.group];
        let at = r.start.ok_or(if v.done { "audio did not place it" } else { "not matched yet: run the graph" })?;
        let phones = |n: usize| if n == 1 { "1 phone's stamps".to_string() } else { format!("{n} phones' stamps") };
        let source = match (g.anchor, g.average) {
            (Some(a), _) => format!("Audio match, anchored on the time of {}", v.rows[a].name),
            (None, Some(avg)) => format!("Audio match, anchored on the average of {}", phones(avg.phones)),
            (None, None) => {
                if !g.rows.iter().any(|&k| v.rows[k].meta.is_some()) {
                    return Err("no time metadata in its synced group: nothing to place it by".into());
                }
                let how = if g.moved { "moved by hand".to_string() } else { format!("the average of {}", phones(g.stamps.map_or(0, |s| s.phones))) };
                format!("Audio match, unanchored: {how}")
            }
        };
        let fitted = fit_words(&v, r);
        Ok(if end {
            let e = r.end.unwrap_or(at + r.len);
            let how = match r.speed {
                _ if g.drift == DriftFrom::AnchorEnd(n - 1) => "; its wired end".to_string(),
                Some((ppm, _)) => format!("; its length at the measured drift {} s/h, {} from start + length", drift_text(ppm), ms_text(e - at - r.len)),
                None => format!(", plus its length {:.3} s", r.len),
            };
            Data::Time { clock, t: r.len, utc: Some(e), error_s: r.end_err, source: format!("{source}{how}{fitted}") }
        } else {
            Data::Time { clock, t: 0.0, utc: Some(at), error_s: r.start_err, source: format!("{source}{fitted}") }
        })
    }

    /// Whether row `k`'s outputs are anchored (Some(true)), unanchored (Some(false)), or give
    /// nothing (None).
    fn output_mark(&self, i: NodeId, v: &MatchView, k: usize) -> Option<bool> {
        self.synced_time(i, &format!("synced {}", k + 1)).ok()?;
        Some(v.anchored(&v.rows[k]))
    }

    /// Group `g` is anchored on the average of its stamps, or stops being. It takes the place of
    /// a row's anchor and of a drag. Logged like any setting.
    pub(crate) fn toggle_average(&mut self, i: NodeId, g: usize) {
        let v = self.match_view(i);
        let Some(grp) = v.groups.get(g) else { return };
        let on = grp.average.is_none();
        if on && grp.stamps.is_none() {
            return;
        }
        let members: Vec<String> = grp.rows.iter().filter_map(|&k| v.rows[k].uid.clone()).collect();
        let name = grp.rows.first().map_or(grp.key.clone(), |&k| v.rows[k].name.clone());
        let old = match grp.anchor {
            Some(a) => v.rows[a].name.clone(),
            None if grp.average.is_some() => "the average of its stamps".to_string(),
            None => "none".to_string(),
        };
        let key = grp.key.clone();
        let Kind::AudioMatch { rows } = &mut self.nodes[i].kind else { return };
        rows.retain(|(k, _)| k != &average_key(&key) && k != &shift_key(&key) && !k.strip_prefix("anchor ").is_some_and(|u| members.iter().any(|m| m == u)));
        if on {
            rows.push((average_key(&key), "on".to_string()));
        }
        let label = self.nodes[i].label.clone();
        let new = if on { "the average of its stamps" } else { "none" };
        self.changes.push(bus::SettingChange { utc: timebase::now_unix(), node: self.nodes[i].uid.clone(), label, name: format!("anchor of the group with {name}"), old, new: new.to_string() });
        self.match_ui.cache.borrow_mut().remove(&i);
        self.save_project();
    }

    /// Row `row`'s clock runs true, or stops: one per group. Logged like any setting.
    pub(crate) fn toggle_speed_ref(&mut self, i: NodeId, row: usize) {
        let v = self.match_view(i);
        let Some(r) = v.rows.get(row) else { return };
        let Some(uid) = r.uid.clone() else { return };
        let g = &v.groups[r.group];
        let members: Vec<String> = g.rows.iter().filter_map(|&k| v.rows[k].uid.clone()).collect();
        let old = g.rows.iter().find(|&&k| v.rows[k].speed_ref).map_or("none".to_string(), |&k| v.rows[k].name.clone());
        let on = !r.speed_ref;
        let name = r.name.clone();
        let Kind::AudioMatch { rows } = &mut self.nodes[i].kind else { return };
        rows.retain(|(k, _)| !k.strip_prefix("speed reference ").is_some_and(|u| members.iter().any(|m| m == u)));
        if on {
            rows.push((speed_ref_key(&uid), "on".to_string()));
        }
        let label = self.nodes[i].label.clone();
        self.changes.push(bus::SettingChange { utc: timebase::now_unix(), node: self.nodes[i].uid.clone(), label, name: format!("speed reference of the group with {name}"), old, new: if on { name.clone() } else { "none".to_string() } });
        self.match_ui.cache.borrow_mut().remove(&i);
        self.save_project();
    }

    /// Row `row`'s phone id becomes `id` (empty: its own phone). Logged like any setting.
    pub(crate) fn set_phone(&mut self, i: NodeId, row: usize, id: &str) {
        let v = self.match_view(i);
        let Some(r) = v.rows.get(row) else { return };
        let Some(uid) = r.uid.clone() else { return };
        let id = id.trim().to_string();
        if id == r.phone {
            return;
        }
        let (old, name) = (r.phone.clone(), r.name.clone());
        let Kind::AudioMatch { rows } = &mut self.nodes[i].kind else { return };
        rows.retain(|(k, _)| k != &phone_key(&uid));
        if !id.is_empty() {
            rows.push((phone_key(&uid), id.clone()));
        }
        let label = self.nodes[i].label.clone();
        let shown = |s: &str| if s.is_empty() { "none".to_string() } else { s.to_string() };
        self.changes.push(bus::SettingChange { utc: timebase::now_unix(), node: self.nodes[i].uid.clone(), label, name: format!("phone of {name}"), old: shown(&old), new: shown(&id) });
        self.match_ui.cache.borrow_mut().remove(&i);
        self.save_project();
    }

    /// Row `row` becomes its group's anchor, or stops being it. Kept by the recording, so it stays
    /// with the recording after a new run. Logged like any setting.
    pub(crate) fn toggle_anchor(&mut self, i: NodeId, row: usize) {
        let v = self.match_view(i);
        let Some(r) = v.rows.get(row) else { return };
        let Some(uid) = r.uid.clone().filter(|_| r.meta.is_some()) else { return };
        let g = &v.groups[r.group];
        let was = g.anchor;
        let members: Vec<String> = g.rows.iter().filter_map(|&k| v.rows[k].uid.clone()).collect();
        let name = r.name.clone();
        let old = was.map_or(if g.average.is_some() { "the average of its stamps" } else { "none" }.to_string(), |a| v.rows[a].name.clone());
        let on = was != Some(row);
        let key = g.key.clone();
        let Kind::AudioMatch { rows } = &mut self.nodes[i].kind else { return };
        rows.retain(|(k, _)| !k.strip_prefix("anchor ").is_some_and(|u| members.iter().any(|m| m == u)));
        if on {
            // A row's anchor takes the place of the average.
            rows.retain(|(k, _)| k != &average_key(&key));
            rows.push((anchor_key(&uid), "on".to_string()));
            self.match_ui.members.insert((i, uid.clone()), members.iter().cloned().collect());
        }
        if let Some(l) = self.match_ui.lost.get_mut(&i) {
            l.retain(|u| !members.contains(u));
        }
        let label = self.nodes[i].label.clone();
        self.changes.push(bus::SettingChange { utc: timebase::now_unix(), node: self.nodes[i].uid.clone(), label, name: format!("anchor of the group with {name}"), old, new: if on { name.clone() } else { "none".to_string() } });
        self.match_ui.cache.borrow_mut().remove(&i);
        self.save_project();
    }

    /// A new result: what left an anchored group since the last one has lost its anchor.
    fn note_lost_anchors(&mut self, i: NodeId, v: &MatchView) {
        let run = self.nodes[i].state.lock().unwrap().run_id;
        if !v.done || self.match_ui.seen_run.get(&i) == Some(&run) {
            return;
        }
        self.match_ui.seen_run.insert(i, run);
        let mut lost: BTreeSet<String> = BTreeSet::new();
        for g in v.groups.iter().filter(|g| g.anchor.is_some()) {
            let holder = v.rows[g.anchor.unwrap_or(0)].uid.clone().unwrap_or_default();
            let now: BTreeSet<String> = g.rows.iter().filter_map(|&k| v.rows[k].uid.clone()).collect();
            if let Some(before) = self.match_ui.members.get(&(i, holder.clone())) {
                lost.extend(before.difference(&now).cloned());
            }
            self.match_ui.members.insert((i, holder), now);
        }
        self.match_ui.lost.insert(i, lost);
        self.match_ui.cache.borrow_mut().remove(&i);
    }

    /// Dropping a SOURCER's audio on a row also draws its time wire, when that row has none.
    pub(crate) fn also_wire_time(&mut self, from: NodeId, fk: usize, to: NodeId, tk: usize) {
        if !matches!(self.nodes[to].kind, Kind::AudioMatch { .. }) || !matches!(self.nodes[from].kind, Kind::Sourcer { .. }) {
            return;
        }
        let (out, inp) = (self.graph.nodes()[from].spec.outputs[fk].name, self.graph.nodes()[to].spec.inputs[tk].name);
        let Some(n) = inp.strip_prefix("audio ") else { return };
        let time = format!("time {n}");
        if out == "audio" && self.graph.feeder(to, &time).is_none() {
            self.wire(from, "time", to, &time);
        }
    }

    /// The files wired into node `i`, and the labels of sources that are not files. `before`: each
    /// source's run before this run started.
    pub(crate) fn audio_match_inputs(&self, i: NodeId, before: &HashMap<NodeId, u64>) -> (Vec<Wired>, Vec<String>) {
        let (mut files, mut other) = (Vec::new(), Vec::new());
        for p in self.graph.nodes()[i].spec.inputs.iter().filter(|p| p.kind == PortKind::Audio) {
            let Some((f, o)) = self.graph.feeder(i, p.name) else { continue };
            let label = self.nodes[f].label.clone();
            match &self.nodes[f].kind {
                Kind::Sourcer { input } => match sourcer_node::input(input) {
                    Some(runtime::Input::File(path)) => {
                        let source = before.get(&f).map(|&r| (self.nodes[f].state.clone(), r));
                        files.push(Wired { label, clock: self.clock_of(f).unwrap_or_default(), path, source, range: None })
                    }
                    _ => other.push(label),
                },
                // A piece of a cut recording: its file, from where to where.
                Kind::CutDetector { .. } => match self.piece_feed(f, o).and_then(|pf| Some((self.source_file(pf.src)?, pf))) {
                    Some((path, pf)) => {
                        let source = Some((self.nodes[pf.src].state.clone(), before.get(&pf.src).copied().unwrap_or(0)));
                        files.push(Wired { label: pf.name, clock: pf.clock, path, source, range: Some((pf.piece.a0, pf.piece.a1)) })
                    }
                    None => other.push(format!("{label} › {o}")),
                },
                _ => other.push(label),
            }
        }
        (files, other)
    }
}

/// Decodes and matches `files` on a thread of its own, writing into `state` (unless `stop` is
/// set first, or a newer run took the state over).
pub fn start(state: Shared, files: Vec<Wired>, other: Vec<String>, stop: Arc<AtomicBool>) {
    let run = RUNS.fetch_add(1, Ordering::Relaxed);
    {
        let mut s = state.lock().unwrap();
        s.reset();
        s.run_id = run;
        s.matching = vec![Stage::Waiting(0.0); files.len()];
    }
    let note = if other.is_empty() { String::new() } else { format!(" ({} left out: files only)", other.join(", ")) };
    let set = |f: &dyn Fn(&mut runtime::NodeState)| {
        let mut s = state.lock().unwrap();
        if s.run_id == run && !stop.load(Ordering::Relaxed) {
            f(&mut s);
        }
    };
    if files.len() < 2 {
        set(&|s| s.status = format!("wire two recordings or more{note}"));
        return;
    }
    set(&|s| {
        s.running = true;
        s.status = format!("waiting for {} recordings to decode…", files.len());
    });
    let stop2 = stop.clone();
    thread::spawn(move || {
        let set = |f: &dyn Fn(&mut runtime::NodeState)| {
            let mut s = state.lock().unwrap();
            if s.run_id == run && !stop2.load(Ordering::Relaxed) {
                f(&mut s);
            }
        };
        // Stopped: no results, and it no longer counts as running.
        let stopped = || {
            let mut s = state.lock().unwrap();
            if s.run_id == run {
                s.running = false;
                s.status = "stopped".into();
            }
        };
        // A core per recording at a time: each waits for its source's audio (or decodes the
        // file), then fingerprints it.
        let next = std::sync::atomic::AtomicUsize::new(0);
        let ready = std::sync::atomic::AtomicUsize::new(0);
        let threads = thread::available_parallelism().map_or(4, |n| n.get()).min(files.len());
        let mut decoded: Vec<(usize, Result<Clip, String>)> = thread::scope(|sc| {
            let hs: Vec<_> = (0..threads)
                .map(|_| {
                    sc.spawn(|| {
                        let mut done = Vec::new();
                        loop {
                            let k = next.fetch_add(1, Ordering::Relaxed);
                            let Some(w) = files.get(k) else { break };
                            if stop2.load(Ordering::Relaxed) {
                                break;
                            }
                            let stage = |to: Stage| {
                                set(&|s| {
                                    if let Some(x) = s.matching.get_mut(k) {
                                        *x = to.clone();
                                    }
                                })
                            };
                            let audio = match &w.source {
                                Some((st, before)) => from_source(st, *before, &stop2, &|secs| stage(Stage::Waiting(secs))),
                                None => None,
                            };
                            let audio = match audio {
                                Some(a) if !a.is_empty() => Ok(a),
                                _ if stop2.load(Ordering::Relaxed) => Err("stopped".to_string()),
                                _ => decode(&w.path, &stop2),
                            };
                            let audio = audio.map(|a| piece_of(a, w.range)).and_then(|a| if a.is_empty() { Err("the piece has no sound".to_string()) } else { Ok(a) });
                            if audio.is_ok() {
                                stage(Stage::Fingerprinting);
                            }
                            let clip = audio.map(Clip::new);
                            stage(match &clip {
                                Ok(c) => Stage::Ready(c.hashes()),
                                Err(e) => Stage::Failed(e.clone()),
                            });
                            done.push((k, clip));
                            let n = ready.fetch_add(1, Ordering::Relaxed) + 1;
                            set(&|s| s.status = format!("fingerprinted {n} of {} recordings…", files.len()));
                        }
                        done
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        let mut slots: Vec<Result<Clip, String>> = files.iter().map(|_| Err("decoder crashed".to_string())).collect();
        for (k, c) in decoded.drain(..) {
            slots[k] = c;
        }
        let decoded = slots;
        if stop2.load(Ordering::Relaxed) {
            return stopped();
        }
        let mut clips = Vec::new();
        let mut used = Vec::new();
        let mut errors = Vec::new();
        for (w, c) in files.iter().zip(decoded) {
            match c {
                Ok(c) => {
                    clips.push(c);
                    used.push(w);
                }
                Err(e) => errors.push(format!("{}: {e}", w.label)),
            }
        }
        set(&|s| s.status = format!("matching {} pairs…", clips.len() * clips.len().saturating_sub(1) / 2));
        let refs: Vec<&Clip> = clips.iter().collect();
        let found = audio_match::match_all(&refs);
        if stop2.load(Ordering::Relaxed) {
            return stopped();
        }
        let r = results(&used, &found, &errors);
        set(&|s| {
            s.outbox = Vec::new();
            s.found = Some(Arc::new(r.found.clone()));
            s.outbox_rev += 1;
            s.status = format!("{}{note}", r.status);
            s.running = false;
            s.done = true;
        });
    });
}

/// The audio source `st` decoded in this run (its run differs from `before` and is done), at the
/// match's rate. None: stopped, or the source ended with no audio (the file is decoded here then).
/// `report` is told the seconds decoded so far.
pub(crate) fn from_source(st: &Shared, before: u64, stop: &AtomicBool, report: &dyn Fn(f64)) -> Option<Vec<f32>> {
    loop {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        let audio = {
            let s = st.lock().unwrap();
            let secs = if s.run_id != before { s.audio_s } else { 0.0 };
            ((s.run_id != before && s.done && !s.running).then(|| s.source.audio.clone()), secs)
        };
        let (audio, secs) = audio;
        report(secs);
        if let Some(a) = audio {
            let a = a.lock().unwrap();
            return (!a.is_empty()).then(|| audio_match::halve(&a));
        }
        thread::sleep(std::time::Duration::from_millis(100));
    }
}

pub(crate) fn decode(path: &str, stop: &AtomicBool) -> Result<Vec<f32>, String> {
    // Its turn among every decoder of the project (the sources' too).
    let Some(_turn) = corroborate::ingest::decode_turn(stop) else { return Err("stopped".into()) };
    let (mut child, rx) = corroborate::ingest::spawn_ffmpeg(path, audio_match::SR)?;
    let mut out = Vec::new();
    for c in rx {
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("stopped".into());
        }
        out.extend(c);
    }
    let _ = child.wait();
    if out.is_empty() { Err("no audio".into()) } else { Ok(out) }
}

/// What the match gives: what it found, kept with the node's state.
/// `found`'s quality is the `quality` type's fields, then the counts it rests on and each pair's;
/// its drift a JSON list, one per pair with a long enough span of links, of how fast a's clock
/// runs against b's (`audio_match::drift`); its same recording an event on each of the two clocks
/// over the matched stretch, its speed change one on b's clock, its gaps one on the recording
/// that lacks the stretch.
struct Results {
    found: Found,
    status: String,
}

/// The stretch of clock b (`a`: of clock a) the hits cover, with half a window either side.
fn span(hits: &[audio_match::Hit], a: bool) -> (f64, f64) {
    let t = |h: &audio_match::Hit| if a { h.t_a } else { h.t_b };
    let lo = hits.iter().map(t).fold(f64::MAX, f64::min) - audio_match::WIN_S / 2.0;
    let hi = hits.iter().map(t).fold(f64::MIN, f64::max) + audio_match::WIN_S / 2.0;
    (lo.max(0.0), hi)
}

fn results(used: &[&Wired], found: &[(usize, usize, audio_match::Pair)], errors: &[String]) -> Results {
    let (mut drifts, mut same, mut speed, mut gaps) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut links = Vec::new();
    let mut fitted = Vec::new();
    let mut pairs = Vec::new();
    let (mut chunks, mut cands, mut weak, mut linked) = (0, 0, 0, 0);
    let mut group: Vec<usize> = (0..used.len()).collect();
    fn root(g: &mut [usize], x: usize) -> usize {
        if g[x] != x {
            let r = root(g, g[x]);
            g[x] = r;
        }
        g[x]
    }
    for (a, b, p) in found {
        let (wa, wb) = (used[*a], used[*b]);
        chunks += p.chunks;
        cands += p.candidates;
        weak += p.weak;
        if !p.hits.is_empty() {
            linked += 1;
            let (ra, rb) = (root(&mut group, *a), root(&mut group, *b));
            group[ra] = rb;
        }
        if audio_match::same_recording(&p.hits) {
            let corr = p.hits.iter().map(|h| h.corr).fold(0.0, f64::max);
            for (w, o, on_a) in [(wa, wb, true), (wb, wa, false)] {
                let (t0, t1) = span(&p.hits, on_a);
                same.push(Event {
                    clock: w.clock.clone(),
                    t_start: t0,
                    t_end: t1,
                    label: format!("same recording as {}", o.label),
                    detail: format!("the audio correlates {corr:.3} (two microphones stay under {}): a copy, not a second witness", audio_match::SAME_CORR),
                    error_s: 0.0,
                });
            }
        }
        let found_gaps = audio_match::gaps(&p.hits);
        for g in &found_gaps {
            let (w, o) = if g.on_a { (wa, wb) } else { (wb, wa) };
            gaps.push(Event {
                clock: w.clock.clone(),
                t_start: g.t0,
                t_end: g.t1,
                label: format!("{:.2} s missing against {}", g.len_s, o.label),
                detail: format!(
                    "somewhere between these moments {} lacks {:.3} ± {:.3} s that {} has: paused or cut here, or {} has an insert",
                    w.label, g.len_s, g.error_s, o.label, o.label
                ),
                error_s: g.error_s,
            });
        }
        // A jump bends a line through all the links: no drift for a pair with a gap.
        if let Some(d) = audio_match::drift(&p.hits).filter(|_| found_gaps.is_empty()) {
            drifts.push(format!(
                "{{\"a\":\"{}\",\"b\":\"{}\",\"ppm\":{:.1},\"error_ppm\":{:.1},\"span_s\":{:.1},\"links\":{}}}",
                json_escape(&wa.label),
                json_escape(&wb.label),
                d.ppm,
                d.error_ppm,
                d.span_s,
                d.links
            ));
            if audio_match::speed_changed(&d) {
                let (t0, t1) = span(&p.hits, false);
                speed.push(Event {
                    clock: wb.clock.clone(),
                    t_start: t0,
                    t_end: t1,
                    label: format!("speed {:+.2} % against {}", d.ppm * 1e-4, wa.label),
                    detail: format!(
                        "{} plays {:.0} ± {:.0} ppm {} than {} over {:.0} s: far beyond clock drift ({} ppm), so one was sped up or slowed down",
                        wb.label,
                        d.ppm.abs(),
                        d.error_ppm,
                        if d.ppm > 0.0 { "faster" } else { "slower" },
                        wa.label,
                        d.span_s,
                        audio_match::SPEED_CHANGE_PPM
                    ),
                    error_s: 0.0,
                });
            }
        }
        for h in &p.hits {
            fitted.push(Link { a: wa.clock.clone(), t_a: h.t_a, b: wb.clock.clone(), t_b: h.t_b, error_s: h.error_s, from: String::new(), enabled: true });
            links.push(format!(
                "{{\"a\":\"{}\",\"t_a\":{:.6},\"b\":\"{}\",\"t_b\":{:.6},\"error_s\":{:.6},\"source\":\"audio match: {} / {}, PSR {:.1}, {} fingerprint votes\"}}",
                json_escape(&wa.clock),
                h.t_a,
                json_escape(&wb.clock),
                h.t_b,
                h.error_s,
                json_escape(&wa.label),
                json_escape(&wb.label),
                h.psr,
                h.votes
            ));
        }
        pairs.push(format!(
            "{{\"a\":\"{}\",\"b\":\"{}\",\"links\":{},\"chunks\":{},\"candidates\":{},\"unconfirmed\":{}}}",
            json_escape(&wa.label),
            json_escape(&wb.label),
            p.hits.len(),
            p.chunks,
            p.candidates,
            p.weak
        ));
    }
    let n = used.len();
    let groups = (0..n).filter(|&x| root(&mut group, x) == x).count();
    let level = if links.is_empty() {
        "none"
    } else if groups == 1 {
        "good"
    } else {
        "weak"
    };
    let verdict = match level {
        "none" => format!("no two of the {n} recordings hear the same sound"),
        "good" => format!("all {n} recordings linked into one group"),
        _ => format!("{n} recordings in {groups} groups: some hear nothing the others do"),
    };
    let errs: Vec<String> = errors.iter().map(|e| format!("\"{}\"", json_escape(e))).collect();
    let quality = format!(
        "{{\"level\":\"{level}\",\"verdict\":\"{}\",\"recordings\":{n},\"groups\":{groups},\"pairs\":{},\"pairs_linked\":{linked},\"links\":{},\"chunks\":{chunks},\"candidates\":{cands},\"unconfirmed\":{weak},\"errors\":[{}],\"per_pair\":[{}]}}",
        json_escape(&verdict),
        found.len(),
        links.len(),
        errs.join(","),
        pairs.join(",")
    );
    let mut status = format!("{} links, {linked} of {} pairs, {groups} group{}", links.len(), found.len(), if groups == 1 { "" } else { "s" });
    if !same.is_empty() {
        status += &format!(", {} same recording", same.len() / 2);
    }
    if !speed.is_empty() {
        status += &format!(", {} speed change", speed.len());
    }
    if !gaps.is_empty() {
        status += &format!(", {} gap{}", gaps.len(), if gaps.len() == 1 { "" } else { "s" });
    }
    Results { found: Found { links: fitted, links_json: format!("[{}]", links.join(",")), same, speed, gaps, quality, drift: format!("[{}]", drifts.join(",")) }, status }
}

/// The node's part of the result file: what it gave on each output, and each row's better time.
pub fn json(app: &App, i: NodeId) -> String {
    let outs: Vec<String> = app.nodes[i].state.lock().unwrap().outbox.iter().map(|(n, d)| format!("\"{}\":{}", json_escape(n), d.to_json())).collect();
    let v = app.match_view(i);
    let rows: Vec<String> = v
        .rows
        .iter()
        .filter(|r| r.wired)
        .map(|r| {
            let meta = r.meta.map_or("null".to_string(), |t| format!("\"{}\"", timebase::fmt_utc(t)));
            let synced = app.synced_time(i, &format!("synced {}", r.n)).map_or("null".to_string(), |d| d.to_json());
            let end = app.synced_time(i, &format!("end {}", r.n)).map_or("null".to_string(), |d| d.to_json());
            let g = &v.groups[r.group];
            let drift = r.speed.map_or("null".to_string(), |(p, e)| {
                format!("{{\"s_per_h\":{:.4},\"error_s_per_h\":{:.4},\"clock_speed\":{:.9},\"against\":\"{}\"}}", s_per_h(p), s_per_h(e), 1.0 + p * 1e-6, json_escape(&drift_against(&v, g)))
            });
            let marks: Vec<String> = r.marks.iter().map(|m| format!("\"{}\"", json_escape(&m.text))).collect();
            let anchor = if g.anchor == Some(r.n - 1) { "\"its time\"" } else if g.average.is_some() { "\"the average of the group's stamps\"" } else { "null" };
            format!(
                "{{\"row\":{},\"recording\":\"{}\",\"phone\":\"{}\",\"time_in\":{meta},\"group\":{},\"anchor\":{anchor},\"speed_reference\":{},\"drift\":{drift},\"links\":{},\"leftover_s\":{:.6},\"marks\":[{}],\"synced\":{synced},\"end\":{end}}}",
                r.n,
                json_escape(&r.name),
                json_escape(&r.phone),
                g.number,
                r.speed_ref,
                r.links,
                r.leftover,
                marks.join(",")
            )
        })
        .collect();
    let found = app.nodes[i].state.lock().unwrap().found.clone();
    let found = found.map_or(String::new(), |f| {
        let ev = |v: &[Event]| Data::Events(v.to_vec()).to_json();
        format!(",\"quality\":{},\"clock_drift\":{},\"same_recording\":{},\"speed_change\":{},\"gaps\":{},\"links\":{}", f.quality, f.drift, ev(&f.same), ev(&f.speed), ev(&f.gaps), f.links_json)
    });
    format!(",\"outputs\":{{{}}}{found},\"rows\":[{}]", outs.join(","), rows.join(","))
}

// ---------------------------------------------------------------- the rows on the node

/// What the rows keep between frames.
#[derive(Default)]
pub struct MatchUi {
    /// Axes moved by hand, by node. None: View all is live, the axis follows the bars.
    pub views: HashMap<NodeId, (f64, f64)>,
    /// The tracks on screen: the wheel over one is not the canvas's.
    pub tracks: Vec<Rect>,
    /// The group being dragged, the axis held still meanwhile, and how far the group had been
    /// dragged before (a drag is kept in the node's rows, `shift <group>`, and logged when it ends).
    grab: Option<(NodeId, String, (f64, f64), f64)>,
    /// Which row is drawn where (row → place), by node, as last drawn (Audio match and Timeline).
    pub slots: HashMap<NodeId, Vec<usize>>,
    /// Who was in each anchored group at the last result, by node and anchor; who has left it since.
    members: HashMap<(NodeId, String), BTreeSet<String>>,
    lost: HashMap<NodeId, BTreeSet<String>>,
    seen_run: HashMap<NodeId, u64>,
    /// Where each row's anchor button is on screen, as last drawn.
    pub anchor_at: HashMap<(NodeId, usize), Rect>,
    /// Where each row's phone id is on screen, as last drawn.
    pub phone_at: HashMap<(NodeId, usize), Rect>,
    /// Each node's rows this frame (cleared at the start of the canvas).
    pub cache: std::cell::RefCell<HashMap<NodeId, Rc<MatchView>>>,
    /// Each node's last fit, by run, outbox revision, references and phones.
    #[allow(clippy::type_complexity)]
    fits: std::cell::RefCell<HashMap<NodeId, ((u64, u64, BTreeSet<String>, BTreeMap<String, String>), Rc<Fits>)>>,
    /// The phone id being typed: node, recording, text.
    phone_edit: Option<(NodeId, String, String)>,
}

// Unzoomed, from the top of the node.
const NODE_W: f32 = 1080.0;
const TOP: f32 = HEAD + 4.0;
const BAR_H: f32 = 30.0;
const RULER_H: f32 = 26.0;
const FOOT_H: f32 = 46.0;
const LEFT_W: f32 = 170.0;
const STATUS_W: f32 = 116.0;
const DRIFT_W: f32 = 100.0;
/// The phone id field on a row's name strip.
const PHONE_W: f32 = 64.0;
const RIGHT_W: f32 = 116.0;
const ROW_DEFAULT: f32 = 64.0;
/// The thinnest row is its file name and nothing else.
const ROW_MIN: f32 = STRIP_H + 2.0;
const ROW_MAX: f32 = 96.0;
const STRIP_H: f32 = 20.0;
/// The thin stripe under a row's audio start time: its optional audio end time input. A row is
/// its height plus this.
const END_H: f32 = 12.0;
/// With ports at least this tall, "audio" and "time" are written on them.
const WORDS_FROM: f32 = 14.0;
/// With ports shorter than this, the anchor button moves up beside the file name.
const ANCHOR_FROM: f32 = 10.0;
/// How far the node's edge is outside the part a look draws in.
const EDGE: f32 = 6.0;

const AMBER: Color32 = Color32::from_rgb(233, 168, 76);
const BLUE: Color32 = Color32::from_rgb(125, 180, 230);
const BOTH: Color32 = Color32::from_rgb(246, 236, 217);
const OFF: Color32 = Color32::from_rgb(65, 70, 78);
/// The end input's stripe while nothing is wired into it.
const OPT: Color32 = Color32::from_rgb(51, 55, 62);
/// A time output, blue like every time port.
const TIME_OUT: Color32 = Color32::from_rgb(122, 167, 224);
const STRIP: Color32 = Color32::from_rgb(53, 58, 66);
const TRACK: Color32 = Color32::from_rgb(29, 32, 37);
const INK: Color32 = Color32::from_rgb(16, 18, 20);
const SEAM: Color32 = Color32::from_rgb(16, 18, 21);
const GRID: Color32 = Color32::from_rgb(49, 53, 60);
/// How solid the blue and amber bars are.
const BAR_ALPHA: f32 = 0.6;
const RED: Color32 = Color32::from_rgb(224, 82, 79);
/// Group borders, in the order of their numbers.
const GROUP_COLORS: [Color32; 8] = [
    Color32::from_rgb(181, 140, 255),
    Color32::from_rgb(240, 139, 192),
    Color32::from_rgb(92, 200, 176),
    Color32::from_rgb(230, 200, 90),
    Color32::from_rgb(120, 190, 250),
    Color32::from_rgb(250, 150, 100),
    Color32::from_rgb(170, 220, 110),
    Color32::from_rgb(210, 170, 230),
];

const YEAR: f64 = 31_557_600.0;

fn row_height(kind: &Kind) -> f32 {
    kind.rows().iter().find(|(k, _)| k == "row height").and_then(|(_, v)| v.parse().ok()).unwrap_or(ROW_DEFAULT).clamp(ROW_MIN, ROW_MAX)
}

/// The order the rows are drawn in; the free row is always last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// By group, then rows with time metadata before those without, then by start.
    GroupMetaStart,
    /// By group, then by phone id (rows with none last), then by start: a phone's files together.
    GroupPhoneStart,
    /// By start, then by group.
    StartGroup,
    /// By group, then by how far the metadata is from audio, nearest first.
    GroupOff,
    /// As wired.
    Wired,
}

impl Order {
    pub const ALL: [Order; 5] = [Order::GroupMetaStart, Order::GroupPhoneStart, Order::StartGroup, Order::GroupOff, Order::Wired];

    /// As saved in the node's rows (`order`).
    fn key(self) -> &'static str {
        match self {
            Order::GroupMetaStart => "group-meta-start",
            Order::GroupPhoneStart => "group-phone-start",
            Order::StartGroup => "start-group",
            Order::GroupOff => "group-off",
            Order::Wired => "wired",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Order::GroupMetaStart => "Group › metadata › start",
            Order::GroupPhoneStart => "Group › phone › start",
            Order::StartGroup => "Start › group",
            Order::GroupOff => "Group › how far off",
            Order::Wired => "As wired",
        }
    }
}

/// The node's order; a project saved with `order by group = true` reads as group, metadata, start.
fn order_of(kind: &Kind) -> Order {
    let rows = kind.rows();
    if let Some((_, v)) = rows.iter().find(|(k, _)| k == "order") {
        return Order::ALL.into_iter().find(|o| o.key() == v).unwrap_or(Order::Wired);
    }
    if rows.iter().any(|(k, v)| k == "order by group" && v == "true") { Order::GroupMetaStart } else { Order::Wired }
}

/// The rows in the order they are drawn: row indices, top to bottom.
pub fn sorted(v: &MatchView, order: Order) -> Vec<usize> {
    let group = |k: usize| match v.groups[v.rows[k].group].number {
        0 => usize::MAX,
        n => n,
    };
    let start = |k: usize| v.rows[k].start.or(v.rows[k].meta).unwrap_or(f64::MAX);
    let off = |k: usize| v.rows[k].off.map_or(f64::MAX, f64::abs);
    let no_meta = |k: usize| v.rows[k].meta.is_none();
    let phone = |k: usize| (v.rows[k].phone.is_empty(), v.rows[k].phone.as_str());
    let mut o: Vec<usize> = (0..v.rows.len()).collect();
    o.sort_by(|&a, &b| {
        let by = match order {
            Order::GroupMetaStart => group(a).cmp(&group(b)).then(no_meta(a).cmp(&no_meta(b))).then(start(a).total_cmp(&start(b))),
            Order::GroupPhoneStart => group(a).cmp(&group(b)).then(phone(a).cmp(&phone(b))).then(start(a).total_cmp(&start(b))),
            Order::StartGroup => start(a).total_cmp(&start(b)).then(group(a).cmp(&group(b))),
            Order::GroupOff => group(a).cmp(&group(b)).then(off(a).total_cmp(&off(b))),
            Order::Wired => std::cmp::Ordering::Equal,
        };
        (!v.rows[a].wired).cmp(&!v.rows[b].wired).then(by).then(a.cmp(&b))
    });
    o
}

/// Where a row's parts are, unzoomed from the top of its place.
struct RowGeom {
    strip: f32,
    half: f32,
}

fn geom(rh: f32) -> RowGeom {
    RowGeom { strip: STRIP_H, half: ((rh - 2.0 - STRIP_H) / 2.0).max(0.0) }
}

/// "View all": a click fits the axis to every bar. The circle inside it turns live on and off:
/// while live, the axis follows the bars. Returns (the button was pressed, the circle was).
pub(crate) fn scale_button(ui: &mut egui::Ui, id: NodeId, rect: Rect, z: f32, live: bool) -> (bool, bool) {
    const LABEL: &str = "View all";
    let centre = Pos2::new(rect.right() - 11.0 * z, rect.center().y);
    let dot = Rect::from_center_size(centre, Vec2::splat(18.0 * z));
    let main = ui.interact(rect, egui::Id::new(("scale-to-all", id)), Sense::click());
    main.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, LABEL));
    let circle = ui.interact(dot, egui::Id::new(("scale-live", id)), Sense::click());
    circle.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, live, "live"));
    let fill = if live {
        Color32::from_rgb(81, 88, 98)
    } else if main.hovered() {
        Color32::from_rgb(70, 76, 85)
    } else {
        Color32::from_rgb(58, 63, 71)
    };
    let p = ui.painter();
    p.rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    p.text(Pos2::new(rect.left() + 8.0 * z, rect.center().y), Align2::LEFT_CENTER, LABEL, FontId::proportional(11.5 * z), TEXT);
    let r = 4.5 * z;
    if live {
        p.circle_filled(centre, r, TEXT);
    } else {
        p.circle_stroke(centre, r, Stroke::new(1.2 * z, if circle.hovered() { TEXT } else { DIM }));
    }
    let pressed = (main.clicked(), circle.clicked());
    main.on_hover_text("scale the time axis to show every bar");
    circle.on_hover_text(if live { "live: the axis follows the bars. Click to stop" } else { "make it live: the axis follows the bars" });
    pressed
}

/// The row height slider: zoomed with the node like everything on it, so it keeps its place
/// beside the legend. `at`: the middle of its left end.
pub(crate) fn row_slider(ui: &mut egui::Ui, at: Pos2, z: f32, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    const W: f32 = 110.0;
    const H: f32 = 18.0;
    let rect = Rect::from_min_size(at - Vec2::new(0.0, H * 0.5 * z), Vec2::new(W * z, H * z));
    let mut s = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt("row-height"));
    s.spacing_mut().slider_width = W * z;
    s.add(egui::Slider::new(value, range).show_value(false));
}

fn place_y(slot: usize, rh: f32) -> f32 {
    TOP + BAR_H + RULER_H + slot as f32 * (rh + END_H)
}

/// `+12 years`, `-3 min`, `+40.2 s`.
fn gap_text(s: f64) -> String {
    let a = s.abs();
    let sign = if s < 0.0 { "-" } else { "+" };
    if a < 120.0 {
        format!("{sign}{a:.1} s")
    } else if a < 7200.0 {
        format!("{sign}{:.0} min", a / 60.0)
    } else if a < 172_800.0 {
        format!("{sign}{:.0} h", a / 3600.0)
    } else if a < 2.0 * 30.44 * 86_400.0 {
        format!("{sign}{:.0} days", a / 86_400.0)
    } else if a < YEAR {
        format!("{sign}{:.0} months", a / (30.44 * 86_400.0))
    } else {
        format!("{sign}{:.1} years", a / YEAR)
    }
}

fn hsl(h: f32, s: f32, l: f32) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let u = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(u(r), u(g), u(b))
}

/// How far off fills a whole half of the Status cell, seconds; less fills part of it, from the
/// middle out.
const FILL_S: f64 = 5.0;

/// Green at 0 s, fully red at 30 s, going black by a year: the cell's fill and text.
fn off_color(d: f64) -> (Color32, Color32) {
    let a = d.abs();
    if a <= 30.0 {
        return (hsl(130.0 * (1.0 - a as f32 / 30.0), 0.62, 0.46), INK);
    }
    let f = ((a / 30.0).ln() / (YEAR / 30.0).ln()).min(1.0) as f32;
    (hsl(0.0, 0.65, 0.46 * (1.0 - f)), if f > 0.3 { Color32::WHITE } else { INK })
}

/// How far a drift fills its half of the Drift cell, 0 to 1, on a log scale: 0.1 s/h (30 ppm, a
/// phone's clock) fills half, 3.6 s/h (1000 ppm, `SPEED_CHANGE_PPM`, a file played at another
/// speed) all of it. Takes ppm.
fn speed_fill(ppm: f64) -> f32 {
    ((1.0 + ppm.abs()).ln() / (1.0 + audio_match::SPEED_CHANGE_PPM).ln()).min(1.0) as f32
}

/// Green near 0, red toward 3.6 s/h. Takes ppm.
fn speed_color(ppm: f64) -> Color32 {
    hsl(130.0 * (1.0 - speed_fill(ppm)), 0.62, 0.46)
}

/// What a row's Drift cell says.
#[derive(Clone, Debug, PartialEq)]
pub enum DriftCell {
    /// Its speed reference button is on: its clock runs true.
    True,
    /// Its drift in ppm (shown in s/h), the error, and how far the row's wired end is from the
    /// measured end.
    Speed(f64, f64, Option<f64>),
    Empty,
}

/// The Drift cell of row `k`.
pub fn drift_cell(v: &MatchView, k: usize) -> DriftCell {
    let r = &v.rows[k];
    if !v.linked(r) {
        return DriftCell::Empty;
    }
    match r.speed {
        _ if v.groups[r.group].drift == DriftFrom::File(k) => DriftCell::True,
        Some((p, e)) => DriftCell::Speed(p, e, r.end_in.zip(r.end).map(|(w, e)| w - e)),
        None => DriftCell::Empty,
    }
}

/// A finding's mark, drawn in `r`: two squares (a copy), two chevrons (another speed), a broken
/// line (a gap).
fn mark_icon(p: &egui::Painter, r: Rect, kind: MarkKind, col: Color32) {
    let at = |x: f32, y: f32| r.min + Vec2::new(x / 16.0 * r.width(), y / 16.0 * r.height());
    let st = Stroke::new((r.width() * 0.1).max(1.0), col);
    match kind {
        MarkKind::Same => {
            p.rect_stroke(Rect::from_min_max(at(1.0, 1.0), at(11.0, 11.0)), 0.0, st, StrokeKind::Inside);
            p.rect_stroke(Rect::from_min_max(at(5.0, 5.0), at(15.0, 15.0)), 0.0, st, StrokeKind::Inside);
        }
        MarkKind::Speed => {
            for dx in [0.0, 6.0] {
                p.add(egui::Shape::line(vec![at(2.0 + dx, 2.0), at(8.0 + dx, 8.0), at(2.0 + dx, 14.0)], st));
            }
        }
        MarkKind::Gap => {
            p.line_segment([at(0.0, 8.0), at(5.0, 8.0)], st);
            p.line_segment([at(11.0, 8.0), at(16.0, 8.0)], st);
            p.line_segment([at(5.0, 3.0), at(5.0, 13.0)], st);
            p.line_segment([at(11.0, 3.0), at(11.0, 13.0)], st);
        }
    }
}

/// Diagonal lines across `r`, `gap` apart.
fn hatch(p: &egui::Painter, r: Rect, gap: f32, st: Stroke) {
    let c = p.with_clip_rect(r.intersect(p.clip_rect()));
    let mut x = r.left() - r.height();
    while x < r.right() {
        c.line_segment([Pos2::new(x, r.bottom()), Pos2::new(x + r.height(), r.top())], st);
        x += gap;
    }
}

/// What a row's status cell says.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Plain(String),
    Bad(String),
    /// Seconds off, and whether that is about its own length.
    Off(f64, bool),
    /// The row holding its group's anchor.
    Anchor,
    Empty,
}

/// The status cell of row `k`, and whether the whole row is greyed.
pub fn cell(v: &MatchView, k: usize) -> (Cell, bool) {
    let r = &v.rows[k];
    if !r.wired {
        return (Cell::Empty, false);
    }
    if r.time_err.is_some() {
        return (Cell::Bad("Time node error".into()), false);
    }
    if let Some(Stage::Failed(_)) = &r.stage {
        return (Cell::Bad("failed".into()), false);
    }
    if r.uid.is_some() && !r.file {
        return (Cell::Plain("files only".into()), false);
    }
    if v.running && !v.done {
        return (
            match &r.stage {
                Some(Stage::Waiting(s)) if *s > 0.0 => Cell::Plain(format!("decoding {s:.0} s")),
                Some(Stage::Waiting(_)) => Cell::Plain("waiting".into()),
                Some(Stage::Fingerprinting) => Cell::Plain("fingerprinting".into()),
                Some(Stage::Ready(_)) => Cell::Plain("fingerprinted".into()),
                _ => Cell::Plain("waiting".into()),
            },
            false,
        );
    }
    if !v.done {
        return (Cell::Plain(r.meta.map_or("no time metadata".to_string(), day_and_time)), false);
    }
    let g = &v.groups[r.group];
    if g.anchor == Some(k) {
        return (Cell::Anchor, false);
    }
    let linked = g.rows.iter().filter(|&&q| v.rows[q].uid.is_some()).count() >= 2;
    if !linked {
        // Nothing to place it by: greyed, and it gives nothing.
        return match r.meta {
            Some(_) => (Cell::Plain("no links found".into()), false),
            None => (Cell::Plain("no links found\nno time metadata".into()), true),
        };
    }
    if r.lost && g.anchor.is_none() {
        return (Cell::Plain("lost its anchor".into()), false);
    }
    match r.off {
        Some(d) => (Cell::Off(d, r.hint), false),
        None => (Cell::Plain("no time metadata".into()), false),
    }
}

fn day_and_time(t: f64) -> String {
    let s = timebase::fmt_utc(t);
    format!("{}\n{}", &s[..10.min(s.len())], timebase::fmt_utc_clock(t))
}

/// The axis that fits every bar, however far apart they are.
pub(crate) fn fit_axis(bars: &[(f64, f64)]) -> (f64, f64) {
    if bars.is_empty() {
        return (0.0, 600.0);
    }
    let lo = bars.iter().map(|b| b.0).fold(f64::MAX, f64::min);
    let hi = bars.iter().map(|b| b.1).fold(f64::MIN, f64::max);
    let pad = ((hi - lo) * 0.05).max(1.0);
    (lo - pad, hi + pad)
}

/// A round step giving about six ticks across `span` seconds.
pub(crate) fn tick_step(span: f64) -> f64 {
    let want = span / 6.0;
    [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0, 21600.0, 86400.0, 604800.0, 2629800.0, 7889400.0, YEAR, 2.0 * YEAR, 5.0 * YEAR, 10.0 * YEAR, 50.0 * YEAR, 100.0 * YEAR, 500.0 * YEAR].into_iter().find(|s| *s >= want).unwrap_or(want)
}

/// A tick's label: time of day, then the date, then the year as the ticks grow apart.
pub(crate) fn tick_text(t: f64, step: f64) -> String {
    let (c, d) = (timebase::fmt_utc_clock(t), timebase::fmt_utc(t));
    if step < 60.0 {
        c
    } else if step < 86400.0 {
        c[..5.min(c.len())].to_string()
    } else if step < YEAR {
        d[..10.min(d.len())].to_string()
    } else {
        d[..4.min(d.len())].to_string()
    }
}

/// An anchor, drawn in `r`.
fn anchor_icon(p: &egui::Painter, r: Rect, col: Color32) {
    let at = |x: f32, y: f32| r.min + Vec2::new(x / 16.0 * r.width(), y / 16.0 * r.height());
    let st = Stroke::new((r.width() * 0.11).max(1.0), col);
    p.circle_stroke(at(8.0, 3.2), r.width() * 0.11, st);
    p.line_segment([at(8.0, 5.0), at(8.0, 14.0)], st);
    p.line_segment([at(5.0, 7.5), at(11.0, 7.5)], st);
    let arc: Vec<Pos2> = (0..=12)
        .map(|k| {
            let a = std::f32::consts::PI * (k as f32 / 12.0);
            at(8.0 - 4.8 * a.cos(), 10.4 + 3.4 * a.sin())
        })
        .collect();
    p.add(egui::Shape::line(arc, st));
}

/// A clock face, drawn in `r`: the speed reference (a clock that runs true).
fn clock_icon(p: &egui::Painter, r: Rect, col: Color32) {
    let st = Stroke::new((r.width() * 0.1).max(1.0), col);
    let c = r.center();
    p.circle_stroke(c, r.width() * 0.42, st);
    p.line_segment([c, c + Vec2::new(0.0, -r.height() * 0.3)], st);
    p.line_segment([c, c + Vec2::new(r.width() * 0.22, 0.0)], st);
}

/// An anchor with a bar over it (a mean, x̄), drawn in `r`: the average anchor.
fn average_icon(p: &egui::Painter, r: Rect, col: Color32) {
    let s = r.width().min(r.height());
    let a = Rect::from_center_size(r.center() + Vec2::new(0.0, s * 0.08), Vec2::splat(s * 0.84));
    anchor_icon(p, a, col);
    p.line_segment([Pos2::new(r.center().x - s * 0.36, r.top() + s * 0.06), Pos2::new(r.center().x + s * 0.36, r.top() + s * 0.06)], Stroke::new((s * 0.09).max(1.0), col));
}

/// `s`, shortened until it fits `w` in `font`.
fn fit_width(p: &egui::Painter, s: &str, w: f32, font: FontId) -> String {
    let mut n = s.chars().count();
    let mut t = s.to_string();
    while n > 2 && p.layout_no_wrap(t.clone(), font.clone(), TEXT).size().x > w {
        n -= 1;
        t = shorten(s, n);
    }
    t
}

/// The phone id's hover.
fn phone_tip(r: &Row) -> String {
    let mut t = "phone id: rows with the same text are one phone, in one colour: one drift (each file keeps its own start) and one vote in the group's averages. Empty: its own phone".to_string();
    if !r.phone_hint.is_empty() {
        t += &format!("\nthe metadata says: {} (only a suggestion: Enter in the empty field takes it)", r.phone_hint);
    }
    t
}

/// A mirror, drawn in `r`: a line with a shape and its reflection. It marks a time that may be the
/// recording's end read as its start.
fn mirror_icon(p: &egui::Painter, r: Rect, col: Color32) {
    let at = |x: f32, y: f32| r.min + Vec2::new(x / 16.0 * r.width(), y / 16.0 * r.height());
    let st = Stroke::new((r.width() * 0.09).max(1.0), col);
    p.line_segment([at(8.0, 1.0), at(8.0, 15.0)], st);
    p.add(egui::Shape::convex_polygon(vec![at(6.0, 3.5), at(6.0, 12.5), at(1.0, 12.5)], col, Stroke::NONE));
    p.add(egui::Shape::closed_line(vec![at(10.0, 3.5), at(15.0, 12.5), at(10.0, 12.5)], st));
}

fn shorten(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    } else {
        s.to_string()
    }
}

/// The bits of a row's hover text.
fn row_hover(v: &MatchView, r: &Row) -> String {
    let mut h = vec![r.name.clone()];
    if !r.phone.is_empty() {
        h.push(format!("phone: {}", r.phone));
    }
    match r.meta {
        Some(t) => h.push(format!("time input: {} UTC ± {} s, {}", timebase::fmt_utc(t), r.meta_err, r.meta_source)),
        None if r.time_wired && !r.meta_source.is_empty() => h.push(format!("the time input gives no time: {}", r.meta_source)),
        None if r.time_wired => h.push("the time input gives no time".to_string()),
        None => h.push("nothing wired into the time input".to_string()),
    }
    if let Some(e) = &r.time_err {
        h.push(e.clone());
    }
    if r.len > 0.0 {
        h.push(format!("length {:.2} s", r.len));
    }
    if let Some(t) = r.start {
        let word = if v.anchored(r) { "audio starts" } else { "audio shown at" };
        h.push(format!("{word} {} UTC", timebase::fmt_utc(t)));
    }
    if v.linked(r) {
        let g = &v.groups[r.group];
        h.push(format!("fitted on {} links of this node, leftover {:.1} ms (root mean square)", r.links, r.leftover * 1000.0));
        let k = v.rows.iter().position(|x| std::ptr::eq(x, r));
        match (g.drift, r.speed) {
            (DriftFrom::File(s), _) if Some(s) == k => h.push("speed reference: its clock runs true; the group's drifts are measured against it".to_string()),
            (from, Some((p, e))) => {
                h.push(format!("drift {} ± {:.2} s/h against {}", drift_text(p), s_per_h(e), drift_against(v, g)));
                h.push(scale_words(p));
                if from == DriftFrom::AnchorEnd(k.unwrap_or(usize::MAX)) {
                    h.push("its wired end gives its real drift: every row's drift is against real time".to_string());
                }
            }
            _ => {}
        }
        if let (Some(s0), Some(e)) = (r.start, r.end) {
            h.push(format!("audio ends {} UTC: {} from start + length", timebase::fmt_utc(e), ms_text(e - s0 - r.len)));
            if let Some(w) = r.end_in.filter(|_| g.drift != DriftFrom::AnchorEnd(k.unwrap_or(usize::MAX))) {
                h.push(format!("its wired end is {} from the measured end", ms_text(w - e)));
            }
        }
        if let (None, Some(avg)) = (g.anchor, g.average) {
            h.push(format!("the group is anchored on the average of {} phones' stamps, ± {:.3} s", avg.phones, avg.error));
        }
    }
    for m in &r.marks {
        h.push(m.text.clone());
    }
    if let Some((s, e)) = v.spreads.get(r.group).copied().flatten() {
        h.push(format!("this group's times disagree by about {s:.1} s read as starts, {e:.1} s read as ends"));
    }
    if r.hint {
        h.push("off by about its own length: the time may mark the end of the recording. A Time node can subtract the length".to_string());
    }
    if let Some(Stage::Ready(n)) = &r.stage {
        h.push(format!("{n} fingerprint hashes"));
    }
    if let Some(Stage::Failed(e)) = &r.stage {
        h.push(e.clone());
    }
    h.join("\n")
}

/// How this node looks on the canvas: a row per recording, a timeline in each, a status column and
/// the rows' yellow outputs.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, app: &App, i: NodeId, _standard: Vec2) -> Vec2 {
        let n = app.graph.nodes()[i].spec.inputs.len() / PER_ROW;
        Vec2::new(NODE_W, place_y(n, row_height(&app.nodes[i].kind)) + FOOT_H + 6.0)
    }

    fn port_look(&self, app: &App, i: NodeId, name: &str) -> Option<PortLook> {
        let rh = row_height(&app.nodes[i].kind);
        let g = geom(rh);
        let number = |base: &str| name.strip_prefix(base).and_then(|n| n.parse::<usize>().ok());
        let row = |n: usize| {
            let k = n.saturating_sub(1);
            let slot = app.match_ui.slots.get(&i).and_then(|s| s.get(k)).copied().unwrap_or(k);
            place_y(slot, rh) + 2.0
        };
        // Thin rows have no room below the name: the dots spread over the whole row.
        let lead = g.strip * (g.half / 12.0).min(1.0);
        let room = rh - 2.0 - lead;
        let dot = (g.half * 0.42).clamp(2.5, 5.0);
        let at = |side: Side, y: f32| Some(PortLook { side: Some(side), at: Some(y), label: Some(PortLabel::Hidden), dot_size: Some(dot), ..Default::default() });
        if let Some(n) = number("audio ") {
            return at(Side::Left, row(n) + lead + room * 0.25);
        }
        if let Some(n) = number("time ") {
            return at(Side::Left, row(n) + lead + room * 0.75);
        }
        // The thin stripe under the start time.
        if let Some(n) = number("end in ") {
            let small = PortLook { dot_size: Some(dot.min(4.0)), ..Default::default() };
            return at(Side::Left, row(n) + rh - 2.0 + END_H * 0.5).map(|l| l.with(&small));
        }
        // The outputs: the start time over the end time, in the middle of their fields.
        if let Some(n) = number("synced ") {
            return at(Side::Right, row(n) + lead + (room + END_H) * 0.25);
        }
        if let Some(n) = number("end ") {
            return at(Side::Right, row(n) + lead + (room + END_H) * 0.75);
        }
        None
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let body = ui.max_rect();
        let z = app.zoom;
        let v = app.match_view(i);
        app.note_lost_anchors(i, &v);
        let v = app.match_view(i);
        let rh = row_height(&app.nodes[i].kind);
        let g = geom(rh);
        let font = |s: f32| FontId::proportional(s * z);
        let mono = |s: f32| FontId::monospace(s * z);
        let p = ui.painter().with_clip_rect(ui.clip_rect());
        // The rows' colours run out to the node's edges, past the part a look draws in.
        let (edge_l, edge_r) = (body.left() - EDGE * z, body.right() + EDGE * z);
        let wide = ui.painter().with_clip_rect(ui.clip_rect().expand2(Vec2::new(EDGE * z, 0.0)).intersect(app.canvas_rect));

        // Draw order: as the Order menu says, the free row always last.
        let ord = order_of(&app.nodes[i].kind);
        let order = sorted(&v, ord);
        let mut slots = vec![0; order.len()];
        for (s, &k) in order.iter().enumerate() {
            slots[k] = s;
        }
        app.match_ui.slots.insert(i, slots);

        let x0 = body.left();
        let track_x = (x0 + LEFT_W * z, body.right() - (STATUS_W + DRIFT_W + RIGHT_W) * z);
        let rows_top = body.top() + (BAR_H + RULER_H) * z;
        let pitch = (rh + END_H) * z;
        let row_top = |slot: usize| rows_top + slot as f32 * pitch;
        let rows_bottom = row_top(order.len());
        let track = Rect::from_min_max(Pos2::new(track_x.0, rows_top - RULER_H * z), Pos2::new(track_x.1, rows_bottom));
        if track.width() <= 10.0 {
            return;
        }

        // The axis.
        let bars: Vec<(f64, f64)> = v.rows.iter().flat_map(|r| [r.meta.map(|t| (t, t + r.len)), r.start.map(|t| (t, r.end.unwrap_or(t + r.len).max(t + r.len)))]).flatten().collect();
        let fit = fit_axis(&bars);
        let held = app.match_ui.grab.as_ref().filter(|g| g.0 == i).map(|g| g.2);
        let (v0, v1) = held.or(app.match_ui.views.get(&i).copied()).unwrap_or(fit);
        let span = (v1 - v0).max(1e-3);
        let tx = |t: f64| track.left() + ((t - v0) / span) as f32 * track.width();

        // The bar along the top.
        let always = !app.match_ui.views.contains_key(&i);
        let mut new_ord = ord;
        let mut new_rh = rh;
        let y = body.top() + BAR_H * 0.5 * z;
        let (mut do_fit, toggle_always) = scale_button(ui, i, Rect::from_min_size(Pos2::new(x0 + 4.0 * z, y - 10.0 * z), Vec2::new(82.0 * z, 20.0 * z)), z, always);
        {
            let rect = Rect::from_min_size(Pos2::new(x0 + 90.0 * z, y - 10.0 * z), Vec2::new(196.0 * z, 20.0 * z));
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("match-order", i)));
            c.spacing_mut().button_padding.y = 2.0 * z;
            c.menu_button(egui::RichText::new(format!("Order: {} ⏷", ord.label())).size(11.5 * z), |ui| {
                for o in Order::ALL {
                    if ui.selectable_label(o == ord, o.label()).clicked() {
                        new_ord = o;
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("the order of the rows: the free row is always last");
        }
        p.text(Pos2::new(x0 + 296.0 * z, y), Align2::LEFT_CENTER, "row height", font(11.5), DIM);
        row_slider(ui, Pos2::new(x0 + 358.0 * z, y), z, &mut new_rh, ROW_MIN..=ROW_MAX);
        // The legend: what the bars are.
        let mut lx = x0 + 482.0 * z;
        for (col, text) in [(AMBER, "time calculated from audio"), (BLUE, "time from metadata")] {
            let sw = Rect::from_min_size(Pos2::new(lx, y - 5.0 * z), Vec2::new(14.0 * z, 10.0 * z));
            p.rect_filled(sw, 0.0, alpha(col, BAR_ALPHA));
            let r = p.text(Pos2::new(sw.right() + 5.0 * z, y), Align2::LEFT_CENTER, text, font(11.0), DIM);
            lx = r.right() + 12.0 * z;
        }
        {
            // The drift: an amber bar to a dashed line, hatched past it.
            let sw = Rect::from_min_size(Pos2::new(lx, y - 5.0 * z), Vec2::new(20.0 * z, 10.0 * z));
            p.rect_filled(sw, 0.0, alpha(AMBER, BAR_ALPHA));
            let cut = sw.left() + 13.0 * z;
            hatch(&p, Rect::from_min_max(Pos2::new(cut, sw.top()), sw.max), 3.0 * z, Stroke::new(z, INK));
            p.extend(egui::Shape::dashed_line(&[Pos2::new(cut, sw.top() - 2.0 * z), Pos2::new(cut, sw.bottom() + 2.0 * z)], Stroke::new(z, BOTH), 2.0 * z, 2.0 * z));
            let r = p.text(Pos2::new(sw.right() + 5.0 * z, y), Align2::LEFT_CENTER, "drift: start + length ¦ measured end", font(11.0), DIM);
            lx = r.right() + 12.0 * z;
        }
        let scale = if span < 86400.0 { format!("{} to {}", timebase::fmt_utc_clock(v0), timebase::fmt_utc_clock(v1)) } else { format!("{} to {}", &timebase::fmt_utc(v0)[..10], &timebase::fmt_utc(v1)[..10]) };
        let scale = p.layout_no_wrap(scale, mono(11.5), DIM);
        let at = Pos2::new(body.right() - 8.0 * z - scale.size().x, y - scale.size().y * 0.5);
        // Left out when the legend has its room.
        if v.real && at.x > lx {
            p.galley(at, scale, DIM);
        }
        p.line_segment([Pos2::new(body.left(), body.top() + BAR_H * z), Pos2::new(body.right(), body.top() + BAR_H * z)], Stroke::new(z, SEAM));

        // The ruler, and the grid down the rows.
        p.rect_filled(Rect::from_min_max(track.min, Pos2::new(track.right(), rows_top)), 0.0, TRACK);
        p.rect_filled(Rect::from_min_max(Pos2::new(track.left(), rows_top), track.max), 0.0, TRACK);
        let step = tick_step(span);
        let mut q = (v0 / step).ceil();
        let mut ticks = 0;
        while q * step <= v1 && ticks < 30 {
            let x = tx(q * step);
            p.line_segment([Pos2::new(x, track.top()), Pos2::new(x, track.bottom())], Stroke::new(z.max(1.0), GRID));
            let label = if v.real { tick_text(q * step, step) } else { format!("{:+.0} s", q * step) };
            p.text(Pos2::new(x + 4.0 * z, track.top() + RULER_H * 0.5 * z), Align2::LEFT_CENTER, label, mono(11.0), DIM);
            q += 1.0;
            ticks += 1;
        }
        p.text(Pos2::new(track.right() + STATUS_W * 0.5 * z, track.top() + RULER_H * 0.5 * z), Align2::CENTER_CENTER, "Status", font(11.0), DIM);
        p.text(Pos2::new(track.right() + (STATUS_W + DRIFT_W * 0.5) * z, track.top() + RULER_H * 0.5 * z), Align2::CENTER_CENTER, "Drift", font(11.0), DIM);
        p.text(Pos2::new(track.right() + (STATUS_W + DRIFT_W + 8.0) * z, track.top() + RULER_H * 0.5 * z), Align2::LEFT_CENTER, "output", font(11.0), DIM);

        // The rows.
        let mut press_anchor: Option<usize> = None;
        let mut press_speed_ref: Option<usize> = None;
        let mut new_phone: Option<(usize, String)> = None;
        for (slot, &k) in order.iter().enumerate() {
            let r = &v.rows[k];
            let top = row_top(slot);
            let (status, dead) = cell(&v, k);
            let fade = |c: Color32| if dead { c.gamma_multiply(0.45) } else { c };
            wide.rect_filled(Rect::from_min_max(Pos2::new(edge_l, top), Pos2::new(edge_r, top + 2.0 * z)), 0.0, SEAM);
            let inner = top + 2.0 * z;
            let strip_h = g.strip * z;
            let half = g.half * z;
            let bottom = top + pitch;
            let left_col = Rect::from_min_max(Pos2::new(x0, inner), Pos2::new(track.left(), bottom));
            let words = g.half >= WORDS_FROM;
            // The optional end input: a thin stripe under the start time, grey until wired.
            let end_sw = Rect::from_min_max(Pos2::new(edge_l, top + rh * z), Pos2::new(track.left() - z, bottom));
            let end_words = |p: &egui::Painter, on: bool| {
                if END_H * z >= 8.0 {
                    let text = if on { "audio end time" } else { "audio end time (optional)" };
                    p.text(Pos2::new(x0 + 21.0 * z, end_sw.center().y), Align2::LEFT_CENTER, text, font(9.5), if on { INK } else { DIM });
                }
            };
            if !r.wired {
                // The free row waits for a wire.
                let d = alpha(DIM, 0.8);
                if words {
                    p.text(Pos2::new(x0 + 14.0 * z, inner + strip_h + half * 0.5), Align2::LEFT_CENTER, format!("audio {}", r.n), font(12.0), d);
                    p.text(Pos2::new(x0 + 14.0 * z, inner + strip_h + half * 1.5), Align2::LEFT_CENTER, "audio start time", font(12.0), d);
                } else {
                    p.text(Pos2::new(x0 + 14.0 * z, (inner + top + rh * z) * 0.5), Align2::LEFT_CENTER, format!("audio {}, audio start time", r.n), font(11.5), d);
                }
                wide.rect_filled(end_sw, 0.0, alpha(OPT, 0.6));
                end_words(&p, false);
                continue;
            }
            // With no room on the ports, the anchor and speed reference buttons sit beside the
            // name, and the phone id shrinks to its colour chip.
            let thin = g.half < ANCHOR_FROM;
            let by_name = thin && r.meta.is_some();
            let linked = v.linked(r);
            wide.rect_filled(Rect::from_min_max(Pos2::new(edge_l, inner), Pos2::new(track.left(), inner + strip_h)), 0.0, fade(STRIP));
            // From the right: the buttons beside the name, then the phone id, then the name.
            let small = strip_h - 6.0 * z;
            let mut x = track.left() - 2.0 * z;
            let mut beside = |on: bool| {
                on.then(|| {
                    let b = Rect::from_center_size(Pos2::new(x - small * 0.5, inner + strip_h * 0.5), Vec2::splat(small));
                    x -= small + 3.0 * z;
                    b
                })
            };
            let anchor_by_name = beside(by_name);
            let ref_by_name = beside(thin && linked);
            let phone_w = if thin { strip_h - 8.0 * z } else { PHONE_W * z };
            let phone_r = r.uid.is_some().then(|| {
                let pr = Rect::from_min_max(Pos2::new(x - phone_w, inner + 3.0 * z), Pos2::new(x, inner + strip_h - 3.0 * z));
                x = pr.left() - 4.0 * z;
                pr
            });
            p.text(left_col.min + Vec2::new(8.0 * z, strip_h * 0.5), Align2::LEFT_CENTER, fit_width(&p, &r.name, x - left_col.left() - 8.0 * z, mono(11.5)), mono(11.5), fade(TEXT));
            // The phone id: rows with the same text are one phone, in one colour.
            if let (Some(pr), Some(uid)) = (phone_r, r.uid.clone()) {
                let id = egui::Id::new(("match-phone", i, uid.as_str()));
                let typing = app.match_ui.phone_edit.as_ref().filter(|e| e.0 == i && e.1 == uid).map(|e| e.2.clone());
                let tip = phone_tip(r);
                app.match_ui.phone_at.insert((i, k), pr);
                if thin && typing.is_none() {
                    match (!r.phone.is_empty()).then(|| phone_color(&r.phone)) {
                        Some(c) => {
                            p.rect_filled(pr, 2.0 * z, fade(c));
                        }
                        None => {
                            p.rect_stroke(pr, 2.0 * z, Stroke::new(z, DIM), StrokeKind::Inside);
                        }
                    }
                    if ui.interact(pr, egui::Id::new(("match-phone-chip", i, r.n)), Sense::click()).on_hover_text(tip).clicked() {
                        app.match_ui.phone_edit = Some((i, uid.clone(), r.phone.clone()));
                        ui.memory_mut(|m| m.request_focus(id));
                    }
                } else {
                    // Typed across the name while the strip has no room for it.
                    let fr = if thin { Rect::from_min_max(Pos2::new(left_col.left() + 4.0 * z, pr.top()), pr.max) } else { pr };
                    let mut text = typing.clone().unwrap_or_else(|| r.phone.clone());
                    let col = (!text.trim().is_empty()).then(|| phone_color(text.trim()));
                    p.rect_filled(fr, 2.0 * z, fade(col.unwrap_or(TRACK)));
                    let hint = if r.phone_hint.is_empty() { "phone" } else { r.phone_hint.as_str() };
                    let te = egui::TextEdit::singleline(&mut text)
                        .id(id)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(3, 0)))
                        .font(mono(10.5))
                        .text_color(if col.is_some() { INK } else { TEXT })
                        .hint_text(egui::RichText::new(hint).font(mono(10.5)).color(alpha(DIM, 0.8)));
                    let resp = ui.put(fr, te).on_hover_text(tip);
                    if resp.has_focus() {
                        app.match_ui.phone_edit = Some((i, uid.clone(), text.clone()));
                    }
                    if resp.lost_focus() {
                        // Enter in the empty field takes the metadata's suggestion.
                        let take = text.trim().is_empty() && !r.phone_hint.is_empty() && ui.input(|x| x.key_pressed(egui::Key::Enter));
                        new_phone = Some((k, if take { r.phone_hint.clone() } else { text.clone() }));
                        app.match_ui.phone_edit = None;
                    }
                }
            }
            // The port swatches: audio over time.
            let audio_sw = Rect::from_min_max(Pos2::new(edge_l, inner + strip_h), Pos2::new(track.left() - z, inner + strip_h + half));
            let time_sw = Rect::from_min_max(Pos2::new(edge_l, audio_sw.bottom()), Pos2::new(track.left() - z, audio_sw.bottom() + half));
            wide.rect_filled(audio_sw, 0.0, fade(if r.uid.is_some() { AMBER } else { OFF }));
            wide.rect_filled(time_sw, 0.0, fade(if r.meta.is_some() { BLUE } else { OFF }));
            wide.rect_filled(Rect::from_min_max(end_sw.min, Pos2::new(end_sw.right(), end_sw.top() + z)), 0.0, SEAM);
            wide.rect_filled(Rect::from_min_max(end_sw.min + Vec2::new(0.0, z), end_sw.max), 0.0, fade(if r.end_in.is_some() { BLUE } else { OPT }));
            end_words(&p, r.end_in.is_some());
            if words {
                p.text(Pos2::new(x0 + 21.0 * z, audio_sw.center().y), Align2::LEFT_CENTER, "audio", font(12.0), if r.uid.is_some() { INK } else { DIM });
                p.text(Pos2::new(x0 + 21.0 * z, time_sw.center().y), Align2::LEFT_CENTER, "audio start time", font(12.0), if r.meta.is_some() { INK } else { DIM });
            }
            // The anchor button, on a time, once the audio has placed the row.
            let port_button = |sw: Rect| {
                let s = (half - 3.0 * z).min(20.0 * z).max(8.0 * z);
                Rect::from_center_size(Pos2::new(sw.right() - s * 0.5 - 5.0 * z, sw.center().y), Vec2::splat(s))
            };
            if r.meta.is_some() {
                let br = anchor_by_name.unwrap_or_else(|| port_button(time_sw));
                let s = br.width();
                app.match_ui.anchor_at.insert((i, k), br);
                let on = v.groups[r.group].anchor == Some(k);
                if v.done {
                    let tip = if on { "this time is the anchor of its synced group: click to take it away" } else { "anchor this synced group on this time" };
                    let resp = ui.interact(br, egui::Id::new(("match-anchor", i, r.n)), Sense::click()).on_hover_text(tip);
                    if resp.clicked() {
                        press_anchor = Some(k);
                    }
                }
                if on {
                    p.rect_filled(br, 0.0, INK);
                }
                // Dark on the blue port, light on the grey name strip.
                let ink = if by_name { TEXT } else { INK };
                p.rect_stroke(br, 0.0, Stroke::new(1.5 * z, if v.done { ink } else { alpha(ink, 0.4) }), StrokeKind::Inside);
                anchor_icon(&p, br.shrink(s * 0.14), if on { BOTH } else if v.done { ink } else { alpha(ink, 0.4) });
            }

            // The speed reference button, on the audio, once audio has linked the row: its clock
            // runs true, and the group's drifts are measured against it.
            if linked {
                let br = ref_by_name.unwrap_or_else(|| port_button(audio_sw));
                let on = r.speed_ref;
                let tip = if on { "speed reference: this file's clock runs true. Click to measure the drifts against the group's average clock again" } else { "speed reference: this file's clock runs true (an official recording, say); the group's drifts are measured against it instead of its average clock" };
                if ui.interact(br, egui::Id::new(("match-speed-ref", i, r.n)), Sense::click()).on_hover_text(tip).clicked() {
                    press_speed_ref = Some(k);
                }
                let ink = if thin { TEXT } else { INK };
                if on {
                    p.rect_filled(br, 0.0, INK);
                }
                p.rect_stroke(br, 0.0, Stroke::new(1.5 * z, ink), StrokeKind::Inside);
                clock_icon(&p, br.shrink(br.width() * 0.16), if on { BOTH } else { ink });
            }

            // The timeline.
            let lane = Rect::from_min_max(Pos2::new(track.left(), inner), Pos2::new(track.right(), bottom));
            let band = |b: (f64, f64)| {
                let (a, c) = (tx(b.0).max(lane.left()), tx(b.1).min(lane.right()));
                (c > a || (b.0 <= v1 && b.1 >= v0)).then(|| Rect::from_min_max(Pos2::new(a, lane.top()), Pos2::new(c.max(a + 2.0 * z), lane.bottom())))
            };
            let blue = r.meta.map(|t| (t, t + r.len));
            let amber = r.start.map(|t| (t, r.end.unwrap_or(t + r.len)));
            let (mut gone, mut shown) = (None, false);
            // See-through, so an overlap shows both bars.
            for (bar, col) in [(blue, BLUE), (amber, AMBER)] {
                let Some(b) = bar else { continue };
                match band(b).filter(|_| b.1 >= v0 && b.0 <= v1) {
                    Some(rect) => {
                        p.rect_filled(rect, 0.0, fade(alpha(col, BAR_ALPHA)));
                        shown = true;
                    }
                    None => {
                        let g = if b.1 < v0 { b.1 - v0 } else { b.0 - v1 };
                        gone = Some(gone.map_or(g, |o: f64| if g.abs() < o.abs() { g } else { o }));
                    }
                }
            }
            if dead {
                p.text(Pos2::new(lane.left() + 8.0 * z, lane.center().y), Align2::LEFT_CENTER, "no metadata, and audio found no match: nothing to place it by", font(11.0), fade(DIM));
            }
            if let (Some(b), true) = (amber, v.anchored(r)) {
                let x = tx(b.1);
                if x > lane.left() + 16.0 * z && x <= lane.right() + 16.0 * z {
                    let s = (13.0 * z).min(lane.height() - 4.0 * z).max(6.0 * z);
                    anchor_icon(&p, Rect::from_min_size(Pos2::new(x - s - 3.0 * z, lane.top() + 2.0 * z), Vec2::splat(s)), INK);
                }
            }
            if let Some(g) = gone.filter(|_| !shown) {
                let (at, side, text) = if g < 0.0 { (lane.left() + 6.0 * z, Align2::LEFT_CENTER, format!("< {}", gap_text(g))) } else { (lane.right() - 6.0 * z, Align2::RIGHT_CENTER, format!("{} >", gap_text(g))) };
                p.text(Pos2::new(at, lane.center().y), side, text, mono(11.0), DIM);
            }
            // The stretch: a dashed line where start + length would end, the bit of bar past it
            // hatched, a bar ending short of it cut; at least 4 px either way, so it shows at any
            // zoom. The size beside it. The speed reference has none.
            if let (Some(s0), Some(e), true) = (r.start, r.end, v.linked(r) && v.groups[r.group].drift != DriftFrom::File(k)) {
                let d = e - (s0 + r.len);
                let (xn, xe) = (tx(s0 + r.len), tx(e));
                if xn > lane.left() && xn < lane.right() {
                    let w = (xe - xn).abs().max(4.0 * z);
                    let piece = if d >= 0.0 { Rect::from_min_max(Pos2::new(xn, lane.top()), Pos2::new(xn + w, lane.bottom())) } else { Rect::from_min_max(Pos2::new(xn - w, lane.top()), Pos2::new(xn, lane.bottom())) };
                    if d >= 0.0 {
                        p.rect_filled(piece, 0.0, fade(alpha(AMBER, BAR_ALPHA)));
                        hatch(&p, piece, 3.0 * z, Stroke::new(z, fade(INK)));
                    } else {
                        p.rect_filled(piece, 0.0, TRACK);
                        p.rect_stroke(piece, 0.0, Stroke::new(z, fade(alpha(AMBER, BAR_ALPHA))), StrokeKind::Inside);
                    }
                    p.extend(egui::Shape::dashed_line(&[Pos2::new(xn, lane.top()), Pos2::new(xn, lane.bottom())], Stroke::new(z, fade(BOTH)), 3.0 * z, 2.0 * z));
                    let text = p.layout_no_wrap(format!("end {}", ms_text(d)), mono(10.5), fade(TEXT));
                    let right = piece.right().max(xn) + 4.0 * z;
                    let at = if right + text.size().x < lane.right() { right } else { piece.left().min(xn) - 4.0 * z - text.size().x };
                    p.galley(Pos2::new(at, lane.bottom() - 4.0 * z - text.size().y), text, TEXT);
                }
            }

            // The status cell.
            let cell_r = Rect::from_min_max(Pos2::new(track.right() + z, inner), Pos2::new(track.right() + STATUS_W * z, bottom));
            match &status {
                Cell::Plain(t) => {
                    let t = if rh < 44.0 { t.lines().last().unwrap_or(t).to_string() } else { t.clone() };
                    // Shrunk until the widest line fits the cell.
                    let mut size = 12.0;
                    while size > 7.0 && p.layout_no_wrap(t.clone(), mono(size), DIM).size().x > cell_r.width() - 6.0 * z {
                        size -= 0.5;
                    }
                    p.text(cell_r.center(), Align2::CENTER_CENTER, t, mono(size), fade(DIM));
                }
                Cell::Bad(t) => {
                    p.text(cell_r.center(), Align2::CENTER_CENTER, t, mono(12.0), RED);
                }
                Cell::Off(d, hint) => {
                    // Split at zero: metadata earlier than audio fills the left half, later the
                    // right, so a column shows from afar whether a group is behind, ahead or both.
                    let (bg, fg) = off_color(*d);
                    let mid = cell_r.center().x;
                    let left = Rect::from_min_max(cell_r.min, Pos2::new(mid, cell_r.bottom()));
                    let right = Rect::from_min_max(Pos2::new(mid, cell_r.top()), cell_r.max);
                    let (full, empty) = if *d < 0.0 { (left, right) } else { (right, left) };
                    // The colour grows out from the middle: the whole half at FILL_S and beyond.
                    let w = (full.width() * (d.abs() / FILL_S).min(1.0) as f32).max(2.0 * z);
                    let fill = if *d < 0.0 { Rect::from_min_max(Pos2::new(mid - w, full.top()), Pos2::new(mid, full.bottom())) } else { Rect::from_min_max(Pos2::new(mid, full.top()), Pos2::new(mid + w, full.bottom())) };
                    p.rect_filled(fill, 0.0, bg);
                    // Dark on a full half, light while the half is mostly the track.
                    let ink = if d.abs() >= FILL_S { fg } else { TEXT };
                    let text = gap_text(*d);
                    let mut size = 13.0;
                    let fits = |s: f32| p.layout_no_wrap(text.clone(), mono(s), ink).size().x <= full.width() - 4.0 * z;
                    while size > 7.0 && !fits(size) {
                        size -= 0.5;
                    }
                    p.text(full.center(), Align2::CENTER_CENTER, text, mono(size), ink);
                    p.line_segment([Pos2::new(mid, cell_r.top()), Pos2::new(mid, cell_r.bottom())], Stroke::new(1.0, GRID));
                    if *hint {
                        let s = (16.0 * z).min(cell_r.height() - 4.0 * z);
                        let icon = Rect::from_center_size(empty.center(), Vec2::splat(s));
                        mirror_icon(&p, icon, TEXT);
                        ui.interact(icon, egui::Id::new(("match-mirror", i, r.n)), Sense::hover()).on_hover_text("off by about its own length: the time may mark the end of the recording, not its start. A Time node can subtract the length");
                    }
                }
                Cell::Anchor => {
                    let s = (22.0 * z).min(cell_r.height() - 2.0 * z);
                    anchor_icon(&p, Rect::from_center_size(cell_r.center(), Vec2::splat(s)), TEXT);
                }
                Cell::Empty => {}
            }
            // What the match found on it: marks in the cell's empty half (or its lower right),
            // the words on hover.
            if !r.marks.is_empty() {
                let s = (13.0 * z).min(cell_r.height() * 0.4);
                let kinds: Vec<MarkKind> = [MarkKind::Same, MarkKind::Speed, MarkKind::Gap].into_iter().filter(|k| r.marks.iter().any(|m| m.kind == *k)).collect();
                let room = match &status {
                    Cell::Off(d, _) if *d < 0.0 => Rect::from_min_max(Pos2::new(cell_r.center().x, cell_r.top()), cell_r.max),
                    Cell::Off(..) => Rect::from_min_max(cell_r.min, Pos2::new(cell_r.center().x, cell_r.bottom())),
                    _ => cell_r,
                };
                let row_w = kinds.len() as f32 * (s + 3.0 * z) - 3.0 * z;
                let mut mx = if matches!(status, Cell::Off(..)) { room.center().x - row_w * 0.5 } else { room.right() - row_w - 4.0 * z };
                let my = room.bottom() - s - 3.0 * z;
                for kind in kinds {
                    let at = Rect::from_min_size(Pos2::new(mx, my), Vec2::splat(s));
                    mark_icon(&p, at, kind, AMBER);
                    let words: Vec<&str> = r.marks.iter().filter(|m| m.kind == kind).map(|m| m.text.as_str()).collect();
                    ui.interact(at.expand(2.0 * z), egui::Id::new(("match-mark", i, r.n, kind as u8)), Sense::hover()).on_hover_text(words.join("\n"));
                    mx += s + 3.0 * z;
                }
            }

            // The Drift cell: how fast the row's clock runs, in seconds an hour, against its
            // group's average clock (or its speed reference, or real time), split at zero like
            // Status (slower on the left), green near 0, red toward 3.6 s/h.
            let st_r = Rect::from_min_max(Pos2::new(cell_r.right() + z, inner), Pos2::new(cell_r.right() + DRIFT_W * z, bottom));
            let fit_text = |p: &egui::Painter, t: &str, w: f32, size: f32| {
                let mut size = size;
                while size > 7.0 && p.layout_no_wrap(t.to_string(), mono(size), TEXT).size().x > w - 4.0 * z {
                    size -= 0.5;
                }
                size
            };
            let drift = drift_cell(&v, k);
            match drift {
                DriftCell::True => {
                    let s = (16.0 * z).min(st_r.height() * 0.45);
                    let icon = Rect::from_center_size(Pos2::new(st_r.center().x, st_r.center().y - s * 0.45), Vec2::splat(s));
                    clock_icon(&p, icon, fade(TEXT));
                    let t = "runs true";
                    p.text(Pos2::new(st_r.center().x, icon.bottom() + 2.0 * z), Align2::CENTER_TOP, t, mono(fit_text(&p, t, st_r.width(), 11.0)), fade(DIM));
                }
                DriftCell::Speed(ppm, _, wired) => {
                    let mid = st_r.center().x;
                    let full = if ppm < 0.0 { Rect::from_min_max(st_r.min, Pos2::new(mid, st_r.bottom())) } else { Rect::from_min_max(Pos2::new(mid, st_r.top()), st_r.max) };
                    let w = (full.width() * speed_fill(ppm)).max(2.0 * z);
                    let fill = if ppm < 0.0 { Rect::from_min_max(Pos2::new(mid - w, full.top()), Pos2::new(mid, full.bottom())) } else { Rect::from_min_max(Pos2::new(mid, full.top()), Pos2::new(mid + w, full.bottom())) };
                    p.rect_filled(fill, 0.0, fade(speed_color(ppm)));
                    let ink = if speed_fill(ppm) >= 0.95 { INK } else { TEXT };
                    let t = format!("{}\ns/h", drift_text(ppm));
                    let size = t.lines().map(|l| fit_text(&p, l, full.width(), 12.0)).fold(12.0, f32::min);
                    p.text(full.center(), Align2::CENTER_CENTER, t, mono(size), fade(ink));
                    p.line_segment([Pos2::new(mid, st_r.top()), Pos2::new(mid, st_r.bottom())], Stroke::new(1.0, GRID));
                    // A wired end, against the measured one: in the empty half.
                    if let Some(off) = wired {
                        let empty = if ppm < 0.0 { Rect::from_min_max(Pos2::new(mid, st_r.top()), st_r.max) } else { Rect::from_min_max(st_r.min, Pos2::new(mid, st_r.bottom())) };
                        let t = format!("wired\nend\n{}", ms_text(off));
                        let size = t.lines().map(|l| fit_text(&p, l, empty.width(), 10.0)).fold(10.0, f32::min);
                        p.text(empty.center(), Align2::CENTER_CENTER, t, mono(size), fade(TEXT));
                    }
                }
                DriftCell::Empty => {}
            }
            if drift != DriftCell::Empty {
                ui.interact(st_r, egui::Id::new(("match-drift", i, r.n)), Sense::hover()).on_hover_text(row_hover(&v, r));
            }

            // The output.
            // The outputs: audio start time over audio end time, blue when they give a time, each
            // marked in its upper left: an anchor when the group is anchored, ? when it is placed by
            // metadata or by hand.
            let out_col = Rect::from_min_max(Pos2::new(st_r.right() + z, inner), Pos2::new(edge_r, bottom));
            wide.rect_filled(Rect::from_min_size(out_col.min, Vec2::new(out_col.width(), strip_h)), 0.0, fade(STRIP));
            p.text(out_col.min + Vec2::new(8.0 * z, strip_h * 0.5), Align2::LEFT_CENTER, shorten(&r.name, 15), mono(11.5), fade(TEXT));
            let mark = app.output_mark(i, &v, k);
            let fields_top = inner + strip_h;
            let fh = (out_col.bottom() - fields_top) * 0.5;
            // A row audio did not link passes its wired end straight through.
            let passes = r.end_wired && !v.linked(r);
            for (f, text) in ["audio start time", "audio end time"].into_iter().enumerate() {
                let fr = Rect::from_min_max(Pos2::new(out_col.left(), fields_top + f as f32 * fh), Pos2::new(out_col.right(), fields_top + (f as f32 + 1.0) * fh));
                // The end passes a wired end straight through, whatever the start does.
                let gives = if f == 1 && passes { r.end_in.is_some() } else { mark.is_some() };
                wide.rect_filled(fr, 0.0, fade(if gives { TIME_OUT } else { OFF }));
                if f == 1 {
                    wide.rect_filled(Rect::from_min_max(fr.min, Pos2::new(edge_r, fr.top() + z)), 0.0, SEAM);
                }
                if fh >= 13.0 * z {
                    p.text(Pos2::new(body.right() - 12.0 * z, fr.center().y), Align2::RIGHT_CENTER, text, font(10.5), if gives { INK } else { DIM });
                }
                let s = (18.0 * z).min(fh - 2.0 * z);
                match mark.filter(|_| gives && s >= 6.0 * z && !(f == 1 && passes)) {
                    Some(true) => anchor_icon(&p, Rect::from_min_size(fr.min + Vec2::new(3.0 * z, 1.0 * z), Vec2::splat(s)), INK),
                    Some(false) => {
                        let at = fr.min + Vec2::new(5.0 * z, -1.0 * z);
                        // Drawn twice, a hair apart: bold.
                        for dx in [0.0, 0.8 * z] {
                            p.text(at + Vec2::new(dx, 0.0), Align2::LEFT_TOP, "?", FontId::proportional(s), INK);
                        }
                    }
                    None => {}
                }
            }
        }
        wide.rect_filled(Rect::from_min_max(Pos2::new(edge_l, rows_bottom), Pos2::new(edge_r, rows_bottom + 2.0 * z)), 0.0, SEAM);

        // Group borders: one round each run of neighbouring rows of a group.
        let mut tabs: Vec<(usize, usize, Rect)> = Vec::new();
        for (gi, grp) in v.groups.iter().enumerate().filter(|(_, g)| g.number > 0) {
            let col = GROUP_COLORS[(grp.number - 1) % GROUP_COLORS.len()];
            let mut s = 0;
            while s < order.len() {
                if !grp.rows.contains(&order[s]) {
                    s += 1;
                    continue;
                }
                let mut e = s;
                while e + 1 < order.len() && grp.rows.contains(&order[e + 1]) {
                    e += 1;
                }
                let rect = Rect::from_min_max(Pos2::new(track.left(), row_top(s) + 2.0 * z), Pos2::new(track.right(), row_top(e + 1)));
                p.rect_stroke(rect, 0.0, Stroke::new(2.0 * z, col), StrokeKind::Inside);
                let tab = Rect::from_min_size(rect.min, Vec2::new(20.0 * z, 16.0 * z).min(rect.size()));
                p.rect_filled(tab, 0.0, col);
                p.text(tab.center(), Align2::CENTER_CENTER, grp.number.to_string(), mono(11.0), INK);
                // The average anchor, beside the number: drawn now, pressed after the track (on
                // top of it).
                if v.done && grp.stamps.is_some() && rect.width() > 40.0 * z {
                    let b = Rect::from_min_size(Pos2::new(tab.right(), tab.top()), Vec2::new(18.0 * z, tab.height()));
                    let on = grp.average.is_some();
                    p.rect_filled(b, 0.0, if on { INK } else { col });
                    p.rect_stroke(b, 0.0, Stroke::new(z, col), StrokeKind::Inside);
                    average_icon(&p, b.shrink(1.5 * z), if on { BOTH } else { INK });
                    tabs.push((gi, s, b));
                }
                s = e + 1;
            }
        }

        // The foot: what the node is doing.
        let status_text = app.nodes[i].state.lock().unwrap().status.clone();
        let mut text = if status_text.is_empty() { "not run".to_string() } else { status_text };
        if let Some(l) = v.leftover {
            text += &format!(" · fitted per group: median leftover {:.1} ms", l * 1000.0);
        }
        p.text(Pos2::new(body.left() + 8.0 * z, rows_bottom + 24.0 * z), Align2::LEFT_CENTER, shorten(&text, 120), font(11.5), DIM);

        // The track: the wheel zooms around the pointer, a drag on an unanchored group's amber bar
        // moves the group, any other drag pans, a double-click fits everything.
        app.match_ui.tracks.push(track);
        let resp = ui.interact(Rect::from_min_max(track.min, Pos2::new(track.right(), rows_bottom)), egui::Id::new(("match-track", i)), Sense::click_and_drag());
        let view_now = |app: &App| app.match_ui.views.get(&i).copied().unwrap_or(fit);
        if let Some(h) = resp.hover_pos() {
            let scroll = ui.input(|k| k.smooth_scroll_delta.y + k.smooth_scroll_delta.x);
            if scroll != 0.0 {
                let f = (-scroll as f64 * 0.002).exp();
                let at = v0 + ((h.x - track.left()) / track.width()) as f64 * span;
                app.match_ui.views.insert(i, (at - (at - v0) * f, at + (v1 - at) * f));
            }
        }
        let row_at = |y: f32| -> Option<&Row> {
            let s = ((y - rows_top) / pitch).floor();
            (s >= 0.0).then(|| order.get(s as usize).map(|&k| &v.rows[k])).flatten()
        };
        if resp.drag_started() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let t = v0 + ((pos.x - track.left()) / track.width()) as f64 * span;
                if let Some(r) = row_at(pos.y).filter(|r| r.start.is_some_and(|s| t >= s && t <= s + r.len) && !v.anchored(r)) {
                    let key = v.groups[r.group].key.clone();
                    let before = saved_shift(&app.nodes[i].kind, &key);
                    app.match_ui.grab = Some((i, key, (v0, v1), before));
                }
            }
        }
        if resp.dragged() {
            let dt = (resp.drag_delta().x / track.width()) as f64 * span;
            match app.match_ui.grab.clone().filter(|g| g.0 == i) {
                Some((_, key, _, _)) => {
                    let to = saved_shift(&app.nodes[i].kind, &key) + dt;
                    if let Kind::AudioMatch { rows } = &mut app.nodes[i].kind {
                        set_row(rows, &shift_key(&key), format!("{to:.3}"));
                    }
                }
                None => {
                    let (a, b) = view_now(app);
                    app.match_ui.views.insert(i, (a - dt, b - dt));
                }
            }
        }
        if resp.drag_stopped() {
            // The drag is saved with the project and logged like any setting.
            if let Some((_, key, _, before)) = app.match_ui.grab.take().filter(|g| g.0 == i) {
                let now = saved_shift(&app.nodes[i].kind, &key);
                if now != before {
                    let name = v.groups.iter().find(|g| g.key == key).and_then(|g| g.rows.first()).map_or(key.clone(), |&k| v.rows[k].name.clone());
                    let label = app.nodes[i].label.clone();
                    app.changes.push(bus::SettingChange { utc: timebase::now_unix(), node: app.nodes[i].uid.clone(), label, name: format!("drag of the group with {name}"), old: format!("{before:+.3} s"), new: format!("{now:+.3} s") });
                    app.save_project();
                }
            }
        }
        if resp.double_clicked() {
            do_fit = true;
        }
        let mut press_average = None;
        for (gi, s, b) in tabs {
            let grp = &v.groups[gi];
            let tip = match (grp.average, grp.stamps) {
                (Some(a), _) => format!("anchored on the average of its stamps: {} phones, one vote each, weighted by their ±; ± {:.3} s. Click to take it away", a.phones, a.error),
                (None, Some(a)) => format!("anchor this group on the average of its stamps ({} phones, one vote each, weighted by their ±; every stamp counts): puts a dragged group back, and takes the place of a row's anchor", a.phones),
                _ => String::new(),
            };
            if ui.interact(b, egui::Id::new(("match-average", i, gi, s)), Sense::click()).on_hover_text(tip).clicked() {
                press_average = Some(gi);
            }
        }
        if let Some(r) = resp.hover_pos().and_then(|h| row_at(h.y)).filter(|r| r.wired) {
            resp.on_hover_text(row_hover(&v, r));
        }
        // While live the axis already fits, and it stays live.
        if do_fit && !always {
            app.match_ui.views.insert(i, fit);
        }
        if toggle_always {
            if always {
                app.match_ui.views.insert(i, (v0, v1));
            } else {
                app.match_ui.views.remove(&i);
            }
        }
        let mut changed = false;
        if let Kind::AudioMatch { rows } = &mut app.nodes[i].kind {
            if new_ord != ord {
                rows.retain(|(k, _)| k != "order by group");
                set_row(rows, "order", new_ord.key().to_string());
                changed = true;
            }
            if (new_rh - rh).abs() >= 0.5 {
                set_row(rows, "row height", format!("{new_rh:.0}"));
                changed = true;
            }
        }
        if changed {
            app.save_project();
        }
        if let Some(k) = press_anchor {
            app.toggle_anchor(i, k);
        }
        if let Some(k) = press_speed_ref {
            app.toggle_speed_ref(i, k);
        }
        if let Some(gi) = press_average {
            app.toggle_average(i, gi);
        }
        if let Some((k, id)) = new_phone {
            app.set_phone(i, k, &id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_keeps_a_clock_years_off() {
        let t = 1_347_000_000.0;
        let (a, b) = fit_axis(&[(t, t + 100.0), (t + 50.0, t + 200.0), (t + 4e8, t + 4e8 + 60.0)]);
        assert!(a < t && b > t + 4e8 + 60.0, "{a} {b}");
        assert_eq!(fit_axis(&[]), (0.0, 600.0));
    }

    /// Every order sorts as its name says, the free row always last.
    #[test]
    fn every_order_sorts_as_named() {
        let row = |n: usize, group: usize, meta: Option<f64>, start: Option<f64>| Row { n, wired: true, group, meta, start, off: meta.zip(start).map(|(m, s)| m - s), ..Default::default() };
        // Group 1 (number 2): rows 1 and 3; group 2 (number 1): rows 2 and 4; row 5 alone; row 6 free.
        let rows = vec![
            row(1, 0, Some(100.0), Some(90.0)),
            row(2, 1, None, Some(50.0)),
            row(3, 0, Some(10.0), Some(40.0)),
            row(4, 1, Some(70.0), Some(60.0)),
            row(5, 2, Some(5.0), None),
            Row { n: 6, group: 3, ..Default::default() },
        ];
        let g = |number: usize, rows: Vec<usize>| Group { number, rows, ..Default::default() };
        let v = MatchView { rows, groups: vec![g(2, vec![0, 2]), g(1, vec![1, 3]), g(0, vec![4]), g(0, vec![5])], done: true, ..Default::default() };
        // Group number 1 first, its row with metadata before the one without; then group 2 by start.
        assert_eq!(sorted(&v, Order::GroupMetaStart), [3, 1, 2, 0, 4, 5]);
        // By where each starts (audio, else metadata).
        assert_eq!(sorted(&v, Order::StartGroup), [4, 2, 1, 3, 0, 5]);
        // By group, nearest first; a row with no offset last in its group.
        assert_eq!(sorted(&v, Order::GroupOff), [3, 1, 0, 2, 4, 5]);
        assert_eq!(sorted(&v, Order::Wired), [0, 1, 2, 3, 4, 5]);
        // An old project's "order by group".
        assert_eq!(order_of(&Kind::AudioMatch { rows: vec![("order by group".into(), "true".into())] }), Order::GroupMetaStart);
        assert_eq!(order_of(&Kind::AudioMatch { rows: vec![("order".into(), "start-group".into())] }), Order::StartGroup);
        assert_eq!(order_of(&Kind::AudioMatch { rows: Vec::new() }), Order::Wired);
    }

    /// b's clock runs 250 ppm slow against a's: links every 5 minutes over 40 minutes.
    fn slow_links() -> Vec<Link> {
        (0..9)
            .map(|k| {
                let t_a = 20.0 + 300.0 * k as f64;
                Link { a: "a".into(), t_a, b: "b".into(), t_b: (t_a - 20.0) * (1.0 - 250e-6), error_s: 0.005, from: String::new(), enabled: true }
            })
            .collect()
    }

    /// b's clock runs 250 ppm slow against a's. Read in the group's average clock, a runs 125 ppm
    /// fast and b 125 ppm slow. With a as the speed reference (its clock runs true), b measures
    /// -250 ppm, and over a 2,400 s file ends 0.6 s after start + length, whichever clock id sorts
    /// first.
    #[test]
    fn a_slow_clock_ends_late() {
        for (a, b) in [("a", "b"), ("zz", "b")] {
            let links: Vec<Link> = slow_links().into_iter().map(|l| Link { a: a.into(), b: b.into(), ..l }).collect();
            let fits = fit_groups(&links, &[a.to_string()].into(), &BTreeMap::new());
            let (fa, fb) = (fits.clocks[a], fits.clocks[b]);
            assert!(fa.at(0.0).abs() < 1e-9, "measured from a's start");
            let ((pa, _), (pb, _)) = (speed_ppm(&fa, 1.0), speed_ppm(&fb, 1.0));
            assert!((pa - 125.0).abs() < 1.0 && (pb + 125.0).abs() < 1.0, "around the average: {pa} {pb}");
            // a runs true.
            let k = 1.0 / (1.0 + fa.rate);
            let (p, e) = speed_ppm(&fb, k);
            assert!((p + 250.0).abs() < 1.0 && e < 5.0, "{p} ± {e}");
            assert!(speed_ppm(&fa, k).0.abs() < 1e-6);
            let (s0, s1) = place(&fb, 2400.0, 1000.0 - k * fa.at(0.0), k);
            assert!((s0 - 1020.0).abs() < 1e-3, "b starts 20 s into a: {s0}");
            assert!((s1 - s0 - 2400.0 - 0.6).abs() < 0.01, "ends {} s after start + length", s1 - s0 - 2400.0);
            assert!(fits.leftover.iter().all(|d| d.abs() < 1e-3), "{:?}", fits.leftover);
        }
    }

    /// An anchor end wired 0.4 s past start + length: the anchor's clock runs slow against real
    /// time, and every row's end moves by that same speed.
    #[test]
    fn an_anchor_end_scales_every_row() {
        let fits = fit_groups(&slow_links(), &["a".to_string()].into(), &BTreeMap::new());
        let (fa, fb) = (fits.clocks["a"], fits.clocks["b"]);
        let (len_a, len_b, meta) = (3000.0, 2400.0, 5000.0);
        let q = (len_a + 0.4) / len_a;
        // Real seconds per second of the average clock, and where its 0 is.
        let k = q / (1.0 + fa.rate);
        let at = meta - k * fa.at(0.0);
        let (s_a, e_a) = place(&fa, len_a, at, k);
        assert!((s_a - meta).abs() < 1e-9 && (e_a - meta - len_a - 0.4).abs() < 1e-6, "the anchor ends at its wired end: {s_a} {e_a}");
        let (s_b, e_b) = place(&fb, len_b, at, k);
        assert!((s_b - meta - 20.0 * q).abs() < 1e-3, "b starts 20 of a's seconds in, at a's real speed: {}", s_b - meta);
        assert!((e_b - s_b - len_b * q / (1.0 - 250e-6)).abs() < 1e-3, "b's length grows by a's speed and its own");
        let ((pa, _), (pb, _)) = (speed_ppm(&fa, k), speed_ppm(&fb, k));
        assert!((pa + 133.3).abs() < 0.1 && (pb - pa + 250.0).abs() < 1.0, "real speeds {pa} {pb}");
    }

    /// Two files of one phone share its drift: a 40 s clip linked once takes the drift its phone's
    /// hour-long file measured.
    #[test]
    fn files_of_one_phone_share_a_drift() {
        let mut links = slow_links();
        links.push(Link { a: "a".into(), t_a: 2900.0, b: "clip".into(), t_b: 10.0, error_s: 0.005, from: String::new(), enabled: true });
        let refs: BTreeSet<String> = ["a".to_string()].into();
        let alone = fit_groups(&links, &refs, &BTreeMap::new());
        let phone: BTreeMap<String, String> = [("b".to_string(), "Pixel".to_string()), ("clip".to_string(), "Pixel".to_string())].into();
        let shared = fit_groups(&links, &refs, &phone);
        let p = |f: &Fits, c: &str| speed_ppm(&f.clocks[c], 1.0).0;
        assert!((p(&shared, "clip") - p(&shared, "b")).abs() < 1e-6, "one drift: {} {}", p(&shared, "clip"), p(&shared, "b"));
        assert!((p(&shared, "b") + 125.0).abs() < 1.0, "one vote for the phone: {}", p(&shared, "b"));
        assert!((p(&alone, "clip") - p(&alone, "b")).abs() > 50.0, "alone the clip has no drift of its own measured: {} {}", p(&alone, "clip"), p(&alone, "b"));
        // Each keeps its own start: the clip's 10 s is a's 2900 s.
        let on_a = corroborate::clock::read_in(&shared.clocks["clip"], &shared.clocks["a"]).at(10.0);
        assert!((on_a - 2900.0).abs() < 1e-3, "{on_a}");
    }

    /// The average anchor: one vote per phone, each weighted by its ±, and every stamp counts, one
    /// 12 years off too.
    #[test]
    fn the_average_counts_every_stamp_once_per_phone() {
        let v = |p: &str, at: f64, e: f64| (p.to_string(), at, e);
        // Phone A's two stamps make one vote, 101 ± 1; B says 110 ± 1; C's clock is 12 years off.
        let off = 12.0 * YEAR;
        let avg = stamp_average(&[v("A", 100.0, 1.0), v("A", 102.0, 1.0), v("B", 110.0, 1.0), v("C", 100.0 + off, 1.0)]).unwrap();
        assert_eq!(avg.phones, 3);
        assert!((avg.at - (101.0 + 110.0 + 100.0 + off) / 3.0).abs() < 1e-3, "{}", avg.at);
        assert!(avg.error > off / 4.0, "the spread shows in the error: {}", avg.error);
        // A sure stamp counts more than a vague one.
        let w = stamp_average(&[v("A", 0.0, 1.0), v("B", 10.0, 3.0)]).unwrap();
        assert!((w.at - 1.0).abs() < 1e-9, "{}", w.at);
        // One phone alone: its stamps' ± (the spread of one vote is none).
        let one = stamp_average(&[v("A", 50.0, 2.0)]).unwrap();
        assert_eq!((one.at, one.error, one.phones), (50.0, 2.0, 1));
        assert!(stamp_average(&[]).is_none());
    }

    #[test]
    fn drifts_read_in_seconds_an_hour() {
        assert_eq!(drift_text(-250.0), "-0.90");
        assert_eq!(drift_text(0.4), "0");
        assert_eq!(drift_text(5000.0), "+18.0");
        assert!(scale_words(-250.0).contains("×0.9997500") && scale_words(-250.0).contains("×1.0002501"), "{}", scale_words(-250.0));
    }

    #[test]
    fn a_phone_is_suggested_from_its_tags() {
        let t = |k: &str, v: &str| (format!("format.tags.{k}"), v.to_string());
        assert_eq!(phone_hint(&[t("com.apple.quicktime.make", "Apple"), t("com.apple.quicktime.model", "iPhone 12")]), "Apple iPhone 12");
        assert_eq!(phone_hint(&[t("com.android.manufacturer", "Samsung"), t("com.android.model", "Samsung SM-G991B")]), "Samsung SM-G991B");
        assert_eq!(phone_hint(&[t("title", "gig"), ("make".into(), "x".into())]), "");
        assert_eq!(phone_color("Pixel 7"), phone_color("Pixel 7"));
    }

    #[test]
    fn speeds_fill_on_a_log_scale() {
        assert!((speed_fill(30.0) - 0.5).abs() < 0.02);
        assert_eq!(speed_fill(-5000.0), 1.0);
        let (green, red) = (speed_color(0.0), speed_color(1000.0));
        assert!(green.g() > green.r() && red.r() > 2 * red.g(), "{green:?} {red:?}");
        assert_eq!(ms_text(0.0472), "+47 ms");
        assert_eq!(ms_text(-0.0001), "0 ms");
        assert_eq!(ms_text(-1.3), "-1.30 s");
    }

    #[test]
    fn gaps_read_plainly() {
        assert_eq!(gap_text(40.2), "+40.2 s");
        assert_eq!(gap_text(-180.0), "-3 min");
        assert_eq!(gap_text(4e8), "+12.7 years");
    }

    #[test]
    fn the_status_colour_goes_from_green_through_red_to_black() {
        let (zero, _) = off_color(0.0);
        assert!(zero.g() > zero.r(), "{zero:?}");
        let (thirty, _) = off_color(30.0);
        assert!(thirty.r() > 2 * thirty.g(), "{thirty:?}");
        let (years, fg) = off_color(YEAR * 3.0);
        assert_eq!((years, fg), (Color32::BLACK, Color32::WHITE));
    }
}
