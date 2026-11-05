//! Plugin node: a separate program speaking the plugin protocol, fed the same audio.
//! Bundled plugins live in their own folders (`meta`, `adsb-check`) and are built as binaries.

use crate::core::{NodeSpec, Port, PortKind, Role};

pub const NAME: &str = "plugin";

/// The plugins that ship with the program: program name (without .exe) and display name.
pub const META: &str = "meta-plugin";
pub const ADSB_CHECK: &str = "adsb-check";
pub const ENF: &str = "enf-plugin";
pub const BUNDLED: [(&str, &str); 3] = [(META, "Metadata"), (ADSB_CHECK, "ADS-B check"), (ENF, "ENF (plugin)")];

/// Display name for a plugin program, from its file name.
pub fn label(path: &str) -> String {
    let stem = std::path::Path::new(path).file_stem().map_or(path.to_string(), |s| s.to_string_lossy().into_owned());
    BUNDLED.iter().find(|(exe, _)| *exe == stem).map_or(stem, |(_, name)| name.to_string())
}

#[cfg(test)]
mod tests {
    /// Cargo.toml must name each bundled plugin's program the same way (it cannot read these constants).
    #[test]
    fn bundled_plugins_are_built_under_these_names() {
        let cargo = include_str!("../../../Cargo.toml");
        for (exe, _) in super::BUNDLED {
            assert!(cargo.contains(&format!("name = \"{exe}\"")), "Cargo.toml has no binary named {exe}");
        }
    }
}

pub fn spec() -> NodeSpec {
    NodeSpec {
        name: NAME,
        role: Role::Detector,
        inputs: vec![Port::req("audio", PortKind::Audio), Port::opt("place", PortKind::Location), Port::opt("time", PortKind::Time)],
        outputs: vec![Port::req("findings", PortKind::Event), Port::req("facts", PortKind::Json)],
    }
}
