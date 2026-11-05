# Result record

Takes anything. It starts with one grey input; wire any output into it and the input takes that
output's colour and name, and a new grey input appears below. Unwiring packs the inputs again.

- **folder**: where results go (empty: `results\` next to the program). **Open** shows it.
- **Save now**: static inputs (metadata, facts, places, text, files) are saved at once into
  `static-<UTC>.json`, with every node they come from.
- **● Record / ■ End recording**: streams (audio, video, time, events, tracks). Each audio input's
  source is written to `<UTC>-<source>.wav` (16 kHz mono) from Record to End recording, and
  `corroborate-<UTC>.json` holds the findings of everything wired in (its evidence trail), saved
  every 30 s and at the end. Video frames are not recorded yet.

The graph runs always: there is no Run button. Changing a source, or what its audio feeds,
restarts it after a moment; **↻ Restart** in the top bar starts every source again from its
beginning.

Output: `location`, the whole world cut by every location wired in.

Files: `mod.rs` (name, label, ports, which kinds are streams), `node.rs` (the node in the
workbench: inputs, saving, recording, its WAV writer, the location).
