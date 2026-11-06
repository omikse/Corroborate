# Power hum (ENF)

ENF (Electrical Network Frequency): the mains hum and its drift, and from it, where the recording
can have been made.

Node: `audio` in, `location` (an area) out. The runs of clear hum go to the timeline and the result
file.

On the canvas the node shows everything itself (no separate window): the Earth map of where its grid
frequency is used (green), the hum frequency over time under it, the **grid data** file chosen right
in the node (empty: the bundled file), and a status column (state, grid frequency, hum runs, audio,
area).

Files: `mod.rs` (the detector: name, label, ports), `node.rs` (the node in the workbench: state while
running, the area its grid frequency allows, its look on the canvas, timeline marks, result file), `plugin.rs` (the same
detector as a plugin for the old viewer, `enf-plugin.exe`). The default grid map is in its own module:
[grid_frequencies](../grid_frequencies/README.md).

## Detecting the hum

A 16 s window every second, a 262 144-point FFT (Fast Fourier Transform) with 0.015 Hz bins, the
best of harmonics 1–4 of 50 and 60 Hz, and a vote for the grid's nominal frequency. Readings that
jump more than 30 mHz from the recent median are flagged as outliers, kept but not used. Runs of
clear hum of 30 s or more are reported. On the reference recording it identifies a 60 Hz grid (US)
and tracks the drift within about ±40 mHz.

## From hum to area

Once the nominal frequency is decided, the node's area is every 1° cell (see `src/area.rs`) whose
centre lies in a grid region of that frequency. To combine it with a known place, wire both into a
Combine locations node or the Result record: an empty result is kept and labelled CONTRADICTION
(the hum is not used anywhere in that place, so one of them is wrong). Nothing is hidden or smoothed
over.

Tested live on the Cornell Lab stream (Ithaca, New York) with no location given: 60 Hz found, the
world cut to 17.85 % of Earth.

## Grid data file

Chosen in the node (the same format the Grid frequencies node reads); swap the file to use other
data. It is read at each Run. Format:

```json
{
  "source": "where the data came from",
  "regions": [
    {"id": "Europe", "frequency_hz": 50, "bounds": [[lat, lon], [lat, lon], [lat, lon], ...]}
  ]
}
```

`bounds` is a polygon of at least three `[lat, lon]` points. A cell counts for a frequency when its
centre is inside any region of that frequency, so overlapping regions allow both frequencies: the
map never wrongly excludes a cell, it only fails to exclude one. Polygons that cross the 180°
meridian are not handled.

The default file is rough boxes per country group: borders are off by up to a few hundred km.

## Limits

- One nominal frequency per run; a recording that moves between grids is not handled.
- The drift itself (matching against grid-frequency logs to date a recording) is measured and
  shown, not yet used.
