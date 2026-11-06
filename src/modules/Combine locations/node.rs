//! The Combine locations node in the workbench: the area it gives. Its look is look.rhai.
//! Compiled into the `corroborate` program.

use super::*;

impl App {
    /// What each wired location still allows, combined: all must hold (AND) or any may (OR).
    pub(crate) fn combine_area_out(&self, i: NodeId, any: bool) -> Result<(Arc<Area>, String), String> {
        let mut acc: Option<WorldMask> = None;
        let mut used = Vec::new();
        for inp in nodes::COMBINE_INPUTS {
            let Some((f, _)) = self.graph.feeder(i, inp) else { continue };
            if let Ok((a, _)) = self.area_from(f) {
                let al = a.allowed();
                acc = Some(match acc {
                    None => al,
                    Some(m) if any => m.or(&al),
                    Some(m) => m.and(&al),
                });
                used.push(self.nodes[f].label.clone());
            }
        }
        let m = acc.ok_or("no location with an area wired in yet")?;
        let how = if any { "any of" } else { "all of" };
        let note = if m.is_empty() {
            format!("CONTRADICTION: nothing is possible under {how} {}", used.join(", "))
        } else {
            format!("possible under {how} {}", used.join(", "))
        };
        Ok((Arc::new(Area::add(m)), note))
    }
}

/// The Combine node on the canvas: drawn by its look.rhai (the mode to pick); the status line
/// says what is left, or CONTRADICTION.
pub struct Look;

impl crate::look::NodeLook for Look {}
