# Time on wires

Status: planned on 2026-11-11 from that day's interview. Nothing is built yet.
Mockup: [mockup.html](mockup.html)

## The idea

Remove the global clock solve. Today `evidence.rs` (`gather_evidence` → `clock::solve`) pools these
and moves every clock, out of sight:
- every node's `anchor` and `link` outputs, wired or not;
- the timeline's manual anchors and links;
- the Audio match anchors.

Playheads, Audio match's group placement and its time outputs all depend on that solve. Nothing on
the canvas shows which evidence moved which recording.

After the change:
- a node's time reaches a recording only along a wire;
- every time carries the steps that made it;
- Audio match fits its own groups, shows the fit, and sends a time and drift-corrected audio out of
  every row.

## Decisions (interview 2026-11-11)

- **Wires only.** There is no hidden pool:
  - ENF, ADS-B check and Planes give Time outputs, which are wired into a recording's time input;
  - manual links and anchors on the timeline become small Value nodes;
  - playheads follow wired times only, and an unwired recording moves alone.
- **The Time type** is a moment, one ± figure (1 sigma) and a list of steps.
  - Each node it passes appends a step: the node, what it did, and the number it changed. This is
    like the Time node's stacked rows.
  - There are no hard limits and no "unknown" state.
  - Relative moments, such as a bang 12.3 s into clip 4, stay inside nodes. If a node ever needs to
    output them, that will be a type and a job of its own.
- **Drift is not in the Time type.**
  - Each Audio match row gets a second output: its audio resampled by the speed Audio match measured.
  - The speed is relative to the group: the anchor's clock counts as running true. ENF could later
    correct a group to real time.
- **Unanchored groups** send times out too. They are placed on the average of the group's metadata,
  or wherever a person dragged them.
  - Each output keeps its own metadata ±. Proper error margins are a separate, later job.
  - A drag is saved with the project, logged like an anchor, and undoable.
- **Audio match UI** (from the mockups):
  - Scale to all becomes View all.
  - Order by group becomes an Order dropdown: Group › metadata › start, Start › group, Group › how
    far off, As wired.
  - A legend: time calculated from audio (amber), time from metadata (blue).
  - The Status and output columns get titles.
  - The Status cell is split at zero: − colours the left half, + the right half.
  - The blue start line in the lanes goes.
  - The output field shows an anchor or ? in its upper left corner.
- **The six bottom outputs** (links, quality, clock drift, same recording, speed change, gaps) are
  proposed to go, with their findings shown as marks in Status. This is not yet confirmed; see Open.

## Stages

Each stage is tested, committed and run before the next starts.

### 1 · Audio match UI

Only `src/modules/Audio match/node.rs` and its tests.

1. Renames and labels:
   - View all;
   - a legend in the top bar, after the row height slider;
   - the "Status" and "output" titles over their columns;
   - drop the `ANCHOR_BLUE` line drawn at `r.meta` in the lanes.
2. The Status cell split at zero.
   - `Cell::Off` fills one half with `off_color`.
   - The text sits in the coloured half; the font is shrunk if the text does not fit.
   - The mirror hint moves to the empty half.
   - A 1 px middle line.
3. The Order dropdown.
   - The row `order` takes `group-meta-start | start-group | group-off | wired`.
   - An old `order by group = true` reads as `group-meta-start`.
   - The sort keys are added to the `order` sort in `body`.
4. The output mark: an anchor or ? in the upper left of the amber field.
5. Unanchored outputs: `synced_time` stops refusing groups with no anchor.
   - The time is `rows[k].start`, with the row's own `meta_err`.
   - The source says "unanchored: average of metadata" or "moved by hand".
6. Saved drags.
   - `match_ui.shift` moves into the node's rows (`shift <group key>`), so it is saved and undone
     with the project.
   - A `SettingChange` is logged when the drag ends.

Tests:
- `drive` PNGs of the node: the split cells, the dropdown open, and an unanchored output with ?;
- a unit test that every order sorts as named;
- a test that a drag survives save, reopen and Ctrl+Z, and moves the output.

### 2 · Steps on the Time type

1. `Data::Time { source: String }` becomes `steps: Vec<Step>`, where
   `Step { node, what, change }`.
   - The JSON form is a `"steps"` list.
   - A plugin's `"source"` text is read as one step.
2. Every place that makes a Time (about 20: data.rs, bus.rs, evidence.rs, SOURCER, Time, Value,
   Timeline, ENF, Audio match) appends a step instead of overwriting the text.
3. Hover on any time output lists its steps (mockup section 1). The results file carries them.

Tests:
- a SOURCER → Time → Audio match chain shows three steps in order, with the numbers each changed;
- the JSON round-trips.

### 3 · Audio match fits its own groups

1. Each synced group is fitted from this node's own links only.
   - Use `clock::solve` on just those links, inside the node.
   - Out of the fit come each row's offset and speed against the anchor, with errors, and the
     leftover per link.
2. The fit is shown in the row's hover and in the time output's step: links, leftover, ppm. The foot
   line gives the median leftover.
3. Placement (`match_view_now`) uses the group's own fit instead of `ev.clocks`.
4. The corrected audio output.
   - Audio is not a value on a wire today: a SOURCER streams it (`PortKind::Audio` "is not sent as a
     value"). Audio match must become an audio producer the way a SOURCER is.
   - Check how `runtime` hands a source's audio to whatever is wired after it, then resample by the
     row's speed.
   - Survey resampling crates first (`rubato`), as rule no 1 says.
5. Same recording, speed change and gaps become marks in the Status cell's empty half, with details
   on hover, if the bottom outputs go.

Tests:
- on Jiku GT_090912, the per-group fit agrees with today's solve to within 1 ms per row;
- the corrected audio of a phone at −250 ppm is 0.6 s longer per 40 min;
- `drive` PNGs.

### 4 · Remove the pool

1. ENF:
   - `grid event time` (anchor) becomes a Time output;
   - `sync recordings` and `clock drift` (links) go, or become Time outputs per recording. To be
     decided when we get there.
2. ADS-B check: `anchors` becomes a Time output.
3. Planes: `anchors = "audio"` goes. A wired time input is just an input, as the ports-simplify
   todo said.
4. Manual anchors and links: dropping one on the timeline creates a Value node of type time. The
   `anchor|` and `link|` project lines and `manual_*` are removed. Old projects lose them, with a
   message saying how many.
5. `gather_evidence`, `update_clocks`, `ev.clocks`, `solve_devices` and `stamp_end` go.
   `clock.rs` stays as a library for stage 3's fit.
6. Playheads (`playhead.rs`): a recording's real-time position comes from the Time wired into it.
   Recordings with no wired time move on their own clock only.

Tests:
- the existing clock and playhead tests rewritten against wired times;
- an ENF → Time → Audio match chain dates a group;
- no evidence reaches a recording without a wire.

### 5 · Docs

README, `docs/ARCHITECTURE.md`, the Audio match and ENF READMEs, and `docs/audio-match-redesign.md`
(its Open list).

## Open

- Must every Time on a wire be a real (UTC, Coordinated Universal Time) moment? This is "not sure
  yet". Until a node needs otherwise, the type keeps `clock`, `t` and `utc` as they are.
- Should the six bottom outputs go, with their findings as Status marks? Asked, not yet answered.
- ENF's link outputs: drop them, or turn them into per-recording Time outputs (stage 4)?
- Proper error margins for unanchored and fitted times: a separate job.
- One speed per phone across groups (`solve_devices`) is lost with the pool. Within a group, the fit
  can still tie one phone's files to one speed.
