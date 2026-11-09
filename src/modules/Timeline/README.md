# Timeline (view)

Receives and displays; it never works out when anything happened. The time it is given is the truth
it draws. Add it by hand, like the Map.

Each row takes a recording's standard audio or video output and a time. The recording is a bar at
that time, as long as its media, with its name, and margins left and right from the error the time
carries. A row with no time is greyed and has no bar.

Row height is adjustable, Sort by time puts the rows in order (the wires may cross), the wheel zooms
and a drag pans. There is one real-time playhead, in UTC (Coordinated Universal Time), shared by all
Timeline nodes and followed by anything with a real time. A SOURCER with no real time keeps its own
playhead, linked to nothing. A click on the ruler moves the playhead; Space or Play runs it.

Not yet: timed events, sound.

Files: `module.toml`, `mod.rs` (its ports), `node.rs` (the rows on the canvas).
