//! SOURCER node: any source in one node. A file on disk (mp3, wav, video, anything ffmpeg reads),
//! a web page (YouTube video or live stream, and the other sites yt-dlp knows), a direct stream
//! URL (http, rtmp, rtsp, hls…) or a microphone. Its outputs follow what it opens: audio, time
//! and metadata always, video when the source has pictures.

use crate::core::{NodeSpec, Port, PortKind, Role, SourceKind};

crate::builtin_manifest!();

/// File endings that carry pictures.
pub const VIDEO_EXT: [&str; 9] = [".mp4", ".mkv", ".webm", ".mov", ".avi", ".m4v", ".flv", ".ts", ".m3u8"];

/// Whether the text (path, URL or `mic:` device) names something with pictures: a video file,
/// a web page yt-dlp reads (YouTube and the like) or a video stream.
pub fn has_video(text: &str) -> bool {
    let t = text.trim().trim_matches('"').to_lowercase();
    if t.is_empty() || t.starts_with("mic:") {
        return false;
    }
    let path = t.split('?').next().unwrap_or("");
    if VIDEO_EXT.iter().any(|e| path.ends_with(e)) {
        return true;
    }
    // A web page (no media ending) is read by yt-dlp, which gets the video with it.
    t.starts_with("http") && !path.rsplit('/').next().unwrap_or("").contains('.')
        || t.contains("youtube.com/")
        || t.contains("youtu.be/")
}

/// Outputs for this source: audio, time, metadata, and video when it has pictures. Video is last
/// so the other outputs keep their place when it comes and goes.
pub fn outputs(text: &str) -> Vec<Port> {
    // Nothing chosen yet: no outputs. They appear once a source is picked.
    if text.trim().trim_matches('"').is_empty() {
        return Vec::new();
    }
    let mut v = crate::modules::source_outputs();
    v.push(Port::req("metadata", PortKind::Json));
    if has_video(text) {
        v.push(Port::req("video", PortKind::Video));
    }
    v
}

pub fn spec_for(text: &str) -> NodeSpec {
    NodeSpec {
        name: manifest().id,
        role: Role::Source(SourceKind::Live),
        // Text from another node replaces what is typed in the field.
        inputs: vec![Port::opt("text", PortKind::Text)],
        outputs: outputs(text),
    }
}

pub fn spec() -> NodeSpec {
    spec_for("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_follows_the_source() {
        assert!(!has_video(r"C:\a\rec.mp3"));
        assert!(!has_video("mic:Microphone (USB)"));
        assert!(has_video(r"C:\a\clip.MP4"));
        assert!(has_video("https://www.youtube.com/watch?v=abc"));
        assert!(!has_video("https://radio.example/stream.mp3"));
        assert!(outputs("").is_empty());
        assert!(outputs("  ").is_empty());
        assert!(spec().outputs.is_empty());
        assert_eq!(outputs("x.wav").len(), 3);
        assert_eq!(outputs("x.mkv").last().unwrap().name, "video");
    }
}
