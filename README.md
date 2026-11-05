# osint-geo-audio

**Corroborate**: live audio-visual forensics for open-source investigations. A node-based workbench for audio analysis, geolocation via Doppler curves and electrical hum, and multi-source synchronization.

## Quick start

```bash
cd doppler-live
cargo build --release           # detector library
cargo build --release --bin corroborate  # workbench GUI
```

Run the workbench:
```bash
./target/release/corroborate.exe
```

Or with a saved project:
```bash
./target/release/corroborate.exe --run
```

## The workbench (Corroborate)

A node graph for audio forensics:

- **Add nodes** from the canvas menu: sources (file, stream, microphone), detectors (Planes, ENF), data (Manual facts, Grid frequencies), plugins, and a record node for results.
- **Wire them**: drag from an output port to an input port (types must match). Audio flows from sources; detectors analyze it and output findings; the record node collects everything.
- **Double-click a node** to open its window: see detected passes, edit coordinates, review the spectrogram, adjust analysis parameters.
- **Timeline view** (bottom) shows every finding on one clock with its time error. All findings from anchored sources (with UTC) appear here.
- **Result JSON** saves every 30 s to your Desktop\record\corroborate\, with all findings, facts, and the graph as evidence.

## Modules

Each module has its own window, data requirements, and output. Read its docs in `docs/modules/`.

### Planes (Doppler)

Detects aircraft by their Doppler-shifted engine noise.

- **Input**: audio stream (any source), optional time anchor
- **Output**: detected passes (speed, closest approach, frequency, grade, distance estimate)
- **Window**: spectrogram with detected passes and plugin prediction curves overlaid. Spawns `doppler-view` as a separate visualization window.

**How it works** (summary):
1. 8192-point STFT (spectrogram) of the audio
2. Find ridges (frequency tracks) using Viterbi path
3. Fit straight-pass Doppler model to bright runs
4. Apply gates (SNR, speed, distance, observability)
5. Merge overlapping candidates and report

See [docs/modules/planes/](docs/modules/planes/) for details.

### Power hum (ENF)

Detects electrical network frequency (50 or 60 Hz) and maps it to geographic regions.

- **Input**: audio stream (required), grid frequency data (required), optional location to validate
- **Output**: detected hum segments, location constraint (feasible regions OR exclusion if contradicted)

**How it works**:
1. Extract 50/60 Hz hum and harmonics from audio
2. Map detected frequency to grid regions (50 Hz = Europe, Africa, Asia; 60 Hz = Americas, Japan, etc.)
3. If location constraint provided, validate: does detected frequency match?
   - Yes → output the matching region
   - No → output exclusion (whole world masked) — signals a contradiction
4. Transparency: contradictions are not hidden; they're data.

See [docs/modules/enf/](docs/modules/enf/) for details.

### Grid Frequencies (data node)

Provides the geographic map of grid frequencies (50/60 Hz zones).

- **Output**: grid frequency data (JSON)
- **Swappable**: use different datasets (regional, historical, custom)
- **Portable**: data travels with the module, no external files needed

Default grid covers major regions. See [docs/modules/grid-frequencies/](docs/modules/grid-frequencies/) for format.

### Manual data

User-entered facts: date, time, coordinates, elevation, notes, custom fields.

- **Output**: place (location), time reference, facts (JSON)
- **Used by**: other modules (geolocation validation), the record (context)

### ADS-B Check (plugin)

Tests Planes detections against Automatic Dependent Surveillance–Broadcast (ADS-B) flight data.

- **Input**: audio, location (from Manual), time
- **Output**: expected vs. heard passes, distance range of the detector
- **Data**: queries adsb.lol every 10 s for live sources; or reads adsb*.jsonl files next to recordings

Requires MANUAL DATA with coordinates and elevation.

### Result record

Collects findings and facts from all sources, writes JSON and metadata.

- **Inputs**: up to 6 event streams, up to 3 fact streams
- **Output**: one JSON file on disk, updated every 30 s
- **Content**: all findings (time, frequency, location, confidence), facts, the graph topology, and the evidence trail

## Synchronization and geolocation

Multiple sources (recordings, live streams) with no initial time anchor can be synchronized by shared events:

1. **Start global**: Manual node with a worldwide location (huge radius).
2. **Add detectors**: Planes on each source, ENF on each audio.
3. **Find a shared event**: a plane flies past multiple sources, or a recognizable sound (explosion, siren) is heard on both.
4. **Sync solver**: takes the shared event, solves for clock offsets, outputs UTC for all sources.
5. **Location constraints stack**: Doppler circles, ENF regions, manual bounds all feed to the Record node. The feasible region shrinks as evidence accumulates.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the sync algorithm.

## Files

```
doppler-live/
├── src/
│   ├── lib.rs              # core types (Value, Location, TimeRef, PortKind)
│   ├── core.rs             # same, expanded
│   ├── graph.rs            # graph: nodes, wires, cycles, ordering
│   ├── timeline.rs         # timeline: findings on one clock, anchors
│   ├── sync.rs             # sync solver: clock offsets from shared events
│   ├── locate.rs           # TDOA locator: time-difference-of-arrival grid search
│   ├── modules/
│   │   ├── planes.rs       # aircraft Doppler detector
│   │   └── enf.rs          # hum detector and frequency-to-region mapping
│   ├── bin/
│   │   ├── doppler-live    # CLI detector (headless)
│   │   ├── doppler-view    # standalone visualization (old viewer)
│   │   └── corroborate/    # workbench GUI (main entry point)
│   │       ├── main.rs     # GUI, graph canvas, timeline
│   │       ├── nodes.rs    # node kinds and ports
│   │       ├── runtime.rs  # run the graph: spawn detectors, plugins
│   │       └── demo.rs     # synthetic protest scenario
│   └── tests/
│       └── protest.rs      # Doppler + ENF sync: ground truth test
├── data/
│   └── grid-frequencies.json  # 50/60 Hz regions (swappable)
├── docs/
│   ├── ARCHITECTURE.md     # overall design
│   ├── VISION.md           # project goals
│   └── modules/
│       ├── planes/
│       ├── enf/
│       └── grid-frequencies/
└── target/release/
    ├── corroborate.exe     # the workbench
    ├── doppler-view.exe    # visualization (opened by Planes node)
    ├── doppler-live.exe    # CLI detector
    ├── enf-plugin.exe      # ENF as plugin (legacy)
    ├── meta-plugin.exe     # metadata extraction
    └── adsb-check.exe      # flight data check
```

## Building

```bash
# Library + CLI detector (lean, ~10 MB)
cargo build --release

# Add GUI workbench and old viewer
cargo build --release --bin corroborate --bin doppler-view

# Tests
cargo test --release
```

## Philosophy

- **Raw and transparent**: every number carries its source and uncertainty. Contradictions are surfaced, not hidden.
- **Efficient**: detection runs fast on live streams. No guessing—sync requires shared events, not assumptions.
- **Modular**: each detector is independent. Swap grid data, add plugins, extend without rebuilding.
- **Portable**: modules are self-contained. Grid data travels with them. No external dependencies beyond ffmpeg.

## Example: overnight test

Run for 12 hours, comparing live aircraft with ADS-B data:

1. Create a Manual node with coordinates and elevation.
2. Add a Stream node pointing to a live camera (YouTube, Twitch, etc.).
3. Add Planes detector, GridFrequencies, and ADS-B Check plugin.
4. Wire them: Stream audio → Planes, Planes findings → Record. Manual coordinates → ADS-B Check.
5. Press Run. Results saved to Desktop\record\corroborate\result.json, updated every 30 s.
6. Open `doppler-view` (spawned automatically) for detailed spectrogram inspection.
7. At dawn, check: how many planes were heard vs. expected? What's the detector's real range?

## License

[See LICENSE](LICENSE)

## References

- Hermes, Dobler et al. (aircraft Doppler geolocation)
- DopplerTrack, colsto (whitened spectrogram, ridge detection)
- IEC 50001 (grid frequencies)
- ADS-B: Automatic Dependent Surveillance–Broadcast
- TDOA: Time Difference of Arrival (multilateration)
