//! Dating by the hum: the hum track matched against a grid frequency log, to confirm (or not) the
//! time and place the recording claims.
//!
//! Given the claimed start (a wired time) and place (a wired location), the node looks for a log
//! of that place's grid covering that time. Only NESO Great Britain is known for now, hardcoded
//! to the folder the logs were downloaded to (`grid logs/neso`, monthly `f-YYYY-M.csv`, 1 s).
//! The match: every whole-second start within ±SEARCH_S of the claim, the counted readings
//! against the log smoothed over the detector's 16 s window; cost = mean squared difference with
//! the mean removed (a recorder's clock error shifts the whole track a little). The best start
//! confirms the claim when it is clear (well under the runner-up) and close to the claim.

use super::{sections, EnfPoint, WIN_S};
use crate::area::WorldMask;
use crate::timebase::{fmt_utc, parse_iso8601};

/// How far from the claimed start the match looks, s.
pub const SEARCH_S: f64 = 3600.0;
/// Fewest counted readings to try a match.
pub const MIN_READINGS: usize = 120;
/// The best start's error (rms, Hz) must be under this…
pub const MAX_RMS_HZ: f64 = 0.010;
/// …and the runner-up's (more than 2 min away) at least this many times larger.
pub const MIN_MARGIN: f64 = 2.0;

/// A grid frequency log the node knows: its name, where its files are, and the area it covers.
pub struct LogSource {
    pub name: &'static str,
    pub folder: &'static str,
    /// Rough outline (lat, lon) of the area the grid covers.
    pub outline: &'static [(f64, f64)],
}

/// The logs known (hardcoded for now): NESO, Great Britain (not Northern Ireland).
pub const SOURCES: &[LogSource] = &[LogSource {
    name: "NESO Great Britain",
    folder: concat!(env!("CARGO_MANIFEST_DIR"), "/src/modules/ENF (power hum)/grid logs/neso"),
    outline: &[
        (49.9, -6.4),
        (49.9, 1.9),
        (51.2, 1.9),
        (52.9, 2.0),
        (55.9, -1.4),
        (58.7, -2.4),
        (60.9, -0.6),
        (60.9, -3.2),
        (58.6, -7.9),
        (56.6, -7.9),
        (55.6, -6.3),
        (55.0, -5.2),
        (54.4, -4.9),
        (53.4, -4.9),
        (51.6, -5.6),
        (50.0, -6.4),
    ],
}];

/// The known log whose area meets `place` (cells allowed).
pub fn source_for(place: &WorldMask) -> Option<&'static LogSource> {
    SOURCES.iter().find(|s| !WorldMask::from_polygons(&[s.outline]).and(place).is_empty())
}

/// A stretch of a grid log: frequency every second from `t0` (UTC, s); NaN where missing.
pub struct GridLog {
    pub name: String,
    pub t0: f64,
    pub f: Vec<f64>,
}

/// One line's time: `2016-07-01 01:00:00 +0100`, `2026-09-01 00:00:00`, `2026-09-01` (UTC when no
/// offset), or `01/01/2014 00:00:00`.
fn line_time(s: &str) -> Option<f64> {
    let s = s.trim();
    let s = if s.get(2..3) == Some("/") { format!("{}-{}-{}{}", s.get(6..10)?, s.get(3..5)?, s.get(0..2)?, s.get(10..)?) } else { s.to_string() };
    parse_iso8601(&s.replacen(" +", "+", 1).replacen(" -", "-", 1))
}

/// Read `dtm,f` lines in [from, to] from CSV text into `log` (whose t0 is set).
fn read_csv(text: &str, from: f64, to: f64, log: &mut GridLog) {
    for line in text.lines().skip(1) {
        let Some((d, f)) = line.split_once(',') else { continue };
        let (Some(t), Ok(f)) = (line_time(d), f.trim().parse::<f64>()) else { continue };
        if t >= from && t <= to {
            log.f[(t - from).round() as usize] = f;
        }
    }
}

/// Year and month of a UTC time.
fn year_month(t: f64) -> (i64, i64) {
    let s = fmt_utc(t);
    (s[0..4].parse().unwrap_or(0), s[5..7].parse().unwrap_or(0))
}

/// The log of `src` from `from` to `to` (UTC, s): the monthly files that cover it. A month file
/// holds local time, so the neighbouring months are read too.
pub fn load(src: &LogSource, from: f64, to: f64) -> Result<GridLog, String> {
    let (from, to) = (from.floor(), to.ceil());
    let mut log = GridLog { name: src.name.into(), t0: from, f: vec![f64::NAN; (to - from) as usize + 1] };
    let mut months: Vec<(i64, i64)> = [from - 86_400.0, from, to, to + 86_400.0].iter().map(|&t| year_month(t)).collect();
    months.dedup();
    let mut read = Vec::new();
    for (y, m) in months {
        let found = ["f", "fnew"].iter().map(|p| format!("{}/{p}-{y}-{m}.csv", src.folder)).find(|p| std::path::Path::new(p).exists());
        if let Some(p) = found {
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{p}: {e}"))?;
            read_csv(&text, from, to, &mut log);
            read.push(format!("{y}-{m:02}"));
        }
    }
    let have = log.f.iter().filter(|v| v.is_finite()).count();
    if have * 2 < log.f.len() {
        return Err(format!("{}: no log for {} (have {} of {} s; months read: {})", src.name, fmt_utc(from), have, log.f.len(), if read.is_empty() { "none".into() } else { read.join(", ") }));
    }
    Ok(log)
}

/// The result of matching the hum against a log.
#[derive(Clone, Debug, PartialEq)]
pub struct Dated {
    pub log: String,
    pub claimed: f64,
    /// Best start of the audio (UTC, s) and its error, rms, Hz.
    pub start: f64,
    pub rms_hz: f64,
    /// The best start more than 2 min away from it, and its error.
    pub runner_up: f64,
    pub runner_up_rms_hz: f64,
    /// The hum's mean offset from the log at the best start, Hz (recorder clock error).
    pub offset_hz: f64,
    pub readings: usize,
    pub confirmed: bool,
}

impl Dated {
    /// Best start minus the claimed one, s.
    pub fn shift_s(&self) -> f64 {
        self.start - self.claimed
    }

    /// The verdict in a few words.
    pub fn verdict(&self) -> String {
        if self.confirmed {
            format!("time confirmed ({:+.0} s)", self.shift_s())
        } else {
            "time not confirmed".into()
        }
    }

    pub fn lines(&self) -> Vec<String> {
        vec![
            format!("log: {}", self.log),
            format!("claimed {}", fmt_utc(self.claimed)),
            format!("found {}", fmt_utc(self.start)),
            format!("error {:.2} mHz (need <{:.0})", self.rms_hz * 1000.0, MAX_RMS_HZ * 1000.0),
            format!("next best {:.2} mHz, ×{:.1} (need ×{MIN_MARGIN:.0})", self.runner_up_rms_hz * 1000.0, self.runner_up_rms_hz / self.rms_hz),
            format!("{} readings, offset {:+.1} mHz", self.readings, self.offset_hz * 1000.0),
        ]
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"log\":\"{}\",\"claimed_utc\":\"{}\",\"found_utc\":\"{}\",\"shift_s\":{:.0},\"rms_hz\":{:.6},\"runner_up_utc\":\"{}\",\"runner_up_rms_hz\":{:.6},\"offset_hz\":{:.6},\"readings\":{},\"confirmed\":{}}}",
            self.log,
            fmt_utc(self.claimed),
            fmt_utc(self.start),
            self.shift_s(),
            self.rms_hz,
            fmt_utc(self.runner_up),
            self.runner_up_rms_hz,
            self.offset_hz,
            self.readings,
            self.confirmed
        )
    }
}

/// Match the counted readings (t: s from the audio's start) against `log` for every whole-second
/// start within ±`search_s` of `claimed`. `tolerance_s`: how far the best start may be from the
/// claim and still confirm it.
pub fn date(pts: &[EnfPoint], log: &GridLog, claimed: f64, search_s: f64, tolerance_s: f64) -> Result<Dated, String> {
    let pts: Vec<EnfPoint> = sections(pts).into_iter().filter(|p| p.ok).collect();
    if pts.len() < MIN_READINGS {
        return Err(format!("{} readings count, need {MIN_READINGS} to match", pts.len()));
    }
    // The log averaged over the detector's window, centred like a reading.
    let w = WIN_S as usize;
    let n = log.f.len();
    let mut smooth = vec![f64::NAN; n];
    for i in w / 2..n.saturating_sub(w / 2) {
        let s = &log.f[i - w / 2..i + w / 2];
        smooth[i] = s.iter().sum::<f64>() / w as f64;
    }
    let ti: Vec<i64> = pts.iter().map(|p| p.t.round() as i64).collect();
    let first = (claimed - search_s - log.t0).round() as i64;
    let last = (claimed + search_s - log.t0).round() as i64;
    let mut costs: Vec<(i64, f64, f64)> = Vec::new();
    for o in first..=last {
        let (mut s1, mut s2, mut k) = (0.0, 0.0, 0usize);
        for (p, &t) in pts.iter().zip(&ti) {
            let j = o + t;
            if j < 0 || j as usize >= n || !smooth[j as usize].is_finite() {
                continue;
            }
            let d = p.hz - smooth[j as usize];
            s1 += d;
            s2 += d * d;
            k += 1;
        }
        // Most readings must fall on the log.
        if k * 10 >= pts.len() * 9 {
            let mean = s1 / k as f64;
            costs.push((o, ((s2 / k as f64) - mean * mean).max(0.0).sqrt(), mean));
        }
    }
    let &(best, rms, mean) = costs.iter().min_by(|a, b| a.1.total_cmp(&b.1)).ok_or("the log does not cover the search")?;
    let (second, rms2) = costs.iter().filter(|c| (c.0 - best).abs() > 120).map(|c| (c.0, c.1)).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((best, f64::NAN));
    let start = log.t0 + best as f64;
    let confirmed = rms < MAX_RMS_HZ && rms2 >= MIN_MARGIN * rms && (start - claimed).abs() <= tolerance_s;
    Ok(Dated { log: log.name.clone(), claimed, start, rms_hz: rms, runner_up: log.t0 + second as f64, runner_up_rms_hz: rms2, offset_hz: mean, readings: pts.len(), confirmed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_times() {
        let t = parse_iso8601("2016-07-07T18:29:08Z").unwrap();
        assert_eq!(line_time("2016-07-07 19:29:08 +0100"), Some(t));
        assert_eq!(line_time("2016-07-07 18:29:08"), Some(t));
        assert_eq!(line_time("07/07/2016 18:29:08"), Some(t));
        assert_eq!(line_time("2026-09-01"), parse_iso8601("2026-09-01T00:00:00Z"));
    }

    #[test]
    fn great_britain_only() {
        assert_eq!(source_for(&WorldMask::circle(51.5, -0.45, 5.0)).map(|s| s.name), Some("NESO Great Britain"));
        assert!(source_for(&WorldMask::circle(48.0, 8.0, 5.0)).is_none(), "Freiburg");
    }

    /// A synthetic hum track cut from a random-walk log is found at its true start, and the claim
    /// is confirmed when right, not when 10 min off.
    #[test]
    fn finds_the_start() {
        let mut seed = 5u64;
        let mut u = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 11) as f64 / (1u64 << 53) as f64) - 0.5
        };
        let mut f = 50.0;
        let raw: Vec<f64> = (0..20_000)
            .map(|_| {
                f += 0.002 * u() - 0.0002 * (f - 50.0);
                f
            })
            .collect();
        let log = GridLog { name: "test".into(), t0: 1000.0, f: raw.clone() };
        let true_start = 1000.0 + 9000.0;
        let pts: Vec<EnfPoint> = (8..1200)
            .map(|t| {
                let i = 9000 + t;
                let hz = raw[i - 8..i + 8].iter().sum::<f64>() / 16.0 + 0.003 + 0.0005 * u();
                EnfPoint { t: t as f64, hz, nominal: 50.0, harmonic: 2, snr_db: 20.0, err_hz: 0.001, used: 2, ok: true, reject: None }
            })
            .collect();
        let d = date(&pts, &log, true_start + 30.0, 3000.0, 60.0).unwrap();
        assert_eq!(d.start, true_start, "{d:?}");
        assert!(d.confirmed, "{d:?}");
        assert!((d.offset_hz - 0.003).abs() < 0.0005, "{d:?}");
        let off = date(&pts, &log, true_start + 600.0, 3000.0, 60.0).unwrap();
        assert_eq!(off.start, true_start);
        assert!(!off.confirmed, "10 min off the claim");
    }
}
