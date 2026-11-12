# Running, the queue, and keeping results

Status: planned on 2026-11-12 and settled in an interview the same day (see Decisions); nothing
built. The mockup: [mockup.html](mockup.html) (open it in a
browser; it runs two made-up projects, Simple and Difficult, so every state can be seen).

## The question it answers

How do I run one node, see what the program is doing and how far it has got, and not wait for the
same work twice?

What prompted it, on a project with a YouTube link in a SOURCER wired into a Cut detector:
- Downloading looked like a live stream (fixed in 22f9f14: a link is live only once yt-dlp says so).
- The Cut detector did not know what to do with a link (fixed in 26e2fda: it waits for the
  download and reads the copy).
- There is no way to run one node: Run starts everything.
- There is no single place to see what waits, what works and what is done.
- Every Run does every piece of work again.

Two words are kept apart in this plan:
- **Run**: work out results (download, decode, scan, match).
- **Play**: move a playhead in real time and hear the sound. Playing never computes anything
  itself; it can only ask for a run, when there is no sound to play yet.

## What we have now (internal survey)

| What | Where | Today |
|---|---|---|
| Run, Stop, Restart | top bar, `main.rs` `run`, `stop_run` | Run starts every source and every Audio match and Cut detector at once. Stop cancels all of it. Restart is Run again. |
| Restart on change | `keep_running`, `run_signature` | While running, any change to a source's input, to what its audio feeds, or to a program's settings restarts the whole graph after 0.5 s. |
| Turns to decode | `ingest::decode_turn` | At most half the processor cores decode at once (SOURCERs, Audio match, Cut detector share it). A turn is waited for silently. |
| Waiting for another node | Audio match and Cut detector (`from_source`, `wait_copy`, `pending`) | Each node waits in its own way: for a source's run to end, for a download's copy, for a Cut detector's pieces. |
| Progress | each node's `status` line; the Cut detector's step row (26e2fda) | Only the Cut detector shows steps with percent. A SOURCER says "yt-dlp: checking the link, downloading…" with no percent. |
| Kept results | none | In memory only. Opening a project, or any Run, does everything again: on the 44 minute video the Cut detector's read takes 49 s every time. |
| What a recording is | `recordings.rs` `print` | A file's size and a print of its first and last 64 KiB (FNV-1a): enough to tell when a file changed. |
| Live sources | `runtime.rs` | A microphone or live stream runs until Stop; ENF, Planes and plugins read it as it comes. |
| Results folder | `project.rs` `results_dir` | `<project>\results\`, for what Result records write. |
| Play | `playhead.rs` | Play or Space plays the selected node's clock; a recording plays to the end of its audio received so far. |

## What is out there (external survey)

- **ComfyUI** (node graphs for image generation): a queue of runs; each node's result is kept,
  keyed by its inputs, and only nodes whose inputs changed (and what comes after them) run again.
  A node can say it must always run (its inputs do not tell the whole story). Several cache
  policies (keep all, least recently used, by free memory).
- **Houdini PDG/TOPs** (task networks): "Cook Output Node" cooks a node and everything it needs.
  Each work item has a state: waiting, scheduled, cooking, cooked, cooked from cache, failed,
  cancelled, dirty. A work item whose expected files are on disk loads them instead of cooking.
- **Dagster** (data pipelines): a result is stale when its code version or an upstream result
  changed, and everything after a stale result is stale too. "Materialize stale and missing"
  brings just those up to date. Without a code version it must assume the code changed.
- Build tools (make, Bazel) and Rust's `salsa` (incremental computation) rest on the same idea:
  a result is named by everything that went into it; if that name is on disk, the work is done.

The design below takes from each: ComfyUI's keys by inputs, Houdini's states and "cook what it
needs", Dagster's code versions and stale marks.

## Design

### Jobs

Every node that does work is a **job**:
- SOURCER: download (links), decode.
- Cut detector, Audio match, ENF, Planes.
- Plugins and programs (modules).

Nodes that only reshape values (Value, Time, Timeline, Combine, Invert, Map, Result record) are
not jobs: they are worked out when read, as now.

A job has:
- **needs**: the jobs whose results it reads, from its wires. A Cut detector needs its SOURCER; an
  Audio match fed pieces needs the Cut detector and its SOURCERs.
- **steps**: named, each with how far it has got, as the Cut detector's top line has now
  (download, decode, every frame, keyframes, sound, compare…).
- **a turn**: what it takes while it works. Three kinds of turns, each limited (a setting,
  `settingsun.txt`):
  - downloads: 2 at once by default (the network);
  - decoding and heavy work: half the processor cores by default, as now;
  - programs: no limit by default.
- **a key** (see Keeping results).

### The queue

One queue for the whole project. A job enters it when asked for (Run, ▶ on a node, Play with no
sound yet, a change while auto-run is on). A job starts when everything it needs is done and a
turn of its kind is free, in the order asked. While waiting it says **for what**: "for SOURCER
'phone 3' (downloading 34%)", or "for a turn to decode (4 of 4 in use)".

States, each with its own colour, on the node and in the queue list:

| State | Means |
|---|---|
| not run | never run, nothing kept |
| stale | its result is from before a change upstream or in its own settings |
| queued | asked for; waits for another job or a turn (and says which) |
| working | a step, with percent and a bar (or a moving block when how far is not known) |
| done | worked out in this session |
| kept | loaded from the kept results, not worked out again |
| failed | why, in words; what needs it fails at once with "needs X, which failed", even while it still waits for something else |
| cancelled | stopped before it ended; what needs it is cancelled at once, "needs X, which was cancelled" |
| live | a live source and what reads it, streaming until Stop |

### Running

- **▶ in a node's top right corner**: run this node and every job it needs that is not up to date. Nodes
  after it are left alone (they turn stale if its result changed).
- **Run** in the top bar: run every job that is not up to date.
- **Stop**: cancel every job, queued or working. What is done stays done (and kept).
- **Run again** (replacing Restart; also on a node's right-click menu): run without the kept
  results, for when something outside the project changed that the key cannot see.
- **Auto** (a switch in the top bar): when on, a change marks the changed node and what comes
  after it stale, and they are queued at once. With kept results this only redoes what changed.
  This replaces today's restart of the whole graph.
- A node's right-click menu also has: Cancel (this job), Forget kept result.

### Keeping results

A job's result is kept on disk under a **key** made of everything that went into it:
- the job's kind and its **method version** (a number in its code, raised when the method
  changes; the build number would throw everything away at every build);
- the settings that change what it works out. Settings that only change what is shown are left
  out: the Cut detector's thresholds and switches are applied to a kept scan, they do not redo it;
- the keys of what it reads: a file by its size and print (as `recordings.rs` has); a YouTube link
  by the print of its downloaded copy.

Where: `<project>\cache\`, one file per result, named by its key. It moves with the project, and
deleting the folder only costs the time to work things out again. The queue list shows how much
is kept, with a button to forget all of it.

What is kept, job by job:

| Job | Kept | Size, roughly |
|---|---|---|
| SOURCER download | the file itself, in `recordings\` (already) | the video |
| SOURCER decode | the audio at 16 kHz (16 bit) and its waveform | 32 kB a second: 85 MB for 44 minutes |
| Cut detector | its scan: lanes, marks, filmstrip | 2645 pictures of 48×27: 10 MB for 44 minutes |
| Audio match | each recording's fingerprints, each pair's links | small |
| ENF | each recording's hum track | small |
| Planes | its detections | small |
| Plugins and programs | only when its manifest says it may be kept (`keep = true`); otherwise it always runs (it may read the clock, the network…) | |

A live source is never kept: it is not the same twice.

### Playing

Play needs the recording's sound. When it is not decoded (or kept) yet, Play queues the SOURCER's
decode and says "Play waits for the sound: decoding 34%"; playback starts when it is there. A
download is never treated as live (22f9f14).

### On the canvas

- Every job node gets a state dot in its title bar's top left corner, before its name: only the
  dot, in the state's colour. While the job is queued or working the dot becomes a turning ring in
  the same colour (slow while queued, faster while working); a live source's dot pulses.
- Hovering the dot says the step and percent, what it waits for, how long it took, or why it
  failed.
- A ▶ in the title bar's top right corner runs the node and what it needs; while it is queued or
  working it is a ■, to cancel.
- Wires stay as they are; only a live stream's wires pulse.
- **Queue** in the top bar, with counts ("2 working · 3 queued"), opens the queue list: every job
  of the session in the order asked, with its state, step, percent, what it waits for, how long it
  took, "kept" when it was loaded, and a cancel button. Under it: how much is kept, and Forget all.

## Stages

1. **Show it.** One kind of progress for every job (steps, percent, state) in the node's state, the
   state dot on job nodes, the queue list (read only), yt-dlp's download percent on the SOURCER.
   No change to how runs start. Test: headless, the states and the list on a project with a link,
   a file, a Cut detector and an Audio match.
2. **The queue.** Jobs with needs and turns; ▶ on a node runs it and what it needs; Stop and Cancel;
   a change marks stale, Auto queues the stale ones. The ad hoc waits in Audio match and the Cut
   detector go. Test: ▶ on an Audio match fed pieces runs the SOURCERs, then the Cut detector, then
   the match, and nothing else; a change to one SOURCER redoes only its row and what follows.
   Then the mockup's Difficult project built for real in a headless test: three downloads for
   two download turns, two Cut detectors sharing a recording, an Audio match needing seven jobs,
   an ENF on a grid log, a plugin that is never kept, a Cut detector with nothing wired, a live
   microphone; a failed download and a cancel spread to what needs them and no further.
3. **Keeping results.** Keys and `cache\`, the Cut detector's scan and the SOURCER's decoded audio
   first, then Audio match and ENF; the kept state; Run again; Forget. Test: open the 44 minute
   project again, the Cut detector is there with no read; change the file, it reads again.
4. **Programs.** `keep = true` in a plugin's manifest; live sources and what reads them as live
   jobs in the queue.

Every stage updates the READMEs of the nodes it touches in the same commit.

## Decisions (interview, 2026-11-12)

1. **After a change**: the changed node and what follows it turn stale and run again at once (Auto,
   a switch in the top bar, on by default). Everything else stays kept. Today's restart of the
   whole graph goes.
2. **Opening a project**: kept results load at once; nothing runs or decodes. Projects still open
   stopped.
3. **Where**: the project's `cache\` folder.
4. **The decoded sound is kept** (about 85 MB per 44 minutes); the queue list shows the size and
   has Forget.
5. **Running one node**: a ▶ in the node's top right corner, on its title bar: run this node and
   what it needs. While it is queued or working it is a ■ (cancel).
6. **Seeing it**: a state dot before each job node's name, only the dot (a turning ring in the
   state's colour while queued or working; hover for the step and percent), and the queue list
   (Queue in the top bar, with counts) for the details.
7. **No Pause** of the queue: Stop and per-job cancel.
8. **Play before the sound is there** queues the SOURCER's download and decode, says what it waits
   for, and plays once the sound is there.
9. **Order**: show it (stage 1), the queue (stage 2), keeping results (stage 3), programs (stage 4).
10. **Plugins and programs** are kept only when their manifest says `keep = true`.
11. **Restart becomes Run again**: works everything out anew without the kept results (one node:
    its right-click menu).
12. **How many at once is a setting**, in `settingsun.txt` next to the program: downloads
    (default 2), decoding and heavy work (default half the processor cores), programs (default no
    limit).

## Sources

- ComfyUI execution and caching: https://github.com/Comfy-Org/ComfyUI (`execution.py`,
  `comfy_execution/caching.py`); a source tour: https://pyshine.com/ComfyUI-Node-Graph-Execution-Engine-Source-Tour/
- Houdini PDG work item states: https://www.sidefx.com/docs/houdini/tops/pdg/workItemState.html ;
  cache states: https://www.sidefx.com/docs/houdini/tops/pdg/workItemCacheState.html ; cooking:
  https://www.sidefx.com/docs/houdini/tops/cooking.html
- Dagster asset versioning and staleness: https://docs.dagster.io/api/assets
