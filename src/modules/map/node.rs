//! The Map node in the workbench: the world map (Robinson projection over NASA Blue Marble, with
//! Natural Earth coastlines) drawn on the canvas and in its window. Other nodes that show an area
//! (Result record) draw it with `draw`. Compiled into the `corroborate` program.

use super::*;

/// Width of the Map node on the canvas; its map is drawn below its port.
const NODE_W: f32 = 320.0;

const KEPT: Color32 = Color32::from_rgba_premultiplied(30, 110, 40, 140);
const CUT: Color32 = Color32::from_rgba_premultiplied(120, 20, 20, 140);

/// The Map node on the canvas: wider, with the map drawn inside it, below its port.
pub struct Look;

impl crate::look::NodeLook for Look {
    fn size(&self, _app: &App, _i: NodeId, standard: Vec2) -> Vec2 {
        Vec2::new(NODE_W, standard.y + NODE_W / ROBINSON_ASPECT as f32 + 8.0)
    }

    fn body(&self, app: &mut App, ui: &mut egui::Ui, i: NodeId) {
        let r = ui.max_rect();
        let map = Rect::from_min_size(r.min, Vec2::new(r.width(), r.width() / ROBINSON_ASPECT as f32));
        let area = app.map_input(i).and_then(|r| r.ok()).map(|(a, _)| a);
        paint(&ui.painter().with_clip_rect(map), map, app.earth.as_ref().map(|t| t.id()), area.as_deref());
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

/// A world map in the Robinson projection. Green: cells the area keeps (where it adds
/// possibility). Red: cells it rules out (where it subtracts). Grey: no area yet.
pub fn draw(ui: &mut egui::Ui, earth: Option<egui::TextureId>, area: Option<&Area>) {
    let w = ui.available_width().clamp(200.0, 1400.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, w / ROBINSON_ASPECT as f32), Sense::hover());
    let p = ui.painter_at(rect);
    paint(&p, rect, earth, area);
    let at = |lat: f64, lon: f64| {
        let (x, y) = robinson(lat, lon);
        Pos2::new(rect.left() + x as f32 * rect.width(), rect.top() + y as f32 * rect.height())
    };
    if let Some(h) = resp.hover_pos() {
        // The nearest cell centre under the pointer.
        let mut best = (f32::MAX, 0.0, 0.0);
        for r in 0..ROWS {
            for c in (0..COLS).step_by(2) {
                let (lat, lon) = (89.5 - r as f64, c as f64 - 179.5);
                let d = at(lat, lon).distance_sq(h);
                if d < best.0 {
                    best = (d, lat, lon);
                }
            }
        }
        let txt = format!("{:.1}°, {:.1}°", best.1, best.2);
        p.text(rect.left_top() + Vec2::new(6.0, 4.0), Align2::LEFT_TOP, txt, FontId::proportional(11.0), TEXT);
    }
    ui.colored_label(
        DIM,
        "green: an area that adds (the place is here) · red: an area that subtracts (not here) · Robinson projection, 1° cells, lines every 30° · image: NASA Blue Marble (public domain) · coastlines: Natural Earth 1:110m",
    );
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

/// The Earth image warped from its plate carrée grid onto the Robinson shape in 2° × 4° patches:
/// vertex positions (0..1 across the map) and texture positions, worked out once.
fn earth_patches() -> &'static (Vec<(Pos2, Pos2)>, Vec<u32>) {
    static P: std::sync::OnceLock<(Vec<(Pos2, Pos2)>, Vec<u32>)> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let (mut verts, mut idx) = (Vec::new(), Vec::new());
        for r in 0..90 {
            let (t, b) = (90.0 - 2.0 * r as f64, 88.0 - 2.0 * r as f64);
            for c in 0..90 {
                let (l, rt) = (-180.0 + 4.0 * c as f64, -176.0 + 4.0 * c as f64);
                let i = verts.len() as u32;
                for (lat, lon) in [(t, l), (t, rt), (b, rt), (b, l)] {
                    let (x, y) = robinson(lat, lon);
                    let uv = Pos2::new(((lon + 180.0) / 360.0) as f32, ((90.0 - lat) / 180.0) as f32);
                    verts.push((Pos2::new(x as f32, y as f32), uv));
                }
                idx.extend([i, i + 1, i + 2, i, i + 2, i + 3]);
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

impl App {
    /// The area wired into a Map node, with how it was reached.
    fn map_input(&self, i: NodeId) -> Option<Result<(Arc<Area>, String), String>> {
        self.graph.feeder(i, "location").map(|(f, _)| self.area_from(f))
    }

    pub(crate) fn map_status(&self, i: NodeId) -> String {
        match self.map_input(i) {
            Some(Ok((a, _))) => area_status(&a),
            Some(Err(e)) => e,
            None => "no location wired".into(),
        }
    }

    pub(crate) fn map_window(&mut self, ui: &mut egui::Ui, i: NodeId) {
        let earth = self.earth.as_ref().map(|t| t.id());
        match self.graph.feeder(i, "location") {
            None => {
                ui.colored_label(DIM, "wire a location into this node: ENF, Manual data, Combine, Invert or the Result record");
                draw(ui, earth, None);
            }
            Some((from, _)) => match self.area_from(from) {
                Ok((a, note)) => {
                    let lab = &self.nodes[from].label;
                    let al = a.allowed();
                    ui.label(format!(
                        "{lab}: {} · {:.2} % of Earth still possible ({:.1} million km²)",
                        if a.add { "adds (green: here)" } else { "subtracts (red: not here)" },
                        al.fraction() * 100.0,
                        al.area_km2() / 1e6
                    ));
                    ui.colored_label(if note.contains("CONTRADICTION") { WARN } else { DIM }, note);
                    draw(ui, earth, Some(&a));
                }
                Err(e) => {
                    ui.colored_label(DIM, format!("{}: {e}", self.nodes[from].label));
                    draw(ui, earth, None);
                }
            },
        }
    }
}
