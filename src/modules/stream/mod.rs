//! Stream node: a live or online source (URL, ffmpeg input, yt-dlp page) as a source.

use crate::core::{NodeSpec, Role, SourceKind};

pub const NAME: &str = "stream";
pub const LABEL: &str = "Stream";

pub fn spec() -> NodeSpec {
    NodeSpec { name: NAME, role: Role::Source(SourceKind::Live), inputs: vec![], outputs: crate::modules::source_outputs() }
}
