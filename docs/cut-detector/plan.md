# Cut detector

Status: researched and planned on 2026-11-12. Nothing is built yet.
Mockup: [mockup.html](mockup.html)

## The question it answers

Where is this recording cut, and into which pieces? Each piece then becomes a recording of its own,
with its own audio, video, time and length, so Audio match, ENF (electric network frequency) and the
Timeline can place every piece on real time the same way they place any recording.

It covers:
- an edited video (a news clip, a compilation, a reupload with parts removed);
- a video cut up and shuffled: the pieces are placed one by one, and their true order follows from
  where they land;
- a live stream that went offline and came back: the pause shows on the Timeline, and the clock on
  either side stays one clock.

It does not decide anything on its own. Every cue it finds is drawn with its strength, and the user
can switch any cut on or off (the "no guessing" rule).

## What we already have (internal survey)

| What | Where | Use for cuts |
|---|---|---|
| ffmpeg and ffprobe as outside programs, every format and stream | `ingest.rs`, SOURCER | decoding at full frame rate; ffmpeg's own filters `scdet` (scene change), `freezedetect`, `blackdetect`, `silencedetect`; `ffprobe -show_packets` / `-show_frames` for timestamps and keyframes. No new dependency. |
| SOURCER's video output, one small frame a second | SOURCER `node.rs` | the filmstrip on the node. Too coarse for cuts (a cut needs every frame), so the detector runs its own ffmpeg pass. |
| SOURCER live reading with PC (personal computer) clock time | SOURCER | outages in a live stream: the clock time at the gap is known exactly. |
| `audio_match::gaps`: where a pair's lag jumps between two links | `src/audio_match.rs` | cut against another recording of the same event: which one lacks a stretch and how long. Shown today only as marks in Audio match's Status column; none on Jiku (unedited phones). |
| `audio_match::match_pair`, `gcc_phat`, Wang fingerprints (audiofp) | `src/audio_match.rs` | the same comparisons, run per piece. |
| ENF hum track at 1 s steps (16 s window), its slope rule | `ENF (power hum)/mod.rs` | hum frequency jumps. Hum phase is not computed yet. |
| `clock::fit_against` with phone ids (one speed per device) | `src/clock.rs` | pieces of one file share a device, so they share one clock speed. |
| Declared but not built: Audio match `edits`, ENF `edits` | their `module.toml` (status "possible") | both say "for the cut detector". |
| Jiku Mobile Video Dataset GT_090912: 50 phone videos, sync ground truth | `tests/jiku.rs`, `tools/jiku.py` | test: cut and shuffle a real recording, check that the pieces land back where the truth says. |

Two rules from the graph shape the design:
- No cycles (`graph.rs`). Pieces from the cut detector go into Audio match, so Audio match's own
  `edits` output cannot feed the same cut detector. The detector therefore compares against other
  recordings itself, with the same library calls (`audio_match`), through an input `compare with`.
  Audio match's `edits` output can then be retired (one mechanism, not two).
- Shared technology stays in the library (module granularity). The hum phase belongs in the ENF
  library code, and ENF's `edits` output gives its breaks as plain events, wired into the detector's
  `cues` input, as agreed earlier.

## What is out there (external survey)

### Picture cuts (shot boundaries)

| Tool or paper | What | Licence, language | Fit |
|---|---|---|---|
| ffmpeg `scdet` filter | score per frame from the mean absolute frame difference; default threshold 10 (the patch suggests 8 to 14) | LGPL (lesser general public licence), already used as an outside program | **first step**: zero cost, hard cuts. Misses fades, fooled by flashes and fast camera moves. |
| PySceneDetect | content, adaptive (2 pass, for camera motion), threshold (fades), histogram and hash detectors | BSD-3 (Berkeley licence), Python, active (5.4k stars, pushed 2026-11-10) | **test oracle** on our test clips, like audiofp was for fingerprints. |
| TransNet V2 (Souček, Lokoč 2020) | neural network on 100-frame windows at 48×27 pixels; F1 (balance of precision and recall) 96.2 on BBC (British Broadcasting Corporation) Planet Earth, 93.9 on RAI (Italian state broadcaster), 77.9 on ClipShots | MIT, weights included, TensorFlow and PyTorch | **later step**: best open model for fades and dissolves. Needs ONNX (Open Neural Network Exchange) export and a Rust runtime (`tract`, pure Rust, or `ort`, a native library next to the exe). Not checked that `tract` runs its 3D convolutions. |
| AutoShot (Zhu et al., CVPR (Computer Vision and Pattern Recognition) workshops 2023) | architecture search; +4.2 F1 over TransNet V2 on short videos, +1 on ClipShots, BBC, RAI | code released, licence not found | not worth the gap over TransNet V2. |
| PERSIST (Lin et al., arXiv 2608.29287, 2026) | a cut only where the change lasts, against shake, flicker and blur | paper only | the same idea is cheap to copy: compare frames a second before and after, not only neighbours. |
| `av-scenechange` (from the rav1e encoder) | Rust crate | MIT, BSD-2, ISC | **no**: its own docs say it finds keyframe places for an encoder, not cuts as a person sees them. |

The gap in all of them: a **jump cut** (same camera, same scene, a few seconds removed) barely changes
the picture. Picture cues alone miss the cuts that matter most for sync. Sound and timestamps catch
them.

### Sound cuts (audio splicing forensics)

| Paper | Cue | Fit |
|---|---|---|
| Rodríguez, Apolinário, Biscainho, IEEE TIFS (Transactions on Information Forensics and Security) 2010 | **hum phase jump**: band-pass round 50/60 Hz, high-precision phase; the size of the jump says inserted or removed | **yes**, with clear hum. Goes into ENF's library. |
| Esquef, Apolinário, Biscainho, IEEE TIFS 2014 | hum frequency against an adaptive threshold; equal error rate 7 % → 4 % | yes, the frequency side of the same track. |
| Nicolalde-Rodríguez et al., EUSIPCO 2013; Niţă et al. 2015 | the same on hum harmonics | yes: ENF already traces 8 harmonics. |
| Pan, Zhang, Lyu, ICASSP 2012 | **local noise level** jumps (kurtosis in band-pass filtered audio stays near constant) | **yes**: cheap, catches pieces from another place or time. |
| Bianchi et al. 2014; Yang, Shi, Huang 2010 | MP3 or AAC (advanced audio coding) frame grid broken at a splice, double compression | later: only when the edit was re-encoded; format-specific work. |
| Moussa, Hirsch, Riess 2022 (Transformer, code at their university) and 2025–26 speech-edit datasets (MIST, AiEdit, HQ-MPSD) | neural splice localisation, mostly speech and deepfake editing | no for now: speech-only, heavy, and HQ-MPSD reports detectors failing on clean splices. |

A limit that follows from the arithmetic, worth showing on the node: at 25 frames a second a frame lasts
40 ms, exactly 2 cycles of 50 Hz hum, so **a cut on frame boundaries leaves the 50 Hz phase unchanged**.
The same holds for 60 Hz at 30 frames a second. In those cases only the hum frequency (when the removed
stretch is long enough for the grid to drift) and the other cues can see the cut. At 30 frames a second
with 50 Hz hum each frame is 1⅔ cycles, so the phase jumps by 240° unless a multiple of 3 frames is
removed.

### Timestamps and stream outages

- `ffprobe -show_packets`: packet times jump where a stream paused (gaps larger than one frame).
- HLS (HTTP live streaming) playlists mark `#EXT-X-DISCONTINUITY` and `#EXT-X-GAP`; ffmpeg's HLS reader
  is known to carry on past a discontinuity as if nothing happened, so we read the playlist ourselves.
- `ffprobe -show_frames` gives keyframe positions. A file cut without re-encoding must start every piece
  on a keyframe, so a keyframe off the regular GOP (group of pictures) rhythm is a cue
  (GOP size estimation, Vázquez-Padín et al., WIFS (Workshop on Information Forensics and Security) 2012; citation from memory, not checked).
- MP4 edit lists (`elst`) can hide removed parts in a file that still contains them.
- ffmpeg `freezedetect`, `blackdetect`, `silencedetect`: frozen picture, black and silence are how
  outages and fillers look in a recording.

### Matching pieces against other recordings

- Ours: `audio_match::gaps` (lag jumps against another recording).
- Video: vPDQ (Meta ThreatExchange, BSD; PDQ in Rust as the `pdqhash` crate) matches clips inside
  longer videos by frame hashes. VCSL (video copy segment localisation, Ant Group, CVPR 2022) is the
  benchmark; its temporal alignment comparison (Hough voting, dynamic programming, DTW (dynamic time
  warping), temporal network) applies directly. Later: for a compilation made of other people's
  videos.

### Reordering shuffled pieces

Nothing ready-made was found. The plan does not need it: each piece is placed on real time on its own
(Audio match against other recordings, ENF against a grid log, or a Time node by hand), and the true
order is where they land. One cue of our own, not from a paper: the hum frequency at the end of one
piece should continue at the start of the next, which can suggest an order even without grid logs.
Weak; shown as a suggestion only.

## Design

### The node

Name **Cut detector**, category Detectors.

Inputs:
- `audio` and `video` (either or both): the recording to split. The detector reads the source itself at
  full frame rate (one extra ffmpeg pass), as Audio match decodes itself.
- `time` (optional): when the recording starts.
- `compare with` (audio, many, optional): other recordings of the same event; lag jumps against each
  are a cue lane of their own.
- `cues` (event, many, optional): cut events from other nodes, for example ENF's `edits`.

Outputs, one row per piece (grow like Audio match's rows):
- `audio n`, `video n`, `length n`: the piece.
- `time n`: the piece's start, as played: the wired start plus where the piece sits in the file, with
  the file's error, and a source line saying "as played, cuts before it may hide time". Audio match
  or ENF downstream place it truly; where they disagree, it shows there (see open questions).
- `cuts` (event): every accepted cut, with the cues behind it, for the result file.

Pieces from one file are one device: Audio match suggests the same phone id for them, so they share
one clock speed.

### On the canvas (see the mockup)

1. A filmstrip and waveform of the whole file.
2. One lane per cue, under it, aligned in time: picture, sound level and noise, hum (from ENF's
   events), timestamps and keyframes, and one lane per recording in `compare with`. Each cue is a mark
   whose height is its strength; the lane's threshold is a line the user can drag.
3. Cuts: a red line where a cue passes its threshold. A click switches a cut off (dashed grey) or puts
   one where no cue reached (a hand cut, marked as such). Hover says which cues agree and by how much.
4. A table of pieces: number, in the file from–to, length, cues behind its start cut, and the output
   port.
5. Once pieces are placed downstream, a second strip "placed" draws them on real time, in their true
   order, with hatched gaps for removed time and arrows where the order differs from the file.

### Stages

1. **Picture and timestamps, by hand first.** The node with filmstrip, ffmpeg `scdet` lane, packet gaps
   and keyframes lane, freeze, black and silence marks; hand cuts; pieces out as rows. Test: a
   synthetic clip cut with ffmpeg at known frames; PySceneDetect on the same clips as the oracle.
2. **Sound cues.** Noise level jumps (Pan 2012) and the sound level lane in the library; ENF's hum phase
   and frequency breaks (Rodríguez 2010, Esquef 2014) as ENF's `edits` output, wired into `cues`; the
   `compare with` lane through `audio_match::gaps`, each jump then located to a fraction of a second with short GCC-PHAT (generalized cross-correlation with phase transform) windows round it (Audio match's links are 15 s apart). Retire Audio match's `edits` output. Test: Jiku
   recordings cut by a few seconds (jump cuts) with no picture change.
3. **Pieces as one device and the placed strip.** Same phone id suggestion, one clock speed, the
   "placed" strip fed back from what is wired downstream. Test: cut a Jiku recording into 4 pieces,
   shuffle them, wire all into Audio match with the other phones; every piece must land within the
   ground truth's error (Jiku's sync points are within about 10 ms), in the true order.
4. **Later, each on its own merit:** TransNet V2 through ONNX for fades and dissolves (after checking
   `tract`); codec frame grid; vPDQ for compilations of other people's videos; live stream outages
   from SOURCER's PC clock and HLS tags.

Every stage updates the node's README in the same commit, like Audio match.

## Open questions (with my recommendation)

1. **Time of a later piece before anything places it.** Recommended: as played (file start + position)
   with the file's error and a clear source line; Audio match shows the disagreement in its Status
   column. Alternative: no time out until something places it.
2. **Does a cut need a threshold or the user's click?** Recommended: thresholds per lane with
   defaults from the tests, every cut switchable; nothing hidden.
3. **Live streams**: stage 4 or with stage 1? Outages are only timestamps, cheap, but need the live
   path through the node. Recommended: stage 4.
4. **TransNet V2**: worth a runtime next to the exe (`ort`), or only if pure-Rust `tract` runs it?
   Recommended: only `tract`, to keep the tool one portable folder.

## Sources

- ffmpeg `scdet` source: https://ffmpeg.org/doxygen/8.0/vf__scdet_8c_source.html
- PySceneDetect: https://github.com/Breakthrough/PySceneDetect
- TransNet V2: https://arxiv.org/pdf/2008.04838 and https://github.com/soCzech/TransNetV2
- AutoShot: https://arxiv.org/pdf/2304.06116
- PERSIST: https://arxiv.org/pdf/2608.29287
- av-scenechange: https://docs.rs/av-scenechange
- Rodríguez, Apolinário, Biscainho 2010: https://dblp.org/rec/journals/tifs/RodriguezAB10
- Esquef et al. 2014, edit detection via instantaneous ENF variations: https://archive.cps-vo.org/node/19529
- Niţă et al., multi-harmonic ENF discontinuity: https://scientificbulletin.upb.ro/static/pdfs/rezbb1_540742.pdf
- Pan, Zhang, Lyu 2012: https://cse.buffalo.edu/~siweilyu/papers/icassp12.pdf
- Bianchi et al. 2014 (MP3 frame offsets): https://link.springer.com/10.1186/1687-417X-2014-10
- Moussa, Hirsch, Riess 2022: https://arxiv.org/pdf/2207.14682
- MIST: https://arxiv.org/html/2605.02223 ; AiEdit: https://arxiv.org/abs/2601.21463v1 ; HQ-MPSD: https://deepfake-demo.aisec.fraunhofer.de/related_work/2512.13012
- VCSL: https://github.com/ant-research/VCSL ; vPDQ and TMK: https://github.com/facebook/ThreatExchange ; PDQ and TMK test drive: https://arxiv.org/pdf/1912.07745
- HLS discontinuity in ffmpeg: https://fftrac-bg.ffmpeg.org/ticket/8009 ; Apple TN2235: https://developer.apple.com/library/archive/technotes/tn2235/_index.html
- ffmpeg freeze, black and silence filters: https://www.ffmpeg.org/ffmpeg-all.html
