//! Invert node: flips a location, so what it added now subtracts, and the other way round.

use crate::core::NodeSpec;

crate::builtin_manifest!(look);

/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
}
