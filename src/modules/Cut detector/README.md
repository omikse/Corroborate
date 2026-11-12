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
- `compare with n` and `events n` grow as rows: other recordings of the same event (files, each
  read like this one), and moments from any node that confirm cuts (see Lanes).

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
- **sound level** (when `audio` is wired): loudness (RMS, root mean square, in dB) every 50 ms;
  a mark where the mean of the next second differs from the second before. Default threshold
  15 dB: on a 16 min phone recording of a crowd (Jiku) the level stepped by more than 12 dB 13
  times, by more than 15 dB 5 times.
- **background noise** (when `audio` is wired): the noise level under the sound for each second,
  every quarter second (Pan, Zhang, Lyu, ICASSP 2012: band-pass channels of a natural sound have
  about the same kurtosis, and noise lowers each channel's kurtosis by its share of the channel's
  variance; a line through the 16 channels gives the noise variance). A mark where the median of
  the 2 s after a moment differs from the 2 s before. It is rough on real sound (on the same phone
  recording it stepped by more than 6 dB 198 times in 16 min, at most 23 dB), so its default
  threshold is 15 dB: a lane to read along with the others more than one that cuts alone. On a
  made sound with its noise 20 dB louder after 6 s it marks 6 s, and nothing else over 15 dB.
- **compare: <recording>**, one per recording wired into `compare with`: this recording matched
  against the other with the audio match's library (`audio_match::match_pair`, links every 15 s,
  run twice with the chunks half a chunk apart), and a mark where the lag between them jumps by
  0.1 s or more. The mark's height is the jump in seconds: forward, this recording lacks what the
  other heard (a cut here); backward, it has a stretch the other lacks (a stretch moved here, or
  the other was paused). The moment is found by GCC-PHAT windows (2 s every half second, then 1 s
  every 20 ms) voting for the old lag or the new one; windows too quiet to tell abstain. A long
  stretch that votes for neither is quiet (one mark in its middle, its error half the stretch) or
  sound the other did not hear (a mark at each end). Marks of the same jump from several compared
  recordings (the same seconds, within 3 s) are one cut: its time the mean of theirs, its error
  half their spread. This is why Audio match's `edits` output is retired: pieces from this node go
  into Audio match, so Audio match could not feed this node back (no cycles).
- **events n**: one lane per wired events input, named after what feeds it. Each event is a single
  moment with its error (one frame when it has none). An event confirms a cut a lane found when it
  falls within its error (a blue dot under the cut, "+ events" in the table); on its own it makes no
  cut. Events on another clock than the recording's are left out and counted.

Cues of different lanes within 1 s of each other are one cut. The picture lanes set the cut's
picture time, the sound lanes its sound time (a J-cut or L-cut has the two apart); with one kind
of cue only, both are the same. Of several cues, the surest (the smallest error) sets the time,
the strongest among equals.

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

- `cargo test --profile fast --lib cuts` also: the noise estimate under clicks (10 dB more noise
  reads 10 ± 2.5 dB more), a noise step and a level step marked on their moment.
- `cargo test --profile fast --bin corroborate cut_tests::a_jump_cut_is_found_by_comparing`: a
  made sound with 25 to 28 s taken out, compared with the original in the node: one cut, at
  25.002 s, two pieces.
- `cargo test --profile fast --test jiku_cuts -- --ignored --nocapture` (needs the Jiku data, see
  tests/jiku.rs), on real phone recordings of one event:
  - a recording with 3 s cut out at 40 s, compared with another phone: the cut at 39.92 s (80 ms
    off), 3.01 s missing (truth 3.00). Neither sound lane passes its threshold there
    (level 13 dB, noise 12 dB): a jump cut in a crowd is for the compare lane.
  - a recording cut into four pieces of 45 s and joined as 3 1 4 2, compared with the 10 other
    phones (3 heard all of it): all three joins found by each of the 3, joined into three cuts at
    44.81, 90.89 and 134.23 s (true 45, 90 and 135; errors stated 1.4, 1.0 and 1.5 s). Each
    piece, matched on its own against the other phones, lands within 9 to 14 ms of where the
    ground truth puts it, and their true order comes back (2 4 1 3).
  The joins are placed only to about a second: the place in the sound where one lag gives way to
  the other is as sharp as the sound there (a quiet start of a piece blurs it). A picture cue,
  where there is video, places it to the frame.

PySceneDetect as an oracle on the same clips (plan, stage 1) is not run yet: it needs a Python
package installed.

## Status

Stages 1 and 2 of the plan are built: picture and timestamps, hand cuts, pieces as rows; sound
level, background noise, `compare with` and the events input. Audio match taking pieces and the
placed strip follow (stage 3).

Files: `module.toml` (ports), `mod.rs` (ports from the rows, the rows' keys), `node.rs` (the run,
the outputs, the canvas), the method in `src/cuts.rs`.
