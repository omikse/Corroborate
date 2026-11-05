# Plugin protocol

A plugin is a separate program, written in any language. The viewer starts each plugin that is switched
on when a source opens, sends it the same audio the built-in modules get, and shows what it reports. A
plugin that crashes stops on its own; the viewer carries on and says so in the plugin's box.

- **Plugins ▸ Import plugin…** adds a program; the list there switches plugins on or off and removes them.
  Changes restart the current source so the plugins see it from the first sample (not while recording).
  The list is kept in the settings file, so plugins are imported once.
- Plugins that come with the viewer (the other programs in its folder: `enf-plugin`, `meta-plugin`,
  `adsb-check`) are added and switched on by themselves the first time the viewer sees them. One you remove
  stays removed (`plugin_removed=` in the settings file); importing it again brings it back.
- **Arguments**: `--source <path or URL>` and `--file` or `--live`, so a plugin can read the source's own
  metadata.
- **Audio in** (stdin): raw f32 little-endian, mono, 16 000 Hz, from the first sample. End of input is the
  end of the source. Files wait for every plugin; live streams never do, and audio a plugin could not keep
  up with is counted in its box. A plugin must read stdin to the end, even if it ignores the audio.
- **Messages out** (stdout): one JSON object per line, times in seconds from the first sample:
  `hello` (name, about, colour), `status` (text for its box), `view` (the band its header shows),
  `track` (a point of a measured frequency line, drawn dashed over the spectrogram) and `finding`
  (start, end, band, label, detail: listed in its box, marked on the bars, written to the result JSON) and
  `meta` (a key and value: a fact about the source for the result JSON, listed in its box).
- **stderr**: free text; the last line is shown in its box.

The protocol, with a reader and writers for Rust plugins, is in `src/plugin.rs`. Two complete examples:
- `enf-plugin`: tracks and findings from the audio. On the reference hour it runs in about 5 s on its own.
