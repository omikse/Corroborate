//! The data bus: one value type per port kind, so any module's output can feed any module's
//! input of the same kind. Each kind has one JSON form, used by programs (protocol 2, see
//! `plugin`) and by the Result record, which stores whatever is wired into it.
//!
//! | kind       | JSON                                                                          |
//! |------------|-------------------------------------------------------------------------------|
//! | `time`     | `{"utc": 1793839234.0, "error_s": 0.5, "source": "file metadata", "recording": "n3"}`: UTC (Unix seconds) ± `error_s`, where it comes from, and which recording it belongs to (the stable id of its source; absent for a time typed in UTC) |
//! | `location` | `{"cells": [[row, first_col, last_col], …], "note": "…"}` (1° cells, row 0 = 89–90° N, col 0 = 180–179° W), plus `"lat"`, `"lon"` and `"elevation"` (m) when it is a point; programs may send `{"lat": 50.1, "lon": 14.4, "radius_km": 25}` instead |
//! | `event`    | `[{"clock": "n3", "t_start": 12.0, "t_end": 14.5, "label": "…", "detail": "…", "error_s": 0.1}, …]` (`error_s`: the event's own measurement error) |
//! | `track`    | `[{"clock": "n3", "id": "h1", "t": 12.0, "value": 59.993, "ok": true}, …]` |
//! | `text`     | `"any text"`                                                                  |
//! | `json`     | any JSON                                                                      |
//! | `file`     | `"C:\\path\\to\\file"`                                                        |
//!
//! Audio and video are not values: audio is the raw stream fed to detectors.
//! Time, location, text, json and file keep the latest value; event and track lists grow.

use crate::area::{Area, WorldMask, COLS, ROWS};
use crate::core::PortKind;
use crate::plugin::{parse_json, Json};
use crate::timebase::json_escape;
use std::sync::Arc;

/// A timed finding: seconds on its clock (by default the source's, from its first sample).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Event {
    /// The clock its times are on (`clock::ClockId`); empty when not known.
    pub clock: String,
    pub t_start: f64,
    pub t_end: f64,
    pub label: String,
    pub detail: String,
    pub error_s: f64,
}

/// One point of a measured line (a frequency, a level), seconds on its clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackPoint {
    /// The clock `t` is on; empty when not known.
    pub clock: String,
    pub id: String,
    pub t: f64,
    pub value: f64,
    pub ok: bool,
}

/// A point on the Earth: degrees, and metres above sea level when known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Site {
    pub lat: f64,
    pub lon: f64,
    pub elevation: Option<f64>,
}

impl Site {
    /// A point written as ISO 6709, as phones and cameras put it in video metadata:
    /// `+51.5074-000.1278/`, `+51.5074-000.1278+010.000/` (elevation in metres), signed decimal
    /// degrees, the ending `/` optional. None when it is not that or not on Earth.
    pub fn from_iso6709(s: &str) -> Option<Site> {
        let s = s.trim().trim_end_matches('/');
        // Split at each sign: [lat, lon, elevation?].
        let mut parts: Vec<String> = Vec::new();
        for ch in s.chars() {
            match ch {
                '+' | '-' => parts.push(ch.to_string()),
                '0'..='9' | '.' => parts.last_mut()?.push(ch),
                _ => return None,
            }
        }
        if !(2..=3).contains(&parts.len()) {
            return None;
        }
        let num = |p: &str| p.parse::<f64>().ok().filter(|v| v.is_finite());
        let (lat, lon) = (num(&parts[0])?, num(&parts[1])?);
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return None;
        }
        let elevation = match parts.get(2) {
            Some(e) => Some(num(e)?),
            None => None,
        };
        Some(Site { lat, lon, elevation })
    }
}

/// A value on a port.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    /// A moment in UTC (Unix seconds) ± `error_s`, the time's whole error (1 sigma); `source`
    /// says where it comes from, `recording` which recording it belongs to (its source's stable
    /// id; empty for a time typed in UTC).
    Time { utc: f64, error_s: f64, source: String, recording: String },
    /// An area; `site` is the point it was made from, when it was (a typed latitude and longitude).
    Location { area: Arc<Area>, note: String, site: Option<Site> },
    Events(Vec<Event>),
    Tracks(Vec<TrackPoint>),
    Text(String),
    /// Any JSON, kept as its text.
    Json(String),
    File(String),
    /// A value of a type from a type.toml (src/types.rs), checked against its fields, kept as
    /// its JSON text.
    Custom { kind: &'static str, json: String },
}

impl Data {
    pub fn kind(&self) -> PortKind {
        match self {
            Data::Time { .. } => PortKind::Time,
            Data::Location { .. } => PortKind::Location,
            Data::Events(_) => PortKind::Event,
            Data::Tracks(_) => PortKind::Track,
            Data::Text(_) => PortKind::Text,
            Data::Json(_) => PortKind::Json,
            Data::File(_) => PortKind::File,
            Data::Custom { kind, .. } => PortKind::Custom(kind),
        }
    }

    /// Lists grow (a new value is added to what came before); the rest is replaced.
    pub fn merge(&mut self, new: Data) {
        match (self, new) {
            (Data::Events(a), Data::Events(b)) => a.extend(b),
            (Data::Tracks(a), Data::Tracks(b)) => a.extend(b),
            (Data::Custom { kind: a, json: ja }, Data::Custom { kind: b, json: jb }) if *a == b && crate::types::lookup(b).is_some_and(|t| t.flow == crate::types::Flow::List) => {
                let mut items = parse_json(ja).map(|j| list(&j)).unwrap_or_default();
                items.extend(parse_json(&jb).map(|j| list(&j)).unwrap_or_default());
                *ja = write_json(&Json::Arr(items));
            }
            (me, new) => *me = new,
        }
    }

    /// Its JSON form (see the table above).
    pub fn to_json(&self) -> String {
        match self {
            Data::Time { utc, error_s, source, recording } => {
                let recording = if recording.is_empty() { String::new() } else { format!(",\"recording\":\"{}\"", json_escape(recording)) };
                format!("{{\"utc\":{utc:.3},\"error_s\":{error_s},\"source\":\"{}\"{recording}}}", json_escape(source))
            }
            Data::Location { area, note, site } => {
                let site = site.map_or(String::new(), |s| format!(",\"lat\":{},\"lon\":{}{}", s.lat, s.lon, s.elevation.map_or(String::new(), |e| format!(",\"elevation\":{e}"))));
                format!("{{\"cells\":{}{site},\"note\":\"{}\"}}", area.cells.runs_json(), json_escape(note))
            }
            Data::Events(v) => {
                let e: Vec<String> = v
                    .iter()
                    .map(|e| {
                        format!(
                            "{{{}\"t_start\":{},\"t_end\":{},\"label\":\"{}\",\"detail\":\"{}\",\"error_s\":{}}}",
                            clock_json(&e.clock),
                            e.t_start,
                            e.t_end,
                            json_escape(&e.label),
                            json_escape(&e.detail),
                            e.error_s
                        )
                    })
                    .collect();
                format!("[{}]", e.join(","))
            }
            Data::Tracks(v) => {
                let p: Vec<String> = v
                    .iter()
                    .map(|p| format!("{{{}\"id\":\"{}\",\"t\":{},\"value\":{},\"ok\":{}}}", clock_json(&p.clock), json_escape(&p.id), p.t, p.value, p.ok))
                    .collect();
                format!("[{}]", p.join(","))
            }
            Data::Text(s) | Data::File(s) => format!("\"{}\"", json_escape(s)),
            Data::Json(s) => s.clone(),
            Data::Custom { json, .. } => json.clone(),
        }
    }

    /// A value of `kind` from its JSON form; says what is wrong when it does not fit.
    pub fn from_json(kind: PortKind, j: &Json) -> Result<Data, String> {
        let num = |o: &Json, k: &str| o.get(k).and_then(Json::num);
        let text = |o: &Json, k: &str| o.get(k).and_then(Json::str).unwrap_or("").to_string();
        match kind {
            PortKind::Time => {
                let utc = num(j, "utc").ok_or("time needs \"utc\" (Unix seconds)")?;
                Ok(Data::Time { utc, error_s: num(j, "error_s").unwrap_or(0.0), source: text(j, "source"), recording: text(j, "recording") })
            }
            PortKind::Location => {
                let note = text(j, "note");
                let site = match (num(j, "lat"), num(j, "lon")) {
                    (Some(lat), Some(lon)) if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) => return Err(format!("location {lat}, {lon} is not on Earth")),
                    (Some(lat), Some(lon)) => Some(Site { lat, lon, elevation: num(j, "elevation") }),
                    _ => None,
                };
                // Cells when given (they are the area); else a circle around the point.
                let cells = if let (Some(s), None) = (site, j.get("cells")) {
                    WorldMask::circle(s.lat, s.lon, num(j, "radius_km").unwrap_or(0.0).max(0.0))
                } else {
                    let mut m = WorldMask::none();
                    let runs = j.get("cells").ok_or("location needs \"cells\" or \"lat\" and \"lon\"")?;
                    for r in runs.arr() {
                        let v: Vec<usize> = r.arr().iter().filter_map(Json::num).map(|x| x as usize).collect();
                        let [row, a, b] = v[..] else { return Err("each cell run is [row, first_col, last_col]".into()) };
                        if row >= ROWS || a > b || b >= COLS {
                            return Err(format!("cell run [{row},{a},{b}] is outside the 180 × 360 grid"));
                        }
                        for c in a..=b {
                            m.cells[row * COLS + c] = true;
                        }
                    }
                    m
                };
                Ok(Data::Location { area: Arc::new(Area::new(cells)), note, site })
            }
            PortKind::Event => {
                let items = list(j);
                Ok(Data::Events(
                    items
                        .iter()
                        .map(|e| {
                            let t_start = num(e, "t_start").or(num(e, "t")).unwrap_or(0.0);
                            Event {
                                clock: text(e, "clock"),
                                t_start,
                                t_end: num(e, "t_end").unwrap_or(t_start),
                                label: text(e, "label"),
                                detail: text(e, "detail"),
                                error_s: num(e, "error_s").unwrap_or(0.0),
                            }
                        })
                        .collect(),
                ))
            }
            PortKind::Track => Ok(Data::Tracks(
                list(j)
                    .iter()
                    .map(|p| TrackPoint { clock: text(p, "clock"), id: text(p, "id"), t: num(p, "t").unwrap_or(0.0), value: num(p, "value").unwrap_or(f64::NAN), ok: !matches!(p.get("ok"), Some(Json::Bool(false))) })
                    .collect(),
            )),
            PortKind::Text => Ok(Data::Text(j.str().ok_or("text must be a string")?.to_string())),
            PortKind::File => Ok(Data::File(j.str().ok_or("file must be a path as a string")?.to_string())),
            PortKind::Json => Ok(Data::Json(write_json(j))),
            PortKind::Custom(id) => {
                let t = crate::types::lookup(id).ok_or(format!("type \"{id}\" is not known"))?;
                let items = if t.flow == crate::types::Flow::List { list(j) } else { vec![j.clone()] };
                for item in &items {
                    check_fields(&t, item)?;
                }
                Ok(Data::Custom { kind: id, json: write_json(j) })
            }
            PortKind::Audio | PortKind::Video | PortKind::Any => Err(format!("{kind:?} is not sent as a value")),
        }
    }

    /// As `from_json`, from text.
    pub fn parse(kind: PortKind, text: &str) -> Result<Data, String> {
        Data::from_json(kind, &parse_json(text).ok_or("not JSON")?)
    }
}

/// Says what is wrong when `j` does not fit type `t`'s fields (a field left out is fine).
fn check_fields(t: &crate::types::TypeDef, j: &Json) -> Result<(), String> {
    use crate::types::FieldType as F;
    let fits = |ty: &F, v: &Json| match (ty, v) {
        (F::Number { .. } | F::Datetime, Json::Num(_)) | (F::Bool, Json::Bool(_)) | (F::Text | F::File | F::Choice(_), Json::Str(_)) | (_, Json::Null) => true,
        _ => false,
    };
    let what = |ty: &F| match ty {
        F::Number { .. } => "a number",
        F::Datetime => "Unix seconds",
        F::Bool => "true or false",
        _ => "text",
    };
    match &t.fields[..] {
        [] => Ok(()),
        [f] if f.name == "value" => match fits(&f.ty, j) {
            true => Ok(()),
            false => Err(format!("{} must be {}", t.id, what(&f.ty))),
        },
        fields => {
            if !matches!(j, Json::Obj(_)) {
                return Err(format!("{} must be an object {{\"{}\": …}}", t.id, fields[0].name));
            }
            for f in fields {
                if let Some(v) = j.get(&f.name) {
                    if !fits(&f.ty, v) {
                        return Err(format!("{}: \"{}\" must be {}", t.id, f.name, what(&f.ty)));
                    }
                }
            }
            Ok(())
        }
    }
}

/// One object or a list of them.
fn list(j: &Json) -> Vec<Json> {
    match j {
        Json::Arr(v) => v.clone(),
        Json::Obj(_) => vec![j.clone()],
        _ => Vec::new(),
    }
}

/// JSON text of a parsed document.
/// `"clock":"…",` for an item that knows its clock, else nothing.
fn clock_json(c: &str) -> String {
    if c.is_empty() {
        String::new()
    } else {
        format!("\"clock\":\"{}\",", json_escape(c))
    }
}

pub fn write_json(j: &Json) -> String {
    match j {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Num(x) if x.is_finite() => x.to_string(),
        Json::Num(_) => "null".into(),
        Json::Str(s) => format!("\"{}\"", json_escape(s)),
        Json::Arr(v) => format!("[{}]", v.iter().map(write_json).collect::<Vec<_>>().join(",")),
        Json::Obj(v) => format!("{{{}}}", v.iter().map(|(k, x)| format!("\"{}\":{}", json_escape(k), write_json(x))).collect::<Vec<_>>().join(",")),
    }
}

/// Base64 (standard alphabet, padded): how protocol 2 sends audio inside a JSON line.
pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// Bytes of base64 text (for programs written in Rust that read protocol 2).
pub fn unbase64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let v: Vec<u8> = s.bytes().filter_map(val).collect();
    let mut out = Vec::with_capacity(v.len() * 3 / 4);
    for c in v.chunks(4) {
        let n = c.iter().enumerate().fold(0u32, |n, (i, &x)| n | (x as u32) << (18 - 6 * i));
        for i in 0..c.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso6709_points() {
        // Apple (com.apple.quicktime.location.ISO6709), with elevation.
        assert_eq!(Site::from_iso6709("+51.5074-000.1278+010.000/"), Some(Site { lat: 51.5074, lon: -0.1278, elevation: Some(10.0) }));
        // Android (location, location-eng): no elevation.
        assert_eq!(Site::from_iso6709("+40.7128-074.0060/"), Some(Site { lat: 40.7128, lon: -74.006, elevation: None }));
        assert_eq!(Site::from_iso6709("-33.8688+151.2093"), Some(Site { lat: -33.8688, lon: 151.2093, elevation: None }));
        for bad in ["", "51.5,-0.12", "+91.0+000.0/", "+10.0+190.0/", "+1.0", "+1.0+2.0+3.0+4.0/", "+1.0x+2.0/", "+-1.0+2.0/"] {
            assert_eq!(Site::from_iso6709(bad), None, "{bad}");
        }
    }

    /// Every kind's value comes back from its JSON form unchanged.
    #[test]
    fn every_kind_round_trips() {
        let mut cells = WorldMask::circle(50.0, 14.0, 300.0);
        cells.cells[0] = true;
        let values = [
            Data::Time { utc: 1_793_839_234.5, error_s: 0.25, source: "file \"metadata\"".into(), recording: String::new() },
            Data::Time { utc: 1_793_839_234.5, error_s: 2.0, source: String::new(), recording: "n3".into() },
            Data::Location { area: Arc::new(Area::new(cells.clone())), note: "not here".into(), site: None },
            Data::Location { area: Arc::new(Area::new(cells)), note: String::new(), site: Some(Site { lat: 42.4797, lon: -76.4514, elevation: Some(335.0) }) },
            Data::Events(vec![
                Event { clock: "n2".into(), t_start: 12.0, t_end: 14.5, label: "clap".into(), detail: "loud\nshort".into(), error_s: 0.1 },
                Event { t_start: 1.0, t_end: 1.0, ..Default::default() },
            ]),
            Data::Tracks(vec![
                TrackPoint { clock: "n2".into(), id: "h1".into(), t: 1.5, value: 59.993, ok: true },
                TrackPoint { id: "h1".into(), t: 2.5, value: 60.001, ok: false, ..Default::default() },
            ]),
            Data::Text("Příbram ½".into()),
            Data::Json("{\"a\":[1,2,{\"b\":null}],\"c\":\"d\"}".into()),
            Data::File(r"C:\cases\12\a.wav".into()),
        ];
        for v in values {
            let back = Data::parse(v.kind(), &v.to_json()).unwrap_or_else(|e| panic!("{v:?}: {e}"));
            assert_eq!(back, v);
        }
    }

    #[test]
    fn programs_may_send_a_circle() {
        let Ok(Data::Location { area, .. }) = Data::parse(PortKind::Location, r#"{"lat":50.1,"lon":14.4,"radius_km":200}"#) else { panic!() };
        assert_eq!(area.cells, WorldMask::circle(50.1, 14.4, 200.0));
        assert!(Data::parse(PortKind::Location, r#"{"lat":95,"lon":0}"#).unwrap_err().contains("not on Earth"));
        assert!(Data::parse(PortKind::Location, r#"{"cells":[[200,0,1]]}"#).unwrap_err().contains("outside"));
        assert!(Data::parse(PortKind::Time, r#"{"when":1}"#).unwrap_err().contains("utc"));
        let Ok(Data::Time { utc, recording, .. }) = Data::parse(PortKind::Time, r#"{"utc":60}"#) else { panic!() };
        assert_eq!((utc, recording.as_str()), (60.0, ""));
    }

    #[test]
    fn lists_grow_and_states_are_replaced() {
        let e = |l: &str| Data::Events(vec![Event { label: l.into(), ..Default::default() }]);
        let mut a = e("one");
        a.merge(e("two"));
        assert!(matches!(&a, Data::Events(v) if v.len() == 2));
        let mut t = Data::Text("old".into());
        t.merge(Data::Text("new".into()));
        assert_eq!(t, Data::Text("new".into()));
    }

    #[test]
    fn base64_round_trips() {
        for n in 0..10 {
            let b: Vec<u8> = (0..n).map(|i| (i * 37 + 5) as u8).collect();
            assert_eq!(unbase64(&base64(&b)), b);
        }
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
    }
}
