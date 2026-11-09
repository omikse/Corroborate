//! Timeline node: recordings drawn where the times wired in say they were. It only displays; the
//! rows on the canvas are in `node.rs`.

use crate::core::NodeSpec;

crate::builtin_manifest!();

/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
}
