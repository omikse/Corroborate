//! Invert node: flips a location, so what it added now subtracts, and the other way round.

use crate::core::{NodeSpec, Port, PortKind, Role};

crate::builtin_manifest!();

pub fn spec() -> NodeSpec {
    NodeSpec {
        name: manifest().id,
        role: Role::Transform,
        inputs: vec![Port::req("location", PortKind::Location)],
        outputs: vec![Port::req("location", PortKind::Location)],
    }
}
