//! Lean real-time detectors for audio, as modules sharing one decoded stream.
//!
//! - `module`: the interface: 16 kHz mono samples in, findings out.
//! - `modules::planes`: aircraft Doppler curves. Whitened low-band STFT (`spectro`) →
//!   Viterbi ridge (`ridge`) → straight-pass Doppler fit (`fit`) → gates (`detect`).
//! - `modules::enf`: ENF (Electrical Network Frequency), the mains hum and its drift.
//! - `core`: the workbench types: values with error and source, locations with radius,
//!   times with error, typed ports and node roles.
//! - `plugin`: the protocol for plugins, separate programs fed the same audio through a pipe.
//! - `ingest` reads WAV files and ffmpeg/stdin streams; `timebase` handles UTC
//!   (Coordinated Universal Time) anchors; `synth` makes test audio.

pub mod core;
pub mod detect;
pub mod fit;
pub mod ingest;
pub mod module;
pub mod modules;
pub mod plugin;
pub mod ridge;
pub mod spectro;
pub mod synth;
pub mod timebase;

pub use detect::{Config, Detector, Event};
pub use module::{Finding, Module, INPUT_SR};

/// Run helper tools (ffmpeg, ffprobe, curl, yt-dlp) without opening a console window of
/// their own on Windows; elsewhere this does nothing.
pub fn no_window(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
