//! Invert node: flips a location, so what it added now subtracts, and the other way round.

use crate::core::{NodeSpec, Port, PortKind, Role};

pub const NAME: &str = "invert";
pub const LABEL: &str = "Invert location";

pub fn spec() -> NodeSpec {
    NodeSpec {
        name: NAME,
        role: Role::Transform,
        inputs: vec![Port::req("location", PortKind::Location)],
        outputs: vec![Port::req("location", PortKind::Location)],
    }
}
