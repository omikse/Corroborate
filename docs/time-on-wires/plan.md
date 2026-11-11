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
- the graph is the history: hovering an output walks back along the wires to say where its time
  came from;
- Audio match fits its own groups, shows the fit, and sends a recalculated start and end time out of
  every row.

## Decisions (interview 2026-11-11)

- **Wires only.** There is no hidden pool:
  - ENF, ADS-B check and Planes give Time outputs, which are wired into a recording's time input;
  - manual links and anchors on the timeline become small Value nodes;
  - playheads follow wired times only, and an unwired recording moves alone.
- **The Time type stays as it is:** a moment, one ± figure (1 sigma) and one source line naming
  the node that made it.
  - There's no history in the data. The graph already is the history; copying the path into every
    value would repeat the wires, and the copies would drift apart.
  - A trace is built from the graph when someone asks: a hover on an output, or the results file
    when it is saved.
  - Where a node combines errors (an anchor's ± with an audio link's), it adds them as squares and
    shows the working in its own hover.
  - There are no hard limits and no "unknown" state.
  - Relative moments, such as a bang 12.3 s into clip 4, stay inside nodes. If a node ever needs to
    output them, that will be a type and a job of its own.
- **Drift is not in the Time type: it shows as an end time.**
  - Nothing in a file says how fast its clock ran. SOURCER can only guess, Audio match measures it
    against the group, and ENF (later) against the grid's real seconds.
  - Each Audio match row gives two plain Time outputs: its recalculated start, and its recalculated
    end = start + length × the measured speed.
    - A clock 250 ppm slow over a 40-minute file ends 0.6 s later than start + length.
    - Any moment inside the file follows: real time at t s into it = start + t × (end − start) ÷
      length.
  - The speed is relative to the group: the anchor's clock counts as running true. ENF could later
    correct a group to real time the same way, with a start and end of its own.
  - There is no stretched audio output. If synced playback is ever needed, a small node can take
    audio plus start and end.
  - The model assumes one steady speed per recording. Files with cuts break it; they stay findings
    (the gap mark) until the cut detector splits them.
- **Rows with nothing to place them by:** a row with no metadata that audio matched to nothing is
  greyed out, and both its outputs give nothing.
  - A row with metadata but no match gives its metadata time as the start, and start + length as
    the end.
  - A row with no metadata in an anchored group gives the time the group puts it at.
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
  - Port names: the inputs are "audio" and "time (start of recording)"; the outputs are
    "time (recalculated start)" and "time (recalculated end)".
  - The time outputs are blue, the colour of every time port (it was amber).
  - A second status column, Stretch, shows the row's speed against its group's anchor in ppm.
    - It is split at zero: slower on the left, faster on the right.
    - It is green near 0 and red toward 1000 ppm (`SPEED_CHANGE_PPM`, where a sped-up file begins).
    - The anchor row reads "reference".
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
4. The output mark: an anchor or ? in the upper left of each output time field.
   - The time outputs turn blue.
   - The port names change. The saved port names (`audio n`, `time n`, `synced n`) stay, so wires
     in old projects hold.
   - Only the labels drawn on the node and the hover texts change.
   - A new output per row, `end n`, "time (recalculated end)": until stage 3 measures speeds it is
     start + length.
5. Unanchored outputs: `synced_time` stops refusing groups with no anchor.
   - A row with no metadata and no links is greyed out (the `dead` fade covers the whole row, the
     outputs included), and `synced_time` gives an error naming why.
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

### 2 · The Time node and the trace

The Time type does not change.

1. **The trace.** A hover on any time output walks back along the wires and lists each node the
   time passed, with what that node did to it (mockup section 1).
   - It is built from the nodes as they are now, so it cannot disagree with the canvas.
   - The results file writes the same trace for each time when it is saved, for readers without
     the project.
2. **The Time node** (mockup section 5) keeps today's rows of seven boxes, with two stacks
   running down them.
   - **Every row shows all seven boxes:** a 0 where nothing changes, not an empty box.
   - **Each row's + or − sits on its left**, between the row's name and its year box.
   - **A ± (s) column on every row**, the input row included, saved as `n.error_s` (0 by default).
     The ± box under Result goes.
   - **The date stack**, as today: each row's boxes are added or taken away, top to bottom, into
     the Result's date. Years and months move on the calendar; days and smaller are exact seconds.
   - **The error stack:** every row's ± adds as squares into the Result's ±, √(Σ ±²).
     - The row's sign does not matter here: an uncertainty is never subtracted.
     - A row of zeros with only a ± is how a judgement goes in, such as phone clocks being often a
       second off.
     - **It is drawn as the formula:**
       - a large root sign over the rows' ± boxes, its hook beside the last row and its bar across
         the top;
       - a small ² at the upper right of each ± box;
       - Result's ± sits below, outside the root.

       The hover on it names it: "root sum of squares".
   - **Wired rows** (SOURCER's `length`) lock their boxes and their ±, both from the wire. The input
     row's ± is whatever came in.
   - There are no notes and no written-out sums: the boxes and the ± column are the whole node.
3. Old projects: the old single `error_s` becomes a row of zeros with that ±, so the Result's ± is
   unchanged.

Tests:
- the date stack moves the time as named (years and months on the calendar), as today;
- the error stack: ±0, ±0.02, ±0 and ±1 give ±1.000; a row's sign does not change it;
- a wired length locks its row, and its ± reaches Result;
- an old project's Time nodes read the same Result and ± as before;
- the trace of a SOURCER → Time → Audio match chain names the three nodes in order;
- `drive` PNGs of the Time node and of the trace hover.

### 3 · Audio match fits its own groups

1. Each synced group is fitted from this node's own links only.
   - Use `clock::solve` on just those links, inside the node.
   - Out of the fit come each row's offset and speed against the anchor, with errors, and the
     leftover per link.
2. The fit is shown in the row's hover and in the time output's source line: links, leftover, ppm. The foot
   line gives the median leftover. The Stretch column is added here, once the speeds exist, with
   room in the node's width for it.
3. Placement (`match_view_now`) uses the group's own fit instead of `ev.clocks`.
4. The end output uses the measured speed: end = start + length × (1 + speed against the anchor).
   - Its ± is the start's ± and the speed's error × length, added as squares.
   - Its hover says how far it lands from start + length.
5. Same recording, speed change and gaps become marks in the Status cell's empty half, with details
   on hover, if the bottom outputs go.

Tests:
- a `drive` PNG of the Stretch column;
- on Jiku GT_090912, the per-group fit agrees with today's solve to within 1 ms per row;
- a phone measured at −250 ppm over a 2,400 s file ends 0.6 s after start + length;
- `drive` PNGs.

### 4 · Remove the pool

1. ENF:
   - `grid event time` (anchor) becomes a Time output;
   - `sync recordings` and `clock drift` (links) become a start and an end Time per recording, tied
     to the grid's real seconds. Wired into an Audio match anchor's time input, they correct a whole
     group to real time.
2. ADS-B check: `anchors` becomes a Time output.
3. Planes: `anchors = "audio"` goes. A wired time input is just an input, as the ports-simplify
   todo said.
4. Manual anchors and links: dropping one on the timeline creates a Value node of type time. The
   `anchor|` and `link|` project lines and `manual_*` are removed. Old projects lose them, with a
   message saying how many.
5. `gather_evidence`, `update_clocks`, `ev.clocks`, `solve_devices` and `stamp_end` go.
   `clock.rs` stays as a library for stage 3's fit.
6. Playheads (`playhead.rs`): a recording's real-time position comes from the start and end wired
   into it, t s in being start + t × (end − start) ÷ length. With only a start, the speed counts as
   1. Recordings with no wired time move on their own clock only.

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
- Proper error margins for unanchored and fitted times: a separate job.
- Shared errors: two times anchored on the same phone share its ±, so their difference is far more
  precise than either. One ± per value can't know that, and a node comparing them would overstate
  the error.
  - The fix would be a tagged ± list on the Time type, such as
    `[clock of …3092640: 1 s, length of …3263724: 0.02 s]`. Entries with the same tag combine
    directly (and cancel when subtracted) before the root sum of squares. Python's `uncertainties`
    package works this way.
  - Not needed until a node compares two times; check for a Rust crate first.
- SOURCER's end output (start + length), with a guessed ± for clock speed (about 100 ppm × length):
  add it in stage 1, or wait until something needs it?
- One speed per phone across groups (`solve_devices`) is lost with the pool. Within a group, the fit
  can still tie one phone's files to one speed.
