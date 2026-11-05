//! Microphone node: live audio from an input device as a source.

use crate::core::{NodeSpec, Role, SourceKind};

pub const NAME: &str = "mic";
pub const LABEL: &str = "Microphone";

pub fn spec() -> NodeSpec {
    NodeSpec { name: NAME, role: Role::Source(SourceKind::Live), inputs: vec![], outputs: crate::modules::source_outputs() }
}
