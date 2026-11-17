# Combine locations (transform)

Where can it be, given several locations at once? Each wired location is a row with a sign: **+**
adds it, **−** takes it away (a click on the sign turns it over). The + rows are joined by the
mode, **all** (every one must hold: AND) or **any** (at least one may: OR), then every − row is
taken away. With no + row it starts from the whole world. Status: built. Plan:
[docs/location-ops/plan.md](../../../docs/location-ops/plan.md).

## Inputs

| Input | Kind | Required | What it's for |
|---|---|---|---|
| `location 1`, `location 2`, … | location | no | one row each; a new one appears as each is wired (up to 64) |

## Steps

| Step | From | Does | Settings |
|---|---|---|---|
| S1 Join | the + rows | AND (all) or OR (any) of their cells; the whole world when there is none | `mode`: all (default) or any |
| S2 Take away | S1, the − rows | removes each − row's cells | each row's sign, + by default |

## Outputs

| Output | From | Error | Tested |
|---|---|---|---|
| `location` | S2 | the 1° cells of its inputs | `mod.rs` tests and `drive.rs::combine_adds_and_takes_away`: exact cell sets |

## Use

Each row shows the node it comes from and how much of Earth that location covers; the status line
says how much is left. An empty result is shown as CONTRADICTION: the locations cannot all be true.
There is no map on the node: wire its output into a Map.

## Limits

- A sign belongs to the node wired in (kept as a `minus <node id>` row in the project), so one node
  wired into two rows has the same sign on both.
- Cells are 1°, so a small place counts as its whole cell.

## Files

| File | Holds |
|---|---|
| `module.toml` | id, label, the all/any variants, ports, the `mode` setting |
| `mod.rs` | the arithmetic (`combine`) and the signs' rows |
| `node.rs` | the node in the workbench: its rows, signs and the area it gives |
| `README.md` | this |

## Tests

| Test | Proves |
|---|---|
| `mod.rs::plus_rows_join_by_mode` | + rows join with AND or OR |
| `mod.rs::minus_rows_are_taken_away` | − rows are removed from the + rows |
| `mod.rs::minus_only_starts_from_the_whole_world` | − rows alone leave the rest of the world |
| `mod.rs::nothing_left_is_a_contradiction` | an empty result |
| `drive.rs::combine_adds_and_takes_away` | a click turns a sign over, it is saved and opened again, Invert agrees |
