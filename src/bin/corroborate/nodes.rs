//! The node kinds of the workbench. Each kind's name, ports and label live in its own folder
//! under `src/modules/` (see that folder's README.md); this file only holds the enum the app
//! stores and dispatches to those modules. Nothing about a node is written down twice.

use corroborate::core::NodeSpec;
use corroborate::modules::{combine, enf, file, grid_frequencies, invert, manual, map, mic, planes, plugin, record, stream};

#[derive(Clone, Debug)]
pub enum Kind {
    File { path: String },
    Stream { url: String },
    Mic { device: String },
    /// Facts typed in by a person: date, time, place, coordinates, notes, own fields.
    Manual { rows: Vec<(String, String)> },
    Planes,
    Enf,
    /// Grid frequency map (50/60 Hz zones by region) for ENF: a JSON file, swappable.
    GridFrequencies { path: String },
    /// A separate program speaking the plugin protocol (meta-plugin, adsb-check, …).
    Plugin { path: String },
    Record,
    /// Shows an area on a world map (Robinson projection).
    Map,
    /// Several locations into one: all must hold (AND) or any may (OR).
    Combine { any: bool },
    /// Flips a location: what it added it now subtracts, and the other way round.
    Invert,
}

pub use combine::INPUTS as COMBINE_INPUTS;
pub use record::{EVENTS as RECORD_EVENTS, FACTS as RECORD_FACTS, LOCATIONS as RECORD_LOCATIONS};

impl Kind {
    pub fn is_source(&self) -> bool {
        matches!(self, Kind::File { .. } | Kind::Stream { .. } | Kind::Mic { .. })
    }

    pub fn default_label(&self) -> String {
        match self {
            Kind::File { .. } => file::LABEL.into(),
            Kind::Stream { .. } => stream::LABEL.into(),
            Kind::Mic { .. } => mic::LABEL.into(),
            Kind::Manual { .. } => manual::LABEL.into(),
            Kind::Planes => planes::LABEL.into(),
            Kind::Enf => enf::LABEL.into(),
            Kind::GridFrequencies { .. } => grid_frequencies::LABEL.into(),
            Kind::Plugin { path } => plugin::label(path),
            Kind::Record => record::LABEL.into(),
            Kind::Map => map::LABEL.into(),
            Kind::Combine { any } => combine::label(*any),
            Kind::Invert => invert::LABEL.into(),
        }
    }

    pub fn spec(&self) -> NodeSpec {
        match self {
            Kind::File { .. } => file::spec(),
            Kind::Stream { .. } => stream::spec(),
            Kind::Mic { .. } => mic::spec(),
            Kind::Manual { .. } => manual::spec(),
            Kind::Planes => planes::spec(),
            Kind::Enf => enf::spec(),
            Kind::GridFrequencies { .. } => grid_frequencies::spec(),
            Kind::Plugin { .. } => plugin::spec(),
            Kind::Record => record::spec(),
            Kind::Combine { .. } => combine::spec(),
            Kind::Invert => invert::spec(),
            Kind::Map => map::spec(),
        }
    }

    /// The one setting a project file keeps for this node (path, URL, device, combine mode).
    /// The node's name in the file is `spec().name`.
    pub fn saved_param(&self) -> String {
        match self {
            Kind::File { path } | Kind::GridFrequencies { path } | Kind::Plugin { path } => path.clone(),
            Kind::Stream { url } => url.clone(),
            Kind::Mic { device } => device.clone(),
            Kind::Combine { any } => combine::mode(*any).into(),
            Kind::Manual { .. } | Kind::Planes | Kind::Enf | Kind::Record | Kind::Map | Kind::Invert => String::new(),
        }
    }

    /// A node read back from a project file: its saved name and setting.
    pub fn from_saved(name: &str, param: String) -> Option<Kind> {
        Some(match name {
            file::NAME => Kind::File { path: param },
            stream::NAME => Kind::Stream { url: param },
            mic::NAME => Kind::Mic { device: param },
            manual::NAME => Kind::Manual { rows: Vec::new() },
            planes::NAME => Kind::Planes,
            enf::NAME => Kind::Enf,
            grid_frequencies::NAME => Kind::GridFrequencies { path: param },
            plugin::NAME => Kind::Plugin { path: param },
            record::NAME => Kind::Record,
            map::NAME => Kind::Map,
            combine::NAME => Kind::Combine { any: combine::is_any(&param) },
            invert::NAME => Kind::Invert,
            _ => return None,
        })
    }
}

pub const PLUGINS: &str = "PLUGINS";

/// The add menu: sections in order, each with the nodes it offers. Button text is each node's label.
pub fn menu() -> Vec<(&'static str, Vec<Kind>)> {
    vec![
        (
            "SOURCES",
            vec![
                Kind::File { path: String::new() },
                Kind::Stream { url: String::new() },
                Kind::Mic { device: String::new() },
                Kind::Manual { rows: manual::defaults() },
            ],
        ),
        ("DETECTORS", vec![Kind::Planes, Kind::Enf]),
        ("DATA", vec![Kind::GridFrequencies { path: String::new() }]),
        (PLUGINS, bundled_plugins().into_iter().map(|path| Kind::Plugin { path }).collect()),
        ("OUTPUT", vec![Kind::Record]),
        ("LOCATION", vec![Kind::Combine { any: false }, Kind::Combine { any: true }, Kind::Invert]),
        ("VIEW", vec![Kind::Map]),
    ]
}

/// Programs next to this one that speak the plugin protocol (bundled plugins).
pub fn bundled_plugins() -> Vec<String> {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) else {
        return Vec::new();
    };
    let mut found: Vec<String> = plugin::BUNDLED
        .iter()
        .map(|(exe, _)| dir.join(format!("{exe}{}", std::env::consts::EXE_SUFFIX)))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

/// The grid-frequency file that ships with the program: next to it, or in the source tree it was built from.
pub fn bundled_grid_file() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let name = grid_frequencies::BUNDLED_FILE;
    let in_source = |root: std::path::PathBuf| root.join(grid_frequencies::SOURCE_DIR).join(name);
    [dir.join(name), dir.join("data").join(name), in_source(dir.join("../..")), in_source(env!("CARGO_MANIFEST_DIR").into())]
        .into_iter()
        .find(|p| p.exists())
        .map_or(String::new(), |p| p.canonicalize().unwrap_or(p).to_string_lossy().trim_start_matches(r"\\?\").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every node the menu offers comes back from a project file as the same node.
    #[test]
    fn every_node_survives_save_and_load() {
        let mut kinds: Vec<Kind> = menu().into_iter().flat_map(|(_, k)| k).collect();
        kinds.push(Kind::Plugin { path: "x/adsb-check.exe".into() });
        for k in kinds {
            let back = Kind::from_saved(k.spec().name, k.saved_param()).expect("name is known");
            assert_eq!(back.spec().name, k.spec().name);
            assert_eq!(back.saved_param(), k.saved_param());
            assert_eq!(back.default_label(), k.default_label());
        }
    }

    /// No two different kinds of node share a name in project files.
    #[test]
    fn node_names_are_unique() {
        let kinds: Vec<Kind> = menu().into_iter().flat_map(|(_, k)| k).collect();
        for a in &kinds {
            for b in &kinds {
                if a.spec().name == b.spec().name {
                    assert_eq!(std::mem::discriminant(a), std::mem::discriminant(b), "{} is used twice", a.spec().name);
                }
            }
        }
    }
}
