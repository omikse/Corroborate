use doppler_live::synth::{render, Pass};
use doppler_live::{ingest, Config, Detector, Event};
use std::fs::OpenOptions;
use std::io::Write;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const USAGE: &str = "\
doppler-live: detect aircraft Doppler curves in audio

USAGE:
  doppler-live file <input> [--csv points.csv] [options]
      Any WAV, or anything ffmpeg can decode (mp3, m4a, mp4, ...).
  doppler-live listen (--url <stream-or-file> | --stdin) [--events events.jsonl] [options]
      Live: --url is decoded by ffmpeg; --stdin reads raw f32le mono at 4000 Hz, e.g.
      ffmpeg -i <src> -ac 1 -ar 4000 -f f32le - | doppler-live listen --stdin
  doppler-live synth <out.wav>
      Write a 10-minute test recording with two passes, a 60 Hz hum and noise.

OPTIONS:
  --snr <z>      minimum mean ridge z (default 2.0)
  --rms <hz>     maximum fit misfit in Hz (default 1.0)
  --fmin <hz> --fmax <hz>   analysis band (default 40-450)
  --quiet        no progress on stderr

Events are printed as JSON lines on stdout.";

struct Args {
    cmd: String,
    pos: Option<String>,
    csv: Option<String>,
    url: Option<String>,
    stdin: bool,
    events: String,
    quiet: bool,
    cfg: Config,
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or(USAGE)?;
    let mut a = Args {
        cmd,
        pos: None,
        csv: None,
        url: None,
        stdin: false,
        events: "events.jsonl".into(),
        quiet: false,
        cfg: Config::default(),
    };
    let num = |v: Option<String>, name: &str| -> Result<f64, String> {
        v.ok_or(format!("{name} needs a value"))?
            .parse::<f64>()
            .map_err(|_| format!("{name}: not a number"))
    };
    while let Some(x) = it.next() {
        match x.as_str() {
            "--csv" => a.csv = Some(it.next().ok_or("--csv needs a path")?),
            "--url" => a.url = Some(it.next().ok_or("--url needs a value")?),
            "--events" => a.events = it.next().ok_or("--events needs a path")?,
            "--stdin" => a.stdin = true,
            "--quiet" => a.quiet = true,
            "--snr" => a.cfg.snr_min = num(it.next(), "--snr")?,
            "--rms" => a.cfg.rms_max = num(it.next(), "--rms")?,
            "--fmin" => a.cfg.fmin = num(it.next(), "--fmin")?,
            "--fmax" => a.cfg.fmax = num(it.next(), "--fmax")?,
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n\n{USAGE}")),
            s => a.pos = Some(s.to_string()),
        }
    }
    Ok(a)
}

fn main() {
    let a = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let r = match a.cmd.as_str() {
        "file" => run_file(&a),
        "listen" => run_listen(&a),
        "synth" => run_synth(&a),
        _ => Err(USAGE.into()),
    };
    if let Err(e) = r {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run_file(a: &Args) -> Result<(), String> {
    let input = a.pos.as_deref().ok_or("file: missing <input>")?;
    let mut det = Detector::new(a.cfg.clone());
    let mut events: Vec<Event> = Vec::new();
    let mut n = 0usize;
    let t0 = Instant::now();
    let mut on_chunk = |x: &[f32], det: &mut Detector, events: &mut Vec<Event>| {
        n += x.len();
        for e in det.push(x) {
            println!("{}", e.to_json(None));
            events.push(e);
        }
    };
    if input.to_ascii_lowercase().ends_with(".wav") {
        ingest::read_wav(input, a.cfg.sr, |x| on_chunk(x, &mut det, &mut events))?;
    } else {
        let (mut child, rx) = ingest::spawn_ffmpeg(input, a.cfg.sr as u32)?;
        for x in rx {
            on_chunk(&x, &mut det, &mut events);
        }
        let _ = child.wait();
    }
    for e in det.flush() {
        println!("{}", e.to_json(None));
        events.push(e);
    }
    let audio_s = n as f64 / a.cfg.sr;
    let wall = t0.elapsed().as_secs_f64();
    if !a.quiet {
        eprintln!(
            "{} event(s) in {:.1} min of audio; processed in {:.1} s ({:.0}x real time)",
            events.len(),
            audio_s / 60.0,
            wall,
            audio_s / wall.max(1e-9)
        );
    }
    if let Some(p) = &a.csv {
        let mut f = std::fs::File::create(p).map_err(|e| format!("{p}: {e}"))?;
        writeln!(f, "event_id,time_s,freq_hz").map_err(|e| e.to_string())?;
        for e in &events {
            for (t, fr) in &e.points {
                writeln!(f, "{},{:.3},{:.3}", e.id, t, fr).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn run_listen(a: &Args) -> Result<(), String> {
    let unix0 = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let (child, rx) = if let Some(u) = &a.url {
        let (c, rx) = ingest::spawn_ffmpeg(u, a.cfg.sr as u32)?;
        (Some(c), rx)
    } else if a.stdin {
        (None, ingest::spawn_stdin((a.cfg.sr / 4.0) as usize))
    } else {
        return Err("listen: give --url <source> or --stdin".into());
    };
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&a.events)
        .map_err(|e| format!("{}: {e}", a.events))?;
    let mut det = Detector::new(a.cfg.clone());
    let mut n = 0usize;
    let mut last = Instant::now();
    if !a.quiet {
        eprintln!("listening; warm-up {:.0} s, events also appended to {}", a.cfg.warmup_s, a.events);
    }
    for x in rx {
        n += x.len();
        for e in det.push(&x) {
            let j = e.to_json(Some(unix0));
            println!("{j}");
            let _ = writeln!(log, "{j}");
            let _ = log.flush();
        }
        if !a.quiet && last.elapsed().as_secs() >= 60 {
            last = Instant::now();
            eprintln!("… {:.1} min of audio processed", n as f64 / a.cfg.sr / 60.0);
        }
    }
    if let Some(mut c) = child {
        let _ = c.wait();
    }
    if !a.quiet {
        eprintln!("stream ended after {:.1} min", n as f64 / a.cfg.sr / 60.0);
    }
    Ok(())
}

fn run_synth(a: &Args) -> Result<(), String> {
    let out = a.pos.as_deref().ok_or("synth: missing <out.wav>")?;
    let sr = 4000.0;
    let passes = [
        Pass { f0: 105.0, v: 60.0, d: 900.0, tc: 200.0, amps: vec![0.12, 0.08, 0.04] },
        Pass { f0: 88.0, v: 75.0, d: 1600.0, tc: 430.0, amps: vec![0.08, 0.06] },
    ];
    let x = render(sr, 600.0, &passes, Some((60.0, 0.3)), 1.0, 7);
    let spec = hound::WavSpec { channels: 1, sample_rate: 4000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(out, spec).map_err(|e| e.to_string())?;
    for s in x {
        w.write_sample(s).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())?;
    eprintln!("wrote {out}: passes at t=200 s (f0 105 Hz) and t=430 s (f0 88 Hz)");
    Ok(())
}
