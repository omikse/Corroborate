# The workbench

The app itself: the canvas, the top bar, projects and recordings, running and keeping results.
What each node does is in its own README (the module table in the [main README](../../../README.md));
what the code here does is in [docs/ARCHITECTURE.md](../../../docs/ARCHITECTURE.md).

## The tool's folder

The tool is one movable folder: everything it keeps lives next to the program (in `app\`), in
`settings\` (the untitled project, recent projects, run turns, keys), `results\` (result JSON,
flight logs), `recordings\`, `cache\` and `grid logs\`. Nothing goes to system folders.

## The canvas

Right-click (or **+ Add node**) to add nodes, drag from a dot to a dot to wire (types must match).
Nodes have no windows: each shows its data and controls on the canvas.

A dot on a node always means the same:

| | means |
|---|---|
| colour | the type (the type table is in the main README) |
| shape | how the data arrives: circle a value (the latest replaces the one before), square a list (items keep coming and are kept), diamond a stream (raw data, not a value: audio, video) |
| filled | needed to run (a required input); optional inputs and every output are open |

Hovering a dot shows its type, what it is, and the value on it now. Hovering a time output walks
back along the wires: each node the time passed, what it did, and the time it gave (the result file
writes the same trace).

| Do | How |
|---|---|
| Zoom | scroll wheel |
| Pan | middle-drag |
| Select | click a node; Shift- or Ctrl-click adds or removes; right-drag a box on empty canvas (with Shift or Ctrl it adds or removes what it touches); Ctrl+A all; Escape none |
| Menus | right-click without moving: on a node, its menu; on empty canvas, Add node |
| Move | drag a selected node: all selected nodes move |
| Delete | Delete or Backspace |
| Copy, cut, paste, duplicate | Ctrl+C, Ctrl+X, Ctrl+V (at the pointer), Ctrl+D |
| Undo, redo | Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z (moves, wires, settings, names, adding, deleting) |
| Frame | F or Home: the selection, else every node |
| Arrange | the top bar's button: see below |
| Wire | drag from an output dot to an input dot, or the other way; right-click an input to unwire |
| Add a node already wired | drag from a dot onto empty canvas: pick from the modules that fit its type (from an input, a Value of its type comes first) |
| Play, pause | Space (or Play on a SOURCER or a Timeline node): the selected node's playhead runs and its sound plays |
| Playhead | click or drag in a SOURCER's waveform, or click in a Timeline node |
| One recording | select sources, right-click: One recording (the parts of one recording); Separate recordings undoes it |
| Group | Ctrl+G: the selected nodes into one group node (ports for the wires in and out); Tab or double-click opens it, Tab or the path at the top left goes back out; Alt+G ungroups |
| Group ports | inside a group, wire a node to the empty `+` of Group Input or Group Output; right-click a Group Output port to remove it |
| Uses of a group | Ctrl+D, copy and paste, or Add node → Groups: every use holds the same nodes (a change inside one is made in all), each with its own wires outside; right-click → Make single gives one use its own |
| Collapse | right-click a node → Collapse leaves its title and port names (saved with the project) |

Copied nodes are project-file text, so they paste into another Corroborate window or a text
editor. Keys go to a text box instead while you type in one. Undoing a setting is logged in the
result file like any other setting change.

**Arrange** lays out the nodes on this level by their wires, left to right, the way Graphviz dot
and ELK (Eclipse Layout Kernel) do: columns that keep the wires as short as they can be, a lane
through every column a long wire passes (so it runs between nodes, not behind them), each column
ordered to reduce crossings using the real port heights, then every wire as level as it can be
(network simplex). A column far taller than what it feeds (fifty recordings into one Audio match)
is dealt into several side by side; separate pieces are packed about the shape of a screen and
nodes with no wires are parked underneath. The result depends on the wires and node sizes, not on
where the nodes stood, so a second press changes nothing. Ctrl+Z puts them back.

## The top bar

- The project menu (its button is the project's name, with • while it has unsaved changes).
- **▶ Run** (every node that is not up to date; lit while anything runs), **■ Stop** (cancel
  everything queued or working), **↻ Run again** (everything anew, without the kept results) and
  **Auto** (on by default: a change makes what it touches stale and runs it again half a second
  after the last change; a project opens stopped, and Auto waits for Run).
- **Queue**, with counts ("2 working · 1 queued", after a run "5 done · 1 failed"): the list of
  every node that does work, in the order asked, with its state, step and percent, what it waits
  for, how long it took or why it failed, and ▶ or ■ to run or cancel it; under it how much is
  kept, and **Forget all kept**.
- **+ Add node**, which also has **Live device** (this computer's microphones and capture cards,
  asked of ffmpeg when it opens) and **Value of a type**.
- **Types**: every data type that travels on wires and where each came from; hovering one names
  the modules that give and take it, clicking one adds a Value node of it.
- **Arrange**, and a warning when recordings are missing or changed, which opens their list.

Messages are at the bottom of the canvas, over the line of keys.

## Running

Every node that does work (SOURCER, Cut detector, Audio match, ENF, Planes, plugins) has a state
dot before its name: grey not run, orange stale (it ran on something that has changed since), a
turning grey ring while queued (waiting for another node or a turn), a turning yellow ring while
working, green done, teal kept (loaded from what an earlier run kept, not worked out again), red
failed, brown cancelled, a pulsing red dot while live. Hover it for the step and percent, what it
waits for, how long it took or why it failed. **▶** in its title bar's top right corner runs it and
every node it needs that is not up to date (nothing after it); while it is queued or working it is
a **■**, which cancels it. Its right-click menu: Run, Run again (this node anew, without its kept
result), Cancel, Forget kept result.

One queue for the project ([docs/run-queue/plan.md](../../../docs/run-queue/plan.md)): a node
starts once what it needs is done and a turn of its kind is free, in the order asked. What each
node needs is in its README. A node that fails or is cancelled ends what needs it at once ("needs
'phone 3', which failed"), and nothing else. Turns: 2 downloads and half the processor cores'
worth of decoding and heavy work at once, set in `settings\run.txt`.

Settings are the only way the canvas changes processing: each value is checked against the
module's `module.toml` before it is used, every change is written to the result file with its
time, and what it touches runs again half a second after the last change (Auto).

## Kept results

What a node works out is kept in the project's `cache\` folder, one file per result, named by what
went into it: the node's kind and method version, its settings that change what it works out, and
what it reads (a file by its size and print, a web video by its link). What each node keeps is in
its README. Opening a project loads them at once (nothing decodes; the project opens stopped), and
a node whose result is kept loads it instead of working it out. A changed file makes its SOURCER
and what follows stale. A live source, Planes and plugins are not kept. Deleting `cache\` only
costs the time to work things out again; an untitled project keeps its results in `cache\` next to
the program.

## Projects

The project menu has **Rename…**, **New…** (Ctrl+N), **Open…** (Ctrl+O), **Open recent**, **Save**
(Ctrl+S), **Save as…** (Ctrl+Shift+S), **Save a copy…**, **Revert to saved**, **Recordings** and
**Show project folder**.

A project is a folder, and the folder's name is its only name: `project.txt` inside, `results\`
(what its Result records write), `recordings\` (files copied into it) and `cache\`. New, Open, Save
as and Save a copy all pick a folder, and New and the two saves can make one; the dialogs start in
`app\projects\`. A folder that already holds another project is refused for New and the saves, so
its file is never written over. **Rename…** puts a box with the name in place of the menu's
button: Enter or clicking away renames its folder, Esc leaves it.

The project file changes only on Save; until then changes are kept in `project.recovery.txt`
beside it, which the next Open restores. New, Open and closing the window ask Save, Don't save or
Cancel when there are unsaved changes. A project that was never given a folder (the first start,
or `--new`) is "untitled" and lives in `settings\project.txt` (written as you work). The project
last opened opens at the next start (`settings\last_project.txt`; the ten last opened are
`settings\recent.txt`).

## Recordings

Every file a SOURCER reads is listed under **Recordings** with its size and a print of its first
and last 64 KiB, and stays listed when its SOURCER is deleted (until Remove). Files stay where they
are unless **Copy into project** puts them in the project's `recordings\`. The project keeps each
one's path relative to the project file and its absolute path; Open looks at the relative path
first, then the absolute one, then for a file of the same size and print in the project folder,
and points the SOURCERs at it. Recordings that are missing (**Locate…**) or changed since they were
added (**Accept change**) are marked.

## The playhead

There is one real-time playhead, in UTC (Coordinated Universal Time), shared by all Timeline nodes.
A recording follows it only while its start time is wired into a Timeline (the
[Timeline README](../../modules/Timeline/README.md) says how). Every other recording keeps its own
playhead, linked to nothing; each view shows it in its own recording's seconds (a SOURCER's
waveform, the Planes view). Space plays the selected node's playhead in real time, with the sound
of its audio. A project with no live source is a **simulation**: every playhead is free. With a
live source it is **live**: its playhead stops at now (the newest audio), and Live jumps back to
it.

## The result file

It keeps every node's outputs, each time with its trace back along the wires, every setting change
with its time, and what each node adds of its own (the [Result record](../../modules/Result%20record/README.md)
writes it).
