//! The grid log store: every grid frequency log the node can match against, in one format.
//!
//! `<store>/<grid>/YYYY-MM.bin`, one file per grid and UTC month: a 24-byte header (`ENFLOG1\0`,
//! the month's first second as i64 UTC, the number of seconds n as u32, how many hold a value as
//! u32) and n little-endian i16: the frequency minus 50 Hz in 0.1 mHz steps, `MISSING` where the
//! log has nothing. About 5 MB per grid and month. Grid ids are the synchronous areas of the grid
//! data file (`synchronous_areas`, e.g. GB, CE, Nordic).
//!
//! The store lives next to the program (`grid logs\`, or `CORROBORATE_GRID_LOGS`). Raw downloads
//! (untrusted, many formats) are converted into it by `grid_import`; matching reads only this.

use crate::timebase::{civil_from_days, days_from_civil};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"ENFLOG1\0";
const HEADER: u64 = 24;
pub const MISSING: i16 = i16::MIN;
/// Frequencies are stored relative to this, in 0.1 mHz steps.
pub const BASE_HZ: f64 = 50.0;
const STEP_HZ: f64 = 0.0001;

/// The store's folder: `CORROBORATE_GRID_LOGS`, else `grid logs` next to the program.
pub fn default_dir() -> PathBuf {
    match std::env::var_os("CORROBORATE_GRID_LOGS") {
        Some(d) => PathBuf::from(d),
        None => crate::home().join("grid logs"),
    }
}

/// UTC start (s) of a month.
pub fn month_start(y: i64, m: i64) -> i64 {
    days_from_civil(y, m, 1) * 86_400
}

/// The (year, month) a UTC second falls in.
pub fn month_of(t: i64) -> (i64, i64) {
    let (y, m, _) = civil_from_days(t.div_euclid(86_400));
    (y, m)
}

fn next_month(y: i64, m: i64) -> (i64, i64) {
    if m == 12 { (y + 1, 1) } else { (y, m + 1) }
}

fn encode(f: f64) -> i16 {
    let v = ((f - BASE_HZ) / STEP_HZ).round();
    if v.is_finite() && v > i16::MIN as f64 && v <= i16::MAX as f64 { v as i16 } else { MISSING }
}

fn decode(v: i16) -> f64 {
    if v == MISSING { f64::NAN } else { BASE_HZ + v as f64 * STEP_HZ }
}

/// One month file's header.
#[derive(Clone, Debug, PartialEq)]
pub struct MonthInfo {
    pub year: i64,
    pub month: i64,
    pub t0: i64,
    pub n: u32,
    pub have: u32,
}

/// The grid log store.
#[derive(Clone, Debug)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, grid: &str, y: i64, m: i64) -> PathBuf {
        self.dir.join(grid).join(format!("{y:04}-{m:02}.bin"))
    }

    /// Grids with at least one month, sorted.
    pub fn grids(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir() && !self.months(&e.file_name().to_string_lossy()).is_empty())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// A grid's months (headers only), in time order.
    pub fn months(&self, grid: &str) -> Vec<MonthInfo> {
        let mut v: Vec<MonthInfo> = std::fs::read_dir(self.dir.join(grid))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| read_header(&e.path()).ok())
            .collect();
        v.sort_by_key(|m| m.t0);
        v
    }

    /// The seconds from `t0` (UTC) on, `len` of them: Hz, NaN where missing.
    pub fn read(&self, grid: &str, t0: i64, len: usize) -> Vec<f64> {
        let mut out = vec![f64::NAN; len];
        let end = t0 + len as i64;
        let (mut y, mut m) = month_of(t0);
        while month_start(y, m) < end {
            let ms = month_start(y, m);
            if let Ok(mut f) = std::fs::File::open(self.path(grid, y, m)) {
                let n = read_header_from(&mut f).map_or(0, |h| h.n as i64);
                let (a, b) = (t0.max(ms), end.min(ms + n));
                if a < b {
                    let mut buf = vec![0u8; ((b - a) * 2) as usize];
                    if f.seek(SeekFrom::Start(HEADER + ((a - ms) * 2) as u64)).is_ok() && f.read_exact(&mut buf).is_ok() {
                        for (k, c) in buf.chunks_exact(2).enumerate() {
                            out[(a - t0) as usize + k] = decode(i16::from_le_bytes([c[0], c[1]]));
                        }
                    }
                }
            }
            (y, m) = next_month(y, m);
        }
        out
    }

    /// A human-readable coverage index, `index.json`: per grid, its months and how full each is.
    pub fn write_index(&self) -> std::io::Result<()> {
        let mut g = Vec::new();
        for grid in self.grids() {
            let ms: Vec<String> = self.months(&grid).iter().map(|m| format!("[\"{:04}-{:02}\",{:.4}]", m.year, m.month, m.have as f64 / m.n.max(1) as f64)).collect();
            g.push(format!("\"{grid}\":[{}]", ms.join(",")));
        }
        std::fs::write(self.dir.join("index.json"), format!("{{\"format\":\"ENFLOG1: 1 s, i16 (f-50 Hz)/0.1 mHz, per UTC month\",\"grids\":{{{}}}}}\n", g.join(",\n")))
    }
}

fn read_header_from(f: &mut impl Read) -> std::io::Result<MonthInfo> {
    let mut h = [0u8; HEADER as usize];
    f.read_exact(&mut h)?;
    if &h[0..8] != MAGIC {
        return Err(std::io::Error::other("not a grid log month"));
    }
    let t0 = i64::from_le_bytes(h[8..16].try_into().unwrap());
    let (year, month) = month_of(t0);
    Ok(MonthInfo { year, month, t0, n: u32::from_le_bytes(h[16..20].try_into().unwrap()), have: u32::from_le_bytes(h[20..24].try_into().unwrap()) })
}

fn read_header(p: &Path) -> std::io::Result<MonthInfo> {
    read_header_from(&mut std::fs::File::open(p)?)
}

/// Writes samples into a grid's months: keeps the months touched in memory, fills them over what
/// the store already holds (new values win), and writes them on `finish`.
pub struct Writer<'a> {
    store: &'a Store,
    grid: String,
    months: HashMap<(i64, i64), Vec<i16>>,
    /// The month last written to: (start, end, key).
    last: (i64, i64, (i64, i64)),
}

impl<'a> Writer<'a> {
    pub fn new(store: &'a Store, grid: &str) -> Self {
        Self { store, grid: grid.into(), months: HashMap::new(), last: (0, 0, (0, 0)) }
    }

    pub fn put(&mut self, t: i64, f: f64) {
        let v = encode(f);
        if v == MISSING {
            return;
        }
        if t >= self.last.0 && t < self.last.1 {
            if let Some(b) = self.months.get_mut(&self.last.2) {
                b[(t - self.last.0) as usize] = v;
                return;
            }
        }
        let (y, m) = month_of(t);
        let ms = month_start(y, m);
        let me = month_start(next_month(y, m).0, next_month(y, m).1);
        self.last = (ms, me, (y, m));
        let buf = self.months.entry((y, m)).or_insert_with(|| {
            let n = (month_start(next_month(y, m).0, next_month(y, m).1) - ms) as usize;
            let mut b = vec![MISSING; n];
            if let Ok(mut f) = std::fs::File::open(self.store.path(&self.grid, y, m)) {
                let mut raw = Vec::new();
                if read_header_from(&mut f).is_ok() && f.read_to_end(&mut raw).is_ok() {
                    for (k, c) in raw.chunks_exact(2).take(n).enumerate() {
                        b[k] = i16::from_le_bytes([c[0], c[1]]);
                    }
                }
            }
            b
        });
        buf[(t - ms) as usize] = v;
    }

    /// Write the months touched; returns how many.
    pub fn finish(self) -> std::io::Result<usize> {
        std::fs::create_dir_all(self.store.dir.join(&self.grid))?;
        for (&(y, m), b) in &self.months {
            let have = b.iter().filter(|&&v| v != MISSING).count() as u32;
            let mut out = Vec::with_capacity(HEADER as usize + b.len() * 2);
            out.extend_from_slice(MAGIC);
            out.extend_from_slice(&month_start(y, m).to_le_bytes());
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
            out.extend_from_slice(&have.to_le_bytes());
            for v in b {
                out.extend_from_slice(&v.to_le_bytes());
            }
            let p = self.store.path(&self.grid, y, m);
            let tmp = p.with_extension("tmp");
            std::fs::File::create(&tmp)?.write_all(&out)?;
            std::fs::rename(&tmp, &p)?;
        }
        Ok(self.months.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_read_across_months() {
        let dir = std::env::temp_dir().join(format!("enf-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store::new(&dir);
        let t = month_start(2016, 8) - 2;
        let mut w = Writer::new(&s, "GB");
        for k in 0..4 {
            w.put(t + k, 50.0 + 0.0123 * k as f64);
        }
        assert_eq!(w.finish().unwrap(), 2);
        let v = s.read("GB", t - 1, 6);
        assert!(v[0].is_nan() && v[5].is_nan());
        assert!((v[1] - 50.0).abs() < 1e-9 && (v[4] - 50.0369).abs() < 1e-9, "{v:?}");
        // A second write fills in, keeping what is there.
        let mut w = Writer::new(&s, "GB");
        w.put(t - 1, 49.9);
        w.finish().unwrap();
        let v = s.read("GB", t - 1, 3);
        assert!((v[0] - 49.9).abs() < 1e-9 && (v[2] - 50.0123).abs() < 1e-9, "{v:?}");
        assert_eq!(s.grids(), ["GB"]);
        assert_eq!(s.months("GB").iter().map(|m| (m.month, m.have)).collect::<Vec<_>>(), [(7, 3), (8, 2)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
