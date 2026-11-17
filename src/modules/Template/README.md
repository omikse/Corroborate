# Template (role)

What question the node answers, in one paragraph and in an investigator's words. Status: built /
partly built / planned. Plan: a link to `docs/<feature>/plan.md`, if there is one.

Copy this folder to start a module: rename it to the module's label, set `id`, `label` and
`category` in `module.toml`, and replace every line here. The layout is explained in
[docs/module-docs/plan.md](../../../docs/module-docs/plan.md).

## Inputs

One row per input in `module.toml`.

| Input | Kind | Required | What it's for |
|---|---|---|---|
| `audio` | audio | yes | the sound the node reads |
| `time` | time | no | what it changes when wired |

## Steps

The work between inputs and outputs, in order. A step that needs more than a row gets a note
under Notes, linked from its row.

| Step | From | Does | Settings |
|---|---|---|---|
| S1 Name | `audio` | one line | value (default, and why) |
| S2 Name | S1, `time` | one line ([notes](#s2-name)) | — |

## Outputs

One row per output in `module.toml`, planned ones too. Tested says on what and the measured
number, or "not yet".

| Output | From | Error | Tested |
|---|---|---|---|
| `events` | S2 | how big, and where it comes from | test case: measured number |
| `location` | — | — | planned |

## Limits

What it cannot do or gets wrong, and what a wrong result looks like.

## Files

| File | Holds |
|---|---|
| `module.toml` | id, label, ports |
| `README.md` | this |

## Tests

| Test | Proves |
|---|---|
| `file.rs::test_name` | what it checks |

<!-- Optional sections, only when needed; delete the rest:

## Use
What the node shows on the canvas and what a person decides.

## Notes
### S2 Name
More than a row can hold.

## Data
Reference data: what it holds, source, licence, date, accuracy.

## Sources
| Source | Taken from it |
|---|---|

## Next
Planned work, linking to a plan.
-->
