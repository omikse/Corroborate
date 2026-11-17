//! Core types of the workbench: every value carries its uncertainty and its source. Places are
//! area masks (`area`), times are UTC ± error on wires (`data`).
//!
//! - `Value<T>`: a result with an error estimate and the node that produced it.
//! - `PortKind`, `Port`, `NodeSpec`: what a node takes and gives, typed and marked
//!   required or optional. Roles: source, detector, transform, solver, record.
//!
//! No GUI code here. The viewer, the graph and the timeline all read these types.

/// Where a value came from: the node and the input or data it was computed from.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub node: String,
    pub detail: String,
}

impl Source {
    pub fn new(node: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { node: node.into(), detail: detail.into() }
    }
}

/// A result with an error estimate. The unit of `error` is the unit of the value.
#[derive(Clone, Debug, PartialEq)]
pub struct Value<T> {
    pub value: T,
    pub error: f64,
    pub source: Source,
}

/// Type of data on a port. A connection is only allowed between equal kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortKind {
    Audio,
    Video,
    Time,
    Location,
    Track,
    Event,
    File,
    Text,
    Json,
    /// An input that takes any kind (the Result record's inputs); it shows the kind wired into it.
    Any,
    /// A type from a type.toml that is not built in (src/types.rs), by its id.
    Custom(&'static str),
}

/// One input or output of a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub name: &'static str,
    pub kind: PortKind,
    /// A required input must be connected for the node to run. An optional input only refines it.
    pub required: bool,
}

impl Port {
    pub const fn req(name: &'static str, kind: PortKind) -> Self {
        Self { name, kind, required: true }
    }
    pub const fn opt(name: &'static str, kind: PortKind) -> Self {
        Self { name, kind, required: false }
    }
}

/// Where a source's data comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// A stream that is still arriving: URL, microphone, webcam.
    Live,
    /// A file, replayed as a stream from its start.
    File,
    /// External records (flight history, grid frequency, weather). Not a stream.
    Reference,
}

/// The place of a node in the hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Source(SourceKind),
    /// Takes audio or video, gives timed events or tracks.
    Detector,
    /// Converts between kinds, for example text to location.
    Transform,
    /// Takes timed events from several streams, gives offsets and locations.
    Solver,
    /// Stores results: the result JSON and the evidence trail.
    Record,
    /// Shows data (a map, a plot); gives nothing.
    View,
}

/// What a node is: its role, typed ports and a name. Nodes are programs; this is the
/// description they announce, so the graph can check connections before anything runs.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeSpec {
    pub name: &'static str,
    pub role: Role,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
}

impl NodeSpec {
    /// True when `from` (an output of one node) may feed `to` (an input of another).
    pub fn can_connect(from: &Port, to: &Port) -> bool {
        from.kind == to.kind || to.kind == PortKind::Any
    }

    /// True when every required input of the node is among the connected inputs.
    pub fn ready(&self, connected: &[&str]) -> bool {
        self.inputs
            .iter()
            .filter(|p| p.required)
            .all(|p| connected.contains(&p.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_is_ready_only_when_required_inputs_are_connected() {
        let spec = NodeSpec {
            name: "aircraft",
            role: Role::Detector,
            inputs: vec![Port::req("audio", PortKind::Audio), Port::opt("time", PortKind::Time)],
            outputs: vec![Port::req("track", PortKind::Track)],
        };
        assert!(!spec.ready(&["time"]));
        assert!(spec.ready(&["audio"]));
        assert!(!NodeSpec::can_connect(&spec.outputs[0], &spec.inputs[0]));
    }

    #[test]
    fn value_keeps_error_and_source() {
        let v = Value { value: 218.0_f64, error: 4.0, source: Source::new("aircraft", "pass 1") };
        assert_eq!(v.source.node, "aircraft");
        assert_eq!(v.error, 4.0);
    }
}
