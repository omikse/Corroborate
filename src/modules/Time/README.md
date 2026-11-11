# Time (data)

Corrects a time, in the open: every change is a row you can see. From top to bottom:

- The input row: a wired time shows here and locks the boxes and its ±; with no wire you type the
  time, and its ±.
- Any number of modification rows. Each shows all seven boxes, 0 where nothing changes: a stamp ten
  years ahead is one row with 10 under year and − on its left. A click on the sign turns it over.
  Rows apply from the top down. Each has its own input for a length: wire SOURCER's `length` in and
  keep − to turn a time that marks the end of a recording into its start. A wired row locks its
  boxes and its ±, both from the wire.
- The shaded row: + row adds a modification row.
- Result: read-only, what goes out on `result`.

Two stacks run down the rows:

- **The date stack:** each row's boxes are added or taken away into Result's date. Years and months
  move on the calendar (minus 10 years keeps the same day and time); days and smaller are exact
  seconds.
- **The error stack:** every row's ± (s), the input's included, adds as squares into Result's ±:
  √(Σ ±²), the root sum of squares. It is drawn as that formula, a root sign over the ± column with
  a small ² on each box, Result's ± below it. A row's sign does not matter here: an uncertainty is
  never subtracted. A row of zeros with only a ± is how a judgement goes in, such as phone clocks
  being often a second off.

A date that does not exist, or a box that is not a whole number, is an error: Result stays empty and
the output gives nothing, and whatever reads it says so. Everything is UTC (Coordinated Universal
Time), which is what files hold; a camera that wrote its local time shows as a whole number of hours
off, and is fixed with a row.

The output's hover walks back along the wires: each node the time passed, what it did, and the time
it gave. The result file writes the same trace.

Projects saved when the node had one error margin box: that margin becomes a row of zeros with that
±, so Result reads the same.

Files: `module.toml`, `mod.rs` (the arithmetic), `node.rs` (the node on the canvas).
