# Planes (Doppler)

Aircraft Doppler curves: does this audio hold propeller-aircraft passes clear enough to geolocate by?
Node: audio and an optional time in, passes out. Its window shows the spectrogram; it also starts
`planes-view` (the old viewer, in its own window; see [VIEWER.md](VIEWER.md)) on the same source.

Files: `mod.rs` (the module: name, label, ports), `spectro.rs` (whitened STFT), `ridge.rs` (Viterbi ridges),
`fit.rs` (straight-pass Doppler model and fit), `detect.rs` (gates, merging, events), `node.rs` (the node in
the workbench: state while running, spectrogram window, timeline marks, result file), `view.rs` (`planes-view`).

## How it works

1. **Spectrogram**: 8192-point STFT at 4 kHz (0.49 Hz bins), 0.25 s hop, 40–450 Hz. Each bin is whitened
   against a running median and MAD, so steady tones fade out. A per-frame median removes broadband bursts.
2. **Ridges**: every 5 s, a global Viterbi path is found through the last 160 s (jumps limited to 4 bins
   per frame, with a penalty per bin). Up to three ridges are peeled off in turn.
3. **Fit**: bright runs of 30 s or more are fitted with a straight-pass Doppler model
   `f(t) = f0·c / (c + v²(t−tc)/√(d² + v²(t−tc)²))` by a robust Levenberg–Marquardt fit (several starts,
   with refits on inliers).
4. **Gates**: SNR, misfit, plausible speed and distance, at least 10 s observed on each side of closest
   approach, closest approach itself observed, a transition long enough to resolve (d/v ≥ 4 s), and a
   smooth monotonic fall (this rejects engines changing RPM).
5. **Reporting**: candidates are held for 10 s. Overlapping curves (the same pass found again, on a harmonic,
   or as a tail) are merged and the best version is reported. Each event gets a provisional grade.

## Event fields

`t_cpa` (closest approach, seconds from stream start), `f0_hz` (rest frequency), `f_pre_hz`/`f_post_hz`
(approach and recede asymptotes), `speed_mps`, `cpa_dist_m` (slant distance), `snr_z` (mean whitened
ridge level), `rms_hz` (fit misfit), `harmonics` (visible partials), `grade`, and `unix_cpa` in live mode.

## Status and limits

- Synthetic tests: passes are recovered with the right parameters; noise and hum give no events.
- One real 60-minute recording: 3 passes graded high, all of which look like Doppler curves in the
  spectrogram, and 2 weak ones graded low. No ground truth (ADS-B) has been checked yet.
- The grade thresholds come from that single recording and are provisional.
- Speeds and distances come from a straight-line, constant-speed model in still air. They are good for
  detection, but approximate as measurements.
- Phone-call audio (300–3400 Hz) is not tested yet. Most of the 60–300 Hz blade-pass energy is cut on
  that channel.

Method references: hermes-aircraft-doppler-geolocation (Viterbi ridge + curve fit), dopplertrack
(whitened spectrogram, straight-pass model), franken_doppler_finder (two-sided checks, ridge
requirement), all independent implementations of colsto's method.
