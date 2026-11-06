# Grid frequencies (reference)

The nominal grid frequency (50 Hz or 60 Hz) for each region, as a JSON file. The ENF node reads it
to match a hum to a grid and a time. Leave the path empty to use the file bundled in this folder, or
choose another file. Output: `data` (JSON).

Files: `mod.rs` (name, label, ports), `node.rs` (the node in the workbench: finding and reading the file, its window, its part of the result file), `grid-frequencies.json` (the bundled data).
