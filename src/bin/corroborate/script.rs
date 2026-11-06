//! look.rhai: a module's look as a script, beside its module.toml. It only draws: it cannot add,
//! remove or wire ports, read files or reach the network, and the data stays with processing.
//! Both functions are optional; `node` describes the node (below).
//!
//! ```rhai
//! // Port placement and the frame, again whenever the node changes.
//! fn layout(node) {
//!     #{
//!         size: [260, 140],                 // unzoomed px
//!         fill: "#2a3038", radius: 10, status: false,
//!         ports: #{ audio: #{ side: "bottom", at: 40, label: "outside",
//!                             dot_size: 6, hidden: node.settings.mode == "off" } },
//!     }
//! }
//!
//! // What is inside the node (below the title), in px from the body's top left.
//! fn draw(node) {
//!     [
//!         text(8, 4, "level " + node.settings.level, #{ size: 12, color: "#c8ccd2" }),
//!         rect(8, 24, 100, 6, #{ fill: "#3a4048", radius: 3 }),
//!         circle(8, 40, 4, #{ fill: "#78aaff" }),
//!         line(0, 50, node.width, 50, #{ color: "#444", width: 1 }),
//!         field("label", 8, 60, 150),       // a text box for a setting (checked as always)
//!         checkbox("loud", 8, 84),
//!         dropdown("mode", 8, 104, 120),
//!         view("map", 160, 4, 100, 60),     // the location wired in, on a world map
//!         native(),                         // the module's own Rust drawing, if it has one
//!     ]
//!     // or #{ items: [...], redraw: 0.5 } to be drawn again every 0.5 s
//! }
//! ```
//!
//! `node`: `label`, `collapsed`, `width`, `height` (the body, unzoomed), `settings` (name → value),
//! `inputs` and `outputs` (each `name`, `kind`, `wired`; outputs also `value`, a line about what
//! they hold, or `()`, and `on`: whether the module gives it now), `status`, `running`, `time` (seconds; read it only with `redraw`).
//!
//! **Speed:** a script runs when what it reads changes (settings, wires, status, outputs,
//! collapse), or at the `redraw` interval it asks for; its result is kept and drawn natively, so
//! panning and zooming never run a script, and nodes off screen are skipped. Each call may take
//! 100 000 operations; a script that fails (or never ends) shows its error on its own node.

use super::*;
use corroborate::module_manifest::{parse_color, FrameLook, PortLabel, PortLook, Side};
use rhai::{Array, Dynamic, Engine, ImmutableString, Map, Scope, AST};
use std::hash::{Hash, Hasher};

/// A module's script, compiled.
struct Compiled {
    text: String,
    ast: Result<AST, String>,
    layout: bool,
    draw: bool,
}

/// Something a script draws, in unzoomed px from the body's top left.
#[derive(Clone, Debug)]
pub enum Item {
    Text { at: Vec2, text: String, size: f32, color: Color32, align: Align2 },
    Rect { min: Vec2, size: Vec2, fill: Option<Color32>, stroke: Option<Stroke>, radius: f32 },
    Circle { c: Vec2, r: f32, fill: Option<Color32>, stroke: Option<Stroke> },
    Line { a: Vec2, b: Vec2, stroke: Stroke },
    Field { setting: String, min: Vec2, w: f32 },
    Checkbox { setting: String, min: Vec2 },
    Dropdown { setting: String, min: Vec2, w: f32 },
    View { name: String, min: Vec2, size: Vec2 },
    Native,
}

/// What a node's script last gave, and what it was given.
#[derive(Clone, Default)]
pub struct NodeOut {
    key: u64,
    at: f64,
    redraw: Option<f64>,
    pub frame: Option<FrameLook>,
    pub ports: Vec<(String, PortLook)>,
    pub items: Option<Vec<Item>>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Scripts {
    engine: Option<Engine>,
    by_module: HashMap<&'static str, Compiled>,
    pub by_node: HashMap<NodeId, NodeOut>,
    /// Script calls made and the time they took (tests and benchmarks read these).
    pub calls: u64,
    pub time: Duration,
}

fn num(d: &Dynamic) -> f32 {
    d.as_float().map(|f| f as f32).or_else(|_| d.as_int().map(|i| i as f32)).unwrap_or(0.0)
}

fn colour(d: Option<&Dynamic>) -> Option<Color32> {
    let s = d?.clone().into_immutable_string().ok()?;
    parse_color(&s).map(|[r, g, b]| Color32::from_rgb(r, g, b))
}

fn item(kind: &str, fields: &[(&str, Dynamic)]) -> Map {
    let mut m = Map::new();
    m.insert("t".into(), kind.into());
    for (k, v) in fields {
        m.insert((*k).into(), v.clone());
    }
    m
}

fn engine() -> Engine {
    let mut e = Engine::new();
    e.set_max_operations(100_000);
    e.set_max_call_levels(32);
    e.set_max_expr_depths(64, 32);
    e.set_max_string_size(10_000);
    e.set_max_array_size(2_000);
    e.set_max_map_size(500);
    e.on_print(|_| {});
    e.on_debug(|_, _, _| {});
    type D = Dynamic;
    e.register_fn("text", |x: D, y: D, s: ImmutableString| item("text", &[("x", x), ("y", y), ("text", s.into())]));
    e.register_fn("text", |x: D, y: D, s: ImmutableString, o: Map| item("text", &[("x", x), ("y", y), ("text", s.into()), ("o", o.into())]));
    e.register_fn("rect", |x: D, y: D, w: D, h: D, o: Map| item("rect", &[("x", x), ("y", y), ("w", w), ("h", h), ("o", o.into())]));
    e.register_fn("circle", |x: D, y: D, r: D, o: Map| item("circle", &[("x", x), ("y", y), ("r", r), ("o", o.into())]));
    e.register_fn("line", |x1: D, y1: D, x2: D, y2: D, o: Map| item("line", &[("x", x1), ("y", y1), ("x2", x2), ("y2", y2), ("o", o.into())]));
    e.register_fn("field", |s: ImmutableString, x: D, y: D, w: D| item("field", &[("s", s.into()), ("x", x), ("y", y), ("w", w)]));
    e.register_fn("checkbox", |s: ImmutableString, x: D, y: D| item("checkbox", &[("s", s.into()), ("x", x), ("y", y)]));
    e.register_fn("dropdown", |s: ImmutableString, x: D, y: D, w: D| item("dropdown", &[("s", s.into()), ("x", x), ("y", y), ("w", w)]));
    e.register_fn("view", |s: ImmutableString, x: D, y: D, w: D, h: D| item("view", &[("s", s.into()), ("x", x), ("y", y), ("w", w), ("h", h)]));
    e.register_fn("native", || item("native", &[]));
    e
}

fn items_of(d: Dynamic) -> Result<(Vec<Item>, Option<f64>), String> {
    let (list, redraw) = if d.is_map() {
        let m = d.cast::<Map>();
        let redraw = m.get("redraw").map(|r| num(r) as f64).filter(|r| *r > 0.0).map(|r| r.max(0.05));
        (m.get("items").cloned().unwrap_or_default().try_cast::<Array>().unwrap_or_default(), redraw)
    } else if d.is_array() {
        (d.cast::<Array>(), None)
    } else if d.is_unit() {
        (Vec::new(), None)
    } else {
        return Err(format!("draw() gave {}, not a list of items", d.type_name()));
    };
    let mut out = Vec::new();
    for x in list {
        let Some(m) = x.try_cast::<Map>() else { return Err("draw(): every item must come from text(), rect(), circle(), line(), field(), checkbox(), dropdown(), view() or native()".into()) };
        let g = |k: &str| m.get(k).map(num).unwrap_or(0.0);
        let s = |k: &str| m.get(k).and_then(|v| v.clone().into_immutable_string().ok()).map(|v| v.to_string()).unwrap_or_default();
        let o = m.get("o").and_then(|v| v.clone().try_cast::<Map>()).unwrap_or_default();
        let on = |k: &str| o.get(k).map(num);
        let stroke = || colour(o.get("stroke")).map(|c| Stroke::new(on("stroke_width").unwrap_or(1.0), c));
        let at = Vec2::new(g("x"), g("y"));
        out.push(match s("t").as_str() {
            "text" => Item::Text {
                at,
                text: s("text"),
                size: on("size").unwrap_or(11.5),
                color: colour(o.get("color")).unwrap_or(TEXT),
                align: match o.get("align").and_then(|v| v.clone().into_immutable_string().ok()).as_deref() {
                    Some("center") => Align2::CENTER_TOP,
                    Some("right") => Align2::RIGHT_TOP,
                    _ => Align2::LEFT_TOP,
                },
            },
            "rect" => Item::Rect { min: at, size: Vec2::new(g("w"), g("h")), fill: colour(o.get("fill")), stroke: stroke(), radius: on("radius").unwrap_or(0.0) },
            "circle" => Item::Circle { c: at, r: g("r"), fill: colour(o.get("fill")), stroke: stroke() },
            "line" => Item::Line { a: at, b: Vec2::new(g("x2"), g("y2")), stroke: Stroke::new(on("width").unwrap_or(1.0), colour(o.get("color")).unwrap_or(LINE)) },
            "field" => Item::Field { setting: s("s"), min: at, w: g("w") },
            "checkbox" => Item::Checkbox { setting: s("s"), min: at },
            "dropdown" => Item::Dropdown { setting: s("s"), min: at, w: g("w") },
            "view" => Item::View { name: s("s"), min: at, size: Vec2::new(g("w"), g("h")) },
            "native" => Item::Native,
            other => return Err(format!("draw(): unknown item \"{other}\"")),
        });
    }
    Ok((out, redraw))
}

fn layout_of(d: Dynamic) -> Result<(FrameLook, Vec<(String, PortLook)>), String> {
    if d.is_unit() {
        return Ok(Default::default());
    }
    let m = d.try_cast::<Map>().ok_or("layout() must give a map like #{ size: [w, h], ports: #{…} }")?;
    let word = |p: &Map, k: &str| p.get(k).and_then(|v| v.clone().into_immutable_string().ok()).map(|v| v.to_string());
    let size = match m.get("size").and_then(|v| v.clone().try_cast::<Array>()) {
        Some(a) if a.len() == 2 && num(&a[0]) > 0.0 && num(&a[1]) > 0.0 => Some((num(&a[0]), num(&a[1]))),
        Some(_) => return Err("layout(): size must be [width, height]".into()),
        None => None,
    };
    let frame = FrameLook {
        size,
        fill: word(&m, "fill").and_then(|c| parse_color(&c)),
        radius: m.get("radius").map(num),
        status: m.get("status").and_then(|v| v.as_bool().ok()),
    };
    let mut ports = Vec::new();
    for (name, p) in m.get("ports").and_then(|v| v.clone().try_cast::<Map>()).unwrap_or_default() {
        let p = p.try_cast::<Map>().ok_or(format!("layout(): ports.{name} must be a map"))?;
        ports.push((
            name.to_string(),
            PortLook {
                side: word(&p, "side").and_then(|s| Side::parse(&s)),
                at: p.get("at").map(num),
                label: match word(&p, "label").as_deref() {
                    Some("inside") => Some(PortLabel::Inside),
                    Some("outside") => Some(PortLabel::Outside),
                    Some("hidden") => Some(PortLabel::Hidden),
                    _ => None,
                },
                dot_size: p.get("dot_size").map(num),
                hidden: p.get("hidden").and_then(|v| v.as_bool().ok()).unwrap_or(false),
            },
        ));
    }
    Ok((frame, ports))
}

impl App {
    /// Compile the scripts that are new or changed (called with the manifests' refresh, so a
    /// look.rhai saved in the modules folder shows within a second).
    pub(crate) fn reload_scripts(&mut self) {
        let sc = &mut self.scripts;
        let engine = sc.engine.get_or_insert_with(engine);
        let mut changed = false;
        for (id, m) in &self.looks {
            let Some(text) = &m.script else {
                changed |= sc.by_module.remove(id).is_some();
                continue;
            };
            if sc.by_module.get(id).is_some_and(|c| c.text == *text) {
                continue;
            }
            let ast = engine.compile(text).map_err(|e| format!("look.rhai: {e}"));
            let has = |f: &str| ast.as_ref().is_ok_and(|a| a.iter_functions().any(|x| x.name == f && x.params.len() == 1));
            let (layout, draw) = (has("layout"), has("draw"));
            sc.by_module.insert(id, Compiled { text: text.clone(), ast, layout, draw });
            changed = true;
        }
        if changed {
            sc.by_node.clear();
        }
    }

    /// What a node's script reads: it runs again only when this changes.
    fn script_input(&self, i: NodeId, body: Vec2, t: f64) -> (u64, Map) {
        let n = &self.nodes[i];
        let spec = &self.graph.nodes()[i].spec;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut node = Map::new();
        node.insert("label".into(), n.label.clone().into());
        node.insert("collapsed".into(), n.collapsed.into());
        node.insert("width".into(), (body.x as f64).into());
        node.insert("height".into(), (body.y as f64).into());
        node.insert("time".into(), t.into());
        (&n.label, n.collapsed, body.x.round() as i64, body.y.round() as i64).hash(&mut h);
        let mut settings = Map::new();
        for (k, v) in n.kind.settings() {
            (&k, &v).hash(&mut h);
            settings.insert(k.into(), v.into());
        }
        node.insert("settings".into(), settings.into());
        let (status, running, rev) = {
            let s = n.state.lock().unwrap();
            let st = match &n.kind {
                Kind::Plugin { .. } | Kind::Module { .. } => plugin_node::status(&s).unwrap_or_else(|| s.status.clone()),
                _ => s.status.clone(),
            };
            (st, s.running, s.outbox_rev)
        };
        (&status, running, rev).hash(&mut h);
        node.insert("status".into(), status.into());
        node.insert("running".into(), running.into());
        let port = |p: &corroborate::core::Port, wired: bool| {
            let mut m = Map::new();
            m.insert("name".into(), p.name.into());
            m.insert("kind".into(), corroborate::module_manifest::kind_id(p.kind).into());
            m.insert("wired".into(), wired.into());
            m
        };
        let ins: Array = spec
            .inputs
            .iter()
            .map(|p| {
                let w = self.graph.feeder(i, p.name).is_some();
                (p.name, w).hash(&mut h);
                port(p, w).into()
            })
            .collect();
        let s = n.state.lock().unwrap();
        let outs: Array = spec
            .outputs
            .iter()
            .map(|p| {
                let w = !self.graph.targets(i, p.name).is_empty();
                let on = self.gives(i, p.name);
                (p.name, w, on).hash(&mut h);
                let mut m = port(p, w);
                m.insert("on".into(), on.into());
                let v = s.outbox.iter().find(|(o, _)| *o == p.name).map_or(Dynamic::UNIT, |(_, d)| bus::summary(d).into());
                m.insert("value".into(), v);
                m.into()
            })
            .collect();
        node.insert("inputs".into(), ins.into());
        node.insert("outputs".into(), outs.into());
        (h.finish(), node)
    }

    /// Run the scripts of the nodes on screen whose input changed (or whose redraw is due).
    pub(crate) fn run_scripts(&mut self, origin: Pos2, area: Rect, t: f64) {
        if self.scripts.by_module.is_empty() {
            return;
        }
        for i in 0..self.nodes.len() {
            if self.nodes[i].hidden {
                continue;
            }
            let id = self.graph.nodes()[i].spec.name;
            let Some(c) = self.scripts.by_module.get(id) else { continue };
            let (has_layout, has_draw) = (c.layout, c.draw);
            let rect = self.node_rect(origin, i);
            if !rect.intersects(area.expand(200.0)) {
                continue;
            }
            let body = (self.body_rect(rect).size() / self.zoom).max(Vec2::ZERO);
            let (key, node) = self.script_input(i, body, t);
            let prev = self.scripts.by_node.get(&i);
            let due = prev.is_some_and(|p| p.redraw.is_some_and(|r| t - p.at >= r));
            if prev.is_some_and(|p| p.key == key) && !due {
                continue;
            }
            let sc = &self.scripts;
            let c = &sc.by_module[id];
            let mut out = NodeOut { key, at: t, ..Default::default() };
            let mut calls = 0;
            let started = Instant::now();
            match &c.ast {
                Err(e) => out.error = Some(e.clone()),
                Ok(ast) => {
                    let engine = sc.engine.as_ref().expect("made with the scripts");
                    let mut call = |f: &str| {
                        calls += 1;
                        engine.call_fn::<Dynamic>(&mut Scope::new(), ast, f, (Dynamic::from_map(node.clone()),)).map_err(|e| format!("look.rhai {f}(): {e}"))
                    };
                    if has_layout {
                        match call("layout").and_then(layout_of) {
                            Ok((f, p)) => {
                                out.frame = Some(f);
                                out.ports = p;
                            }
                            Err(e) => out.error = Some(e),
                        }
                    }
                    if has_draw && out.error.is_none() {
                        match call("draw").and_then(items_of) {
                            Ok((items, redraw)) => {
                                out.items = Some(items);
                                out.redraw = redraw;
                            }
                            Err(e) => out.error = Some(e),
                        }
                    }
                }
            }
            let sc = &mut self.scripts;
            sc.calls += calls;
            sc.time += started.elapsed();
            sc.by_node.insert(i, out);
        }
    }

    /// What node `i`'s script says about its frame.
    pub(crate) fn script_frame(&self, i: NodeId) -> Option<FrameLook> {
        self.scripts.by_node.get(&i)?.frame.clone()
    }

    /// What node `i`'s script says about port `name`.
    pub(crate) fn script_port(&self, i: NodeId, name: &str) -> Option<PortLook> {
        self.scripts.by_node.get(&i)?.ports.iter().find(|(n, _)| n == name).map(|(_, l)| l.clone())
    }

    /// Draw node `i`'s body from its script. False when it has no draw() (the Rust look draws).
    pub(crate) fn draw_script(&mut self, ui: &mut egui::Ui, i: NodeId, body: Rect) -> bool {
        let Some(out) = self.scripts.by_node.get(&i) else { return false };
        if let Some(e) = &out.error {
            let e = e.clone();
            ui.painter().text(body.min, Align2::LEFT_TOP, wrap(&e, 46), FontId::proportional(10.5 * self.zoom), WARN);
            return true;
        }
        let Some(items) = out.items.clone() else { return false };
        let z = self.zoom;
        let at = |v: Vec2| body.min + v * z;
        let p = ui.painter().clone();
        for it in items {
            match it {
                Item::Text { at: a, text, size, color, align } => {
                    p.text(at(a), align, text, FontId::proportional(size * z), color);
                }
                Item::Rect { min, size, fill, stroke, radius } => {
                    let r = Rect::from_min_size(at(min), size * z);
                    if let Some(f) = fill {
                        p.rect_filled(r, radius * z, f);
                    }
                    if let Some(s) = stroke {
                        p.rect_stroke(r, radius * z, Stroke::new(s.width * z, s.color), StrokeKind::Inside);
                    }
                }
                Item::Circle { c, r, fill, stroke } => {
                    if let Some(f) = fill {
                        p.circle_filled(at(c), r * z, f);
                    }
                    if let Some(s) = stroke {
                        p.circle_stroke(at(c), r * z, Stroke::new(s.width * z, s.color));
                    }
                }
                Item::Line { a, b, stroke } => {
                    p.line_segment([at(a), at(b)], Stroke::new(stroke.width * z, stroke.color));
                }
                Item::Field { setting, min, w } => self.setting_widget(ui, i, &setting, Rect::from_min_size(at(min), Vec2::new(w, 18.0) * z), 0),
                Item::Checkbox { setting, min } => self.setting_widget(ui, i, &setting, Rect::from_min_size(at(min), Vec2::new(18.0, 18.0) * z), 1),
                Item::Dropdown { setting, min, w } => self.setting_widget(ui, i, &setting, Rect::from_min_size(at(min), Vec2::new(w, 18.0) * z), 2),
                Item::View { name, min, size } => {
                    let r = Rect::from_min_size(at(min), size * z);
                    match name.as_str() {
                        "map" => {
                            let area = self.view_area(i);
                            map_node::paint(&p, r, self.earth.as_ref().map(|t| t.id()), area.as_deref());
                        }
                        other => {
                            p.text(r.min, Align2::LEFT_TOP, format!("no view \"{other}\" (there is: map)"), FontId::proportional(10.5 * z), WARN);
                        }
                    }
                }
                Item::Native => look::of(&self.nodes[i].kind).body(self, ui, i),
            }
        }
        true
    }

    /// Whether node `i` gives output `port` now (its `on` in look.rhai): a SOURCER gives nothing
    /// until a source is chosen and video only for sources with pictures; others give every one.
    pub(crate) fn gives(&self, i: NodeId, port: &str) -> bool {
        match &self.nodes[i].kind {
            Kind::Sourcer { input } => corroborate::modules::sourcer::gives(input, port),
            _ => true,
        }
    }

    /// The location a node's map view shows: what is wired into its first location input, else
    /// its own.
    fn view_area(&self, i: NodeId) -> Option<Arc<Area>> {
        let spec = &self.graph.nodes()[i].spec;
        let wired = spec.inputs.iter().filter(|p| p.kind == PortKind::Location).find_map(|p| self.graph.feeder(i, p.name));
        match wired {
            Some((f, _)) => self.area_from(f).ok().map(|(a, _)| a),
            None => self.area_from(i).ok().map(|(a, _)| a),
        }
    }

    /// A setting's editor at `r`: 0 a text box, 1 a checkbox, 2 a list of its choices. Values go
    /// through `set_setting` (checked) like the module's own editors.
    fn setting_widget(&mut self, ui: &mut egui::Ui, i: NodeId, name: &str, r: Rect, how: u8) {
        let Some(s) = self.manifest_of(i).and_then(|m| m.setting(name).cloned()) else {
            ui.painter().text(r.min, Align2::LEFT_TOP, format!("no setting {name}"), FontId::proportional(10.5 * self.zoom), WARN);
            return;
        };
        let cur = self.nodes[i].kind.settings().into_iter().find(|(k, _)| k == name).map_or(s.default.clone(), |(_, v)| v);
        let mut set = None;
        match how {
            1 => {
                let mut b = cur == "true";
                if ui.put(r, egui::Checkbox::without_text(&mut b)).changed() {
                    set = Some(b.to_string());
                }
            }
            2 => {
                let choices = match &s.ty {
                    corroborate::module_manifest::SettingType::Choice(c) => c.clone(),
                    _ => vec![cur.clone()],
                };
                ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                    egui::ComboBox::from_id_salt(("script setting", i, name)).width(r.width()).selected_text(&cur).show_ui(ui, |ui| {
                        for c in &choices {
                            if ui.selectable_label(*c == cur, c).clicked() {
                                set = Some(c.clone());
                            }
                        }
                    });
                });
            }
            _ => {
                let key = (i, name.to_string());
                let bad = self.setting_errors.contains_key(&key);
                let draft = self.drafts.entry(key).or_insert_with(|| cur.clone());
                let mut edit = egui::TextEdit::singleline(draft);
                if bad {
                    edit = edit.text_color(WARN);
                }
                if ui.put(r, edit).changed() {
                    set = Some(draft.clone());
                }
            }
        }
        if let Some(v) = set {
            match self.set_setting(i, name, &v) {
                Ok(()) => {
                    self.setting_errors.remove(&(i, name.to_string()));
                }
                Err(e) => {
                    self.setting_errors.insert((i, name.to_string()), e);
                }
            }
        }
    }
}

/// Text broken into lines of about `w` characters.
fn wrap(s: &str, w: usize) -> String {
    let mut out = String::new();
    let mut line = 0;
    for word in s.split(' ') {
        if line > 0 && line + word.len() > w {
            out.push('\n');
            line = 0;
        } else if line > 0 {
            out.push(' ');
            line += 1;
        }
        out.push_str(word);
        line += word.chars().count();
    }
    out
}
