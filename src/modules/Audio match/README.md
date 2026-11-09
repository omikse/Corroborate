# Audio match (detector)

The time module. Each row takes one recording's audio and the time its metadata claims (`audio N`
and `time N`, growing as they are wired; dropping a SOURCER's audio on a row also draws its time
wire) and gives a better time on its yellow output, `synced N`.

It finds where recordings of one event hear the same sound, and links their clocks: moment `t_a` on
one recording is moment `t_b` on another; every pair is matched. The links go into the clock solve
like any link in the project (wired or not), which works out each recording's offset and drift.

Recordings linked by audio are a synced group, drawn with a coloured border and its number; only
this node shows them, and audio alone decides what a group is. Until a group has an anchor its amber
bars are only shown, with the middle of the audio on the middle of its times, and can be dragged;
that changes no data and nothing comes out. A click on the anchor of any row with a time anchors the
group there: the audio's start snaps to that time, every other row keeps its real offset, and every
row's yellow output gives its time, with an error from the anchor and the links. A time 200 years
off is never left out: it stays in view until a person corrects it with a Time node.

The status column says: before a run the row's time or `no time metadata`, during it the progress,
after it how many seconds off the time is from the audio (green at 0 s, red at 30 s, going black
into years), with a mirror icon when that may be a stamp marking the end.

Outputs:
- `links`: one link per matched chunk, both clocks named, each with its error (1 sigma) from how
  clearly the audio matched (4 ms for a clear match, more for a weak one) and where it came from
  (the two recordings, the cross-correlation's PSR, the fingerprint votes).
- `quality`: level (good: all recordings in one group; weak: several groups; none: no links),
  the verdict, and the counts it rests on, per pair too: chunks tried, fingerprint candidates,
  candidates the cross-correlation did not confirm, links.
- `clock drift`: for each pair whose links span 30 s or more, how fast one clock runs against
  the other (ppm, with its error), from that pair's links alone (the solve uses all of them).
  On Jiku within 24 ppm of the truth (median), 300 ppm (95 %): short overlaps and moving phones.
- `same recording`: two files that are one recording (a copy, a re-upload, a re-encode), so not
  two witnesses: an event on each over the stretch they share, when their lined-up audio
  correlates over 0.8 (Jiku: two phones 0.18 median, 0.64 at most; a video against its own
  audio 1.000).
- `speed change`: a pair drifting over 1000 ppm beyond 3 sigma (quartz stays within about 300):
  one was sped up or slowed down. Only up to about 1 %, where the fingerprints still match.
- `gaps`: where a pair's offset jumps 0.1 s or more between two links in a row, an event on the
  recording that lacks the stretch (paused or cut there, or the other has an insert), between
  its two links. A pair with a gap gets no drift. None on Jiku (unedited phone recordings).
- Possible (drawn greyed): edits (for the cut detector), shared sounds, sound source.

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
rows on the canvas, the outputs and the result file).
