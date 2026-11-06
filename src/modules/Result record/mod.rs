//! Result record node: takes anything wired into it, saves what is static at once and records
//! what streams until recording ends, and gives the location its location inputs agree on.

use crate::core::{NodeSpec, Port, PortKind, Role};

crate::builtin_manifest!();

/// Input names, in order. The node shows the wired ones plus one free (grey) input.
pub const INPUTS: [&str; 24] = [
    "in 1", "in 2", "in 3", "in 4", "in 5", "in 6", "in 7", "in 8", "in 9", "in 10", "in 11", "in 12", "in 13", "in 14", "in 15",
    "in 16", "in 17", "in 18", "in 19", "in 20", "in 21", "in 22", "in 23", "in 24",
];

/// A fresh record: one free input.
pub fn spec() -> NodeSpec {
    NodeSpec { name: manifest().id, role: Role::Record, inputs: vec![Port::opt(INPUTS[0], PortKind::Any)], outputs: vec![Port::req("location", PortKind::Location)] }
}

/// Kinds that keep arriving while a source runs: recorded from Record until End recording.
/// The rest (metadata, facts, places, text, files) is static: saved at once.
pub fn is_stream(k: PortKind) -> bool {
    matches!(k, PortKind::Audio | PortKind::Video | PortKind::Time | PortKind::Event | PortKind::Track)
}

pub fn kind_name(k: PortKind) -> &'static str {
    match k {
        PortKind::Audio => "audio",
        PortKind::Video => "video",
        PortKind::Time => "time",
        PortKind::Location => "location",
        PortKind::Track => "tracks",
        PortKind::Event => "events",
        PortKind::File => "file",
        PortKind::Text => "text",
        PortKind::Json => "data",
        PortKind::Any => "anything",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;

    /// Wiring into the free input adds the next one; unwiring packs them again.
    #[test]
    fn inputs_grow_and_pack() {
        let mut g = Graph::new();
        let src = g.add("m", NodeSpec { name: "m", role: Role::Transform, inputs: vec![], outputs: vec![Port::req("a", PortKind::Json), Port::req("b", PortKind::Audio)] });
        let rec = g.add("r", spec());
        g.connect(src, "a", rec, "in 1").unwrap();
        g.pack_any_inputs(rec, &INPUTS);
        assert_eq!(g.nodes()[rec].spec.inputs.len(), 2);
        g.connect(src, "b", rec, "in 2").unwrap();
        g.pack_any_inputs(rec, &INPUTS);
        assert_eq!(g.nodes()[rec].spec.inputs.len(), 3);
        g.disconnect(rec, "in 1");
        g.pack_any_inputs(rec, &INPUTS);
        assert_eq!(g.nodes()[rec].spec.inputs.len(), 2);
        assert_eq!(g.feeder(rec, "in 1"), Some((src, "b")));
    }
}
