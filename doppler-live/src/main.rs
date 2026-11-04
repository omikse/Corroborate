use doppler_live::modules::{enf::Enf, planes::Planes};
use doppler_live::synth::{render, Pass};
use doppler_live::{ingest, timebase, Config, Finding, Module, INPUT_SR};
use std::fs::OpenOptions;
use std::io::Write;
use std::time::Instant;

const USAGE: &str = "\
doppler-live: detection modules for audio (aircraft Doppler curves, mains-hum ENF)

USAGE:
  doppler-live file <input> [--csv points.csv] [options]
      Any WAV, or anything ffmpeg can decode (mp3, m4a, mp4, mka, ...).
  doppler-live listen (--url <stream-or-file> | --stdin) [--events events.jsonl] [options]
      Live: --url is decoded by ffmpeg; --stdin reads raw f32le mono at 16000 Hz, e.g.
      ffmpeg -i <src> -ac 1 -ar 16000 -f f32le - | doppler-live listen --stdin
  doppler-live synth <out.wav>
      Write a 10-minute test recording with two passes, a 60 Hz hum and noise.

OPTIONS:
  --modules <list>   comma-separated: planes, enf (default: planes,enf)
  --snr <z>          planes: minimum mean ridge z (default 2.0)
  --rms <hz>         planes: maximum fit misfit in Hz (default 1.0)
  --fmin <hz> --fmax <hz>   planes: analysis band (default 40-450)
  --quiet            no progress on stderr

Findings are printed as JSON lines on stdout, each with a \"module\" field.
ENF = Electrical Network Frequency (the mains hum and its drift).";

struct Args {
    cmd: String,
    pos: Option<String>,
    csv: Option<String>,
    url: Option<String>,
    stdin: bool,
    events: String,
    quiet: bool,
    modules: String,
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
        modules: "planes,enf".into(),
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
            "--modules" => a.modules = it.next().ok_or("--modules needs a list")?,
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

/// The selected modules; planes uses the command-line detector settings.
fn modules(a: &Args) -> Result<Vec<Box<dyn Module>>, String> {
    a.modules
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|n| -> Result<Box<dyn Module>, String> {
            match n.trim() {
                "planes" => Ok(Box::new(Planes::new(a.cfg.clone()))),
                "enf" => Ok(Box::new(Enf::new())),
                other => Err(format!("unknown module '{other}' (available: planes, enf)")),
            }
        })
        .collect()
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
    let mut mods = modules(a)?;
    let mut found: Vec<Finding> = Vec::new();
    let mut n = 0usize;
    let t0 = Instant::now();
    let anchor = timebase::file_anchor(input);
    if !a.quiet {
        for m in &mods {
            eprintln!("{}", m.describe());
        }
        match &anchor {
            Some(an) => eprintln!("UTC anchor: {} ({})", timebase::fmt_utc(an.utc0), an.source),
            None => eprintln!("no UTC anchor: times are relative to the start of the file"),
        }
    }
    let mut on_chunk = |x: &[f32], mods: &mut Vec<Box<dyn Module>>, found: &mut Vec<Finding>| {
        n += x.len();
        for m in mods.iter_mut() {
            for f in m.push(x) {
                println!("{}", f.to_json(anchor.as_ref()));
                found.push(f);
            }
        }
    };
    if input.to_ascii_lowercase().ends_with(".wav") {
        ingest::read_wav(input, INPUT_SR, |x| on_chunk(x, &mut mods, &mut found))?;
    } else {
        let (mut child, rx) = ingest::spawn_ffmpeg(input, INPUT_SR as u32)?;
        for x in rx {
            on_chunk(&x, &mut mods, &mut found);
        }
        let _ = child.wait();
    }
    for m in mods.iter_mut() {
        for f in m.flush() {
            println!("{}", f.to_json(anchor.as_ref()));
            found.push(f);
        }
    }
    let audio_s = n as f64 / INPUT_SR;
    let wall = t0.elapsed().as_secs_f64();
    if !a.quiet {
        let count = |m: &str| found.iter().filter(|f| f.module() == m).count();
        eprintln!(
            "{} plane pass(es), {} ENF segment(s) in {:.1} min of audio; processed in {:.1} s ({:.0}x real time)",
            count("planes"),
            count("enf"),
            audio_s / 60.0,
            wall,
            audio_s / wall.max(1e-9)
        );
    }
    if let Some(p) = &a.csv {
        let mut f = std::fs::File::create(p).map_err(|e| format!("{p}: {e}"))?;
        writeln!(f, "event_id,time_s,freq_hz").map_err(|e| e.to_string())?;
        for fd in &found {
            if let Finding::Plane(e) = fd {
                for (t, fr) in &e.points {
                    writeln!(f, "{},{:.3},{:.3}", e.id, t, fr).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    Ok(())
}

fn run_listen(a: &Args) -> Result<(), String> {
    let mut mods = modules(a)?;
    let anchor: timebase::SharedAnchor = Default::default();
    let (child, rx) = if let Some(u) = &a.url {
        let (c, rx, first) = ingest::spawn_ffmpeg_logged(u, INPUT_SR as u32, None)?;
        timebase::upgrade_from_hls(u.clone(), first, anchor.clone(), 0.0);
        (Some(c), rx)
    } else if a.stdin {
        (None, ingest::spawn_stdin((INPUT_SR / 20.0) as usize))
    } else {
        return Err("listen: give --url <source> or --stdin".into());
    };
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&a.events)
        .map_err(|e| format!("{}: {e}", a.events))?;
    let mut n = 0usize;
    let mut last = Instant::now();
    let mut last_src: Option<String> = None;
    if !a.quiet {
        for m in &mods {
            eprintln!("{}", m.describe());
        }
        eprintln!("listening; planes warm-up {:.0} s; findings also appended to {}", a.cfg.warmup_s, a.events);
    }
    for x in rx {
        if n == 0 {
            // Until (unless) the stream supplies capture times, anchor on our own clock.
            let mut an = anchor.lock().unwrap();
            if an.is_none() {
                *an = Some(timebase::received_anchor(0.0));
            }
        }
        if !a.quiet {
            let an = anchor.lock().unwrap();
            if let Some(cur) = an.as_ref() {
                if last_src.as_deref() != Some(cur.source.as_str()) {
                    eprintln!(
                        "UTC anchor: start = {} ({}); lag behind this PC's clock ≈ {:.1} s",
                        timebase::fmt_utc(cur.utc0),
                        cur.source,
                        timebase::now_unix() - cur.utc(n as f64 / INPUT_SR)
                    );
                    last_src = Some(cur.source.clone());
                }
            }
        }
        n += x.len();
        for m in mods.iter_mut() {
            for f in m.push(&x) {
                let j = f.to_json(anchor.lock().unwrap().as_ref());
                println!("{j}");
                let _ = writeln!(log, "{j}");
                let _ = log.flush();
            }
        }
        if !a.quiet && last.elapsed().as_secs() >= 60 {
            last = Instant::now();
            eprintln!("… {:.1} min of audio processed", n as f64 / INPUT_SR / 60.0);
        }
    }
    for m in mods.iter_mut() {
        for f in m.flush() {
            let j = f.to_json(anchor.lock().unwrap().as_ref());
            println!("{j}");
            let _ = writeln!(log, "{j}");
        }
    }
    if let Some(mut c) = child {
        let _ = c.wait();
    }
    if !a.quiet {
        eprintln!("stream ended after {:.1} min", n as f64 / INPUT_SR / 60.0);
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
    eprintln!("wrote {out}: passes at t=200 s (f0 105 Hz) and t=430 s (f0 88 Hz), 60 Hz hum");
    Ok(())
}
