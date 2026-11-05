//! The Stream node in the workbench: its window, what it opens at Run, and resolving web pages
//! to a stream URL. Compiled into the `corroborate` program.

use super::*;

/// What the node opens at Run, if a URL is set.
pub fn input(url: &str) -> Option<Input> {
    (!url.trim().is_empty()).then(|| Input::Url(url.trim().to_string()))
}

/// YouTube page links need resolving to a direct stream URL (yt-dlp); others are used as-is.
pub fn resolve(url: &str) -> Result<String, String> {
    if !(url.contains("youtube.com/") || url.contains("youtu.be/")) {
        return Ok(url.to_string());
    }
    let out = corroborate::no_window(&mut std::process::Command::new("yt-dlp"))
        .args(["--no-warnings", "-g", "-f", "ba/b", url])
        .output()
        .map_err(|e| format!("could not run yt-dlp (is it installed?): {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match text.lines().next() {
        Some(u) if out.status.success() && u.contains("://") => Ok(u.trim().to_string()),
        _ => Err(format!("yt-dlp: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no URL returned"))),
    }
}

pub fn window(ui: &mut egui::Ui, url: &mut String, st: &Shared) {
    ui.horizontal(|ui| {
        ui.label("URL");
        ui.add(egui::TextEdit::singleline(url).desired_width(340.0).hint_text("https://… (YouTube links are resolved with yt-dlp)"));
    });
    source_info(ui, st);
}
