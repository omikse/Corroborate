//! Source data node: any audio source in one node. A file on disk (mp3, wav, video, anything ffmpeg
//! reads), a web page (YouTube video or live stream, and the other sites yt-dlp knows), or a direct
//! stream URL (http, rtmp, rtsp, hls…).

use crate::core::{NodeSpec, Role, SourceKind};

pub const NAME: &str = "source";
pub const LABEL: &str = "Source data";

pub fn spec() -> NodeSpec {
    NodeSpec { name: NAME, role: Role::Source(SourceKind::Live), inputs: vec![], outputs: crate::modules::source_outputs() }
}
