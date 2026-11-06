//! Where on Earth something can be: a mask of 1° × 1° cells, starting as the whole world.
//! Each piece of evidence is a mask of the cells it allows; stacking evidence is AND.
//! An empty result means the evidence contradicts itself: shown as such, never hidden.
//!
//! Also the grid-frequency map (which cells run on 50 Hz, which on 60 Hz), read from a
//! swappable JSON file. A cell counts for a frequency when its centre lies inside any region
//! of that frequency, so overlapping regions allow both (it never wrongly excludes a cell).

use crate::plugin::{parse_json, Json};

pub const ROWS: usize = 180;
pub const COLS: usize = 360;
const EARTH_KM2: f64 = 510_072_000.0;

/// Cell (row, col) covers latitudes 90-row-1 .. 90-row and longitudes col-180 .. col-179.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldMask {
    pub cells: Vec<bool>,
}

fn centre(r: usize, c: usize) -> (f64, f64) {
    (89.5 - r as f64, c as f64 - 179.5)
}

impl WorldMask {
    pub fn world() -> Self {
        Self { cells: vec![true; ROWS * COLS] }
    }

    pub fn none() -> Self {
        Self { cells: vec![false; ROWS * COLS] }
    }

    pub fn get(&self, r: usize, c: usize) -> bool {
        self.cells[r * COLS + c]
    }

    /// Cells whose centre lies inside any of the polygons (points as (lat, lon)).
    pub fn from_polygons(polys: &[&[(f64, f64)]]) -> Self {
        let mut m = Self::none();
        for r in 0..ROWS {
            for c in 0..COLS {
                let p = centre(r, c);
                m.cells[r * COLS + c] = polys.iter().any(|poly| inside(p, poly));
            }
        }
        m
    }

    /// Cells whose centre is within `radius_km` of a point; the cell holding the point always counts.
    pub fn circle(lat: f64, lon: f64, radius_km: f64) -> Self {
        let mut m = Self::none();
        for r in 0..ROWS {
            for c in 0..COLS {
                let (a, b) = centre(r, c);
                m.cells[r * COLS + c] = distance_km(lat, lon, a, b) <= radius_km;
            }
        }
        let r = ((90.0 - lat).floor() as usize).min(ROWS - 1);
        let c = ((lon + 180.0).floor() as usize).min(COLS - 1);
        m.cells[r * COLS + c] = true;
        m
    }

    pub fn and(&self, o: &WorldMask) -> WorldMask {
        WorldMask { cells: self.cells.iter().zip(&o.cells).map(|(a, b)| *a && *b).collect() }
    }

    pub fn or(&self, o: &WorldMask) -> WorldMask {
        WorldMask { cells: self.cells.iter().zip(&o.cells).map(|(a, b)| *a || *b).collect() }
    }

    pub fn not(&self) -> WorldMask {
        WorldMask { cells: self.cells.iter().map(|a| !a).collect() }
    }

    pub fn is_empty(&self) -> bool {
        !self.cells.iter().any(|&x| x)
    }

    /// Area of the allowed cells (cells shrink towards the poles), km².
    pub fn area_km2(&self) -> f64 {
        let row_km2 = |r: usize| {
            let (n, s) = ((90.0 - r as f64).to_radians(), (89.0 - r as f64).to_radians());
            EARTH_KM2 / 2.0 * (n.sin() - s.sin()) / COLS as f64
        };
        (0..ROWS).map(|r| row_km2(r) * (0..COLS).filter(|&c| self.get(r, c)).count() as f64).sum()
    }

    pub fn fraction(&self) -> f64 {
        self.area_km2() / EARTH_KM2
    }

    /// Allowed cells as runs per row: `[[row, first_col, last_col], …]`, for the result JSON.
    pub fn runs_json(&self) -> String {
        let mut v = Vec::new();
        for r in 0..ROWS {
            let mut c = 0;
            while c < COLS {
                if self.get(r, c) {
                    let a = c;
                    while c + 1 < COLS && self.get(r, c + 1) {
                        c += 1;
                    }
                    v.push(format!("[{r},{a},{c}]"));
                }
                c += 1;
            }
        }
        format!("[{}]", v.join(","))
    }
}

/// A location signal: the cells it marks, and whether it adds them (the place is here: drawn
/// green) or subtracts them (the place is not here: drawn red). Subtract(X) allows the same as
/// Add(not X); inverting flips add and subtract and keeps the cells.
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    pub cells: WorldMask,
    pub add: bool,
}

impl Area {
    pub fn add(cells: WorldMask) -> Self {
        Self { cells, add: true }
    }

    /// The cells still possible under this signal alone.
    pub fn allowed(&self) -> WorldMask {
        if self.add {
            self.cells.clone()
        } else {
            self.cells.not()
        }
    }

    pub fn inverted(&self) -> Area {
        Area { cells: self.cells.clone(), add: !self.add }
    }
}

fn inside((lat, lon): (f64, f64), poly: &[(f64, f64)]) -> bool {
    let mut odd = false;
    let n = poly.len();
    for i in 0..n {
        let (ai, bi) = poly[i];
        let (aj, bj) = poly[(i + n - 1) % n];
        if (ai > lat) != (aj > lat) && lon < (bj - bi) * (lat - ai) / (aj - ai) + bi {
            odd = !odd;
        }
    }
    odd
}

fn distance_km(a1: f64, b1: f64, a2: f64, b2: f64) -> f64 {
    let (p1, p2) = (a1.to_radians(), a2.to_radians());
    let h = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * ((b2 - b1).to_radians() / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().min(1.0).asin()
}

#[derive(Clone, Debug)]
pub struct GridRegion {
    pub id: String,
    pub hz: f64,
    pub polygon: Vec<(f64, f64)>,
}

/// A synchronous area: one grid, one frequency everywhere in it at any second (what a grid
/// frequency log covers). Its parts can join or leave at dates (UTC s; None: always).
#[derive(Clone, Debug)]
pub struct SyncArea {
    pub id: String,
    pub name: String,
    pub hz: f64,
    pub parts: Vec<(Vec<(f64, f64)>, Option<f64>, Option<f64>)>,
}

impl SyncArea {
    /// The cells of the area at UTC time `t` (all parts ever, when None).
    pub fn mask_at(&self, t: Option<f64>) -> WorldMask {
        let polys: Vec<&[(f64, f64)]> = self
            .parts
            .iter()
            .filter(|(_, from, to)| t.is_none_or(|t| from.is_none_or(|f| t >= f) && to.is_none_or(|e| t < e)))
            .map(|(p, _, _)| p.as_slice())
            .collect();
        WorldMask::from_polygons(&polys)
    }
}

/// The grid-frequency map: regions with their nominal frequency, the synchronous areas, and
/// where the data came from.
#[derive(Clone, Debug)]
pub struct GridMap {
    pub source: String,
    pub regions: Vec<GridRegion>,
    pub areas: Vec<SyncArea>,
}

impl GridMap {
    pub fn parse(doc: &str) -> Result<GridMap, String> {
        let j = parse_json(doc).ok_or("not valid JSON")?;
        let mut regions = Vec::new();
        for (i, r) in j.get("regions").map_or(&[][..], Json::arr).iter().enumerate() {
            let id = r.get("id").and_then(Json::str).unwrap_or("?").to_string();
            let hz = r.get("frequency_hz").and_then(Json::num).ok_or(format!("region {i} ({id}): no frequency_hz"))?;
            let polygon: Vec<(f64, f64)> = r
                .get("bounds")
                .map_or(&[][..], Json::arr)
                .iter()
                .filter_map(|p| Some((p.arr().first()?.num()?, p.arr().get(1)?.num()?)))
                .collect();
            if polygon.len() < 3 {
                return Err(format!("region {i} ({id}): bounds need at least 3 [lat, lon] points"));
            }
            regions.push(GridRegion { id, hz, polygon });
        }
        if regions.is_empty() {
            return Err("no regions".into());
        }
        let source = j.get("source").and_then(Json::str).unwrap_or("unknown").to_string();
        let poly = |b: Option<&Json>| -> Vec<(f64, f64)> { b.map_or(&[][..], Json::arr).iter().filter_map(|p| Some((p.arr().first()?.num()?, p.arr().get(1)?.num()?))).collect() };
        let date = |v: Option<&Json>, what: &str| -> Result<Option<f64>, String> {
            match v.and_then(Json::str) {
                None => Ok(None),
                Some(s) => crate::timebase::parse_iso8601(s).map(Some).ok_or(format!("{what}: not a date: {s}")),
            }
        };
        let mut areas = Vec::new();
        for a in j.get("synchronous_areas").map_or(&[][..], Json::arr) {
            let id = a.get("id").and_then(Json::str).unwrap_or("?").to_string();
            let hz = a.get("frequency_hz").and_then(Json::num).ok_or(format!("synchronous area {id}: no frequency_hz"))?;
            let mut parts = Vec::new();
            for p in a.get("parts").map_or(&[][..], Json::arr) {
                let b = poly(p.get("bounds"));
                if b.len() < 3 {
                    return Err(format!("synchronous area {id}: a part needs at least 3 [lat, lon] points"));
                }
                parts.push((b, date(p.get("from"), &id)?, date(p.get("to"), &id)?));
            }
            let name = a.get("name").and_then(Json::str).unwrap_or(&id).to_string();
            areas.push(SyncArea { id, name, hz, parts });
        }
        Ok(GridMap { source, regions, areas })
    }

    pub fn area(&self, id: &str) -> Option<&SyncArea> {
        self.areas.iter().find(|a| a.id == id)
    }

    /// Regions whose frequency matches the measured nominal (50 or 60 Hz).
    pub fn regions_for(&self, nominal: f64) -> Vec<&GridRegion> {
        self.regions.iter().filter(|r| (r.hz - nominal).abs() < 5.0).collect()
    }

    pub fn mask_for(&self, nominal: f64) -> WorldMask {
        let polys: Vec<&[(f64, f64)]> = self.regions_for(nominal).iter().map(|r| r.polygon.as_slice()).collect();
        WorldMask::from_polygons(&polys)
    }
}

/// Robinson projection (the published table, 5 degree steps, linear in between): (lat, lon) in
/// degrees to (x, y) in 0..1 across and down the map. The map is `ROBINSON_ASPECT` times wider than high.
pub fn robinson(lat: f64, lon: f64) -> (f64, f64) {
    const X: [f64; 19] = [
        1.0, 0.9986, 0.9954, 0.9900, 0.9822, 0.9730, 0.9600, 0.9427, 0.9216, 0.8962, 0.8679, 0.8350, 0.7986, 0.7597, 0.7186, 0.6732,
        0.6213, 0.5722, 0.5322,
    ];
    const Y: [f64; 19] = [
        0.0, 0.0620, 0.1240, 0.1860, 0.2480, 0.3100, 0.3720, 0.4340, 0.4958, 0.5571, 0.6176, 0.6769, 0.7346, 0.7903, 0.8435, 0.8936,
        0.9394, 0.9761, 1.0,
    ];
    let a = lat.abs().min(90.0) / 5.0;
    let i = (a.floor() as usize).min(17);
    let f = a - i as f64;
    let (xf, yf) = (X[i] + (X[i + 1] - X[i]) * f, Y[i] + (Y[i + 1] - Y[i]) * f);
    let x = 0.8487 * xf * lon.clamp(-180.0, 180.0).to_radians();
    let y = 1.3523 * yf * lat.signum();
    (0.5 + x / (2.0 * 0.8487 * std::f64::consts::PI), 0.5 - y / (2.0 * 1.3523))
}

pub const ROBINSON_ASPECT: f64 = 0.8487 * std::f64::consts::PI / 1.3523;

#[cfg(test)]
mod tests {
    /// The shipped synchronous areas: places fall in the right grid, and parts join at their
    /// dates (the Baltic states moved from their own area to Continental Europe in 2025).
    #[test]
    fn synchronous_areas() {
        use super::*;
        let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/modules/Grid frequencies (50 or 60 Hz)/grid-frequencies.json")).unwrap();
        let g = GridMap::parse(&doc).unwrap();
        let t = |s: &str| crate::timebase::parse_iso8601(s);
        let at = |lat: f64, lon: f64, when: Option<f64>| -> Vec<&str> {
            let p = WorldMask::circle(lat, lon, 1.0);
            g.areas.iter().filter(|a| !a.mask_at(when).and(&p).is_empty()).map(|a| a.id.as_str()).collect()
        };
        assert_eq!(at(51.54, -0.48, None), ["GB"], "Hillingdon");
        assert_eq!(at(48.0, 7.85, None), ["CE"], "Freiburg");
        assert_eq!(at(59.33, 18.07, None), ["Nordic"], "Stockholm");
        assert_eq!(at(53.35, -6.26, None), ["IE"], "Dublin");
        assert_eq!(at(54.69, 25.28, t("2024-06-01")), ["Baltic"], "Vilnius 2024");
        assert_eq!(at(54.69, 25.28, t("2025-06-01")), ["CE"], "Vilnius 2025");
        assert_eq!(at(50.45, 30.52, t("2021-06-01")), Vec::<&str>::new(), "Kyiv 2021");
        assert_eq!(at(50.45, 30.52, t("2023-06-01")), ["CE"], "Kyiv 2023");
    }

    #[test]
    fn robinson_corners() {
        let (x, y) = robinson(0.0, 0.0);
        assert!((x - 0.5).abs() < 1e-9 && (y - 0.5).abs() < 1e-9);
        assert!((robinson(0.0, 180.0).0 - 1.0).abs() < 1e-9);
        assert!(robinson(90.0, 0.0).1.abs() < 1e-9);
        assert!((robinson(90.0, 180.0).0 - (0.5 + 0.5 * 0.5322)).abs() < 1e-9);
    }

    use super::*;

    #[test]
    fn invert_flips_what_is_allowed() {
        let c = WorldMask::circle(50.0, 10.0, 500.0);
        let a = Area::add(c.clone());
        assert_eq!(a.allowed(), c);
        assert_eq!(a.inverted().allowed(), c.not());
        assert_eq!(a.inverted().inverted(), a);
        assert!(a.allowed().and(&a.inverted().allowed()).is_empty());
    }

    #[test]
    fn world_is_whole_earth() {
        assert!((WorldMask::world().fraction() - 1.0).abs() < 1e-9);
        assert!(WorldMask::none().is_empty());
    }

    #[test]
    fn grid_mask_and_contradiction() {
        let doc = r#"{"source":"t","regions":[
            {"id":"A","frequency_hz":50,"bounds":[[40,0],[40,20],[60,20],[60,0]]},
            {"id":"B","frequency_hz":60,"bounds":[[20,-120],[20,-70],[50,-70],[50,-120]]}]}"#;
        let g = GridMap::parse(doc).unwrap();
        let m50 = g.mask_for(50.0);
        let poland = WorldMask::circle(52.0, 19.0, 300.0);
        assert!(!m50.and(&poland).is_empty());
        assert!(g.mask_for(60.0).and(&poland).is_empty());
        assert!(m50.fraction() < 0.02 && m50.fraction() > 0.005);
    }
}
