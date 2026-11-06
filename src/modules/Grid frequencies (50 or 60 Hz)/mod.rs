//! Grid frequencies node: the 50/60 Hz zones by region that ENF readings are matched against.
//! The data ships as `grid-frequencies.json` in this folder; an empty path means that file.

use crate::core::{NodeSpec, Port, PortKind, Role, SourceKind};

crate::builtin_manifest!();
pub const BUNDLED_FILE: &str = "grid-frequencies.json";
/// This folder, from the repository root (where the bundled file is found in a source build).
pub const SOURCE_DIR: &str = "src/modules/Grid frequencies (50 or 60 Hz)";

pub fn spec() -> NodeSpec {
    NodeSpec {
        name: manifest().id,
        role: Role::Source(SourceKind::Reference),
        inputs: vec![],
        outputs: vec![Port::req("data", PortKind::Json)],
    }
}
