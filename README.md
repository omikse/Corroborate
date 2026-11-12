# Corroborate

Live/Static audio-visual forensics using open source data. A node workbench:
sources feed detectors, detectors give findings and areas, and the Result record keeps all of it
with the wires as the evidence trail. Why and where it is going: [VISION.md](VISION.md); how it is
built: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Build and run

Rust, plus ffmpeg on `PATH` (yt-dlp for YouTube links).

**run.cmd** (double-click it, or run it from the repository root) builds the current code and
starts the workbench from `app\`, the tool's own folder. The build is in the window title
(`Corroborate · <build>`): the commit, with `+` when there were uncommitted changes. It works while the app is open: the open
window keeps its build and the next start gets the new one (`tools\run.ps1` says how).

```bash
run.cmd                   # build, update app\, start the workbench (no console window)
run.cmd -NoStart          # build and update app\ only
run.cmd --run             # ... start and press Run on the saved project
run.cmd --new             # ... start with an empty canvas
run.cmd -Release          # ... with the full release build (slow); otherwise the fast one
cargo test --profile fast # same build profile as run.cmd: shares its compiled libraries
```

cargo writes only `target\`; nothing runs from there, so building never waits for an open app.
Release builds have no console window; debug builds keep it.

The tool is one movable folder: everything it keeps lives next to the program (in `app\`), in
`settings\` (the project, planes-view's settings), `results\` (result JSON, flight logs) and
`recordings\`. Nothing goes to system folders.

In the workbench: right-click (or **+ Add node**) to add nodes, drag from a dot to a dot to wire
(types must match). Nodes have no windows: each shows its data and controls on the canvas.

Every node that does work (SOURCER, Cut detector, Audio match, ENF, Planes, plugins) has a state
dot before its name: grey not run, orange stale (it ran on something that has changed since), a
turning grey ring while queued (waiting for another node or a turn), a turning yellow ring while
working, green done, teal kept (loaded from what an earlier run kept, not worked out again), red
failed, brown cancelled, a pulsing red dot while live. Hover it for the step and percent, what it
waits for, how long it took or why it failed. **▶** in its title bar's top right corner runs it and
every node it needs that is not up to date (nothing after it); while it is queued or working it is
a **■**, which cancels it. Its right-click menu: Run, Run again (this node anew, without its kept
result), Cancel, Forget kept result.

One queue for the project (docs/run-queue/plan.md, stage 2): a node starts once what it needs is
done and a turn of its kind is free, in the order asked. A Cut detector needs the SOURCERs it reads;
an Audio match needs the SOURCERs of its files and the Cut detectors whose pieces it takes; ENF,
Planes and plugins read a SOURCER's sound as it decodes, so they run with it (asking for one runs
its SOURCER again, from its kept sound when it has one). A node that fails or is
cancelled ends what needs it at once ("needs 'phone 3', which failed"), and nothing else. Turns:
2 downloads and half the processor cores' worth of decoding and heavy work at once, set in
`settings\run.txt`.

What a node works out is kept in the project's `cache\` folder (stage 3), one file per result,
named by what went into it: the node's kind and method version, its settings that change what it
works out, and what it reads (a file by its size and print, a web video by its link). Kept: a
SOURCER's decoded sound (16 kHz, 16 bit: about 85 MB for 44 minutes) with its waveform, a Cut
detector's scan, an Audio match's result, an ENF's hum track and dating. Opening a project loads
them at once (nothing decodes; the project opens stopped), and a node whose result is kept loads
it instead of working it out (a SOURCER reads its kept sound for the ENF, Planes or plugins on
it). A changed file makes its SOURCER and what follows stale. Two SOURCERs on the same file share
one kept sound. A live source, Planes and plugins are not kept. Deleting `cache\` only costs the
time to work things out again; an untitled project keeps its results in `cache\` next to the
program.

The top bar: the project menu (its button is the project's name, with • while it has unsaved
changes); **▶ Run** (every node that is not up to date; lit while anything runs), **■ Stop**
(cancel everything queued or working), **↻ Run again** (everything anew, without the kept results)
and **Auto** (on by default: a change makes what it touches stale and runs it again half a second
after the last change; a project opens stopped, and Auto waits for Run); **Queue**, with counts
("2 working · 1 queued", after a run "5 done · 1 failed"), which opens the list of every node that
does work, in the order asked: its state, step and percent (with a bar), what it waits for, how
long it took or why it failed, and ▶ or ■ to run or cancel it, and under it how much is kept,
with **Forget all kept**; **+ Add node**, which
also has **Live device** (this computer's microphones and capture cards, asked of ffmpeg when it
opens) and **Value of a type**; **Types**, every data type that travels on wires; **Arrange**; and a warning when recordings are missing or changed,
which opens their list. Messages are at the bottom of the canvas, over the line of keys.

Projects: the project menu has **Rename…**, **New…** (Ctrl+N), **Open…** (Ctrl+O), **Open recent**,
**Save** (Ctrl+S), **Save as…** (Ctrl+Shift+S), **Save a copy…**, **Revert to saved**,
**Recordings** and **Show project folder**. New, Open, Save as and Save a copy all pick a folder, and New and the two saves can make
one: a project is that folder and the folder's name is its only name, with `project.txt` inside,
`results\` (what its Result records write) and `recordings\` (files copied into it); the dialogs start in
`app\projects\`. A folder that already holds another project is refused for New and the saves, so
its file is never written over. **Rename…** puts a box with the name in place of the menu's
button: Enter or clicking away renames its folder, Esc leaves it. The project file changes only on Save; until then changes are kept in
`project.recovery.txt` beside it, which the next Open restores. New, Open and closing the window
ask Save, Don't save or Cancel when there are unsaved changes. A project that was never given a
folder (the first start, or `--new`) is "untitled" and lives in `settings\project.txt` (written as
you work). The project last opened opens at the next start
(`settings\last_project.txt`; the ten last opened are `settings\recent.txt`). `--new` starts with an empty untitled project.

Recordings belong to the project: every file a SOURCER reads is listed under **Recordings** with
its size and a print of its first and last 64 KiB, and stays listed when its SOURCER is deleted
(until Remove). Files stay where they are unless **Copy into project** puts them in the project's
`recordings\`. The project keeps each one's path relative to the project file and its absolute
path; Open looks at the relative path first, then the absolute one, then for a file of the same
size and print in the project folder, and points the SOURCERs at it. Recordings that are missing
(**Locate…**) or changed since they were added (**Accept change**) are marked.

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
| Arrange | the button in the top bar (Clean up before): lays out the nodes on this level by their wires, left to right, the way Graphviz dot and ELK (Eclipse Layout Kernel) do: columns that keep the wires as short as they can be, a lane through every column a long wire passes (so it runs between nodes, not behind them), each column ordered to reduce crossings using the real port heights, then every wire as level as it can be (network simplex). A column far taller than what it feeds (fifty recordings into one Audio match) is dealt into several side by side; separate pieces are packed about the shape of a screen and nodes with no wires are parked underneath. The result depends on the wires and node sizes, not on where the nodes stood, so a second press changes nothing. Ctrl+Z puts them back |
| Wire | drag from an output dot to an input dot, or the other way; right-click an input to unwire |
| Add a node already wired | drag from a dot onto empty canvas: pick from the modules that fit its type (from an input, a Value of its type comes first) |
| Play, pause | Space (or Play on a SOURCER or a Timeline node): the selected node's playhead runs and its sound plays |
| Playhead | click or drag in SOURCER's waveform, or click in a Timeline node |
| Same clock | select sources, right-click: Same clock (Separate clocks undoes it) |
| Audio match's rows | one row per recording: blue is where its audio start time input puts it, amber where audio puts it after the match (the legend in the top bar says so). Under the start time, a thin stripe takes an optional audio end time; unwired it stays grey. Status is split at zero: metadata earlier than audio fills the left half, later the right. Each row gives an audio start time and an audio end time (start + length at the measured drift; a row audio did not link passes its wired end through), blue, marked with an anchor when its group is anchored and ? when it is placed by metadata or by hand. A row with no metadata that audio linked to nothing is greyed out and gives nothing. Drag an amber bar to move an unanchored group: the drag is saved with the project, logged and undone with Ctrl+Z. The anchor button on a row's time anchors its synced group there; the anchor on the group's number tab anchors it on the average of its stamps. Type a phone id beside a row's name: rows with the same id are one phone (one drift, one vote). The clock button on a row's audio makes it the speed reference (its clock runs true). The Drift column gives each row's drift in seconds per hour. Order sorts the rows: group › metadata › start, group › phone › start, start › group, group › how far off, or as wired. Wheel zooms, drag pans, double-click views all. View all fits the axis to every bar; the circle inside it makes that live, so the axis follows the bars until you zoom or pan |
| Time node | + row adds a row; every box shows 0 until changed; a click on a row's sign turns it over; wire a length into a row to add or take away a recording's length; each row's ± adds as squares into Result's ± (root sum of squares) |
| Time output | hover: where the time came from, walked back along the wires, node by node (also in the result file as `traces`) |
| Group | Ctrl+G: the selected nodes into one group node (ports for the wires in and out); Tab or double-click opens it, Tab or the path at the top left goes back out; Alt+G ungroups |
| Group ports | inside a group, wire a node to the empty `+` of Group Input or Group Output; right-click a Group Output port to remove it |
| Uses of a group | Ctrl+D, copy and paste, or Add node → Groups: every use holds the same nodes (a change inside one is made in all), each with its own wires outside; right-click → Make single gives one use its own |

Copied nodes are project-file text, so they paste into another Corroborate window or a text
editor. Keys go to a text box instead while you type in one. Undoing a setting is logged in the
result file like any other setting change.

## Data types

What travels on a wire has a type, and every type is a folder like a module: `src/types/<id>/type.toml`
(in the tool, `app\types\<id>\`, read again every second; a change there wins over the built-in
one). It says the type's `label`, `color`, `flow`, `description` and its `[[field]]`s (name, type
number|text|datetime|bool|choice|file, label, unit, min, max, default, required); the format is at
the top of `src/types.rs`. **Types** in the top bar (and **+ Add node → Value of a type**) lists them all with where each came from;
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
time, and what it touches runs again half a second after the last change (Auto).

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
| [SOURCER](<src/modules/SOURCER/README.md>) | `sourcer` | audio, time, metadata, video and location (GPS in the metadata) from a file, a YouTube video or live stream, a stream URL, or an audio device |
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
| [Map](<src/modules/Map/README.md>) | `map` | locations on a map you can move and zoom, as many as you wire |

Time types: a **time** is `{"clock", "t", "utc"?, "error_s", "source"}`: moment `t` seconds on a
clock, with `utc` (Unix seconds) only when that clock is tied to UTC; `{"utc": …}` alone is UTC. An
**event** list and a **track** carry their `clock`; each event's `error_s` is its own measurement
error. A node that learns when a recording happened gives it as time outputs, usually **audio start
time** and **audio end time**.

## Time: clocks, wires and the Timeline node

Every source (file, stream, device) is its own **clock**; selected sources can share one (Same
clock, e.g. the parts of one recording). A node fed audio is on the clock of that audio's source.

**Times travel on wires, and only on wires.** There is no project-wide pool of evidence and no
global clock solve (removed 2026-11-11): a time reaches a node only when it is wired there, and its
hover walks back along the wires to say where it came from. A time written in a file (its creation
time, or our recorder's sidecar) is only a claim: it goes on SOURCER's time output, raw, with an
error of 0. A live source's own time (stream date-time, PC clock) goes on its time output with its
error. A time made by hand is a Value node of type time.

**Audio match** is the time module: each recording comes in with the time its metadata claims and
goes out with a better one. It fits each group of recordings audio links together from its own
links: every recording's offset and drift, files of one phone (same phone id) sharing one drift.
Which recordings are in a group exists only inside that node. A time is read as the recording's
start; when a file means something else, a **Time** node corrects it, row by row, in the open.
Audio can't say which moment a stamp marks, so a person anchors a group on one time to trust (a
row's, or the average of the group's stamps) and confirms it with other techniques. ENF (power hum)
and ADS-B check give audio start times of their own (ADS-B check also an end, from the speed its
matched aircraft passes measure).

Errors are 1 sigma and combine as root-sum-square. Clocks **drift**: phones run a few to a few
hundred ppm (parts per million) off, shown in seconds per hour. Audio match measures a group's
drifts against the group's average clock, a speed reference recording, or real time from the anchor
recording's wired end; each end time is the start plus the length at the measured drift.

The **Timeline** node (add it by hand, like the Map) only receives and displays: each row takes a
recording's audio or video output, its start time and, if known, its end time (usually Audio match's
outputs), and draws a bar from start to end with margins from their errors. A wire's value is seen
by hovering its dot.

There is one real-time **playhead**, in UTC (Coordinated Universal Time), shared by all Timeline
nodes. A recording follows it only while its start time is wired into a Timeline: moment t is at
start + t, or with the end wired too, t = (playhead − start) × length ÷ (end − start). Every other
recording keeps its own playhead, linked to nothing. Each view shows it on its own clock (SOURCER's
waveform, Planes' view). Space plays the selected node's playhead in real time, with the sound of
its audio. A project with no live source is a **simulation**: every playhead is free. With a live
source it is **live**: its playhead stops at now (the newest audio), and Live jumps back to it.

The result file keeps every node's outputs, each time with its trace back along the wires, and
Audio match's links, fit and findings.

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
- **Map**: everything wired into it on one map you can move and zoom, in the node (pins, tinted
  areas; any number of inputs). See Maps below. World image: NASA Blue Marble (public domain, 2048 × 1024, stored as `earth.idx`, a 256-colour
  palette and one byte per pixel); coastlines: Natural Earth 1:110m (public domain).

### Maps (offline)

The Map node never fetches anything while you work unless you switch Google on. Without map files it draws the built-in world
image (NASA Blue Marble). For street-level detail put `.pmtiles` files (vector tiles, Protomaps
basemap) in `maps\` next to the program; every file there is drawn, in the Protomaps dark style
(walkers). Get one region once with the [pmtiles CLI](https://docs.protomaps.com/pmtiles/cli):

```
pmtiles extract https://build.protomaps.com/<YYYYMMDD>.pmtiles maps\london.pmtiles --bbox=-0.6,51.2,0.4,51.8 --maxzoom=15
```

Each zoom level roughly doubles the size: a city to zoom 15 is tens to a few hundred MB, the
whole world to zoom 6 a few tens of MB. The download itself names the area to the server, so do
it before the investigation, or for a wider box than the place of interest.

### Google maps (online)

The list at the top of a Map node switches every Map node to Google's satellite photos, satellite with names, or road map.
It is the one online part of the map, so it starts on **Offline maps** every time and says
**ONLINE** while on: Google sees which part of the world you look at. It needs your own API key
from [Google Cloud](https://console.cloud.google.com/) with the Map Tiles API enabled, kept in
`settings\google_maps_key.txt` next to the program and never in a project, so you paste it once. The node counts the
tiles it requested since the program started; Google's free allowance is per month. Tiles are
only held in memory, and Google's terms allow viewing them only: no image analysis, no
extracting data, no offline use.

## Other code

- `src/`: `core` (values with error and source), `graph` (typed wires),
  `sync` (clock offsets from shared events), `locate` (TDOA, Time Difference of Arrival),
  `clock` (clock fits: Audio match's groups from its links, ADS-B check's passes), `timebase`
  (UTC anchors of live sources), `plugin` (protocol and
  JSON), `ingest` (ffmpeg, WAV), `sound_out` (sound output).
- `src/bin/corroborate/`: the workbench GUI. `src/main.rs`: the headless CLI (`corroborate-cli`).
- `tools/adsb-log.py`: logs aircraft around a point while you record.

The old pre-workbench version (doppler-view) is tag `v0.1-doppler-live`.
