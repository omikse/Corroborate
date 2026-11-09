# Time (data)

Corrects a time, in the open: every change is a row you can see. From top to bottom:

- The input row: a wired time shows here and locks the boxes; with no wire you type the time.
- Any number of modification rows. Fill in only the boxes you need: a stamp ten years ahead is one
  row with 10 under year and - selected. An empty box counts as 0. Rows apply from the top down.
  Each has its own input for a length: wire SOURCER's `length` in and select - to turn a time that
  marks the end of a recording into its start.
- The shaded row: + adds a modification row.
- Result: read-only, what goes out on `result`.
- The error margin (s), which replaces whatever error came in.

Years and months move on the calendar (minus 10 years keeps the same day and time); days and
smaller are exact seconds. A date that does not exist, or a box that is not a whole number, is an
error: Result stays empty and the output gives nothing, and whatever reads it says so. Everything is
UTC (Coordinated Universal Time), which is what files hold; a camera that wrote its local time shows
as a whole number of hours off, and is fixed with a row.

Files: `module.toml`, `mod.rs` (the arithmetic), `node.rs` (the node on the canvas).
