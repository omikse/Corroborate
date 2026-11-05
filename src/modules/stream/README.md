# Stream (source)

Takes audio from a URL and plays it through the graph as it arrives. Web pages are resolved with
yt-dlp, and ffmpeg decodes the result. Ports: outputs `audio` and `time`. No inputs.

Files: `mod.rs` (name, label, ports), `node.rs` (the node in the workbench: its window, what it opens at Run, resolving web pages with yt-dlp).
