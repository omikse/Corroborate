//! Time model: every timestamp is relative (seconds from the first sample). Real time is
//! a separate, labelled claim: an anchor giving the UTC (Coordinated Universal Time) of
//! relative time 0, and where that came from.
//!
//! Anchor sources, best first:
//! - HLS (HTTP Live Streaming) `#EXT-X-PROGRAM-DATE-TIME`: capture time stamped by the
//!   broadcaster or platform, matched to the exact segment our audio starts with.
//! - A sidecar `<name>.json` written by our own recorder.
//! - File metadata `creation_time` (often the copy or download time: low confidence).
//! - The PC clock when the first audio arrived ("received": late by the stream's delay).
//!
//! No date/time crate: ISO 8601 parsing and formatting by the civil-from-days algorithm.

use std::process::Command;

#[derive(Clone, Debug)]
pub struct Anchor {
    /// UTC (Unix seconds) of relative time 0.
    pub utc0: f64,
    /// Where it came from, in words, for display and output.
    pub source: String,
}

impl Anchor {
    pub fn utc(&self, rel: f64) -> f64 {
        self.utc0 + rel
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// Parse `2026-11-04T20:06:39.312+00:00`, `...Z`, or `2026-11-04 20:06:39` (taken as UTC).
pub fn parse_iso8601(s: &str) -> Option<f64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if !matches!(s.get(10..11), Some("T") | Some(" ")) {
        return Some(days_from_civil(y, mo, d) as f64 * 86_400.0);
    }
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut i = 19;
    let mut frac = 0.0;
    if s.get(i..i + 1) == Some(".") {
        let j = s[i + 1..].find(|c: char| !c.is_ascii_digit()).map_or(s.len(), |k| i + 1 + k);
        frac = format!("0{}", &s[i..j]).parse::<f64>().ok()?;
        i = j;
    }
    let mut off = 0i64;
    match s.get(i..i + 1) {
        Some("+") | Some("-") => {
            let sign = if &s[i..i + 1] == "-" { -1 } else { 1 };
            let oh = num(i + 1, i + 3)?;
            let om = s.get(i + 4..i + 6).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
            off = sign * (oh * 3600 + om * 60);
        }
        _ => {}
    }
    let days = days_from_civil(y, mo, d);
    Some((days * 86_400 + h * 3600 + mi * 60 + se - off) as f64 + frac)
}

/// `2026-11-04T20:06:39.312Z`
pub fn fmt_utc(t: f64) -> String {
    let ms = (t * 1000.0).round() as i64;
    let (secs, msr) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{msr:03}Z", rem / 3600, (rem / 60) % 60, rem % 60)
}

/// `20:06:39` (UTC time of day).
pub fn fmt_utc_clock(t: f64) -> String {
    let s = t.floor() as i64;
    let rem = s.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", rem / 3600, (rem / 60) % 60, rem % 60)
}

/// `20261104-200639` for file names.
pub fn utc_stamp(t: f64) -> String {
    let s = fmt_utc(t);
    format!("{}{}{}-{}{}{}", &s[0..4], &s[5..7], &s[8..10], &s[11..13], &s[14..16], &s[17..19])
}

pub fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Key that identifies a segment across playlist fetches: YouTube's `/sq/<n>/`, else the
/// URL without scheme, host and query.
fn segment_key(url: &str) -> String {
    if let Some(i) = url.find("/sq/") {
        let rest = &url[i + 4..];
        let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !n.is_empty() {
            return format!("sq:{n}");
        }
    }
    let no_scheme = url.split_once("://").map_or(url, |x| x.1);
    let path = no_scheme.split_once('/').map_or("", |x| x.1);
    path.split('?').next().unwrap_or("").to_string()
}

fn fetch(url: &str) -> Option<String> {
    let out = Command::new("curl").args(["-s", "-L", "--max-time", "10", url]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Segment URL → capture UTC, from an HLS media playlist (follows one master level).
fn playlist_times(url: &str) -> Vec<(String, f64)> {
    let Some(text) = fetch(url) else { return Vec::new() };
    if text.contains("#EXT-X-STREAM-INF") {
        let mut out = Vec::new();
        let mut next_is_variant = false;
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with("#EXT-X-STREAM-INF") {
                next_is_variant = true;
            } else if next_is_variant && !l.is_empty() && !l.starts_with('#') {
                next_is_variant = false;
                out.extend(playlist_times(&resolve(url, l)));
            }
        }
        return out;
    }
    let mut out = Vec::new();
    let mut t: Option<f64> = None;
    let mut dur = 0.0;
    for line in text.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
            t = parse_iso8601(v);
        } else if let Some(v) = l.strip_prefix("#EXTINF:") {
            dur = v.split(',').next().and_then(|x| x.trim().parse().ok()).unwrap_or(0.0);
        } else if !l.is_empty() && !l.starts_with('#') {
            if let Some(tt) = t {
                out.push((segment_key(&resolve(url, l)), tt));
                t = Some(tt + dur);
            }
        }
    }
    out
}

fn resolve(base: &str, rel: &str) -> String {
    if rel.contains("://") {
        return rel.to_string();
    }
    match base.rfind('/') {
        Some(i) => format!("{}/{}", &base[..i], rel.trim_start_matches('/')),
        None => rel.to_string(),
    }
}

/// Capture UTC of the segment `seg_url` in the HLS playlist at `playlist_url`, if the
/// playlist carries PROGRAM-DATE-TIME.
pub fn hls_segment_utc(playlist_url: &str, seg_url: &str) -> Option<f64> {
    let key = segment_key(seg_url);
    playlist_times(playlist_url).into_iter().find(|(k, _)| *k == key).map(|(_, t)| t)
}

/// Shared, updatable anchor: starts with the best guess, upgraded when a better one is found.
pub type SharedAnchor = std::sync::Arc<std::sync::Mutex<Option<Anchor>>>;

/// Live sources: in the background, wait for ffmpeg's first HLS segment and, if the
/// playlist carries PROGRAM-DATE-TIME, replace the anchor with the segment's capture time.
/// `rel_of_segment` is the relative time of that segment's first sample (0 for our decoder).
pub fn upgrade_from_hls(url: String, first: crate::ingest::FirstSegment, anchor: SharedAnchor, rel_of_segment: f64) {
    std::thread::spawn(move || {
        for _ in 0..300 {
            let seg = first.lock().unwrap().clone();
            if let Some(seg) = seg {
                if let Some(t) = hls_segment_utc(&url, &seg) {
                    *anchor.lock().unwrap() = Some(Anchor {
                        utc0: t - rel_of_segment,
                        source: "stream PROGRAM-DATE-TIME, segment-exact (stamped by the broadcaster or platform)".into(),
                    });
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

/// PC-clock anchor for a live source: `rel` seconds of audio have arrived by now.
pub fn received_anchor(rel: f64) -> Anchor {
    Anchor { utc0: now_unix() - rel, source: "PC clock at reception (late by the stream's delay)".into() }
}

/// `creation_time` from container metadata (ffprobe), if any.
pub fn file_creation_time(path: &str) -> Option<f64> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format_tags=creation_time", "-of", "default=nw=1:nk=1", path])
        .output()
        .ok()?;
    parse_iso8601(String::from_utf8_lossy(&out.stdout).lines().next()?)
}

/// Sidecar written by our recorder: `<file without extension>.json` with `"utc_start"`.
pub fn sidecar_anchor(path: &str) -> Option<Anchor> {
    let stem = path.rsplit_once('.').map_or(path, |x| x.0);
    let text = std::fs::read_to_string(format!("{stem}.json")).ok()?;
    let v = json_str(&text, "utc_start")?;
    let src = json_str(&text, "utc_source").unwrap_or_else(|| "unknown".into());
    Some(Anchor { utc0: parse_iso8601(&v)?, source: format!("recording sidecar ({src})") })
}

/// Best anchor for a file: our sidecar, else container creation_time (low confidence).
pub fn file_anchor(path: &str) -> Option<Anchor> {
    sidecar_anchor(path).or_else(|| {
        file_creation_time(path).map(|t| Anchor { utc0: t, source: "file creation_time (low confidence)".into() })
    })
}

/// Minimal string-field lookup in our own flat JSON (no serde).
fn json_str(text: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let i = text.find(&pat)? + pat.len();
    let rest = text[i..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// Escape a string for JSON output.
pub fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_roundtrip() {
        let t = parse_iso8601("2026-11-04T20:06:39.312+00:00").unwrap();
        assert_eq!(fmt_utc(t), "2026-11-04T20:06:39.312Z");
        let t2 = parse_iso8601("2026-11-04T22:06:39.312+02:00").unwrap();
        assert!((t - t2).abs() < 1e-6);
        assert_eq!(fmt_utc(0.0), "1970-01-01T00:00:00.000Z");
        assert_eq!(utc_stamp(t), "20261104-200639");
    }

    #[test]
    fn segment_keys() {
        assert_eq!(segment_key("https://rr3.x.com/videoplayback/id/a/sq/4106074/goap/x/seg.ts"), "sq:4106074");
        assert_eq!(segment_key("https://rr4.y.com/videoplayback/id/b/sq/4106074/dur/5/seg.ts"), "sq:4106074");
        assert_eq!(segment_key("https://cdn.example/live/seg123.ts?tok=1"), "live/seg123.ts");
    }
}
