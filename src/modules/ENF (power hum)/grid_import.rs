//! Converts raw grid frequency downloads into the store (`grid_store`). Each raw file is
//! imported once (`imported.txt` in the store lists them with their sizes); a file that grew or
//! is new is imported again. Raw downloads are untrusted data: only parsed, never run.
//!
//! Known layouts under the raw folder:
//! - `neso/` (GB): `f-YYYY-M.csv` / `fnew-YYYY-M.csv`, some zipped. `dtm,f`; times with an offset
//!   (`2016-07-01 01:00:00 +0100`), UK local time (`01/07/2014 01:00:00`, 2014), or UTC with no
//!   offset (the `fnew` files).
//! - `energy_charts/` (CE): one JSON per UTC day, `unix_seconds` and `data` arrays.
//! - `zenodo_2012_2021/` (record 5105820): `YYYY_cleansed.zip` holding zipped yearly CSVs per
//!   operator, naive local time with summer time: TransnetBW (CE, CET), Fingrid (Nordic, EET).
//!   Its Nationalgrid files are skipped (NESO has them).

use super::grid_store::{Store, Writer};
use crate::timebase::days_from_civil;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// European summer time (since 1996): from the last Sunday of March to the last Sunday of
/// October, 01:00 UTC both.
fn eu_summer(utc: i64) -> bool {
    let y = crate::modules::enf::grid_store::month_of(utc).0;
    let last_sunday = |m: i64| {
        let d31 = days_from_civil(y, m, 31);
        // 1970-01-01 was a Thursday: (days + 4) % 7 == 0 on Sundays.
        d31 - (d31 + 4).rem_euclid(7)
    };
    let (a, b) = (last_sunday(3) * 86_400 + 3600, last_sunday(10) * 86_400 + 3600);
    utc >= a && utc < b
}

/// Naive local time (with EU summer time) to UTC. `std_s`: the zone's standard offset. In the
/// repeated autumn hour the first pass is summer time, the second standard time (by order).
pub struct LocalClock {
    std_s: i64,
    last: i64,
}

impl LocalClock {
    pub fn new(std_s: i64) -> Self {
        Self { std_s, last: i64::MIN }
    }

    /// None in the skipped spring hour.
    pub fn utc(&mut self, local: i64) -> Option<i64> {
        let summer = local - self.std_s - 3600;
        let winter = local - self.std_s;
        let ok: Vec<i64> = [summer, winter].into_iter().filter(|&c| eu_summer(c) == (c == summer)).collect();
        let t = ok.iter().copied().find(|&c| c > self.last).or(ok.first().copied())?;
        self.last = t;
        Some(t)
    }
}

/// `YYYY-MM-DD HH:MM:SS` (or `YYYY-MM-DD`) as seconds, read as if UTC; also returns the rest
/// of the field after the seconds (an offset, if any).
fn naive(s: &[u8]) -> Option<(i64, &[u8])> {
    let d = |a: usize, b: usize| -> Option<i64> {
        let mut v = 0i64;
        for &c in s.get(a..b)? {
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (c - b'0') as i64;
        }
        Some(v)
    };
    let (y, mo, day, rest_at) = if s.get(2) == Some(&b'/') { (d(6, 10)?, d(3, 5)?, d(0, 2)?, 10) } else { (d(0, 4)?, d(5, 7)?, d(8, 10)?, 10) };
    let base = days_from_civil(y, mo, day) * 86_400;
    if s.len() < rest_at + 9 {
        return Some((base, &[]));
    }
    Some((base + d(rest_at + 1, rest_at + 3)? * 3600 + d(rest_at + 4, rest_at + 6)? * 60 + d(rest_at + 7, rest_at + 9)?, &s[rest_at + 9..]))
}

/// An offset like ` +0100` / `+01:00`, in seconds; None when there is none.
fn offset(rest: &[u8]) -> Option<i64> {
    let s = std::str::from_utf8(rest).ok()?.trim();
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits: String = s[1..].chars().filter(|c| c.is_ascii_digit()).collect();
    let h: i64 = digits.get(0..2)?.parse().ok()?;
    let m: i64 = digits.get(2..4).and_then(|v| v.parse().ok()).unwrap_or(0);
    Some(sign * (h * 3600 + m * 60))
}

/// How a CSV's times are read.
#[derive(Clone, Copy)]
enum Times {
    /// NESO: an explicit offset if given; else dd/mm/yyyy is UK local time; else UTC.
    Neso,
    /// Naive local time in a zone with EU summer time and this standard offset.
    Local(i64),
}

/// Read a `time,frequency` CSV (a header line is skipped as unparsable) into `w`.
fn read_csv(path: &Path, times: Times, w: &mut Writer) -> Result<usize, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut r = BufReader::with_capacity(1 << 20, f);
    let mut line = Vec::with_capacity(64);
    let (mut uk, mut local) = (LocalClock::new(0), LocalClock::new(match times { Times::Local(s) => s, _ => 0 }));
    let mut n = 0;
    loop {
        line.clear();
        if r.read_until(b'\n', &mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let Some(comma) = line.iter().position(|&c| c == b',') else { continue };
        let (ts, fv) = (&line[..comma], &line[comma + 1..]);
        let Some((t, rest)) = naive(ts) else { continue };
        let t = match times {
            Times::Neso => match offset(rest) {
                Some(o) => Some(t - o),
                None if ts.get(2) == Some(&b'/') => uk.utc(t),
                None => Some(t),
            },
            Times::Local(_) => local.utc(t),
        };
        let Some(t) = t else { continue };
        let Ok(f) = std::str::from_utf8(fv).unwrap_or("").trim().parse::<f64>() else { continue };
        if f.is_finite() {
            w.put(t, f);
            n += 1;
        }
    }
    Ok(n)
}

/// Energy-Charts day: `{"unix_seconds":[...],"data":[...]}` (values may be null).
fn read_energy_charts(path: &Path, w: &mut Writer) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let arr = |key: &str| -> Vec<&str> {
        let Some(i) = text.find(&format!("\"{key}\":[")) else { return Vec::new() };
        let s = &text[i + key.len() + 4..];
        s[..s.find(']').unwrap_or(0)].split(',').map(str::trim).collect()
    };
    let (ts, fs) = (arr("unix_seconds"), arr("data"));
    let mut n = 0;
    for (t, f) in ts.iter().zip(&fs) {
        if let (Ok(t), Ok(f)) = (t.parse::<i64>(), f.parse::<f64>()) {
            w.put(t, f);
            n += 1;
        }
    }
    Ok(n)
}

/// The system's zip-capable tar (bsdtar ships with Windows 10 and later).
fn tar() -> std::process::Command {
    let exe = if cfg!(windows) { "C:\\Windows\\System32\\tar.exe" } else { "bsdtar" };
    let mut c = std::process::Command::new(exe);
    crate::no_window(&mut c);
    c
}

/// Extract `members` (all when empty) of a zip into `dir`.
fn unzip(zip: &Path, dir: &Path, members: &[&str]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let out = tar().arg("-xf").arg(zip).arg("-C").arg(dir).args(members).output().map_err(|e| format!("tar: {e}"))?;
    if !out.status.success() {
        return Err(format!("unzip {}: {}", zip.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

fn csvs_in(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            v.extend(csvs_in(&p));
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("csv")) {
            v.push(p);
        }
    }
    v
}

fn files(dir: &Path, ext: &[&str]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| ext.iter().any(|e| x.eq_ignore_ascii_case(e))))
        .collect();
    v.sort();
    v
}

/// Import everything new under `raw` into `store`. `say` gets one line per file.
pub fn import_all(raw: &Path, store: &Store, say: &mut dyn FnMut(&str)) -> Result<(), String> {
    std::fs::create_dir_all(&store.dir).map_err(|e| e.to_string())?;
    let done_path = store.dir.join("imported.txt");
    let mut done: HashMap<String, u64> = std::fs::read_to_string(&done_path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.rsplit_once('\t').and_then(|(k, v)| Some((k.to_string(), v.parse().ok()?))))
        .collect();
    let tmp = store.dir.join("tmp");
    let mut jobs: Vec<(PathBuf, &str)> = Vec::new();
    jobs.extend(files(&raw.join("neso"), &["csv", "zip"]).into_iter().map(|p| (p, "neso")));
    jobs.extend(files(&raw.join("zenodo_2012_2021"), &["zip"]).into_iter().filter(|p| p.to_string_lossy().contains("_cleansed")).map(|p| (p, "zenodo")));
    let ec = files(&raw.join("energy_charts"), &["json"]);
    for (p, kind) in jobs {
        let key = p.strip_prefix(raw).unwrap_or(&p).to_string_lossy().replace('\\', "/");
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        if done.get(&key) == Some(&size) {
            continue;
        }
        let t = std::time::Instant::now();
        let n = match kind {
            "neso" => {
                let mut w = Writer::new(store, "GB");
                let n = if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
                    let _ = std::fs::remove_dir_all(&tmp);
                    unzip(&p, &tmp, &[])?;
                    csvs_in(&tmp).iter().map(|c| read_csv(c, Times::Neso, &mut w)).sum::<Result<usize, String>>()?
                } else {
                    read_csv(&p, Times::Neso, &mut w)?
                };
                w.finish().map_err(|e| e.to_string())?;
                n
            }
            _ => {
                // YYYY_cleansed.zip: YYYY_cleansed/<operator>/YYYY.zip, each one CSV.
                let name = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                let year = name.split('_').next().unwrap_or("");
                let mut n = 0;
                let listing = tar().arg("-tf").arg(&p).output().map_err(|e| format!("tar: {e}"))?;
                let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
                for (op, grid, std_s) in [("TransnetBW", "CE", 3600), ("Fingrid", "Nordic", 7200)] {
                    let _ = std::fs::remove_dir_all(&tmp);
                    let inner = format!("{name}/{op}/{year}.zip");
                    if !listing.lines().any(|l| l.trim() == inner) {
                        say(&format!("{key}: no {op} ({grid}) in it"));
                        continue;
                    }
                    unzip(&p, &tmp, &[&inner])?;
                    let tmp2 = tmp.join("csv");
                    unzip(&tmp.join(&inner), &tmp2, &[])?;
                    let mut w = Writer::new(store, grid);
                    for c in csvs_in(&tmp2) {
                        n += read_csv(&c, Times::Local(std_s), &mut w)?;
                    }
                    w.finish().map_err(|e| e.to_string())?;
                }
                n
            }
        };
        let _ = std::fs::remove_dir_all(&tmp);
        say(&format!("{key}: {n} s of log ({:.0} s)", t.elapsed().as_secs_f64()));
        done.insert(key, size);
        save_done(&done_path, &done)?;
    }
    // Energy-Charts: many small days, one writer.
    let todo: Vec<&PathBuf> = ec
        .iter()
        .filter(|p| {
            let key = p.strip_prefix(raw).unwrap_or(p).to_string_lossy().replace('\\', "/");
            done.get(&key) != std::fs::metadata(p).ok().map(|m| m.len()).as_ref()
        })
        .collect();
    if !todo.is_empty() {
        let mut w = Writer::new(store, "CE");
        let mut n = 0;
        for p in &todo {
            n += read_energy_charts(p, &mut w)?;
            let key = p.strip_prefix(raw).unwrap_or(p).to_string_lossy().replace('\\', "/");
            done.insert(key, std::fs::metadata(p).map(|m| m.len()).unwrap_or(0));
        }
        w.finish().map_err(|e| e.to_string())?;
        save_done(&done_path, &done)?;
        say(&format!("energy_charts: {} days, {n} s of log", todo.len()));
    }
    store.write_index().map_err(|e| e.to_string())
}

fn save_done(path: &Path, done: &HashMap<String, u64>) -> Result<(), String> {
    let mut v: Vec<String> = done.iter().map(|(k, s)| format!("{k}\t{s}")).collect();
    v.sort();
    std::fs::write(path, v.join("\n") + "\n").map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timebase::parse_iso8601;

    fn utc(s: &str) -> i64 {
        parse_iso8601(s).unwrap() as i64
    }

    #[test]
    fn local_times() {
        // CET: winter +1, summer +2; summer 2016 from 27 March to 30 October.
        let mut c = LocalClock::new(3600);
        let l = |s: &str| naive(s.as_bytes()).unwrap().0;
        assert_eq!(c.utc(l("2016-01-10 12:00:00")), Some(utc("2016-01-10T11:00:00Z")));
        assert_eq!(c.utc(l("2016-03-27 02:30:00")), None, "skipped spring hour");
        assert_eq!(c.utc(l("2016-07-07 20:29:08")), Some(utc("2016-07-07T18:29:08Z")));
        // The repeated autumn hour: first summer, then winter time.
        assert_eq!(c.utc(l("2016-10-30 02:30:00")), Some(utc("2016-10-30T00:30:00Z")));
        assert_eq!(c.utc(l("2016-10-30 02:30:00")), Some(utc("2016-10-30T01:30:00Z")));
        // UK local (dd/mm/yyyy in NESO 2014).
        let mut uk = LocalClock::new(0);
        assert_eq!(uk.utc(l("01/07/2014 01:00:00")), Some(utc("2014-07-01T00:00:00Z")));
        assert_eq!(offset(naive(b"2016-07-01 01:00:00 +0100").unwrap().1), Some(3600));
        assert_eq!(offset(naive(b"2023-07-01 00:00:00").unwrap().1), None);
        assert_eq!(naive(b"2026-09-01").map(|x| x.0), Some(utc("2026-09-01T00:00:00Z")));
    }
}
