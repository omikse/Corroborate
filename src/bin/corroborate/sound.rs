//! Sound: while playing, the selected node's audio (its own for a source, otherwise its audio
//! input's source) plays at that node's playhead. The output opens at the first Play.

use super::*;
use std::sync::{Arc, Mutex};

/// What the output callback reads: a source's audio (16 kHz) and where it is in it.
#[derive(Default)]
pub struct Feed {
    pub audio: Option<Arc<Mutex<Vec<i16>>>>,
    /// In samples.
    pub pos: f64,
    pub on: bool,
}

#[derive(Default)]
pub struct Sound {
    pub feed: Arc<Mutex<Feed>>,
    stream: Option<cpal::Stream>,
    /// Why there is no sound (no output device …), said once.
    pub error: Option<String>,
}

impl App {
    /// The audio to play now and the time in it: the selected node's source at its playhead.
    pub(crate) fn sound_target(&self) -> Option<(Arc<Mutex<Vec<i16>>>, f64)> {
        if !self.ph.playing {
            return None;
        }
        let i = self.selected?;
        let src = if self.nodes[i].kind.is_source() { i } else { self.audio_source(i)?.0 };
        let t = self.playhead_on(&self.clock_of(src)?)?;
        let audio = self.nodes[src].state.lock().ok()?.source.audio.clone();
        Some((audio, t))
    }

    /// Each frame: points the output at the selected node's audio and playhead (it plays on by
    /// itself and is set back when it drifts from the playhead by more than 0.3 s).
    pub(crate) fn sound_tick(&mut self) {
        let sr = corroborate::INPUT_SR;
        let want = self.sound_target();
        {
            let mut f = self.sound.feed.lock().unwrap();
            match &want {
                None => f.on = false,
                Some((a, t)) => {
                    if !f.audio.as_ref().is_some_and(|x| Arc::ptr_eq(x, a)) || (f.pos / sr - t).abs() > 0.3 {
                        f.pos = t * sr;
                    }
                    f.audio = Some(a.clone());
                    f.on = true;
                }
            }
        }
        if want.is_some() && self.sound.stream.is_none() && self.sound.error.is_none() {
            let feed = self.sound.feed.clone();
            match corroborate::sound_out::open(sr, move || {
                let mut f = feed.lock().unwrap();
                if !f.on || f.pos < 0.0 {
                    f.pos += 1.0;
                    return None;
                }
                let i = f.pos as usize;
                f.pos += 1.0;
                let v = f.audio.as_ref()?.lock().unwrap().get(i).copied();
                v.map(|s| s as f32 / 32767.0)
            }) {
                Ok(s) => self.sound.stream = Some(s),
                Err(e) => {
                    self.msg = format!("no sound: {e}");
                    self.sound.error = Some(e);
                }
            }
        }
    }
}
