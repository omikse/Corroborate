//! The Map node in the workbench: the world map (Robinson projection over NASA Blue Marble, with
//! Natural Earth coastlines) drawn on the canvas and in its window. Other nodes that show an area
//! (Result record) draw it with `draw`. Compiled into the `corroborate` program.

use super::*;

/// Width of the Map node on the canvas; its map is drawn below its port.
pub const NODE_W: f32 = 320.0;

const KEPT: Color32 = Color32::from_rgba_premultiplied(30, 110, 40, 140);
const CUT: Color32 = Color32::from_rgba_premultiplied(120, 20, 20, 140);

/// Extra height the map takes on the canvas, below the ports.
pub fn canvas_extra() -> f32 {
    NODE_W / ROBINSON_ASPECT as f32 + 8.0
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

pub fn paint(p: &egui::Painter, rect: Rect, earth: Option<egui::TextureId>, area: Option<&Area>) {
    let at = |lat: f64, lon: f64| {
        let (x, y) = robinson(lat, lon);
        Pos2::new(rect.left() + x as f32 * rect.width(), rect.top() + y as f32 * rect.height())
    };
    // The Earth image, warped from its plate carrée grid onto the Robinson shape in 2° × 4° patches.
    match earth {
        Some(tex) => {
            let mut img = egui::epaint::Mesh::with_texture(tex);
            for r in 0..90 {
                let (t, b) = (90.0 - 2.0 * r as f64, 88.0 - 2.0 * r as f64);
                for c in 0..90 {
                    let (l, rt) = (-180.0 + 4.0 * c as f64, -176.0 + 4.0 * c as f64);
                    let i = img.vertices.len() as u32;
                    for (lat, lon) in [(t, l), (t, rt), (b, rt), (b, l)] {
                        let uv = Pos2::new(((lon + 180.0) / 360.0) as f32, ((90.0 - lat) / 180.0) as f32);
                        img.vertices.push(egui::epaint::Vertex { pos: at(lat, lon), uv, color: Color32::WHITE });
                    }
                    img.add_triangle(i, i + 1, i + 2);
                    img.add_triangle(i, i + 2, i + 3);
                }
            }
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
    let grat = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0, 0, 0, 90));
    for k in -6i32..=6 {
        let lon = k as f64 * 30.0;
        let pts: Vec<Pos2> = (0..=36).map(|j| at(90.0 - j as f64 * 5.0, lon)).collect();
        p.add(egui::Shape::line(pts, if k.abs() == 6 { Stroke::new(1.0, LINE) } else { grat }));
    }
    for k in -2..=2 {
        let lat = k as f64 * 30.0;
        p.line_segment([at(lat, -180.0), at(lat, 180.0)], grat);
    }
}

/// NASA Blue Marble (public domain), 2048 × 1024 plate carrée, stored as a 256-colour palette
/// followed by one palette index per pixel.
pub fn load_earth(ctx: &egui::Context) -> TextureHandle {
    const W: usize = 2048;
    const H: usize = 1024;
    let raw: &[u8] = include_bytes!("earth.idx");
    let (pal, idx) = raw.split_at(768);
    let mut px = Vec::with_capacity(W * H * 4);
    for &i in &idx[..W * H] {
        let k = i as usize * 3;
        px.extend_from_slice(&[pal[k], pal[k + 1], pal[k + 2], 255]);
    }
    ctx.load_texture("earth", ColorImage::from_rgba_unmultiplied([W, H], &px), TextureOptions::LINEAR)
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

    /// The map drawn inside the node on the canvas, below its port.
    pub(crate) fn map_on_canvas(&self, painter: &egui::Painter, rect: Rect, i: NodeId) {
        let top = rect.top() + (HEAD + ROW + 8.0) * self.zoom;
        let w = rect.width() - 12.0 * self.zoom;
        let map = Rect::from_min_size(Pos2::new(rect.left() + 6.0 * self.zoom, top), Vec2::new(w, w / ROBINSON_ASPECT as f32));
        let area = self.map_input(i).and_then(|r| r.ok()).map(|(a, _)| a);
        paint(&painter.with_clip_rect(map), map, self.earth.as_ref().map(|t| t.id()), area.as_deref());
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
