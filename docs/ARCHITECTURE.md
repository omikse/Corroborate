# Code map

How the code is laid out, for anyone changing it. What the tool is for is in the
[main README](../README.md); how to use the app is in the [workbench README](../src/bin/corroborate/README.md).
Each file starts with a comment saying what it holds; this page only says where to look.

## Layers

1. **The library** (`src/`, crate `corroborate`): signal work and the data model, no GUI.
2. **Modules** (`src/modules/<Label>/`): one folder per node kind, holding its `module.toml`,
   README, code and data. A built-in module's `node.rs` is what it does in the workbench.
3. **Types** (`src/types/<id>/type.toml`): what travels on wires.
4. **The workbench** (`src/bin/corroborate/`): draws the graph and calls into the modules.
5. **Separate programs**: plugins and the CLI, built from the same crate.

Nodes fall into a few roles (`role` in `module.toml`): sources (a file, a stream, a device),
reference data (grid frequencies, a typed value), detectors (audio or video in, findings out),
transforms (one kind into another: correcting a time, combining areas), records (the result file)
and views (Map, Timeline).

## The library

| File | Holds |
|---|---|
| `data.rs` | the data bus: one value per type and its JSON form |
| `types.rs` | types as folders: reading `type.toml`, the format at the top |
| `module_manifest.rs` | `module.toml`: every key, explained at the top |
| `modules.rs`, `module.rs` | the module list and the interface built-in detectors share (16 kHz mono in, findings out) |
| `graph.rs` | nodes, typed ports and wires; the checks before anything runs |
| `core.rs` | values with error and source, port kinds, node roles |
| `area.rs` | a location as a mask of cells that adds or subtracts |
| `clock.rs`, `sync.rs` | fitting recordings' offsets and speeds from shared moments (Audio match, ADS-B check) |
| `audio_match.rs` | which recordings hear the same sound, and the method's measurements |
| `cuts.rs` | where a recording is cut (Cut detector) |
| `locate.rs` | a sound's position from its arrival times (TDOA, Time Difference of Arrival) |
| `ingest.rs`, `timebase.rs`, `sound_out.rs` | reading audio (WAV, ffmpeg), live sources' UTC, playing sound |
| `plugin.rs` | the plugin protocol ([docs/plugins.md](plugins.md)) with a reader and writers for Rust plugins |
| `node_layout.rs`, `node_layout/simplex.rs` | Arrange: the layered layout and network simplex |
| `synth.rs` | synthetic test audio |

## The workbench (`src/bin/corroborate/`)

| File | Holds |
|---|---|
| `main.rs` | the window, the top bar, drawing the graph |
| `nodes.rs` | the node kinds; the matches on `Kind` only call into each module's `node.rs` |
| `canvas.rs` | selecting, moving, copying, undo |
| `groups.rs` | group nodes |
| `drop_menu.rs`, `types_menu.rs` | the menu for a wire dropped on empty canvas; the Types menu |
| `look.rs`, `layout.rs`, `script.rs` | how a node looks: its `Look`, `[look]` in `module.toml`, `look.rhai` |
| `bus.rs` | reading any output as a value of its type |
| `runtime.rs` | running a graph: decoding sources and feeding what they wire into |
| `jobs.rs`, `queue.rs`, `kept.rs` | job state, the queue, kept results ([docs/run-queue/plan.md](run-queue/plan.md)) |
| `evidence.rs`, `trace.rs` | which recording a node's times belong to; a time's trace back along the wires |
| `playhead.rs`, `sound.rs` | the playheads and playing sound |
| `project.rs`, `recordings.rs` | projects and their recordings |
| `google_maps.rs` | Google map tiles for the Map node |
| `drive.rs`, `cleanup_lab.rs`, `*_tests.rs` | headless tests that drive the app like a user |

## Programs

| Binary | From | What |
|---|---|---|
| `corroborate` | `src/bin/corroborate/main.rs` | the workbench (feature `gui`) |
| `corroborate-cli` | `src/main.rs` | headless: detectors on a file or stream, synthetic audio, `grid-logs` import |
| `planes-view` | `src/modules/Planes (Doppler)/view.rs` | the old viewer ([VIEWER.md](<../src/modules/Planes (Doppler)/VIEWER.md>)) |
| `enf-plugin`, `metadata`, `adsb-check` | their module folders | plugins |

`examples/bus_echo.rs` is a complete protocol 2 plugin; `tools/` holds `run.ps1` (build and deploy
to `app\`), `adsb-log.py` and `jiku.py`.

## Tests

`tests/`: `synthetic.rs` (detectors on made-up audio), `protest.rs` (sync and TDOA on four made-up
streams), `cuts.rs`, `jiku.rs` and `jiku_cuts.rs` (the Jiku dataset, fetched by `tools/jiku.py`).
The workbench's own tests are the `*_tests.rs` files beside its code. Run them with
`cargo test --profile fast`.

## Decisions that shape the code

- Times and locations travel only on wires: there is no project-wide pool of evidence and no
  global solve ([docs/time-on-wires/plan.md](time-on-wires/plan.md)).
- A module is a folder; dropping one into `app\modules\` adds it while the app runs.
- Nodes have no windows: each draws itself on the canvas.
- Settings are the only way the canvas changes processing, and every change is logged.
- egui/eframe 0.36.
