//! The data bus: one value type per port kind, so any module's output can feed any module's
//! input of the same kind. Each kind has one JSON form, used by programs (protocol 2, see
//! `plugin`) and by the Result record, which stores whatever is wired into it.
//!
//! | kind       | JSON                                                                          |
//! |------------|-------------------------------------------------------------------------------|
//! | `time`     | `{"utc0": 1793839234.0, "error_s": 0.5, "source": "file metadata"}`           |
//! | `location` | `{"add": true, "cells": [[row, first_col, last_col], …], "note": "…"}` (1° cells, row 0 = 89–90° N, col 0 = 180–179° W); programs may send `{"lat": 50.1, "lon": 14.4, "radius_km": 25}` instead |
//! | `event`    | `[{"t_start": 12.0, "t_end": 14.5, "label": "…", "detail": "…", "error_s": 0.1}, …]` |
//! | `track`    | `[{"id": "h1", "t": 12.0, "value": 59.993, "ok": true}, …]`                 |
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

/// A timed finding: seconds from the source's first sample.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub t_start: f64,
    pub t_end: f64,
    pub label: String,
    pub detail: String,
    pub error_s: f64,
}

/// One point of a measured line (a frequency, a level), seconds from the first sample.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackPoint {
    pub id: String,
    pub t: f64,
    pub value: f64,
    pub ok: bool,
}

/// A value on a port.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    Time { utc0: f64, error_s: f64, source: String },
    Location { area: Arc<Area>, note: String },
    Events(Vec<Event>),
    Tracks(Vec<TrackPoint>),
    Text(String),
    /// Any JSON, kept as its text.
    Json(String),
    File(String),
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
        }
    }

    /// Lists grow (a new value is added to what came before); the rest is replaced.
    pub fn merge(&mut self, new: Data) {
        match (self, new) {
            (Data::Events(a), Data::Events(b)) => a.extend(b),
            (Data::Tracks(a), Data::Tracks(b)) => a.extend(b),
            (me, new) => *me = new,
        }
    }

    /// Its JSON form (see the table above).
    pub fn to_json(&self) -> String {
        match self {
            Data::Time { utc0, error_s, source } => format!("{{\"utc0\":{utc0:.3},\"error_s\":{error_s},\"source\":\"{}\"}}", json_escape(source)),
            Data::Location { area, note } => format!("{{\"add\":{},\"cells\":{},\"note\":\"{}\"}}", area.add, area.cells.runs_json(), json_escape(note)),
            Data::Events(v) => {
                let e: Vec<String> = v
                    .iter()
                    .map(|e| {
                        format!(
                            "{{\"t_start\":{},\"t_end\":{},\"label\":\"{}\",\"detail\":\"{}\",\"error_s\":{}}}",
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
                let p: Vec<String> = v.iter().map(|p| format!("{{\"id\":\"{}\",\"t\":{},\"value\":{},\"ok\":{}}}", json_escape(&p.id), p.t, p.value, p.ok)).collect();
                format!("[{}]", p.join(","))
            }
            Data::Text(s) | Data::File(s) => format!("\"{}\"", json_escape(s)),
            Data::Json(s) => s.clone(),
        }
    }

    /// A value of `kind` from its JSON form; says what is wrong when it does not fit.
    pub fn from_json(kind: PortKind, j: &Json) -> Result<Data, String> {
        let num = |o: &Json, k: &str| o.get(k).and_then(Json::num);
        let text = |o: &Json, k: &str| o.get(k).and_then(Json::str).unwrap_or("").to_string();
        match kind {
            PortKind::Time => {
                let utc0 = num(j, "utc0").ok_or("time needs \"utc0\" (Unix seconds)")?;
                Ok(Data::Time { utc0, error_s: num(j, "error_s").unwrap_or(0.0), source: text(j, "source") })
            }
            PortKind::Location => {
                let add = !matches!(j.get("add"), Some(Json::Bool(false)));
                let note = text(j, "note");
                let cells = if let (Some(lat), Some(lon)) = (num(j, "lat"), num(j, "lon")) {
                    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                        return Err(format!("location {lat}, {lon} is not on Earth"));
                    }
                    WorldMask::circle(lat, lon, num(j, "radius_km").unwrap_or(0.0).max(0.0))
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
                Ok(Data::Location { area: Arc::new(Area { cells, add }), note })
            }
            PortKind::Event => {
                let items = list(j);
                Ok(Data::Events(
                    items
                        .iter()
                        .map(|e| {
                            let t_start = num(e, "t_start").or(num(e, "t")).unwrap_or(0.0);
                            Event { t_start, t_end: num(e, "t_end").unwrap_or(t_start), label: text(e, "label"), detail: text(e, "detail"), error_s: num(e, "error_s").unwrap_or(0.0) }
                        })
                        .collect(),
                ))
            }
            PortKind::Track => Ok(Data::Tracks(
                list(j)
                    .iter()
                    .map(|p| TrackPoint { id: text(p, "id"), t: num(p, "t").unwrap_or(0.0), value: num(p, "value").unwrap_or(f64::NAN), ok: !matches!(p.get("ok"), Some(Json::Bool(false))) })
                    .collect(),
            )),
            PortKind::Text => Ok(Data::Text(j.str().ok_or("text must be a string")?.to_string())),
            PortKind::File => Ok(Data::File(j.str().ok_or("file must be a path as a string")?.to_string())),
            PortKind::Json => Ok(Data::Json(write_json(j))),
            PortKind::Audio | PortKind::Video | PortKind::Any => Err(format!("{kind:?} is not sent as a value")),
        }
    }

    /// As `from_json`, from text.
    pub fn parse(kind: PortKind, text: &str) -> Result<Data, String> {
        Data::from_json(kind, &parse_json(text).ok_or("not JSON")?)
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

    /// Every kind's value comes back from its JSON form unchanged.
    #[test]
    fn every_kind_round_trips() {
        let mut cells = WorldMask::circle(50.0, 14.0, 300.0);
        cells.cells[0] = true;
        let values = [
            Data::Time { utc0: 1_793_839_234.5, error_s: 0.25, source: "file \"metadata\"".into() },
            Data::Location { area: Arc::new(Area { cells, add: false }), note: "not here".into() },
            Data::Events(vec![Event { t_start: 12.0, t_end: 14.5, label: "clap".into(), detail: "loud\nshort".into(), error_s: 0.1 }]),
            Data::Tracks(vec![TrackPoint { id: "h1".into(), t: 1.5, value: 59.993, ok: true }, TrackPoint { id: "h1".into(), t: 2.5, value: 60.001, ok: false }]),
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
        assert!(area.add);
        assert!(Data::parse(PortKind::Location, r#"{"lat":95,"lon":0}"#).unwrap_err().contains("not on Earth"));
        assert!(Data::parse(PortKind::Location, r#"{"cells":[[200,0,1]]}"#).unwrap_err().contains("outside"));
        assert!(Data::parse(PortKind::Time, r#"{"when":1}"#).unwrap_err().contains("utc0"));
    }

    #[test]
    fn lists_grow_and_states_are_replaced() {
        let e = |l: &str| Data::Events(vec![Event { t_start: 0.0, t_end: 0.0, label: l.into(), detail: String::new(), error_s: 0.0 }]);
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
