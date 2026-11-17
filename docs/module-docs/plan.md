# Module documentation

Status: settled on 2026-11-17 (see Decisions). Stage 1 done: the Template module is the guide, the
ENF link is fixed, and the rules are in [CLAUDE.md](../../CLAUDE.md). Stages 2 and 3 not started.

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
| What `module.toml` keys mean | `src/module_manifest.rs` (top) | Template's Making a module and `docs/plugins.md` link to it |
| How one node works, what it keeps, its limits | its README | The workbench README keeps app-wide behaviour (canvas, queue, cache, projects, playhead) and links |
| Why the tool exists, its principles | the main README (VISION.md merged into it, 2026-11-17) | nowhere else |
| Which modules exist | the module folders | Main README's table: links only, checked against the folders |
| Which types exist, what each carries | the main README's type table | a type's fields: its `type.toml` (format: `src/types.rs`); their JSON: the Template README |
| What a dot's colour, shape and fill mean | the workbench README (`src/bin/corroborate/README.md`) | |
| Where code lives | `docs/ARCHITECTURE.md` (a code map) | each file's opening comment |
| How to work on the repo (commits, docs, tests) | `CLAUDE.md` | |
| Why it was designed so | `docs/<feature>/plan.md` (dated history) | READMEs link to it |
| What is built | each README's Status line | the main README does not say |

Code comments and on-canvas hover texts stay as they are: they describe code or label the UI.

Duplicates to remove in stages 2–3:
- ENF's "Use cases" table repeats its outputs in `module.toml`: drop it.
- `module.toml` keys described in `docs/plugins.md`: replace with a link.
- Done 2026-11-17 with the README/VISION merge: the main README's node-specific Time, Location and
  map setup moved to their modules, the module table lists every module, ARCHITECTURE.md became a
  code map, VISION.md was merged into the main README.

## Check

Dropped on 2026-11-18: no test checks the READMEs. Whoever changes a module (person or agent)
keeps its README in step, following [CLAUDE.md](../../CLAUDE.md) and the Template.

## Stages

1. The guide (the Template module) and the rules (CLAUDE.md); fix the ENF link. Done.
2. The thin modules: Grid frequencies (takes over the grid data format), Combine locations, Invert
   location, Result record, Map, Value, Timeline, Metadata, Plugin.
3. The rest: ENF, Cut detector, Audio match, SOURCER, Planes, ADS-B check, Time.

## Decisions (2026-11-17)

- Three tables (Inputs, Steps, Outputs) at the core; the guide stays short.
- Ports written by hand, checked against `module.toml`.
- One README per module; plans and research stay in `docs/<feature>/plan.md`.
- ~~The check fails the build once every module follows the layout.~~ Dropped 2026-11-18: a
  prompt (CLAUDE.md) instead of a test.
- One home per fact (table above); `module.toml` owns ports and output use cases.
