# Concept: live visual and auditory forensics

## The idea

Every recording carries more than its content. A phone video of a protest also holds the planes that
flew over it, the hum of the power grid it was near, thunder, horns, the angle of the sun and the
timestamps of the stream it went out on. Each of these is a small, measurable fact about where and when
the recording was made. On its own, each fact is weak. Combined, and checked against records from the
outside world, they can establish the place and time of a recording, line up many recordings of the
same event, and locate what happened in it.

This project is a workbench for doing that, with audio, images and video, on recordings and on live
streams, in a way anyone can follow and check.

## Why it is needed

Visual and auditory forensics is an established practice. Newsrooms and research groups have
reconstructed events by synchronising hundreds of videos and placing every camera on a map, and have
located gunshots from the sound in footage. But this work is done by expert teams, by hand, after the
event, over days or weeks.

Single techniques exist as separate tools: several programs geolocate a recording from aircraft Doppler
curves; others extract the power-grid hum; others compute sun and shadow angles. General OSINT
(open-source intelligence) tools connect facts about people, domains and addresses, but they do not look
inside audio or video at all.

What is missing is one place where these signals work together, where they can be used while an event
is still happening, and where every conclusion shows how it was reached.

## Principles

- **Raw and transparent.** Data is shown as it arrives. Nothing is hidden or smoothed without saying so.
  Every number is labelled with where it came from.
- **Every value has an uncertainty.** A time is never just a time: it is a time, give or take, from a
  named source. Conclusions inherit the uncertainty of what they were built from, and it is always shown.
- **Streams first.** Everything is built to work on live data. A file is simply a stream replayed at full
  speed, so the same tools serve both.
- **Capture the truth while it exists.** Some evidence is short-lived: aircraft positions, live
  timestamps, a stream that will be deleted. The workbench collects it alongside the recording.
- **People decide.** The tools measure, compare and suggest. A person reviews and makes the call.

## Building blocks: nodes

The work is done by **nodes**. A node takes inputs and produces outputs. An aircraft node takes audio and
outputs the planes it heard. A flight-data node takes a place and a time and outputs the planes that
should have been there. A metadata node takes a file and outputs what the file says about itself.

Some inputs are required, others optional. Optional inputs do not change what a node does, but they help
it: a time window narrows a search, a known location sharpens an estimate. Connecting the output of one
node to the optional input of another lets the nodes refine each other.

Inputs and outputs have types, such as audio, time, location or image. Where two nodes describe the same
thing in different forms, a **transform** node converts between them, for example coordinates written
as text into a location. Chains that work well can be saved as a single, reusable node.

Each node is its own program, written in any language. A node that crashes does not take the workbench
down, and people can build and share their own.

## The workspace

The workspace combines two familiar ideas: the node editor of tools like Blender, and the timeline of
music software like FL Studio.

- **The graph** is where nodes are placed and wired together.
- **The timeline** shows every node's results on one shared clock, in lanes: the plane that was
  expected, the plane that was heard, the hum, the bang, the change in the light. When things line up
  across lanes, it is visible at a glance.
- **Each node has its own window** showing its raw data: the spectrogram, the hum trace, the video frame.

Selecting any result highlights the chain of nodes and data that produced it. That chain is the
evidence trail.

## What the signals tell us

- **Aircraft overhead.** Propeller planes leave a characteristic falling tone as they pass. Matched
  against public flight-tracking data, a heard pass gives both a time and a distance, and several passes
  can give a position.
- **The power grid's hum.** Mains electricity runs at 50 or 60 Hz and wobbles slightly, in the same way
  across a whole grid at the same moment. The hum names the region, can be matched against grid records
  to find the time, and reveals cuts and edits.
- **Shared sounds.** Thunder, bangs, horns, sirens and chants are heard by several recordings at once.
  They are the threads that tie recordings together.
- **Light and sky.** Sun angle and shadows indicate time and place. A flash seen in two videos aligns
  them, because light arrives instantly. Weather can be checked against weather records.
- **What the recording says about itself.** File metadata, stream timestamps, and anything an
  investigator already knows, entered by hand.

None of these is decisive alone. Together they confirm or contradict one another.

## Synchronising many sources

When an event is filmed by several people, their recordings start at different, unknown moments. The
workbench lines them up using everything they share. Coarse anchors, such as stream timestamps, the hum
or a known overflight, bring them within seconds. Sharp shared sounds and flashes bring them much closer.
Because sound takes time to travel, each shared sound also says something about where each recorder
stood. A solver combines all of it into the best set of offsets and positions, each with its own
uncertainty. A single trusted clock, from one stream or from an aircraft track, then places the whole
set in real time.

Once the recordings are synchronised, a loud event heard by several of them can be located from the
differences in when it arrived.

## Examples

- **Verifying a live feed.** A public webcam claims to be in a certain town. Planes heard on its audio
  are checked against the flights that actually passed over that town.
- **A protest.** Several people are streaming when a loud bang goes off. The streams are synchronised,
  the bang is placed on a map with an area of uncertainty, and the evidence is kept for review.
- **A posted video.** Its claimed time and place are tested against the hum, the planes, the sun and the
  weather.
- **A phone call.** Sounds in the background narrow down where the caller was.

## The result

Every investigation produces one record: what was analysed, by which nodes, using which outside data,
what was found and how certain it is. The record can be shared, checked and reproduced by someone else.

## Responsibility

The same tools that locate an event also locate the people recording it, who are often journalists and
protesters. A confident but wrong result, given to someone acting in real time, can do harm. So results
always carry their uncertainty, a person always decides, only public sources are used, and every
technique is proven on past events before it is trusted live.

## Where it stands

Built so far: live and file analysis of audio, aircraft Doppler detection, power-grid hum tracking,
plugins as separate programs, metadata and manual data, result records, and a first check of detections
against live flight data. Next: the timeline with lanes, typed inputs and outputs for nodes, synchronising
several recordings, matching hum against grid records, and adding video.
