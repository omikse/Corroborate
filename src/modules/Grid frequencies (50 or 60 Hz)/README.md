# Grid frequencies (reference)

Which places on Earth use 50 Hz mains power and which 60 Hz, and the synchronous areas (one grid,
one frequency everywhere at any second). The [ENF](../ENF/README.md) node reads it to turn a hum
into an area, and to know which grid logs cover a place. Status: built.

## Inputs

None. The file is chosen in the node: leave the path empty for the file bundled in this folder, or
choose another file in the same format.

## Steps

| Step | From | Does | Settings |
|---|---|---|---|
| S1 Read | the file | reads and checks it when the graph (re)starts | the file's path (empty: bundled) |

## Outputs

| Output | From | Error | Tested |
|---|---|---|---|
| `data` | S1 | as the file: 1° cells, see [Data](#data) | `area.rs::synchronous_areas`: Tokyo 50, Osaka 60, São Paulo 60, Asunción 50, Riyadh 60, Barbados 50, San Juan 60; Copenhagen Nordic, Esbjerg CE, Sardinia and Gran Canaria in no logged grid |

## Limits

- A cell counts when any part of a country lies in it, so a 1° cell on a border counts for both
  sides (up to about 100 km past a border). It never wrongly excludes a place; it may fail to
  exclude one.
- One frequency per country, except Japan (split along the Fuji River – Itoigawa line) and French
  Polynesia (both). Single towns or islands that differ from their country (Petit Saint Vincent,
  60 Hz) are not split out.
- Antarctica is left out: no grid, stations run their own generators at 50 or 60 Hz.
- Synchronous areas: islands joined only by DC links are cut out where they are big enough to
  matter (Canaries, Balearics, Corsica, Sardinia, Shetland, the East Aegean islands). Smaller ones
  are not checked. Ukraine's "Burshtyn island" (synchronous with Continental Europe before 2022)
  and Malta (since 2015) are not in it.

## Files

| File | Holds |
|---|---|
| `module.toml` | id, label, ports |
| `README.md` | this |
| `grid-frequencies.json` | the bundled data, made by `tools/grid_cells.py` |
| `mod.rs` | the bundled file's name |
| `node.rs` | the node in the workbench: finding and reading the file, its field on the canvas, its part of the result file |

## Tests

| Test | Proves |
|---|---|
| `area.rs::synchronous_areas` | places fall in the right frequency and grid, and parts join at their dates |
| `area.rs::grid_mask_and_contradiction` | the format is read; cells outside the grid are refused |

## Data

```json
{
  "source": "where the data came from",
  "regions": [
    {"id": "Europe 50 Hz", "frequency_hz": 50, "countries": ["…"], "cells": [[row, first_col, last_col], …]}
  ],
  "synchronous_areas": [
    {"id": "CE", "name": "Continental Europe", "frequency_hz": 50, "parts": [
      {"units": ["…"], "cells": [[row, first_col, last_col], …]},
      {"units": ["UKR", "MDA"], "from": "2022-03-16", "cells": […]},
      {"units": ["EST", "LVA", "LTU"], "from": "2025-02-09", "cells": […]}
    ]}
  ]
}
```

Cells are 1° runs, as in a `location` (row 0 is 89–90° N, column 0 is 180–179° W). A cell in
regions of both frequencies allows both. A part with `from`/`to` (UTC dates) belongs to its area
only then. `countries` and `units` say what went in and are not read.

The bundled file is made by `python -I tools/grid_cells.py <download folder>` from:

| Source | Taken from it | Licence |
|---|---|---|
| [Natural Earth](https://www.naturalearthdata.com) 1:10m admin-0 map units, [v5.1.2](https://github.com/nvkelso/natural-earth-vector/tree/v5.1.2) | every country's border (the UK split into its countries, Belgium into its regions) | public domain |
| [Wikipedia, Mains electricity by country](https://en.wikipedia.org/wiki/Mains_electricity_by_country), read 2026-11-18 | which countries use 60 Hz (43, plus Japan's west and French Polynesia); every other one 50 Hz | CC BY-SA |
| [ENTSO-E](https://www.entsoe.eu): synchronous areas | which countries are in GB, Ireland, Nordic, Continental Europe and Baltic; Turkey joined 2010-09-18, Ukraine and Moldova 2022-03-16, the Baltic states 2025-02-09 | facts |

Shipped areas: GB, IE (Ireland and Northern Ireland), Nordic (Norway, Sweden, Finland, Åland,
East Denmark), CE, Baltic. Denmark is split at the Great Belt: cells from 10° to 11° E count for
both CE and Nordic.
