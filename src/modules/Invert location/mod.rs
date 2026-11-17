//! Invert node: everywhere except the location wired in, as a plain area.

use crate::core::NodeSpec;

crate::builtin_manifest!(look);

/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
}
