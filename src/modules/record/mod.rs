//! Result record node: stores findings and evidence, and the location they agree on.

use crate::core::{NodeSpec, Port, PortKind, Role};

pub const NAME: &str = "record";
pub const LABEL: &str = "Result record";

/// Record inputs: findings from up to six nodes, facts from up to three, locations from up to three.
pub const EVENTS: [&str; 6] = ["findings 1", "findings 2", "findings 3", "findings 4", "findings 5", "findings 6"];
pub const FACTS: [&str; 3] = ["facts 1", "facts 2", "facts 3"];
pub const LOCATIONS: [&str; 3] = ["location 1", "location 2", "location 3"];

pub fn spec() -> NodeSpec {
    NodeSpec {
        name: NAME,
        role: Role::Record,
        inputs: EVENTS
            .iter()
            .map(|n| Port::opt(n, PortKind::Event))
            .chain(FACTS.iter().map(|n| Port::opt(n, PortKind::Json)))
            .chain(LOCATIONS.iter().map(|n| Port::opt(n, PortKind::Location)))
            .collect(),
        outputs: vec![Port::req("location", PortKind::Location)],
    }
}
