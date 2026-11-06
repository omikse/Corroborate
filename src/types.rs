//! Data types: what travels on a wire. Every type is a folder holding a `type.toml`, like modules
//! are folders holding a `module.toml`. The built-in ones (`src/types/<id>/`) are embedded; the
//! tool's `types\` folder overrides them and adds new ones while the program runs, and a module
//! folder may bring its own in `types\<id>.toml`. All of them join one shared list.
//!
//! ```toml
//! id = "location"          # lowercase letters, digits and -; what module.toml ports name as kind
//! label = "Location"
//! color = "#c785c9"        # the colour of its dots and wires
//! flow = "value"           # value: one value, the latest replaces the one before (circle dot)
//!                          # list: items that keep coming and are kept (square dot)
//!                          # stream: continuous raw data, not a value (diamond dot)
//! description = "An area of the Earth ..."
//!
//! [[field]]                # what a Value node asks for; the JSON it builds is {name: value, ...}
//! name = "lat"             # a single field named "value" is the JSON itself (text, file, json)
//! type = "number"          # number | text | datetime (UTC text, sent as Unix seconds) | bool | choice | file
//! label = "latitude"       # shown instead of the name
//! unit = "°"
//! min = -90                # number: its range; choice: choices = ["a", "b"]
//! max = 90
//! default = 0
//! ```

use crate::module_manifest::{intern, parse_color};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

/// How a type's data arrives; it gives the dot its shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// One value; the latest replaces the one before. Circle.
    Value,
    /// Items that keep coming and are kept. Square.
    List,
    /// Continuous raw data, not a value. Diamond.
    Stream,
}

impl Flow {
    pub fn name(self) -> &'static str {
        match self {
            Flow::Value => "value",
            Flow::List => "list",
            Flow::Stream => "stream",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldType {
    Number { min: Option<f64>, max: Option<f64> },
    Text,
    /// Typed as UTC text (2026-11-04 20:06:39), sent as Unix seconds.
    Datetime,
    Bool,
    Choice(Vec<String>),
    File,
}

/// One thing a value of the type is made of.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub label: String,
    pub ty: FieldType,
    pub unit: String,
    pub default: String,
}

impl Field {
    /// The value as it will be used, or why it is refused. Empty is allowed: the field is left out.
    pub fn check(&self, v: &str) -> Result<String, String> {
        let v = v.trim();
        if v.is_empty() {
            return Ok(String::new());
        }
        match &self.ty {
            FieldType::Text | FieldType::File => Ok(v.to_string()),
            FieldType::Number { min, max } => {
                let x: f64 = v.parse().map_err(|_| format!("{}: \"{v}\" is not a number", self.label))?;
                if !x.is_finite() || min.is_some_and(|m| x < m) || max.is_some_and(|m| x > m) {
                    let lo = min.map_or("…".into(), |m| m.to_string());
                    let hi = max.map_or("…".into(), |m| m.to_string());
                    return Err(format!("{}: {v} is outside {lo} to {hi}", self.label));
                }
                Ok(v.to_string())
            }
            FieldType::Datetime => match crate::timebase::parse_iso8601(v) {
                Some(_) => Ok(v.to_string()),
                None => Err(format!("{}: \"{v}\" is not a UTC time like 2026-11-04 20:06:39", self.label)),
            },
            FieldType::Bool => match v {
                "true" | "false" => Ok(v.to_string()),
                _ => Err(format!("{}: must be true or false", self.label)),
            },
            FieldType::Choice(c) => {
                if c.iter().any(|x| x == v) {
                    Ok(v.to_string())
                } else {
                    Err(format!("{}: \"{v}\" is not one of {}", self.label, c.join(", ")))
                }
            }
        }
    }

    /// The checked value as JSON.
    pub fn json(&self, v: &str) -> Result<Option<String>, String> {
        let v = self.check(v)?;
        if v.is_empty() {
            return Ok(None);
        }
        Ok(Some(match &self.ty {
            FieldType::Number { .. } | FieldType::Bool => v,
            FieldType::Datetime => crate::timebase::parse_iso8601(&v).unwrap_or(0.0).to_string(),
            _ => format!("\"{}\"", crate::timebase::json_escape(&v)),
        }))
    }
}

/// A data type: its look, how it flows, and what it is made of.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeDef {
    pub id: &'static str,
    pub label: String,
    pub color: [u8; 3],
    pub flow: Flow,
    pub description: String,
    pub fields: Vec<Field>,
    /// Where it came from: "built in", the types folder, or a module's folder.
    pub origin: String,
}

impl TypeDef {
    /// One value of the type as JSON, from field values by name. A single field named "value" is
    /// the JSON itself; else an object of the fields that are filled in. `json` fields are kept as
    /// written (they are JSON already).
    pub fn json(&self, values: &[(String, String)]) -> Result<String, String> {
        let get = |n: &str| values.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str()).unwrap_or("");
        if let [f] = &self.fields[..] {
            if f.name == "value" {
                if self.id == "json" {
                    return Ok(get("value").trim().to_string());
                }
                return Ok(f.json(get("value"))?.unwrap_or_else(|| "\"\"".into()));
            }
        }
        let mut parts = Vec::new();
        for f in &self.fields {
            if let Some(j) = f.json(get(&f.name))? {
                parts.push(format!("\"{}\": {j}", crate::timebase::json_escape(&f.name)));
            }
        }
        Ok(format!("{{{}}}", parts.join(", ")))
    }
}

/// Reads a type.toml. Errors say what is wrong and where.
pub fn parse(text: &str) -> Result<TypeDef, String> {
    let t: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string().trim().to_string())?;
    let string = |t: &toml::Table, k: &str| -> Result<Option<String>, String> {
        match t.get(k) {
            None => Ok(None),
            Some(toml::Value::String(s)) => Ok(Some(s.clone())),
            Some(v) => Err(format!("`{k}` must be text in quotes, not {}", v.type_str())),
        }
    };
    let id = string(&t, "id")?.ok_or("no `id`")?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(format!("`id` \"{id}\" must be lowercase letters, digits and -"));
    }
    let label = string(&t, "label")?.unwrap_or_else(|| id.clone());
    let color = match string(&t, "color")? {
        None => [154, 163, 173],
        Some(c) => parse_color(&c).ok_or(format!("`color` \"{c}\" must be a colour like \"#ff8800\""))?,
    };
    let flow = match string(&t, "flow")?.as_deref() {
        None | Some("value") => Flow::Value,
        Some("list") => Flow::List,
        Some("stream") => Flow::Stream,
        Some(f) => return Err(format!("`flow` \"{f}\" must be value, list or stream")),
    };
    let mut fields: Vec<Field> = Vec::new();
    let list = match t.get("field") {
        None => Vec::new(),
        Some(toml::Value::Array(a)) => a.iter().map(|x| x.as_table().ok_or("each [[field]] must be a section")).collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("`field` must be [[field]] sections".into()),
    };
    for f in list {
        let name = string(f, "name")?.ok_or("a [[field]] has no `name`")?;
        let num = |k: &str| -> Result<Option<f64>, String> {
            match f.get(k) {
                None => Ok(None),
                Some(toml::Value::Integer(n)) => Ok(Some(*n as f64)),
                Some(toml::Value::Float(x)) => Ok(Some(*x)),
                Some(_) => Err(format!("[[field]] {name}: `{k}` must be a number")),
            }
        };
        let ty = match string(f, "type")?.as_deref().unwrap_or("text") {
            "number" => FieldType::Number { min: num("min")?, max: num("max")? },
            "text" => FieldType::Text,
            "datetime" => FieldType::Datetime,
            "bool" => FieldType::Bool,
            "file" => FieldType::File,
            "choice" => FieldType::Choice(match f.get("choices") {
                Some(toml::Value::Array(a)) => a.iter().map(|x| x.as_str().map(str::to_string).ok_or(format!("[[field]] {name}: `choices` must list text in quotes"))).collect::<Result<_, _>>()?,
                _ => return Err(format!("[[field]] {name}: a choice needs `choices = [\"a\", \"b\"]`")),
            }),
            other => return Err(format!("[[field]] {name}: type \"{other}\" must be number, text, datetime, bool, choice or file")),
        };
        let default = match f.get("default") {
            None => String::new(),
            Some(toml::Value::String(x)) => x.clone(),
            Some(toml::Value::Integer(n)) => n.to_string(),
            Some(toml::Value::Float(x)) => x.to_string(),
            Some(toml::Value::Boolean(b)) => b.to_string(),
            Some(_) => return Err(format!("[[field]] {name}: `default` must be text, a number or true/false")),
        };
        if fields.iter().any(|q| q.name == name) {
            return Err(format!("[[field]] {name} is declared twice"));
        }
        let field = Field { label: string(f, "label")?.unwrap_or_else(|| name.clone()), name, ty, unit: string(f, "unit")?.unwrap_or_default(), default };
        field.check(&field.default).map_err(|e| format!("[[field]] default: {e}"))?;
        fields.push(field);
    }
    Ok(TypeDef { id: intern(&id), label, color, flow, description: string(&t, "description")?.unwrap_or_default(), fields, origin: "built in".into() })
}

macro_rules! builtin_types {
    ($($id:literal),*) => {
        [$(include_str!(concat!("types/", $id, "/type.toml"))),*]
    };
}

/// The types built into the program (src/types/<id>/type.toml).
pub fn builtin() -> &'static [Arc<TypeDef>] {
    static B: OnceLock<Vec<Arc<TypeDef>>> = OnceLock::new();
    B.get_or_init(|| {
        builtin_types!("audio", "video", "time", "location", "track", "event", "file", "text", "json", "any")
            .iter()
            .map(|t| Arc::new(parse(t).expect("built-in type.toml (checked by tests)")))
            .collect()
    })
}

/// The type folders in `dir` (each subfolder with a type.toml), read. A folder whose file cannot
/// be read or parsed comes back with the error, so it can be shown.
pub fn discover(dir: &Path) -> Vec<(PathBuf, Result<TypeDef, String>)> {
    let mut found: Vec<(PathBuf, Result<TypeDef, String>)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("type.toml"))
        .filter(|p| p.is_file())
        .map(|p| (p.clone(), read(&p, &format!("types\\{}", p.parent().and_then(|d| d.file_name()).unwrap_or_default().to_string_lossy()))))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// One type.toml file, its origin set to `origin`.
pub fn read(file: &Path, origin: &str) -> Result<TypeDef, String> {
    let mut t = std::fs::read_to_string(file).map_err(|e| e.to_string()).and_then(|s| parse(&s))?;
    t.origin = origin.to_string();
    Ok(t)
}

/// Where types come from besides the program: the tool's types\ folder, and each modules folder's
/// modules (their types\ folders), by modules folder.
#[derive(Default)]
struct Sources {
    folder: Vec<Arc<TypeDef>>,
    modules: Vec<(PathBuf, Vec<Arc<TypeDef>>)>,
    merged: Vec<Arc<TypeDef>>,
    conflicts: Vec<(&'static str, Vec<String>)>,
}

static SOURCES: RwLock<Option<Sources>> = RwLock::new(None);

/// Equal but for where it came from.
fn same(a: &TypeDef, b: &TypeDef) -> bool {
    (a.id, &a.label, a.color, a.flow, &a.description, &a.fields) == (b.id, &b.label, b.color, b.flow, &b.description, &b.fields)
}

/// One list from the built-in types, the types folder (it overrides them) and modules' types (a
/// module may reuse a type; one that defines it differently is a conflict).
fn merge(folder: &[Arc<TypeDef>], modules: &[Arc<TypeDef>]) -> (Vec<Arc<TypeDef>>, Vec<(&'static str, Vec<String>)>) {
    let mut v: Vec<Arc<TypeDef>> = builtin().iter().map(|b| folder.iter().find(|t| t.id == b.id).cloned().unwrap_or_else(|| b.clone())).collect();
    for t in folder {
        if !v.iter().any(|a| a.id == t.id) {
            v.push(t.clone());
        }
    }
    let mut conflicts: Vec<(&'static str, Vec<String>)> = Vec::new();
    for t in modules {
        match v.iter().find(|a| a.id == t.id) {
            None => v.push(t.clone()),
            Some(a) if same(a, t) => {}
            Some(a) => match conflicts.iter_mut().find(|(id, _)| *id == t.id) {
                Some((_, o)) => o.push(t.origin.clone()),
                None => conflicts.push((t.id, vec![a.origin.clone(), t.origin.clone()])),
            },
        }
    }
    (v, conflicts)
}

fn update(f: impl FnOnce(&mut Sources)) {
    let mut g = SOURCES.write().unwrap();
    let s = g.get_or_insert_with(Sources::default);
    f(s);
    let modules: Vec<Arc<TypeDef>> = s.modules.iter().flat_map(|(_, v)| v.iter().cloned()).collect();
    (s.merged, s.conflicts) = merge(&s.folder, &modules);
}

/// Sets the types found in the tool's types\ folder; they override built-in ones of the same id.
pub fn set_found(types: Vec<TypeDef>) {
    update(|s| s.folder = types.into_iter().map(Arc::new).collect());
}

/// Sets the types the modules in modules folder `dir` bring.
pub fn set_module_types(dir: &Path, types: Vec<TypeDef>) {
    let types: Vec<Arc<TypeDef>> = types.into_iter().map(Arc::new).collect();
    update(|s| match s.modules.iter_mut().find(|(d, _)| d == dir) {
        Some((_, v)) => *v = types,
        None => s.modules.push((dir.to_path_buf(), types)),
    });
}

/// Every type: the built-in ones (overridden by the types folder), then the ones found.
pub fn all() -> Vec<Arc<TypeDef>> {
    match SOURCES.read().unwrap().as_ref() {
        Some(s) => s.merged.clone(),
        None => builtin().to_vec(),
    }
}

/// Why type `id` cannot be used: defined differently in two places. Wires of it are refused.
pub fn conflict(id: &str) -> Option<String> {
    let g = SOURCES.read().unwrap();
    let (_, o) = g.as_ref()?.conflicts.iter().find(|(k, _)| *k == id)?;
    Some(format!("type \"{id}\" is defined differently in {}", o.join(" and ")))
}

/// Every type defined differently in two places, with the places.
pub fn conflicts() -> Vec<(&'static str, Vec<String>)> {
    SOURCES.read().unwrap().as_ref().map(|s| s.conflicts.clone()).unwrap_or_default()
}

/// The types in a module folder's types\ (`types\<id>.toml`), or what is wrong with one.
pub fn of_module(folder: &Path) -> Result<Vec<TypeDef>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder.join("types")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
    files.sort();
    let name = folder.file_name().unwrap_or_default().to_string_lossy();
    files
        .iter()
        .map(|f| {
            let file = f.file_name().unwrap_or_default().to_string_lossy();
            read(f, &format!("modules\\{name}\\types\\{file}")).map_err(|e| format!("types\\{file}: {e}"))
        })
        .collect()
}

/// The type of port kind `k`; a grey value type when there is none.
pub fn of(k: crate::core::PortKind) -> Arc<TypeDef> {
    let id = crate::module_manifest::kind_id(k);
    lookup(id).unwrap_or_else(|| Arc::new(TypeDef { id, label: id.into(), color: [154, 163, 173], flow: Flow::Value, description: String::new(), fields: Vec::new(), origin: "unknown".into() }))
}

/// The type `id`, if there is one.
pub fn lookup(id: &str) -> Option<Arc<TypeDef>> {
    match SOURCES.read().unwrap().as_ref() {
        Some(s) => s.merged.iter().find(|t| t.id == id).cloned(),
        None => builtin().iter().find(|t| t.id == id).cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_types_read() {
        let ids: Vec<&str> = builtin().iter().map(|t| t.id).collect();
        assert_eq!(ids, ["audio", "video", "time", "location", "track", "event", "file", "text", "json", "any"]);
        let flow = |id| lookup(id).unwrap().flow;
        assert_eq!((flow("audio"), flow("event"), flow("location")), (Flow::Stream, Flow::List, Flow::Value));
        assert_eq!(lookup("location").unwrap().color, [0xc7, 0x85, 0xc9]);
        assert!(lookup("smell").is_none());
    }

    #[test]
    fn fields_make_json() {
        let loc = lookup("location").unwrap();
        let v = |a: &[(&str, &str)]| a.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        assert_eq!(loc.json(&v(&[("lat", "50"), ("lon", "14.4"), ("add", "true")])).unwrap(), "{\"lat\": 50, \"lon\": 14.4, \"add\": true}");
        assert!(loc.json(&v(&[("lat", "95")])).unwrap_err().contains("outside"));
        let time = lookup("time").unwrap();
        assert_eq!(time.json(&v(&[("utc0", "1970-01-01 00:01:00")])).unwrap(), "{\"utc0\": 60}");
        assert!(time.json(&v(&[("utc0", "yesterday")])).unwrap_err().contains("UTC"));
        assert_eq!(lookup("text").unwrap().json(&v(&[("value", "a \"b\"")])).unwrap(), "\"a \\\"b\\\"\"");
        assert_eq!(lookup("json").unwrap().json(&v(&[("value", "[1, 2]")])).unwrap(), "[1, 2]");
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(parse("label = \"a\"").unwrap_err().contains("id"));
        assert!(parse("id = \"a\"\nflow = \"river\"").unwrap_err().contains("value, list or stream"));
        assert!(parse("id = \"a\"\ncolor = \"red\"").unwrap_err().contains("colour"));
        assert!(parse("id = \"a\"\n[[field]]\nname = \"x\"\ntype = \"blob\"").unwrap_err().contains("blob"));
        assert!(parse("id = \"a\"\n[[field]]\nname = \"x\"\ntype = \"number\"\nmax = 1\ndefault = 3").unwrap_err().contains("outside"));
    }

    #[test]
    fn a_folder_overrides_and_adds() {
        let dir = std::env::temp_dir().join(format!("corroborate-types-{}", std::process::id()));
        for (id, col) in [("location", "#010203"), ("spectrum", "#445566")] {
            std::fs::create_dir_all(dir.join(id)).unwrap();
            std::fs::write(dir.join(id).join("type.toml"), format!("id = \"{id}\"\ncolor = \"{col}\"\nflow = \"stream\"")).unwrap();
        }
        let found: Vec<Arc<TypeDef>> = discover(&dir).into_iter().map(|(_, t)| Arc::new(t.unwrap())).collect();
        assert_eq!(found[1].origin, "types\\spectrum");
        let (all, conflicts) = merge(&found, &[]);
        assert!(conflicts.is_empty());
        let get = |id: &str| all.iter().find(|t| t.id == id).unwrap().clone();
        assert_eq!(get("location").color, [1, 2, 3]);
        assert_eq!(get("spectrum").flow, Flow::Stream);
        assert_eq!(all.len(), 11);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn modules_reuse_or_conflict() {
        let t = |id: &str, col: &str, origin: &str| {
            let mut t = parse(&format!("id = \"{id}\"
color = \"{col}\"")).unwrap();
            t.origin = origin.into();
            Arc::new(t)
        };
        let (all, conflicts) = merge(&[], &[t("spectrum", "#445566", "a"), t("spectrum", "#445566", "b"), t("tick", "#000000", "a")]);
        assert_eq!((all.len(), conflicts.len()), (12, 0), "the same type twice is one type");
        let (_, conflicts) = merge(&[], &[t("spectrum", "#445566", "a"), t("spectrum", "#ffffff", "b"), t("location", "#000000", "c")]);
        assert_eq!(conflicts, [("spectrum", vec!["a".to_string(), "b".to_string()]), ("location", vec!["built in".to_string(), "c".to_string()])]);
    }
}
