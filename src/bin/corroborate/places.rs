//! The location panel (right): every location in the project on one map, as the timeline shows
//! every time. Nothing is wired: any node's location output is on it, a SOURCER's as soon as its
//! file's metadata gives coordinates.
//!
//! - **Map**: walkers (Web Mercator, scroll to zoom, drag to pan). Detail comes from `.pmtiles`
//!   files in `maps\` next to the program (offline vector tiles, e.g. a Protomaps extract of a
//!   region; README "Maps"): nothing is fetched while working. Without them the built-in Blue
//!   Marble image is drawn instead, in the same projection. Google's satellite and road maps can
//!   be switched on (google_maps.rs): online, so offline at every start and marked while on.
//! - **Points** (a site: typed latitude and longitude, or a file's GPS) are pins in their clock
//!   group's colour; **areas** are their 1° cells, green where they add and red where they
//!   subtract.
//! - **List** under the map: one line per location; a click selects its node and centres the map
//!   on it. New points are framed by themselves (until you move the map).

use super::*;
use corroborate::data::{Data, Site};
use walkers::{lat_lon, MapMemory, PmTiles, Projector, Tiles};

/// Where map files are read from: `maps\` next to the program.
pub fn maps_dir() -> PathBuf {
    corroborate::home().join("maps")
}

/// One location on the panel.
pub struct Place {
    pub node: NodeId,
    pub name: String,
    pub site: Option<Site>,
    pub area: Arc<Area>,
    pub note: String,
    pub color: Color32,
    pub group: String,
}

/// The panel's state.
pub struct PlacesState {
    pub open: bool,
    pub memory: MapMemory,
    /// Map files found in `maps\` (path, tiles); read once, again on ↻.
    tiles: Option<Vec<(PathBuf, PmTiles)>>,
    /// The points last framed: new ones frame again.
    framed: Vec<(i64, i64)>,
    /// Row under the pointer (its pin is drawn larger).
    hover: Option<usize>,
    /// Google's map tiles, when switched on (google_maps.rs).
    pub google: google_maps::Google,
}

impl Default for PlacesState {
    fn default() -> Self {
        PlacesState { open: true, memory: MapMemory::default(), tiles: None, framed: Vec::new(), hover: None, google: Default::default() }
    }
}

/// `51.50740° N, 0.12780° W`.
pub fn site_text(s: &Site) -> String {
    let mut t = format!("{:.5}° {}, {:.5}° {}", s.lat.abs(), if s.lat >= 0.0 { "N" } else { "S" }, s.lon.abs(), if s.lon >= 0.0 { "E" } else { "W" });
    if let Some(e) = s.elevation {
        t.push_str(&format!(", {e:.0} m"));
    }
    t
}

/// The zoom that shows `span` degrees across, for points; one point gets street level when there
/// are map files, else as close as the world image is still sharp (4.5).
fn frame_zoom(span_deg: f64, detail: bool) -> f64 {
    if span_deg < 1e-4 {
        return if detail { 15.0 } else { 4.5 };
    }
    (360.0 / (span_deg * 1.6)).log2().clamp(1.0, if detail { 16.0 } else { 4.5 })
}

impl App {
    /// Every location an output of the project gives now, with its node and group colour.
    pub(crate) fn places(&self) -> Vec<Place> {
        let cg = self.canvas_groups();
        let mut out = Vec::new();
        for i in 0..self.nodes.len() {
            if self.nodes[i].hidden {
                continue;
            }
            for p in &self.graph.nodes()[i].spec.outputs {
                if p.kind != PortKind::Location || !self.gives(i, p.name) {
                    continue;
                }
                if let Ok(Data::Location { area, note, site }) = self.value_from(i, p.name) {
                    let (color, group) = match cg.of_node[i].first() {
                        Some(&g) => (cg.groups[g].color, clock_view::group_label(cg.nums[g])),
                        None => (TEXT, String::new()),
                    };
                    out.push(Place { node: i, name: self.node_name(i), site, area, note, color, group });
                }
            }
        }
        out
    }

    /// The line choosing what the map shows; with Google on, what that means.
    fn map_source_bar(&mut self, ui: &mut egui::Ui) {
        use google_maps::MapSource;
        let g = &mut self.places.google;
        let mut pick = g.source;
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            egui::ComboBox::from_id_salt("map-source").selected_text(pick.label()).show_ui(ui, |ui| {
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
            }
        });
        g.set_source(pick);
        if g.source == MapSource::Offline {
            return;
        }
        if g.key.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.colored_label(DIM, "API key");
                ui.add(egui::TextEdit::singleline(&mut g.draft).password(true).desired_width(170.0)).on_hover_text("your own key from console.cloud.google.com, with the Map Tiles API enabled; kept in settings\\ next to the program");
                if ui.small_button("Use").clicked() {
                    let typed = g.draft.clone();
                    g.use_key(&typed);
                }
            });
        }
        if !g.error.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                if ui.small_button("Try again").clicked() {
                    g.again();
                }
                if !g.key.is_empty() && ui.small_button("Change key").clicked() {
                    g.forget_key();
                }
                ui.add(egui::Label::new(egui::RichText::new(g.error.clone()).size(10.5).color(Color32::from_rgb(235, 80, 80))).truncate()).on_hover_text(g.error.clone());
            });
        }
    }

    /// The panel's contents.
    pub(crate) fn places_view(&mut self, ui: &mut egui::Ui) {
        let places = self.places();
        if self.places.tiles.is_none() {
            let mut found = Vec::new();
            if let Ok(rd) = std::fs::read_dir(maps_dir()) {
                let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pmtiles"))).collect();
                paths.sort();
                for p in paths {
                    let tiles = PmTiles::with_style(&p, walkers::Style::protomaps_dark(), ui.ctx().clone());
                    found.push((p, tiles));
                }
            }
            self.places.tiles = Some(found);
        }
        let n_maps = self.places.tiles.as_ref().map_or(0, |t| t.len());

        // Header: title, what is on it, map files.
        let mut rescan = false;
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            if ui.small_button(if self.places.open { ">" } else { "<" }).on_hover_text("show or hide the location panel").clicked() {
                self.places.open = !self.places.open;
            }
            if !self.places.open {
                return;
            }
            ui.label(egui::RichText::new("Location").strong().color(TEXT));
            let pts = places.iter().filter(|p| p.site.is_some()).count();
            ui.colored_label(DIM, format!("{pts} points, {} areas", places.len() - pts));
            ui.separator();
            let maps = if self.places.google.live() {
                "Google".to_string()
            } else if n_maps == 0 {
                "world image only".to_string()
            } else {
                format!("{n_maps} map file{}", if n_maps == 1 { "" } else { "s" })
            };
            ui.colored_label(DIM, maps).on_hover_text(format!(
                "offline map files (.pmtiles) are read from {}\nnothing is fetched from the internet; see README, Maps",
                maps_dir().display()
            ));
            rescan = ui.small_button("↻").on_hover_text("read the maps folder again").clicked();
        });
        if rescan {
            self.places.tiles = None;
        }
        if !self.places.open {
            return;
        }
        let ctx = ui.ctx().clone();
        self.places.google.tick(&ctx);
        self.map_source_bar(ui);
        let online = self.places.google.live();

        // Frame new points (a point arriving, or more of them).
        let pts: Vec<(f64, f64)> = places.iter().filter_map(|p| p.site.map(|s| (s.lat, s.lon))).collect();
        let key: Vec<(i64, i64)> = pts.iter().map(|&(a, b)| ((a * 1e5) as i64, (b * 1e5) as i64)).collect();
        if !pts.is_empty() && key != self.places.framed {
            let (mut lo, mut hi) = ((90.0f64, 180.0f64), (-90.0f64, -180.0f64));
            for &(a, b) in &pts {
                lo = (lo.0.min(a), lo.1.min(b));
                hi = (hi.0.max(a), hi.1.max(b));
            }
            self.places.memory.center_at(lat_lon((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0));
            let _ = self.places.memory.set_zoom(frame_zoom((hi.0 - lo.0).max(hi.1 - lo.1), n_maps > 0 || online));
            self.places.framed = key;
        }

        let list_h = (22.0 + 30.0 * places.len() as f32).clamp(40.0, ui.available_height() * 0.4);
        let map_h = (ui.available_height() - list_h).max(120.0);
        let earth = self.earth.as_ref().map(|t| t.id());
        let hover = self.places.hover;
        let mut clicked_pin: Option<NodeId> = None;
        let credit = self.places.google.credit();
        // The view on screen: north, south, east, west.
        let mut seen: Option<(f64, f64, f64, f64)> = None;
        ui.allocate_ui(Vec2::new(ui.available_width(), map_h), |ui| {
            let PlacesState { tiles, google, memory, .. } = &mut self.places;
            let tiles = tiles.as_mut().expect("read above");
            let (base, layers): (Option<&mut dyn Tiles>, &mut [(PathBuf, PmTiles)]) = match (google.tiles_mut(), tiles.split_first_mut()) {
                (Some(g), _) => (Some(g as &mut dyn Tiles), &mut []),
                (None, Some((first, rest))) => (Some(&mut first.1 as &mut dyn Tiles), rest),
                (None, None) => (None, &mut []),
            };
            let detail = base.is_some();
            let mut map = walkers::Map::new(base, memory, lat_lon(20.0, 0.0));
            for l in layers.iter_mut() {
                map = map.with_layer(&mut l.1, 1.0);
            }
            map.show(ui, |ui, resp, proj, _mem| {
                let p = ui.painter().with_clip_rect(resp.rect);
                if !detail {
                    p.rect_filled(resp.rect, 0.0, BG);
                    if let Some(tex) = earth {
                        paint_earth(&p, proj, tex);
                    }
                }
                // Areas: only for locations without a point (a point's 1° cell would read as
                // uncertainty it does not have).
                for pl in places.iter().filter(|pl| pl.site.is_none()) {
                    paint_area(&p, proj, &pl.area);
                }
                for (k, pl) in places.iter().enumerate() {
                    let Some(s) = pl.site else { continue };
                    let at = proj.project(lat_lon(s.lat, s.lon)).to_pos2();
                    let big = hover == Some(k);
                    p.circle_filled(at, if big { 8.0 } else { 6.0 }, pl.color);
                    p.circle_stroke(at, if big { 8.0 } else { 6.0 }, Stroke::new(1.5, Color32::WHITE));
                    p.text(at + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, format!("{} {}", pl.group, pl.name).trim(), FontId::proportional(11.5), Color32::WHITE);
                    if resp.clicked() && resp.interact_pointer_pos().is_some_and(|h| h.distance(at) < 10.0) {
                        clicked_pin = Some(pl.node);
                    }
                }
                if places.is_empty() {
                    p.text(resp.rect.center(), Align2::CENTER_CENTER, "no locations yet: a file with GPS in its metadata, a typed place, ENF…", FontId::proportional(11.5), TEXT);
                }
                if !detail {
                    p.text(resp.rect.left_bottom() + Vec2::new(6.0, -4.0), Align2::LEFT_BOTTOM, "world image (NASA Blue Marble); street detail: .pmtiles in maps\\", FontId::proportional(10.0), DIM);
                }
                if let Some(h) = resp.hover_pos() {
                    let g = proj.unproject(h.to_vec2());
                    p.text(resp.rect.left_top() + Vec2::new(6.0, 4.0), Align2::LEFT_TOP, site_text(&Site { lat: g.y(), lon: g.x(), elevation: None }), FontId::monospace(10.5), TEXT);
                }
                let (nw, se) = (proj.unproject(resp.rect.left_top().to_vec2()), proj.unproject(resp.rect.right_bottom().to_vec2()));
                seen = Some((nw.y(), se.y(), se.x(), nw.x()));
                // Google's terms: its name and the copyright text, on the map, in full.
                if let Some(c) = &credit {
                    let text = p.layout(c.clone(), FontId::proportional(10.5), Color32::WHITE, resp.rect.width() - 12.0);
                    let at = resp.rect.right_bottom() - text.size() - Vec2::splat(6.0);
                    p.rect_filled(Rect::from_min_size(at, text.size()).expand(3.0), 2.0, Color32::from_black_alpha(170));
                    p.galley(at, text, Color32::WHITE);
                }
            });
        });
        if let (true, Some((n, s, e, w))) = (online, seen) {
            let g = &mut self.places.google;
            g.ask_view(self.places.memory.zoom(), n, s, e, w, &ctx);
            // No closer than the imagery here goes (past it the tiles are blank).
            if let Some(most) = g.max_zoom_at((n + s) / 2.0, (e + w) / 2.0).filter(|m| self.places.memory.zoom() > *m) {
                let _ = self.places.memory.set_zoom(most);
            }
        }

        // The list: one line per location.
        let mut pick: Option<(NodeId, Option<Site>)> = clicked_pin.map(|n| (n, None));
        self.places.hover = None;
        egui::ScrollArea::vertical().id_salt("places-list").auto_shrink([false, false]).show(ui, |ui| {
            for (k, pl) in places.iter().enumerate() {
                let what = match pl.site {
                    Some(s) => site_text(&s),
                    None => format!("{} · {}", if pl.area.add { "area" } else { "not here" }, map_node::area_status(&pl.area)),
                };
                let r = ui
                    .horizontal(|ui| {
                        ui.colored_label(pl.color, if pl.group.is_empty() { "·".to_string() } else { pl.group.clone() });
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&pl.name).size(11.5).color(TEXT));
                            ui.label(egui::RichText::new(&what).size(10.0).monospace().color(DIM));
                        });
                    })
                    .response
                    .interact(Sense::click())
                    .on_hover_text(if pl.note.is_empty() { "click to select its node".to_string() } else { format!("{}\nclick to select its node", pl.note) });
                if r.hovered() {
                    self.places.hover = Some(k);
                }
                if r.clicked() {
                    pick = Some((pl.node, pl.site));
                }
            }
        });
        if let Some((n, site)) = pick {
            self.pick_only(n);
            if let Some(s) = site {
                self.places.memory.center_at(lat_lon(s.lat, s.lon));
            }
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
