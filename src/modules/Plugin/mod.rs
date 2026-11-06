//! Plugin node: a separate program speaking the plugin protocol, fed the same audio.
//! Programs that ship with the tool are module folders of their own (Metadata, ADS-B check) whose
//! module.toml names the program; Add node offers them from there.

use crate::core::NodeSpec;

crate::builtin_manifest!();

/// Display name for a plugin program: the label of the module whose program it is (the
/// module.toml in its folder, or a built-in module naming it), else its file name.
pub fn label(path: &str) -> String {
    let p = std::path::Path::new(path);
    let file = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let in_folder = p.parent().and_then(|d| std::fs::read_to_string(d.join("module.toml")).ok()).and_then(|t| crate::module_manifest::parse(&t).ok());
    if let Some(m) = in_folder.filter(|m| m.program.as_deref() == Some(&file)) {
        return m.label;
    }
    if let Some(m) = crate::modules::builtin().into_iter().find(|m| m.program.as_deref() == Some(&file)) {
        return m.label.clone();
    }
    p.file_stem().map_or(path.to_string(), |s| s.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    /// Every program a built-in module names is built by Cargo under that name.
    #[test]
    fn module_programs_are_built_under_these_names() {
        let cargo = include_str!("../../../Cargo.toml");
        for m in crate::modules::builtin() {
            for exe in m.program.iter().chain(&m.programs) {
                let name = exe.trim_end_matches(".exe");
                assert!(cargo.contains(&format!("name = \"{name}\"")), "Cargo.toml has no binary named {name} ({})", m.label);
            }
        }
    }
}

/// Its ports and role: its module.toml.
pub fn spec() -> NodeSpec {
    manifest().spec()
}
