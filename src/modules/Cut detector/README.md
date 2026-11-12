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
  the file itself (one extra ffmpeg pass over every frame): a file, or a video from the web (a
  YouTube link) once its SOURCER has downloaded it into the project's `recordings\` (the run waits
  for that download). A live stream or a microphone is named in the status and not read.
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

Pieces go on as recordings: into Audio match (a row of its own, the file's pieces sharing one
phone id, so one clock speed) and into a Timeline. Other nodes that read a source's audio as it
decodes (ENF, Planes, plugins) do not take pieces yet.

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
- **background noise** (when `audio` is wired; it does not cut on its own: a mark over its
  threshold makes a cut only with another lane's cue within a second, and alone shows as a cut
  that is off, "background noise alone", for a person to switch on): the noise level under the sound for each second,
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

- The top line: the run's steps (the SOURCER's download when it has one to wait for, every frame,
  keyframes, sound, level and noise, each compare), each with a dot: grey waiting, yellow working
  (with how far, in percent and a bar, or a moving block when that is not known), green done, red
  failed (hover for why). Once all are done: how long it took, how many cuts are on, how many
  pieces.
- The filmstrip (a picture a second; a long recording's pictures go into one texture in rows, as
  a texture has a largest side), the waveform (the source's), then the lanes: a lane's curve,
  its marks (strong when over the threshold) and its threshold as a dashed line. Drag the
  triangle at the right end of a lane up or down to change its threshold; the change is saved
  with the project and goes into the result file's setting changes.
- Cuts are red lines down the rows, at the picture time over the picture rows and the sound time
  over the sound rows; a cut that is off is a grey dashed line. Hover a cut to read the cues
  behind it; click it to play it. The playhead is a yellow line down the rows: click the rows
  anywhere else, or click or drag the ruler, to move it there. Double-click the rows to cut there
  by hand (on the nearest frame, marked "hand"). Every switch goes into the result file.
- The player, under the ruler, to check each cut by eye and ear: the picture at the playhead
  (every frame, read a few seconds at a time at 320 by 180), the sound through the program's sound
  output (the node is selected, so its recording's sound plays, as Play on a SOURCER does).
  `◀ cut` and `cut ▶` go to the cut before or after and play it, from 2 s before it to 2 s after,
  then pause; the cut it is on is marked by a white triangle over the filmstrip. Next to them: Play
  or Pause from the playhead (Space too), Play the cut again, and Switch it off (or on); under them
  where the playhead is, the cut's time with its error, on or off, and the cues behind it.
- The pieces table: where each piece's picture and sound sit in the file, its length, the cues
  behind the cut before it, and its time out.
- The placed strip, once pieces go into an Audio match: each piece where it was placed (on real
  time when its group is anchored, else on the group's clock), in that order, the time between
  them hatched with its length. Under it the order in words, red when it is not the file's: the
  true order of a shuffled recording.

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
  outputs and wires. The player: a cut plays from 2 s before to 2 s after and pauses there, the
  pictures it reads are on the run's times (the cut's frame among them), every step of the run
  ends done. A YouTube link: the copy in the project's `recordings\` is read, the run waits while
  there is none, a copy next to the program is copied into the project. A filmstrip of 44 minutes
  (2645 pictures) fits a texture in rows (in one row it is 127 000 pixels wide, which the graphics
  card refuses: the program crashed on such a video once its run ended).
- `cargo test --profile fast --bin corroborate cut_tests::a_long_video -- --ignored --nocapture`:
  a 44 minute YouTube video, when its copy is in `app\recordings`.

- `cargo test --profile fast --lib cuts` also: the noise estimate under clicks (10 dB more noise
  reads 10 ± 2.5 dB more), a noise step and a level step marked on their moment.
- `cargo test --profile fast --bin corroborate cut_tests::a_jump_cut_is_found_by_comparing`: a
  made sound with 25 to 28 s taken out, compared with the original in the node: one cut, at
  25.002 s, two pieces.
- `cargo test --profile fast --bin corroborate cut_tests::shuffled_pieces_go_back_in_order`: the
  made sound cut at 20 and 40 s and joined as 2 1 3: the joins at 20.03 and 39.99 s, the three
  pieces placed by an Audio match at +20.00, +0.03 and +40.01 s into the original, as one phone,
  and the placed strip reads 2 1 3. A join at the other recording's very start is placed by the
  second either side of it (windows there can only vote one way).
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
  - `cargo test --profile fast --bin corroborate cut_tests::jiku -- --ignored --nocapture`: the
    same shuffled recording in the workbench: a Cut detector comparing it with the 3 phones, its
    four pieces wired into an Audio match with them. The three joins are on (the background noise
    adds two marks of its own, off), and each piece lands 14 to 22 ms from the ground truth
    against the first phone (on that phone's clock), all four as one phone; the placed strip reads
    2 4 1 3.
  The joins are placed only to about a second: the place in the sound where one lag gives way to
  the other is as sharp as the sound there (a quiet start of a piece blurs it). A picture cue,
  where there is video, places it to the frame.

PySceneDetect as an oracle on the same clips (plan, stage 1) is not run yet: it needs a Python
package installed.

## Status

Stages 1 to 3 of the plan are built: picture and timestamps, hand cuts, pieces as rows; sound
level, background noise, `compare with` and the events input; pieces into Audio match as one
device, and the placed strip. What is left is below; nothing of it is started.

## Further development

Stage 4 of the plan, each on its own merit:
- **Fades and dissolves**: TransNet V2 (a neural shot boundary detector) as a lane, only if the
  pure-Rust `tract` runs its ONNX (Open Neural Network Exchange) model, so the tool stays one
  portable folder. `scdet` sees hard cuts only; a slow dissolve never passes its threshold.
- **Codec frame grid**: MP3 or AAC (advanced audio coding) frames, and the encoder's block grid in
  the picture, broken where a re-encoded file was spliced (Bianchi et al. 2014; Yang, Shi, Huang
  2010). Format-specific work, for edits that were re-encoded.
- **Compilations of other people's videos**: vPDQ (frame hashes, the `pdqhash` crate) to find each
  piece inside the original videos, as `compare with` does for sound; VCSL (video copy segment
  localisation) as the benchmark.
- **Live stream outages**: a gap in a live reading is known exactly from SOURCER's PC (personal
  computer) clock; HLS (HTTP live streaming) playlists mark `#EXT-X-DISCONTINUITY` and
  `#EXT-X-GAP`, read from the playlist itself (ffmpeg's reader carries on past them). Needs the
  live path through the node: streams and microphones are not read now.

Found while building stages 1 to 3:
- **Pieces into ENF, Planes and plugins**: they read a source's audio as it decodes and take no
  pieces yet. ENF on each piece would place it against a grid log, the second way to put shuffled
  pieces in order.
- **Hum cues as events**: hum phase jumps (Rodríguez et al. 2010) and frequency breaks (Esquef et
  al. 2014) belong in ENF's library and come here as plain events. At 25 frames a second with 50 Hz
  hum (or 30 with 60 Hz) a cut on frame boundaries leaves the phase unchanged; worth stating on
  the node. The hum at the end of one piece continuing at the start of another could suggest an
  order without grid logs (weak, a suggestion only).
- **Sharper sound joins**: the compare lane places a join only to about a second on crowd sound.
  More recordings in `compare with` narrow it (the marks' spread); a picture cue places it to the
  frame. A finer vote where the lag change is small is untried.
- **MP4 edit lists** (`elst`): a file can hold removed parts it does not play; read them as cues.
- **Silence lane**: `silencedetect` marks (in the plan's stage 1) are not drawn yet.
- **PySceneDetect as an oracle** on the test clips (plan, stage 1): needs a Python package
  installed, so it waits for that download.
- **Background noise lane**: too rough on crowd sound to cut alone (it only confirms). A steadier
  estimate (longer windows, or only the quiet moments between sounds) would let it cut.

Files: `module.toml` (ports), `mod.rs` (ports from the rows, the rows' keys), `node.rs` (the run,
the outputs, the canvas), the method in `src/cuts.rs`.
