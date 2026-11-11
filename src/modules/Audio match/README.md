# Audio match (detector)

The time module. Each row takes one recording's audio, the time its metadata claims, and
optionally when it ends (`audio N`, `time N` and `end in N`, drawn as audio, audio start time and
audio end time (optional); they grow as they are wired, and dropping a SOURCER's audio on a row also
draws its time wire). Each row gives a better audio start time and audio end time (`synced N` and
`end N`).

It finds where recordings of one event hear the same sound, and links their clocks: moment `t_a` on
one recording is moment `t_b` on another; every pair is matched. Recordings linked by audio are a
synced group, drawn with a coloured border and its number; only this node shows them, and audio
alone decides what a group is.

**The fit.** Each group is fitted from this node's own links only (`clock::fit`, the clock solve
without its breakdown): every recording's offset and speed against the group's reference
recording, with errors, and how far each link sits from the fit (its leftover). The reference is
the anchor's recording, else the group's longest; its clock counts as running true. The row's hover
and its outputs' source lines give the fit: links, leftover, ppm. The foot line gives the median
leftover.

**Placing a group.** A click on the anchor of a row with a time anchors its group there: the
audio's start snaps to that time, and every other row is placed along the fit from it, its error
the anchor's and the fit's added as squares. Until then the group is placed on the average of its
metadata (or where a person dragged it: saved, logged and undone with the project), each time with
its own metadata error, and marked ?. A time 200 years off is never left out: it stays in view until
a person corrects it with a Time node. A row with no time metadata that audio linked to nothing has
nothing to place it by: it is greyed out and gives nothing.

**The end.** End = start + length at the measured speed. A clock 250 ppm slow over a 40-minute file
ends 0.6 s after start + length. The end's error is the start's and the speed's error × the length,
added as squares. The optional end input:
- on the anchor row it gives the anchor's real speed, (end − start) ÷ length, so every row's speed
  becomes real, not relative to the anchor; the anchor's Stretch cell says "speed from its wired
  end";
- on another row it is compared with the measured end (in the Stretch cell and the hover);
- on a row audio did not link it passes straight through.

The columns:
- **Status:** before a run the row's time or `no time metadata`, during it the progress, after it
  how far the time is from the audio, split at zero (earlier fills the left half, later the right;
  green at 0 s, red at 30 s, going black into years), with a mirror icon when that may be a stamp
  marking the end. Marks for what the match found, the words on hover:
  - same recording (two squares): two files that are one recording (a copy, a re-upload, a
    re-encode), so not two witnesses, when their lined-up audio correlates over 0.8 (Jiku: two
    phones 0.18 median, 0.64 at most; a video against its own audio 1.000);
  - speed change (two chevrons): a pair drifting over 1000 ppm beyond 3 sigma (quartz stays within
    about 300): one was sped up or slowed down. Only up to about 1 %, where the fingerprints still
    match;
  - gap (a broken line): where a pair's offset jumps 0.1 s or more between two links in a row, on
    the recording that lacks the stretch (paused or cut there, or the other has an insert). None on
    Jiku (unedited phone recordings). The fit assumes one steady speed per recording, which a cut
    breaks, until the cut detector splits such files.
- **Stretch:** the row's speed against its group's reference, in ppm, split at zero (slower on the
  left), green near 0 and red toward 1000 ppm on a log scale (30 ppm, a phone's clock, fills half).
  The reference row reads "reference".
- **output:** the audio start time over the audio end time, with an anchor or ? in the corner.

On the timeline the amber bar runs from the start to the measured end. A dashed line marks where
start + length would end; the bit past it is hatched, a bar ending short of it is cut, at least
4 px either way, with the size beside it ("end +47 ms"). The reference has none.

Outputs:
- `links`: one link per matched chunk, both clocks named, each with its error (1 sigma) from how
  clearly the audio matched (4 ms for a clear match, more for a weak one) and where it came from
  (the two recordings, the cross-correlation's PSR, the fingerprint votes). Until the global clock
  solve goes they go into it too.
- `synced N`, `end N`: the row's audio start time and audio end time.
- Possible (drawn greyed): edits (for the cut detector), shared sounds, sound source.

Gone (2026-11-11): quality, clock drift, same recording, speed change and gaps. The marks show them;
the result file keeps the quality (level, verdict, counts per pair), each pair's own clock drift
(for pairs whose links span 30 s or more; on Jiku within 24 ppm of the truth, median), and the
events. A saved wire into one is dropped when the project opens, with a message.

How: Wang landmark fingerprints (audiofp) of each recording at 8 kHz; every 15 s of one
recording is matched against the whole other; each clear match is refined with GCC-PHAT
(generalized cross-correlation with phase transform) over 8 s. The method, its numbers and why,
and the findings on the Jiku Mobile Video Dataset (50 phones at one event: 95 % of the ground
truth's spanning-tree edges found, none wrong, moments within 16 ms for 95 %) are in the library
module `src/audio_match.rs`; the real-data test is `tests/jiku.rs` (`tools/jiku.py` fetches it).

Limits now: files only (a stream or microphone wired in is named in the status and left out);
each file is decoded again here, and all are matched once their decoding is done, not while
the sources play.

Files: `module.toml` (ports and use cases), `mod.rs` (its ports), `node.rs` (runs the match, the
fit, the rows on the canvas, the outputs and the result file).
