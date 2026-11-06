# Value (source)

One value of any data type, typed in on the canvas: a place, a time, a note, a list of events, or
a type a module brings. Pick the type at the top; the node then asks for that type's fields (its
`type.toml`, see Data types in the main README) and its one output, `value`, takes that type.

- A value type (location, time, text, file, JSON, …) has one row per field. A field the type marks
  as needed is hinted "needed"; until it is filled the status says what to fill in.
- A list type (event, track) has items: **+ item** adds one, 🗙 removes it.
- Changing the type is a setting change (kept in the result file); the fields start again, and
  wires the new type does not fit are removed.

Wired into a plugin (protocol 1), its filled fields are passed as `--manual name=value` when the
plugin starts; a latitude and longitude also as `coordinates=lat, lon` (ADS-B check reads the
site this way).

Files: `module.toml`, `mod.rs` (the type's rows, its JSON and value), `node.rs` (the node in the
workbench: the type picker and field editors, status, result file).
