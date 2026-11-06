//! `module.toml`: what a module is. Every module is a folder named by its label, holding this file
//! (and its README, data, programs, later its look). Built-in modules embed theirs; folders dropped
//! into the tool's `modules\` folder are found while the program runs (see `discover`).
//!
//! ```toml
//! id = "adsb-check"            # stable, kebab-case: what project files keep
//! label = "ADS-B check"        # shown on the node and in Add node; the folder's name
//! category = "Plugins"         # the Add node heading (no category: not offered there)
//! order = 1                    # place within the heading (else by label)
//! aliases = ["old-name"]       # names older project files used
//! programs = ["x-helper.exe"]  # other programs of the module, kept in its folder
//!
//! role = "detector"            # source-file|source-live|reference|detector|transform|solver|record|view
//!
//! [[input]]                    # what can be wired in; kinds: audio video time location track
//! name = "audio"               # event file text json any
//! kind = "audio"
//! required = true              # must be wired for the module to run (default false)
//!
//! [[output]]                   # what it gives
//! name = "location"
//! kind = "location"
//!
//! [[setting]]                  # a value the person sets on the node: the only way the GUI
//! name = "threshold"           # changes processing. type: text|path|number|bool|choice
//! type = "number"
//! default = 0.5
//! min = 0                      # number: its range; choice: choices = ["a", "b"]
//! max = 1
//!
//! [processing]
//! program = "adsb-check.exe"   # a separate program speaking the plugin protocol
//! protocol = 2                 # 1: audio in, findings out (default); 2: the data bus (src/plugin.rs)
//!
//! [look]                       # how the node looks; anything left out stays as the workbench draws it
//! size = [260, 140]            # unzoomed px
//! fill = "#2a3038"
//! radius = 10                  # corner radius
//! status = false               # hide the status line
//!
//! [look.ports.audio]           # one port (input or output) by name; the same keys may also go
//! side = "bottom"              # in its [[input]] / [[output]]. side: left right top bottom
//! at = 40                      # px along that edge from its start (left or top); else spaced evenly
//! label = "outside"            # inside | outside | hidden
//! dot = "square"               # circle | square | diamond
//! dot_size = 6
//! color = "#ff8800"
//!
//! A `look.rhai` beside module.toml can change the look while the node runs (see the workbench's
//! script.rs).
//!
//! [[variant]]                  # offered more than once in Add node, each with its own settings
//! label = "Combine locations (any)"
//! mode = "any"
//! ```

use crate::core::{NodeSpec, Port, PortKind, Role, SourceKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The Add node headings that come first, in this order. Other categories follow alphabetically.
pub const CATEGORIES: [&str; 7] = ["Sources", "Detectors", "Data", "Plugins", "Output", "Location", "View"];

/// One more entry in Add node for the same module, with settings of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub label: String,
    pub settings: Vec<(String, String)>,
}

/// What a setting holds; every value written to it is checked against this.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingType {
    Text,
    Path,
    Number { min: Option<f64>, max: Option<f64> },
    Bool,
    Choice(Vec<String>),
}

/// A value the person sets on a node: the only channel from the GUI to processing.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    pub name: String,
    pub ty: SettingType,
    pub default: String,
}

impl Setting {
    /// The value as it will be used, or why it is refused.
    pub fn check(&self, v: &str) -> Result<String, String> {
        let v = v.trim();
        match &self.ty {
            SettingType::Text | SettingType::Path => Ok(v.to_string()),
            SettingType::Number { min, max } => {
                let x: f64 = v.parse().map_err(|_| format!("{}: \"{v}\" is not a number", self.name))?;
                if !x.is_finite() || min.is_some_and(|m| x < m) || max.is_some_and(|m| x > m) {
                    let lo = min.map_or("…".into(), |m| m.to_string());
                    let hi = max.map_or("…".into(), |m| m.to_string());
                    return Err(format!("{}: {v} is outside {lo} to {hi}", self.name));
                }
                Ok(v.to_string())
            }
            SettingType::Bool => match v {
                "true" | "false" => Ok(v.to_string()),
                _ => Err(format!("{}: must be true or false", self.name)),
            },
            SettingType::Choice(c) => {
                if c.iter().any(|x| x == v) {
                    Ok(v.to_string())
                } else {
                    Err(format!("{}: \"{v}\" is not one of {}", self.name, c.join(", ")))
                }
            }
        }
    }
}

/// A port kind by its module.toml name.
pub fn port_kind(s: &str) -> Option<PortKind> {
    Some(match s {
        "audio" => PortKind::Audio,
        "video" => PortKind::Video,
        "time" => PortKind::Time,
        "location" => PortKind::Location,
        "track" => PortKind::Track,
        "event" => PortKind::Event,
        "file" => PortKind::File,
        "text" => PortKind::Text,
        "json" => PortKind::Json,
        "any" => PortKind::Any,
        _ => return None,
    })
}

/// The module.toml name of a port kind.
pub fn kind_id(k: PortKind) -> &'static str {
    match k {
        PortKind::Audio => "audio",
        PortKind::Video => "video",
        PortKind::Time => "time",
        PortKind::Location => "location",
        PortKind::Track => "track",
        PortKind::Event => "event",
        PortKind::File => "file",
        PortKind::Text => "text",
        PortKind::Json => "json",
        PortKind::Any => "any",
    }
}

fn role(s: &str) -> Option<Role> {
    Some(match s {
        "source-file" => Role::Source(SourceKind::File),
        "source-live" => Role::Source(SourceKind::Live),
        "reference" => Role::Source(SourceKind::Reference),
        "detector" => Role::Detector,
        "transform" => Role::Transform,
        "solver" => Role::Solver,
        "record" => Role::Record,
        "view" => Role::View,
        _ => return None,
    })
}

/// An edge of a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub fn parse(s: &str) -> Option<Side> {
        Some(match s {
            "left" => Side::Left,
            "right" => Side::Right,
            "top" => Side::Top,
            "bottom" => Side::Bottom,
            _ => return None,
        })
    }
}

/// Where a port's name is written: inside the node beside its dot, outside it, or not at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortLabel {
    Inside,
    Outside,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DotShape {
    Circle,
    Square,
    Diamond,
}

/// How one port looks; None keeps what the workbench does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortLook {
    pub side: Option<Side>,
    /// Unzoomed px along its edge from the edge's start (left end or top end).
    pub at: Option<f32>,
    pub label: Option<PortLabel>,
    pub dot: Option<DotShape>,
    pub dot_size: Option<f32>,
    pub color: Option<[u8; 3]>,
    /// Not drawn (a script's choice; a wired port is drawn all the same).
    pub hidden: bool,
}

impl PortLook {
    /// `over` on top of this: what it sets wins.
    pub fn with(&self, over: &PortLook) -> PortLook {
        PortLook {
            side: over.side.or(self.side),
            at: over.at.or(self.at),
            label: over.label.or(self.label),
            dot: over.dot.or(self.dot),
            dot_size: over.dot_size.or(self.dot_size),
            color: over.color.or(self.color),
            hidden: self.hidden || over.hidden,
        }
    }
}

/// How the node's frame looks; None keeps what the workbench does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameLook {
    pub size: Option<(f32, f32)>,
    pub fill: Option<[u8; 3]>,
    pub radius: Option<f32>,
    pub status: Option<bool>,
}

/// `#rrggbb`
pub fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn look_num(t: &toml::Table, k: &str, what: &str) -> Result<Option<f32>, String> {
    match t.get(k) {
        None => Ok(None),
        Some(toml::Value::Integer(n)) => Ok(Some(*n as f32)),
        Some(toml::Value::Float(f)) => Ok(Some(*f as f32)),
        Some(_) => Err(format!("{what}: `{k}` must be a number")),
    }
}

fn look_color(t: &toml::Table, k: &str, what: &str) -> Result<Option<[u8; 3]>, String> {
    match t.get(k) {
        None => Ok(None),
        Some(toml::Value::String(s)) => parse_color(s).map(Some).ok_or(format!("{what}: `{k}` \"{s}\" must be a colour like \"#ff8800\"")),
        Some(_) => Err(format!("{what}: `{k}` must be a colour like \"#ff8800\"")),
    }
}

/// The look keys of a port (in its [[input]]/[[output]] or [look.ports.<name>]).
fn port_look(t: &toml::Table, what: &str) -> Result<PortLook, String> {
    let word = |k: &str| -> Result<Option<&str>, String> {
        match t.get(k) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(format!("{what}: `{k}` must be text in quotes")),
        }
    };
    Ok(PortLook {
        side: match word("side")? {
            None => None,
            Some(s) => Some(Side::parse(s).ok_or(format!("{what}: side \"{s}\" must be left, right, top or bottom"))?),
        },
        at: look_num(t, "at", what)?,
        label: match word("label")? {
            None => None,
            Some("inside") => Some(PortLabel::Inside),
            Some("outside") => Some(PortLabel::Outside),
            Some("hidden") => Some(PortLabel::Hidden),
            Some(s) => return Err(format!("{what}: label \"{s}\" must be inside, outside or hidden")),
        },
        dot: match word("dot")? {
            None => None,
            Some("circle") => Some(DotShape::Circle),
            Some("square") => Some(DotShape::Square),
            Some("diamond") => Some(DotShape::Diamond),
            Some(s) => return Err(format!("{what}: dot \"{s}\" must be circle, square or diamond")),
        },
        dot_size: look_num(t, "dot_size", what)?,
        color: look_color(t, "color", what)?,
        hidden: false,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: &'static str,
    pub label: String,
    pub category: Option<String>,
    pub order: Option<i64>,
    pub aliases: Vec<String>,
    pub variants: Vec<Variant>,
    /// The program doing the processing (plugin protocol), relative to the module's folder.
    pub program: Option<String>,
    /// Other programs of the module, kept in its folder.
    pub programs: Vec<String>,
    /// The folder it was found in; None for a module built into the program.
    pub folder: Option<PathBuf>,
    pub role: Role,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    pub settings: Vec<Setting>,
    /// How its program talks: 1 audio in, findings out; 2 the data bus (ports and settings).
    pub protocol: u32,
    /// How the node looks ([look] and the port look keys).
    pub frame: FrameLook,
    pub port_looks: Vec<(String, PortLook)>,
    /// Its look.rhai, if it has one.
    pub script: Option<String>,
}

impl Manifest {
    /// The node it makes: its id, role and declared ports.
    pub fn spec(&self) -> NodeSpec {
        NodeSpec { name: self.id, role: self.role, inputs: self.inputs.clone(), outputs: self.outputs.clone() }
    }

    pub fn setting(&self, name: &str) -> Option<&Setting> {
        self.settings.iter().find(|s| s.name == name)
    }

    /// Settings as they start: each declared one at its default.
    pub fn default_settings(&self) -> Vec<(String, String)> {
        self.settings.iter().map(|s| (s.name.clone(), s.default.clone())).collect()
    }

    /// How port `name` looks, as module.toml says.
    pub fn port_look(&self, name: &str) -> PortLook {
        self.port_looks.iter().find(|(n, _)| n == name).map(|(_, l)| l.clone()).unwrap_or_default()
    }
}

/// A name that lives as long as the program, kept once however often it is asked for. Ports and
/// node names are `&'static str`; names read from files come through here.
pub fn intern(s: &str) -> &'static str {
    static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut names = NAMES.lock().unwrap();
    let set = names.get_or_insert_with(HashSet::new);
    if let Some(k) = set.get(s) {
        return k;
    }
    let k: &'static str = Box::leak(s.to_string().into_boxed_str());
    set.insert(k);
    k
}

/// Reads a module.toml. Errors say what is wrong and where.
pub fn parse(text: &str) -> Result<Manifest, String> {
    let t: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string().trim().to_string())?;
    let string = |k: &str| -> Result<Option<String>, String> {
        match t.get(k) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Ok(Some(s.clone())),
            Some(v) => Err(format!("`{k}` must be text in quotes, not {}", v.type_str())),
        }
    };
    let strings = |v: Option<&toml::Value>, k: &str| -> Result<Vec<String>, String> {
        match v {
            None => Ok(Vec::new()),
            Some(toml::Value::Array(a)) => a.iter().map(|x| x.as_str().map(str::to_string).ok_or(format!("`{k}` must list text in quotes"))).collect(),
            Some(_) => Err(format!("`{k}` must be a list like [\"a\", \"b\"]")),
        }
    };
    let id = string("id")?.ok_or("no `id`")?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(format!("`id` \"{id}\" must be lowercase letters, digits and -"));
    }
    let label = string("label")?.ok_or("no `label`")?;
    let order = match t.get("order") {
        None => None,
        Some(toml::Value::Integer(n)) => Some(*n),
        Some(_) => return Err("`order` must be a whole number".into()),
    };
    let (program, protocol) = match t.get("processing") {
        None => (None, 1),
        Some(toml::Value::Table(p)) => {
            let program = match p.get("program") {
                None => None,
                Some(toml::Value::String(s)) => Some(s.clone()),
                Some(_) => return Err("`processing.program` must be text in quotes".into()),
            };
            let protocol = match p.get("protocol") {
                None => 1,
                Some(toml::Value::Integer(n @ 1..=2)) => *n as u32,
                Some(_) => return Err("`processing.protocol` must be 1 or 2".into()),
            };
            (program, protocol)
        }
        Some(_) => return Err("`processing` must be a [processing] section".into()),
    };
    let role = match string("role")? {
        None => Role::Transform,
        Some(r) => role(&r).ok_or(format!("`role` \"{r}\" must be one of source-file, source-live, reference, detector, transform, solver, record, view"))?,
    };
    let tables = |k: &str| -> Result<Vec<&toml::Table>, String> {
        match t.get(k) {
            None => Ok(Vec::new()),
            Some(toml::Value::Array(a)) => a.iter().map(|x| x.as_table().ok_or(format!("each [[{k}]] must be a section"))).collect(),
            Some(_) => Err(format!("`{k}` must be [[{k}]] sections")),
        }
    };
    let mut port_looks: Vec<(String, PortLook)> = Vec::new();
    let mut ports = |k: &str, req_default: bool| -> Result<Vec<Port>, String> {
        let mut v: Vec<Port> = Vec::new();
        for p in tables(k)? {
            let name = p.get("name").and_then(|x| x.as_str()).ok_or(format!("a [[{k}]] has no `name`"))?;
            let kind = p.get("kind").and_then(|x| x.as_str()).ok_or(format!("[[{k}]] {name} has no `kind`"))?;
            let kind = port_kind(kind).ok_or(format!("[[{k}]] {name}: kind \"{kind}\" must be one of audio, video, time, location, track, event, file, text, json, any"))?;
            let required = match p.get("required") {
                None => req_default,
                Some(toml::Value::Boolean(b)) => *b,
                Some(_) => return Err(format!("[[{k}]] {name}: `required` must be true or false")),
            };
            if v.iter().any(|q| q.name == name) {
                return Err(format!("[[{k}]] {name} is declared twice"));
            }
            let l = port_look(p, &format!("[[{k}]] {name}"))?;
            if l != PortLook::default() {
                port_looks.push((name.to_string(), l));
            }
            v.push(Port { name: intern(name), kind, required });
        }
        Ok(v)
    };
    let inputs = ports("input", false)?;
    let outputs = ports("output", true)?;
    let frame = match t.get("look") {
        None => FrameLook::default(),
        Some(toml::Value::Table(l)) => {
            let size = match l.get("size") {
                None => None,
                Some(toml::Value::Array(a)) if a.len() == 2 => {
                    let n: Vec<f32> = a.iter().filter_map(|x| x.as_float().or(x.as_integer().map(|i| i as f64))).map(|x| x as f32).collect();
                    match n[..] {
                        [w, h] if w > 0.0 && h > 0.0 => Some((w, h)),
                        _ => return Err("[look] `size` must be [width, height], both above 0".into()),
                    }
                }
                Some(_) => return Err("[look] `size` must be [width, height]".into()),
            };
            if let Some(toml::Value::Table(ps)) = l.get("ports") {
                for (name, p) in ps {
                    let p = p.as_table().ok_or(format!("[look.ports.{name}] must be a section"))?;
                    let over = port_look(p, &format!("[look.ports.{name}]"))?;
                    match port_looks.iter_mut().find(|(n, _)| n == name) {
                        Some((_, l)) => *l = l.with(&over),
                        None => port_looks.push((name.clone(), over)),
                    }
                }
            }
            let status = match l.get("status") {
                None => None,
                Some(toml::Value::Boolean(b)) => Some(*b),
                Some(_) => return Err("[look] `status` must be true or false".into()),
            };
            FrameLook { size, fill: look_color(l, "fill", "[look]")?, radius: look_num(l, "radius", "[look]")?, status }
        }
        Some(_) => return Err("`look` must be a [look] section".into()),
    };
    let mut settings: Vec<Setting> = Vec::new();
    for s in tables("setting")? {
        let name = s.get("name").and_then(|x| x.as_str()).ok_or("a [[setting]] has no `name`")?.to_string();
        let num = |k: &str| -> Result<Option<f64>, String> {
            match s.get(k) {
                None => Ok(None),
                Some(toml::Value::Integer(n)) => Ok(Some(*n as f64)),
                Some(toml::Value::Float(f)) => Ok(Some(*f)),
                Some(_) => Err(format!("[[setting]] {name}: `{k}` must be a number")),
            }
        };
        let ty = match s.get("type").and_then(|x| x.as_str()).unwrap_or("text") {
            "text" => SettingType::Text,
            "path" => SettingType::Path,
            "number" => SettingType::Number { min: num("min")?, max: num("max")? },
            "bool" => SettingType::Bool,
            "choice" => SettingType::Choice(strings(s.get("choices"), "choices")?),
            other => return Err(format!("[[setting]] {name}: type \"{other}\" must be text, path, number, bool or choice")),
        };
        let default = match s.get("default") {
            None => match &ty {
                SettingType::Number { min, .. } => min.unwrap_or(0.0).to_string(),
                SettingType::Bool => "false".into(),
                SettingType::Choice(c) => c.first().cloned().unwrap_or_default(),
                _ => String::new(),
            },
            Some(toml::Value::String(x)) => x.clone(),
            Some(toml::Value::Integer(n)) => n.to_string(),
            Some(toml::Value::Float(f)) => f.to_string(),
            Some(toml::Value::Boolean(b)) => b.to_string(),
            Some(_) => return Err(format!("[[setting]] {name}: `default` must be text, a number or true/false")),
        };
        let setting = Setting { name: name.clone(), ty, default };
        setting.check(&setting.default).map_err(|e| format!("[[setting]] default: {e}"))?;
        if settings.iter().any(|q| q.name == name) {
            return Err(format!("[[setting]] {name} is declared twice"));
        }
        settings.push(setting);
    }
    let mut variants = Vec::new();
    match t.get("variant") {
        None => {}
        Some(toml::Value::Array(a)) => {
            for v in a {
                let toml::Value::Table(v) = v else { return Err("each [[variant]] must be a section".into()) };
                let label = v.get("label").and_then(|x| x.as_str()).ok_or("a [[variant]] has no `label`")?.to_string();
                let settings = v
                    .iter()
                    .filter(|(k, _)| *k != "label")
                    .map(|(k, x)| {
                        let v = match x {
                            toml::Value::String(s) => s.clone(),
                            toml::Value::Integer(n) => n.to_string(),
                            toml::Value::Float(f) => f.to_string(),
                            toml::Value::Boolean(b) => b.to_string(),
                            other => return Err(format!("[[variant]] `{k}` must be text, a number or true/false, not {}", other.type_str())),
                        };
                        Ok((k.clone(), v))
                    })
                    .collect::<Result<_, String>>()?;
                variants.push(Variant { label, settings });
            }
        }
        Some(_) => return Err("`variant` must be [[variant]] sections".into()),
    }
    Ok(Manifest {
        id: intern(&id),
        label,
        category: string("category")?,
        order,
        aliases: strings(t.get("aliases"), "aliases")?,
        variants,
        program,
        programs: strings(t.get("programs"), "programs")?,
        folder: None,
        role,
        inputs,
        outputs,
        settings,
        protocol,
        frame,
        port_looks,
        script: None,
    })
}

/// The module folders in `dir`: each subfolder with a module.toml, read. A folder whose file
/// cannot be read or parsed comes back with the error, so it can be shown.
pub fn discover(dir: &Path) -> Vec<(PathBuf, Result<Manifest, String>)> {
    let mut found: Vec<(PathBuf, Result<Manifest, String>)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("module.toml").is_file())
        .map(|p| {
            let m = std::fs::read_to_string(p.join("module.toml")).map_err(|e| e.to_string()).and_then(|t| parse(&t)).map(|mut m| {
                m.folder = Some(p.clone());
                m.script = std::fs::read_to_string(p.join("look.rhai")).ok();
                m
            });
            (p, m)
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Add node's headings in order, each with its modules in order: the known categories first,
/// others alphabetically; within one, `order` first, then by label.
pub fn by_category<'a>(modules: impl IntoIterator<Item = &'a Manifest>) -> Vec<(String, Vec<&'a Manifest>)> {
    let mut cats: Vec<(String, Vec<&Manifest>)> = Vec::new();
    for m in modules {
        let Some(c) = &m.category else { continue };
        match cats.iter_mut().find(|(k, _)| k == c) {
            Some((_, v)) => v.push(m),
            None => cats.push((c.clone(), vec![m])),
        }
    }
    let rank = |c: &str| CATEGORIES.iter().position(|k| *k == c).unwrap_or(CATEGORIES.len());
    cats.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then_with(|| a.0.cmp(&b.0)));
    for (_, v) in &mut cats {
        v.sort_by(|a, b| a.order.unwrap_or(i64::MAX).cmp(&b.order.unwrap_or(i64::MAX)).then_with(|| a.label.cmp(&b.label)));
    }
    cats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_full_manifest() {
        let m = parse(
            "id = \"x-check\"\nlabel = \"X check\"\ncategory = \"Checks\"\norder = 2\naliases = [\"x\"]\nprograms = [\"x-helper.exe\"]\n\
             [processing]\nprogram = \"x.exe\"\n[[variant]]\nlabel = \"X (fast)\"\nmode = \"fast\"\n",
        )
        .unwrap();
        assert_eq!(m.id, "x-check");
        assert_eq!(m.category.as_deref(), Some("Checks"));
        assert_eq!(m.order, Some(2));
        assert_eq!(m.aliases, ["x"]);
        assert_eq!(m.program.as_deref(), Some("x.exe"));
        assert_eq!(m.programs, ["x-helper.exe"]);
        assert_eq!(m.variants, [Variant { label: "X (fast)".into(), settings: vec![("mode".into(), "fast".into())] }]);
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(parse("label = \"a\"").unwrap_err().contains("id"));
        assert!(parse("id = \"Bad Id\"\nlabel = \"a\"").unwrap_err().contains("lowercase"));
        assert!(parse("id = \"a\"\nlabel = \"a\"\norder = \"x\"").unwrap_err().contains("order"));
        let e = parse("id = \"a\"\nlabel = \"a\nx").unwrap_err();
        assert!(e.contains("line"), "{e}");
    }

    #[test]
    fn categories_known_first_then_alphabetical() {
        let m = |id: &str, label: &str, cat: &str, order: Option<i64>| Manifest {
            id: intern(id),
            label: label.into(),
            category: Some(cat.into()),
            order,
            aliases: vec![],
            variants: vec![],
            program: None,
            programs: vec![],
            folder: None,
            role: Role::Transform,
            inputs: vec![],
            outputs: vec![],
            settings: vec![],
            protocol: 1,
            frame: FrameLook::default(),
            port_looks: vec![],
            script: None,
        };
        let all = [m("b", "B", "Zeta", None), m("v", "Map", "View", None), m("s2", "Manual", "Sources", Some(2)), m("s1", "SOURCER", "Sources", Some(1)), m("a", "A", "Checks", None)];
        let cats = by_category(&all);
        let names: Vec<&str> = cats.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(names, ["Sources", "View", "Checks", "Zeta"]);
        assert_eq!(cats[0].1.iter().map(|m| m.id).collect::<Vec<_>>(), ["s1", "s2"]);
    }

    #[test]
    fn ports_and_settings() {
        let m = parse(
            r#"id = "x"
label = "X"
role = "detector"
[[input]]
name = "audio"
kind = "audio"
required = true
[[input]]
name = "place"
kind = "location"
[[output]]
name = "claps"
kind = "event"
[[setting]]
name = "level"
type = "number"
default = 0.5
min = 0
max = 1
[[setting]]
name = "mode"
type = "choice"
choices = ["fast", "slow"]
[processing]
program = "x.exe"
protocol = 2
"#,
        )
        .unwrap();
        let s = m.spec();
        assert_eq!(s.role, Role::Detector);
        assert_eq!(s.inputs, [Port::req("audio", PortKind::Audio), Port::opt("place", PortKind::Location)]);
        assert_eq!(s.outputs, [Port::req("claps", PortKind::Event)]);
        assert_eq!(m.protocol, 2);
        assert_eq!(m.default_settings(), [("level".to_string(), "0.5".to_string()), ("mode".to_string(), "fast".to_string())]);
        let level = m.setting("level").unwrap();
        assert_eq!(level.check(" 0.7 "), Ok("0.7".into()));
        assert!(level.check("2").unwrap_err().contains("outside"));
        assert!(level.check("loud").unwrap_err().contains("not a number"));
        assert!(m.setting("mode").unwrap().check("medium").unwrap_err().contains("fast, slow"));
        assert!(parse("id = \"x\"\nlabel = \"X\"\n[[output]]\nname = \"a\"\nkind = \"smell\"").unwrap_err().contains("smell"));
        assert!(parse("id = \"x\"\nlabel = \"X\"\n[[setting]]\nname = \"n\"\ntype = \"number\"\ndefault = 5\nmax = 1").unwrap_err().contains("outside"));
    }

    #[test]
    fn looks() {
        let m = parse(
            r##"id = "x"
label = "X"
[[input]]
name = "audio"
kind = "audio"
side = "bottom"
at = 40
[[output]]
name = "out"
kind = "event"
[look]
size = [260, 140]
fill = "#2a3038"
radius = 10
status = false
[look.ports.audio]
dot = "square"
color = "#ff8800"
[look.ports.video]
label = "hidden"
"##,
        )
        .unwrap();
        assert_eq!(m.frame, FrameLook { size: Some((260.0, 140.0)), fill: Some([0x2a, 0x30, 0x38]), radius: Some(10.0), status: Some(false) });
        let a = m.port_look("audio");
        assert_eq!((a.side, a.at, a.dot, a.color), (Some(Side::Bottom), Some(40.0), Some(DotShape::Square), Some([255, 136, 0])));
        assert_eq!(m.port_look("video").label, Some(PortLabel::Hidden), "a port made in code can be given a look by name");
        assert_eq!(m.port_look("out"), PortLook::default());
        assert!(parse("id = \"x\"\nlabel = \"X\"\n[look]\nfill = \"red\"").unwrap_err().contains("colour"));
        assert!(parse("id = \"x\"\nlabel = \"X\"\n[look.ports.a]\nside = \"up\"").unwrap_err().contains("left, right"));
    }

    #[test]
    fn intern_keeps_one_copy() {
        assert!(std::ptr::eq(intern("same name"), intern(&String::from("same name"))));
    }
}

/// `manifest()` for a module built into the program: its folder's module.toml, embedded and read
/// once. Use it in the module's mod.rs, next to its module.toml.
#[macro_export]
macro_rules! builtin_manifest {
    () => {
        /// What this module is: its folder's module.toml.
        pub fn manifest() -> &'static $crate::module_manifest::Manifest {
            static M: std::sync::OnceLock<$crate::module_manifest::Manifest> = std::sync::OnceLock::new();
            M.get_or_init(|| $crate::module_manifest::parse(include_str!("module.toml")).expect("its module.toml (checked by tests)"))
        }
    };
    (look) => {
        /// What this module is: its folder's module.toml, with its look.rhai.
        pub fn manifest() -> &'static $crate::module_manifest::Manifest {
            static M: std::sync::OnceLock<$crate::module_manifest::Manifest> = std::sync::OnceLock::new();
            M.get_or_init(|| {
                let mut m = $crate::module_manifest::parse(include_str!("module.toml")).expect("its module.toml (checked by tests)");
                m.script = Some(include_str!("look.rhai").to_string());
                m
            })
        }
    };
}
