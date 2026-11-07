//! Audio match node: which wired recordings hear the same sound, as links between their clocks.
//! The method is in the library (`crate::audio_match`), shared with whatever else needs it; the
//! node picks its inputs and gives the links and their quality.

use crate::core::NodeSpec;

crate::builtin_manifest!();

/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
}
