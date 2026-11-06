# Power hum (ENF)

ENF (Electrical Network Frequency): the mains hum and its drift, and from it, where the recording
can have been made.

Node: `audio` in, `location` (an area) out. The runs of clear hum go to the timeline and the result
file.

On the canvas the node shows everything itself (no separate window): the Earth map of where its grid
frequency is used (green), the hum frequency over time under it, the **grid data** file chosen right
in the node (empty: the bundled file), and a status column (state, grid frequency, hum runs, audio,
area).

Files: `mod.rs` (the detector: name, label, ports), `node.rs` (the node in the workbench: state while
running, the area its grid frequency allows, its look on the canvas, timeline marks, result file), `plugin.rs` (the same
detector as a plugin for the old viewer, `enf-plugin.exe`). The default grid map is in its own module:
[grid_frequencies](../grid_frequencies/README.md).

## Detecting the hum

A 16 s window every second, a 262 144-point FFT (Fast Fourier Transform) with 0.015 Hz bins, the
best of harmonics 1–4 of 50 and 60 Hz, and a vote for the grid's nominal frequency. Readings that
jump more than 30 mHz from the recent median are flagged as outliers, kept but not used. Runs of
clear hum of 30 s or more are reported. On the reference recording it identifies a 60 Hz grid (US)
and tracks the drift within about ±40 mHz.

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
Output: what it would give the project.

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
