//! Core types of the workbench: every value carries its uncertainty and its source.
//!
//! - `Value<T>`: a result with an error estimate and the node that produced it.
//! - `Location`: a point with a radius and the method that set the radius.
//! - `TimeRef`: an absolute (UTC) or relative time, always with an error in seconds.
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

/// Mean Earth radius used for distances, in metres.
const EARTH_R_M: f64 = 6_371_000.0;

/// How a location was obtained. It sets what the radius means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Device GPS (Global Positioning System): metres.
    Gps,
    /// IP address lookup: often kilometres.
    Ip,
    /// A place name or a map pin given by a person.
    PlaceName,
    /// Aircraft Doppler passes matched to flight data: kilometres.
    Doppler,
    /// Arrival times of a shared sound across synchronised streams.
    SharedSound,
    /// Coordinates written in file metadata. As wide as their source.
    Metadata,
    /// Typed in by a person.
    Manual,
}

/// A point with the radius (metres) inside which the true place is expected.
#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
    pub method: Method,
}

impl Location {
    /// Returns `None` when the coordinates are out of range or the radius is negative or not finite.
    pub fn new(lat: f64, lon: f64, radius_m: f64, method: Method) -> Option<Self> {
        let ok = (-90.0..=90.0).contains(&lat)
            && (-180.0..=180.0).contains(&lon)
            && radius_m.is_finite()
            && radius_m >= 0.0;
        ok.then_some(Self { lat, lon, radius_m, method })
    }

    /// Great-circle distance to another point, in metres (haversine formula).
    pub fn distance_m(&self, other: &Location) -> f64 {
        let (p1, p2) = (self.lat.to_radians(), other.lat.to_radians());
        let dp = p2 - p1;
        let dl = (other.lon - self.lon).to_radians();
        let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
        2.0 * EARTH_R_M * a.sqrt().min(1.0).asin()
    }

    /// True when the two radii overlap: the sources do not contradict each other.
    pub fn overlaps(&self, other: &Location) -> bool {
        self.distance_m(other) <= self.radius_m + other.radius_m
    }
}

/// A time with its error. Either it is tied to UTC, or only to a relative clock whose
/// origin is not yet known (an unknown offset, to be found by the solver).
#[derive(Clone, Debug, PartialEq)]
pub enum TimeRef {
    /// UTC in Unix seconds, with the error in seconds.
    Absolute { utc: f64, error_s: f64 },
    /// Seconds on the named relative clock (for example one stream's own timeline).
    Relative { t: f64, clock: String, error_s: f64 },
}

impl TimeRef {
    pub fn error_s(&self) -> f64 {
        match self {
            TimeRef::Absolute { error_s, .. } | TimeRef::Relative { error_s, .. } => *error_s,
        }
    }

    /// True when the time is tied to UTC, so it can sit at its real place on a shared timeline.
    pub fn is_absolute(&self) -> bool {
        matches!(self, TimeRef::Absolute { .. })
    }
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
        from.kind == to.kind
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
    fn distance_matches_known_route() {
        // Paris to London, about 343.5 km great-circle.
        let paris = Location::new(48.8566, 2.3522, 0.0, Method::Manual).unwrap();
        let london = Location::new(51.5074, -0.1278, 0.0, Method::Manual).unwrap();
        let d = paris.distance_m(&london);
        assert!((d - 343_500.0).abs() < 1_000.0, "got {d}");
    }

    #[test]
    fn rejects_bad_coordinates_and_radius() {
        assert!(Location::new(91.0, 0.0, 10.0, Method::Gps).is_none());
        assert!(Location::new(0.0, 181.0, 10.0, Method::Gps).is_none());
        assert!(Location::new(0.0, 0.0, -1.0, Method::Gps).is_none());
        assert!(Location::new(0.0, 0.0, f64::NAN, Method::Gps).is_none());
        assert!(Location::new(0.0, 0.0, 0.0, Method::Gps).is_some());
    }

    #[test]
    fn overlapping_radii_agree_and_separate_ones_contradict() {
        let a = Location::new(0.0, 0.0, 5_000.0, Method::Gps).unwrap();
        let near = Location::new(0.0, 0.07, 5_000.0, Method::Doppler).unwrap(); // ~7.8 km away
        let far = Location::new(0.0, 0.5, 5_000.0, Method::Doppler).unwrap(); // ~55 km away
        assert!(a.overlaps(&near));
        assert!(!a.overlaps(&far));
    }

    #[test]
    fn time_ref_reports_error_and_absoluteness() {
        let abs = TimeRef::Absolute { utc: 1_793_839_234.0, error_s: 0.5 };
        let rel = TimeRef::Relative { t: 12.0, clock: "stream A".into(), error_s: 30.0 };
        assert!(abs.is_absolute());
        assert!(!rel.is_absolute());
        assert_eq!(rel.error_s(), 30.0);
    }

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
