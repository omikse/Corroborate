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
