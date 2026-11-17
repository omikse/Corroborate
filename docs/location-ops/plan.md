# Locations without subtract

Status: planned on 2026-11-17 from that day's interview; built on 2026-11-18.

## The idea

A location is only a place: an area of the Earth, or a point. It no longer says whether it adds or
subtracts. Adding and taking away areas is one node's job, Combine locations, the way correcting a
time is the Time node's.

## Nodes

| Node | Does |
|---|---|
| Combine locations | one row per wired location, each with a sign: **+** adds, **−** takes away (a click turns it over, as on the Time node's rows). The mode joins the + rows: **all** must hold (AND) or **any** may (OR). Then every − row is taken away. With no + row, it starts from the whole world. An empty result is shown as CONTRADICTION. Its status line says what share of Earth is left; no map on the node |
| Invert location | everywhere except the area wired in, given as a plain area |
| Map | shows what is wired in, nothing more; every area in one colour (its input's), no green/red |
| Result record | records what is wired in; it no longer works out a location of its own (its `location` output goes) |

## Changes

- `src/area.rs`: `Area` loses `add`; `allowed()` is its cells. Invert gives the complement.
- `src/data.rs` and `src/types/location/type.toml`: the JSON form loses `"add"`.
- Combine locations: rows with a sign (`module.toml`, `look.rhai`, `node.rs`), mode for the + rows.
- Map: tint by input, not by add/subtract.
- Result record: the location output and its intersecting go.
- No backwards compatibility (dev stage): old projects with subtracting areas or a Result record
  location wire just lose them.
- Docs: the main README's type table and Examples, the Template's JSON table, and the Combine,
  Invert, Map and Result record READMEs.

## Tests

- + rows in all and any mode, − rows taken away, − rows only (from the whole world), an empty
  result.
- Invert of an area and of the whole world.
- The JSON form round-trips without `add`.

## Decisions (2026-11-17)

- No subtract in the location type; Combine locations does the arithmetic with a sign per row.
- Three nodes stay apart: Map shows, Invert inverts, Combine adds and takes away.
- The Result record only records.
- No preview map on Combine; wire a Map.
- No backwards compatibility.
- (2026-11-18, building) A row's sign is kept by the node wired in (`minus <node id>`, as Audio
  match keeps its rows), not by port: ports renumber when a wire goes. One node wired twice has
  one sign.
- (2026-11-18) Map colours each input from a fixed list (blue, green, orange, purple, cyan,
  yellow); pins take their input's colour too.
