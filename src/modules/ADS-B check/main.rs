//! adsb-check: a test plugin for the planes module, with ADS-B (Automatic Dependent
//! Surveillance–Broadcast) flight data as the ground truth.
//!
//! Flight data sets the expectations: every aircraft whose closest approach to the recording site is
//! within EXPECT_KM is an expected pass, due to be heard at its closest approach plus the sound's
//! travel time. The planes detector (this plugin's own copy, same settings as the viewer, so pass
//! numbers match the PLANES list) confirms each one or not:
//! - expected and heard: the Doppler curve the aircraft's track predicts (rest frequency taken from the
//!   heard pass) is drawn dashed over the spectrogram, for a visual check;
//! - expected but not heard;
//! - heard with no aircraft in the data (a plane without ADS-B, or a false detection).
//! The status sums it up by distance: the detector's real range at this site.
//!
//! Needs MANUAL DATA `coordinates` (lat, lon) and, for close passes, `elevation` (m above sea level);
//! the UTC of the audio (files: `--utc0` from the viewer; live: this PC's clock); and ADS-B data:
//! files: every `adsb*.jsonl` next to the source (`tools/adsb-log.py` writes them); live: polls
//! adsb.lol every 10 s (backing off when refused) and logs to `adsb-logs/` in the working folder.
//! Aircraft without ADS-B Out, or too low for any receiver, are not in the data.

use corroborate::modules::planes::Planes;
use corroborate::plugin::{self, Json, PluginFinding};
use corroborate::{ingest, timebase, Config, Event, Finding, Module};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

const C: f64 = 343.0;
/// Expected passes: closest approach within this slant distance, the planes detector's own
/// distance limit (Config d_max): an airliner cruising overhead is 10–12 km away.
const EXPECT_KM: f64 = 15.0;
/// A heard pass confirms an expected one when their closest approaches are this close in time.
const MATCH_S: f64 = 20.0;
const RADIUS_NM: f64 = 25.0;
const POLL_S: f64 = 10.0;
/// Longest wait between polls when the API refuses or fails (it doubles up to this).
const MAX_WAIT_S: f64 = 60.0;
const COLOR: [u8; 3] = [255, 140, 90];
const PLANES_BAND: (f64, f64) = (40.0, 450.0);
const BINS_KM: [f64; 4] = [2.0, 5.0, 10.0, EXPECT_KM];

// ---------------------------------------------------------------- flight data

struct Fix {
    t: f64,
    lat: f64,
    lon: f64,
    alt_m: Option<f64>,
    gs_ms: Option<f64>,
}

#[derive(Default)]
struct Aircraft {
    hex: String,
    flight: String,
    reg: String,
    typ: String,
    fixes: Vec<Fix>,
}

impl Aircraft {
    fn name(&self) -> String {
        let mut n = [&self.flight, &self.reg, &self.hex].into_iter().find(|s| !s.is_empty()).cloned().unwrap_or_default();
        if !self.typ.is_empty() {
            n = format!("{n} {}", self.typ);
        }
        n
    }

    /// Position (lat, lon, altitude m) at `t`, between two fixes at most 30 s apart.
    fn at(&self, t: f64) -> Option<(f64, f64, f64)> {
        let i = self.fixes.partition_point(|f| f.t <= t);
        if i == 0 || i >= self.fixes.len() {
            return None;
        }
        let (a, b) = (&self.fixes[i - 1], &self.fixes[i]);
        if b.t - a.t > 30.0 {
            return None;
        }
        let u = (t - a.t) / (b.t - a.t);
        let alt = match (a.alt_m, b.alt_m) {
            (Some(x), Some(y)) => x + u * (y - x),
            (Some(x), None) | (None, Some(x)) => x,
            _ => return None,
        };
        Some((a.lat + u * (b.lat - a.lat), a.lon + u * (b.lon - a.lon), alt))
    }

    fn gs_at(&self, t: f64) -> Option<f64> {
        let i = self.fixes.partition_point(|f| f.t <= t).min(self.fixes.len() - 1);
        self.fixes[i].gs_ms.or_else(|| i.checked_sub(1).and_then(|j| self.fixes[j].gs_ms))
    }
}

#[derive(Default)]
struct Adsb {
    ac: HashMap<String, Aircraft>,
    polls: usize,
    t_min: f64,
    t_max: f64,
}

impl Adsb {
    /// One poll of an aircraft API (`ac` array, readsb fields), taken at `data_utc`.
    fn add_poll(&mut self, data_utc: f64, ac: &[Json]) {
        if self.polls == 0 {
            (self.t_min, self.t_max) = (data_utc, data_utc);
        }
        self.t_min = self.t_min.min(data_utc);
        self.t_max = self.t_max.max(data_utc);
        self.polls += 1;
        for a in ac {
            let num = |k: &str| a.get(k).and_then(Json::num);
            let (Some(hex), Some(lat), Some(lon)) = (a.get("hex").and_then(Json::str), num("lat"), num("lon")) else { continue };
            let t = data_utc - num("seen_pos").unwrap_or(0.0);
            // Geometric (GNSS) altitude if sent, else barometric; "ground" counts as no altitude.
            let alt_m = num("alt_geom").or_else(|| num("alt_baro")).map(|ft| ft * 0.3048);
            let gs_ms = num("gs").map(|kt| kt * 0.514444);
            let e = self.ac.entry(hex.to_string()).or_insert_with(|| Aircraft { hex: hex.to_string(), ..Default::default() });
            for (k, slot) in [("flight", &mut e.flight), ("r", &mut e.reg), ("t", &mut e.typ)] {
                if let Some(v) = a.get(k).and_then(Json::str) {
                    if !v.trim().is_empty() {
                        *slot = v.trim().to_string();
                    }
                }
            }
            let i = e.fixes.partition_point(|f| f.t < t);
            let dup = e.fixes.get(i).is_some_and(|f| (f.t - t).abs() < 0.5) || (i > 0 && (e.fixes[i - 1].t - t).abs() < 0.5);
            if !dup {
                e.fixes.insert(i, Fix { t, lat, lon, alt_m, gs_ms });
            }
        }
    }
}

// ---------------------------------------------------------------- geometry

#[derive(Clone, Copy)]
struct Site {
    lat: f64,
    lon: f64,
    elev: f64,
}

impl Site {
    /// Slant distance (m) from the site to (lat, lon, altitude m).
    fn range(&self, p: (f64, f64, f64)) -> f64 {
        const R: f64 = 6_371_000.0;
        let x = R * self.lat.to_radians().cos() * (p.1 - self.lon).to_radians();
        let y = R * (p.0 - self.lat).to_radians();
        let z = p.2 - self.elev;
        (x * x + y * y + z * z).sqrt()
    }
}

/// A closest approach found in the flight data.
struct Approach {
    hex: String,
    name: String,
    /// When the aircraft was closest (UTC), and when its sound from there arrives.
    t_emit: f64,
    heard: f64,
    r: f64,
    alt: f64,
    gs: Option<f64>,
    note: &'static str,
}

/// Every closest approach within EXPECT_KM in the data. Where a track starts or ends close by
/// (take-off, landing, coverage), its first or last point counts, marked as such.
fn approaches(adsb: &Adsb, site: Site) -> Vec<Approach> {
    let mut out = Vec::new();
    for a in adsb.ac.values() {
        if a.fixes.len() < 2 {
            continue;
        }
        let mut found: Vec<(f64, f64, &'static str)> = Vec::new();
        let mut seg: Vec<(f64, f64)> = Vec::new();
        let (t0, t1) = (a.fixes[0].t.ceil(), a.fixes[a.fixes.len() - 1].t.floor());
        let mut t = t0;
        while t <= t1 + 1.0 {
            match a.at(t).map(|p| site.range(p)) {
                Some(r) if t <= t1 => seg.push((t, r)),
                _ => {
                    minima(&seg, adsb, &mut found);
                    seg.clear();
                }
            }
            t += 1.0;
        }
        // One per minute at most: the closest.
        found.sort_by(|x, y| x.1.total_cmp(&y.1));
        let mut kept: Vec<(f64, f64, &'static str)> = Vec::new();
        for f in found {
            if !kept.iter().any(|k| (k.0 - f.0).abs() < 60.0) {
                kept.push(f);
            }
        }
        for (t, r, note) in kept {
            let alt = a.at(t).map_or(f64::NAN, |p| p.2);
            out.push(Approach { hex: a.hex.clone(), name: a.name(), t_emit: t, heard: t + r / C, r, alt, gs: a.gs_at(t), note });
        }
    }
    out.sort_by(|x, y| x.heard.total_cmp(&y.heard));
    out
}

fn minima(seg: &[(f64, f64)], adsb: &Adsb, found: &mut Vec<(f64, f64, &'static str)>) {
    let n = seg.len();
    if n < 3 {
        return;
    }
    for i in 0..n {
        let (t, r) = seg[i];
        if r > EXPECT_KM * 1000.0 {
            continue;
        }
        let note = if i == 0 {
            if seg[1].1 > r && t > adsb.t_min + 60.0 { Some("track starts here: took off or first seen") } else { None }
        } else if i == n - 1 {
            if seg[n - 2].1 >= r && t < adsb.t_max - 60.0 { Some("track ends here: landed or lost") } else { None }
        } else if r <= seg[i - 1].1 && r < seg[i + 1].1 {
            Some("")
        } else {
            None
        };
        if let Some(note) = note {
            found.push((t, r, note));
        }
    }
}

/// The Doppler curve the aircraft's track predicts around `t_emit`, from rest frequency `f0`:
/// (relative time heard, Hz).
fn predicted(a: &Aircraft, site: Site, t_emit: f64, f0: f64, utc0: f64) -> Vec<(f64, f64)> {
    let mut v = Vec::new();
    for k in -90..=90 {
        let t = t_emit + k as f64;
        let (Some(p0), Some(p), Some(p1)) = (a.at(t - 1.0), a.at(t), a.at(t + 1.0)) else { continue };
        let r = site.range(p);
        let vr = (site.range(p1) - site.range(p0)) / 2.0;
        v.push((t + r / C - utc0, f0 * C / (C + vr)));
    }
    v
}

// ---------------------------------------------------------------- the check

struct Expect {
    id: u64,
    a: Approach,
    done: bool,
    /// Counted (inside the audio, within the detector's limits) and heard.
    counted: bool,
    /// Inside the audio, but flying outside the detector's speed limits: it cannot be reported.
    beyond: bool,
    heard_as: Option<u64>,
}

struct Heard {
    e: Event,
    claimed: bool,
    done: bool,
}

struct Check<W: Write> {
    out: W,
    site: Site,
    site_note: String,
    utc0: Option<(f64, String)>,
    adsb: Arc<Mutex<Adsb>>,
    adsb_desc: String,
    /// The planes detector's speed limits (m/s).
    gates: (f64, f64),
    polls_seen: usize,
    expects: Vec<Expect>,
    heard: Vec<Heard>,
    next_id: u64,
    last_status: String,
}

impl<W: Write> Check<W> {
    fn line(&mut self, s: String) {
        let _ = writeln!(self.out, "{s}");
    }

    /// New expectations from new flight data.
    fn refresh(&mut self) {
        let adsb = self.adsb.clone();
        let adsb = adsb.lock().unwrap();
        if adsb.polls == self.polls_seen {
            return;
        }
        self.polls_seen = adsb.polls;
        for a in approaches(&adsb, self.site) {
            if !self.expects.iter().any(|x| x.a.hex == a.hex && (x.a.t_emit - a.t_emit).abs() < 30.0) {
                self.next_id += 1;
                self.expects.push(Expect { id: self.next_id, a, done: false, counted: false, beyond: false, heard_as: None });
            }
        }
    }

    /// Settle expectations and heard passes once the detector has had time to report them
    /// (or at the end of the audio, `fin`).
    fn resolve(&mut self, audio_t: f64, fin: bool) {
        let Some((utc0, _)) = self.utc0.clone() else { return };
        let adsb = self.adsb.clone();
        let adsb = adsb.lock().unwrap();
        for xi in 0..self.expects.len() {
            let x = &self.expects[xi];
            if x.done {
                continue;
            }
            let rel = x.a.heard - utc0;
            if rel < 0.0 || (fin && rel > audio_t) {
                self.expects[xi].done = true; // outside the audio: not counted
                continue;
            }
            if !fin && audio_t < rel + 120.0 {
                continue;
            }
            let best = self
                .heard
                .iter()
                .enumerate()
                .filter(|(_, h)| !h.claimed && (h.e.t_cpa - rel).abs() <= MATCH_S)
                .min_by(|a, b| (a.1.e.t_cpa - rel).abs().total_cmp(&(b.1.e.t_cpa - rel).abs()))
                .map(|(i, _)| i);
            let x = &self.expects[xi];
            let (id, name, r, alt, gs, note) = (x.id, x.a.name.clone(), x.a.r, x.a.alt, x.a.gs, x.a.note);
            let gs_txt = gs.map_or("? m/s".into(), |g| format!("{g:.0} m/s"));
            let when = format!(
                "closest {} UTC + {:.1} s sound travel = heard {} UTC{}",
                timebase::fmt_utc_clock(x.a.t_emit),
                r / C,
                timebase::fmt_utc_clock(x.a.heard),
                if note.is_empty() { String::new() } else { format!(" ({note})") }
            );
            let hex = x.a.hex.clone();
            let t_emit = x.a.t_emit;
            match best {
                Some(hi) => {
                    self.heard[hi].claimed = true;
                    let e = self.heard[hi].e.clone();
                    let dt = e.t_cpa - rel;
                    let label = format!("✓ #{} {name}  {:.2} km  {gs_txt}  Δt {dt:+.0} s", e.id, r / 1000.0);
                    let detail = format!(
                        "{when}\nheard as pass #{} at Δt {dt:+.1} s\nADS-B: {:.2} km slant, altitude {:.0} m, ground speed {gs_txt}, hex {hex}\nDoppler fit: {:.2} km, {:.0} m/s, f0 {:.1} Hz\ndashed curve: what this track predicts, from f0",
                        e.id,
                        r / 1000.0,
                        alt,
                        e.dist / 1000.0,
                        e.speed,
                        e.f0
                    );
                    let f = PluginFinding { id, t_start: e.t_start, t_end: e.t_end, f_lo: e.f_post.min(e.f_pre), f_hi: e.f_post.max(e.f_pre), label, detail };
                    self.line(plugin::finding(&f));
                    if let Some(a) = adsb.ac.get(&hex) {
                        let track = format!("#{} {}", e.id, a.name());
                        for (t, hz) in predicted(a, self.site, t_emit, e.f0, utc0) {
                            self.line(plugin::track(&track, t, hz, true));
                        }
                    }
                    let x = &mut self.expects[xi];
                    (x.done, x.counted, x.heard_as) = (true, true, Some(e.id));
                }
                None => {
                    // Too fast or slow for the detector's gates: it cannot report this one by design.
                    let beyond = gs.is_some_and(|g| g > self.gates.1 * 1.1 || g < self.gates.0 * 0.9);
                    let (label, why) = if beyond {
                        (
                            format!("– beyond detector  {name}  {:.2} km  alt {:.0} m  {gs_txt}", r / 1000.0, alt),
                            format!(
                                "ground speed {gs_txt} is outside the planes detector's {:.0}–{:.0} m/s: it cannot report this pass",
                                self.gates.0, self.gates.1
                            ),
                        )
                    } else {
                        (
                            format!("· not heard  {name}  {:.2} km  alt {:.0} m  {gs_txt}", r / 1000.0, alt),
                            format!("no pass detected within ±{MATCH_S:.0} s"),
                        )
                    };
                    let detail = format!("{when}\n{why}\nhex {hex}");
                    let f = PluginFinding { id, t_start: rel - 15.0, t_end: rel + 15.0, f_lo: PLANES_BAND.0, f_hi: PLANES_BAND.1, label, detail };
                    self.line(plugin::finding(&f));
                    let x = &mut self.expects[xi];
                    (x.done, x.counted, x.beyond) = (true, !beyond, beyond);
                }
            }
        }
        // Heard passes no expectation claimed.
        for hi in 0..self.heard.len() {
            let h = &self.heard[hi];
            if h.done || h.claimed || (!fin && audio_t < h.e.t_cpa + 150.0) {
                continue;
            }
            let t_cpa = h.e.t_cpa;
            if !fin && self.expects.iter().any(|x| !x.done && (x.a.heard - utc0 - t_cpa).abs() <= MATCH_S) {
                continue;
            }
            let heard_utc = utc0 + t_cpa;
            let why = if heard_utc < adsb.t_min || heard_utc > adsb.t_max {
                "no ADS-B data at that time".to_string()
            } else {
                // The nearest aircraft in the data around then, for context.
                let mut near: Option<(f64, f64, String)> = None;
                for a in adsb.ac.values() {
                    let mut t = heard_utc - 120.0;
                    while t <= heard_utc + 30.0 {
                        if let Some(p) = a.at(t) {
                            let r = self.site.range(p);
                            if near.as_ref().is_none_or(|n| r < n.0) {
                                near = Some((r, t + r / C - heard_utc, a.name()));
                            }
                        }
                        t += 2.0;
                    }
                }
                match near {
                    Some((r, dt, n)) => format!("no aircraft within {EXPECT_KM:.0} km in the data; nearest {n} at {:.1} km, Δt {dt:+.0} s", r / 1000.0),
                    None => "no aircraft in the data then".to_string(),
                }
            };
            let e = h.e.clone();
            self.next_id += 1;
            let label = format!("? #{} heard, no aircraft  ({:.2} km, {:.0} m/s fit)", e.id, e.dist / 1000.0, e.speed);
            let detail = format!("pass #{} heard at {} UTC\n{why}\na plane without ADS-B, below receiver coverage, or a false detection", e.id, timebase::fmt_utc_clock(heard_utc));
            let f = PluginFinding { id: self.next_id, t_start: e.t_start, t_end: e.t_end, f_lo: e.f_post.min(e.f_pre), f_hi: e.f_post.max(e.f_pre), label, detail };
            self.line(plugin::finding(&f));
            self.heard[hi].done = true;
        }
        drop(adsb);
        self.status(fin);
    }

    fn status(&mut self, fin: bool) {
        let counted: Vec<&Expect> = self.expects.iter().filter(|x| x.counted).collect();
        let heard = counted.iter().filter(|x| x.heard_as.is_some()).count();
        let lone = self.heard.iter().filter(|h| h.done && !h.claimed).count();
        let beyond = self.expects.iter().filter(|x| x.beyond).count();
        let mut bins = Vec::new();
        let mut lo = 0.0;
        for hi in BINS_KM {
            let n = counted.iter().filter(|x| x.a.r / 1000.0 >= lo && x.a.r / 1000.0 < hi).count();
            let k = counted.iter().filter(|x| x.a.r / 1000.0 >= lo && x.a.r / 1000.0 < hi && x.heard_as.is_some()).count();
            bins.push(format!("{lo:.0}–{hi:.0} km {k}/{n}"));
            lo = hi;
        }
        let adsb = self.adsb.lock().unwrap();
        let data = if adsb.polls == 0 {
            "no flight data yet".to_string()
        } else {
            format!("{} aircraft, {}–{} UTC", adsb.ac.len(), timebase::fmt_utc_clock(adsb.t_min), timebase::fmt_utc_clock(adsb.t_max))
        };
        drop(adsb);
        let utc = self.utc0.as_ref().map_or("UTC: waiting for audio".to_string(), |u| format!("UTC: {}", u.1));
        let summary = format!(
            "expected ≤{EXPECT_KM:.0} km: {} · heard {heard} · heard without aircraft {lone}\nbeyond the detector's {:.0}–{:.0} m/s (jets at cruise), not counted: {beyond}",
            counted.len(),
            self.gates.0,
            self.gates.1
        );
        let text = format!(
            "{}\n{utc}\nADS-B: {} · {data}\n{summary}\nheard/expected by distance: {}{}",
            self.site_note,
            self.adsb_desc,
            bins.join(" · "),
            if fin { "\n(final)" } else { "" }
        );
        let n_expected = counted.len();
        if text != self.last_status {
            self.line(plugin::status(&text));
            self.line(plugin::meta("expected passes (≤15 km)", &n_expected.to_string()));
            self.line(plugin::meta("expected and heard", &heard.to_string()));
            self.line(plugin::meta("heard without aircraft", &lone.to_string()));
            self.line(plugin::meta("beyond detector speed limits", &beyond.to_string()));
            self.line(plugin::meta("heard/expected by distance", &bins.join(", ")));
            self.last_status = text;
        }
    }
}

// ---------------------------------------------------------------- input

/// The site from the Value node wired into `place`: `coordinates` "lat, lon" and optional
/// `elevation` (m, leading number).
fn site_from(manual: &[(String, String)]) -> Result<(Site, String), String> {
    let get = |k: &str| manual.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str());
    let coords = get("coordinates").ok_or("needs the site: wire a Value node of type location (latitude, longitude, elevation) into place")?;
    let nums: Vec<f64> = coords.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|s| s.parse().ok()).collect();
    let (lat, lon) = match nums[..] {
        [lat, lon, ..] if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) => (lat, lon),
        _ => return Err(format!("could not read coordinates \"{coords}\": write them as lat, lon")),
    };
    let elev = get("elevation").and_then(|v| v.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).find(|s| !s.is_empty())?.parse().ok());
    let note = match elev {
        Some(e) => format!("site {lat:.5}, {lon:.5}, {e:.0} m (manual data)"),
        None => format!("site {lat:.5}, {lon:.5} (manual data); no elevation: sea level assumed, so low passes read far"),
    };
    Ok((Site { lat, lon, elev: elev.unwrap_or(0.0) }, note))
}

/// Read stdin to the end (the viewer waits for every plugin to take its audio).
fn drain() {
    let mut buf = vec![0u8; 1 << 16];
    let mut i = std::io::stdin().lock();
    while matches!(i.read(&mut buf), Ok(n) if n > 0) {}
}

/// Flight data logged next to a file: every adsb*.jsonl in its folder.
fn load_logs(source: &str, adsb: &mut Adsb) -> Vec<String> {
    let dir = std::path::Path::new(source).parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("adsb") && name.ends_with(".jsonl")) {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        for line in text.lines() {
            let Some(j) = plugin::parse_json(line) else { continue };
            let Some(ac) = j.get("ac") else { continue };
            let t = j.get("data_utc").and_then(Json::num).or_else(|| j.get("now").and_then(Json::num).map(|ms| ms / 1000.0)).or_else(|| j.get("pc_utc").and_then(Json::num));
            if let Some(t) = t {
                adsb.add_poll(t, ac.arr());
            }
        }
        names.push(name);
    }
    names
}

/// Compact JSON text of a value (for the live log).
fn ser(j: &Json) -> String {
    match j {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Num(x) => x.to_string(),
        Json::Str(s) => format!("\"{}\"", timebase::json_escape(s)),
        Json::Arr(v) => format!("[{}]", v.iter().map(ser).collect::<Vec<_>>().join(",")),
        Json::Obj(v) => format!("{{{}}}", v.iter().map(|(k, x)| format!("\"{}\":{}", timebase::json_escape(k), ser(x))).collect::<Vec<_>>().join(",")),
    }
}

/// Live: poll adsb.lol around the site every POLL_S, into `adsb` and a log file.
fn poll_live(site: Site, adsb: Arc<Mutex<Adsb>>, log_path: std::path::PathBuf) {
    const KEEP: [&str; 10] = ["hex", "flight", "r", "t", "lat", "lon", "alt_baro", "alt_geom", "gs", "seen_pos"];
    std::thread::spawn(move || {
        if let Some(d) = log_path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).ok();
        let url = format!("https://api.adsb.lol/v2/point/{:.5}/{:.5}/{RADIUS_NM}", site.lat, site.lon);
        let mut wait = POLL_S;
        loop {
            let t0 = std::time::Instant::now();
            let got = corroborate::no_window(&mut std::process::Command::new("curl"))
                .args(["-s", "-m", "10", "-A", "corroborate adsb-check plugin", &url])
                .output();
            // A refusal (rate limit) or failure is not aircraft JSON: wait longer, up to a minute.
            let parsed = got.ok().and_then(|o| plugin::parse_json(&String::from_utf8_lossy(&o.stdout))).filter(|j| j.get("ac").is_some());
            wait = if parsed.is_some() { POLL_S } else { (wait * 2.0).min(MAX_WAIT_S) };
            if let Some(j) = parsed {
                let pc = timebase::now_unix();
                let data_utc = j.get("now").and_then(Json::num).map_or(pc, |ms| ms / 1000.0);
                let ac = j.get("ac").map(Json::arr).unwrap_or(&[]);
                adsb.lock().unwrap().add_poll(data_utc, ac);
                if let Some(f) = log.as_mut() {
                    let kept: Vec<String> = ac
                        .iter()
                        .map(|a| match a {
                            Json::Obj(v) => ser(&Json::Obj(v.iter().filter(|(k, _)| KEEP.contains(&k.as_str())).cloned().collect())),
                            other => ser(other),
                        })
                        .collect();
                    let _ = writeln!(f, "{{\"pc_utc\":{pc:.3},\"data_utc\":{data_utc:.3},\"api\":\"adsb.lol\",\"ac\":[{}]}}", kept.join(","));
                }
            }
            let left = wait - t0.elapsed().as_secs_f64();
            if left > 0.0 {
                std::thread::sleep(std::time::Duration::from_secs_f64(left));
            }
        }
    });
}

fn main() {
    let (source, live) = plugin::source_args();
    let manual = plugin::manual_args();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let _ = writeln!(out, "{}", plugin::hello("ADS-B CHECK", "test: flight data expects passes, the planes detector confirms", COLOR));
    let fail = |out: &mut dyn Write, why: &str| {
        let _ = writeln!(out, "{}", plugin::status(why));
        let _ = out.flush();
        drain();
    };
    let (site, site_note) = match site_from(&manual) {
        Ok(s) => s,
        Err(e) => return fail(&mut out, &e),
    };
    let adsb: Arc<Mutex<Adsb>> = Default::default();
    let adsb_desc = if live {
        let path = std::path::PathBuf::from("adsb-logs").join(format!("adsb-{}.jsonl", timebase::utc_stamp(timebase::now_unix())));
        let desc = format!("live from adsb.lol every {POLL_S:.0} s, logged to {}", path.display());
        let _ = writeln!(out, "{}", plugin::meta("adsb log", &path.display().to_string()));
        poll_live(site, adsb.clone(), path);
        desc
    } else {
        let src = source.clone().unwrap_or_default();
        let names = load_logs(&src, &mut adsb.lock().unwrap());
        if names.is_empty() {
            return fail(&mut out, &format!("{site_note}\nno flight data: put adsb*.jsonl next to the file (tools/adsb-log.py logs one)"));
        }
        let _ = writeln!(out, "{}", plugin::meta("adsb files", &names.join(", ")));
        format!("from {}", names.join(", "))
    };
    let utc0 = if live { None } else { plugin::utc0_args() };
    if !live && utc0.is_none() {
        return fail(&mut out, "this file has no UTC (Coordinated Universal Time) anchor, so it cannot be matched to flight data: record it with REC, or give it a creation time");
    }
    let _ = writeln!(out, "{}", plugin::meta("site", &site_note));
    let _ = writeln!(out, "{}", plugin::meta("rules", &format!("expected = closest approach ≤ {EXPECT_KM:.0} km; heard = a pass within ±{MATCH_S:.0} s of closest approach + sound travel")));

    let cfg = Config::default();
    let mut ck = Check {
        out,
        site,
        site_note,
        utc0,
        adsb,
        adsb_desc,
        gates: (cfg.v_min, cfg.v_max),
        polls_seen: 0,
        expects: Vec::new(),
        heard: Vec::new(),
        next_id: 0,
        last_status: String::new(),
    };
    let mut planes = Planes::new(cfg);
    let mut samples: u64 = 0;
    let mut next_check = 0.0;
    for x in ingest::spawn_stdin(800) {
        if ck.utc0.is_none() {
            ck.utc0 = Some((timebase::now_unix() - samples as f64 / 16_000.0, "this PC's clock when the audio arrived (late by the stream's delay)".into()));
        }
        samples += x.len() as u64;
        for f in planes.push(&x) {
            if let Finding::Plane(e) = f {
                ck.heard.push(Heard { e, claimed: false, done: false });
            }
        }
        let audio_t = samples as f64 / 16_000.0;
        if audio_t >= next_check {
            next_check = audio_t + 5.0;
            ck.refresh();
            ck.resolve(audio_t, false);
            if ck.out.flush().is_err() {
                return;
            }
        }
    }
    for f in planes.flush() {
        if let Finding::Plane(e) = f {
            ck.heard.push(Heard { e, claimed: false, done: false });
        }
    }
    ck.refresh();
    ck.resolve(samples as f64 / 16_000.0, true);
    let _ = ck.out.flush();
}
