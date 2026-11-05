//! The node kinds of the workbench: what each one is, its typed ports, and its settings.
//! The old viewer's features, as nodes: sources (file, stream, microphone), the planes and
//! ENF detectors, plugins (metadata, ADS-B check, any other program), manual data, and the
//! result record.

use corroborate::core::{NodeSpec, Port, PortKind, Role, SourceKind};

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

pub const MANUAL_KEYS: [&str; 5] = ["date", "time", "place", "coordinates", "notes"];

/// Record inputs: findings from up to six nodes and facts from up to three.
pub const RECORD_EVENTS: [&str; 6] = ["findings 1", "findings 2", "findings 3", "findings 4", "findings 5", "findings 6"];
pub const RECORD_FACTS: [&str; 3] = ["facts 1", "facts 2", "facts 3"];
pub const COMBINE_INPUTS: [&str; 4] = ["location 1", "location 2", "location 3", "location 4"];
pub const RECORD_LOCATIONS: [&str; 3] = ["location 1", "location 2", "location 3"];

impl Kind {
    pub fn is_source(&self) -> bool {
        matches!(self, Kind::File { .. } | Kind::Stream { .. } | Kind::Mic { .. })
    }

    pub fn default_label(&self) -> String {
        match self {
            Kind::File { .. } => "File".into(),
            Kind::Stream { .. } => "Stream".into(),
            Kind::Mic { .. } => "Microphone".into(),
            Kind::Manual { .. } => "Manual data".into(),
            Kind::Planes => "Planes (Doppler)".into(),
            Kind::Enf => "Power hum (ENF)".into(),
            Kind::GridFrequencies { .. } => "Grid frequencies (50/60 Hz)".into(),
            Kind::Plugin { path } => plugin_label(path),
            Kind::Record => "Result record".into(),
            Kind::Map => "Map".into(),
            Kind::Combine { any: false } => "Combine locations (all)".into(),
            Kind::Combine { any: true } => "Combine locations (any)".into(),
            Kind::Invert => "Invert location".into(),
        }
    }

    pub fn spec(&self) -> NodeSpec {
        use PortKind::*;
        let source_out = vec![Port::req("audio", Audio), Port::req("time", Time)];
        match self {
            Kind::File { .. } => NodeSpec { name: "file", role: Role::Source(SourceKind::File), inputs: vec![], outputs: source_out },
            Kind::Stream { .. } => NodeSpec { name: "stream", role: Role::Source(SourceKind::Live), inputs: vec![], outputs: source_out },
            Kind::Mic { .. } => NodeSpec { name: "mic", role: Role::Source(SourceKind::Live), inputs: vec![], outputs: source_out },
            Kind::Manual { .. } => NodeSpec {
                name: "manual",
                role: Role::Source(SourceKind::Reference),
                inputs: vec![],
                outputs: vec![Port::req("place", Location), Port::req("time", Time), Port::req("facts", Json)],
            },
            Kind::Planes => NodeSpec {
                name: "planes",
                role: Role::Detector,
                inputs: vec![Port::req("audio", Audio), Port::opt("time", Time)],
                outputs: vec![Port::req("passes", Event)],
            },
            Kind::Enf => NodeSpec {
                name: "enf",
                role: Role::Detector,
                inputs: vec![
                    Port::req("audio", Audio),
                    Port::req("grid", Json),
                    Port::opt("time", Time),
                    Port::opt("location", Location),
                ],
                outputs: vec![Port::req("hum", Event), Port::req("location", Location)],
            },
            Kind::GridFrequencies { .. } => NodeSpec {
                name: "grid-frequencies",
                role: Role::Source(SourceKind::Reference),
                inputs: vec![],
                outputs: vec![Port::req("data", Json)],
            },
            Kind::Plugin { .. } => NodeSpec {
                name: "plugin",
                role: Role::Detector,
                inputs: vec![Port::req("audio", Audio), Port::opt("place", Location), Port::opt("time", Time)],
                outputs: vec![Port::req("findings", Event), Port::req("facts", Json)],
            },
            Kind::Record => NodeSpec {
                name: "record",
                role: Role::Record,
                inputs: RECORD_EVENTS
                    .iter()
                    .map(|n| Port::opt(n, Event))
                    .chain(RECORD_FACTS.iter().map(|n| Port::opt(n, Json)))
                    .chain(RECORD_LOCATIONS.iter().map(|n| Port::opt(n, Location)))
                    .collect(),
                outputs: vec![Port::req("location", Location)],
            },
            Kind::Combine { .. } => NodeSpec {
                name: "combine",
                role: Role::Transform,
                inputs: COMBINE_INPUTS.iter().map(|n| Port::opt(n, Location)).collect(),
                outputs: vec![Port::req("location", Location)],
            },
            Kind::Invert => NodeSpec {
                name: "invert",
                role: Role::Transform,
                inputs: vec![Port::req("location", Location)],
                outputs: vec![Port::req("location", Location)],
            },
            Kind::Map => NodeSpec { name: "map", role: Role::View, inputs: vec![Port::req("location", Location)], outputs: vec![] },
        }
    }
}

pub fn plugin_label(path: &str) -> String {
    let stem = std::path::Path::new(path).file_stem().map_or(path.to_string(), |s| s.to_string_lossy().into_owned());
    match stem.as_str() {
        "meta-plugin" => "Metadata".into(),
        "adsb-check" => "ADS-B check".into(),
        "enf-plugin" => "ENF (plugin)".into(),
        other => other.to_string(),
    }
}

/// Programs next to this one that speak the plugin protocol (bundled plugins).
pub fn bundled_plugins() -> Vec<String> {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) else {
        return Vec::new();
    };
    let mut found: Vec<String> = ["meta-plugin", "adsb-check", "enf-plugin"]
        .iter()
        .map(|n| dir.join(format!("{n}{}", std::env::consts::EXE_SUFFIX)))
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
    [
        dir.join("grid-frequencies.json"),
        dir.join("data").join("grid-frequencies.json"),
        dir.join("../../src/modules/enf/grid-frequencies.json"),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/modules/enf/grid-frequencies.json"),
    ]
    .into_iter()
    .find(|p| p.exists())
    .map_or(String::new(), |p| p.canonicalize().unwrap_or(p).to_string_lossy().trim_start_matches(r"\\?\").to_string())
}

pub fn manual_defaults() -> Vec<(String, String)> {
    MANUAL_KEYS.iter().map(|k| (k.to_string(), String::new())).collect()
}
