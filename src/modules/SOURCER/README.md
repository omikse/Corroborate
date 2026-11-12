# SOURCER (source)

Any source in one node. A new node is two boxes and one input, its outputs hidden. Type or paste a source in the first box, or Browse for a file:
- a file: mp3, wav, flac, ogg, m4a, video files, anything ffmpeg reads;
- a web page: YouTube videos and live streams, and the other sites yt-dlp knows;
- a direct stream URL: http(s) media files, HLS (`.m3u8`), rtmp, rtsp;
- a system stream: pick it in the second box, which lists every audio device ffmpeg sees (microphones, digital and virtual mics, capture cards). It is written `mic:<device>` into the source.

System audio (what you hear) and per-application capture are not supported: they need WASAPI loopback, which does not go through ffmpeg.

The line under the field says which of these it is. Web pages are resolved with yt-dlp, then ffmpeg
decodes. Files, YouTube videos and media files on a server are read as recordings, as fast as they decode;
live streams and microphones are read live (UTC (Coordinated Universal Time) from the PC clock at
first audio). yt-dlp says whether a page is live. A video that is not live is downloaded by yt-dlp into
the project's `recordings\` folder first (YouTube refuses its direct links to ffmpeg), with its picture
(at most 480 lines), and kept there as evidence: the project holds its own copy, and moves with its
folder. A project not saved yet has no folder: it downloads into `recordings\` next to the program, and
once saved, its next run copies the video from there into its own folder (no download). The folder's
`downloads.txt` lists each link with its file (by name, so the list holds when the folder moves), so a
later run (any change upstream or downstream) reads the copy without calling yt-dlp; YouTube links
match by video id. Delete the file in the project's `recordings\` to download again. The download's
percent (yt-dlp's progress lines) is on the status line, the state dot and the queue list; Stop ends
the download. A link is
live only once yt-dlp says so: while it checks the link or downloads, the source is not live (its
wires do not pulse, its playhead does not follow the audio coming in).

When YouTube answers "Sign in to confirm you're not a bot", the SOURCER asks again with the cookies
of a browser signed in to YouTube (yt-dlp reads them locally: Firefox, then Edge, Chrome, Brave;
Chromium browsers usually only while closed) and keeps using the first that works. To choose
yourself, put yt-dlp options one per line in `settings\yt-dlp.txt` next to the program, e.g.
`--cookies-from-browser firefox` or `--cookies C:\path\cookies.txt`; then only those are used.

Ports:
- input `text` (optional): text from another node replaces what is typed in the field. No node gives
  text yet; when it is wired the node says so and uses the field.
- outputs `audio`, `time`, `length`, `metadata`, `video` and `location`, all declared in
  `module.toml`. A file's time and length are read once, before any Run, and a Run does not clear
  them. `time` is what the file says, raw, with an error of 0: the whole project reads it as the
  start of the recording, and a Time node corrects a file that means something else.
  `look.rhai` hides them until a source is chosen, `video` unless it has pictures (video files, web
  pages, video streams), and `location` until the file's metadata gives coordinates (ISO 6709 in
  `location`, `location-eng` or `com.apple.quicktime.location.ISO6709`: the exact point, shown on
  a Map node wired to it). A wired output is always shown, so emptying the source box keeps wires as
  they are.
- setting `source`: the path, URL or `mic:<device>` in the first box.

The run's steps (download for a web page, then decode, with percent from the file's length) and
its state are on the dot before its name and in the queue list. In the queue it needs nothing: it
waits only for a turn (2 downloads at once, half the processor cores decoding, `settings\run.txt`).
The nodes reading its sound as it decodes (ENF, Planes, plugins) run with it: asking for one of
them runs the SOURCER again, feeding the ones queued (the others keep their results); they wait
for it there ("for 'phone 3' (download 34%)") and fail with it ("needs 'phone 3', which failed").
A Cut detector or an Audio match after it starts once its run is done, and reads its decoded sound
(and its downloaded copy). A change to its input, or to the file itself (its size and print), makes
it and what follows stale.

Kept (`src/bin/corroborate/kept.rs`): once it has decoded a file or a web video that is not live,
its sound (16 kHz, 16 bit, about 85 MB for 44 minutes) is kept in the project's `cache\`, with the
file it opened, its time anchor and ffprobe's view of it. Opening the project reads it back (the
waveform, the sound for playing and for the nodes after it; nothing decodes), and a run reads it
instead of decoding, feeding the ENF, Planes or plugins on it (its step is then "read the kept
sound", its dot teal). A file is known by its size and print, a web video by its link; two
SOURCERs on one file share its kept sound. A microphone or a live stream is never kept. Run again
decodes anew.

On the canvas: the two boxes, then (once there is a source) status and UTC clock, the waveform (the whole recording, or the last 30 s when live;
hover it to read the time), and for video a viewer with one small frame a second (hover the waveform
to scrub it). `metadata` is ffprobe's view of the source (format, duration, tags, streams) plus what
was opened; it goes into the result JSON. While a live stream runs, the wires of the nodes it feeds
pulse towards their inputs.

Projects saved with the old File, Stream or Microphone nodes load into this node.

Files: `module.toml` (ports, setting, look), `look.rhai` (which outputs show, the node's size),
`mod.rs` (which outputs it gives), `node.rs` (the node on the canvas, what it opens at Run,
resolving web pages with yt-dlp, the waveform, frames and metadata).
