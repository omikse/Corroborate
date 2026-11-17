//! The SOURCER node in the workbench: on the canvas, a source box (file, web page or stream URL,
//! with Browse) and a system streams box (microphones, virtual mics, capture cards), then what it
//! gave so far, a waveform of the audio and, for sources with pictures, a video viewer: one still
//! frame while it is shown expanded, the video read one frame a second after a click on it. Its size
//! and which outputs show are its look.rhai: a new node is just the two boxes, outputs hidden. Web pages are
//! resolved with yt-dlp. Compiled into the `corroborate` program.

use super::*;
use corroborate::modules::sourcer::has_video;
use corroborate::timebase::FileFacts;
use std::io::Read;
use std::path::Path;

/// Direct media URLs ffmpeg opens itself; every other http(s) link is a page for yt-dlp.
const DIRECT: [&str; 12] = [".mp3", ".wav", ".flac", ".ogg", ".opus", ".m4a", ".aac", ".mp4", ".webm", ".mkv", ".m3u8", ".ts"];
/// A microphone is written as `mic:<device name>`.
const MIC: &str = "mic:";
/// Samples (16 kHz) per waveform column: 0.1 s.
const PEAK: usize = 1600;
/// Video frames are kept small and at most this many; past it every other one is dropped.
const FW: usize = 192;
const FH: usize = 108;
const MAX_FRAMES: usize = 600;

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

#[derive(Default)]
struct FactsInner {
    read: HashMap<String, FileFacts>,
    queue: std::collections::VecDeque<String>,
    busy: bool,
}

/// What each file says about itself, by path, read one file at a time on a thread of their own,
/// once each. Kept in the app, so a Run does not clear it.
#[derive(Clone, Default)]
pub struct Facts(Arc<std::sync::Mutex<FactsInner>>);

impl Facts {
    /// What is known of `path` so far: None while it has not been read.
    pub fn known(&self, path: &str) -> Option<FileFacts> {
        self.0.lock().unwrap().read.get(path).cloned()
    }

    /// Has `path` read, if it has not been yet.
    fn ask(&self, path: &str, ctx: &egui::Context) {
        let mut s = self.0.lock().unwrap();
        if s.read.contains_key(path) || s.queue.iter().any(|p| p == path) {
            return;
        }
        s.queue.push_back(path.to_string());
        if !s.busy {
            s.busy = true;
            let (me, ctx) = (self.0.clone(), ctx.clone());
            std::thread::spawn(move || loop {
                let Some(p) = me.lock().unwrap().queue.pop_front() else {
                    me.lock().unwrap().busy = false;
                    return;
                };
                let r = corroborate::timebase::file_facts(&p);
                me.lock().unwrap().read.insert(p, r);
                ctx.request_repaint();
            });
        }
    }

    /// A file's facts set by hand (tests).
    #[cfg(test)]
    pub fn put(&self, path: &str, f: FileFacts) {
        self.0.lock().unwrap().read.insert(path.to_string(), f);
    }
}

impl App {
    /// The file source `i` opens, when it is a file.
    pub(crate) fn source_file(&self, i: NodeId) -> Option<String> {
        let Kind::Sourcer { input: text } = &self.nodes[i].kind else { return None };
        match input(text) {
            Some(Input::File(p)) => Some(p),
            _ => None,
        }
    }

    /// Every file source's time and length are read as soon as it is chosen, before any Run.
    pub(crate) fn ask_file_facts(&self, ctx: &egui::Context) {
        for i in (0..self.nodes.len()).filter(|&i| !self.nodes[i].hidden) {
            if let Some(p) = self.source_file(i) {
                self.facts.ask(&p, ctx);
            }
        }
    }

    /// What file source `i` says about itself, once read.
    pub(crate) fn facts_of(&self, i: NodeId) -> Option<FileFacts> {
        self.facts.known(&self.source_file(i)?)
    }

    /// The media's length: a file's, or what was decoded once a source that is not live has ended.
    pub(crate) fn source_length(&self, i: NodeId) -> Option<f64> {
        if let Some(f) = self.facts_of(i) {
            if let Some(l) = f.len.filter(|l| *l > 0.0) {
                return Some(l);
            }
        }
        let s = self.nodes[i].state.lock().unwrap();
        (s.done && !s.live && s.audio_s > 0.0).then_some(s.audio_s)
    }

    /// SOURCER's time output for a file: what the file says, raw, with no error of its own.
    pub(crate) fn file_time(&self, i: NodeId, recording: &str) -> Result<corroborate::data::Data, String> {
        let f = self.facts_of(i).ok_or("still reading the file")?;
        let utc = f.stamp.ok_or("no time in the file's metadata")?;
        Ok(corroborate::data::Data::Time { utc, error_s: 0.0, source: f.source, recording: recording.to_string() })
    }

    /// SOURCER's length output.
    pub(crate) fn length_out(&self, i: NodeId) -> Result<corroborate::data::Data, String> {
        let l = self.source_length(i).ok_or("length not known yet")?;
        Ok(corroborate::data::Data::Custom { kind: "length", json: format!("{{\"seconds\":{l}}}") })
    }
}

/// What kind of source the text is, for the node's label line.
pub(crate) fn kind_of(text: &str) -> &'static str {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        "file, YouTube link, stream URL or microphone"
    } else if t.starts_with(MIC) {
        "microphone (live)"
    } else if !t.contains("://") {
        if has_video(&t) { "video file (recording)" } else { "audio file (recording)" }
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
/// A video that is not live is downloaded into the folder `dir` (the project's recordings\),
/// telling `report` how far (0 to 1); `stop` ends the download.
pub fn resolve(url: &str, dir: &Path, report: &dyn Fn(f32), stop: &AtomicBool) -> Result<(String, bool), String> {
    if !kind_of(url).contains("yt-dlp") {
        let path = url.to_lowercase();
        let file = DIRECT.iter().filter(|e| **e != ".m3u8").any(|e| path.split('?').next().unwrap_or("").ends_with(e));
        return Ok((url.to_string(), !file));
    }
    // Downloaded before: read the copy, no yt-dlp call (no network, no sign-in check).
    if let Some(p) = downloaded(url, dir) {
        return Ok((p, false));
    }
    let out = run_ytdlp(&["--no-warnings", "--no-playlist", "-f", "ba/b", "--print", "is_live", "--print", "urls", url], &|_| {}, stop)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let is_live = lines.next().map(str::trim) == Some("True");
    match lines.next() {
        Some(u) if out.status.success() && u.contains("://") && is_live => Ok((u.trim().to_string(), true)),
        // Not live: YouTube refuses its direct links to ffmpeg (403), so yt-dlp downloads the
        // video into the project's recordings\ and it is read as a file. The copy stays there as
        // evidence.
        Some(_) if out.status.success() => download(url, dir, report, stop).map(|p| (p, false)),
        _ => Err(ytdlp_error("yt-dlp", &out.stderr, "no URL returned")),
    }
}

/// Extra yt-dlp options the user put in `settings\yt-dlp.txt` (one per line, `#` comments),
/// e.g. `--cookies-from-browser firefox` or `--cookies C:\path\cookies.txt` when YouTube asks
/// to sign in. Read at every call, so a change needs no restart.
fn ytdlp_extra() -> Vec<String> {
    std::fs::read_to_string(corroborate::home().join("settings").join("yt-dlp.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .flat_map(|l| match l.split_once(' ') {
            Some((k, v)) => vec![k.to_string(), v.trim().trim_matches('"').to_string()],
            None => vec![l.to_string()],
        })
        .collect()
}

/// The browser whose cookies last got past YouTube's sign-in check, reused for later calls.
static COOKIE_BROWSER: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

/// Run yt-dlp (no window) with the user's extra options. When YouTube asks to sign in ("not a
/// bot") and no options are set, try again with the cookies of the signed-in browsers (Firefox,
/// Edge, Chrome, Brave: yt-dlp reads them locally) and keep the first that works.
fn run_ytdlp(args: &[&str], report: &dyn Fn(f32), stop: &AtomicBool) -> Result<std::process::Output, String> {
    let run = |extra: &[String]| ytdlp_call(extra, args, report, stop);
    let extra = ytdlp_extra();
    let known = *COOKIE_BROWSER.lock().unwrap();
    let first = match (extra.is_empty(), known) {
        (true, Some(b)) => run(&["--cookies-from-browser".into(), b.into()])?,
        _ => run(&extra)?,
    };
    let signin = |o: &std::process::Output| !o.status.success() && String::from_utf8_lossy(&o.stderr).contains("Sign in to confirm");
    if !extra.is_empty() || !signin(&first) {
        return Ok(first);
    }
    for b in ["firefox", "edge", "chrome", "brave"] {
        let o = run(&["--cookies-from-browser".into(), b.into()])?;
        if o.status.success() {
            *COOKIE_BROWSER.lock().unwrap() = Some(b);
            return Ok(o);
        }
    }
    Ok(first)
}

/// What yt-dlp's progress lines start with (`PROGRESS_TEMPLATE`).
const PROGRESS_MARK: &str = "corroborate-progress";

/// Its progress as a line of bytes downloaded, the total, and the total's estimate (raw numbers,
/// no colours; "NA" when not known).
const PROGRESS_TEMPLATE: &str = "download:corroborate-progress %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s";

/// How far a download is (0 to 1) from one of yt-dlp's progress lines; None for other lines.
pub(crate) fn progress_of(line: &str) -> Option<f32> {
    let rest = line.split_once(PROGRESS_MARK)?.1;
    let mut n = rest.split_whitespace().map(|w| w.parse::<f64>().ok());
    let got = n.next()??;
    let total = n.next().flatten().or_else(|| n.next().flatten()).filter(|t| *t > 0.0)?;
    Some((got / total).clamp(0.0, 1.0) as f32)
}

/// One yt-dlp call (no window), its progress lines read as they come (on either stream) and told
/// to `report`; `stop` ends it.
fn ytdlp_call(extra: &[String], args: &[&str], report: &dyn Fn(f32), stop: &AtomicBool) -> Result<std::process::Output, String> {
    use std::io::BufRead;
    let mut child = corroborate::no_window(&mut std::process::Command::new("yt-dlp"))
        .args(extra)
        .args(["--progress", "--newline", "--progress-template", PROGRESS_TEMPLATE])
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run yt-dlp (is it installed?): {e}"))?;
    // Each stream on a thread of its own: (from stderr, line).
    let (tx, rx) = std::sync::mpsc::channel::<(bool, Vec<u8>)>();
    let out = child.stdout.take().ok_or("no yt-dlp output")?;
    let err = child.stderr.take().ok_or("no yt-dlp errors stream")?;
    let pipe = |r: Box<dyn std::io::Read + Send>, is_err: bool| {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(r).split(b'\n') {
                let Ok(line) = line else { break };
                if tx.send((is_err, line)).is_err() {
                    break;
                }
            }
        })
    };
    let threads = [pipe(Box::new(out), false), pipe(Box::new(err), true)];
    drop(tx);
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
        }
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok((is_err, line)) => match progress_of(&String::from_utf8_lossy(&line)) {
                Some(f) => report(f),
                None => {
                    let to = if is_err { &mut stderr } else { &mut stdout };
                    to.extend_from_slice(&line);
                    to.push(b'\n');
                }
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    for t in threads {
        let _ = t.join();
    }
    let status = child.wait().map_err(|e| format!("yt-dlp: {e}"))?;
    if stop.load(Ordering::Relaxed) {
        return Err("stopped".into());
    }
    Ok(std::process::Output { status, stdout, stderr })
}

/// yt-dlp's last error line; when YouTube asks to sign in, how to give yt-dlp the cookies.
fn ytdlp_error(what: &str, stderr: &[u8], none: &str) -> String {
    let e = String::from_utf8_lossy(stderr);
    let last = e.lines().last().unwrap_or(none).to_string();
    if last.contains("Sign in") || last.contains("cookies") {
        format!(
            "{what}: {last} — no signed-in browser's cookies worked (Firefox, Edge, Chrome, Brave; Chrome and Edge must be closed); sign in to YouTube in Firefox, or put `--cookies <cookies.txt>` in {}",
            corroborate::home().join("settings").join("yt-dlp.txt").display()
        )
    } else {
        format!("{what}: {last}")
    }
}

/// Download a video (one file with picture and sound, at most 480 lines; sound only when there is
/// none) with yt-dlp to `<dir>\<site> <id>.<ext>`.
fn download(url: &str, dir: &Path, report: &dyn Fn(f32), stop: &AtomicBool) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    let template = dir.join("%(extractor)s %(id)s.%(ext)s").to_string_lossy().into_owned();
    let out = run_ytdlp(&["--no-warnings", "--no-playlist", "-f", "b[height<=480]/ba/b", "--print", "after_move:filepath", "-o", &template, url], report, stop)?;
    let text = String::from_utf8_lossy(&out.stdout);
    match text.lines().rev().find(|l| !l.trim().is_empty()) {
        Some(p) if out.status.success() && std::path::Path::new(p.trim()).exists() => {
            let p = p.trim().to_string();
            remember(url, &p, dir);
            Ok(p)
        }
        _ => Err(ytdlp_error("yt-dlp download", &out.stderr, "no file written")),
    }
}

/// `<dir>\downloads.txt`: one `url<TAB>file` line per download (the file's name when it is in
/// `dir`, so the list holds when the project folder moves), so the same link is read from its copy
/// without asking yt-dlp again.
fn downloads_list(dir: &Path) -> std::path::PathBuf {
    dir.join("downloads.txt")
}

/// The URL as a key: trimmed, and for YouTube just the video id (watch?v=, youtu.be/, shorts/,
/// live/, with or without extra parameters).
fn url_key(url: &str) -> String {
    let u = url.trim();
    let id = |s: &str| s.split(['&', '?', '#', '/']).next().unwrap_or("").to_string();
    if u.contains("youtube.com/") || u.contains("youtu.be/") {
        for mark in ["v=", "youtu.be/", "shorts/", "live/", "embed/"] {
            if let Some((_, rest)) = u.split_once(mark) {
                let v = id(rest);
                if !v.is_empty() {
                    return format!("youtube {v}");
                }
            }
        }
    }
    u.to_string()
}

/// The copy of this URL listed in `dir`'s downloads.txt, if it is still there.
pub(crate) fn listed(url: &str, dir: &Path) -> Option<String> {
    let key = url_key(url);
    let text = std::fs::read_to_string(downloads_list(dir)).ok()?;
    text.lines().rev().filter_map(|l| l.split_once('\t')).map(|(k, p)| (k, dir.join(p))).find(|(k, p)| url_key(k) == key && p.is_file()).map(|(_, p)| p.to_string_lossy().into_owned())
}

/// The file an earlier download of this URL left in `dir`. One left in recordings\ next to the
/// program (before projects kept their downloads, or by a project not saved yet) is copied into
/// `dir` first, so the project holds its own copy.
pub(crate) fn downloaded(url: &str, dir: &Path) -> Option<String> {
    if let Some(p) = listed(url, dir) {
        return Some(p);
    }
    let shared = corroborate::home().join("recordings");
    if crate::recordings::key(&shared) == crate::recordings::key(dir) {
        return None;
    }
    let old = listed(url, &shared)?;
    let name = Path::new(&old).file_name()?;
    std::fs::create_dir_all(dir).ok()?;
    let to = dir.join(name);
    if !to.is_file() {
        // Through a temporary name, so a copy cut short is never taken for the file.
        let part = dir.join(format!("{}.part", name.to_string_lossy()));
        std::fs::copy(&old, &part).ok()?;
        std::fs::rename(&part, &to).ok()?;
    }
    let p = to.to_string_lossy().into_owned();
    remember(url, &p, dir);
    Some(p)
}

fn remember(url: &str, path: &str, dir: &Path) {
    use std::io::Write;
    let p = Path::new(path);
    let shown = match (p.parent(), p.file_name()) {
        (Some(d), Some(f)) if crate::recordings::key(d) == crate::recordings::key(dir) => f.to_string_lossy().into_owned(),
        _ => path.to_string(),
    };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(downloads_list(dir)) {
        let _ = writeln!(f, "{}\t{shown}", url.trim());
    }
}

#[cfg(test)]
mod download_tests {
    use super::url_key;

    #[test]
    fn youtube_links_share_a_key() {
        let k = url_key("https://www.youtube.com/watch?v=qJ4EtDtDAr0");
        assert_eq!(k, "youtube qJ4EtDtDAr0");
        assert_eq!(url_key("https://youtu.be/qJ4EtDtDAr0?t=30"), k);
        assert_eq!(url_key("https://www.youtube.com/watch?feature=share&v=qJ4EtDtDAr0&t=5s"), k);
        assert_eq!(url_key("https://www.youtube.com/live/qJ4EtDtDAr0"), k);
        assert_eq!(url_key(" https://example.com/a "), "https://example.com/a");
    }
}

// ---------------------------------------------------------------- what a run gives

/// One small video frame (RGB) at `t` seconds of the source.
pub struct Frame {
    pub t: f64,
    pub rgb: Vec<u8>,
}

/// What the SOURCER keeps from a run for its own view and its metadata output.
#[derive(Default)]
pub struct SourceState {
    /// Lowest and highest sample of every 0.1 s.
    pub peaks: Vec<[f32; 2]>,
    acc: (f32, f32, usize),
    /// Video frames, oldest first, at least `frame_step` seconds apart.
    pub frames: Vec<Frame>,
    pub frame_step: f64,
    /// About the pictures: "no video stream", an ffmpeg error, or empty.
    pub video_note: String,
    /// The pictures are read only when asked (a big project would otherwise decode every video
    /// at once): what ffmpeg opens with the run's Stop, and what was asked so far.
    pictures: Option<(String, Arc<AtomicBool>)>,
    pub frames_asked: Asked,
    /// ffprobe's view of the source, plus what was opened: key, value.
    pub meta: Vec<(String, String)>,
    /// The audio itself (16 kHz), for sound at the playhead.
    pub audio: std::sync::Arc<std::sync::Mutex<Vec<i16>>>,
}

/// Which pictures of a source have been asked for: none yet, one still frame (the node is
/// shown expanded), or one a second all through (a click on the video).
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum Asked {
    #[default]
    Nothing,
    Still,
    All,
}

impl SourceState {
    pub fn push_audio(&mut self, x: &[f32]) {
        self.audio.lock().unwrap().extend(x.iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16));
        for &v in x {
            self.peak(v);
        }
    }

    /// Sound as it is kept (16 bit): the same as `push_audio` without converting it twice.
    pub fn push_i16(&mut self, x: &[i16]) {
        self.audio.lock().unwrap().extend_from_slice(x);
        for &v in x {
            self.peak(v as f32 / 32767.0);
        }
    }

    fn peak(&mut self, v: f32) {
        let (lo, hi, n) = &mut self.acc;
        if *n == 0 {
            (*lo, *hi) = (v, v);
        } else {
            *lo = lo.min(v);
            *hi = hi.max(v);
        }
        *n += 1;
        if *n == PEAK {
            self.peaks.push([*lo, *hi]);
            *n = 0;
        }
    }

    /// Whether ffprobe found pictures (they are read when asked).
    pub fn has_video(&self) -> bool {
        self.pictures.is_some()
    }

    /// What `start_side` finds out, from a kept result instead of ffprobe.
    pub fn restore_side(&mut self, opened: String, meta: Vec<(String, String)>, video: bool, stop: Arc<AtomicBool>) {
        self.meta = meta;
        if video {
            self.pictures = Some((opened, stop));
        } else {
            self.video_note = "no video stream".into();
        }
    }

    fn push_frame(&mut self, t: f64, rgb: Vec<u8>) {
        if self.frames.last().is_some_and(|f| t < f.t + self.frame_step) {
            return;
        }
        self.frames.push(Frame { t, rgb });
        if self.frames.len() > MAX_FRAMES {
            let mut k = 0;
            self.frames.retain(|_| {
                k += 1;
                k % 2 == 1
            });
            self.frame_step = self.frame_step.max(1.0) * 2.0;
        }
    }

    /// The frame shown at `t`: the last one at or before it.
    fn frame_at(&self, t: f64) -> Option<(usize, &Frame)> {
        let i = self.frames.partition_point(|f| f.t <= t);
        (i > 0).then(|| (i - 1, &self.frames[i - 1]))
    }
}

/// Metadata of a source, beside the audio: ffprobe once. Its pictures come later, when the node
/// asks (`ask_frames`): one still frame when it is shown expanded, all of them on a click.
pub fn start_side(ff_input: String, typed: String, live: bool, st: Shared, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut meta = vec![("input".to_string(), typed), ("opened".to_string(), ff_input.clone()), ("live".to_string(), live.to_string())];
        let probe = corroborate::no_window(&mut std::process::Command::new("ffprobe"))
            .args(["-v", "error", "-show_format", "-show_streams", "-of", "flat", &ff_input])
            .output();
        let mut video = false;
        match probe {
            Ok(o) if o.status.success() => {
                for line in String::from_utf8_lossy(&o.stdout).lines() {
                    let Some((k, v)) = line.split_once('=') else { continue };
                    let v = v.trim_matches('"').to_string();
                    let keep = k.starts_with("format.tags.")
                        || ["format.format_long_name", "format.duration", "format.bit_rate"].contains(&k)
                        || ["codec_type", "codec_name", "width", "height", "sample_rate", "channels", "r_frame_rate"]
                            .iter()
                            .any(|f| k.starts_with("streams.stream.") && k.ends_with(&format!(".{f}")));
                    video |= k.ends_with(".codec_type") && v == "video";
                    if keep {
                        meta.push((k.trim_start_matches("streams.").to_string(), v));
                    }
                }
            }
            Ok(o) => meta.push(("ffprobe".into(), String::from_utf8_lossy(&o.stderr).trim().to_string())),
            Err(e) => meta.push(("ffprobe".into(), format!("could not run (is it on PATH?): {e}"))),
        }
        {
            let mut s = st.lock().unwrap();
            s.source.meta = meta;
            if video {
                s.source.pictures = Some((ff_input, stop));
            } else {
                s.source.video_note = "no video stream".into();
            }
        }
    });
}

/// Asks for a source's pictures (`Asked::Still` or `Asked::All`) once its metadata says it has
/// some; asking again for the same, or for less, does nothing. All of them replace the still.
pub fn ask_frames(st: &Shared, want: Asked) {
    let (input, stop) = {
        let mut s = st.lock().unwrap();
        let had = s.source.frames_asked;
        let Some((input, stop)) = s.source.pictures.clone() else { return };
        if had == Asked::All || had == want || want == Asked::Nothing {
            return;
        }
        s.source.frames_asked = want;
        (input, stop)
    };
    let st = st.clone();
    std::thread::spawn(move || {
        let r = match want {
            Asked::Still => still(&input, &st),
            _ => frames(&input, &st, &stop),
        };
        if let Err(e) = r {
            st.lock().unwrap().source.video_note = e;
        }
    });
}

/// The letterboxing filter: FW × FH whatever the video's shape.
fn fit() -> String {
    format!("scale={FW}:{FH}:force_original_aspect_ratio=decrease,pad={FW}:{FH}:(ow-iw)/2:(oh-ih)/2")
}

/// One frame a second in (the very first is often black), else the first.
fn still(input: &str, st: &Shared) -> Result<(), String> {
    for at in ["1", "0"] {
        let o = corroborate::no_window(&mut std::process::Command::new("ffmpeg"))
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", at, "-i", input, "-an", "-frames:v", "1", "-vf", &fit(), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .map_err(|e| format!("could not start ffmpeg for video: {e}"))?;
        if o.stdout.len() >= FW * FH * 3 {
            let mut s = st.lock().unwrap();
            // A click may have started the whole video meanwhile.
            if s.source.frames.is_empty() {
                s.source.push_frame(at.parse().unwrap_or(0.0), o.stdout[..FW * FH * 3].to_vec());
            }
            return Ok(());
        }
    }
    Err("ffmpeg gave no video frame".into())
}

/// Read one frame a second, FW × FH RGB (letterboxed), into the state until the end or Stop.
fn frames(input: &str, st: &Shared, stop: &AtomicBool) -> Result<(), String> {
    let vf = format!("fps=1,{}", fit());
    let mut child = corroborate::no_window(&mut std::process::Command::new("ffmpeg"))
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i", input, "-an", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start ffmpeg for video: {e}"))?;
    let mut out = child.stdout.take().ok_or("no ffmpeg output")?;
    let mut n = 0u64;
    loop {
        let mut buf = vec![0u8; FW * FH * 3];
        if out.read_exact(&mut buf).is_err() || stop.load(Ordering::Relaxed) {
            break;
        }
        let mut s = st.lock().unwrap();
        if n == 0 {
            // The still goes: these come in order from the start.
            s.source.frames.clear();
            s.source.frame_step = 0.0;
        }
        s.source.push_frame(n as f64, buf);
        drop(s);
        n += 1;
    }
    let _ = child.kill();
    let _ = child.wait();
    if n == 0 && !stop.load(Ordering::Relaxed) {
        return Err("ffmpeg gave no video frames".into());
    }
    Ok(())
}

/// Where the source was recorded: the first metadata entry naming a location that reads as
/// ISO 6709 (`location`, `location-eng`, `com.apple.quicktime.location.ISO6709`, on the file or a
/// stream), with its key.
pub fn site(s: &SourceState) -> Option<(corroborate::data::Site, String)> {
    s.meta.iter().filter(|(k, _)| k.to_lowercase().contains("location")).find_map(|(k, v)| corroborate::data::Site::from_iso6709(v).map(|p| (p, k.clone())))
}

/// The location output: the recording's point (the 1° cell it is in as the area).
pub fn location(s: &SourceState) -> Result<corroborate::data::Data, String> {
    let (p, key) = site(s).ok_or("no coordinates in the metadata")?;
    let area = Area { cells: corroborate::area::WorldMask::circle(p.lat, p.lon, 0.0), add: true };
    Ok(corroborate::data::Data::Location { area: Arc::new(area), note: format!("from the file's metadata ({key})"), site: Some(p) })
}

/// The metadata output in the result JSON.
pub fn json(s: &NodeState) -> String {
    let m: Vec<String> = s.source.meta.iter().map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v))).collect();
    format!(",\"audio_s\":{:.1},\"live\":{},\"metadata\":{{{}}}", s.audio_s, s.live, m.join(","))
}

impl App {
    /// The text a node gives to a text input. No node has a text output yet: a wired text input
    /// says so on the SOURCER.
    fn text_out(&self, _from: NodeId) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------- on the canvas

const WAVE_H: f32 = 64.0;
const VIDEO_H: f32 = 150.0;
const WAVE: Color32 = Color32::from_rgb(224, 164, 88);

/// The SOURCER on the canvas: the path or URL with Browse, the system streams box, its kind,
/// status and clock, the waveform, and for sources with pictures the video viewer (hover the
/// waveform to scrub it).
pub struct Look;

impl crate::look::NodeLook for Look {
    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = crate::look::content(app, ui, i);
        let z = app.zoom;
        let st = app.nodes[i].state.clone();
        let Kind::Sourcer { input } = &app.nodes[i].kind else { return };
        let mut input = input.clone();
        // A wire into the text input replaces what is typed.
        let fed = app.graph.wires().iter().find(|w| w.to == i && w.to_port == 0).map(|w| w.from);
        let fed_text = fed.and_then(|f| app.text_out(f));
        if let Some(t) = &fed_text {
            input = t.clone();
        }
        let mut refresh = false;
        // The playhead on this source's clock (stronger when its group is the active one).
        let clock = app.clock_of(i).unwrap_or_default();
        let ph = app.playhead_on(&clock).map(|t| (t, app.is_active_clock(&clock)));
        let mut set_ph: Option<f64> = None;
        let mut play: Option<PlayClick> = None;
        crate::look::area(ui, r, |ui| {
            ui.add_enabled_ui(fed_text.is_none(), |ui| {
                // Row 1: file, YouTube link or stream URL. The text input overrides this box.
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut input).hint_text("file, YouTube link or stream URL").desired_width(r.width() - 80.0 * z));
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("audio / video", &["mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "mp4", "mkv", "webm", "mov", "avi"])
                            .add_filter("any", &["*"])
                            .pick_file()
                        {
                            input = p.to_string_lossy().into_owned();
                        }
                    }
                });
                // Row 2: system streams. Every audio device ffmpeg lists: microphones, digital and
                // virtual mics, capture cards. Picking one writes `mic:<device>` into the source.
                let shown = match input.trim().strip_prefix(MIC) {
                    Some(d) => d.trim().to_string(),
                    None => "system stream: microphone, capture card…".to_string(),
                };
                let combo = egui::ComboBox::from_id_salt(format!("sys_stream_{i}"))
                    .selected_text(shown)
                    .width(r.width() - 8.0 * z)
                    .show_ui(ui, |ui| {
                        if app.mics.is_empty() {
                            ui.label("no audio devices found");
                        }
                        for m in &app.mics {
                            let current = input.trim().strip_prefix(MIC).is_some_and(|d| d.trim() == m.as_str());
                            if ui.selectable_label(current, m.as_str()).clicked() {
                                input = format!("{MIC}{m}");
                            }
                        }
                    });
                refresh = combo.response.clicked();
            });
            match (fed, &fed_text) {
                (Some(f), None) => crate::look::line(ui, format!("text input: {} gives no text, the field is used", app.nodes[f].label), WARN),
                (Some(f), Some(_)) => crate::look::line(ui, format!("text from {}", app.nodes[f].label), DIM),
                _ => crate::look::line(ui, kind_of(&input), DIM),
            }
            source_info(ui, &st, app.source_file(i).is_some().then(|| app.facts_of(i)));
            if !input.trim().is_empty() {
                ui.add_space(4.0 * z);
                play = play_row(ui, &st, ph, app.ph.playing && app.is_active_clock(&clock));
                let (cursor, set) = waveform(ui, &st, z, i, ph);
                set_ph = set;
                if has_video(&input) {
                    ui.add_space(4.0 * z);
                    // The frame under the pointer, else at the playhead.
                    video(app, ui, i, &st, cursor.or(set).or(ph.map(|p| p.0)), z);
                }
            }
        });
        if let Some(t) = set_ph {
            app.set_playhead(&clock, t);
        }
        match play {
            Some(PlayClick::Play) => app.play_source(i),
            Some(PlayClick::Live) => {
                app.pick_only(i);
                app.go_live();
                app.ph.playing = true;
            }
            None => {}
        }
        if refresh {
            match corroborate::ingest::list_microphones() {
                Ok(m) => app.mics = m,
                Err(e) => app.msg = e,
            }
        }
        if let Kind::Sourcer { input: d } = &mut app.nodes[i].kind {
            *d = input;
        }
    }
}

enum PlayClick {
    Play,
    Live,
}

/// Play / Pause for this source's playhead, where it is, and for a live source how far behind
/// now it is, with Live to jump back. A recording is a simulation: played in real time from the
/// playhead, paused anywhere. A live source keeps receiving audio while paused.
fn play_row(ui: &mut egui::Ui, st: &Shared, ph: Option<(f64, bool)>, playing: bool) -> Option<PlayClick> {
    let (live, a) = st.lock().map(|s| (s.live, s.audio_s)).unwrap_or((false, 0.0));
    let mut click = None;
    ui.horizontal(|ui| {
        let tip = if live { "play or pause this live source (Space); the audio keeps coming in while paused" } else { "play or pause this recording in real time, a simulation (Space)" };
        if ui.small_button(if playing { "Pause" } else { "Play" }).on_hover_text(tip).clicked() {
            click = Some(PlayClick::Play);
        }
        let at = ph.map(|p| p.0);
        if live {
            let behind = at.map_or(0.0, |t| (a - t).max(0.0));
            if behind < 0.5 {
                ui.colored_label(Color32::from_rgb(235, 80, 80), "LIVE");
            } else {
                ui.colored_label(WARN, format!("{} behind live", fmt_secs(behind)));
                if ui.small_button("Live").on_hover_text("jump back to now and follow it").clicked() {
                    click = Some(PlayClick::Live);
                }
            }
        } else {
            ui.colored_label(DIM, "simulation");
        }
        if let Some(t) = at {
            ui.colored_label(DIM, format!("{} / {}", fmt_secs(t), fmt_secs(a)));
        }
    });
    click
}

/// The audio so far, min/max per column: the whole recording, or the last 30 s when live.
/// Draws the playhead `ph` (time, active); a click or drag moves it. Returns the time under the
/// pointer and where the playhead was moved to.
fn waveform(ui: &mut egui::Ui, st: &Shared, z: f32, i: NodeId, ph: Option<(f64, bool)>) -> (Option<f64>, Option<f64>) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, WAVE_H * z), Sense::click_and_drag());
    ui.data_mut(|d| d.insert_temp(egui::Id::new(("wave-rect", i)), rect));
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0 * z, BG);
    let s = st.lock().unwrap();
    let peaks = &s.source.peaks;
    if peaks.is_empty() {
        p.text(rect.center(), Align2::CENTER_CENTER, "waveform appears at Run", FontId::proportional(10.5 * z), DIM);
        return (None, None);
    }
    let dt = PEAK as f64 / corroborate::INPUT_SR;
    let first = if s.live { peaks.len().saturating_sub((30.0 / dt) as usize) } else { 0 };
    let shown = &peaks[first..];
    let cols = rect.width().max(1.0) as usize;
    let per = (shown.len() as f32 / cols as f32).max(1.0);
    let gain = shown.iter().map(|q| q[0].abs().max(q[1].abs())).fold(1e-4f32, f32::max);
    let mid = rect.center().y;
    let half = rect.height() * 0.46;
    let mut mesh = egui::epaint::Mesh::default();
    let ncol = ((shown.len() as f32 / per).ceil() as usize).min(cols);
    let colw = rect.width() / (shown.len() as f32 / per).max(1.0);
    for c in 0..ncol {
        let a = (c as f32 * per) as usize;
        let b = (((c + 1) as f32 * per) as usize).clamp(a + 1, shown.len());
        let (lo, hi) = shown[a..b].iter().fold((f32::MAX, f32::MIN), |(l, h), q| (l.min(q[0]), h.max(q[1])));
        let x = rect.left() + c as f32 * colw;
        let r = Rect::from_min_max(Pos2::new(x, mid - hi / gain * half), Pos2::new(x + colw.max(1.0), mid - lo / gain * half + 1.0));
        mesh.add_colored_rect(r, WAVE);
    }
    p.add(egui::Shape::mesh(mesh));
    let t0 = first as f64 * dt;
    let t1 = peaks.len() as f64 * dt;
    let span_text = if s.live { format!("last {:.0} s, live", t1 - t0) } else { format!("0 – {}", fmt_secs(t1)) };
    p.text(rect.left_top() + Vec2::new(4.0, 2.0) * z, Align2::LEFT_TOP, span_text, FontId::monospace(9.5 * z), DIM);
    let len = shown.len().max(1) as f64 * dt;
    let t_at = |x: f32| t0 + ((x - rect.left()) / rect.width()) as f64 * len;
    let set = if resp.clicked() || resp.dragged() { resp.interact_pointer_pos().map(|h| t_at(h.x.clamp(rect.left(), rect.right()))) } else { None };
    if let Some((t, active)) = set.map(|t| (t, true)).or(ph) {
        let x = rect.left() + ((t - t0) / len) as f32 * rect.width();
        if (rect.left()..=rect.right()).contains(&x) {
            let c = PLAYHEAD;
            p.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(if active { 2.0 } else { 1.0 }, if active { c } else { c.gamma_multiply(0.5) }));
        }
    }
    let Some(hover) = resp.hover_pos().filter(|h| rect.contains(*h)) else { return (None, set) };
    let t = t_at(hover.x);
    p.line_segment([Pos2::new(hover.x, rect.top()), Pos2::new(hover.x, rect.bottom())], Stroke::new(1.0, TEXT));
    p.text(Pos2::new(hover.x + 4.0 * z, rect.bottom() - 2.0 * z), Align2::LEFT_BOTTOM, fmt_secs(t), FontId::monospace(9.5 * z), TEXT);
    (Some(t), set)
}

/// The video frame at the waveform's pointer, or the newest one.
/// Only a still frame is read at first, once the viewer is on screen; a click reads the video
/// (one frame a second) so the waveform and the playhead can scrub it.
fn video(app: &mut App, ui: &mut egui::Ui, i: NodeId, st: &Shared, cursor: Option<f64>, z: f32) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, VIDEO_H * z), Sense::click());
    if ui.is_rect_visible(rect) {
        ask_frames(st, Asked::Still);
    }
    let asked = st.lock().unwrap().source.frames_asked;
    let resp = if asked == Asked::All { resp } else { resp.on_hover_text("click to play the video: read it one frame a second, to scrub with the waveform") };
    if resp.clicked() {
        ask_frames(st, Asked::All);
    }
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0 * z, Color32::BLACK);
    let s = st.lock().unwrap();
    let pick = match cursor {
        // Before the first frame (the still is a second in): the first.
        Some(t) => s.source.frame_at(t).or_else(|| s.source.frames.first().map(|f| (0, f))),
        None => s.source.frames.len().checked_sub(1).map(|k| (k, &s.source.frames[k])),
    };
    let Some((k, f)) = pick else {
        let note = match (s.source.video_note.as_str(), s.source.frames_asked) {
            ("", Asked::Nothing) => "video appears at Run",
            ("", _) => "reading a frame…",
            (n, _) => n,
        };
        p.text(rect.center(), Align2::CENTER_CENTER, note, FontId::proportional(10.5 * z), DIM);
        return;
    };
    // One texture per node, rebuilt when the shown frame changes.
    let stamp = (k as u64) << 32 | f.t as u64;
    let tex = match app.textures.get_mut(&i) {
        Some((old, _, tex)) if *old == stamp => tex.id(),
        _ => {
            let img = ColorImage::from_rgb([FW, FH], &f.rgb);
            let tex = ui.ctx().load_texture(format!("video{i}"), img, TextureOptions::LINEAR);
            let id = tex.id();
            app.textures.insert(i, (stamp, f.t, tex));
            id
        }
    };
    let h = rect.height();
    let fw = (h * FW as f32 / FH as f32).min(rect.width());
    let fr = Rect::from_center_size(rect.center(), Vec2::new(fw, fw * FH as f32 / FW as f32));
    p.image(tex, fr, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    p.text(rect.left_bottom() + Vec2::new(4.0, -2.0) * z, Align2::LEFT_BOTTOM, fmt_secs(f.t), FontId::monospace(9.5 * z), TEXT);
    if s.source.frames_asked == Asked::Still {
        p.text(rect.center(), Align2::CENTER_CENTER, "▶", FontId::proportional(28.0 * z), TEXT.gamma_multiply(0.8));
        p.text(rect.right_top() + Vec2::new(-4.0, 2.0) * z, Align2::RIGHT_TOP, "still: click to play", FontId::proportional(9.5 * z), TEXT);
    }
    if !s.source.video_note.is_empty() {
        p.text(rect.right_bottom() + Vec2::new(-4.0, -2.0) * z, Align2::RIGHT_BOTTOM, &s.source.video_note, FontId::proportional(9.5 * z), WARN);
    }
}

/// Seconds as H:MM:SS (or M:SS under an hour).
pub fn fmt_secs(t: f64) -> String {
    let t = t.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}
