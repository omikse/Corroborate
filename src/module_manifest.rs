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
//! [processing]
//! program = "adsb-check.exe"   # a separate program speaking the plugin protocol
//!
//! [[variant]]                  # offered more than once in Add node, each with its own settings
//! label = "Combine locations (any)"
//! mode = "any"
//! ```

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
    let program = match t.get("processing") {
        None => None,
        Some(toml::Value::Table(p)) => match p.get("program") {
            None => None,
            Some(toml::Value::String(s)) => Some(s.clone()),
            Some(_) => return Err("`processing.program` must be text in quotes".into()),
        },
        Some(_) => return Err("`processing` must be a [processing] section".into()),
    };
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
        };
        let all = [m("b", "B", "Zeta", None), m("v", "Map", "View", None), m("s2", "Manual", "Sources", Some(2)), m("s1", "SOURCER", "Sources", Some(1)), m("a", "A", "Checks", None)];
        let cats = by_category(&all);
        let names: Vec<&str> = cats.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(names, ["Sources", "View", "Checks", "Zeta"]);
        assert_eq!(cats[0].1.iter().map(|m| m.id).collect::<Vec<_>>(), ["s1", "s2"]);
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
}
