# Corroborate

Live/Static audio-visual forensics using open source data. A node workbench:
sources feed detectors, detectors give findings and areas, and the Result record keeps all of it
with the wires as the evidence trail. Why and where it is going: [VISION.md](VISION.md); how it is
built: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Build and run

Rust, plus ffmpeg on `PATH` (yt-dlp for YouTube links).

**run.cmd** (double-click it, or run it from the repository root) builds the current code and
starts the workbench from `app\`, the tool's own folder. The build is shown in the top bar: the
commit, with `+` when there were uncommitted changes. It works while the app is open: the open
window keeps its build and the next start gets the new one (`tools\run.ps1` says how).

```bash
run.cmd                   # build, update app\, start the workbench (no console window)
run.cmd -NoStart          # build and update app\ only
run.cmd --run             # ... start and press Run on the saved project
run.cmd --new             # ... start with an empty canvas
cargo test --release
```

cargo writes only `target\`; nothing runs from there, so building never waits for an open app.
Release builds have no console window; debug builds keep it.

The tool is one movable folder: everything it keeps lives next to the program (in `app\`), in
`settings\` (the project, planes-view's settings), `results\` (result JSON, flight logs) and
`recordings\`. Nothing goes to system folders.

In the workbench: right-click to add nodes, drag from an output to an input to wire (types must
match). Nodes have no windows: each shows its data and controls on the canvas. With no saved project the
canvas starts empty; `--new` opens an empty one (the old project is copied to `settings\project.bak.txt`).

## Modules

Every node is a module: a folder named by the module's label, under `src/modules/` (and, in the
tool, under `app\modules\`). Its `module.toml` says what it is: `id` (what project files keep),
`label`, `category` (the Add node heading), `order`, `aliases` (names older files used), `role`,
its ports (`[[input]]`, `[[output]]`: name, kind, required), its settings (`[[setting]]`: name,
type, default, range or choices) and, for a module that is a separate program,
`[processing] program` and `protocol`. `node.rs` holds what a built-in module does in the
workbench (state while running, canvas, status, timeline marks, result file). The app
(`src/bin/corroborate/`) only draws the graph and calls into the modules.

**Data between modules:** every output can be read as a value of its kind (time, location, event,
track, text, json, file; audio is the stream itself), and any input of the same kind takes it.
One JSON form per kind (`src/data.rs`) is what programs send and receive and what the Result
record stores, so a module made by someone else feeds Map, Combine, Invert or the record like a
built-in one. Settings are the only way the canvas changes processing: each value is checked
against the module.toml before it is used, every change is written to the result file with its
time, and the graph restarts half a second after the last change.

**Adding a module:** drop its folder into `app\modules\`. The workbench reads module folders every
second: the module appears in Add node under its category. A folder with a program
(`[processing] program = "x.exe"`, plugin protocol: [docs/plugins.md](docs/plugins.md)) runs as a
plugin; with `protocol = 2` it is on the data bus, with the ports and settings its module.toml
declares (example: `examples/bus_echo.rs`). A project using a module the tool does not have keeps
it as a grey placeholder with its settings and wires, and saves it back unchanged.

To change one node, edit only its folder:
- **Name, menu place:** `module.toml` (in `app\modules\<label>\` it takes effect while the program runs).
- **Ports, settings:** `module.toml` (SOURCER's and Result record's ports follow what is opened or
  wired, so theirs are made in `mod.rs`).
- **Look on the canvas, without touching code:**
  - `module.toml` `[look]`: the node's `size`, `fill`, corner `radius`, `status = false` to hide the
    status line; for each port (in its `[[input]]`/`[[output]]`, or `[look.ports.<name>]`): `side`
    (left, right, top, bottom), `at` (px along that edge), `label` (inside, outside, hidden), `dot`
    (circle, square, diamond), `dot_size`, `color`. Anything left out stays as it was.
  - `look.rhai` beside it (a [Rhai](https://rhai.rs) script): `layout(node)` changes the frame and
    ports while the node runs (for example hide a port until a setting is filled), `draw(node)`
    draws inside the node: text, rectangles, circles, lines, editors for the module's settings,
    a map view, and `native()` for the module's own Rust drawing. Scripts only draw: they cannot
    add, remove or wire ports, and a wired port is always shown. A script runs only when what it
    reads changes, never while the canvas pans or zooms; one that fails shows its error on its own
    node. The full list is at the top of `src/bin/corroborate/script.rs`; Invert location and Map
    have one each.
  - In `app\modules\<label>\` both files take effect within a second of saving.
- **Look in Rust:** the `Look` at the end of `node.rs` (methods in `src/bin/corroborate/look.rs`).
  `size` sets the node's size, `fill` its colour, `body` draws inside it (pictures, and controls
  such as buttons or a file picker), and `status` (false) hides the status line. The Map node is the
  example: its map is drawn by `body`. Any node can draw a map with `map_node::paint`.
- **Collapse:** right-click any node → Collapse leaves its title and port names (saved with the
  project).

| Module (folder) | id | What it gives |
|---|---|---|
| [SOURCER](<src/modules/SOURCER/README.md>) | `sourcer` | audio, time, metadata and video from a file, a YouTube video or live stream, a stream URL, or an audio device |
| [Manual data](<src/modules/Manual data/README.md>) | `manual-data` | facts typed in: date, time, place, coordinates, notes |
| [Planes (Doppler)](<src/modules/Planes (Doppler)/README.md>) | `planes` | aircraft passes: speed, closest approach, distance |
| [ENF (power hum)](<src/modules/ENF (power hum)/README.md>) | `enf` | mains hum runs, and the area where that grid frequency is used |
| [Grid frequencies (50 or 60 Hz)](<src/modules/Grid frequencies (50 or 60 Hz)/README.md>) | `grid-frequencies` | 50/60 Hz zones by region, for ENF |
| [Plugin](<src/modules/Plugin/README.md>) | `plugin` | runs a separate program on the same audio |
| [ADS-B check](<src/modules/ADS-B check/README.md>) | `adsb-check` | flight data versus heard passes (program `adsb-check.exe`) |
| [Metadata](<src/modules/Metadata/README.md>) | `metadata` | what a file says about itself (program `metadata.exe`) |
| [Result record](<src/modules/Result record/README.md>) | `result-record` | the result JSON, evidence trail and agreed location |
| [Combine locations](<src/modules/Combine locations/README.md>) | `combine-locations` | several locations into one (all or any) |
| [Invert location](<src/modules/Invert location/README.md>) | `invert-location` | flips what a location adds and subtracts |
| [Map](<src/modules/Map/README.md>) | `map` | a location on a world map |

Plugins are separate programs: [docs/plugins.md](docs/plugins.md). The old viewer, now `planes-view`, is a
separate program that nothing opens by itself: [VIEWER.md](<src/modules/Planes (Doppler)/VIEWER.md>).

## Location: the world, cut down by evidence

A location is a mask of 1° cells (`src/area.rs`). The Result record starts with the
whole world and keeps only the cells every area wired into its `location` inputs allows. An empty
result is shown as a contradiction, not hidden.

Every location signal either **adds** its cells (the place is here: green) or **subtracts** them
(the place is not here: red). Location nodes:
- **Combine locations**: what several locations still allow, all of them (AND) or any (OR).
- **Invert location**: flips adds and subtracts on the same cells.
- **Map**: draws a location over the Earth in the Robinson projection, in the node and in its
  window. Image: NASA Blue Marble (public domain, 2048 × 1024, stored as `earth.idx`, a 256-colour
  palette and one byte per pixel); coastlines: Natural Earth 1:110m (public domain).

## Other code

- `src/`: `core` (values with error and source), `graph` (typed wires),
  `sync` (clock offsets from shared events), `locate` (TDOA, Time Difference of Arrival),
  `timebase` (UTC anchors), `plugin` (protocol and JSON), `ingest` (ffmpeg, WAV).
- `src/bin/corroborate/`: the workbench GUI. `src/main.rs`: the headless CLI (`corroborate-cli`).
- `tools/adsb-log.py`: logs aircraft around a point while you record.

The old pre-workbench version (doppler-view) is tag `v0.1-doppler-live`.
