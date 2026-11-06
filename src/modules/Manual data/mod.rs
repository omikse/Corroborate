//! Manual data node: facts typed in by a person (date, time, place, coordinates, notes, own fields).

use crate::core::{NodeSpec, Port, PortKind, Role, SourceKind};

crate::builtin_manifest!();
/// The fields every Manual data node starts with, and what to type in them.
pub const FIELDS: [(&str, &str); 5] = [
    ("date", "YYYY-MM-DD"),
    ("time", "HH:MM, with the time zone"),
    ("place", "town, region, country"),
    ("coordinates", "lat, lon"),
    ("notes", "anything else"),
];

pub fn defaults() -> Vec<(String, String)> {
    FIELDS.iter().map(|(k, _)| (k.to_string(), String::new())).collect()
}

pub fn spec() -> NodeSpec {
    NodeSpec {
        name: manifest().id,
        role: Role::Source(SourceKind::Reference),
        inputs: vec![],
        outputs: vec![Port::req("place", PortKind::Location), Port::req("time", PortKind::Time), Port::req("facts", PortKind::Json)],
    }
}
