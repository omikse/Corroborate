//! The Source data node in the workbench: on the canvas, what to open (file, web page, stream URL
//! or microphone), Browse and Microphone buttons, and what it gave so far. Web pages are resolved
//! with yt-dlp. Compiled into the `corroborate` program.

use super::*;

/// Direct media URLs ffmpeg opens itself; every other http(s) link is a page for yt-dlp.
const DIRECT: [&str; 12] = [".mp3", ".wav", ".flac", ".ogg", ".opus", ".m4a", ".aac", ".mp4", ".webm", ".mkv", ".m3u8", ".ts"];
/// A microphone is written as `mic:<device name>`.
const MIC: &str = "mic:";

/// What the node opens at Run: a microphone, a URL if it has a scheme, otherwise a file.
pub fn input(text: &str) -> Option<Input> {
    let t = text.trim().trim_matches('"');
    if t.is_empty() {
        None
    } else if let Some(d) = t.strip_prefix(MIC) {
        (!d.trim().is_empty()).then(|| Input::Mic(d.trim().to_string()))
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
        "file, YouTube link, stream URL or microphone"
    } else if t.starts_with(MIC) {
        "microphone (live)"
    } else if !t.contains("://") {
        "file (recording)"
    } else if t.contains("youtube.com/") || t.contains("youtu.be/") {
        "YouTube: video read as a recording, live stream read live (yt-dlp)"
    } else if !t.starts_with("http") || DIRECT.iter().any(|e| t.split('?').next().unwrap_or("").ends_with(e)) {
        "stream URL (ffmpeg)"
    } else {
        "web page (yt-dlp)"
    }
}

/// A direct media URL, and whether it is live. Web pages go through yt-dlp, which says both;
/// direct URLs are live unless they end in a file extension (an mp3 on a server is a recording).
pub fn resolve(url: &str) -> Result<(String, bool), String> {
    if !kind_of(url).contains("yt-dlp") {
        let path = url.to_lowercase();
        let file = DIRECT.iter().filter(|e| **e != ".m3u8").any(|e| path.split('?').next().unwrap_or("").ends_with(e));
        return Ok((url.to_string(), !file));
    }
    let out = corroborate::no_window(&mut std::process::Command::new("yt-dlp"))
        .args(["--no-warnings", "--no-playlist", "-f", "ba/b", "--print", "is_live", "--print", "urls", url])
        .output()
        .map_err(|e| format!("could not run yt-dlp (is it installed?): {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let is_live = lines.next().map(str::trim) == Some("True");
    match lines.next() {
        Some(u) if out.status.success() && u.contains("://") && is_live => Ok((u.trim().to_string(), true)),
        // Not live: YouTube refuses its direct links to ffmpeg (403), so yt-dlp downloads the
        // audio into recordings\ and it is read as a file. The copy stays there as evidence.
        Some(_) if out.status.success() => download(url).map(|p| (p, false)),
        _ => Err(format!("yt-dlp: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no URL returned"))),
    }
}

/// Download a video's audio with yt-dlp to `recordings\<site> <id>.<ext>`; reuses an earlier download.
fn download(url: &str) -> Result<String, String> {
    let dir = corroborate::home().join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let out = corroborate::no_window(&mut std::process::Command::new("yt-dlp"))
        .args(["--no-warnings", "--no-playlist", "-f", "ba/b", "--print", "after_move:filepath", "-o"])
        .arg(dir.join("%(extractor)s %(id)s.%(ext)s"))
        .arg(url)
        .output()
        .map_err(|e| format!("could not run yt-dlp (is it installed?): {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match text.lines().rev().find(|l| !l.trim().is_empty()) {
        Some(p) if out.status.success() && std::path::Path::new(p.trim()).exists() => Ok(p.trim().to_string()),
        _ => Err(format!("yt-dlp download: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no file written"))),
    }
}

/// The Source data node on the canvas: the path or URL with Browse and Microphone, its kind, then
/// status, audio and clock.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, _app: &App, _i: NodeId, _standard: Vec2) -> Vec2 {
        Vec2::new(400.0, 165.0)
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let z = app.zoom;
        let st = app.nodes[i].state.clone();
        let Kind::Source { input } = &app.nodes[i].kind else { return };
        let mut input = input.clone();
        let mut refresh = false;
        crate::look::area(ui, r, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut input).hint_text("file, YouTube link or stream URL").desired_width(r.width() - 140.0 * z));
                if ui.button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("audio / video", &["mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "mp4", "mkv", "webm", "mov", "avi"])
                        .add_filter("any", &["*"])
                        .pick_file()
                    {
                        input = p.to_string_lossy().into_owned();
                    }
                }
                let mic = ui.menu_button("Mic ▾", |ui| {
                    if app.mics.is_empty() {
                        ui.label("no microphones found");
                    }
                    for m in &app.mics {
                        if ui.button(m).clicked() {
                            input = format!("{MIC}{m}");
                            ui.close();
                        }
                    }
                });
                refresh = mic.response.clicked();
            });
            crate::look::line(ui, kind_of(&input), DIM);
            source_info(ui, &st);
        });
        if refresh {
            match corroborate::ingest::list_microphones() {
                Ok(m) => app.mics = m,
                Err(e) => app.msg = e,
            }
        }
        if let Kind::Source { input: d } = &mut app.nodes[i].kind {
            *d = input;
        }
    }

    fn status(&self) -> bool {
        false
    }
}
