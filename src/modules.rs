//! Modules: each node kind has its own folder under `src/modules/` with its code and a README.md.
//! `mod.rs` (in this library) holds the node's name, label and ports, and for planes and enf the
//! detector. `node.rs` holds what the node does in the workbench (its state while running, window,
//! timeline marks, result file); it is compiled into the `corroborate` program, not this library.
//! Plugin folders (adsb-check, meta) hold a separate program each, built as its own binary (see Cargo.toml).

use crate::core::{Port, PortKind};

pub mod combine;
pub mod enf;
pub mod grid_frequencies;
pub mod invert;
pub mod manual;
pub mod map;
pub mod plugin;
pub mod planes;
pub mod record;
pub mod source;

/// Outputs of every source node: audio and time.
pub fn source_outputs() -> Vec<Port> {
    vec![Port::req("audio", PortKind::Audio), Port::req("time", PortKind::Time)]
}
