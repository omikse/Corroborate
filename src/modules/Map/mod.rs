//! Map node: shows an area on a world map in the Robinson projection, over NASA Blue Marble.

use crate::core::{NodeSpec, Port, PortKind, Role};

crate::builtin_manifest!();

pub fn spec() -> NodeSpec {
    NodeSpec { name: manifest().id, role: Role::View, inputs: vec![Port::req("location", PortKind::Location)], outputs: vec![] }
}
