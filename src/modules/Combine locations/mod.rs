//! Combine node: several locations into one, with all of them (AND) or any of them (OR).

use crate::core::NodeSpec;

crate::builtin_manifest!();
pub const INPUTS: [&str; 4] = ["location 1", "location 2", "location 3", "location 4"];

/// The mode as written in project files and labels: "any" (OR) or "all" (AND).
pub fn mode(any: bool) -> &'static str {
    if any { "any" } else { "all" }
}

pub fn is_any(mode: &str) -> bool {
    mode == "any"
}

/// The node's name for a mode: its [[variant]] in module.toml.
pub fn label(any: bool) -> String {
    let m = manifest();
    m.variants.iter().find(|v| v.settings.iter().any(|(k, x)| k == "mode" && x == mode(any))).map_or_else(|| format!("{} ({})", m.label, mode(any)), |v| v.label.clone())
}

/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
}
