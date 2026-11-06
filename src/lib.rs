//! Lean real-time detectors for audio, as modules sharing one decoded stream.
//!
//! - `module`: the interface: 16 kHz mono samples in, findings out.
//! - `modules::planes`: aircraft Doppler curves. Whitened low-band STFT (`spectro`) →
//!   Viterbi ridge (`ridge`) → straight-pass Doppler fit (`fit`) → gates (`detect`).
//! - `modules::enf`: ENF (Electrical Network Frequency), the mains hum and its drift.
//! - `core`: the workbench types: values with error and source, locations with radius,
//!   times with error, typed ports and node roles.
//! - `data`: the data bus: one value type per port kind and its JSON form.
//! - `plugin`: the protocol for plugins, separate programs fed the same audio through a pipe.
//! - `ingest` reads WAV files and ffmpeg/stdin streams; `timebase` handles UTC
//!   (Coordinated Universal Time) anchors; `synth` makes test audio.

pub mod area;
pub mod core;
pub mod data;
pub mod ingest;
pub mod locate;
pub mod module;
pub mod module_manifest;
pub mod modules;
pub mod plugin;
pub mod graph;
pub mod sync;
pub mod synth;
pub mod timebase;

pub use modules::planes::detect::{Config, Detector, Event};
pub use module::{Finding, Module, INPUT_SR};

/// The folder the program runs from. Settings, projects and results live under it, so the tool
/// can be moved as one folder: nothing is kept in system folders.
pub fn home() -> std::path::PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).unwrap_or_default()
}

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
