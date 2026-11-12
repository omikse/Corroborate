# Cut detector (detector)

Where a recording is cut, and into which pieces. Each piece then comes out as a recording of its
own, with its own audio, video, time and length, so the nodes that place recordings in time can
place every piece the same way. Plan, research and sources: [docs/cut-detector/plan.md](../../../docs/cut-detector/plan.md);
the mockup it follows: [docs/cut-detector/mockup.html](../../../docs/cut-detector/mockup.html).

It decides nothing for good. Every cue is drawn with its strength, every lane has a threshold you
can drag, and any cut can be switched on or off, or put in by hand.

## Ports

Inputs:
- `audio` and `video` (either or both, from one source): the recording to split. The node reads
  the file itself (one extra ffmpeg pass over every frame). Files only for now: a stream or a
  microphone is named in the status and not read.
- `time` (optional): when the recording starts, usually the SOURCER's time.
- `compare with n` and `events n` grow as rows: other recordings of the same event, and moments
  from any node that confirm cuts (see Lanes).

Outputs:
- `cuts` (event): the cuts that are on, one event each: `t_start` at the picture's cut, its
  `error_s` (half a frame for a picture cue), a label (`cut 2`, or `cut 3 (by hand)`), and in
  `detail` the cues behind it, the sound's time when it differs (J-cut or L-cut) and the events
  that confirm it.
- One row per piece: `audio n` and `video n` (streams), `time n` and `length n`. A piece's video
  runs from its picture cut to the next; its audio from its sound cut to the next.
  `time n` is the piece's start as played: the wired time plus where the piece starts in the file,
  with the file's error, and a source that says so ("cuts before it may hide time"). The first
  piece is on the file's own clock (its moments are the file's); every later piece is a clock of
  its own (`<file clock>~n`).

How many pieces there are is saved with the project (row `pieces`), so wires from piece outputs
stay while a project opens, before anything has been read. When cuts are switched off, the
outputs of pieces that are gone go with their wires.

## Lanes

Each lane is a row under the filmstrip and the waveform, on the recording's own time:
- **picture**: ffmpeg's `scdet` filter (scene change score for every frame: the mean absolute
  difference to the frame before, in percent; ffmpeg's default threshold 10), on frames brought to
  320 pixels wide first. A cut must last (the idea of PERSIST, Lin et al. 2026): the frame before it
  is compared with the frame a second after it, and the frame a second before with the frame after.
  Where the picture comes back (a flash, something passing in front of the camera) the cut starts
  off and says "a flash: the picture comes back": its lasting score is under half the threshold, or
  under a third of its own score. Frozen and black stretches (`freezedetect`, `blackdetect`) are
  counted in the lane's note.
- **keyframes and gaps** (ffprobe's packets): packet times that jump (a pause, pieces joined
  without re-encoding), and keyframes off the file's regular rhythm (a piece cut without
  re-encoding starts on a keyframe; Vázquez-Padín et al. 2012 for the idea). A file with no regular
  rhythm (an encoder that puts keyframes at scene changes) gives no keyframe cues, and says so.
- **events n**: one lane per wired events input, named after what feeds it. Each event is a single
  moment with its error (one frame when it has none). An event confirms a cut a lane found when it
  falls within its error (a blue dot under the cut, "+ events" in the table); on its own it makes no
  cut. Events on another clock than the recording's are left out and counted.

Cues of different lanes within 1 s of each other are one cut. The picture lanes set the cut's
picture time; sound lanes (stage 2) its sound time.

## On the canvas

- The top line: how many cuts are on, how many pieces, and what the run is doing.
- The filmstrip (a picture a second), the waveform (the source's), then the lanes: a lane's curve,
  its marks (strong when over the threshold) and its threshold as a dashed line. Drag the
  triangle at the right end of a lane up or down to change its threshold; the change is saved
  with the project and goes into the result file's setting changes.
- Cuts are red lines down the rows, at the picture time over the picture rows and the sound time
  over the sound rows; a cut that is off is a grey dashed line. Hover a cut to read the cues
  behind it. Click it to switch it; click anywhere else on the rows to cut there by hand (on the
  nearest frame, marked "hand"). Every switch goes into the result file.
- The pieces table: where each piece's picture and sound sit in the file, its length, the cues
  behind the cut before it, and its time out.

Rows saved with the project: `pieces`, `thr <lane>` (a threshold you set), `cut <seconds>` (`on`,
`off` or `hand`). A switch belongs to the cut whose picture or sound time is within 0.25 s.

## Result file

The node's lanes (marks, threshold, note), every cut whether on or off (picture and sound time,
error, whether the lanes put it on, the switch, the cues in words) and the pieces.

## Tests

- `cargo test --profile fast --lib cuts`: metadata lines, keyframe rhythm and packet jumps,
  cutting cues into cuts with picture and sound times, flashes, switches, events, pieces.
- `cargo test --profile fast --test cuts`: a clip of four test pictures cut at 3, 6 and 9 s with a
  two-frame white flash at 4.48 s: the three cuts are found on their frames (scores 18, 22, 30;
  lasting 21, 24, 23), the flash (score 38, lasting 5.7) starts off. A phone-like file (a keyframe
  every second) cut and joined without re-encoding: the join is the one keyframe off the rhythm.
- `cargo test --profile fast --bin corroborate cut_tests`: the node headless on that clip: four
  pieces, their times and lengths, a cut switched off, a hand cut, saved and opened again with its
  outputs and wires.

PySceneDetect as an oracle on the same clips (plan, stage 1) is not run yet: it needs a Python
package installed.

## Status

Stage 1 of the plan (picture and timestamps, hand cuts, pieces as rows) is built. Sound cues,
`compare with` and Audio match taking pieces follow.

Files: `module.toml` (ports), `mod.rs` (ports from the rows, the rows' keys), `node.rs` (the run,
the outputs, the canvas), the method in `src/cuts.rs`.
