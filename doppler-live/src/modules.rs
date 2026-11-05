//! Modules: each has its own folder under `src/modules/` with its code and a README.md.
//! Library modules (planes, enf) read the shared 16 kHz stream; plugin folders (adsb-check,
//! meta) hold a separate program each, built as its own binary (see Cargo.toml).

pub mod enf;
pub mod planes;
