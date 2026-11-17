# Corroborate

A workbench for OSINT researchers to find out where and when a recording was made, from what the
recording itself carries, checked against records from the outside world. Sources feed detectors,
detectors give times and areas, and the Result record keeps all of it, with the wires as the
evidence trail.

## The idea

Every recording carries more than its content. A phone video of a protest also holds the planes
that flew over it, the hum of the power grid it was near, the sounds other people's phones heard
too, the angle of the sun and the timestamps of the stream it went out on. Each of these is a
small, measurable fact about where and when the recording was made. On its own, each fact is weak.
Combined, and checked against outside records, they can establish the place and time of a
recording, line up many recordings of the same event, and locate what happened in it.

## Why it is needed

Newsrooms and research groups reconstruct events by synchronising hundreds of videos and placing
every camera on a map, and locate gunshots from the sound in footage. That work is done by expert
teams, by hand, after the event, over days or weeks. Single techniques exist as separate tools:
aircraft Doppler geolocation, mains hum extraction, sun and shadow angles. General OSINT tools
connect facts about people, domains and addresses, but do not look inside audio or video at all.

What is missing is one place where these signals work together, on recordings and on live streams,
and where every conclusion shows how it was reached.

## Principles

- **Raw and transparent.** Data is shown as it arrives. Nothing is hidden or smoothed without
  saying so. Every number is labelled with where it came from.
- **Every value has an uncertainty.** A time is a time give or take, from a named source.
  Conclusions inherit the uncertainty of what they were built from, and it is always shown. Errors
  are 1 sigma and combine as root-sum-square.
- **Live, files, or both at once.** A live feed and a posted file go into the same nodes, side by
  side in one project.
- **Capture the truth while it exists.** Some evidence is short-lived: aircraft positions, live
  timestamps, a stream that will be deleted. The workbench collects it alongside the recording.
- **People decide.** The tools measure, compare and suggest. A person reviews and makes the call.
- **A result can be checked.** Every number shows the trail of nodes, wires and settings it came
  from, and someone else can open the project with its recordings and run it again to the same
  answer.

## Where it is heading

- **Every node its own program.** Today most nodes are built in and plugins are separate programs
  ([docs/plugins.md](docs/plugins.md)). The aim is that every node is one, in any language, so a
  node that crashes does not take the workbench down and people can build and share their own.
- **Video.** Light and sky (sun angle, shadows, flashes, the flicker of mains lighting), and in the
  long run a 3D reconstruction of the scene built from the footage itself, not from map data laid
  over it.

## Examples

- **A posted video.** Its SOURCER gives the time its file claims. The ENF node hears the grid's hum:
  50 Hz cuts the world to the places that use it, and a match against the grid logs gives the
  audio's start in UTC ± a second. A Time node fixes a camera that wrote local time; a Map shows
  what is left of the world.
- **A protest.** Several people film when a loud bang goes off. Audio match lines the recordings up
  from the sounds they share, a person anchors the group on the time they trust, and each
  recording gets a start and end time. The Timeline shows them side by side.
- **Verifying a live feed.** A public webcam claims to be in a certain town. Planes heard on its
  audio are checked against the flights that actually passed over that town (ADS-B check), which
  also gives the time.
- **A phone call.** Sounds in the background narrow down where the caller was.

A time is a moment in UTC with its error and source, belonging to a recording (usually its start or
end); a location is an area of the Earth, or an exact point. Adding locations up and taking them
away from each other is a node's job (Combine locations). Both travel only on wires: a node knows a time or a place only when
one is wired into it, and hovering it walks back along the wires to where it came from.

## Responsibility

The same tools that locate an event also locate the people recording it, who are often journalists
and protesters. A confident but wrong result, given to someone acting in real time, can do harm. So
results always carry their uncertainty, a person always decides, and every technique is proven on
past events before it is trusted live.

## Build and run

What it needs, each on `PATH`:

| Program | What for | Without it | Install (Windows) |
|---|---|---|---|
| [Rust](https://rustup.rs) | building the tool | nothing builds | `winget install Rustlang.Rustup` |
| [ffmpeg](https://ffmpeg.org) (with ffprobe) | opening video and any audio that is not WAV; reading a file's metadata (date, GPS) | only WAV files open; 18 tests fail | `winget install Gyan.FFmpeg` |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | web video links (YouTube and others) | links do not open | `winget install yt-dlp.yt-dlp` |

Open a new terminal after installing, so it sees the new `PATH`.

**run.cmd** (double-click it, or run it from the repository root) builds the current code and
starts the workbench from `app\`, the tool's own folder. The build is in the window title
(`Corroborate · <build>`): the commit, with `+` when there were uncommitted changes. It works while
the app is open: the open window keeps its build and the next start gets the new one
(`tools\run.ps1` says how).

```bash
run.cmd                   # build, update app\, start the workbench (no console window)
run.cmd -NoStart          # build and update app\ only
run.cmd --run             # ... start and press Run on the saved project
run.cmd --new             # ... start with an empty canvas
run.cmd -Release          # ... with the full release build (slow); otherwise the fast one
cargo test --profile fast # same build profile as run.cmd: shares its compiled libraries
```

Five workbench tests run the example plugin `examples/bus_echo.rs`; build it first with
`cargo build --profile fast --example bus_echo`, or they fail saying `bus_echo.exe is missing`.

cargo writes only `target\`; nothing runs from there, so building never waits for an open app.
Release builds have no console window; debug builds keep it.

How to use the workbench (the canvas, keys, projects, running): [the workbench README](src/bin/corroborate/README.md).
How the code is laid out: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Data types

What travels on a wire has a type. These are shared by every module, so a module made by someone
else feeds Map, Combine locations or the Result record like a built-in one. Each is a folder,
`src/types/<id>/type.toml` (its fields; the format is at the top of `src/types.rs`); their JSON
forms are in the [Template README](src/modules/Template/README.md).

| Type | What it carries |
|---|---|
| `time` | a moment in UTC with its error and where it comes from, belonging to a recording: when it started, ended, or something happened |
| `location` | an area of the Earth, or an exact point |
| `length` | a duration in seconds, such as how long a recording is |
| `event` | timed findings (a clap, a plane passing), each with a start, an end and a label; a list that keeps growing |
| `track` | a measured line over time (a frequency, a level); a list that keeps growing |
| `quality` | how good a detector's signal is: good, weak or none, why, and the numbers behind it |
| `audio` | sound as it is recorded: a stream, not a value |
| `video` | moving pictures as they are recorded: a stream, not a value |
| `text` | any text: a note, a name, a statement |
| `file` | a path to a file on this computer |
| `json` | any JSON document, passed on as it is |
| `any` | an input that takes every type |

A module can bring a type of its own (`types\<id>.toml` in its folder); the Template README says
how.

## Modules

Every node is a module: a folder named by its label under `src/modules/` (in the tool,
`app\modules\`). Dropping a folder in adds the module while the app runs. To make one, copy
[Template](src/modules/Template/README.md).

| Module | What it gives |
|---|---|
| [SOURCER](<src/modules/SOURCER/README.md>) | audio, time, length, metadata, video and location from a file, a web video or live stream, a stream URL, or an audio device |
| [Value](<src/modules/Value/README.md>) | one value of any type, typed in: a place, a time, a note, events |
| [Time](<src/modules/Time/README.md>) | a time corrected in the open, row by row (a camera's local time, a stamp at the end) |
| [Audio match](<src/modules/Audio match/README.md>) | recordings of one event lined up by the sounds they share: each one's start and end time, and its drift |
| [Cut detector](<src/modules/Cut detector/README.md>) | where a recording was cut, and its pieces as recordings of their own |
| [ENF](src/modules/ENF/README.md) | the mains hum: the area of its grid frequency, its quality, the hum track, and the audio's start time from grid logs |
| [Grid frequencies (50 or 60 Hz)](<src/modules/Grid frequencies (50 or 60 Hz)/README.md>) | 50/60 Hz zones by region, for ENF |
| [Planes (Doppler)](<src/modules/Planes (Doppler)/README.md>) | aircraft passes: speed, closest approach, distance |
| [ADS-B check](<src/modules/ADS-B check/README.md>) | flight data against heard passes, and the audio's start and end time from them |
| [Metadata](<src/modules/Metadata/README.md>) | what a file says about itself |
| [Plugin](<src/modules/Plugin/README.md>) | runs a separate program on the same audio |
| [Combine locations](<src/modules/Combine locations/README.md>) | locations added up and taken away: + rows joined (all or any), − rows removed |
| [Invert location](<src/modules/Invert location/README.md>) | everywhere except a location |
| [Map](<src/modules/Map/README.md>) | locations on a map you can move and zoom |
| [Timeline](<src/modules/Timeline/README.md>) | recordings as bars from their start to their end, on one real-time playhead |
| [Result record](<src/modules/Result record/README.md>) | the result JSON and the evidence trail: everything wired into it |

The old pre-workbench version (doppler-view) is tag `v0.1-doppler-live`; its viewer lives on as
`planes-view` ([VIEWER.md](<src/modules/Planes (Doppler)/VIEWER.md>)).
