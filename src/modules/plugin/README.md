# Plugin (detector)

Runs a separate program that speaks the plugin protocol
([docs/plugins.md](../../../docs/plugins.md)). It is fed the shared audio through a pipe and reports
findings and facts. Inputs: `audio`, optional `place` and `time`. Outputs: `findings` (events) and
`facts` (JSON). The bundled plugins are Metadata (`meta`) and ADS-B check (`adsb-check`); their
labels come from the program's file name.
