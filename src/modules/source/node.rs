//! The Source data node in the workbench: on the canvas, what to open (file, web page or stream
//! URL), a Browse button, and what it gave so far. Web pages are resolved with yt-dlp.
//! Compiled into the `corroborate` program.

use super::*;

/// Direct media URLs ffmpeg opens itself; every other http(s) link is a page for yt-dlp.
const DIRECT: [&str; 12] = [".mp3", ".wav", ".flac", ".ogg", ".opus", ".m4a", ".aac", ".mp4", ".webm", ".mkv", ".m3u8", ".ts"];

/// What the node opens at Run: a URL if it has a scheme, otherwise a file.
pub fn input(text: &str) -> Option<Input> {
    let t = text.trim().trim_matches('"');
    if t.is_empty() {
        None
    } else if t.contains("://") {
        Some(Input::Url(t.to_string()))
    } else {
        Some(Input::File(t.to_string()))
    }
}

/// What kind of source the text is, for the node's label line.
fn kind_of(text: &str) -> &'static str {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        "file, web page or stream URL"
    } else if !t.contains("://") {
        "file"
    } else if t.contains("youtube.com/") || t.contains("youtu.be/") {
        "YouTube (yt-dlp)"
    } else if !t.starts_with("http") || DIRECT.iter().any(|e| t.split('?').next().unwrap_or("").ends_with(e)) {
        "stream URL (ffmpeg)"
    } else {
        "web page (yt-dlp)"
    }
}

/// Web pages are resolved to a direct media URL with yt-dlp; direct URLs are used as they are.
pub fn resolve(url: &str) -> Result<String, String> {
    if !kind_of(url).contains("yt-dlp") {
        return Ok(url.to_string());
    }
    let out = corroborate::no_window(&mut std::process::Command::new("yt-dlp"))
        .args(["--no-warnings", "--no-playlist", "-g", "-f", "ba/b", url])
        .output()
        .map_err(|e| format!("could not run yt-dlp (is it installed?): {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match text.lines().next() {
        Some(u) if out.status.success() && u.contains("://") => Ok(u.trim().to_string()),
        _ => Err(format!("yt-dlp: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no URL returned"))),
    }
}

/// The Source data node on the canvas: the path or URL with Browse, its kind, then status, audio and clock.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, _app: &App, _i: NodeId, _standard: Vec2) -> Vec2 {
        Vec2::new(380.0, 165.0)
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let z = app.zoom;
        let st = app.nodes[i].state.clone();
        let Kind::Source { input } = &mut app.nodes[i].kind else { return };
        crate::look::area(ui, r, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(input).hint_text("file, YouTube link or stream URL").desired_width(r.width() - 70.0 * z));
                if ui.button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("audio / video", &["mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "mp4", "mkv", "webm", "mov", "avi"])
                        .add_filter("any", &["*"])
                        .pick_file()
                    {
                        *input = p.to_string_lossy().into_owned();
                    }
                }
            });
            crate::look::line(ui, kind_of(input), DIM);
            source_info(ui, &st);
        });
    }

    fn status(&self) -> bool {
        false
    }
}
