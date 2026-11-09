//! The Map node: everything wired into it on one map you can move and zoom, on the canvas.
//! Wire a location in at any step of a chain to see that step; it has as many inputs as you wire.
//!
//! - **Map**: walkers (Web Mercator, wheel to zoom, drag to move, each node its own view). Detail
//!   comes from `.pmtiles` files in `maps\` next to the program (offline vector tiles, README
//!   "Maps"); without them the built-in Blue Marble image is drawn in the same projection. Google's
//!   satellite and road maps can be switched on (google_maps.rs): one choice for all Map nodes,
//!   online, so offline at every start and marked while on.
//! - **Points** (a typed latitude and longitude, a file's GPS) are pins; a click selects the node they come from. **Areas** are their 1° cells, green where
//!   they add and red where they subtract. New inputs are framed by themselves.
//!
//! `paint` is the small fixed world map (Robinson over NASA Blue Marble, Natural Earth coastlines)
//! that other nodes showing an area (Result record, ENF) draw.

use super::*;
use corroborate::data::{Data, Site};
use walkers::{lat_lon, MapMemory, PmTiles, Projector, Tiles};

/// The map's size in the node.
const NODE_W: f32 = 360.0;
const MAP_H: f32 = 240.0;

const PIN: Color32 = Color32::from_rgb(96, 165, 250);
const KEPT: Color32 = Color32::from_rgba_premultiplied(30, 110, 40, 140);
const CUT: Color32 = Color32::from_rgba_premultiplied(120, 20, 20, 140);

/// The Map node on the canvas: wider, with the map in it.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, _app: &App, _i: NodeId, standard: Vec2) -> Vec2 {
        Vec2::new(NODE_W, standard.y + MAP_H)
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        app.map_body(ui, i);
    }
}

/// One line about an area: whether it adds or subtracts, and how much of Earth it leaves.
pub fn area_status(a: &Area) -> String {
    let al = a.allowed();
    if al.is_empty() {
        "nothing possible: contradiction".into()
    } else {
        format!("{} · {:.1} % of Earth possible", if a.add { "adds" } else { "subtracts" }, al.fraction() * 100.0)
    }
}

/// Natural Earth 1:110m coastlines, built into the program: lines of (lat, lon).
fn coastlines() -> &'static [Vec<(f64, f64)>] {
    static C: std::sync::OnceLock<Vec<Vec<(f64, f64)>>> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        include_str!("coastlines.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| {
                l.split(' ')
                    .filter_map(|p| {
                        let (lon, lat) = p.split_once(',')?;
                        Some((lat.parse().ok()?, lon.parse().ok()?))
                    })
                    .collect()
            })
            .collect()
    })
}

/// The Earth image warped from its plate carrée grid onto the Robinson shape in 3° × 6° patches
/// that share their corners: vertex positions (0..1 across the map) and texture positions,
/// worked out once. 3,600 patches; finer did not look different at node size and cost more.
fn earth_patches() -> &'static (Vec<(Pos2, Pos2)>, Vec<u32>) {
    const R: usize = 60;
    const C: usize = 60;
    static P: std::sync::OnceLock<(Vec<(Pos2, Pos2)>, Vec<u32>)> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let mut verts = Vec::with_capacity((R + 1) * (C + 1));
        for r in 0..=R {
            let lat = 90.0 - 180.0 * r as f64 / R as f64;
            for c in 0..=C {
                let lon = -180.0 + 360.0 * c as f64 / C as f64;
                let (x, y) = robinson(lat, lon);
                let uv = Pos2::new(((lon + 180.0) / 360.0) as f32, ((90.0 - lat) / 180.0) as f32);
                verts.push((Pos2::new(x as f32, y as f32), uv));
            }
        }
        let mut idx = Vec::with_capacity(R * C * 6);
        let at = |r: usize, c: usize| (r * (C + 1) + c) as u32;
        for r in 0..R {
            for c in 0..C {
                let (a, b, d, e) = (at(r, c), at(r, c + 1), at(r + 1, c + 1), at(r + 1, c));
                idx.extend([a, b, d, a, d, e]);
            }
        }
        (verts, idx)
    })
}

pub fn paint(p: &egui::Painter, rect: Rect, earth: Option<egui::TextureId>, area: Option<&Area>) {
    let at = |lat: f64, lon: f64| {
        let (x, y) = robinson(lat, lon);
        Pos2::new(rect.left() + x as f32 * rect.width(), rect.top() + y as f32 * rect.height())
    };
    let place = |u: Pos2| Pos2::new(rect.left() + u.x * rect.width(), rect.top() + u.y * rect.height());
    // The Earth image, with the coastlines and the 30° lines already drawn into it (see `load_earth`).
    match earth {
        Some(tex) => {
            let (verts, idx) = earth_patches();
            let mut img = egui::epaint::Mesh::with_texture(tex);
            img.vertices = verts.iter().map(|&(u, uv)| egui::epaint::Vertex { pos: place(u), uv, color: Color32::WHITE }).collect();
            img.indices = idx.clone();
            p.add(egui::Shape::mesh(img));
        }
        None => {
            let pts: Vec<Pos2> = (0..=36).map(|j| at(90.0 - j as f64 * 5.0, 180.0)).chain((0..=36).map(|j| at(-90.0 + j as f64 * 5.0, -180.0))).collect();
            p.add(egui::Shape::convex_polygon(pts, Color32::from_rgb(44, 48, 54), Stroke::NONE));
        }
    }
    // The area's cells, tinted over the image: green when it adds, red when it subtracts.
    // Merged into runs per row: within a row the projection is linear in longitude, so a run is
    // exactly a trapezoid.
    if let Some(area) = area {
        let tint = if area.add { KEPT } else { CUT };
        let mut mesh = egui::epaint::Mesh::default();
        for r in 0..ROWS {
            let (top, bot) = (90.0 - r as f64, 89.0 - r as f64);
            let mut c = 0;
            while c < COLS {
                if !area.cells.get(r, c) {
                    c += 1;
                    continue;
                }
                let a = c;
                while c + 1 < COLS && area.cells.get(r, c + 1) {
                    c += 1;
                }
                let (l, rt) = (a as f64 - 180.0, c as f64 - 179.0);
                let i = mesh.vertices.len() as u32;
                for q in [at(top, l), at(top, rt), at(bot, rt), at(bot, l)] {
                    mesh.colored_vertex(q, tint);
                }
                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i, i + 2, i + 3);
                c += 1;
            }
        }
        p.add(egui::Shape::mesh(mesh));
    }
    if earth.is_none() {
        // No image: the coastlines as lines (slower; normally they are in the image).
        let coast = Stroke::new(0.8, Color32::from_rgba_unmultiplied(235, 240, 235, 110));
        for line in coastlines() {
            let mut seg: Vec<Pos2> = Vec::new();
            for w in line.windows(2) {
                if seg.is_empty() {
                    seg.push(at(w[0].0, w[0].1));
                }
                if (w[1].1 - w[0].1).abs() > 180.0 {
                    p.add(egui::Shape::line(std::mem::take(&mut seg), coast));
                    continue;
                }
                seg.push(at(w[1].0, w[1].1));
            }
            p.add(egui::Shape::line(seg, coast));
        }
    }
}

const EARTH_W: usize = 2048;
const EARTH_H: usize = 1024;

/// Blend a colour into one pixel of the Earth image (plate carrée: x from longitude, y from latitude).
fn blend(px: &mut [u8], lat: f64, lon: f64, rgb: [u8; 3], a: f32) {
    let x = (((lon + 180.0) / 360.0) * EARTH_W as f64).floor().clamp(0.0, (EARTH_W - 1) as f64) as usize;
    let y = (((90.0 - lat) / 180.0) * EARTH_H as f64).floor().clamp(0.0, (EARTH_H - 1) as f64) as usize;
    // Two pixels wide, so the line survives the image being drawn small.
    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let k = ((y + dy).min(EARTH_H - 1) * EARTH_W + (x + dx).min(EARTH_W - 1)) * 4;
        for c in 0..3 {
            px[k + c] = (px[k + c] as f32 * (1.0 - a) + rgb[c] as f32 * a).round() as u8;
        }
    }
}

/// A line in the image from one (lat, lon) to another, a pixel at a time.
fn line_into(px: &mut [u8], a: (f64, f64), b: (f64, f64), rgb: [u8; 3], alpha: f32) {
    let step = 180.0 / EARTH_H as f64;
    let n = (((b.0 - a.0).abs().max((b.1 - a.1).abs())) / step).ceil().max(1.0) as usize;
    for k in 0..=n {
        let f = k as f64 / n as f64;
        blend(px, a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, rgb, alpha);
    }
}

/// NASA Blue Marble (public domain), 2048 × 1024 plate carrée, stored as a 256-colour palette
/// followed by one palette index per pixel. The coastlines (Natural Earth 1:110m) and the lines
/// every 30° are drawn into it once here, so maps cost little to draw each frame.
pub fn load_earth(ctx: &egui::Context) -> TextureHandle {
    let raw: &[u8] = include_bytes!("earth.idx");
    let (pal, idx) = raw.split_at(768);
    let mut px = Vec::with_capacity(EARTH_W * EARTH_H * 4);
    for &i in &idx[..EARTH_W * EARTH_H] {
        let k = i as usize * 3;
        px.extend_from_slice(&[pal[k], pal[k + 1], pal[k + 2], 255]);
    }
    for k in -5i32..=5 {
        line_into(&mut px, (90.0, k as f64 * 30.0), (-90.0, k as f64 * 30.0), [0, 0, 0], 0.35);
    }
    for k in -2i32..=2 {
        line_into(&mut px, (k as f64 * 30.0, -180.0), (k as f64 * 30.0, 180.0), [0, 0, 0], 0.35);
    }
    for line in coastlines() {
        for w in line.windows(2) {
            if (w[1].1 - w[0].1).abs() <= 180.0 {
                line_into(&mut px, w[0], w[1], [235, 240, 235], 0.6);
            }
        }
    }
    // Mipmaps: smooth when the map is drawn much smaller than the image.
    let opts = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
    ctx.load_texture("earth", ColorImage::from_rgba_unmultiplied([EARTH_W, EARTH_H], &px), opts)
}

/// Where map files are read from: `maps\` next to the program.
pub fn maps_dir() -> PathBuf {
    corroborate::home().join("maps")
}

/// One location wired into a Map node.
pub struct Place {
    pub node: NodeId,
    pub name: String,
    pub site: Option<Site>,
    pub area: Arc<Area>,
    /// Just a point: its area is only the 1° cell it is in, which says nothing about how sure it
    /// is. A place typed with a radius has a site and an area, and shows both.
    pub point: bool,
}

/// What the Map nodes share: the map files and Google, and each node's own view.
#[derive(Default)]
pub struct Maps {
    /// Map files found in `maps\` (path, tiles); read once, again on ↻.
    tiles: Option<Vec<(PathBuf, PmTiles)>>,
    /// Google's map tiles, when switched on (google_maps.rs).
    pub google: google_maps::Google,
    /// By the node's stable id.
    pub views: HashMap<String, NodeMap>,
    /// Where the maps are on screen: the canvas leaves the wheel to them.
    pub over: Vec<Rect>,
}

/// One Map node's view.
#[derive(Default)]
pub struct NodeMap {
    pub memory: MapMemory,
    /// What was last framed: new inputs frame again.
    framed: Vec<[i64; 4]>,
    pub ask: google_maps::ViewAsk,
}

/// `51.50740° N, 0.12780° W`.
pub fn site_text(s: &Site) -> String {
    let mut t = format!("{:.5}° {}, {:.5}° {}", s.lat.abs(), if s.lat >= 0.0 { "N" } else { "S" }, s.lon.abs(), if s.lon >= 0.0 { "E" } else { "W" });
    if let Some(e) = s.elevation {
        t.push_str(&format!(", {e:.0} m"));
    }
    t
}

/// The zoom that shows `span` degrees across; one point gets street level when there is detail to
/// see, else as close as the world image is still sharp (4.5).
fn frame_zoom(span_deg: f64, detail: bool) -> f64 {
    if span_deg < 1e-4 {
        return if detail { 15.0 } else { 4.5 };
    }
    (360.0 / (span_deg * 1.6)).log2().clamp(0.0, if detail { 16.0 } else { 4.5 })
}

/// South, west, north and east of what a location puts on the map, in degrees. An area that only
/// subtracts, or the whole world (no evidence yet), has nothing to frame.
fn extent(p: &Place) -> Option<[f64; 4]> {
    if let (true, Some(s)) = (p.point, p.site) {
        return Some([s.lat, s.lon, s.lat, s.lon]);
    }
    if !p.area.add || p.area.cells.fraction() > 0.999 {
        return None;
    }
    let mut b: Option<[f64; 4]> = None;
    for r in 0..ROWS {
        for c in 0..COLS {
            if p.area.cells.get(r, c) {
                let (top, left) = (90.0 - r as f64, c as f64 - 180.0);
                let e = b.get_or_insert([top - 1.0, left, top, left + 1.0]);
                *e = [e[0].min(top - 1.0), e[1].min(left), e[2].max(top), e[3].max(left + 1.0)];
            }
        }
    }
    b
}

/// At most one cell of the map.
fn one_cell(a: &Area) -> bool {
    (0..ROWS).flat_map(|r| (0..COLS).map(move |c| (r, c))).filter(|&(r, c)| a.cells.get(r, c)).nth(1).is_none()
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

impl App {
    /// The locations wired into a Map node, and why those without one have none.
    pub(crate) fn map_inputs(&self, i: NodeId) -> (Vec<Place>, Vec<String>) {
        let (mut places, mut errors) = (Vec::new(), Vec::new());
        for p in &self.graph.nodes()[i].spec.inputs {
            let Some((f, out)) = self.graph.feeder(i, p.name) else { continue };
            match self.value_from(f, out) {
                Ok(Data::Location { area, site, .. }) => places.push(Place { node: f, name: self.node_name(f), point: site.is_some() && one_cell(&area), site, area }),
                Ok(_) => errors.push(format!("{} gives no location", self.node_name(f))),
                Err(e) => errors.push(e),
            }
        }
        (places, errors)
    }

    pub(crate) fn map_status(&self, i: NodeId) -> String {
        let (places, errors) = self.map_inputs(i);
        match (places.as_slice(), errors.first()) {
            ([], None) => "no location wired".into(),
            ([], Some(e)) => e.clone(),
            ([p], None) => p.site.filter(|_| p.point).map_or_else(|| area_status(&p.area), |s| site_text(&s)),
            _ => {
                let pts = places.iter().filter(|p| p.point).count();
                let mut t = format!("{}, {}", plural(pts, "point"), plural(places.len() - pts, "area"));
                if !errors.is_empty() {
                    t.push_str(&format!(" · {} without a value", errors.len()));
                }
                t
            }
        }
    }

    /// Reads `maps\` the first time a map is drawn.
    fn read_map_files(&mut self, ctx: &egui::Context) {
        if self.maps.tiles.is_some() {
            return;
        }
        let mut found = Vec::new();
        if let Ok(rd) = std::fs::read_dir(maps_dir()) {
            let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pmtiles"))).collect();
            paths.sort();
            for p in paths {
                let tiles = PmTiles::with_style(&p, walkers::Style::protomaps_dark(), ctx.clone());
                found.push((p, tiles));
            }
        }
        self.maps.tiles = Some(found);
    }

    /// The line choosing what the maps show; with Google on, what that means.
    fn map_source_bar(&mut self, ui: &mut egui::Ui, i: NodeId) {
        use google_maps::MapSource;
        let gap = 6.0 * self.zoom;
        let n_maps = self.maps.tiles.as_ref().map_or(0, |t| t.len());
        let mut rescan = false;
        let g = &mut self.maps.google;
        let mut pick = g.source;
        ui.horizontal(|ui| {
            ui.add_space(gap);
            egui::ComboBox::from_id_salt(("map-source", i)).selected_text(pick.label()).show_ui(ui, |ui| {
                for s in MapSource::ALL {
                    ui.selectable_value(&mut pick, s, s.label());
                }
            });
            if g.live() {
                let n = g.fetched.load(Ordering::Relaxed);
                ui.colored_label(WARN, format!("ONLINE · {n} tiles")).on_hover_text(
                    "Google sees which part of the world you look at.\nThe tiles are counted since the program started; Google's free allowance is per month.\nFor viewing only: its terms forbid image analysis, extracting data and offline use.",
                );
                if ui.small_button("key").on_hover_text("forget the saved API key").clicked() {
                    g.forget_key();
                }
            } else if g.connecting() {
                ui.colored_label(DIM, "connecting to Google…");
            } else if g.source == MapSource::Offline {
                ui.colored_label(DIM, if n_maps == 0 { "world image".to_string() } else { plural(n_maps, "map file") })
                    .on_hover_text(format!("offline map files (.pmtiles) are read from {}\nnothing is fetched from the internet; see README, Maps", maps_dir().display()));
                rescan = ui.small_button("↻").on_hover_text("read the maps folder again").clicked();
            }
        });
        g.set_source(pick);
        if g.source != MapSource::Offline && g.key.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(gap);
                ui.colored_label(DIM, "API key");
                ui.add(egui::TextEdit::singleline(&mut g.draft).password(true).desired_width(170.0)).on_hover_text("your own key from console.cloud.google.com, with the Map Tiles API enabled; kept in settings\\ next to the program, asked once");
                if ui.small_button("Use").clicked() {
                    let typed = g.draft.clone();
                    g.use_key(&typed);
                }
            });
        }
        if g.source != MapSource::Offline && !g.error.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(gap);
                if ui.small_button("Try again").clicked() {
                    g.again();
                }
                if !g.key.is_empty() && ui.small_button("Change key").clicked() {
                    g.forget_key();
                }
                ui.add(egui::Label::new(egui::RichText::new(g.error.clone()).color(Color32::from_rgb(235, 80, 80))).truncate()).on_hover_text(g.error.clone());
            });
        }
        if rescan {
            self.maps.tiles = None;
        }
    }

    /// The node's contents: the line choosing the map, then the map.
    pub(crate) fn map_body(&mut self, ui: &mut egui::Ui, i: NodeId) {
        let ctx = ui.ctx().clone();
        self.read_map_files(&ctx);
        self.maps.google.tick(&ctx);
        self.map_source_bar(ui, i);
        self.read_map_files(&ctx);
        let (places, _) = self.map_inputs(i);
        let colours: Vec<(Color32, String)> = places.iter().map(|_| (PIN, String::new())).collect();
        let online = self.maps.google.live();
        let n_maps = self.maps.tiles.as_ref().map_or(0, |t| t.len());
        let uid = self.nodes[i].uid.clone();
        let earth = self.earth.as_ref().map(|t| t.id());
        let z = self.zoom;
        let rest = ui.available_rect_before_wrap();
        self.maps.over.push(rest.intersect(ui.clip_rect()));
        let Maps { tiles, google, views, .. } = &mut self.maps;
        let view = views.entry(uid).or_default();

        // Frame new inputs (one arriving, or changing).
        let boxes: Vec<[f64; 4]> = places.iter().filter_map(extent).collect();
        let key: Vec<[i64; 4]> = boxes.iter().map(|b| b.map(|x| (x * 1e5) as i64)).collect();
        if !boxes.is_empty() && key != view.framed {
            let [s, w, n, e] = boxes.iter().fold([90.0f64, 180.0, -90.0, -180.0], |a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
            view.memory.center_at(lat_lon((s + n) / 2.0, (w + e) / 2.0));
            let _ = view.memory.set_zoom(frame_zoom((n - s).max(e - w), n_maps > 0 || online));
            view.framed = key;
        }

        let mut clicked_pin: Option<NodeId> = None;
        let credit = google.credit(&view.ask);
        // The view on screen: north, south, east, west.
        let mut seen: Option<(f64, f64, f64, f64)> = None;
        let tiles = tiles.as_mut().expect("read above");
        let (base, layers): (Option<&mut dyn Tiles>, &mut [(PathBuf, PmTiles)]) = match (google.tiles_mut(), tiles.split_first_mut()) {
            (Some(g), _) => (Some(g as &mut dyn Tiles), &mut []),
            (None, Some((first, rest))) => (Some(&mut first.1 as &mut dyn Tiles), rest),
            (None, None) => (None, &mut []),
        };
        let detail = base.is_some();
        ui.allocate_ui(rest.size(), |ui| {
            let mut map = walkers::Map::new(base, &mut view.memory, lat_lon(20.0, 0.0)).zoom_with_ctrl(false);
            for l in layers.iter_mut() {
                map = map.with_layer(&mut l.1, 1.0);
            }
            map.show(ui, |ui, resp, proj, _mem| {
                let p = ui.painter().with_clip_rect(resp.rect.intersect(ui.clip_rect()));
                if !detail {
                    p.rect_filled(resp.rect, 0.0, BG);
                    if let Some(tex) = earth {
                        paint_earth(&p, proj, tex);
                    }
                }
                for pl in places.iter().filter(|pl| !pl.point) {
                    paint_area(&p, proj, &pl.area);
                }
                for (pl, (color, group)) in places.iter().zip(&colours) {
                    let Some(s) = pl.site else { continue };
                    let at = proj.project(lat_lon(s.lat, s.lon)).to_pos2();
                    p.circle_filled(at, 6.0 * z, *color);
                    p.circle_stroke(at, 6.0 * z, Stroke::new(1.5 * z, Color32::WHITE));
                    p.text(at + Vec2::new(10.0 * z, 0.0), Align2::LEFT_CENTER, format!("{group} {}", pl.name).trim(), FontId::proportional(11.5 * z), Color32::WHITE);
                    if resp.clicked() && resp.interact_pointer_pos().is_some_and(|h| h.distance(at) < 10.0 * z) {
                        clicked_pin = Some(pl.node);
                    }
                }
                if places.is_empty() {
                    p.text(resp.rect.center(), Align2::CENTER_CENTER, "wire in a location", FontId::proportional(11.5 * z), TEXT);
                }
                if let Some(h) = resp.hover_pos() {
                    let g = proj.unproject(h.to_vec2());
                    p.text(resp.rect.left_top() + Vec2::new(6.0, 4.0) * z, Align2::LEFT_TOP, site_text(&Site { lat: g.y(), lon: g.x(), elevation: None }), FontId::monospace(10.5 * z), TEXT);
                }
                let (nw, se) = (proj.unproject(resp.rect.left_top().to_vec2()), proj.unproject(resp.rect.right_bottom().to_vec2()));
                seen = Some((nw.y(), se.y(), se.x(), nw.x()));
                // Google's terms: its name and the copyright text, on the map, in full.
                if let Some(c) = &credit {
                    let text = p.layout(c.clone(), FontId::proportional(10.5 * z), Color32::WHITE, resp.rect.width() - 12.0 * z);
                    let at = resp.rect.right_bottom() - text.size() - Vec2::splat(6.0 * z);
                    p.rect_filled(Rect::from_min_size(at, text.size()).expand(3.0 * z), 2.0, Color32::from_black_alpha(170));
                    p.galley(at, text, Color32::WHITE);
                }
            });
        });
        if let (true, Some((n, s, e, w))) = (online, seen) {
            google.ask_view(&mut view.ask, view.memory.zoom(), n, s, e, w, &ctx);
            // No closer than the imagery here goes (past it the tiles are blank).
            if let Some(most) = google.max_zoom_at(&view.ask, (n + s) / 2.0, (e + w) / 2.0).filter(|m| view.memory.zoom() > *m) {
                let _ = view.memory.set_zoom(most);
            }
        }
        if let Some(n) = clicked_pin {
            self.pick_only(n);
        }
    }
}

/// Clamp to where Web Mercator ends.
fn merc_lat(lat: f64) -> f64 {
    lat.clamp(-85.05, 85.05)
}

/// The Blue Marble image (plate carrée) warped onto the map's projection in 3° patches.
fn paint_earth(p: &egui::Painter, proj: &Projector, tex: egui::TextureId) {
    const STEP: f64 = 3.0;
    let (rows, cols) = ((170.0 / STEP) as usize, (360.0 / STEP) as usize);
    let mut mesh = egui::epaint::Mesh::with_texture(tex);
    for r in 0..=rows {
        let lat = merc_lat(85.0 - r as f64 * STEP);
        for c in 0..=cols {
            let lon = -180.0 + c as f64 * STEP;
            let pos = proj.project(lat_lon(lat, lon)).to_pos2();
            let uv = Pos2::new(((lon + 180.0) / 360.0) as f32, ((90.0 - lat) / 180.0) as f32);
            mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
        }
    }
    let at = |r: usize, c: usize| (r * (cols + 1) + c) as u32;
    for r in 0..rows {
        for c in 0..cols {
            mesh.add_triangle(at(r, c), at(r, c + 1), at(r + 1, c + 1));
            mesh.add_triangle(at(r, c), at(r + 1, c + 1), at(r + 1, c));
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// An area's 1° cells, merged into runs per row: green where it adds, red where it subtracts.
/// A whole-world area (no evidence yet) is not drawn: it says nothing.
fn paint_area(p: &egui::Painter, proj: &Projector, area: &Area) {
    if area.add && area.cells.fraction() > 0.999 {
        return;
    }
    let tint = if area.add { Color32::from_rgba_unmultiplied(60, 200, 90, 70) } else { Color32::from_rgba_unmultiplied(220, 60, 60, 70) };
    let mut mesh = egui::epaint::Mesh::default();
    for r in 0..ROWS {
        let (top, bot) = (merc_lat(90.0 - r as f64), merc_lat(89.0 - r as f64));
        if top <= bot {
            continue;
        }
        let mut c = 0;
        while c < COLS {
            if !area.cells.get(r, c) {
                c += 1;
                continue;
            }
            let a = c;
            while c + 1 < COLS && area.cells.get(r, c + 1) {
                c += 1;
            }
            let (l, rt) = (a as f64 - 180.0, c as f64 - 179.0);
            let tl = proj.project(lat_lon(top, l)).to_pos2();
            let br = proj.project(lat_lon(bot, rt)).to_pos2();
            mesh.add_colored_rect(Rect::from_two_pos(tl, br), tint);
            c += 1;
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_lines() {
        assert_eq!(site_text(&Site { lat: 51.5074, lon: -0.1278, elevation: Some(10.0) }), "51.50740° N, 0.12780° W, 10 m");
        assert_eq!(site_text(&Site { lat: -33.8688, lon: 151.2093, elevation: None }), "33.86880° S, 151.20930° E");
        assert_eq!(frame_zoom(0.0, true), 15.0);
        assert_eq!(frame_zoom(0.0, false), 4.5);
        assert!(frame_zoom(10.0, true) < 6.0);
    }
}
