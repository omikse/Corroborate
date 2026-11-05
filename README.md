# osint-geo-audio

Real-time detection of aircraft Doppler curves in audio. It answers one question quickly: does this
recording or live feed contain propeller-aircraft passes clear enough to be worth an expensive
geolocation run (matching against ADS-B tracks)? Geolocation itself is not part of this project.

## doppler-live

A Rust crate with five binaries:

- `doppler-live`: the lean headless detector (dependencies: `realfft`, `hound`). It prints passes as
  JSON lines.
- `doppler-view`: an optional live viewer (egui) showing the waveform, the scrolling whitened
  spectrogram, a live spectrum slice, raw ridges, held candidates and detected passes as fitted
  curves. It is built only with `--features gui`, so the detector stays small.
- `enf-plugin`: the ENF module as a viewer plugin (see Plugins below).
- `meta-plugin`: a viewer plugin that records what a source says about itself (tags, coordinates, file times).
- `adsb-check`: a test plugin that checks the planes detector against ADS-B flight data.

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
  - **Space**: play or pause the audio (the audio switch)
  - **Left / Right arrows**: move the playhead (files) or the view (streams) by 5 s; **Shift** for 30 s
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

The CLI picks modules with `--modules planes,enf` (the default). In the viewer, planes is built in and
ENF comes as a plugin (`enf-plugin.exe`), imported with **Plugins ▸ Import plugin…**:
- Each module has an eye icon in its header (like the eye in Blender or Photoshop): click it to hide that
  module on the graph, its markers, the frequency bar and the minimap. Its list stays, and the eye opens it again.
- Each module has its own box in the right panel. Click a finding to jump to it. Click the coloured
  header of a box (green for planes, blue for ENF) to show that module's view.
  A plugin's box also shows its status, how its process stands, and any audio it could not keep up with.
- **View ▸ Full spectrogram** shows the whole 0–8 kHz band. **View ▸ Waveform** and **View ▸ Spectrum slice**
  show or hide the waveform above the spectrogram and the spectrum slice to its right, giving the spectrogram more room.
- The **frequency bar** on the left shows the whole
  0–8 kHz band on a log scale, with passes and hum marked. Its box is the visible band: drag the box to
  move it, drag its top or bottom edge to zoom, or click outside it to centre there.
- Findings outside the visible band show as ▲/▼ arrows on the edge.
- The **minimap** (toggle in the top bar) shows the whole 0–8 kHz band while zoomed in.
- **ENF** is the same spectrogram, zoomed in on the hum: the **ENF** header opens it at a 1 Hz span. Zoom
  further (down to 0.2 Hz) to see the drift. The measured hum is always drawn as dashed lines over the
  spectrogram, so the drift shows even where one spectrogram bin (0.49 Hz) is wider than the drift. The
  drift itself is 40 mHz (millihertz) or less, so at the planes zoom it is about a pixel wide.

### Plugins

A plugin is a separate program, written in any language. The viewer starts each plugin that is switched
on when a source opens, sends it the same audio the built-in modules get, and shows what it reports. A
plugin that crashes stops on its own; the viewer carries on and says so in the plugin's box.

- **Plugins ▸ Import plugin…** adds a program; the list there switches plugins on or off and removes them.
  Changes restart the current source so the plugins see it from the first sample (not while recording).
- **Arguments**: `--source <path or URL>` and `--file` or `--live`, so a plugin can read the source's own
  metadata.
- **Audio in** (stdin): raw f32 little-endian, mono, 16 000 Hz, from the first sample. End of input is the
  end of the source. Files wait for every plugin; live streams never do, and audio a plugin could not keep
  up with is counted in its box. A plugin must read stdin to the end, even if it ignores the audio.
- **Messages out** (stdout): one JSON object per line, times in seconds from the first sample:
  `hello` (name, about, colour), `status` (text for its box), `view` (the band its header shows),
  `track` (a point of a measured frequency line, drawn dashed over the spectrogram) and `finding`
  (start, end, band, label, detail: listed in its box, marked on the bars, written to the result JSON) and
  `meta` (a key and value: a fact about the source for the result JSON, listed in its box).
- **stderr**: free text; the last line is shown in its box.

The protocol, with a reader and writers for Rust plugins, is in `src/plugin.rs`. Two complete examples:
- `enf-plugin`: tracks and findings from the audio. On the reference hour it runs in about 5 s on its own.
- `meta-plugin`: no audio analysis, only facts: ffprobe's container, duration, size, codec, every tag
  (title, artist, creation_time…), coordinates from ISO 6709 location tags (phones and cameras write these),
  other streams such as cover art, and the file system's created and modified times. Each value is labelled
  with where it came from. Note that file system times are when the file arrived on this disk, which is
  often not when the audio was recorded.

### Testing against flight data: `adsb-check`

ADS-B (Automatic Dependent Surveillance–Broadcast) data sets the expectations, and the planes detector
confirms them. Every aircraft whose closest approach to the site is within 15 km (the detector's own distance
limit) is an expected pass, due to be heard at its closest approach plus the sound's travel time. Each one is
then:
- **✓ heard**: a detected pass within ±20 s. The Doppler curve that the aircraft's real track predicts (from the
  heard pass's rest frequency) is drawn dashed over the spectrogram: if it lies on the heard curve, it is the
  same aircraft. Its distance and speed are listed next to the Doppler fit's.
- **· not heard**.
- **– beyond detector**: flying outside the detector's 25–150 m/s (airliners at cruise). Listed, not counted.

Passes heard with no aircraft in the data are listed too (planes without ADS-B, or false detections). The
status sums it up as heard/expected by distance: the detector's real range at that site.

It needs:
- the site: MANUAL DATA `coordinates` (lat, lon) and an `elevation` field in metres (add it with **+ field**);
  then switch the plugin off and on so it starts with them;
- the UTC of the audio: recordings made with REC carry it; live sources use this PC's clock;
- flight data: for files, every `adsb*.jsonl` next to the file; live, it polls adsb.lol every 10 s itself, waiting longer when the API asks it to.
  `tools/adsb-log.py` logs the aircraft around a point while you record:
  ```bash
  python tools/adsb-log.py --lat 42.4797 --lon -76.4514 --radius-nm 25 --minutes 80 --out <recording folder>/adsb-around-site.jsonl
  ```
  Free history: adsb.lol publishes a full daily archive (the next day); OpenSky Network has history for
  approved research accounts. Small planes without ADS-B Out, or below receiver coverage, are not in any of it.

### Manual data and the result JSON

- **MANUAL DATA**, a box in the right panel, holds what you know about the source: date, time, place,
  coordinates and notes, plus fields of your own (**+ field**). It is part of the viewer, always on.
- Everything about a source ends up in one JSON: what was analysed and how, the manual data, the passes, and
  each plugin with its state, facts (`meta`) and findings.
  - **Files**: `<file>.doppler.json` next to the file, written when the analysis ends and again whenever a
    plugin or the manual data adds to it (the bottom bar says where it went). Opening the file again loads
    its manual data back.
  - **Recordings** (REC on a live source): the recording's own `.json`, in its `doppler-<UTC>/` folder.

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
