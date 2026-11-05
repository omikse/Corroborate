# File (source)

Reads a recording from disk and gives its audio and time to the rest of the graph. Any WAV file, or
anything ffmpeg can decode, works. Ports: outputs `audio` and `time`. No inputs.

Files: `mod.rs` (name, label, ports), `node.rs` (the node in the workbench: its window and what it opens at Run).
