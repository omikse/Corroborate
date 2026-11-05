//! meta-plugin: what the source says about itself, as plugin metadata.
//!
//! A separate program (protocol in `corroborate::plugin`). For a file it reads, with ffprobe:
//! the container, duration, size, the audio stream (codec, rate, channels), every tag (title,
//! artist, creation_time, …), coordinates from location tags (ISO 6709), other streams (cover
//! art, video) and the file system's created/modified times. For a live source it records the
//! address. Every value is labelled with where it came from; nothing is inferred.
//!
//! The audio is read to the end and ignored, so the viewer's analysis is never held up.

use corroborate::plugin;
use corroborate::timebase;
use std::io::{Read, Write};
use std::process::Command;

/// Coordinates from an ISO 6709 string such as "+40.7128-074.0060+010.000/".
fn iso6709(v: &str) -> Option<(f64, f64)> {
    let v = v.trim().trim_end_matches('/');
    // Split before each sign after the first character.
    let mut parts = Vec::new();
    let mut cur = String::new();
    for (i, ch) in v.chars().enumerate() {
        if i > 0 && (ch == '+' || ch == '-') {
            parts.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    parts.push(cur);
    let lat: f64 = parts.first()?.parse().ok()?;
    let lon: f64 = parts.get(1)?.parse().ok()?;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

/// `key="value"` or `key=value` lines from `ffprobe -of flat`.
fn flat_fields(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v);
            (k.to_string(), v.replace("\\\"", "\"").replace("\\\\", "\\"))
        })
        .filter(|(_, v)| !v.is_empty() && v != "N/A")
        .collect()
}

fn file_meta(path: &str, out: &mut impl Write) -> usize {
    let mut n = 0;
    let mut put = |out: &mut dyn Write, k: &str, v: &str| {
        let _ = writeln!(out, "{}", plugin::meta(k, v));
        n += 1;
    };
    if let Ok(md) = std::fs::metadata(path) {
        let unix = |t: std::time::SystemTime| t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs_f64());
        if let Some(t) = md.created().ok().and_then(unix) {
            put(out, "file created (file system, UTC)", &timebase::fmt_utc(t));
        }
        if let Some(t) = md.modified().ok().and_then(unix) {
            put(out, "file modified (file system, UTC)", &timebase::fmt_utc(t));
        }
    }
    let probed = corroborate::no_window(&mut Command::new("ffprobe"))
        .args(["-v", "error", "-show_format", "-show_streams", "-of", "flat", path])
        .output();
    let fields = match probed {
        Ok(o) if o.status.success() => flat_fields(&String::from_utf8_lossy(&o.stdout)),
        Ok(o) => {
            let _ = writeln!(out, "{}", plugin::status(&format!("ffprobe: {}", String::from_utf8_lossy(&o.stderr).trim())));
            return n;
        }
        Err(e) => {
            let _ = writeln!(out, "{}", plugin::status(&format!("could not run ffprobe (is it on PATH?): {e}")));
            return n;
        }
    };
    let get = |k: &str| fields.iter().find(|(f, _)| f == k).map(|(_, v)| v.as_str());
    for (k, label) in [
        ("format.format_long_name", "container"),
        ("format.duration", "duration (s)"),
        ("format.size", "size (bytes)"),
        ("format.bit_rate", "bit rate (bit/s)"),
    ] {
        if let Some(v) = get(k) {
            put(out, label, v);
        }
    }
    for (k, v) in &fields {
        if let Some(tag) = k.strip_prefix("format.tags.") {
            put(out, &format!("tag {tag}"), v);
        }
    }
    // Streams: the audio in detail, others in one line each (cover art, video).
    let mut i = 0;
    while let Some(kind) = get(&format!("streams.stream.{i}.codec_type")) {
        let sf = |f: &str| get(&format!("streams.stream.{i}.{f}")).unwrap_or("?").to_string();
        if kind == "audio" {
            put(out, &format!("stream {i} audio"), &format!("{}, {} Hz, {} channel(s) {}", sf("codec_long_name"), sf("sample_rate"), sf("channels"), get(&format!("streams.stream.{i}.channel_layout")).unwrap_or("")));
        } else {
            put(out, &format!("stream {i} {kind}"), &sf("codec_long_name"));
        }
        let prefix = format!("streams.stream.{i}.tags.");
        for (k, v) in &fields {
            if let Some(tag) = k.strip_prefix(&prefix) {
                put(out, &format!("stream {i} tag {tag}"), v);
            }
        }
        i += 1;
    }
    // Coordinates: any location tag in ISO 6709 form (phones and cameras write these).
    for (k, v) in &fields {
        if k.to_ascii_lowercase().contains("location") {
            if let Some((lat, lon)) = iso6709(v) {
                put(out, "coordinates", &format!("{lat:.6}, {lon:.6} (from {k})"));
            }
        }
    }
    n
}

fn main() {
    let (source, live) = plugin::source_args();
    // Take the audio to the end in the background, so the viewer is never held up.
    let drain = std::thread::spawn(|| {
        let mut buf = vec![0u8; 1 << 16];
        let mut stdin = std::io::stdin().lock();
        while matches!(stdin.read(&mut buf), Ok(n) if n > 0) {}
    });
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let _ = writeln!(out, "{}", plugin::hello("META", "what the source says about itself (ffprobe, file system)", [200, 160, 255]));
    match (&source, live) {
        (None, _) => {
            let _ = writeln!(out, "{}", plugin::status("no --source given: nothing to read"));
        }
        (Some(src), true) => {
            let _ = writeln!(out, "{}", plugin::meta("source", src));
            let _ = writeln!(out, "{}", plugin::meta("kind", "live"));
            let _ = writeln!(out, "{}", plugin::status("live source: only its address is known"));
        }
        (Some(src), false) => {
            let _ = writeln!(out, "{}", plugin::meta("source", src));
            let n = file_meta(src, &mut out);
            let _ = writeln!(out, "{}", plugin::status(&format!("{n} fields read from the file; all go into the result JSON")));
        }
    }
    let _ = out.flush();
    let _ = drain.join();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_iso6709() {
        assert_eq!(iso6709("+40.7128-074.0060+010.000/"), Some((40.7128, -74.006)));
        assert_eq!(iso6709("-33.8688+151.2093/"), Some((-33.8688, 151.2093)));
        assert_eq!(iso6709("nonsense"), None);
    }

    #[test]
    fn reads_flat_output() {
        let f = flat_fields("format.tags.title=\"raw \\\"take\\\"\"\nformat.nb_streams=2\nformat.x=\"N/A\"");
        assert_eq!(f, vec![("format.tags.title".into(), "raw \"take\"".into()), ("format.nb_streams".into(), "2".into())]);
    }
}
