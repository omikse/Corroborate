# Module documentation

Status: planned and settled on 2026-11-17 (see Decisions). Nothing built yet.

## Why

Module READMEs are uneven: four are thorough (Cut detector, ENF, Audio match, SOURCER), each in
its own layout; nine are a paragraph and a file list, among them the ones others rest on (Grid
frequencies, Combine locations, Result record). The ENF README holds the grid data format and links
to a Grid frequencies README path that does not exist. Nothing checks a README against its
`module.toml`.

## Layout

A node is inputs, steps in between, and outputs. The README is built on three short tables that
follow each output back to its inputs; prose only where a row is not enough.

```markdown
# <Label> (<role>)

<What question it answers, in one paragraph.> Status: built / partly / planned.

## Inputs
| Input | Kind | Required | What it's for |

## Steps
| Step | From | Does | Settings |

## Outputs
| Output | From | Error | Tested |

## Limits

## Files
## Tests
```

Add a section only when it is needed: **Use** (what the node shows and what a person decides),
**Data** (reference data: source, licence, date), **Notes** (a step that needs more than a row),
**Sources**, **Next**.

A few rules: every Tested cell says what it was tested on, or "not yet"; every outside fact names
its source; a fact lives in one module and others link to it.

### Example: ENF (power hum), outputs

| Output | From | Error | Tested |
|---|---|---|---|
| `location` | S3 (50 or 60 Hz) | grid outline, 1° cells | Cornell Lab stream: 60 Hz |
| `hum track` | S2 (clean readings) | per reading | synthetic hum: median 4 mHz |
| `audio start time` | S4 (date against logs) | from the match | Hillingdon 2016: 1 s off |
| `audio end time` | — | — | planned |

## One home per fact

Nodes are also described outside their README (survey 2026-11-17). Each fact lives in one place;
the others link to it.

| Fact | Lives in | Elsewhere |
|---|---|---|
| A port's name, kind, required; an output's about, needs, when, source, status | `module.toml` (the app shows it on hover) | The README's Outputs table adds only From, Error, Tested |
| What `module.toml` keys mean | `src/module_manifest.rs` (top) | Main README, `docs/plugins.md`, Template link to it |
| How one node works, what it keeps, its limits | its README | Main README keeps app-wide ideas (canvas, queue, cache, time on wires, the location model) and links |
| Which modules exist | the module folders | Main README's table: links only, checked against the folders |
| A type's fields | its `type.toml` (format: `src/types.rs`) | Main README explains only the dots |
| Why it was designed so | `docs/<feature>/plan.md` (dated history) | READMEs link to it |
| What is built | each README's Status line | VISION.md and ARCHITECTURE.md link to the module list |

Code comments and on-canvas hover texts stay as they are: they describe code or label the UI.

Duplicates to remove in stages 2–3:
- ENF's "Use cases" table repeats its 16 outputs in `module.toml`: drop it.
- The main README's Time and Location sections describe Audio match, Time, Timeline, Combine,
  Invert, Map and its map setup: move what is node-specific to their READMEs.
- The main README lists what each node keeps; ENF's README too: keep it per module.
- The main README's module table misses Audio match, Cut detector, Time and Timeline.
- `module.toml` keys described in the main README and `docs/plugins.md`: replace with a link.
- VISION.md's "Where it stands" (grid matching "next") and ARCHITECTURE.md's "Implemented so far"
  (`src/bin/corroborate.rs`) are out of date: link to the module list instead.

## Check

A test (`tests/module_docs.rs`): the required sections are there, the Inputs and Outputs
tables have a row for each port in `module.toml`, relative links work, and the main README's module
table lists every module folder. It reports only, until
stage 3, then fails the build.

## Stages

1. The guide (`docs/module-docs.md`, about a page, the template above) and the check; fix the ENF link.
2. The thin modules: Grid frequencies (takes over the grid data format), Combine locations, Invert
   location, Result record, Map, Value, Timeline, Metadata, Plugin.
3. The rest: ENF, Cut detector, Audio match, SOURCER, Planes, ADS-B check, Time. The check then
   fails the build.

## Decisions (2026-11-17)

- Three tables (Inputs, Steps, Outputs) at the core; the guide stays short.
- Ports written by hand, checked against `module.toml`.
- One README per module; plans and research stay in `docs/<feature>/plan.md`.
- The check fails the build once every module follows the layout.
- One home per fact (table above); `module.toml` owns ports and output use cases.
