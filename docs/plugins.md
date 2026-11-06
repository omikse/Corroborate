# Plugin protocol

A plugin is a separate program, written in any language. The viewer starts each plugin that is switched
on when a source opens, sends it the same audio the built-in modules get, and shows what it reports. A
plugin that crashes stops on its own; the viewer carries on and says so in the plugin's box.

- **Plugins ▸ Import plugin…** adds a program; the list there switches plugins on or off and removes them.
  Changes restart the current source so the plugins see it from the first sample (not while recording).
  The list is kept in the settings file, so plugins are imported once.
- Plugins that come with the tool (programs in its own folder and in its module folders,
  `modules\<module>\`: `enf-plugin`, `metadata`, `adsb-check`) are added and switched on by themselves
  the first time the viewer sees them. One you remove
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

## Protocol 2: the data bus

A module folder whose `module.toml` says `[processing] protocol = 2` declares its ports and settings
there, and its program talks in JSON lines both ways:

```toml
[[input]]
name = "place"
kind = "location"        # audio video time location track event file text json

[[output]]
name = "seen"
kind = "event"

[[setting]]
name = "label"
type = "text"            # text path number bool choice (number: min, max; choice: choices)
default = "got a place"

[processing]
program = "bus_echo.exe"
protocol = 2
```

- **Arguments:** `--set name=value` for each setting, already checked. A changed setting restarts it.
- **stdin:** `{"type":"audio","pcm":"<base64 f32 LE mono 16 kHz>"}` when audio is wired in,
  `{"type":"in","port":"place","value":…}` at start and whenever that input's value changes, and
  `{"type":"end"}` when the audio source ends. It starts when the graph runs, with or without audio.
- **stdout:** the messages above, plus `{"type":"out","port":"seen","value":…}`. Event and track
  outputs grow by each list sent; the other kinds keep the latest value. The value goes on to every
  input wired to that output, and into the result file.
- **Values:** one JSON form per kind, in `src/data.rs`. A location may be sent as
  `{"lat":50.1,"lon":14.4,"radius_km":25}` and is received as 1° cells.

`examples/bus_echo.rs` is a complete one (the workbench's tests run it).

The protocol, with a reader and writers for Rust plugins, is in `src/plugin.rs`. Two complete examples:
- `enf-plugin`: tracks and findings from the audio. On the reference hour it runs in about 5 s on its own.
