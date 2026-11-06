# Power hum (ENF)

ENF (Electrical Network Frequency): the mains hum and its drift, and from it, where the recording
can have been made.

Node: `audio` in. Out, one per use case (see Use cases): `location` (grid 50/60 Hz: the area of
that frequency, wire it into a Map to see the coverage), `quality` (the verdict below) and
`hum track` (the frequency over time) are built; the 13 others are drawn greyed with their text on
hover and nothing to wire. Each output is in its type's colour when the signal is good enough for
its use case (`min_quality` in module.toml) and grey when it is too weak; a grey built output gives
nothing and says what it needs. The runs of clear hum go to the timeline and the result file.

On the canvas the node shows everything itself (no separate window): the Earth map of where its grid
frequency is used (green), the hum frequency over time under it (readings that count), the **grid
data** file chosen right in the node (empty: the bundled file), the **quality** verdict and a status
column (state, grid frequency, hum runs, audio, area).

Files: `mod.rs` (the detector: name, label, ports), `node.rs` (the node in the workbench: state while
running, the area its grid frequency allows, its look on the canvas, timeline marks, result file), `plugin.rs` (the same
detector as a plugin for the old viewer, `enf-plugin.exe`). The default grid map is in its own module:
[grid_frequencies](../grid_frequencies/README.md).

## Detecting the hum

A 16 s window every second, a 262 144-point FFT (Fast Fourier Transform) with 0.015 Hz bins. For
50 and 60 Hz, the band around each harmonic 1–8 is divided by its own noise, rescaled to the
fundamental and summed with a weight per harmonic (its running power at the reading, so harmonics
without hum don't drown the ones with it): the combined spectrum. Each reading is searched within
±25 mHz of a 5 s average of it; the score is its excess over what noise alone gives (clear hum:
7 dB or more). The nominal frequency is decided by comparing the two sums with each harmonic capped,
so one loud tone (a fan) can't win, and only readings in an 8 s consistent run vote. Each reading
carries its error.

Readings that don't count are kept with the reason: too weak, grid not decided yet, slope rule (a
grid can't move faster than 2.5 mHz/s) or section rule (not inside 8 s of good readings). The
rejection rules come from [libhum](https://github.com/RaphaelJ/libhum). Runs of clear hum of 30 s
or more are reported.

Measured on synthetic hum (harmonics 2, 3, 5 only, drifting ±20 mHz): in heavy noise 22 readings
with a median error of 4 mHz (the detector before: 33 at 6.5 mHz; libhum 12 at 1.6 mHz); in heavier
noise none, where the detector before gave readings about 108 mHz wrong. A 60 Hz fan over 50 Hz hum
is decided 50. Not yet tested on real recordings.

## Quality

The node says whether the hum is good enough, and why: **good enough to date** (a clean run of at
least 3:00 with an error under 10 mHz), **weak** (some readings count: grid and hum track only) or
**no hum**. Next to it: the score, readings kept of all, the longest clean run, the error and the
harmonics that stood out, each with what it needs, and how many readings were rejected for each
reason. A clean track that barely moves is flagged as a possible fan or tone (a grid always wanders)
and never called good. The thresholds are placeholders until tested on real recordings and grid
logs. The `location` output gives no area while no reading counts.

## From hum to area

Once the nominal frequency is decided, the node's area is every 1° cell (see `src/area.rs`) whose
centre lies in a grid region of that frequency. To combine it with a known place, wire both into a
Combine locations node or the Result record: an empty result is kept and labelled CONTRADICTION
(the hum is not used anywhere in that place, so one of them is wrong). Nothing is hidden or smoothed
over.

Tested live on the Cornell Lab stream (Ithaca, New York) with no location given: 60 Hz found, the
world cut to 17.85 % of Earth.

## Grid data file

Chosen in the node (the same format the Grid frequencies node reads); swap the file to use other
data. It is read at each Run. Format:

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
map never wrongly excludes a cell, it only fails to exclude one. Polygons that cross the 180°
meridian are not handled.

The default file is rough boxes per country group: borders are off by up to a few hundred km.

## Use cases

What the hum can tell, from published research and tools (checked 2026-11-06). Status: **now**
(the module does it), **planned** (agreed next), **possible** (shown in the literature, not planned).
Output: what it would give the project. These are the node's outputs: their text, status and the
signal each needs live in `module.toml` and show on the node.

| Use case | What it gives | Needs | Status |
|---|---|---|---|
| Grid nominal frequency (50 or 60 Hz) | location: area of that frequency | hum in the audio | now |
| Sync recordings to each other | link: offset between two clocks ± error | two recordings overlapping in time, same grid | planned |
| Time of recording | anchor: audio moment = UTC ± error | grid frequency log for that time | planned |
| Which grid within one nominal frequency (e.g. Great Britain or continental Europe or Nordic) | location: that grid's area | logs of the candidate grids for that time | possible |
| Edits: cuts, splices, inserts | events: breaks in hum frequency or phase (for the cut detector) | clear hum | possible |
| Speed change or resampling | fact: speed factor (a steady 50.4 Hz hum means 0.8 % faster) | clear hum | possible |
| Device clock drift | fact: sample-rate error in ppm, for the clocks | long recording plus a log or a second recording | possible |
| Tape speed wander (old analogue recordings) | fact: speed over time, to correct it | clear hum | possible |
| Claimed time check | contradiction (red) when the hum doesn't match the log at the claimed time | log | possible (falls out of anchors) |
| Fake live stream (a pre-recorded loop) | contradiction: hum doesn't match the grid's near-real-time data | near-real-time grid data | possible |
| Time from grid events (power plant trips) | anchor: a published frequency dip found in the audio | event list with times | possible |
| Re-recording (audio played back and recorded again) | fact: two hum signatures overlaid | clear hum | possible |
| Synthetic or deepfake audio | weak hint: no or incoherent hum where one is expected | expectation of mains nearby | possible, weak |
| Location within one grid | location: rough, by trilateration | simultaneous recordings at known sites | possible, research stage |
| Hum from video (light flicker, rolling shutter) | the same track from silent video | mains lighting, video frames | possible, heavy |

The absence of hum is weak evidence only: battery devices still pick hum up, and shielded or filtered
devices may not.

**Open grid frequency data**

| Grid | Data | Resolution |
|---|---|---|
| Great Britain | [NESO system frequency](https://www.neso.energy/data-portal/system-frequency-data), monthly CSV files, NESO Open Data Licence | 1 s |
| Nordic | [Fingrid frequency history](https://data.fingrid.fi/en/datasets/339), monthly archives, times in Finnish local time | 0.1 s |
| Continental Europe, Great Britain, Nordic | [pre-processed series on Zenodo](https://zenodo.org/records/3744121), and the KIT [power grid frequency database](https://kit-iai-dracos.github.io/PGF_IAI/database/) | 1 s |

**Methods and tools seen**

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

## Limits

- One nominal frequency per run; a recording that moves between grids is not handled.
- The drift itself (matching against grid-frequency logs to date a recording) is measured and
  shown, not yet used.
