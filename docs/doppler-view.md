# doppler-view (the old viewer)

The stable viewer from before the node workbench (tag `v0.1-doppler-live`), still built from
`doppler-live/src/bin/doppler-view.rs`. The Planes node starts it on the same source.

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

## Viewer controls

- **Menu bar**: **Source ▸ Open file…** (native file dialog) or **Open stream…** (paste a URL; YouTube
  links are resolved with yt-dlp). Opening a new source closes the current one cleanly. **Settings ▸
  Recording…** sets the output folder, the format (raw stream copy or decoded WAV) and the file-name
  prefix. **Settings ▸ Theme** switches between light and dark; the plot areas stay dark in both.
  `doppler-view` can also start with no arguments.
- **Settings file**: one file, `%APPDATA%\doppler-view\settings.txt`, keeps the theme, the recording folder,
  format and prefix, the view toggles and the plugins, and is loaded at every start. The app rewrites it
  whenever you change something; **Settings ▸ Show settings file** opens its folder. Edit it by hand only
  while the app is closed.
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


## Manual data and the result JSON

- **MANUAL DATA**, a box in the right panel, holds what you know about the source: date, time, place,
  coordinates and notes, plus fields of your own (**+ field**). It is part of the viewer, always on.
- Everything about a source ends up in one JSON: what was analysed and how, the manual data, the passes, and
  each plugin with its state, facts (`meta`) and findings.
  - **Files**: `<file>.doppler.json` next to the file, written when the analysis ends and again whenever a
    plugin or the manual data adds to it (the bottom bar says where it went). Opening the file again loads
    its manual data back.
  - **Recordings** (REC on a live source): the recording's own `.json`, in its `doppler-<UTC>/` folder.
