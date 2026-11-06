//! Result record node: takes anything wired into it, saves what is static at once and records
//! what streams until recording ends, and gives the location its location inputs agree on.

use crate::core::{NodeSpec, PortKind};

crate::builtin_manifest!();

/// Its ports: its module.toml (one input that takes anything and grows as it is wired).
pub fn spec() -> NodeSpec {
    manifest().spec()
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
        PortKind::Custom(id) => id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Port, Role};
    use crate::graph::Graph;

    /// Wiring into the free input adds the next one; unwiring packs them again.
    #[test]
    fn inputs_grow_and_pack() {
        let mut g = Graph::new();
        let src = g.add("m", NodeSpec { name: "m", role: Role::Transform, inputs: vec![], outputs: vec![Port::req("a", PortKind::Json), Port::req("b", PortKind::Audio)] });
        let rec = g.add("r", spec());
        let names = manifest().many_names();
        assert_eq!(g.nodes()[rec].spec.inputs[0].name, "in 1");
        g.connect(src, "a", rec, "in 1").unwrap();
        g.pack_many(rec, &names);
        assert_eq!(g.nodes()[rec].spec.inputs.len(), 2);
        g.connect(src, "b", rec, "in 2").unwrap();
        g.pack_many(rec, &names);
        assert_eq!(g.nodes()[rec].spec.inputs.len(), 3);
        g.disconnect(rec, "in 1");
        g.pack_many(rec, &names);
        assert_eq!(g.nodes()[rec].spec.inputs.len(), 2);
        assert_eq!(g.feeder(rec, "in 1"), Some((src, "b")));
    }
}
