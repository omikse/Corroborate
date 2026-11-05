# Corroborate

Live/Static audio-visual forensics using open source data. A node workbench:
sources feed detectors, detectors give findings and areas, and the Result record keeps all of it
with the wires as the evidence trail. Why and where it is going: [VISION.md](VISION.md); how it is
built: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Build and run

Rust, plus ffmpeg on `PATH` (yt-dlp for YouTube links).

```bash
cargo build --release     # corroborate, planes-view, corroborate-cli and the plugins
cargo test --release
target/release/corroborate              # the workbench
target/release/corroborate --run        # open the saved project and press Run
target/release/corroborate --demo-enf   # the ENF location demo
```

The tool is one movable folder: everything it keeps lives next to the program, in `settings\`
(the project, planes-view's settings), `results\` (result JSON, flight logs) and `recordings\`.
Nothing goes to system folders.

In the workbench: right-click to add nodes, drag from an output to an input to wire (types must
match), double-click a node to open its window. **Demos** has the synthetic protest scenario and the
ENF location demo.

## Modules

Every node is a module: its own folder under `src/modules/` with its code and a README. `mod.rs`
holds the node's name, label and ports, written only there; `node.rs` holds everything it does in the
workbench (state while running, window, canvas status, timeline marks, result file). The app
(`src/bin/corroborate/`) only draws the graph and calls into the modules.

To change one node, edit only its folder:
- **Ports:** `spec()` in `mod.rs`. Built-in projects skip wires to ports a node no longer has.
- **Look on the canvas:** the `Look` at the end of `node.rs` (methods in `src/bin/corroborate/look.rs`).
  `size` sets the node's size, `fill` its colour, `body` draws inside it (pictures, and controls
  such as buttons or a file picker), `status` (false) hides the status line, and `window` (false) turns off
  the double-click window. The Map node is the example: its map is drawn by `body`. Any node can
  draw a map with `map_node::paint`.

| Node | Folder | What it gives |
|---|---|---|
| File | [file](src/modules/file/README.md) | audio and time from a recording on disk |
| Stream | [stream](src/modules/stream/README.md) | audio and time from a URL, live |
| Microphone | [mic](src/modules/mic/README.md) | audio and time from a capture device |
| Manual data | [manual](src/modules/manual/README.md) | facts typed in: date, time, place, coordinates, notes |
| Planes (Doppler) | [planes](src/modules/planes/README.md) | aircraft passes: speed, closest approach, distance |
| Power hum (ENF) | [enf](src/modules/enf/README.md) | mains hum runs, and the area where that grid frequency is used |
| Grid frequencies | [grid_frequencies](src/modules/grid_frequencies/README.md) | 50/60 Hz zones by region, for ENF |
| Plugin | [plugin](src/modules/plugin/README.md) | runs a separate program on the same audio |
| ADS-B check (plugin) | [adsb-check](src/modules/adsb-check/README.md) | flight data versus heard passes |
| Metadata (plugin) | [meta](src/modules/meta/README.md) | what a file says about itself |
| Result record | [record](src/modules/record/README.md) | the result JSON, evidence trail and agreed location |
| Combine locations | [combine](src/modules/combine/README.md) | several locations into one (all or any) |
| Invert location | [invert](src/modules/invert/README.md) | flips what a location adds and subtracts |
| Map | [map](src/modules/map/README.md) | a location on a world map |

Plugins are separate programs: [docs/plugins.md](docs/plugins.md). The old viewer, now `planes-view`, is a
separate program that nothing opens by itself: [planes/VIEWER.md](src/modules/planes/VIEWER.md).

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

- `src/`: `core` (values with error and source), `graph` (typed wires), `timeline`,
  `sync` (clock offsets from shared events), `locate` (TDOA, Time Difference of Arrival),
  `timebase` (UTC anchors), `plugin` (protocol and JSON), `ingest` (ffmpeg, WAV).
- `src/bin/corroborate/`: the workbench GUI. `src/main.rs`: the headless CLI (`corroborate-cli`).
- `tools/adsb-log.py`: logs aircraft around a point while you record.

The old pre-workbench version (doppler-view) is tag `v0.1-doppler-live`.
