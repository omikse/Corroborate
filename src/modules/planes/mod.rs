//! Planes module: aircraft Doppler curves (propeller blade-pass tones).
//!
//! Takes the shared 16 kHz stream, downsamples it to the detector's 4 kHz, and runs the
//! detector (`detect.rs` here): whitened STFT, Viterbi ridges, straight-pass Doppler fit, gates.

pub mod detect;
pub mod fit;
pub mod ridge;
pub mod spectro;

use detect::{Config, Detector, Event};
use crate::ingest::Resampler;
use crate::core::{NodeSpec, Port, PortKind, Role};
use crate::module::{Finding, Module, INPUT_SR};

pub const NAME: &str = "planes";
pub const LABEL: &str = "Planes (Doppler)";

/// The node's ports: the one definition, used by the workbench and by `Module::spec`.
pub fn spec() -> NodeSpec {
    NodeSpec {
        name: NAME,
        role: Role::Detector,
        inputs: vec![Port::req("audio", PortKind::Audio)],
        outputs: vec![Port::req("aircrafts", PortKind::Event)],
    }
}

pub struct Planes {
    rs: Resampler,
    det: Detector,
    buf: Vec<f32>,
}

impl Planes {
    pub fn new(cfg: Config) -> Self {
        Self { rs: Resampler::new(INPUT_SR, cfg.sr), det: Detector::new(cfg), buf: Vec::with_capacity(1024) }
    }

    /// As `Module::push`, also handing each whitened frame (time, z per bin) to `on_frame`
    /// (the viewer draws them).
    pub fn push_with(&mut self, x: &[f32], on_frame: impl FnMut(f64, &[f32])) -> Vec<Event> {
        self.buf.clear();
        self.rs.process(x, &mut self.buf);
        self.det.push_with(&self.buf, on_frame)
    }

    pub fn flush_events(&mut self) -> Vec<Event> {
        self.det.flush()
    }

    /// The underlying detector (ridges, held candidates, bin frequencies) for display.
    pub fn detector(&self) -> &Detector {
        &self.det
    }
}

impl Module for Planes {
    fn name(&self) -> &'static str {
        NAME
    }

    fn spec(&self) -> NodeSpec {
        spec()
    }

    fn describe(&self) -> String {
        let c = &self.det.cfg;
        format!(
            "planes: 16 kHz → {:.0} Hz (FIR lowpass 1.3 kHz); STFT {} points ({:.2} Hz bins), hop {:.2} s, \
             band {:.0}–{:.0} Hz, per-bin whitening; Viterbi ridge over {:.0} s every {:.0} s; \
             straight-pass Doppler fit; gates: snr ≥ {}, rms ≤ {} Hz, speed {:.0}–{:.0} m/s",
            c.sr,
            c.nfft,
            c.sr / c.nfft as f64,
            c.hop as f64 / c.sr,
            c.fmin,
            c.fmax,
            c.window_s,
            c.step_s,
            c.snr_min,
            c.rms_max,
            c.v_min,
            c.v_max
        )
    }

    fn push(&mut self, x: &[f32]) -> Vec<Finding> {
        self.push_with(x, |_, _| {}).into_iter().map(Finding::Plane).collect()
    }

    fn flush(&mut self) -> Vec<Finding> {
        self.flush_events().into_iter().map(Finding::Plane).collect()
    }
}
