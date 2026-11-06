//! Map node: shows an area on a world map in the Robinson projection, over NASA Blue Marble.

use crate::core::NodeSpec;

crate::builtin_manifest!(look);

/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
}
