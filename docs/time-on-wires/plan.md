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
  - **An optional end input per row,** "audio end time (optional)": a thin third stripe under
    "audio start time", grey until wired. Unwired, the end counts as start + length.
    - Wired on the anchor row, it sets the group's real speed, so the Stretch figures become real
      rather than relative to the anchor. The anchor's Stretch cell says "speed from its wired end".
    - On other rows, it is compared with the measured end, as the start is with Status.
    - On a row with no audio match, it passes straight through to "audio end time".
    - Possible sources: ENF (start and end tied to the grid), a speed detector run before Audio
      match, or a file's modified time from SOURCER as a rough end with a wide ±.
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
  - Port names: the inputs are "audio", "audio start time" and "audio end time (optional)"; the
    outputs are "audio start time" and "audio end time".
  - The time outputs are blue, the colour of every time port (it was amber).
  - A second status column, Stretch, shows the row's speed against its group's anchor in ppm.
    - It is split at zero: slower on the left, faster on the right.
    - It is green near 0 and red toward 1000 ppm (`SPEED_CHANGE_PPM`, where a sped-up file begins).
    - The anchor row reads "reference".
- **The six bottom outputs go** (links, quality, clock drift, same recording, speed change, gaps).
  - Same recording, speed change and gaps show as marks in the Status cell, with details on hover.
  - The summary stays in the foot line.
  - `links` goes last, in stage 4, since the hidden solve reads it until then.
- **SOURCER gets no end output yet.** Audio match assumes start + length when its end input is
  unwired. An end output comes when ENF or a speed detector gives real ends.
- **Order of work:** stages 1 to 5 as below.

## Stages

Each stage is tested, committed and run before the next starts.

### 1 · Audio match UI

Only `src/modules/Audio match/node.rs` and its tests.

Done 2026-11-11. Beyond the node: a row of growing ports may now have several outputs
(`many = true` on more than one `[[output]]`, the last ones), so `graph.rs` and
`module_manifest.rs` changed too; port hovers show the titles.

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
   - A new output per row, `end n`, "audio end time": until stage 3 measures speeds it is
     start + length.
   - A new optional input per row, `end in n`, "audio end time (optional)", drawn as a thin grey
     stripe under the start time. In stage 1 it passes straight through to the end output when
     wired. The row grows by the stripe (about 12 px at the default height).
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

Done 2026-11-11. The trace is `src/bin/corroborate/trace.rs`: it walks back through Time nodes
(their time input) and Audio match rows (the anchor row's time input when anchored, the row's own
otherwise, the end input when it passes through); other nodes end the walk. A wired length may carry
an `error_s` beside its `seconds`; SOURCER's gives none, so it reads ±0. The node grew to 600 wide.

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

Done 2026-11-11. Differences from the plan below:
- The fit is `clock::fit_against`, the clock solve without its breakdown (`clock::fit`), read in
  the reference's seconds. The reference is the anchor's recording, else the group's longest
  (so an unanchored group shows speeds too). Every clock gets the global solve's speed prior; the
  reference is not held still. Holding it still and pulling the others toward its speed placed
  Jiku GT_090912's starts 21 ms off the truth (median; 168 ms for 95 %) against 4 ms (25 ms).
- "Agrees with today's solve within 1 ms" holds in the anchor's seconds (Jiku group 6: 0.002 ms).
  In real seconds the two differ by the anchor's own speed, which audio cannot measure. The node
  counts the anchor's clock as running true; the global solve made the group's average clock
  run true. On Jiku the average is closer to the truth (starts 177 ms off against 280 ms, median,
  from references up to 900 s away), since the reference phones ran up to 266 ppm fast. The
  anchor's wired end fixes it.
- The speed is the clock's seconds per reference second, minus 1, so a slow clock is negative.
  End = start + length ÷ (1 + speed): −250 ppm over 2,400 s ends 0.6 s late.
- The errors of a speed and of an end come from a fit with the reference held still. Proper
  error margins remain the later job.
- The match keeps what it found (`Found`: links, same recording, speed change, gaps, quality,
  each pair's drift) in the node's state. The result file writes it; the outbox keeps only
  `links`.

1. Each synced group is fitted from this node's own links only.
   - Use `clock::solve` on just those links, inside the node.
   - Out of the fit come each row's offset and speed against the anchor, with errors, and the
     leftover per link.
2. The fit is shown in the row's hover and in the time output's source line: links, leftover, ppm. The foot
   line gives the median leftover. The Stretch column is added here, once the speeds exist, with
   room in the node's width for it.
3. The stretch on the timeline: the amber bar runs from the start to the measured end, at its real
   length.
   - A dashed line marks where start + length would end.
   - The bit past it is hatched; a bar ending short of it is cut.
   - A label beside it gives the size ("end +47 ms").
   - The hatch is at least 4 px wide so it shows at any zoom.
   - The legend gets a third entry: "stretch: start + length ┆ measured end".
   - The anchor row has none: it is the reference.
4. Placement (`match_view_now`) uses the group's own fit instead of `ev.clocks`.
5. The end output uses the measured speed: end = start + length × (1 + speed against the anchor).
   - Its ± is the start's ± and the speed's error × length, added as squares.
   - Its hover says how far it lands from start + length.
6. The optional end input:
   - **Wired on the anchor row:** it gives the anchor's real speed, (end − start) ÷ length. Every
     row's speed is then that times its speed against the anchor.
   - **Wired on another row:** it is compared with the measured end, as Status compares the start.
7. Same recording, speed change and gaps become marks in the Status cell's empty half, with details
   on hover. The outputs quality, clock drift, same recording, speed change and gaps go. Saved
   wires into them are dropped with a message, though no saved project wires them today.

Tests:
- a `drive` PNG of the Stretch column;
- on Jiku GT_090912, the per-group fit agrees with today's solve to within 1 ms per row;
- a phone measured at −250 ppm over a 2,400 s file ends 0.6 s after start + length;
- an anchor end wired 0.4 s past start + length shifts every row's end by the same speed;
- `drive` PNGs.

### 3b · Phones, the group's average clock, and drift in s/h

From the interview of 2026-11-11, after stage 3. It's all inside Audio match; nothing is global.

What stage 3 left open:
- the anchor's file was taken as perfect;
- a dragged group could not get back to its average;
- each file counted as its own phone;
- speeds were shown in ppm (parts per million), a unit that means little to most people.

1. **Phone id.** A free-text field on each row's name strip, beside the file name.
   - Rows with the same text are one phone and share a colour. An empty field is its own phone,
     which is today's behaviour.
   - Make, model or serial from the metadata, when the file has them, shows greyed as a
     suggestion. It counts only once a person accepts or types it.
   - Saved per recording (`phone <uid>`), logged and undone like the anchor.
   - With no room on the strip, the field shrinks to its colour chip, and a click edits it.
   - A new Order option, Group › phone › start, puts a phone's files together.
2. **One drift per phone.** Files of one phone keep their own offsets (each started when it was
   pressed) but share one drift, one crystal: `clock::fit_against` takes the phone ids as the
   device map (the `solve_devices` mechanism). A short clip then borrows its phone's drift from
   the longer files.
3. **The group's average clock sets the drift**, not any one file.
   - One vote per phone, weighted by how well each phone's drift is measured.
   - Every phone counts: a weighted mean, with nothing dropped for being far off. A sped-up file
     shows its speed change mark.
   - The anchor row stops reading "reference" and shows its own drift like every row.
   - **Speed reference:** a button of its own on each row says "this file's clock runs true",
     e.g. the official concert recording from a professional camera. One per group; it overrides
     the average.
   - An end wired into the anchor row still gives the real drift and overrides both.
4. **The average anchor.** A button on the group's number tab.
   - It puts a dragged group back at its average.
   - It anchors the group on a virtual anchor instead of one file. Its position is the weighted
     mean of the group's stamps: each stamp votes for where the group sits, one vote per phone,
     weighted by its ±. Every stamp counts, one 12 years off included; wrong metadata shows
     plainly on the timeline, and fixing it is a person's job (a Time node).
   - Its error is the stamps' weighted spread ÷ √phones.
   - The outputs get the anchor mark. The source line says "anchored on the average of N phones'
     stamps".
   - It and a file anchor exclude each other. Dragging stops while anchored.
   - After a run, groups stay unanchored, marked ?, at that average.
5. **Drift, in seconds per hour.** One name everywhere:
   - the Stretch column becomes **Drift**, in s/h; −0.9 s/h is a clock losing 0.9 s an hour,
     i.e. −250 ppm;
   - the hover gives the exact scale, ×0.9997500 (real seconds per file second);
   - ppm stays internal only;
   - the colour runs from green near 0 to red at 3.6 s/h (1000 ppm, `SPEED_CHANGE_PPM`).

Tests:
- two files sharing a phone id share one drift, and a 40 s clip takes its phone's;
- the average anchor puts a group at the weighted mean of its stamps, one vote per phone, a
  stamp 12 years off included;
- the speed reference button makes that file's drift 0 and measures the others against it;
- the drift reads in s/h, with the scale in the hover;
- `drive` PNGs of the phone ids with colours, the average anchor, and the Drift column.

### 4 · Remove the pool

What the global solve touches today, module by module, and what each becomes. Surveyed
2026-11-11 from the code: every caller of `ev.clocks`, `ev.links`, `gather_evidence`, the
playheads and the `anchor`/`link` types.

**App core (`src/bin/corroborate`)**
1. `evidence.rs`:
   - Goes: `gather_evidence`, `update_clocks`, `carry_playheads`, `not_comparable`, and
     `ev.clocks`, `ev.links`, `ev.anchors` and `ev.off` (evidence switched off by hand).
   - Stays: `clock_of`, the recording identity a Time's `clock` names, which Audio match uses to
     group files on one clock.
2. `main.rs`: the per-frame `update_clocks()` call and `ev.stamp_end.clear()` go.
3. The project file:
   - `evidence_text` stops writing the `anchor|`, `link|` and `off|` lines.
   - `evidence_line` still reads them from old projects, drops them, and says how many.
   - Manual anchors and links become small Value nodes of type time.
4. The results file: `clocks_json` (the solved clocks and the "not comparable" list) goes. Each
   time gets its trace instead (stage 2).
5. The **Same clock / Separate clocks** actions on a source's right-click menu (`same_clock`,
   `separate_clocks`, `shares_clock`) tied two files from one phone to one solved speed
   (`solve_devices`).
   - They stay as labels only: Audio match still groups rows on one clock.
   - Their only effect on speed is inside Audio match's own fit.
6. `playhead.rs`:
   - Each recording keeps its own playhead on its own clock (`playhead_on`, `set_playhead`), as
     now.
   - Today's "absolute groups" and "floating clocks" came from the solve's clock kinds.
     - They go.
     - The real-time playhead (`ph.utc`, `set_utc_playhead`) lives in the **Timeline node**.
   - A recording follows it only when its start (and end) times are wired into that Timeline. Its
     position is t = (utc − start) × length ÷ (end − start).
   - `active_clock`, `active_key`, `is_active_clock` and `live_now` are reworked around that.
7. `sound.rs`: playback reads `playhead_on(clock_of(src))`, which is unchanged.
8. `bus.rs`:
   - A plugin's own Time output (from its anchor, `s.anchor`) is unchanged.
   - Events and tracks it receives without a clock still get the clock of the audio they heard.

**Modules (`src/modules`)**

9. **Audio match:**
   - It places groups from its own fit (stage 3), not from `ev.clocks`.
   - `synced_time` reads the fit.
   - `links` goes.
   - `migrate_match` (which read `ev.stamp_end` to convert projects older than per-row times)
     goes; such projects are not read any more.
10. **ENF (power hum):**
    - `sync recordings` and `clock drift` (links) go.
    - `time of recording` (already a Time) becomes "audio start time", and a new "audio end time"
      comes from the hum's own speed against the grid.
    - `grid event time` (anchor) becomes a Time output.
    - The node's cursor (`clock_of` + `playhead_on`) is unchanged.
11. **ADS-B check:**
    - `anchors` (anchor) becomes "audio start time", and "audio end time" when the matched planes
      span long enough to measure a speed.
    - Its `time` input stays an input.
12. **Planes (Doppler):** the `anchors = "audio"` key on its `time` input goes. A wired time is just
    an input. Its playhead cursor is unchanged.
13. **Plugin:** the manifest key `anchors = "audio"` goes from `module_manifest.rs`
    (`anchoring_inputs`), so no plugin can feed the pool. Plugins with `anchor` or `link` outputs
    give Time outputs instead.
14. **Timeline:**
    - Its rows already take wired Times (the `time` input); it gains the real-time playhead from
      point 6.
    - Each row needs the recording's start and, if known, end.
15. **SOURCER:** its time output (the metadata, ±0) is unchanged; `stamp_end` goes with the old
    format.
16. **Types:** `anchor` and `link` (`src/types/anchor`, `src/types/link`) are retired, so the Types
    menu no longer lists them. A project wire carrying one is dropped with a message; none is
    expected.
17. **Events and tracks** (Planes, ENF, plugins) stay on their recording's clock, in seconds into
    the recording. A node that needs them in real time takes that recording's start and end on a
    wire beside them.

**Library (`src`)**

18. `clock.rs` stays: `solve` is used inside Audio match (stage 3) on one group's links.
    `anchors_from_json`, `links_from_json` and `solve_devices` go.

Tests:
- the existing clock and playhead tests rewritten against wired times;
- clicking the Timeline moves the playheads of the recordings wired into it, and only those;
- an ENF → Time → Audio match chain dates a group;
- an old project with `anchor|` and `link|` lines opens, drops them, and says how many;
- no evidence reaches a recording without a wire.

### 5 · Docs

README, `docs/ARCHITECTURE.md`, the Audio match and ENF READMEs, and `docs/audio-match-redesign.md`
(its Open list).

## Open

- Must every Time on a wire be a real (UTC, Coordinated Universal Time) moment? This is "not sure
  yet". Until a node needs otherwise, the type keeps `clock`, `t` and `utc` as they are.
- Proper error margins for unanchored and fitted times: a separate job.
- Shared errors: two times anchored on the same phone share its ±, so their difference is far more
  precise than either. One ± per value can't know that, and a node comparing them would overstate
  the error.
  - The fix would be a tagged ± list on the Time type, such as
    `[clock of …3092640: 1 s, length of …3263724: 0.02 s]`. Entries with the same tag combine
    directly (and cancel when subtracted) before the root sum of squares. Python's `uncertainties`
    package works this way.
  - Not needed until a node compares two times; check for a Rust crate first.
- One speed per phone across groups (`solve_devices`) is lost with the pool. Within a group, the fit
  can still tie one phone's files to one speed.
