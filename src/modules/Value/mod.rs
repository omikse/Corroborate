//! Value node: one value of any data type, typed in by a person. Its output takes the type
//! picked; what it asks for is the type's fields (src/types.rs). A list type (events, a track)
//! holds several items, saved as rows `1.label`, `2.label`, …; a value type's rows are its field
//! names.

use crate::core::{NodeSpec, Port, PortKind};
use crate::data::Data;
use crate::module_manifest::{intern, port_kind};
use crate::types::{self, Flow, TypeDef};
use std::sync::{Arc, Mutex};

crate::builtin_manifest!();

/// The types a Value node can make: every value and list type (streams come from sources, and
/// "any" is only for inputs).
pub fn types() -> Vec<Arc<TypeDef>> {
    types::all().into_iter().filter(|t| t.flow != Flow::Stream && t.id != "any").collect()
}

/// The kind on the output for type `ty` (a type not known keeps its name, so its wires stay).
pub fn kind(ty: &str) -> PortKind {
    port_kind(ty).unwrap_or_else(|| PortKind::Custom(intern(ty)))
}

/// Its ports: one output, of its type.
pub fn spec(ty: &str) -> NodeSpec {
    let mut s = manifest().spec();
    s.outputs = vec![Port::req("value", kind(ty))];
    s
}

fn is_list(t: &TypeDef) -> bool {
    t.flow == Flow::List
}

/// The rows of one item of a list type: its fields, numbered `n`.
pub fn item_rows(t: &TypeDef, n: usize) -> Vec<(String, String)> {
    t.fields.iter().map(|f| (format!("{n}.{}", f.name), f.default.clone())).collect()
}

/// The rows a new node of type `ty` starts with: each field at its default (a list: one item).
pub fn defaults(ty: &str) -> Vec<(String, String)> {
    let Some(t) = types::lookup(ty) else { return Vec::new() };
    if is_list(&t) {
        return item_rows(&t, 1);
    }
    t.fields.iter().map(|f| (f.name.clone(), f.default.clone())).collect()
}

/// A list type's items, in order: each item's number and its rows by field name.
pub fn items(rows: &[(String, String)]) -> Vec<(usize, Vec<(String, String)>)> {
    let mut v: Vec<(usize, Vec<(String, String)>)> = Vec::new();
    for (k, x) in rows {
        let Some((n, f)) = k.split_once('.').and_then(|(n, f)| Some((n.parse::<usize>().ok()?, f))) else { continue };
        match v.iter_mut().find(|(m, _)| *m == n) {
            Some((_, r)) => r.push((f.to_string(), x.clone())),
            None => v.push((n, vec![(f.to_string(), x.clone())])),
        }
    }
    v
}

/// The value as JSON (a list type: an array of its items), or what is wrong with it.
pub fn json(ty: &str, rows: &[(String, String)]) -> Result<String, String> {
    let t = types::lookup(ty).ok_or(format!("type \"{ty}\" is not known: add types\\{ty}\\type.toml"))?;
    if is_list(&t) {
        let v: Vec<String> = items(rows).iter().map(|(_, r)| t.json(r)).collect::<Result<_, _>>()?;
        return Ok(format!("[{}]", v.join(", ")));
    }
    t.json(rows)
}

/// The value on the output. Building a location takes a few ms and the canvas asks every frame,
/// so the last few are kept, keyed by the JSON.
pub fn data(ty: &str, rows: &[(String, String)]) -> Result<Data, String> {
    type Memo = Vec<(String, Result<Data, String>)>;
    static MEMO: Mutex<Memo> = Mutex::new(Vec::new());
    let j = json(ty, rows)?;
    let key = format!("{ty}|{j}");
    if let Some((_, r)) = MEMO.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return r.clone();
    }
    let r = Data::parse(kind(ty), &j);
    let mut memo = MEMO.lock().unwrap();
    if memo.len() >= 8 {
        let _ = memo.remove(0);
    }
    memo.push((key, r.clone()));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(a: &[(&str, &str)]) -> Vec<(String, String)> {
        a.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn values_of_types() {
        assert_eq!(spec("time").outputs, [Port::req("value", PortKind::Time)]);
        assert!(types().iter().all(|t| !matches!(t.id, "audio" | "video" | "any")));
        let loc = data("location", &rows(&[("lat", "50"), ("lon", "15"), ("radius_km", "300"), ("add", "true")])).unwrap();
        assert!(matches!(&loc, Data::Location { area, .. } if area.add && area.allowed().fraction() > 0.0));
        let time = data("time", &rows(&[("utc0", "2026-11-04 20:06:39"), ("error_s", "2")])).unwrap();
        assert_eq!(time, Data::Time { utc0: 1793822799.0, error_s: 2.0, source: String::new() });
        assert_eq!(data("time", &rows(&[("utc0", "")])).unwrap_err(), "fill in time (UTC)");
        assert!(data("smell", &[]).unwrap_err().contains("not known"));
    }

    #[test]
    fn lists_have_items() {
        assert_eq!(defaults("event")[0].0, "1.t_start");
        let mut r = rows(&[("1.t_start", "3"), ("1.label", "clap")]);
        r.extend(rows(&[("2.t_start", "9"), ("2.label", "door")]));
        match data("event", &r).unwrap() {
            Data::Events(v) => assert_eq!(v.iter().map(|e| (e.t_start, e.label.as_str())).collect::<Vec<_>>(), [(3.0, "clap"), (9.0, "door")]),
            other => panic!("{other:?}"),
        }
    }
}
