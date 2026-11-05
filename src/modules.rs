//! Modules: each node kind has its own folder under `src/modules/` with its code and a README.md.
//! Library modules hold the node's ports and labels (and, for planes and enf, the detector).
//! Plugin folders (adsb-check, meta) hold a separate program each, built as its own binary (see Cargo.toml).

use crate::core::{Port, PortKind};

pub mod combine;
pub mod enf;
pub mod file;
pub mod grid_frequencies;
pub mod invert;
pub mod manual;
pub mod map;
pub mod mic;
pub mod plugin;
pub mod planes;
pub mod record;
pub mod stream;

/// Outputs of every source node: audio and time.
pub fn source_outputs() -> Vec<Port> {
    vec![Port::req("audio", PortKind::Audio), Port::req("time", PortKind::Time)]
}
