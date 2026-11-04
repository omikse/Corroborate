//! Audio sources: WAV files (hound, resampled here), and anything ffmpeg can read
//! (files or stream URLs) decoded by an ffmpeg child process to raw f32 mono at the target rate.

use std::io::{self, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver};
use std::thread;

/// Streaming anti-alias lowpass + decimator to `sr_out`. Only the band below ~450 Hz matters,
/// so a short windowed-sinc FIR (passband to ~600 Hz, stop well before aliases reach it) is
/// enough. Each output takes the nearest input sample as its centre.
pub struct Resampler {
    ratio: f64,
    taps: Vec<f32>,
    half: usize,
    buf: Vec<f32>,
    base: u64,
    next: u64,
    pass: bool,
}

impl Resampler {
    pub fn new(sr_in: f64, sr_out: f64) -> Self {
        let pass = (sr_in - sr_out).abs() < 1e-6;
        let fc = 1300.0 / sr_in;
        let half = ((1.8 * sr_in / 1400.0).ceil() as usize).max(8);
        let mut taps: Vec<f32> = (0..=2 * half)
            .map(|i| {
                let n = i as f64 - half as f64;
                let sinc = if n == 0.0 {
                    2.0 * fc
                } else {
                    (2.0 * std::f64::consts::PI * fc * n).sin() / (std::f64::consts::PI * n)
                };
                let w = 0.54 + 0.46 * (std::f64::consts::PI * n / half as f64).cos();
                (sinc * w) as f32
            })
            .collect();
        let s: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= s);
        Self { ratio: sr_in / sr_out, taps, half, buf: Vec::new(), base: 0, next: 0, pass }
    }

    pub fn process(&mut self, x: &[f32], out: &mut Vec<f32>) {
        if self.pass {
            out.extend_from_slice(x);
            return;
        }
        self.buf.extend_from_slice(x);
        loop {
            let centre = (self.next as f64 * self.ratio).round() as u64;
            if centre < self.base + self.half as u64 {
                // Leading edge: not enough history, treat missing samples as zero.
                let mut acc = 0.0f32;
                for (j, &t) in self.taps.iter().enumerate() {
                    let idx = centre as i64 - self.half as i64 + j as i64 - self.base as i64;
                    if idx >= 0 && (idx as usize) < self.buf.len() {
                        acc += t * self.buf[idx as usize];
                    }
                }
                if centre + self.half as u64 >= self.base + self.buf.len() as u64 {
                    break;
                }
                out.push(acc);
                self.next += 1;
                continue;
            }
            if centre + self.half as u64 >= self.base + self.buf.len() as u64 {
                break;
            }
            let s = (centre - self.half as u64 - self.base) as usize;
            let acc: f32 = self.taps.iter().zip(&self.buf[s..]).map(|(a, b)| a * b).sum();
            out.push(acc);
            self.next += 1;
        }
        // Drop input no future output needs.
        let centre = (self.next as f64 * self.ratio).round() as u64;
        let keep_from = centre.saturating_sub(self.half as u64);
        if keep_from > self.base {
            let n = ((keep_from - self.base) as usize).min(self.buf.len());
            self.buf.drain(..n);
            self.base += n as u64;
        }
    }
}

/// Read a WAV file in chunks, mixed to mono and resampled to `sr_out`, calling `f` per chunk.
pub fn read_wav(path: &str, sr_out: f64, mut f: impl FnMut(&[f32])) -> Result<(), String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("{path}: {e}"))?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let mut rs = Resampler::new(spec.sample_rate as f64, sr_out);
    let mut mono = Vec::with_capacity(4096);
    let mut out = Vec::with_capacity(4096);
    let mut acc = 0.0f32;
    let mut k = 0usize;
    let mut push = |v: f32, mono: &mut Vec<f32>, out: &mut Vec<f32>, f: &mut dyn FnMut(&[f32])| {
        acc += v;
        k += 1;
        if k == ch {
            mono.push(acc / ch as f32);
            acc = 0.0;
            k = 0;
            if mono.len() == 4096 {
                out.clear();
                rs.process(mono, out);
                f(out);
                mono.clear();
            }
        }
    };
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in r.samples::<f32>() {
                push(s.map_err(|e| e.to_string())?, &mut mono, &mut out, &mut f);
            }
        }
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            for s in r.samples::<i32>() {
                push(s.map_err(|e| e.to_string())? as f32 * scale, &mut mono, &mut out, &mut f);
            }
        }
    }
    drop(push);
    if !mono.is_empty() {
        out.clear();
        rs.process(&mono, &mut out);
        f(&out);
    }
    Ok(())
}

/// Decode `input` (file path or URL) with ffmpeg to f32 mono at `sr` and deliver chunks of
/// about 0.25 s over a bounded channel from a reader thread.
pub fn spawn_ffmpeg(input: &str, sr: u32) -> Result<(Child, Receiver<Vec<f32>>), String> {
    spawn_ffmpeg_from(input, sr, None)
}

/// As `spawn_ffmpeg`, optionally seeking to `start_s` seconds first.
pub fn spawn_ffmpeg_from(input: &str, sr: u32, start_s: Option<f64>) -> Result<(Child, Receiver<Vec<f32>>), String> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    if let Some(s) = start_s {
        cmd.args(["-ss", &format!("{s:.3}")]);
    }
    let mut child = cmd
        .args(["-i", input])
        .args(["-vn", "-ac", "1", "-ar", &sr.to_string(), "-f", "f32le", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("could not start ffmpeg (is it on PATH?): {e}"))?;
    let stdout = child.stdout.take().ok_or("no ffmpeg stdout")?;
    let rx = spawn_reader(stdout, (sr / 4) as usize);
    Ok((child, rx))
}

/// Raw f32 little-endian mono samples from stdin.
pub fn spawn_stdin(chunk: usize) -> Receiver<Vec<f32>> {
    spawn_reader(io::stdin(), chunk)
}

fn spawn_reader<R: Read + Send + 'static>(mut src: R, chunk: usize) -> Receiver<Vec<f32>> {
    let (tx, rx) = sync_channel::<Vec<f32>>(64);
    thread::spawn(move || {
        let mut bytes = vec![0u8; chunk * 4];
        let mut fill = 0usize;
        loop {
            match src.read(&mut bytes[fill..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => fill += n,
            }
            if fill == bytes.len() {
                let v: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                fill = 0;
                if tx.send(v).is_err() {
                    return;
                }
            }
        }
        let v: Vec<f32> = bytes[..fill - fill % 4]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        if !v.is_empty() {
            let _ = tx.send(v);
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_keeps_low_tone_and_rate() {
        let sr_in = 48_000.0;
        let x: Vec<f32> = (0..48_000 * 2)
            .map(|i| (2.0 * std::f64::consts::PI * 120.0 * i as f64 / sr_in).sin() as f32)
            .collect();
        let mut rs = Resampler::new(sr_in, 4000.0);
        let mut out = Vec::new();
        for c in x.chunks(1234) {
            rs.process(c, &mut out);
        }
        assert!((out.len() as i64 - 8000).abs() < 40, "len {}", out.len());
        let rms = (out[1000..7000].iter().map(|v| v * v).sum::<f32>() / 6000.0).sqrt();
        assert!((rms - 0.7071).abs() < 0.02, "rms {rms}");
    }
}
