//! File node: a recording on disk (WAV, or anything ffmpeg reads) as a source.

use crate::core::{NodeSpec, Role, SourceKind};

pub const NAME: &str = "file";
pub const LABEL: &str = "File";

pub fn spec() -> NodeSpec {
    NodeSpec { name: NAME, role: Role::Source(SourceKind::File), inputs: vec![], outputs: crate::modules::source_outputs() }
}
