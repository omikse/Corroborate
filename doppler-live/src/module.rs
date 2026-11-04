//! The module interface. Every module reads the same decoded stream (16 kHz mono, read-only
//! chunks, one shared clock: relative seconds from the first sample) and reports findings.
//! Each module does its own preparation (downsampling, filtering) and says what it did in
//! `describe`, so every number in the output can be traced to how it was made.

use crate::detect::Event;
use crate::modules::enf::EnfSegment;
use crate::timebase::Anchor;

/// Sample rate of the shared stream every module receives.
pub const INPUT_SR: f64 = 16_000.0;

/// A detection from one module. An enum rather than an open-ended trait object: the set of
/// kinds is small, and each keeps its own typed fields.
#[derive(Clone, Debug)]
pub enum Finding {
    Plane(Event),
    Enf(EnfSegment),
}

impl Finding {
    pub fn module(&self) -> &'static str {
        match self {
            Finding::Plane(_) => "planes",
            Finding::Enf(_) => "enf",
        }
    }

    /// The finding's reference time: closest approach for planes, middle for ENF.
    pub fn t_ref(&self) -> f64 {
        match self {
            Finding::Plane(e) => e.t_cpa,
            Finding::Enf(s) => 0.5 * (s.t_start + s.t_end),
        }
    }

    pub fn span(&self) -> (f64, f64) {
        match self {
            Finding::Plane(e) => (e.t_start, e.t_end),
            Finding::Enf(s) => (s.t_start, s.t_end),
        }
    }

    /// Frequency band (Hz) the finding occupies, for display.
    pub fn band(&self) -> (f64, f64) {
        match self {
            Finding::Plane(e) => (e.f_post, e.f_pre),
            Finding::Enf(s) => (s.min_hz * s.harmonic as f64, s.max_hz * s.harmonic as f64),
        }
    }

    /// One JSON object with a `"module"` field first.
    pub fn to_json(&self, anchor: Option<&Anchor>) -> String {
        let body = match self {
            Finding::Plane(e) => e.to_json(anchor),
            Finding::Enf(s) => s.to_json(anchor),
        };
        format!("{{\"module\":\"{}\",{}", self.module(), &body[1..])
    }
}

pub trait Module: Send {
    fn name(&self) -> &'static str;
    /// How this module processes the stream (rates, bands, resolution, method), for the
    /// output files and the viewer.
    fn describe(&self) -> String;
    /// Feed the next chunk of the shared stream (INPUT_SR, mono).
    fn push(&mut self, x: &[f32]) -> Vec<Finding>;
    /// End of input: report anything still held.
    fn flush(&mut self) -> Vec<Finding>;
}

/// Build modules by name (`planes`, `enf`); unknown names are an error.
pub fn build(names: &[&str]) -> Result<Vec<Box<dyn Module>>, String> {
    names
        .iter()
        .map(|n| -> Result<Box<dyn Module>, String> {
            match n.trim() {
                "planes" => Ok(Box::new(crate::modules::planes::Planes::new(crate::Config::default()))),
                "enf" => Ok(Box::new(crate::modules::enf::Enf::new())),
                other => Err(format!("unknown module '{other}' (available: planes, enf)")),
            }
        })
        .collect()
}
