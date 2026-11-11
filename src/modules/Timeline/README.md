# Timeline (view)

Receives and displays; it never works out when anything happened. The time it is given is the truth
it draws. Add it by hand, like the Map.

Each row takes a recording's standard audio or video output, the time it starts and, if known, the
time it ends (an Audio match row's audio start time and audio end time, say). The recording is a bar
from its start to its end (to start + its media's length when no end is wired), with its name, and
margins from each time's error: the start's on the left, the end's on the right. A row with no time
is greyed and has no bar.

Row height is adjustable, Sort by time puts the rows in order (the wires may cross), the wheel zooms
and a drag pans. There is one real-time playhead, in UTC (Coordinated Universal Time), shared by all
Timeline nodes. A recording follows it only while its start time is wired into a Timeline: its
moment t is at start + t, or with the end wired too, t = (playhead − start) × length ÷ (end − start).
Every other recording (a SOURCER with nothing wired here) keeps its own playhead, linked to nothing.
When a recording starts or stops following, its playhead moves over at the same moment. A click on
the ruler moves the real-time playhead, and with it every recording wired in, and only those; Space
or Play runs it.

Not yet: timed events, sound.

Files: `module.toml`, `mod.rs` (its ports), `node.rs` (the rows on the canvas).
