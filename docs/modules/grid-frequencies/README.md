# Grid Frequencies Data Node

Provides electrical grid frequency geographic mapping for ENF (Electrical Network Frequency) detection.

## Format

GeoJSON-like structure mapping regions to their grid frequency standard:

```json
{
  "version": "1.0",
  "source": "IEC 50001, regional standards",
  "generated": "2026-11-05",
  "regions": [
    {
      "id": "Europe",
      "frequency_hz": 50.0,
      "coverage": "continental",
      "bounds": [
        [51.5, -0.1],
        [55.0, 20.0],
        ...
      ]
    },
    {
      "id": "USA",
      "frequency_hz": 60.0,
      "coverage": "continental",
      "bounds": [...]
    },
    {
      "id": "Japan",
      "frequency_hz": 50.0,
      "coverage": "western_region",
      "bounds": [...]
    },
    {
      "id": "Japan-East",
      "frequency_hz": 60.0,
      "coverage": "eastern_region",
      "bounds": [...]
    }
  ]
}
```

## Fields

- **id**: Region identifier (unique)
- **frequency_hz**: 50.0 or 60.0 (standard for this region)
- **coverage**: "continental", "regional", "country", etc. (metadata)
- **bounds**: Ordered lat/lon points defining the region polygon (closed loop)

## Swappability

This node can be replaced with alternative datasets:
- Historical versions (grids changed over time)
- High-resolution (country or sub-region level)
- Custom (research-specific, corrected, etc.)
- Versioned (date-stamped)

The ENF node connects to whichever grid-frequencies node feeds it.

## Data integrity

- Bounds should be valid polygons (no self-intersecting, closed)
- Check for overlaps (expected in transition zones, document them)
- Include source and date for traceability

## See also

- [ENF Module](../enf/) — uses this data for geolocation
