# Architecture groundwork

This is the plan for the rewrite on the `rewrite` branch. The concept is in [VISION.md](../VISION.md).
The stable version is tagged `v0.1-doppler-live` and copied to `C:\Apps\DopplerOsint`.

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

Anything with a time can be shown. An absolute time (UTC) sits at its real place with its error as a
margin. A relative time (unknown offset) sits on its own clock, with a wide, faded margin until the
solver places it.

## Locations always carry a radius

`Location` is a point, a radius in metres and the method that set the radius (GPS, IP, Doppler, shared
sound, metadata, manual). Two locations that overlap agree; ones that do not overlap contradict each other.

## Implemented so far

`doppler-live/src/core.rs`: `Value`, `Location` (with distance and overlap), `TimeRef`, `PortKind`,
`Port` (required or optional), `Role` and `NodeSpec` (connection and readiness checks). Covered by tests.

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

Not yet: locating an event from arrival times, the viewer using these types, and the timeline UI.
