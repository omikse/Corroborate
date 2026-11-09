# Map (view)

Everything wired into it on one map you can move and zoom, on the canvas. Wire a location in at any
step of a chain to see that step, and use one Map per step to compare them. It takes as many
locations as you wire (`location 1`, `location 2`, …); no outputs.

Points (a typed place, a file's GPS) are pins in their clock group's colour; a click selects the
node they come from. Areas are their 1° cells: green adds, red subtracts. New inputs are framed by
themselves. The wheel zooms, a drag moves, the status line says what is on it.

The list at the top picks the map, for all Map nodes: offline (the built-in world image, or
`.pmtiles` files in `maps\` next to the program) or Google's satellite and road maps (your API key,
pasted once; see the main README, Maps).

Files: `mod.rs` (name, label, ports), `node.rs` (the map on the canvas, and `paint`: the small world
map other nodes draw), `earth.idx` (NASA Blue Marble, public domain), `coastlines.txt` (Natural
Earth 1:110m).
