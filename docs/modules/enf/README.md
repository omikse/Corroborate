# ENF Module

Electrical Network Frequency (ENF) detection for audio geolocation.

## What it does

Extracts the hum frequency (50 or 60 Hz fundamental + harmonics) from audio and maps it to geographic regions where that grid frequency is standard. Outputs a location constraint reflecting the detected frequency's likely region(s).

## Inputs

- **audio** (required): Audio stream, any sample rate
- **location** (optional): Location constraint to validate against detected frequency
  - If provided and frequency contradicts it, outputs exclusion (whole world masked)

## Outputs

- **location**: LocationConstraint
  - Detected frequency → feasible regions (50/60 Hz zones)
  - Or Exclude if constrained input contradicts the detected frequency
- **segments**: ENF findings (time, frequency, harmonic, confidence)

## Data dependency

Requires grid frequency data (grid-frequencies node):
- Maps frequencies to geographic regions
- Swappable — can use different datasets
- Travels with the module

## Transparency

- Contradictions are surfaced, not hidden
- If detected frequency doesn't match provided location, output masks the whole world
- This signals: something needs investigation

## See also

- [DESIGN.md](DESIGN.md) — algorithm and implementation details
- [grid-frequencies](../grid-frequencies/) — data format
