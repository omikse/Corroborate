//! Manual data node: facts typed in by a person (date, time, place, coordinates, notes, own fields).

use crate::core::NodeSpec;

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

/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
}
