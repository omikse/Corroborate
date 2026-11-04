//! Lean real-time detector for aircraft Doppler curves in audio.
//!
//! Pipeline: whitened low-band STFT (`spectro`) → Viterbi ridge (`ridge`) → straight-pass
//! Doppler fit (`fit`) → gates and de-duplication (`detect`). `ingest` reads WAV files and
//! ffmpeg/stdin streams; `synth` makes test audio.

pub mod detect;
pub mod fit;
pub mod ingest;
pub mod ridge;
pub mod spectro;
pub mod synth;

pub use detect::{Config, Detector, Event};
