# Map (view)

Everything wired into it on one map you can move and zoom, on the canvas. Wire a location in at any
step of a chain to see that step, and use one Map per step to compare them. It takes as many
locations as you wire (`location 1`, `location 2`, …); no outputs.

Points (a typed place, a file's GPS) are pins in their clock group's colour; a click selects the
node they come from. Areas are their 1° cells: green adds, red subtracts. New inputs are framed by
themselves. The wheel zooms, a drag moves, the status line says what is on it.

The list at the top picks the map, for all Map nodes: offline (the built-in world image, or
`.pmtiles` files in `maps\` next to the program) or Google's satellite and road maps.

## Offline maps

The Map node never fetches anything while you work unless you switch Google on. Without map files
it draws the built-in world image: NASA Blue Marble (public domain, 2048 × 1024, stored as
`earth.idx`, a 256-colour palette and one byte per pixel) with Natural Earth 1:110m coastlines
(public domain). For street-level detail put `.pmtiles` files (vector tiles, Protomaps basemap) in
`maps\` next to the program; every file there is drawn, in the Protomaps dark style (walkers). Get
one region once with the [pmtiles CLI](https://docs.protomaps.com/pmtiles/cli):

```
pmtiles extract https://build.protomaps.com/<YYYYMMDD>.pmtiles maps\london.pmtiles --bbox=-0.6,51.2,0.4,51.8 --maxzoom=15
```

Each zoom level roughly doubles the size: a city to zoom 15 is tens to a few hundred MB, the whole
world to zoom 6 a few tens of MB. The download itself names the area to the server, so do it
before the investigation, or for a wider box than the place of interest.

## Google maps (online)

The list at the top switches every Map node to Google's satellite photos, satellite with names, or
road map. It is the one online part of the map, so it starts on **Offline maps** every time and says
**ONLINE** while on: Google sees which part of the world you look at. It needs your own API key
from [Google Cloud](https://console.cloud.google.com/) with the Map Tiles API enabled, kept in
`settings\google_maps_key.txt` next to the program and never in a project, so you paste it once.
The node counts the tiles it requested since the program started; Google's free allowance is per
month. Tiles are only held in memory, and Google's terms allow viewing them only: no image
analysis, no extracting data, no offline use.

Files: `module.toml` (ports), `look.rhai`, `mod.rs`, `node.rs` (the map on the canvas, and `paint`:
the small world map other nodes draw), `earth.idx`, `coastlines.txt`.
