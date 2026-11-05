//! Grid frequencies node: the 50/60 Hz zones by region that ENF readings are matched against.
//! The data ships as `grid-frequencies.json` in this folder; an empty path means that file.

use crate::core::{NodeSpec, Port, PortKind, Role, SourceKind};

pub const NAME: &str = "grid-frequencies";
pub const LABEL: &str = "Grid frequencies (50/60 Hz)";
pub const BUNDLED_FILE: &str = "grid-frequencies.json";
/// This folder, from the repository root (where the bundled file is found in a source build).
pub const SOURCE_DIR: &str = "src/modules/grid_frequencies";

pub fn spec() -> NodeSpec {
    NodeSpec {
        name: NAME,
        role: Role::Source(SourceKind::Reference),
        inputs: vec![],
        outputs: vec![Port::req("data", PortKind::Json)],
    }
}
