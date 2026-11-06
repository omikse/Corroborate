//! Modules: each node kind has its own folder under `src/modules/`, named by its label, with its
//! module.toml (id, label, Add node category; see `module_manifest`), code and a README.md.
//! `mod.rs` (in this library) holds the node's name, label and ports, and for planes and enf the
//! detector. `node.rs` holds what the node does in the workbench (its state while running, window,
//! timeline marks, result file); it is compiled into the `corroborate` program, not this library.
//! Plugin folders (adsb-check, meta) hold a separate program each, built as its own binary (see Cargo.toml).

use crate::core::{Port, PortKind};

// Folders are named by each module's label; the Rust module is its id in snake_case.
#[path = "modules/Combine locations/mod.rs"]
pub mod combine_locations;
#[path = "modules/ENF (power hum)/mod.rs"]
pub mod enf;
#[path = "modules/Grid frequencies (50 or 60 Hz)/mod.rs"]
pub mod grid_frequencies;
#[path = "modules/Invert location/mod.rs"]
pub mod invert_location;
#[path = "modules/Manual data/mod.rs"]
pub mod manual_data;
#[path = "modules/Map/mod.rs"]
pub mod map;
#[path = "modules/Plugin/mod.rs"]
pub mod plugin;
#[path = "modules/Planes (Doppler)/mod.rs"]
pub mod planes;
#[path = "modules/Result record/mod.rs"]
pub mod result_record;
#[path = "modules/SOURCER/mod.rs"]
pub mod sourcer;

/// Every module built into the program, by its module.toml: those with Rust code here, and the
/// program-only ones (Metadata, ADS-B check) whose code is a binary of its own.
pub fn builtin() -> Vec<&'static crate::module_manifest::Manifest> {
    use crate::module_manifest::{parse, Manifest};
    use std::sync::OnceLock;
    static METADATA: OnceLock<Manifest> = OnceLock::new();
    static ADSB_CHECK: OnceLock<Manifest> = OnceLock::new();
    let embedded = |m: &'static OnceLock<Manifest>, text: &'static str| -> &'static Manifest { m.get_or_init(|| parse(text).expect("its module.toml (checked by tests)")) };
    vec![
        sourcer::manifest(),
        manual_data::manifest(),
        planes::manifest(),
        enf::manifest(),
        grid_frequencies::manifest(),
        plugin::manifest(),
        result_record::manifest(),
        combine_locations::manifest(),
        invert_location::manifest(),
        map::manifest(),
        embedded(&METADATA, include_str!("modules/Metadata/module.toml")),
        embedded(&ADSB_CHECK, include_str!("modules/ADS-B check/module.toml")),
    ]
}

/// Outputs of every source node: audio and time.
pub fn source_outputs() -> Vec<Port> {
    vec![Port::req("audio", PortKind::Audio), Port::req("time", PortKind::Time)]
}
