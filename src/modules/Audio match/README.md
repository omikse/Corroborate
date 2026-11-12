# Audio match (detector)

The time module. Each row takes one recording's audio, the time its metadata claims, and
optionally when it ends (`audio N`, `time N` and `end in N`, drawn as audio, audio start time and
audio end time (optional); they grow as they are wired, and dropping a SOURCER's audio on a row also
draws its time wire). Each row gives a better audio start time and audio end time (`synced N` and
`end N`).

A row's audio may be a piece of a cut recording (a Cut detector's `audio n`): it is a recording of
its own, from its piece of the file, on its own clock, named `<file> · piece n`. Its phone id starts
as `<file> (cut)`, the same for every piece of the file (they come from one device, so they share
one drift); typing another replaces it. A node fed pieces waits for its Cut detectors to read their
recordings, and runs again whenever their pieces change (a cut switched, a threshold dragged).

It finds where recordings of one event hear the same sound, and links their clocks: moment `t_a` on
one recording is moment `t_b` on another; every pair is matched. Recordings linked by audio are a
synced group, drawn with a coloured border and its number; only this node shows them, and audio
alone decides what a group is.

**Phones.** Each row's name strip has a phone id field, beside the file name. Rows with the same
text are one phone, in one colour: their files keep their own starts (each began when it was
pressed) but share one drift, one crystal, and count as one vote in the group's averages. An empty
field is its own phone. When the file's metadata names its device (make, model, serial), that shows
greyed in the empty field as a suggestion only; Enter in the empty field takes it. A change is
saved with the project, logged and undone. On a row too thin for the field it shrinks to its
colour chip, and a click edits it. The Order menu's Group › phone › start puts a phone's files
together.

**The fit.** Each group is fitted from this node's own links only (`clock::fit_against`, the clock
solve without its breakdown): every recording's offset and drift, with errors, and how far each
link sits from the fit (its leftover). Every phone's drift gets the same prior (±100 ppm against
real time), so the fit puts the group's drifts around its average clock: one vote per phone, each
weighted by how well the links measure it, none dropped. A 40 s clip of a phone takes the drift
its phone's longer files measured. The row's hover and its outputs' source lines give the fit:
links, leftover, drift. The foot line gives the median leftover.

**Drift** is how fast a recording's clock runs, in seconds an hour: −0.9 s/h is a clock losing
0.9 s an hour (−250 ppm, parts per million). It is measured against, in order of precedence:
- real time, when the anchor row's end is wired: (end − start) ÷ length gives the anchor's real
  speed, and every row's drift follows from it;
- a speed reference: the clock button on a row's audio says that file's clock runs true (an
  official concert recording from a professional camera, say), one per group; its Drift cell
  reads "runs true";
- the group's average clock, otherwise.

**Placing a group.** After a run a group sits where its stamps put it, marked ?: each stamp votes
for where the group is, one vote per phone (its stamps' mean, weighted by their ±), the phones'
votes weighted by their ±. Every stamp counts, one 12 years off too: wrong metadata shows plainly
on the timeline, and fixing it is a person's job (a Time node). A person may drag the group
(saved, logged and undone with the project); each time then keeps its own metadata error. Two
ways to anchor it:
- **on a row's time:** a click on the anchor of a row with a time: the audio's start snaps to that
  time, and every other row is placed along the fit from it, its error the anchor's and the fit's
  added as squares;
- **on the average:** the anchor button on the group's number tab puts the group back at the
  average of its stamps (undoing a drag) and anchors it there. Its error is the larger of the
  votes' weighted spread ÷ √phones and what their ± give (one phone alone has no spread). The
  source lines say "anchored on the average of N phones' stamps".

The two exclude each other; a drag stops while anchored. A row with no time metadata that audio
linked to nothing has nothing to place it by: it is greyed out and gives nothing.

**The end.** End = start + length at the measured drift. A clock losing 0.9 s/h over a 40-minute
file ends 0.6 s after start + length. The end's error is the start's and the drift's error × the
length, added as squares. The optional end input:
- on the anchor row it gives real time (above);
- on another row it is compared with the measured end (in the Drift cell and the hover);
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
- **Drift:** the row's drift in s/h, split at zero (slower on the left), green near 0 and red
  toward 3.6 s/h on a log scale (0.1 s/h, a phone's clock, fills half). The hover gives the exact
  scale: the clock's speed (×0.9997500 file seconds per real second) and what a file length is
  multiplied by to give real seconds. A wired end's distance from the measured end shows in the
  empty half.
- **output:** the audio start time over the audio end time, with an anchor or ? in the corner.

On the timeline the amber bar runs from the start to the measured end. A dashed line marks where
start + length would end; the bit past it is hatched, a bar ending short of it is cut, at least
4 px either way, with the size beside it ("end +47 ms"). The speed reference has none.

Outputs:
- `synced N`, `end N`: the row's audio start time and audio end time. These are the only way the
  node's times reach other nodes: there is no global clock solve any more (2026-11-11), so a
  recording is placed elsewhere only by wiring its times there (a Timeline, say).
- Possible (drawn greyed): shared sounds, sound source.

Retired (2026-11-12): the possible `edits` output. Pieces of a cut recording come from the Cut
detector into this node, so this node's findings cannot go back into the Cut detector (a cycle);
the Cut detector compares recordings itself with the same library calls (its `compare with`
inputs).

Gone (2026-11-11): quality, clock drift, same recording, speed change, gaps and links. The marks
show the findings; the result file keeps the quality (level, verdict, counts per pair), each pair's
own clock drift (for pairs whose links span 30 s or more; on Jiku within 24 ppm of the truth,
median), the events, and every link: one per matched chunk, both clocks named, each with its error
(1 sigma) from how clearly the audio matched (4 ms for a clear match, more for a weak one) and
where it came from (the two recordings, the cross-correlation's PSR, the fingerprint votes). A
saved wire from one is dropped when the project opens, with a message naming it. A project saved
before rows had a time each (before 2026-11-10) is no longer converted: its wire into the old
single time input is dropped, with a message.

How: Wang landmark fingerprints (audiofp) of each recording at 8 kHz; every 15 s of one
recording is matched against the whole other; each clear match is refined with GCC-PHAT
(generalized cross-correlation with phase transform) over 8 s. The method, its numbers and why,
and the findings on the Jiku Mobile Video Dataset (50 phones at one event: 95 % of the ground
truth's spanning-tree edges found, none wrong, moments within 16 ms for 95 %) are in the library
module `src/audio_match.rs`; the real-data test is `tests/jiku.rs` (`tools/jiku.py` fetches it).

Limits now: files only (a stream or microphone wired in is named in the status and left out);
each file is decoded again here, and all are matched once their decoding is done, not while
the sources play.

The result file gives per row its phone id, its anchor (its time, or the group's average), whether
it is the speed reference, and its drift (s/h, its error, the clock's speed, and what it is against).

Files: `module.toml` (ports and use cases), `mod.rs` (its ports), `node.rs` (runs the match, the
fit, the rows on the canvas, the outputs and the result file).
