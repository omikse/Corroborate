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
pub mod timebase;

pub use detect::{Config, Detector, Event};

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
