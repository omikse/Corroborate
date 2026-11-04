//! Global Viterbi ridge through a whitened spectrogram window.
//!
//! The best path maximises the sum of (clipped) whitened power along it minus a penalty on
//! each bin jump, with jumps limited to `max_jump` bins per frame. Same idea as the Viterbi
//! tracer in hermes-aircraft-doppler-geolocation's `doppler_track.py`.

/// Emission score for a frame/bin: clipped so one loud click cannot drag the path.
#[inline]
fn emit(z: f32) -> f32 {
    z.clamp(-2.0, 6.0)
}

/// Reusable buffers for `viterbi`.
pub struct Viterbi {
    prev: Vec<f32>,
    cur: Vec<f32>,
    bp: Vec<u8>,
}

impl Viterbi {
    pub fn new(max_frames: usize, nb: usize) -> Self {
        Self {
            prev: vec![0.0; nb],
            cur: vec![0.0; nb],
            bp: vec![0; max_frames * nb],
        }
    }

    /// `z` is `t` frames of `nb` bins, row-major by frame. Writes the best bin per frame into
    /// `path` (length `t`).
    pub fn run(
        &mut self,
        z: &[f32],
        t: usize,
        nb: usize,
        max_jump: usize,
        lambda: f32,
        path: &mut Vec<usize>,
    ) {
        assert!(t > 0 && z.len() >= t * nb && max_jump < 128);
        path.clear();
        path.resize(t, 0);
        for b in 0..nb {
            self.prev[b] = emit(z[b]);
        }
        let mj = max_jump as isize;
        for k in 1..t {
            let row = &z[k * nb..(k + 1) * nb];
            let bprow = &mut self.bp[k * nb..(k + 1) * nb];
            for b in 0..nb {
                let mut best = f32::NEG_INFINITY;
                let mut arg = 0u8;
                let lo = (b as isize - mj).max(0);
                let hi = (b as isize + mj).min(nb as isize - 1);
                for p in lo..=hi {
                    let j = (b as isize - p).unsigned_abs() as f32;
                    let s = self.prev[p as usize] - lambda * j;
                    if s > best {
                        best = s;
                        arg = (p - b as isize + mj) as u8;
                    }
                }
                self.cur[b] = best + emit(row[b]);
                bprow[b] = arg;
            }
            std::mem::swap(&mut self.prev, &mut self.cur);
        }
        let mut b = 0;
        let mut best = f32::NEG_INFINITY;
        for (i, &s) in self.prev.iter().enumerate() {
            if s > best {
                best = s;
                b = i;
            }
        }
        for k in (0..t).rev() {
            path[k] = b;
            if k > 0 {
                let off = self.bp[k * nb + b] as isize - mj;
                b = (b as isize + off) as usize;
            }
        }
    }
}

/// Sub-bin peak position around integer bin `b` of one frame by parabolic interpolation.
pub fn subbin(row: &[f32], b: usize) -> f64 {
    if b == 0 || b + 1 >= row.len() {
        return b as f64;
    }
    let (a, c, e) = (row[b - 1] as f64, row[b] as f64, row[b + 1] as f64);
    let den = a - 2.0 * c + e;
    if den >= 0.0 {
        return b as f64;
    }
    b as f64 + (0.5 * (a - e) / den).clamp(-0.5, 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_a_sloped_line() {
        let (t, nb) = (60, 40);
        let mut z = vec![0.0f32; t * nb];
        for k in 0..t {
            z[k * nb + 30 - k / 3] = 5.0;
        }
        let mut v = Viterbi::new(t, nb);
        let mut path = Vec::new();
        v.run(&z, t, nb, 3, 0.3, &mut path);
        for k in 0..t {
            assert_eq!(path[k], 30 - k / 3, "frame {k}");
        }
    }
}
