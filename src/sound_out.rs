//! Sound out: plays a mono signal on the default output device, resampled linearly from `rate` to
//! the device's rate, the same value on every channel. `next` gives the next sample, or None for
//! silence (gaps in the input are heard as gaps). Used by the workbench and Planes' viewer.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub fn open(rate: f64, next: impl FnMut() -> Option<f32> + Send + 'static) -> Result<cpal::Stream, String> {
    let dev = cpal::default_host().default_output_device().ok_or("no audio output device")?;
    let sup = dev.default_output_config().map_err(|e| e.to_string())?;
    let ch = sup.channels() as usize;
    let step = rate / sup.sample_rate() as f64;
    let cfg = sup.config();
    let err = |e| eprintln!("audio output: {e}");
    let stream = match sup.sample_format() {
        cpal::SampleFormat::F32 => dev.build_output_stream::<f32, _, _>(cfg, render(next, ch, step, |v| v), err, None),
        cpal::SampleFormat::I16 => dev.build_output_stream::<i16, _, _>(cfg, render(next, ch, step, |v| (v * 32767.0) as i16), err, None),
        cpal::SampleFormat::U16 => dev.build_output_stream::<u16, _, _>(cfg, render(next, ch, step, |v| ((v * 32767.0) + 32768.0) as u16), err, None),
        f => return Err(format!("unsupported output sample format {f:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn render<T: Copy>(
    mut next: impl FnMut() -> Option<f32> + Send + 'static,
    ch: usize,
    step: f64,
    conv: impl Fn(f32) -> T + Send + 'static,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static {
    let (mut pos, mut a, mut b) = (0.0f64, 0.0f32, 0.0f32);
    move |data: &mut [T], _| {
        for frame in data.chunks_mut(ch) {
            pos += step;
            while pos >= 1.0 {
                pos -= 1.0;
                a = b;
                b = next().unwrap_or(0.0);
            }
            let v = conv((a + (b - a) * pos as f32).clamp(-1.0, 1.0));
            for s in frame.iter_mut() {
                *s = v;
            }
        }
    }
}
