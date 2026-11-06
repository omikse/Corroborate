//! The node kinds of the workbench. What a module is (id, label, Add node category, aliases,
//! programs) is in its folder's module.toml; its ports and code are in its folder too. This file
//! holds the enum the app stores, and finds modules: built in, and dropped into the tool's
//! `modules\` folder (read again every second, so a new folder shows up in Add node while the
//! program runs). Nothing about a node is written down twice.

use corroborate::core::{NodeSpec, Port, PortKind, Role};
use corroborate::module_manifest::{self as mm, Manifest};
use corroborate::modules::{combine_locations, enf, grid_frequencies, invert_location, map, planes, plugin, result_record, sourcer, value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub enum Kind {
    Sourcer { input: String },
    /// One value of any type typed in by a person: the type's id and its fields (a list type's
    /// items as `1.field`, `2.field`, …).
    Value { ty: String, rows: Vec<(String, String)> },
    Planes,
    /// The grid-frequency file it reads (empty: the bundled file).
    Enf { grid: String },
    /// Grid frequency map (50/60 Hz zones by region) for ENF: a JSON file, swappable.
    GridFrequencies { path: String },
    /// A separate program speaking the plugin protocol (Metadata, ADS-B check, any other).
    Plugin { path: String },
    /// The folder results are saved in (empty: results next to the program).
    ResultRecord { folder: String },
    /// Shows an area on a world map (Robinson projection).
    Map,
    /// Several locations into one: all must hold (AND) or any may (OR).
    CombineLocations { any: bool },
    /// Flips a location: what it added it now subtracts, and the other way round.
    InvertLocation,
    /// A module with no Rust code: a program on the data bus (protocol 2) with the ports and
    /// settings its module.toml declares. Settings are saved as `row|` lines, like Manual's.
    Module { m: Arc<Manifest>, settings: Vec<(String, String)> },
    /// A module this project uses that this tool does not have. Kept as it was saved (name,
    /// setting, rows, wires) so saving the project loses nothing; its ports come from the saved wires.
    Missing { name: String, param: String, rows: Vec<(String, String)> },
}

pub use combine_locations::INPUTS as COMBINE_INPUTS;
pub use result_record::INPUTS as RECORD_INPUTS;

/// The Add node heading "Plugin from file…" goes under.
pub const PLUGINS: &str = "Plugins";

impl Kind {
    pub fn is_source(&self) -> bool {
        matches!(self, Kind::Sourcer { .. })
    }

    /// A built-in module's node by its id, with a [[variant]]'s settings (Combine's mode).
    fn of_id(id: &str, settings: &[(String, String)]) -> Option<Kind> {
        let set = |k: &str| settings.iter().find(|(q, _)| q == k).map(|(_, v)| v.as_str());
        Some(match id {
            "sourcer" => Kind::Sourcer { input: String::new() },
            "value" => Kind::Value { ty: "location".into(), rows: value::defaults("location") },
            "planes" => Kind::Planes,
            "enf" => Kind::Enf { grid: String::new() },
            "grid-frequencies" => Kind::GridFrequencies { path: String::new() },
            "result-record" => Kind::ResultRecord { folder: String::new() },
            "combine-locations" => Kind::CombineLocations { any: combine_locations::is_any(set("mode").unwrap_or("all")) },
            "invert-location" => Kind::InvertLocation,
            "map" => Kind::Map,
            _ => return None,
        })
    }

    pub fn default_label(&self) -> String {
        let label = |m: &Manifest| modules().into_iter().find(|x| x.id == m.id).map_or(m.label.clone(), |x| x.label);
        match self {
            Kind::Sourcer { .. } => label(sourcer::manifest()),
            Kind::Value { .. } => label(value::manifest()),
            Kind::Planes => label(planes::manifest()),
            Kind::Enf { .. } => label(enf::manifest()),
            Kind::GridFrequencies { .. } => label(grid_frequencies::manifest()),
            Kind::Plugin { path } => plugin::label(path),
            Kind::ResultRecord { .. } => label(result_record::manifest()),
            Kind::Map => label(map::manifest()),
            Kind::CombineLocations { any } => combine_locations::label(*any),
            Kind::InvertLocation => label(invert_location::manifest()),
            Kind::Module { m, .. } => m.label.clone(),
            Kind::Missing { name, .. } => format!("{name} (module missing)"),
        }
    }

    pub fn spec(&self) -> NodeSpec {
        match self {
            Kind::Sourcer { .. } => sourcer::spec(),
            Kind::Value { ty, .. } => value::spec(ty),
            Kind::Planes => planes::spec(),
            Kind::Enf { .. } => enf::spec(),
            Kind::GridFrequencies { .. } => grid_frequencies::spec(),
            Kind::Plugin { .. } => plugin::spec(),
            Kind::ResultRecord { .. } => result_record::spec(),
            Kind::CombineLocations { .. } => combine_locations::spec(),
            Kind::InvertLocation => invert_location::spec(),
            Kind::Map => map::spec(),
            Kind::Module { m, .. } => m.spec(),
            Kind::Missing { name, .. } => NodeSpec { name: mm::intern(name), role: Role::View, inputs: vec![], outputs: vec![] },
        }
    }

    /// The one setting a project file keeps for this node (path, URL, device, combine mode).
    /// The node's name in the file is `spec().name`: the module's id.
    pub fn saved_param(&self) -> String {
        match self {
            Kind::GridFrequencies { path } | Kind::Plugin { path } | Kind::Enf { grid: path } | Kind::ResultRecord { folder: path } => path.clone(),
            Kind::Sourcer { input } => input.clone(),
            Kind::CombineLocations { any } => combine_locations::mode(*any).into(),
            Kind::Missing { param, .. } => param.clone(),
            Kind::Value { ty, .. } => ty.clone(),
            Kind::Planes | Kind::Map | Kind::InvertLocation | Kind::Module { .. } => String::new(),
        }
    }

    /// A node read back from a project file: its saved name (an id, or a name older files used)
    /// and setting. A name no module has gives a `Missing` node that keeps what was saved.
    pub fn from_saved(name: &str, param: String) -> Kind {
        if name == "mic" {
            return Kind::Sourcer { input: format!("mic:{param}") };
        }
        let id = corroborate::modules::builtin().into_iter().find(|m| m.id == name || m.aliases.iter().any(|a| a == name)).map(|m| m.id);
        match id {
            Some("sourcer") => Kind::Sourcer { input: param },
            Some("enf") => Kind::Enf { grid: param },
            Some("grid-frequencies") => Kind::GridFrequencies { path: param },
            Some("plugin") => Kind::Plugin { path: find_program(&param) },
            Some("result-record") => Kind::ResultRecord { folder: param },
            Some("combine-locations") => Kind::CombineLocations { any: combine_locations::is_any(&param) },
            Some("value") => Kind::Value { ty: if param.is_empty() { "location".into() } else { param }, rows: Vec::new() },
            Some(id) => Kind::of_id(id, &[]).unwrap_or(Kind::Missing { name: name.into(), param, rows: Vec::new() }),
            None => match modules().into_iter().find(|m| m.id == name).and_then(Kind::module) {
                Some(k) => k,
                None => Kind::Missing { name: name.into(), param, rows: Vec::new() },
            },
        }
    }

    /// The node of a dropped-in module on the data bus (protocol 2 with a program), its
    /// settings at their defaults.
    pub fn module(m: Manifest) -> Option<Kind> {
        (m.protocol == 2 && m.program.is_some()).then(|| Kind::Module { settings: m.default_settings(), m: Arc::new(m) })
    }

    /// Lines a project file keeps under the node: a Value's fields, a module's settings.
    pub fn rows(&self) -> &[(String, String)] {
        match self {
            Kind::Value { rows, .. } | Kind::Module { settings: rows, .. } | Kind::Missing { rows, .. } => rows,
            _ => &[],
        }
    }

    /// The node's settings by the names its module.toml declares (SOURCER's `source`, Combine's
    /// `mode`, a bus module's own), as look.rhai and the setting editors see them. A Value's
    /// fields are given too, for a script to read.
    pub fn settings(&self) -> Vec<(String, String)> {
        match self {
            Kind::Sourcer { input } => vec![("source".into(), input.clone())],
            Kind::Value { ty, rows } => std::iter::once(("type".to_string(), ty.clone())).chain(rows.iter().cloned()).collect(),
            Kind::CombineLocations { any } => vec![("mode".into(), combine_locations::mode(*any).into())],
            _ => self.rows().to_vec(),
        }
    }

    /// Set a declared setting to a value its check already passed.
    pub fn put_setting(&mut self, name: &str, v: String) {
        match (self, name) {
            (Kind::Sourcer { input }, "source") => *input = v,
            (Kind::CombineLocations { any }, "mode") => *any = combine_locations::is_any(&v),
            (Kind::Value { ty, rows }, "type") => {
                *rows = value::defaults(&v);
                *ty = v;
            }
            (Kind::Module { settings, .. }, _) => match settings.iter_mut().find(|(k, _)| k == name) {
                Some((_, x)) => *x = v,
                None => settings.push((name.to_string(), v)),
            },
            _ => {}
        }
    }

    /// A row from a project file: a Value's field, a module setting (a value its checks refuse
    /// keeps the default), a missing module's row as it was.
    pub fn add_row(&mut self, k: String, v: String) {
        match self {
            Kind::Value { rows, .. } | Kind::Missing { rows, .. } => rows.push((k, v)),
            Kind::Module { m, settings } => {
                if let Some(Ok(v)) = m.setting(&k).map(|s| s.check(&v)) {
                    if let Some(slot) = settings.iter_mut().find(|(n, _)| *n == k) {
                        slot.1 = v;
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- finding modules

/// The tool's modules folder: built-in module folders (copied there by run.cmd, where their
/// module.toml can be changed while the program runs) and folders dropped in.
pub fn modules_dir() -> PathBuf {
    corroborate::home().join("modules")
}

/// The tool's types folder: built-in type folders (copied there by run.cmd, where they can be
/// changed while the program runs) and folders dropped in.
pub fn types_dir() -> PathBuf {
    corroborate::home().join("types")
}

/// Reads the types folder into the shared type list (called with the manifests' refresh, once a
/// second).
pub fn refresh_types() {
    let found = corroborate::types::discover(&types_dir()).into_iter().filter_map(|(_, t)| t.ok()).collect();
    corroborate::types::set_found(found);
}

type Found = Vec<(PathBuf, Result<Manifest, String>)>;
static FOUND: Mutex<Option<(Instant, PathBuf, Found)>> = Mutex::new(None);

/// The module folders in `dir`, read at most once a second.
pub fn found_in(dir: &Path) -> Found {
    let dir = dir.to_path_buf();
    let mut f = FOUND.lock().unwrap();
    let fresh = f.as_ref().is_some_and(|(t, d, _)| *d == dir && t.elapsed() < Duration::from_secs(1));
    if !fresh {
        *f = Some((Instant::now(), dir.clone(), mm::discover(&dir)));
    }
    f.as_ref().unwrap().2.clone()
}

/// Every module the tool has: the built-in ones (their copy in the modules folder wins, so a
/// label or category can be changed there) and the folders dropped in.
pub fn modules() -> Vec<Manifest> {
    modules_in(&modules_dir())
}

/// As `modules`, from the modules folder `dir`.
pub fn modules_in(dir: &Path) -> Vec<Manifest> {
    let found: Vec<Manifest> = found_in(dir).into_iter().filter_map(|(_, m)| m.ok()).collect();
    let mut all: Vec<Manifest> = corroborate::modules::builtin()
        .into_iter()
        .map(|b| found.iter().find(|m| m.id == b.id).cloned().unwrap_or_else(|| b.clone()))
        .collect();
    for m in found {
        if !all.iter().any(|a| a.id == m.id) {
            all.push(m);
        }
    }
    all
}

/// Where a module's program is: in its folder, else next to this program (a build in target\).
pub fn program_path(m: &Manifest, program: &str) -> PathBuf {
    let in_folder = m.folder.clone().unwrap_or_else(|| modules_dir().join(&m.label)).join(program);
    if in_folder.exists() {
        return in_folder;
    }
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(program)));
    beside.filter(|p| p.exists()).unwrap_or(in_folder)
}

/// A plugin path from a project file. Programs that moved into their module's folder (or were
/// renamed: meta-plugin is Metadata's `metadata.exe`) are found again by their name.
fn find_program(path: &str) -> String {
    if path.is_empty() || Path::new(path).exists() {
        return path.to_string();
    }
    let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    for m in modules() {
        let Some(prog) = &m.program else { continue };
        let pstem = Path::new(prog).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if pstem == stem || m.aliases.iter().any(|a| *a == stem) {
            return program_path(&m, prog).to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

/// The add menu: headings in order (from the modules' `category`), each with its entries: the
/// button text and the node it makes. "Plugin from file…" is added under Plugins by the menu.
pub fn menu() -> Vec<(String, Vec<(String, Kind)>)> {
    menu_in(&modules_dir())
}

/// As `menu`, from the modules folder `dir`.
pub fn menu_in(dir: &Path) -> Vec<(String, Vec<(String, Kind)>)> {
    let all = modules_in(dir);
    mm::by_category(&all)
        .into_iter()
        .map(|(cat, ms)| {
            let mut entries = Vec::new();
            for m in ms {
                let kind_for = |settings: &[(String, String)]| match (&m.program, Kind::of_id(m.id, settings)) {
                    (_, Some(k)) => Some(k),
                    (Some(_), None) if m.protocol == 2 => Kind::module((*m).clone()),
                    (Some(p), None) => Some(Kind::Plugin { path: program_path(m, p).to_string_lossy().into_owned() }),
                    (None, None) => None,
                };
                if m.variants.is_empty() {
                    if let Some(k) = kind_for(&[]) {
                        entries.push((m.label.clone(), k));
                    }
                } else {
                    for v in &m.variants {
                        if let Some(k) = kind_for(&v.settings) {
                            entries.push((v.label.clone(), k));
                        }
                    }
                }
            }
            (cat, entries)
        })
        .collect()
}

/// Ports for a `Missing` node, from the wires saved with it: an input that takes anything for
/// each wire coming in, an output of the kind its wire goes into for each going out.
pub fn missing_ports(inputs: &[&str], outputs: &[(&str, PortKind)]) -> (Vec<Port>, Vec<Port>) {
    let mut ins: Vec<Port> = Vec::new();
    for n in inputs {
        if !ins.iter().any(|p| p.name == *n) {
            ins.push(Port::opt(mm::intern(n), PortKind::Any));
        }
    }
    let mut outs: Vec<Port> = Vec::new();
    for (n, k) in outputs {
        if !outs.iter().any(|p| p.name == *n) {
            outs.push(Port::req(mm::intern(n), *k));
        }
    }
    (ins, outs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_entries() -> Vec<Kind> {
        menu().into_iter().flat_map(|(_, e)| e.into_iter().map(|(_, k)| k)).collect()
    }

    /// Every node the menu offers comes back from a project file as the same node.
    #[test]
    fn every_node_survives_save_and_load() {
        let mut kinds = all_entries();
        // A program that exists (a path that does not would be looked for by its name).
        kinds.push(Kind::Plugin { path: std::env::current_exe().unwrap().to_string_lossy().into_owned() });
        for k in kinds {
            let back = Kind::from_saved(k.spec().name, k.saved_param());
            assert_eq!(back.spec().name, k.spec().name);
            assert_eq!(back.saved_param(), k.saved_param());
            assert_eq!(back.default_label(), k.default_label());
        }
    }

    /// Names older project files used load as today's modules.
    #[test]
    fn old_names_load() {
        for (old, id) in [("source", "sourcer"), ("file", "sourcer"), ("record", "result-record"), ("invert", "invert-location"), ("combine", "combine-locations")] {
            assert_eq!(Kind::from_saved(old, String::new()).spec().name, id, "{old}");
        }
        assert!(matches!(Kind::from_saved("mic", "USB".into()), Kind::Sourcer { input } if input == "mic:USB"));
        assert!(matches!(Kind::from_saved("teleporter", "x".into()), Kind::Missing { name, param, .. } if name == "teleporter" && param == "x"));
    }

    /// One name per module: every module folder is named by its label, holds a module.toml,
    /// and the ids are unique; every node kind's project-file name is its module's id.
    #[test]
    fn folder_label_and_id_agree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("modules");
        let mut ids = Vec::new();
        for (dir, m) in mm::discover(&root) {
            let m = m.unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
            assert_eq!(dir.file_name().unwrap().to_string_lossy(), m.label, "folder name is the module's label");
            assert!(!ids.contains(&m.id), "id {} used twice", m.id);
            ids.push(m.id);
        }
        for b in corroborate::modules::builtin() {
            assert!(ids.contains(&b.id), "built-in {} has no folder", b.id);
        }
        for k in all_entries() {
            if !matches!(k, Kind::Plugin { .. }) {
                assert!(ids.contains(&k.spec().name), "{} is saved under a name no module has", k.spec().name);
            }
        }
    }

    /// A module folder dropped into the modules folder shows up in Add node under its own
    /// category, and runs its program as a plugin; a broken one is left out.
    #[test]
    fn dropped_in_module_appears() {
        let dir = crate::drive::out_dir().join("modules_dropin");
        let _ = std::fs::remove_dir_all(&dir);
        let me = dir.join("Clap finder");
        std::fs::create_dir_all(&me).unwrap();
        std::fs::write(me.join("module.toml"), "id = \"clap-finder\"
label = \"Clap finder\"
category = \"Checks\"
[processing]
program = \"clap.exe\"
").unwrap();
        std::fs::create_dir_all(dir.join("Broken")).unwrap();
        std::fs::write(dir.join("Broken").join("module.toml"), "label = no quotes").unwrap();
        let m = menu_in(&dir);
        let checks = m.iter().find(|(c, _)| c == "Checks").expect("a Checks heading");
        assert_eq!(m.last().unwrap().0, "Checks", "new headings come after the known ones");
        let (label, kind) = &checks.1[0];
        assert_eq!(label, "Clap finder");
        assert!(matches!(kind, Kind::Plugin { path } if path.ends_with("clap.exe") && path.contains("Clap finder")), "{kind:?}");
        assert_eq!(kind.default_label(), "Clap finder", "a plugin is named by its module");
    }

    /// The menu follows the modules' categories, with today's headings in their order.
    #[test]
    fn menu_follows_categories() {
        let m = menu();
        let heads: Vec<&str> = m.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(heads, ["Sources", "Detectors", "Data", "Plugins", "Output", "Location", "View"]);
        let labels: Vec<&str> = m.iter().flat_map(|(_, e)| e.iter().map(|(l, _)| l.as_str())).collect();
        for want in ["SOURCER", "Value", "Planes (Doppler)", "ENF (power hum)", "ADS-B check", "Metadata", "Combine locations (all)", "Combine locations (any)", "Invert location", "Map"] {
            assert!(labels.contains(&want), "{want} missing from {labels:?}");
        }
    }
}
