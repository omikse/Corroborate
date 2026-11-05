# osint-geo-audio

**Corroborate**: live audio-visual forensics for open-source investigations. A node workbench:
sources feed detectors, detectors give findings and areas, and the Result record keeps all of it
with the wires as the evidence trail. Why and where it is going: [VISION.md](VISION.md); how it is
built: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Build and run

ffmpeg must be on `PATH`; yt-dlp for YouTube links.

```bash
cd doppler-live
cargo build --release --features gui      # corroborate, doppler-view, plugins, CLI
cargo test --release --features gui
```

```bash
target/release/corroborate            # the workbench; the project is kept in %APPDATA%\corroborate\project.txt
target/release/corroborate --run      # open the saved project and press Run
```

In the workbench: right-click to add nodes, drag from an output to an input to wire (types must
match), double-click a node to open its window. Results go to `Desktop\record\corroborate\`.

## Modules

Each module has its own folder under `doppler-live/src/modules/` with its code and a README.

| Module | Folder | What it gives |
|---|---|---|
| Planes (Doppler) | [planes](doppler-live/src/modules/planes/README.md) | aircraft passes: speed, closest approach, distance |
| Power hum (ENF) + Grid frequencies | [enf](doppler-live/src/modules/enf/README.md) | mains hum runs, and the area where that grid frequency is used |
| ADS-B check (plugin) | [adsb-check](doppler-live/src/modules/adsb-check/README.md) | flight data versus heard passes |
| Metadata (plugin) | [meta](doppler-live/src/modules/meta/README.md) | what a file says about itself |

Plugins are separate programs: [docs/plugins.md](docs/plugins.md). The old viewer, still started by
the Planes node: [docs/doppler-view.md](docs/doppler-view.md).

## Location: the world, cut down by evidence

A location is a mask of 1° cells (`doppler-live/src/area.rs`). The Result record starts with the
whole world and keeps only the cells every area wired into its `location` inputs allows. An empty
result is shown as a contradiction, not hidden. The **Map** node draws any location in the Robinson
projection: green where it is still possible, red where it is ruled out, with Natural Earth 1:110m
coastlines (public domain) built into the program.

## Other code

- `doppler-live/src/`: `core` (values with error and source), `graph` (typed wires), `timeline`,
  `sync` (clock offsets from shared events), `locate` (TDOA, Time Difference of Arrival),
  `timebase` (UTC anchors), `plugin` (protocol and JSON), `ingest` (ffmpeg, WAV).
- `doppler-live/src/bin/corroborate/`: the workbench GUI. `doppler-live/src/main.rs`: the headless CLI.
- `tools/adsb-log.py`: logs aircraft around a point while you record.

The stable pre-workbench version is tag `v0.1-doppler-live` on `main`.
