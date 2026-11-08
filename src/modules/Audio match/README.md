# Audio match (detector)

Finds where recordings of one event hear the same sound, and links their clocks: moment `t_a` on
one recording is moment `t_b` on another. Wire every recording's audio in (a new input appears
each time one is wired: `audio 1`, `audio 2`, …); every pair is matched. The links go into the
clock solve like any link in the project (wired or not), which works out each recording's offset
and drift; the timeline then shows the recordings in one group, side by side.

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
- Possible (drawn greyed): edits, shared sounds, sound source, gaps.

How: Wang landmark fingerprints (audiofp) of each recording at 8 kHz; every 15 s of one
recording is matched against the whole other; each clear match is refined with GCC-PHAT
(generalized cross-correlation with phase transform) over 8 s. The method, its numbers and why,
and the findings on the Jiku Mobile Video Dataset (50 phones at one event: 95 % of the ground
truth's spanning-tree edges found, none wrong, moments within 16 ms for 95 %) are in the library
module `src/audio_match.rs`; the real-data test is `tests/jiku.rs` (`tools/jiku.py` fetches it).

Limits now: files only (a stream or microphone wired in is named in the status and left out);
each file is decoded again here, and all are matched once their decoding is done, not while
the sources play.

Files: `module.toml` (ports and use cases), `mod.rs` (its ports), `node.rs` (runs the match,
fills the outputs and the result file).
