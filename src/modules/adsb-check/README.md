# ADS-B check (plugin)

A test plugin: it checks the planes detector against ADS-B (Automatic Dependent
Surveillance–Broadcast) flight data. A separate program speaking the plugin protocol
([docs/plugins.md](../../../../docs/plugins.md)); `main.rs` is all of it.

ADS-B (Automatic Dependent Surveillance–Broadcast) data sets the expectations, and the planes detector
confirms them. Every aircraft whose closest approach to the site is within 15 km (the detector's own distance
limit) is an expected pass, due to be heard at its closest approach plus the sound's travel time. Each one is
then:
- **✓ heard**: a detected pass within ±20 s. The Doppler curve that the aircraft's real track predicts (from the
  heard pass's rest frequency) is drawn dashed over the spectrogram: if it lies on the heard curve, it is the
  same aircraft. Its distance and speed are listed next to the Doppler fit's.
- **· not heard**.
- **– beyond detector**: flying outside the detector's 25–150 m/s (airliners at cruise). Listed, not counted.

Passes heard with no aircraft in the data are listed too (planes without ADS-B, or false detections). The
status sums it up as heard/expected by distance: the detector's real range at that site.

To try it: a SOURCER on a live stream from a known site (for example the Cornell Lab FeederWatch
stream, 42.4797, -76.4514, 335 m), wired into Planes and this plugin, with a Manual data node for the site.

It needs:
- the site: Manual data `coordinates` (lat, lon) and an `elevation` field in metres (add it with **+ field**);
  then switch the plugin off and on so it starts with them;
- the UTC of the audio: recordings made with REC carry it; live sources use this PC's clock;
- flight data: for files, every `adsb*.jsonl` next to the file; live, it polls adsb.lol every 10 s itself, waiting longer when the API asks it to.
  `tools/adsb-log.py` logs the aircraft around a point while you record:
  ```bash
  python tools/adsb-log.py --lat 42.4797 --lon -76.4514 --radius-nm 25 --minutes 80 --out <recording folder>/adsb-around-site.jsonl
  ```
  Free history: adsb.lol publishes a full daily archive (the next day); OpenSky Network has history for
  approved research accounts. Small planes without ADS-B Out, or below receiver coverage, are not in any of it.
