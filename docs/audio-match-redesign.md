# Audio match redesign

Status: built on 2026-11-09, from interviews that day. Later work is under Open.
Mockup: https://claude.ai/artifact/5Boi6NUG3GXEho8YYUYdnf

## The idea

Audio match becomes the time module. Each recording comes in with its metadata time and goes out
with a better one. Time reaches the node only through wires, never by the node reading a file on
its own. Bad data stays visible until a person corrects it.

```
SOURCER --audio-----------------------> Audio match row N --yellow time--> anything
SOURCER --raw time--> [Time node] ----> (blue time input)
```

## SOURCER

- Outputs the raw time of the file, unchanged. It no longer has a start/end choice.
- The whole project reads a metadata time as the recording's start. Nothing detects or changes this
  automatically; the person flips the wrong ones in a Time node.
- Gets a new `length` output, the media's length, for doing arithmetic on times in a Time node.
  Audio match gets no third connector for it: length is a property of the audio on the audio wire,
  so a node that changes the audio's speed also changes its length.
- The error on that time is ±0 by default. Filling in a real error is the job of the Time node or
  another module.
- Files made with the app's REC button (a `.json` beside the recording) are treated like any other
  time.
- Reads the time and the media length once, before Run, and keeps them through a Run.

## Time node

One per row, where a row needs one. From the user's sketch, top to bottom, each row being year,
month, day, hours, minutes, seconds and milliseconds boxes:

- `time` input port, feeding the input row.
- Any number of stackable modification rows.
- A shaded empty row: clicking + turns it into a real modification row.
- The Result row, feeding the `time` output port.
- One error margin ± (s) box under Result. It replaces whatever error came in.

Input row: with a wire it shows the incoming time and locks. Without one you type the time there.

Modification rows:

- Fill in every box or only the ones you need. A stamp ten years ahead is one row with 10 in the
  year box and - selected. An empty box counts as 0.
- + or - is selected on the right of the row. Rows apply top to bottom.
- Every row has its own input port. Wiring a duration in, such as SOURCER's `length`, fills that
  row's boxes and locks them; + or - decides the direction. This is how an end stamp becomes a
  start.
- Years and months move on the calendar: minus 10 years keeps the same day and time. Days and
  smaller are exact seconds.

Result is read-only: always the input with all rows applied, and it follows the input. Every change
is visible as a row. A date that doesn't exist (2018-02-29) or a bad box gives an error and greys
out the output, and the Audio match row says so.

No time-type dropdown: the node works in UTC (Coordinated Universal Time), which is what the app
reads from files. A camera that wrote local time into the file shows up as a whole number of hours
off, and is fixed with a row.

The output is always a start time.

## Audio match

### Ports

- Inputs grow in pairs: `audio N` and `time N`. The single `time` input goes away.
- Dropping a SOURCER's audio on a row also draws its time wire. Both are normal visible wires.
- Each row has one output: the yellow time.

### Row

From left to right: inputs (filename, audio, time), timeline, status column, yellow output.
Row height is adjustable, down to thin rows where the filename and labels hide.

### Placement before an anchor

Within a synced group, take the average middle of its metadata bars and the average middle of its
audio bars, over the rows that have a time, and line the two up. This is visual only.

The person can drag an unanchored group. Dragging changes no data.

A stamp 13 or 200 years off is never ignored or filtered out. It pulls the average, stays visible,
and the person corrects it in a Time node.

### Anchors

- One anchor per synced group. Any row with a time can hold it.
- The audio's start snaps to that row's time. Every other row keeps its real offset.
- While anchored, each yellow bar shows a small anchor in its top-right corner and the group can't
  be dragged.
- After a re-run the anchor stays with its recording. A part that split off goes back to grey and
  shows that it lost its anchor.

### Yellow output

| Row | Output |
|---|---|
| Group has no anchor | Nothing, drawn grey |
| Group has an anchor | Time on the group's clock, error = anchor's error + audio link error |
| Linked, no metadata | Same as its group |
| No links found | A group of one: grey until its own anchor is selected |

### Status column

| When | Shows |
|---|---|
| Before the run | Metadata start time, or "no time metadata" |
| During the run | Progress |
| After the run | Seconds off, green at 0 s, fully red at 30 s, going black into years |
| Offset within a margin of the row's own length | The seconds off, plus a note: about its length. It changes no data |
| Anchor's own row | The anchor icon |
| No metadata and no links | "no links found", and the whole row is grey |

Seconds off follow the picture: dragging a group or editing a Time node changes them live.

The length hint needs care before an anchor. If every stamp in a group marks the end, the average
placement absorbs most of the error, and each row is only off by its length minus the group's
average length. So before an anchor the hint comes from `stamp_spread`: would these stamps agree
better if read as ends? After an anchor each row's offset is compared with its own length directly.

### Synced groups

- After the match, the timeline of every row in a synced group gets a border in that group's colour
  and the group's number. Neighbouring rows of one group share a single border. Rows of one group
  separated by another group's row get separate borders with the same number and colour.
- A group of one has no border or number.
- An "Order by group" button at the top puts each group's rows together. Wires will cross more, and
  that is accepted. It only changes the order rows are drawn in: `audio N` keeps its number.
- Audio decides what a group is. A person can't take a recording out of a group or join two groups
  by hand. To line up two groups, anchor each one.
- Groups don't fold away for now. Thin rows handle big projects.

### Footer

Kept very simple for now: one status line and the node's other outputs. It is to be redesigned later
to show what the match is doing, as part of a separate view of the calculation.

### View

Fit all and Always fit stay. Reset is removed. Wheel zooms and drag pans, and either one turns
Always fit off. Fit all includes every bar, even one years away.

## Clock solve

It stays, behind Audio match. Links are still solved together for accuracy, and the result leaves
only through the yellow outputs.

## What a time can prove

Audio only says how recordings lie against each other, and a stamp can't say which moment it marks.
Flip a stamp between start and end, shift by the same amount, and the match is perfect again. Group
agreement can only tell the two readings apart when the clips differ in length, and even then the
whole group can still be off together. So the design picks one time to trust (the anchor) and
confirms it with other techniques.

Confirming needs no new display: anchor a group on one row, wire another technique's time (power
hum, for example) into a different row's time input, and that row's seconds off is how far the two
techniques disagree.

## Groups stay inside Audio match

Synced groups are internal to Audio match. No other node or part of the canvas shows them.

| Today, outside the node | Where | Plan |
|---|---|---|
| Glow, outline and group number (1°, 2°) on nodes and wires | `clock_view.rs:349` `draw_glow`, `canvas_groups` | Remove |
| Time and ± boxes in the middle of wires, including their group colour and clicking one to open a timeline row | `clock_view.rs:451` `draw_wire_boxes`, `Evidence.focus` | Remove entirely |
| Result record's warning "times on N clock groups" | `evidence.rs:335` | Remove or reword without groups |
| Bottom timeline: one row per group, drag a group, snap to make a link, Sync list | `timeline.rs` | Remove with the panel |
| One playhead per floating group | `playhead.rs` | Replace with one real-time playhead |

The result file keeps its clock data (kind, group, offset, error): it is a record of the solve, not
a display. Same clock on SOURCERs is a person's choice about sources, not an audio group, and stays.

With the wire boxes gone, a wire's value is seen by hovering its port, which already shows it
(`bus.rs:219` `port_hover`). The error breakdown (`breakdown_lines`), the confidence and maximum error
settings, and the Sync list go with the panel. How the match was calculated will live in Audio
match as a separate view, later.

## Timeline

The system timeline panel is removed and replaced by a Timeline node, added by hand like the Map.
This build makes a basic one that works; the node gets its own design work after that.

The Timeline only receives and displays. It doesn't work out when anything happened: the time it
is given is the truth it draws.

Basic Timeline node:

- Inputs grow in rows, each taking one recording's standard audio or video output and a time. The
  yellow output of Audio match is the usual time.
- Each recording is a bar at its time, as long as its media, with the filename, and margins left and
  right from the error carried by the time.
- A row with no time is greyed out, keeps its name and draws no bar.
- Row height is resizable, a button sorts by start time, wheel zooms and drag pans.
- One real-time playhead, in UTC (Coordinated Universal Time), shared by all Timeline nodes and
  followed by anything with a real time. A SOURCER with no real time keeps its own local playhead,
  linked to nothing.
- No sound on the Timeline node for now.

Not in the basic node: timed events (the Timeline will receive events already in real time, and
Audio match producing those is later work), confidence and maximum error settings, drag-to-link.

Until events arrive, detector results such as power hum or plane passes have no timeline to appear
on. They remain on their own nodes and in the result file.

## Code changes

1. `evidence.rs:95`: a wire on a SOURCER's time output no longer trusts the stamp. This has to land
   before one-drag wiring, or every stamp enters the clock solve.
2. SOURCER: read stamp and length before Run, keep them through `NodeState::reset`, give ±0 error.
3. Remove `stamp_end` and `start_utc`'s shift. `start_utc` uses the error as the length today, so
   this goes together with the ±0 change.
4. Paired growing inputs: `Manifest::spec`, `many_names`, `Graph::pack_many`, and project loading,
   which today assigns many-input wires by order. Pairs must move together.
5. Growing per-row outputs with unique names, and a per-port colour key (`PortLook` has none, and
   colour comes from the data type today).
6. `port_place` (`layout.rs`): ports follow the rows.
7. Time node: a new module beside Value.
8. `Look::body` (`node.rs`): rows, status column, anchors, group drag, group borders, Order by
   group, row height, view buttons.
9. Replace `fit_stamps` with middle placement, keeping `stamp_spread` for the length hint. Stop
   dropping outliers in `fit_axis`.
10. Anchors: stored per node in the project. A selected anchor needs its own evidence path, because
    `evidence.rs:104` skips a recording's own time.
11. New `length` data type in `src/types` (none exists for a duration) and SOURCER output. The bars
    read length from the recording on the audio wire, known before Run from step 2.

## Migration

- `stamp-end|` project lines: create a Time node with the length subtracted when loading.
- A wire into the old `time` input: its recording holds the group's anchor (it meant the same).
- Every other recording's time is wired to its row, so old projects still show their metadata.
- Manual anchors and links saved from the old panel stay evidence and stay in the result file;
  there is no display for them now.

## Order of work

1. SOURCER and the evidence rule.
2. Paired ports, per-row output and port placement.
3. Time node.
4. Node body: placement, anchors, status column, view.
5. Yellow output and anchors in the clock solve.
6. Migration of old projects.
7. Remove the group displays outside Audio match and the system timeline panel; add the basic
   Timeline node and the single real-time playhead.

Tested with `cargo test --profile fast` and headless drive scenarios with PNGs on a copy of the
Jiku project, then `run.cmd`.

## Known risks

- Raw anchors look exact. The clock solve floors an error of ±0 at 1 ms (`clock.rs:211`), so
  anchoring an unchecked file stamp claims millisecond accuracy unless a Time node sets a margin.
- One bad stamp moves a group's whole unanchored picture. Out of 19 stamps, one 13 years off moves
  it about 8 months. This is intended, since the person sees it and fixes it.
- Correct data hides the bar colours. Once stamps agree within a second, each row is almost all
  overlap colour.
- 50 recordings at full row height are about 4,800 px tall. Thin rows are the answer.

## Open

- The size of the length hint's margin (5 s for now).
- A speed up/slow down detector before Audio match. It outputs real stretched or squeezed audio, so
  other modules see ordinary audio. Fingerprints only match up to about 1 % of speed change, so a
  larger change needs correcting first. To build it: `audio_match_inputs` has to accept audio that
  doesn't come straight from a SOURCER; the node needs a `length` output, since SOURCER's is the
  file's and not the corrected one; and `clock_at` puts a node fed audio on its source's clock, so
  times on the corrected audio need mapping back to the original file's.
- Full Timeline node design and timed events.
- Evidence switches: projects keep their `off|` lines, but nothing on screen sets them any more.
- The separate view of how Audio match calculated its result.
