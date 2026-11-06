# Architecture groundwork

This is the plan for the rewrite on the `rewrite` branch. The concept is in [VISION.md](../VISION.md).
The old stable version (doppler-view) is tag `v0.1-doppler-live`; this repo no longer touches its copy in `C:\Apps\DopplerOsint`.

## Layers

1. **Core (Rust library).** Sample streams, time, anchors, typed values with uncertainty and source.
   No GUI code. Everything that analyses signals lives here or in nodes.
2. **Nodes.** Each node is a separate program (as plugins are now) with typed inputs and outputs.
   Crash isolation and any language stay as they are.
3. **Graph.** Holds nodes and wires. Checks that port types match before running.
4. **Workspace (viewer).** Timeline with lanes, a window per node, and the evidence trail.

## Core types (to build first)

- `Stream`: sample source (file replayed at full speed, or live), with a sample rate and an anchor.
- `Anchor`: a known UTC time for a stream, with its source and uncertainty (for example a stream's
  own PROGRAM-DATE-TIME, a file's metadata, or an overflight match).
- `Value<T>`: a result with `value`, `uncertainty`, `source` (which node and which input data).
- `Event`: a time (with uncertainty) and a label, shown on timeline lanes.
- `Port`: typed input or output. Kinds: `Audio`, `Video`, `Time`, `Location`, `Track`, `Event`,
  `Json`. Each port is `required` or `optional`.

## Node contract

A node describes itself in a `hello` line: name, inputs (kind, required or optional), outputs (kind).
The existing plugin protocol already sends `hello`, `track`, `finding`, `meta` and `view`; the rewrite
adds typed ports and `Value` fields to these messages.

## Migration order

1. Core types and `Value` in the library, with tests.
2. Port the existing modules (planes, ENF, metadata, adsb-check) as nodes using the new types.
3. Timeline with lanes that read `Event` and `Value` outputs.
4. Graph wiring and the evidence trail.
5. Synchronisation node (anchors and shared events into a solver).

## Stack decisions

- egui/eframe is 0.36 on this branch (0.37 had no node editor crates yet). Main stays on 0.37.
- Node editor: wait for egui-snarl or egui_node_editor to support egui 0.36, or patch snarl.
- Keep the plugin protocol working during the rewrite, so existing plugins still run.

## Node hierarchy

- **Sources** (no inputs): *live* (stream URL, microphone, webcam) and *file* (replayed as a stream).
  Also *reference* data (flight history, grid frequency, weather), which is timed but not a stream.
- **Detectors** (audio or video in, timed events or tracks out): aircraft Doppler, hum, bang, flash.
- **Transforms** (convert kinds): text to location, track to location, flight match.
- **Solver** (timed events from several streams in, offsets and locations out).
- **Records** (the result JSON and the evidence trail).

## Timeline rule

Every time says which clock it is on and how sure it is (1 sigma). Each source is a clock, named
by its node's stable id (`n1`, `n2`, … kept in the project file, never reused; the settings log
uses it too), or shared by several sources. `src/clock.rs` solves the project's evidence, anchors
(a moment on a clock is at a UTC) and links (the same moment on two clocks), through `sync::solve`
into each clock's kind (absolute, relative, unanchored), offset, error, group and each piece of
evidence's share of the error; evidence over 3 sigma off is a contradiction. Absolute times sit at
their UTC with the clock error as a margin; relative and unanchored rows float on their own offset
until a link places them.

## Locations always carry a radius

`Location` is a point, a radius in metres and the method that set the radius (GPS, IP, Doppler, shared
sound, metadata, typed in). Two locations that overlap agree; ones that do not overlap contradict each other.

## Implemented so far

`src/core.rs`: `Value`, `Location` (with distance and overlap), `TimeRef`, `PortKind`,
`Port` (required or optional), `Role` and `NodeSpec` (connection and readiness checks). Covered by tests.

`src/types.rs`: data types as folders (`src/types/<id>/type.toml`, the tool's `types\`, and a
module's own `types\<id>.toml`), one shared list; a port's kind is a type's id, and a type's
colour, flow (value, list, stream) and fields give its dots and the Value node's editor.

## Graph and timeline (in the library)

- `graph.rs`: nodes with typed ports and wires. Rules: kinds must match, an input takes one wire,
  an output may feed many inputs, no cycles. Also `missing` (required inputs not yet wired), `order`
  (upstream first) and `trail` (the evidence trail of a node).
- `timeline.rs`: lanes of items with a `TimeRef` and a `Source`. Absolute items have a UTC window
  (time ± error), relative items wait for the solver (`unplaced`). `overlapping` lists items whose
  windows could be the same moment. `strength` is a display choice: narrow margin, strong colour.

- `sync.rs`: the sync solver. Each shared event k heard by stream i gives offset_i + t_ik = T_k (± error,
  which should include the sound travel spread). Anchors fix streams to UTC; without them the first
  stream is the reference. Weighted least squares gives every offset with its error; streams with no
  chain of shared events are reported unsolved; residuals flag wrong matches.

- `locate.rs`: TDOA (Time Difference of Arrival). Arrival times on the shared clock plus recorder
  positions give the source position and time; recorder radius is added to each arrival error; the
  result radius is 2 sigma of the worst direction. Three recorders work but cannot be checked.
- `tests/protest.rs`: four streams, unknown offsets, sync, then a bang located. Flash-synced
  (1/30 s): radius ~15 m. Synced on sounds from unknown places: radius ~790 m. For locating, sync
  needs instant events (light) or sounds from known positions; sound-only sync is too coarse.

- `src/bin/corroborate.rs`: the new workbench GUI (`cargo run --release --features gui --bin corroborate`).
  Now `src/bin/corroborate/`: the node canvas, the clock glow and wire boxes (`clock_view.rs`), the
  evidence and solve (`evidence.rs`), the timeline and Sync list (`timeline.rs`), linked playheads,
  play and live (`playhead.rs`) and sound (`sound.rs`). See the README's Time section.
