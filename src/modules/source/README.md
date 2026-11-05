# Source data (source)

Any audio source in one node. Type or paste it, or Browse for a file:
- a file: mp3, wav, flac, ogg, m4a, video files, anything ffmpeg reads;
- a web page: YouTube videos and live streams, and the other sites yt-dlp knows;
- a direct stream URL: http(s) media files, HLS (`.m3u8`), rtmp, rtsp.

The line under the field says which of these it is. Web pages are resolved with yt-dlp, then ffmpeg
decodes. Files are read as a recording (UTC (Coordinated Universal Time) from file metadata); URLs
are live (UTC from the PC clock at first audio). Ports: outputs `audio` and `time`. No inputs.
Projects saved with the old File or Stream nodes load into this node.

Files: `mod.rs` (name, label, ports), `node.rs` (the node on the canvas, what it opens at Run,
resolving web pages with yt-dlp).
