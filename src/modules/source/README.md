# Source data (source)

Any audio source in one node. Type or paste it, or Browse for a file:
- a file: mp3, wav, flac, ogg, m4a, video files, anything ffmpeg reads;
- a web page: YouTube videos and live streams, and the other sites yt-dlp knows;
- a direct stream URL: http(s) media files, HLS (`.m3u8`), rtmp, rtsp;
- a microphone: pick it from **Mic ▾** (written `mic:<device>`).

The line under the field says which of these it is. Web pages are resolved with yt-dlp, then ffmpeg
decodes. Files, YouTube videos and media files on a server are read as recordings, as fast as they decode;
live streams and microphones are read live (UTC (Coordinated Universal Time) from the PC clock at
first audio). yt-dlp says whether a page is live. A video that is not live is downloaded by yt-dlp into
`recordings\` first (YouTube refuses its direct links to ffmpeg) and kept there as evidence. Ports: outputs `audio` and `time`. No inputs.
Projects saved with the old File, Stream or Microphone nodes load into this node.

Files: `mod.rs` (name, label, ports), `node.rs` (the node on the canvas, what it opens at Run,
resolving web pages with yt-dlp).
