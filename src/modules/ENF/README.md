# ENF (detector)

ENF (Electrical Network Frequency) is the mains hum. Power grids run at 50 Hz or 60 Hz, but the
real frequency wanders by tens of millihertz, the same way across a whole grid. Equipment near a
microphone leaks that hum into recordings. Its frequency tells which part of the world the
recording can be from. Its wandering, matched against grid frequency logs, tells when it was made.
Status: built (area, quality, hum track, start time from grid logs). Not yet tested on many real
recordings.

## Inputs

| Input | Kind | Required | What it's for |
|---|---|---|---|
| `audio` | audio | yes | the sound it listens to |
| `time` | time | no | the claimed start of the audio: the logs are searched ±1 h around it, and the match confirms it or not |
| `location` | location | no | the claimed place: only grid logs of grids that meet it are searched. It does not limit the `location` output; to join areas, use [Combine locations](<../Combine locations/README.md>) |

The **grid data** file is chosen in the node (empty: the bundled file of
[Grid frequencies](<../Grid frequencies (50 or 60 Hz)/README.md>)); see [Grid data file](#grid-data-file).

## Steps

| Step | From | Does | Settings |
|---|---|---|---|
| S1 Readings | `audio` | every second, the mains hum frequency over the last 16 s, with its score and error ([notes](#s1-readings)) | — |
| S2 Grid vote | S1 | 50 or 60 Hz, decided by readings in an 8 s consistent run | — |
| S3 Rejection | S1, S2 | readings that don't count are kept with the reason ([notes](#s3-rejection)) | — |
| S4 Runs | S3 | runs of clear hum of 30 s or more, to the timeline and the result file | — |
| S5 Quality | S3 | good enough to date, weak, or no hum, and why ([notes](#s5-quality)) | — |
| S6 Area | S2, grid data | every 1° cell whose centre is in a region of that frequency | grid data file |
| S7 Dating | S3, `time`, `location`, grid logs | at the end of a file, the hum matched against the grid log store ([notes](#s7-dating)) | — |
| S8 Start error | S7 | how sure the start is, from how sharp the match is ([notes](#s8-start-error)) | — |
| S9 Blind place | S7 | with no `location` wired, a clear match puts the recording in that grid's area at that date | — |

## Outputs

Each output is in its type's colour when the signal is good enough for its use (`min_quality` in
`module.toml`), and grey when it is too weak. A grey output gives nothing and says what it needs.

| Output | From | Error | Tested |
|---|---|---|---|
| `location` | S6, or S9 after a blind match | grid outlines are rough boxes: borders off by up to a few hundred km, at 1° | Cornell Lab stream (Ithaca, New York), no place wired: 60 Hz, 17.85 % of Earth left |
| `quality` | S5 | thresholds are placeholders | synthetic hum: good, weak and none told apart (`drive.rs::enf_quality`) |
| `hum track` | S1–S3 | each reading carries its own | synthetic hum in heavy noise: 22 readings, median 4 mHz off |
| `audio start time` | S7, S8 | from the match's sharpness (S8) | Hillingdon council stream (7 July 2016, on-screen 18:29:08 UTC): found 18:29:07 UTC, 4.1 mHz against 37 mHz next best. Synthetic: ±0.31 s for a clean track, ±2.3 s with 20× the reading noise |
| `audio end time` | — | — | planned: the start plus the length at the recording's own speed, from its drift against the log |

Eleven more outputs are possible uses of the hum, drawn greyed on the node with their text on
hover and nothing to wire. Their text is in `module.toml`; the research behind them is under
[Use cases](#use-cases).

## Limits

- One nominal frequency per run: a recording that moves between grids is not handled.
- The absence of hum is weak evidence only: battery devices still pick hum up, and shielded or
  filtered ones may not.
- The quality thresholds (7 dB, a 3:00 run, 10 mHz) and the match rules (10 mHz, 2× margin) are
  placeholders until tested on more real recordings and grid logs.
- A clean track that barely moves is flagged as a possible fan or tone and never called good. A
  real but very calm grid would be flagged too.
- Grid logs are stored as 50 Hz only: 60 Hz grids (the Americas, part of Japan) cannot be dated.
- The start error assumes one independent reading per 16 s window and an honest log. A wrong log
  or a long stretch of wrong readings gives a confident wrong start; the runner-up and the margin
  are shown so a person can judge.
- Grid data polygons that cross the 180° meridian are not handled.
- The grid log store is not part of what the kept result is known by: after adding grid logs,
  Run again on the node (right-click) dates anew. That reads the hum again too.

## Files

| File | Holds |
|---|---|
| `module.toml` | id, label, ports, each output's use, needs and `min_quality` |
| `README.md` | this |
| `mod.rs` | the detector: readings, grid vote, rejection, runs, quality |
| `node.rs` | the node in the workbench: its run, area, dating, outputs, look on the canvas, result file |
| `log_match.rs` | dating: the hum track against the grid log store, and the start's error |
| `grid_store.rs` | the grid log store's files |
| `grid_import.rs` | filling the store from raw downloads |
| `plugin.rs` | the same detector as a plugin for the old viewer (`enf-plugin.exe`) |

## Tests

| Test | Proves |
|---|---|
| `mod.rs::tracks_50hz_drift`, `finds_60hz_grid`, `noise_has_no_hum` | the hum and its grid are found; noise alone gives none |
| `mod.rs::weak_hum_in_noise`, `weaker_hum_no_wrong_readings` | weak hum gives readings; heavier noise gives none rather than wrong ones |
| `mod.rs::fan_does_not_win_over_hum` | a 60 Hz fan over 50 Hz hum is decided 50 |
| `mod.rs::rejected_readings_say_why`, `quality_levels`, `steady_tone_is_flagged` | rejections keep their reason; the quality verdict; a steady tone is not called good |
| `log_match.rs::finds_the_start_blind` | the true start found among 3 million, the FFT agreeing with brute force |
| `log_match.rs::start_error_from_sharpness` | a clean match is sure to about a second; noisier hum gives a larger error |
| `grid_store.rs::write_and_read_across_months`, `grid_import.rs::local_times` | the store's files; local time stamps to UTC |
| `drive.rs::enf_quality` | the node's three quality levels, with screenshots |
| `drive.rs::enf_date` (ignored: needs a real recording and the logs) | a real recording dated against the store |
| `kept_tests.rs::kept_results_load_when_the_project_opens` | the hum track and its dating are kept and loaded |

## Use

On the canvas the node shows everything itself: the Earth map of where its grid frequency is used,
the hum frequency over time under it (readings that count), the grid data file, the quality
verdict, GRID LOG MATCH (the verdict, what was searched, the best start and grid, its error, the
runner-up, the start's error) and a status column. A person reads the runner-up and the margin
before trusting a date.

## Notes

### S1 Readings

A 16 s window every second, a 262 144-point FFT (Fast Fourier Transform) with 0.015 Hz bins. For
50 and 60 Hz, the band around each harmonic 1–8 is divided by its own noise, rescaled to the
fundamental and summed with a weight per harmonic (its running power at the reading, so harmonics
without hum don't drown the ones with it): the combined spectrum. Each reading is searched within
±25 mHz of a 5 s average of it; the score is its excess over what noise alone gives (clear hum:
7 dB or more). The nominal frequency is decided by comparing the two sums with each harmonic
capped, so one loud tone (a fan) can't win.

### S3 Rejection

Readings that don't count are kept with the reason: too weak, grid not decided yet, slope rule (a
grid can't move faster than 2.5 mHz/s) or section rule (not inside 8 s of good readings). The
rules come from [libhum](https://github.com/RaphaelJ/libhum).

### S5 Quality

**Good enough to date**: a clean run of at least 3:00 with an error under 10 mHz. **Weak**: some
readings count (grid and hum track only). **No hum**. Next to it: the score, readings kept of all,
the longest clean run, the error and the harmonics that stood out, and how many readings were
rejected for each reason. The `location` output gives no area while no reading counts.

### S7 Dating

Grids of the hum's frequency that the store has, limited to those meeting a wired `location`;
around a wired `time` (±1 h) or blind over every second of their logs. Every start is scored at
once by FFT correlation: the rms of hum minus log smoothed over the 16 s window, mean removed (a
recorder's clock error shifts the whole track). Blind over all of GB 2014–2026 takes seconds. A
match counts when its error is under 10 mHz, the next best (2 min away, or in another grid) is at
least 2× worse, and, with a wired time, it lies within that time's error (at least 10 s).

### S8 Start error

The misfit is worked out again at the starts up to 4 s either side of the best one, and a
parabola is fitted to its square. The error is where the misfit, summed over the independent
readings, grows by one reading's worth of noise, as in a least-squares fit. Readings 1 s apart
share most of their 16 s window, so one in 16 counts as independent. Starts are whole seconds,
which adds 1/√12 s (root-sum-square). A match with no curvature gets 120 s, the span that counts
as one match.

## Data

### Grid data file

The same format the Grid frequencies node reads; swap the file to use other data. Format:

```json
{
  "source": "where the data came from",
  "regions": [
    {"id": "Europe", "frequency_hz": 50, "bounds": [[lat, lon], [lat, lon], [lat, lon], ...]}
  ]
}
```

`bounds` is a polygon of at least three `[lat, lon]` points. A cell counts for a frequency when its
centre is inside any region of that frequency, so overlapping regions allow both frequencies: the
map never wrongly excludes a cell, it only fails to exclude one. The default file is rough boxes
per country group.

It also lists the **synchronous areas** (one grid, one frequency everywhere at any second: what a
grid frequency log covers), each as location data that can change over time:

```json
"synchronous_areas": [
  {"id": "CE", "name": "Continental Europe", "frequency_hz": 50, "parts": [
    {"bounds": [[lat, lon], ...]},
    {"from": "2022-03-16", "bounds": [...]},
    {"from": "2025-02-09", "bounds": [...]}
  ]}
]
```

A part with `from`/`to` (UTC dates) belongs to the area only then: Ukraine and Moldova joined
Continental Europe on 2022-03-16, the Baltic states on 2025-02-09 (before that `Baltic`, with
Russia's grid). Shipped: GB, CE, Nordic, IE (Ireland), Baltic; outlines rough, at 1°. The `id` is
the grid's folder in the grid log store.

### Grid log store

`grid logs\` next to the program (or `CORROBORATE_GRID_LOGS`): `<grid id>\YYYY-MM.bin`, one file
per grid and UTC month, 1 s, the frequency minus 50 Hz in 0.1 mHz steps (i16, missing marked), a
small header (`grid_store.rs`); `index.json` lists each grid's months and how full they are. About
5 MB a month per grid. Stored now: GB, CE, Nordic.

Filled from raw downloads (untrusted, many formats) by
`corroborate-cli grid-logs <downloads folder>`, which reads only new or changed files
(`imported.txt`): run it again after a download tops up (`grid_import.rs`).

| Folder | Grid | Time stamps |
|---|---|---|
| `neso\` `f-YYYY-M.csv`, `fnew-…` (some zipped) | GB | with offset (`+0100`); 2014 `dd/mm/yyyy` UK local; `fnew` UTC |
| `zenodo_2012_2021\YYYY_cleansed.zip` | CE (TransnetBW), Nordic (Fingrid) | naive local with summer time (CET, EET) |
| `energy_charts\YYYY-MM-DD.json` | CE (Fraunhofer ISE, Freiburg) | unix seconds |

Open grid frequency data:

| Grid | Data | Resolution |
|---|---|---|
| Great Britain | [NESO system frequency](https://www.neso.energy/data-portal/system-frequency-data), monthly CSV files, NESO Open Data Licence | 1 s |
| Nordic | [Fingrid frequency history](https://data.fingrid.fi/en/datasets/339), monthly archives, times in Finnish local time | 0.1 s |
| Continental Europe, Great Britain, Nordic | [pre-processed series on Zenodo](https://zenodo.org/records/3744121), and the KIT [power grid frequency database](https://kit-iai-dracos.github.io/PGF_IAI/database/) | 1 s |

### Kept

Its hum track (readings, runs, the grid's frequency) and the dating are kept in the project's
`cache\`, under its SOURCER's sound (its file's size and print), its grid data file (and when that
changed) and the time and place wired in. Opening the project loads them (teal dot; the area is
worked out again), without reading the sound. A changed grid data file makes it stale: it reads its
SOURCER's sound again (from the kept sound, without decoding) and dates anew.

## Use cases

What the hum can tell, from published research and tools (checked 2026-11-06). **now**: the module
does it; **planned**: agreed next; **possible**: shown in the literature, not planned.

| Use case | What it gives | Needs | Status |
|---|---|---|---|
| Grid nominal frequency (50 or 60 Hz) | location: area of that frequency | hum in the audio | now |
| Audio start time | time: the recording's start in UTC ± error | grid frequency log for that time | now |
| Audio end time | time: the start plus the length at the recording's own speed against the grid | a long clear match against a log | planned |
| Which grid within one nominal frequency (e.g. Great Britain or continental Europe or Nordic) | location: that grid's area | logs of the candidate grids for that time | possible |
| Edits: cuts, splices, inserts | events: breaks in hum frequency or phase (for the cut detector) | clear hum | possible |
| Speed change or resampling | fact: speed factor (a steady 50.4 Hz hum means 0.8 % faster) | clear hum | possible |
| Tape speed wander (old analogue recordings) | fact: speed over time, to correct it | clear hum | possible |
| Claimed time check | contradiction (red) when the hum doesn't match the log at the claimed time | log | possible (falls out of the start time against the wired time) |
| Fake live stream (a pre-recorded loop) | contradiction: hum doesn't match the grid's near-real-time data | near-real-time grid data | possible |
| Time from grid events (power plant trips) | time: a published frequency dip found in the audio | event list with times | possible |
| Re-recording (audio played back and recorded again) | fact: two hum signatures overlaid | clear hum | possible |
| Synthetic or deepfake audio | weak hint: no or incoherent hum where one is expected | expectation of mains nearby | possible, weak |
| Location within one grid | location: rough, by trilateration | simultaneous recordings at known sites | possible, research stage |
| Hum from video (light flicker, rolling shutter) | the same track from silent video | mains lighting, video frames | possible, heavy |

## Sources

- [Wu et al. 2025](https://www.mdpi.com/2079-9292/14/24/4905): harmonics 2–7, 50/60 Hz decided by
  smoothness, harmonic chosen by noise rate, RFA (Robust Filtering Algorithm) cleaning. Its neural
  classifier is not taken (trained on its own data, opaque).
- [Su et al., ENF for multimedia synchronization](https://www.mast.umd.edu/files/paper_ENF/conf_paper8_ICASSP14.pdf):
  syncing audio and video and correcting tape speed with the hum.
- [Garg, Hajj-Ahmad, Wu 2021](https://arxiv.org/pdf/2105.00668): the hum differs slightly across one
  grid; location by trilateration.
- [Vatansever, Dirik, Memon 2019](https://arxiv.org/pdf/1903.09889): hum from video through rolling
  shutter.
- [Robust ENF estimation, harmonic enhancement](https://arxiv.org/pdf/2011.03414); tamper detection:
  [audio](https://pmc.ncbi.nlm.nih.gov/articles/PMC10458025/),
  [video](https://pmc.ncbi.nlm.nih.gov/articles/PMC12609218/).
- [Factors affecting ENF matching](https://www.sciencedirect.com/science/article/pii/S2352864823000226).
- Code: [libhum](https://github.com/RaphaelJ/libhum) (extract and match, a 1 h signal against 10
  years in about 20 s), [ENF_extraction](https://github.com/geokarant/ENF_extraction) (MATLAB:
  STFT, Capon, ESPRIT, MUSIC estimators).
