# Metadata (plugin)

What a source says about itself: no audio analysis, only facts. A separate program speaking the
plugin protocol ([docs/plugins.md](../../../../docs/plugins.md)), built as `meta-plugin.exe`.

It reports ffprobe's container, duration, size, codec, every tag
(title, artist, creation_time…), coordinates from ISO 6709 location tags (phones and cameras write these),
other streams such as cover art, and the file system's created and modified times. Each value is labelled
with where it came from. Note that file system times are when the file arrived on this disk, which is
often not when the audio was recorded.
