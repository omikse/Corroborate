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

In the workbench: right-click (or **Add node**) to add nodes, drag from a dot to a dot to wire
(types must match). Nodes have no windows: each shows its data and controls on the canvas.

Projects: the top bar shows the project's name beside **File**. **New** empties the canvas,
**Open…** opens a project file and **Save as…** saves this one to a new file (both start in
`app\projects\`); every project saves itself to its own file as you work. Until it is saved
somewhere, the project is "untitled" and lives in `settings\project.txt`; New keeps a non-empty
untitled one as `settings\project.bak.txt`. The project last opened opens at the next start
(`settings\last_project.txt` says which). `--new` starts with an empty untitled project.

Canvas keys and mouse:

| Do | How |
|---|---|
| Zoom | scroll wheel |
| Pan | middle-drag |
| Select | click a node; Shift- or Ctrl-click adds or removes; right-drag a box on empty canvas (with Shift or Ctrl it adds or removes what it touches); Ctrl+A all; Escape none |
| Menus | right-click without moving: on a node, its menu; on empty canvas, Add node |
| Move | drag a selected node: all selected nodes move |
| Delete | Delete or Backspace |
| Copy, cut, paste, duplicate | Ctrl+C, Ctrl+X, Ctrl+V (at the pointer), Ctrl+D |
| Undo, redo | Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z (moves, wires, settings, names, adding, deleting) |
| Frame | F or Home: the selection, else every node |
| Wire | drag from an output dot to an input dot, or the other way; right-click an input to unwire |
| Add a node already wired | drag from a dot onto empty canvas: pick from the modules that fit its type (from an input, a Value of its type comes first) |
| Play, pause | Space (or Play in the top bar or the timeline): the selected node's playhead runs and its sound plays |
| Playhead | click or drag in SOURCER's waveform, or click the timeline's ruler |
| Same clock | select sources, right-click: Same clock (Separate clocks undoes it) |
| Link two clocks | drag a floating timeline row onto an event on another row, then give the error |

Copied nodes are project-file text, so they paste into another Corroborate window or a text
editor. Keys go to a text box instead while you type in one. Undoing a setting is logged in the
result file like any other setting change.

## Data types

What travels on a wire has a type, and every type is a folder like a module: `src/types/<id>/type.toml`
(in the tool, `app\types\<id>\`, read again every second; a change there wins over the built-in
one). It says the type's `label`, `color`, `flow`, `description` and its `[[field]]`s (name, type
number|text|datetime|bool|choice|file, label, unit, min, max, default, required); the format is at
the top of `src/types.rs`. **Types** in the top bar lists them all with where each came from;
hovering one names the modules that give and take it, clicking one adds a Value node of it.

A dot on a node always means the same:

| | means |
|---|---|
| colour | the type |
| shape | how the data arrives: circle a value (the latest replaces the one before: time, location, text, file, JSON), square a list (items keep coming and are kept: event, track), diamond a stream (raw data, not a value: audio, video) |
| filled | needed to run (a required input); optional inputs and every output are open |

Hovering a dot shows its type, what it is, and the value on it now.

**A module bringing a new type:** put `types\<id>.toml` (same format) in the module's folder and
name `<id>` as the `kind` of its ports. Dropping that one folder in adds the module and its type.
Another module that needs the same type reuses the id and need not define it again. If two places
define the same id differently, the Types menu and every node using it say so, and wires of it are
refused until one is changed. Values of a type are checked against its fields as they pass.

## Modules

Every node is a module: a folder named by the module's label, under `src/modules/` (and, in the
tool, under `app\modules\`). Its `module.toml` says what it is: `id` (what project files keep),
`label`, `category` (the Add node heading), `order`, `aliases` (names older files used), `role`,
its ports (`[[input]]`, `[[output]]`: name, kind (a type's id), required; `many = true` on the last input makes it grow as it is wired; an `[[output]]` can say what it is for: `title`, `status` built/planned/possible (planned and possible ones are drawn greyed, nothing to wire), `about`, `needs`, `when`, `source`, and `min_quality` none/weak/good, the signal it needs), its settings (`[[setting]]`: name,
type, default, range or choices) and, for a module that is a separate program,
`[processing] program` and `protocol`. `node.rs` holds what a built-in module does in the
workbench (state while running, canvas, status, timeline marks, result file). The app
(`src/bin/corroborate/`) only draws the graph and calls into the modules.

**Data between modules:** every output can be read as a value of its type (time, location, event,
track, text, json, file, or a type a module brings; audio is the stream itself), and any input of
the same type takes it.
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
- **Ports, settings:** `module.toml` (Result record's inputs follow what is wired, so they are
  made in `mod.rs`).
- **Look on the canvas, without touching code:**
  - `module.toml` `[look]`: the node's `size`, `fill`, corner `radius`, `status = false` to hide the
    status line; for each port (in its `[[input]]`/`[[output]]`, or `[look.ports.<name>]`): `side`
    (left, right, top, bottom), `at` (px along that edge), `label` (inside, outside, hidden),
    `dot_size`. A dot's colour, shape and fill always mean the same (see Data types). Anything
    left out stays as it was.
  - `look.rhai` beside it (a [Rhai](https://rhai.rs) script): `layout(node)` changes the frame and
    ports while the node runs (for example hide a port until a setting is filled), `draw(node)`
    draws inside the node: text, rectangles, circles, lines, editors for the module's settings,
    a map view, and `native()` for the module's own Rust drawing. Scripts only draw: they cannot
    add, remove or wire ports, and a wired port is always shown. A script runs only when what it
    reads changes, never while the canvas pans or zooms; one that fails shows its error on its own
    node. The full list is at the top of `src/bin/corroborate/script.rs`; SOURCER, Combine
    locations, Invert location and Map have one each.
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
| [Value](<src/modules/Value/README.md>) | `value` | one value of any type typed in: a place, a time, a note, events, … |
| [Planes (Doppler)](<src/modules/Planes (Doppler)/README.md>) | `planes` | aircraft passes: speed, closest approach, distance |
| [ENF (power hum)](<src/modules/ENF (power hum)/README.md>) | `enf` | mains hum runs, the area where that grid frequency is used, the hum track, and a quality verdict (good enough to date / weak / no hum, and why); all 16 use cases on its outputs |
| [Grid frequencies (50 or 60 Hz)](<src/modules/Grid frequencies (50 or 60 Hz)/README.md>) | `grid-frequencies` | 50/60 Hz zones by region, for ENF |
| [Plugin](<src/modules/Plugin/README.md>) | `plugin` | runs a separate program on the same audio |
| [ADS-B check](<src/modules/ADS-B check/README.md>) | `adsb-check` | flight data versus heard passes (program `adsb-check.exe`) |
| [Metadata](<src/modules/Metadata/README.md>) | `metadata` | what a file says about itself (program `metadata.exe`) |
| [Result record](<src/modules/Result record/README.md>) | `result-record` | the result JSON, evidence trail and agreed location |
| [Combine locations](<src/modules/Combine locations/README.md>) | `combine-locations` | several locations into one (all or any) |
| [Invert location](<src/modules/Invert location/README.md>) | `invert-location` | flips what a location adds and subtracts |
| [Map](<src/modules/Map/README.md>) | `map` | a location on a world map |

Time types: a **time** is `{"clock", "t", "utc"?, "error_s", "source"}`: moment `t` seconds on a
clock, with `utc` (Unix seconds) only when that clock is tied to UTC; `{"utc": …}` alone is UTC. An
**event** list and a **track** carry their `clock`; each event's `error_s` is its own measurement
error. An **anchor** list (`{t, utc, error_s, clock?, source?}`) says a moment on a clock is at a
UTC; a **link** (`{t_a, b, t_b, error_s, a?}`) says a moment on one clock is a moment on another.
Any module may output anchors and links: they become evidence (below).

## Time: clocks, evidence and the timeline

Every source (file, stream, device) is its own **clock**; selected sources can share one (Same
clock, e.g. photos from one camera). A clock is **absolute** (tied to UTC), **relative** (linked to
other clocks, a group with no UTC) or **unanchored**. Drift is not handled yet, only the offset.

**Evidence** ties clocks down: a source's own anchor (stream date-time, PC clock, file creation
time, sidecar file, each with its error), a time wired into an input marked `anchors = "audio"`
(Planes has one), every anchor and link output in the project (wired or not; ADS-B check gives one
per matched pass), and manual links from the timeline. All of it is solved together by least
squares each time it changes: one link to UTC anchors a whole group, more evidence tightens the
error, and evidence over 3 sigma off the rest is a **contradiction**, shown red. Every piece can be
switched off; the switches are saved in the project and logged in the result file.

Errors are 1 sigma and combine as root-sum-square. Two kinds: the **clock error** (shared by
everything on that clock, what the solve gives) and an event's **own error** (e.g. the Doppler fit's
closest-approach time), which never adds up. Shown at 95 % by default; the timeline switches 68, 95
or 99.7 %, and an optional maximum ± turns anything over it red.

On the canvas each clock group has a soft **glow** behind its nodes and wires in the group's colour:
solid absolute, dashed relative, faint unanchored, striped for a node on several. Wires carrying
times (time, event, track, anchor, link) show a **box** at their middle: the time (UTC, or +mm:ss on
its source; for lists the count and span) over ± the error. Hover it for what the error is made of;
click it to open its row in the timeline.

The **timeline** (panel under the canvas, resizable, collapsible) has one row per clock group,
absolute first on the shared UTC axis, then relative, then unanchored, each opening into one row per
source. Events are marks or bars, the clock error a margin fading out, the own error whiskers. Its
**Sync list** shows every anchor and link: who gave it, its error, its share of the clock's error,
its switch, contradictions in red; a click selects the node. Links are drawn between rows, anchors
pinned to the axis. Drag a floating row so one of its events meets an event on another row: it asks
for the error (filled in from the zoom) and makes a manual link, or an anchor when the other row is
absolute, and the row joins that group.

**Playheads** are linked: every absolute group shares one (UTC), each floating group has its own,
and each view shows it on its own clock (SOURCER's waveform, Planes' view, the timeline). The
selected node's group has the active one. Space plays it in real time, with the sound of the
selected node's audio. A project with no live source is a **simulation**: every playhead is free.
With a live source it is **live**: a live group's playhead stops at now (the newest audio), Live
jumps back to it, and file sources stay free unless linked to a live group.

The result file keeps the clocks (kind, group, offset, error and its breakdown), all evidence with
its switches and contradictions, and each event with its clock and UTC ± error when known. Events
on clock groups with no link between them are listed as `not_comparable`, and the Result record
warns about them.

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
  `clock` (clocks solved from anchors and links), `timebase` (UTC anchors), `plugin` (protocol and
  JSON), `ingest` (ffmpeg, WAV), `sound_out` (sound output).
- `src/bin/corroborate/`: the workbench GUI. `src/main.rs`: the headless CLI (`corroborate-cli`).
- `tools/adsb-log.py`: logs aircraft around a point while you record.

The old pre-workbench version (doppler-view) is tag `v0.1-doppler-live`.
