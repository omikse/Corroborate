//! SOURCER node: any source in one node. A file on disk (mp3, wav, video, anything ffmpeg reads),
//! a web page (YouTube video or live stream, and the other sites yt-dlp knows), a direct stream
//! URL (http, rtmp, rtsp, hls…) or a microphone. Its outputs (module.toml) are audio, time, length,
//! metadata, video and location; it gives video only when the source has pictures, and location
//! only when its metadata holds coordinates (decided at run time, in the workbench). A file's time
//! and length are read once, before any Run; the time is what the file says, with an error of 0.

use crate::core::NodeSpec;

crate::builtin_manifest!(look);

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

/// Whether the source gives output `port` now: nothing until a source is chosen, video only
/// when it has pictures. The ports are always there (module.toml); look.rhai hides those off.
pub fn gives(text: &str, port: &str) -> bool {
    let t = text.trim().trim_matches('"');
    if t.is_empty() {
        return false;
    }
    match port {
        "video" => has_video(text),
        // A microphone goes on for as long as it is listened to.
        "length" => !t.starts_with("mic:"),
        _ => true,
    }
}

/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
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
        assert!(!gives("", "audio") && !gives("  ", "time"));
        assert!(gives("x.wav", "audio") && gives("x.wav", "metadata") && !gives("x.wav", "video"));
        assert!(gives("x.mkv", "video"));
        let names: Vec<&str> = spec().outputs.iter().map(|p| p.name).collect();
        assert_eq!(names, ["audio", "time", "length", "metadata", "video", "location"]);
        assert!(gives("x.wav", "length") && !gives("mic:USB", "length"));
        assert_eq!(spec().inputs[0].name, "text");
    }
}
