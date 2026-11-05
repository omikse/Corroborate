# ENF Module Design

## Algorithm

### 1. Detect hum frequency

- Input: audio stream (any sample rate)
- Compute spectrogram (narrow-band for precision)
- Look for peaks at 50 Hz and 60 Hz and their harmonics (100, 120, 150, 180, etc.)
- Track frequency over time (grid frequency can drift slightly, ±0.5 Hz)
- Output: detected frequency + confidence

### 2. Identify harmonics and quality

- Measure harmonic series (fundamental + overtones)
- Narrow, stable peaks = high confidence
- Broad or noisy = low confidence
- Absence of harmonics = no grid hum detected

### 3. Map frequency to regions

- 50 Hz → Europe, Africa, Asia (most), Australia
- 60 Hz → Americas, Japan, parts of Asia
- Boundaries from grid-frequencies data (swappable)
- Handle regional exceptions (some countries use both, or have changed)

### 4. Validate against location constraint (if provided)

```
if location_input:
  detected_freq → regions
  if detected_freq region ∩ location_constraint == ∅:
    output: Exclude(whole world)  # contradiction surfaced
  else:
    output: Polygon(intersection)
else:
  output: Polygon(detected_freq regions)
```

## Output

**Segments:**
```rust
pub struct EnfSegment {
    pub t_start: f64,
    pub t_end: f64,
    pub mean_hz: f64,           // detected frequency
    pub min_hz: f64,
    pub max_hz: f64,
    pub harmonic: u8,           // which harmonic (1=50/60, 2=100/120, etc.)
    pub confidence: f64,        // 0–1, based on peak strength & stability
}
```

**Location constraint:**
```rust
LocationConstraint {
    geometry: Polygon { points: [...] },  // feasible regions
    method: "ENF 50/60 Hz grid".into(),
    confidence: mean_hz_stability,
}
```

## Data input

Grid frequencies node provides:
```json
{
  "regions": [
    {
      "name": "Europe",
      "frequency_hz": 50,
      "bounds": [[lat, lon], ...]
    },
    ...
  ]
}
```

## Notes

- Does not require location input (can work alone)
- Transparency: contradictions masked as exclusions, not errors
- Frequency can drift; use confidence to weight the output
- Some regions have mixed standards or historical changes (data-driven)
