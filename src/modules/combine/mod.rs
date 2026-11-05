//! Combine node: several locations into one, with all of them (AND) or any of them (OR).

use crate::core::{NodeSpec, Port, PortKind, Role};

pub const NAME: &str = "combine";
pub const INPUTS: [&str; 4] = ["location 1", "location 2", "location 3", "location 4"];

/// The mode as written in project files and labels: "any" (OR) or "all" (AND).
pub fn mode(any: bool) -> &'static str {
    if any { "any" } else { "all" }
}

pub fn is_any(mode: &str) -> bool {
    mode == "any"
}

pub fn label(any: bool) -> String {
    format!("Combine locations ({})", mode(any))
}

pub fn spec() -> NodeSpec {
    NodeSpec {
        name: NAME,
        role: Role::Transform,
        inputs: INPUTS.iter().map(|n| Port::opt(n, PortKind::Location)).collect(),
        outputs: vec![Port::req("location", PortKind::Location)],
    }
}
