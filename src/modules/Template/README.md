# Template (role)

What question the node answers, in one paragraph and in an investigator's words. Status: built /
partly built / planned. Plan: a link to `docs/<feature>/plan.md`, if there is one.

Copy this folder to start a module: rename it to the module's label, set `id`, `label` and
`category` in `module.toml`, and replace every line here. The layout is explained in
[docs/module-docs/plan.md](../../../docs/module-docs/plan.md). The two sections after this one
(Making a module, Data on the wires) are for whoever makes a module: delete them in your copy.

## Making a module

A module is its folder. Drop it into `app\modules\` and the workbench reads it within a second: it
appears in Add node under its `category`. Every key of `module.toml` is explained at the top of
`src/module_manifest.rs`.

- **Built in:** Rust in this folder (`mod.rs`, `node.rs`), listed in `src/bin/corroborate/nodes.rs`.
- **A separate program:** `[processing] program = "x.exe"` and `protocol = 2` in `module.toml`; it
  gets the ports and settings `module.toml` declares and talks JSON lines
  ([docs/plugins.md](../../../docs/plugins.md); a complete one: `examples/bus_echo.rs`).
- **A type of its own:** put `types\<id>.toml` (the format of `src/types/<id>/type.toml`) in this
  folder and name `<id>` as the `kind` of its ports. Another module that needs it reuses the id.
  If two places define the same id differently, the Types menu and every node using it say so, and
  wires of it are refused until one is changed.
- **Its look, without code:** `[look]` in `module.toml` (the node's `size`, `fill`, corner `radius`,
  `status = false`; per port `side`, `at`, `label`, `dot_size`), and `look.rhai` beside it (a
  [Rhai](https://rhai.rs) script: `layout(node)` changes the frame and ports while the node runs,
  `draw(node)` draws inside it; the full list is at the top of `src/bin/corroborate/script.rs`).
  Scripts only draw: they cannot add, remove or wire ports, and a wired port is always shown. In
  `app\modules\<label>\` both take effect within a second of saving.
- **Its look, in Rust:** the `Look` at the end of `node.rs` (`src/bin/corroborate/look.rs`): `size`,
  `fill`, `body` (anything drawn inside, such as a map with `map_node::paint`), `status`.

A project using a module the tool does not have keeps it as a grey placeholder with its settings
and wires, and saves it back unchanged.

## Data on the wires

Every output can be read as a value of its type, and any input of that type takes it. Each type
has one JSON form (`src/data.rs`): what programs send and receive, and what the Result record
stores. The types themselves are listed in the [main README](../../../README.md#data-types).

| Type | JSON |
|---|---|
| `time` | `{"utc": 1793839234.5, "error_s": 0.25, "source": "where it comes from", "recording": "n3"}`: UTC in Unix seconds, the error 1 sigma, and the recording it belongs to (left out when none; old `"clock"` is read as it) |
| `location` | an area: `{"add": true, "cells": [[row, first_col, last_col], …], "note": "…"}`; a point: `{"lat": 50.1, "lon": 14.4, "elevation": 335}`, with `"radius_km"` for a circle around it |
| `event` | a list: `[{"t_start": 12.0, "t_end": 13.5, "label": "…", "detail": "…", "error_s": 0.02}]`, in seconds into the recording |
| `track` | a list: `[{"id": "hum", "t": 12.0, "value": 50.012, "ok": true}]` |
| `text`, `file` | a JSON string |
| `json` | any JSON |
| `length`, `quality`, a module's own type | an object with the type's fields (its `type.toml`), checked as it passes |
| `audio`, `video` | streams, not values: audio reaches a program as `{"type":"audio","pcm":"<base64 f32 LE mono 16 kHz>"}` |

Errors are 1 sigma and combine as root-sum-square. A location's cells are 1° today (row 0 is
90° N, column 0 is 180° W).

## Inputs

One row per input in `module.toml`.

| Input | Kind | Required | What it's for |
|---|---|---|---|
| `audio` | audio | yes | the sound the node reads |
| `time` | time | no | what it changes when wired |

## Steps

The work between inputs and outputs, in order. A step that needs more than a row gets a note
under Notes, linked from its row.

| Step | From | Does | Settings |
|---|---|---|---|
| S1 Name | `audio` | one line | value (default, and why) |
| S2 Name | S1, `time` | one line ([notes](#s2-name)) | — |

## Outputs

One row per output in `module.toml`, planned ones too. Tested says on what and the measured
number, or "not yet".

| Output | From | Error | Tested |
|---|---|---|---|
| `events` | S2 | how big, and where it comes from | test case: measured number |
| `location` | — | — | planned |

## Limits

What it cannot do or gets wrong, and what a wrong result looks like.

## Files

| File | Holds |
|---|---|
| `module.toml` | id, label, ports |
| `README.md` | this |

## Tests

| Test | Proves |
|---|---|
| `file.rs::test_name` | what it checks |

<!-- Optional sections, only when needed; delete the rest:

## Use
What the node shows on the canvas and what a person decides.

## Notes
### S2 Name
More than a row can hold.

## Data
Reference data: what it holds, source, licence, date, accuracy.

## Sources
| Source | Taken from it |
|---|---|

## Next
Planned work, linking to a plan.
-->
