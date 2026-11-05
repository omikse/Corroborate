//! Map node: shows an area on a world map in the Robinson projection, over NASA Blue Marble.

use crate::core::{NodeSpec, Port, PortKind, Role};

pub const NAME: &str = "map";
pub const LABEL: &str = "Map";

pub fn spec() -> NodeSpec {
    NodeSpec { name: NAME, role: Role::View, inputs: vec![Port::req("location", PortKind::Location)], outputs: vec![] }
}
