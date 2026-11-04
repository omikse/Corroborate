# osint-geo-audio

Real-time detection of aircraft Doppler curves in audio. It answers one question quickly: does this
recording or live feed contain propeller-aircraft passes clear enough to be worth an expensive
geolocation run (matching against ADS-B tracks)? Geolocation itself is not part of this project.

## doppler-live

A Rust crate with two binaries:

- `doppler-live`: the lean headless detector (dependencies: `realfft`, `hound`). It prints passes as
  JSON lines.
- `doppler-view`: an optional live viewer (egui) showing the waveform, the scrolling whitened
  spectrogram, a live spectrum slice, raw ridges, held candidates and detected passes as fitted
  curves. It is built only with `--features gui`, so the detector stays small.

ffmpeg must be on `PATH` for anything other than WAV input.

```bash
cd doppler-live
cargo build --release                  # detector only
cargo build --release --features gui   # adds doppler-view
cargo test --release
```

```bash
# A file: WAV, or anything ffmpeg decodes
doppler-live file recording.mp3 --csv points.csv

# Live: a stream URL or file through ffmpeg, or raw f32le mono 4 kHz on stdin
doppler-live listen --url "<stream url>" --events events.jsonl
ffmpeg -i <src> -ac 1 -ar 4000 -f f32le - | doppler-live listen --stdin

# Viewer
doppler-view recording.mp3        # analysed at full speed; scroll the whole timeline
doppler-view "<stream url>"       # live; press REC to keep the timeline and save a WAV

# A 10-minute synthetic test recording
doppler-live synth test.wav
```

For YouTube live streams, get a direct stream URL with `yt-dlp -g -f "ba/b" <video url>`.

### Viewer controls

- **Menu bar**: **Source ▸ Open file…** (native file dialog) or **Open stream…** (paste a URL; YouTube
  links are resolved with yt-dlp). Opening a new source closes the current one cleanly. **Settings ▸
  Recording…** sets the output folder, the format (raw stream copy or decoded WAV) and the file-name
  prefix. **Settings ▸ Theme** switches between light and dark; the plot areas stay dark in both.
  Settings are saved in `%APPDATA%\doppler-view\settings.txt`. `doppler-view` can also start with no
  arguments.
- **Defaults**: light theme, smooth on, audio on. Audio stays as you left it when you switch sources.

- **Recordings**: analysed as fast as the machine allows, so the whole timeline can be scrolled
  straight away. Click the spectrogram to set the playhead, and tick **audio** to listen from there.
  Clicking a pass in the list jumps to it and plays from 20 s before its closest approach.
- **Live streams**: only the visible 160 s is kept. **● REC** keeps the timeline from that moment, with
  no limit, and saves `doppler-<UTC>/` in the folder the app runs from (change it in Settings ▸ Recording…), holding the raw stream copy (`.mka`) and a `.json` description. You can open
  that file later as a recording.
- **smooth**: on, live streams play out through a 4 s buffer and the view scrolls between frames.
  Off is the raw view: data appears exactly as it arrives, network bursts and 0.25 s steps included.
- **audio**: plays the sound as received. A choppy stream sounds choppy.
- Shortcuts (over the spectrogram or the frequency bar):
  - **wheel**: zoom the frequency view, keeping the frequency under the pointer in place
  - **Alt+wheel**: move the frequency view up or down
  - **Ctrl+wheel**: zoom time, from 10 s up to the whole timeline (the box on the timeline bar scales with it)
  - **Shift+wheel** or sideways scroll: scroll time
  - **Page Up / Page Down**: move the frequency view by most of its height
  - **Home** or **0**: go to the start of the timeline
  - **End**: go to the live edge (or the end of a file); the LIVE / FOLLOW button does the same
- Zoomed out past 2048 frames, each column of the spectrogram shows the maximum of the frames it
  covers, so short bright tones stay visible.

### How it works

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

### Modules

All modules read the same decoded stream (16 kHz mono, one shared clock) and each says in the
output how it processed it. Each module lives in its own file under `src/modules/`.

- **planes** (`planes.rs`): aircraft Doppler curves, as described above.
- **enf** (`enf.rs`): ENF (Electrical Network Frequency), the mains hum and its drift. A 16 s window every
  second, a 262 144-point FFT (Fast Fourier Transform) with 0.015 Hz bins, the best of harmonics 1–4 of 50 and
  60 Hz, and a vote for the grid's nominal frequency. Readings that jump more than 30 mHz from the recent median
  are flagged as outliers, kept but not used. Runs of clear hum of 30 s or more are reported. On the reference
  recording it identifies a 60 Hz grid (US) and tracks the drift within about ±40 mHz.

The CLI picks modules with `--modules planes,enf` (the default). The viewer shows both:
- Each module has its own box in the right panel. Click a finding to jump to it. Click the coloured
  header of a box (green for planes, blue for ENF) to show that module's view.
- **View ▸ Full spectrogram** shows the whole 0–8 kHz band. **View ▸ Waveform** and **View ▸ Spectrum slice**
  show or hide the waveform above the spectrogram and the spectrum slice to its right, giving the spectrogram more room.
- The **frequency bar** on the left shows the whole
  0–8 kHz band on a log scale, with passes and hum marked. Its box is the visible band: drag the box to
  move it, drag its top or bottom edge to zoom, or click outside it to centre there.
- Findings outside the visible band show as ▲/▼ arrows on the edge.
- The **minimap** (toggle in the top bar) shows the whole 0–8 kHz band while zoomed in.
- The **ENF** view shows the spectrogram around the hum with the measured hum frequency dashed over it, and a
  drift strip underneath in mHz (millihertz) against the nominal frequency. The drift (tens of mHz) is far
  smaller than one spectrogram bin (0.49 Hz), so it only shows in the strip.

### Event fields

`t_cpa` (closest approach, seconds from stream start), `f0_hz` (rest frequency), `f_pre_hz`/`f_post_hz`
(approach and recede asymptotes), `speed_mps`, `cpa_dist_m` (slant distance), `snr_z` (mean whitened
ridge level), `rms_hz` (fit misfit), `harmonics` (visible partials), `grade`, and `unix_cpa` in live mode.

### Status and limits

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
