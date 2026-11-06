//! A wire dragged from a dot and let go on empty canvas: a menu of the modules that take its type
//! (dragged from an output) or give it (dragged from an input, with a Value node of the type
//! first). The one picked is added where the wire was let go, and wired.

use super::*;
use corroborate::module_manifest::kind_id;

/// The dot a wire was dragged from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum End {
    /// (node, output index)
    Out(NodeId, usize),
    /// (node, input index)
    In(NodeId, usize),
}

/// The open menu: where the wire came from and where it was let go (screen and canvas).
pub(crate) struct DropMenu {
    pub end: End,
    pub screen: Pos2,
    pub world: Pos2,
}

impl App {
    fn end_kind(&self, end: End) -> PortKind {
        let spec = &self.graph.nodes()[match end {
            End::Out(n, _) | End::In(n, _) => n,
        }]
        .spec;
        match end {
            End::Out(_, k) => spec.outputs[k].kind,
            End::In(_, k) => spec.inputs[k].kind,
        }
    }

    /// What the menu offers for a wire from `end`: a label and the node it adds.
    pub(crate) fn drop_choices(&self, end: End) -> Vec<(String, Kind)> {
        let kind = self.end_kind(end);
        let mut v: Vec<(String, Kind)> = Vec::new();
        if let End::In(..) = end {
            let t = corroborate::types::of(kind);
            let ty = if kind == PortKind::Any { "location" } else { kind_id(kind) };
            if t.flow != corroborate::types::Flow::Stream {
                let label = if kind == PortKind::Any { "Value".to_string() } else { format!("Value ({})", t.label) };
                v.push((label, Kind::Value { ty: ty.into(), rows: corroborate::modules::value::defaults(ty) }));
            }
        }
        for (_, entries) in nodes::menu() {
            for (label, k) in entries {
                if matches!(k, Kind::Value { .. }) {
                    continue;
                }
                let spec = k.spec();
                let fits = match end {
                    End::Out(..) => spec.inputs.iter().any(|p| p.kind == kind || p.kind == PortKind::Any),
                    End::In(..) => spec.outputs.iter().any(|p| p.kind == kind || kind == PortKind::Any),
                };
                if fits {
                    v.push((label, k));
                }
            }
        }
        v
    }

    /// Adds `kind` at `world` (to the left of it for a wire into an input) and wires it to `end`.
    pub(crate) fn drop_add(&mut self, end: End, kind: Kind, world: Pos2) -> NodeId {
        let want = self.end_kind(end);
        let id = self.add(kind, world);
        if let End::In(..) = end {
            let w = self.node_rect(Pos2::ZERO, id).width() / self.zoom;
            self.nodes[id].pos.x -= w + 20.0;
        }
        let spec = self.graph.nodes()[id].spec.clone();
        match end {
            End::Out(n, k) => {
                let out = self.graph.nodes()[n].spec.outputs[k].name;
                match spec.inputs.iter().find(|p| p.kind == want) {
                    Some(p) => self.wire(n, out, id, p.name),
                    None => self.wire_many(n, out, id),
                }
            }
            End::In(n, k) => {
                let inp = self.graph.nodes()[n].spec.inputs[k].name;
                if let Some(p) = spec.outputs.iter().find(|p| p.kind == want || want == PortKind::Any) {
                    self.graph.disconnect(n, inp);
                    self.wire(id, p.name, n, inp);
                }
            }
        }
        self.sync_many_ports();
        self.pick_only(id);
        self.save_project();
        id
    }

    /// The menu, while it is open. Clicking outside it or Escape closes it.
    pub(crate) fn drop_menu_ui(&mut self, ctx: &egui::Context) {
        let Some(m) = &self.drop_menu else { return };
        let (end, screen, world) = (m.end, m.screen, m.world);
        let choices = self.drop_choices(end);
        let kind = self.end_kind(end);
        let mut pick = None;
        let r = egui::Area::new(egui::Id::new("drop-menu")).fixed_pos(screen).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                let t = corroborate::types::of(kind);
                let what = if let End::Out(..) = end { "takes" } else { "gives" };
                ui.colored_label(DIM, egui::RichText::new(format!("add a node that {what} {}", t.label.to_lowercase())).small());
                if choices.is_empty() {
                    ui.colored_label(DIM, "no module does");
                }
                for (k, (label, _)) in choices.iter().enumerate() {
                    if ui.button(label).clicked() {
                        pick = Some(k);
                    }
                }
            });
        });
        let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !r.response.rect.contains(p)));
        if let Some(k) = pick {
            self.drop_menu = None;
            let (label, kind) = choices[k].clone();
            let id = self.drop_add(end, kind, world);
            self.msg = format!("added {label} and wired it");
            let _ = id;
        } else if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drop_menu = None;
        }
    }
}
