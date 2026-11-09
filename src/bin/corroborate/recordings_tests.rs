//! Projects as folders and the recordings they keep: Save and the unsaved mark, the recovery
//! file, the question about unsaved changes, recordings found again after the project folder
//! moved, missing and changed recordings, copies into the project.
//! Run: cargo test --profile fast --bin corroborate recordings_tests -- --nocapture

use super::*;
use crate::drive::{out_dir, Drive};
use project::{project_file_of, project_in, project_place, push_recent, recent_in, recovery_of, taken_by, Answer, Then};
use recordings::{relative, State};
use std::path::Path;

/// A fresh folder under target\drive for one test.
fn fresh(name: &str) -> PathBuf {
    let d = out_dir().join("projects").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A short WAV file (8 kHz, a tone at `hz`).
fn wav(p: &Path, hz: f32, secs: f32) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let spec = hound::WavSpec { channels: 1, sample_rate: 8000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(p, spec).unwrap();
    for k in 0..(8000.0 * secs) as usize {
        w.write_sample(((k as f32 * hz * std::f32::consts::TAU / 8000.0).sin() * 8000.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

/// Each visible node's kind, setting and position, and each wire, to compare workbenches.
fn graph_of(app: &App) -> Vec<String> {
    let keep: Vec<NodeId> = (0..app.nodes.len()).filter(|&i| !app.nodes[i].hidden).collect();
    let mut v: Vec<String> = keep.iter().map(|&i| format!("{} {:?} {:?}", app.nodes[i].kind.spec().name, app.nodes[i].kind.saved_param(), app.nodes[i].pos)).collect();
    let g = app.graph.nodes();
    v.extend(app.graph.wires().iter().map(|w| format!("{}.{} -> {}.{}", w.from, g[w.from].spec.outputs[w.from_port].name, w.to, g[w.to].spec.inputs[w.to_port].name)));
    v
}

fn source_inputs(app: &App) -> Vec<String> {
    app.nodes.iter().filter(|n| !n.hidden).filter_map(|n| match &n.kind {
        Kind::Sourcer { input } => Some(input.clone()),
        _ => None,
    }).collect()
}

#[test]
fn places_and_relative_paths() {
    let p = |s: &str| PathBuf::from(s);
    assert_eq!(project_place(&p(r"C:\cases\case one.txt")), p(r"C:\cases\case one\case one.txt"), "Save as makes the project's folder");
    assert_eq!(project_place(&p(r"C:\cases\case one\case one.txt")), p(r"C:\cases\case one\case one.txt"), "already in its folder");
    assert_eq!(project_place(&p(r"C:\cases\x")), p(r"C:\cases\x\x.txt"), "gets .txt");
    assert_eq!(recovery_of(&p(r"C:\a\b.txt")), p(r"C:\a\b.recovery.txt"));
    if cfg!(windows) {
        assert_eq!(relative(&p(r"C:\cases\one"), &p(r"C:\cases\one\recordings\a.wav")), Some(p(r"recordings\a.wav")));
        assert_eq!(relative(&p(r"C:\cases\one"), &p(r"C:\media\a.wav")), Some(p(r"..\..\media\a.wav")));
        assert_eq!(relative(&p(r"C:\Cases\One"), &p(r"c:\cases\one\a.wav")), Some(p("a.wav")), "any case on Windows");
        assert_eq!(relative(&p(r"C:\cases"), &p(r"D:\a.wav")), None, "another drive");
    }
}

/// Save as picks a folder: the project is `<folder>\<folder name>.txt`; Open on a folder finds
/// that file, or the one project file there.
#[test]
fn folders_are_projects() {
    let d = fresh("folders").join("case two");
    std::fs::create_dir_all(&d).unwrap();
    assert_eq!(project_in(&d), d.join("case two.txt"));
    assert_eq!(project_file_of(&d), None, "an empty folder holds no project");
    let mut a = App::empty();
    a.add(Kind::Map, Pos2::new(10.0, 10.0));
    a.save_project_as(project_in(&d));
    assert_eq!(a.project_path.as_deref(), Some(d.join("case two.txt").as_path()), "no folder inside the picked one");
    assert_eq!(project_file_of(&d), Some(d.join("case two.txt")));
    let other = d.parent().unwrap().join("renamed");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::copy(d.join("case two.txt"), other.join("old name.txt")).unwrap();
    std::fs::write(other.join("notes.txt"), "not a project").unwrap();
    assert_eq!(project_file_of(&other), Some(other.join("old name.txt")), "the only project file in a renamed folder");
}

/// New makes the project in the picked folder at once, and Open on that folder finds it, nodes
/// added since and all. An empty project is still a project.
#[test]
fn new_makes_a_project_in_its_folder() {
    let d = fresh("new").join("harbour case");
    let mut a = App::empty();
    a.add(Kind::Map, Pos2::new(10.0, 10.0));
    a.new_project(&d);
    let file = d.join("harbour case.txt");
    assert_eq!(a.project_path.as_deref(), Some(file.as_path()), "the project file takes the folder's name");
    assert!(a.nodes.is_empty(), "New empties the canvas");
    assert!(!a.dirty(), "nothing to save yet");
    assert_eq!(project_file_of(&d), Some(file.clone()), "the new, empty project is there to Open");
    let mut b = App::empty();
    b.open_project(file.clone());
    assert_eq!(b.project_path.as_deref(), Some(file.as_path()));
    assert!(!b.dirty(), "an empty project opens with nothing unsaved");
    a.add(Kind::Map, Pos2::new(10.0, 10.0));
    assert!(a.dirty());
    assert!(a.save());
    let mut c = App::empty();
    c.open_project(project_file_of(&d).unwrap());
    assert_eq!(c.nodes.len(), 1, "saves go into the folder");
}

/// New and the saves are refused a folder holding another project; a project's own folder is
/// fine for its own Save as, and a folder of other files is free.
#[test]
fn a_folder_holding_a_project_is_taken() {
    let root = fresh("taken");
    let d = root.join("case");
    let mut a = App::empty();
    a.add(Kind::Map, Pos2::new(10.0, 10.0));
    a.save_project_as(project_in(&d));
    let file = project_in(&d);
    assert_eq!(taken_by(&d, None), Some(file.clone()), "for New and Save a copy");
    assert_eq!(taken_by(&d, Some(&file)), None, "its own folder, for its own Save as");
    assert_eq!(taken_by(&d, Some(&root.join("other").join("other.txt"))), Some(file), "another project's Save as");
    let notes = root.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("list.txt"), "not a project").unwrap();
    assert_eq!(taken_by(&notes, None), None, "other files take no room");
}

#[test]
fn recent_projects_newest_first_once() {
    let d = fresh("recent");
    for k in [1, 2, 3, 2] {
        push_recent(&d, &PathBuf::from(format!(r"C:\cases\p{k}\p{k}.txt")));
    }
    let names: Vec<String> = recent_in(&d).iter().map(|p| p.file_stem().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["p2", "p3", "p1"]);
    for k in 0..20 {
        push_recent(&d, &PathBuf::from(format!(r"C:\x\{k}.txt")));
    }
    assert_eq!(recent_in(&d).len(), 10, "ten at most");
}

/// Save as puts the project in its own folder; a change marks it unsaved and goes to the
/// recovery file, not the project file; Save writes it and drops the recovery file; New and
/// Open bring back the same graph and recordings, unchanged after a few frames.
#[test]
fn save_mark_and_round_trip() {
    let dir = fresh("round_trip");
    let clip = dir.join("media").join("north gate.wav");
    wav(&clip, 440.0, 1.0);
    let mut d = Drive::new();
    let src = d.app().add(Kind::Sourcer { input: clip.to_string_lossy().into_owned() }, Pos2::new(40.0, 60.0));
    let enf = d.app().add(Kind::Enf { grid: String::new() }, Pos2::new(400.0, 60.0));
    d.app().wire(src, "audio", enf, "audio");
    d.frames(3);
    assert_eq!(d.app().library.list.len(), 1, "the SOURCER's file joined the project");
    d.app().save_project_as(dir.join("case one.txt"));
    let file = dir.join("case one").join("case one.txt");
    assert_eq!(d.app().project_path.as_deref(), Some(file.as_path()));
    let text = std::fs::read_to_string(&file).unwrap();
    let rec = text.lines().find(|l| l.starts_with("rec|")).expect("a recording line");
    assert!(rec.contains(r"|..\media\north gate.wav|") || rec.contains("|../media/north gate.wav|"), "relative to the project file: {rec}");
    assert!(!d.app().dirty());
    d.frames(3);
    assert!(!d.app().dirty(), "still saved after some frames");
    d.shot("project_saved");

    d.app().nodes[enf].pos = Pos2::new(500.0, 200.0);
    d.app().save_project();
    assert!(d.app().dirty(), "a moved node is an unsaved change");
    assert!(recovery_of(&file).is_file(), "changes go to the recovery file");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text, "and not into the project file");
    d.frames(2);
    d.shot("project_unsaved");
    assert!(d.app().save());
    assert!(!recovery_of(&file).exists(), "Save drops the recovery file");
    assert!(!d.app().dirty());
    let want = graph_of(d.app());

    d.app().request(Then::New(dir.join("next case")));
    assert!(d.app().ask.is_none(), "nothing unsaved: no question");
    d.frames(2);
    assert!(d.app().nodes.is_empty() && d.app().library.list.is_empty());
    d.app().open_project(file.clone());
    d.frames(5);
    assert_eq!(graph_of(d.app()), want, "Open brings the same graph back");
    assert_eq!(d.app().library.list.len(), 1);
    assert_eq!(d.app().library.list[0].state, State::Ok);
    assert!(!d.app().dirty(), "an opened project has no unsaved changes");
}

/// Changes never saved come back from the recovery file at the next Open (marked unsaved);
/// Revert to saved drops them.
#[test]
fn recovery_restores_and_revert_drops() {
    let dir = fresh("recovery");
    let mut a = App::empty();
    a.add(Kind::Map, Pos2::new(10.0, 10.0));
    a.save_project_as(dir.join("r.txt"));
    let file = a.project_path.clone().unwrap();
    a.add(Kind::InvertLocation, Pos2::new(300.0, 10.0));
    a.save_project();
    assert!(recovery_of(&file).is_file());
    drop(a);

    let mut b = App::empty();
    b.open_project(file.clone());
    assert_eq!(b.nodes.len(), 2, "the unsaved node came back");
    assert!(b.dirty(), "and is marked unsaved");
    assert!(b.msg.contains("restored"), "{}", b.msg);
    b.request(Then::Revert);
    assert_eq!(b.ask, Some(Then::Revert), "Revert asks first");
    b.answer(Answer::Discard);
    assert_eq!(b.nodes.len(), 1, "back to the saved project");
    assert!(!b.dirty());
    assert!(!recovery_of(&file).exists());
}

/// New with unsaved changes asks; Cancel keeps all, Don't save goes on (and drops the
/// recovery file), Save writes first.
#[test]
fn unsaved_changes_ask_first() {
    let dir = fresh("ask");
    let mut d = Drive::new();
    d.app().add(Kind::Map, Pos2::new(40.0, 60.0));
    d.app().save_project_as(dir.join("ask.txt"));
    let file = d.app().project_path.clone().unwrap();
    d.app().add(Kind::Map, Pos2::new(400.0, 60.0));
    d.app().save_project();
    let next = dir.join("next case");
    d.app().request(Then::New(next.clone()));
    d.frames(3);
    assert_eq!(d.app().ask, Some(Then::New(next.clone())));
    d.shot("project_ask");
    d.click("Cancel");
    assert!(d.app().ask.is_none());
    assert_eq!(d.app().nodes.len(), 2, "Cancel keeps everything");
    assert!(!next.exists(), "Cancel makes no new project");
    d.app().request(Then::New(next));
    d.frames(2);
    d.click("Save");
    assert!(d.app().nodes.is_empty(), "saved, then a new project");
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().filter(|l| l.starts_with("node|")).count(), 2, "Save wrote both nodes");
    d.app().open_project(file.clone());
    d.app().add(Kind::Map, Pos2::new(400.0, 400.0));
    d.app().save_project();
    d.app().request(Then::Open(file.clone()));
    d.frames(2);
    d.click("Don't save");
    d.frames(2);
    assert_eq!(d.app().nodes.len(), 2, "the third node was not saved");
    assert!(!recovery_of(&file).exists());
}

/// A project folder moved somewhere else, with its recordings inside it and beside it: Open
/// finds them by their relative paths and points the SOURCERs there; a recording nowhere to be
/// found is missing, one edited since is changed, and a file renamed inside the folder is found
/// by its print.
#[test]
fn moved_missing_and_changed_recordings() {
    let root = fresh("moved");
    let old = root.join("old place");
    let (inside, beside, gone, edited, renamed) = (
        old.join("case").join("recordings").join("inside.wav"),
        old.join("clips").join("beside.wav"),
        old.join("clips").join("gone.wav"),
        old.join("clips").join("edited.wav"),
        old.join("case").join("renamed.wav"),
    );
    for (k, f) in [&inside, &beside, &gone, &edited, &renamed].into_iter().enumerate() {
        wav(f, 300.0 + 100.0 * k as f32, 0.5 + 0.1 * k as f32);
    }
    let mut a = App::empty();
    for (k, f) in [&inside, &beside, &gone, &edited, &renamed].into_iter().enumerate() {
        a.add(Kind::Sourcer { input: f.to_string_lossy().into_owned() }, Pos2::new(40.0, 60.0 + 200.0 * k as f32));
    }
    a.sync_recordings();
    assert_eq!(a.library.list.len(), 5);
    a.save_project_as(old.join("case").join("case.txt"));
    let want = graph_of(&a).len();
    drop(a);

    let new = root.join("new place");
    std::fs::rename(&old, &new).unwrap();
    std::fs::remove_file(new.join("clips").join("gone.wav")).unwrap();
    wav(&new.join("clips").join("edited.wav"), 999.0, 0.8);
    std::fs::rename(new.join("case").join("renamed.wav"), new.join("case").join("recordings").join("now here.wav")).unwrap();

    let mut b = App::empty();
    b.open_project(new.join("case").join("case.txt"));
    assert_eq!(graph_of(&b).len(), want);
    let state = |b: &App, name: &str| b.library.list.iter().find(|r| r.path.to_string_lossy().contains(name)).map(|r| r.state.clone());
    assert!(matches!(state(&b, "inside.wav"), Some(State::Moved(_))), "{:?}", b.library.list);
    assert!(matches!(state(&b, "beside.wav"), Some(State::Moved(_))));
    assert_eq!(state(&b, "gone.wav"), Some(State::Missing));
    assert_eq!(state(&b, "edited.wav"), Some(State::Changed));
    assert!(matches!(state(&b, "now here.wav"), Some(State::Moved(_))), "found by its print: {:?}", b.library.list);
    let inputs = source_inputs(&b);
    let nk = recordings::key(&new);
    for want in ["inside.wav", "beside.wav", "edited.wav", "now here.wav"] {
        let i = inputs.iter().find(|i| i.contains(want)).unwrap_or_else(|| panic!("{want}: {inputs:#?}"));
        assert!(recordings::key(Path::new(i)).starts_with(&nk), "{want} read from the new place: {i}");
    }
    assert!(inputs.iter().any(|i| i.contains("old place") && i.contains("gone.wav")), "a missing one keeps its path");
    // Locate the missing one; accept the edited one.
    wav(&root.join("found.wav"), 500.0, 0.5);
    let gone_id = b.library.list.iter().find(|r| r.state == State::Missing).unwrap().id.clone();
    b.move_recording(&gone_id, root.join("found.wav"), State::Unchecked);
    assert!(source_inputs(&b).iter().any(|i| i.contains("found.wav")));
    assert_eq!(b.library.get(&gone_id).unwrap().state, State::Changed, "another file than the one added");
    let edited_id = b.library.list.iter().find(|r| r.path.ends_with("edited.wav")).unwrap().id.clone();
    b.accept_change(&edited_id);
    assert_eq!(b.library.get(&edited_id).unwrap().state, State::Ok);

    let mut d = Drive::with(b);
    d.frames(3);
    d.click_contains("Recordings (5");
    d.shot("project_recordings_menu");
}

/// Copy into project copies a recording into the project's recordings\ folder on its own thread
/// and the SOURCER reads the copy; a deleted SOURCER's recording stays until Remove.
#[test]
fn copy_into_project_and_remove() {
    let dir = fresh("copy");
    let clip = dir.join("elsewhere").join("tower.wav");
    wav(&clip, 600.0, 1.0);
    let mut d = Drive::new();
    let src = d.app().add(Kind::Sourcer { input: clip.to_string_lossy().into_owned() }, Pos2::new(40.0, 60.0));
    d.frames(2);
    let id = d.app().library.list[0].id.clone();
    assert!(!d.app().copy_into_project(&id), "untitled: nowhere to copy to");
    d.app().save_project_as(dir.join("c.txt"));
    assert!(d.app().copy_into_project(&id));
    assert!(d.wait_until(10.0, |a| !a.copying()), "the copy finished");
    let copy = dir.join("c").join("recordings").join("tower.wav");
    assert!(copy.is_file());
    assert!(clip.is_file(), "the original stays");
    assert_eq!(recordings::key(Path::new(&source_inputs(d.app())[0])), recordings::key(&copy), "the SOURCER reads the copy");
    assert!(d.app().save());
    assert!(std::fs::read_to_string(dir.join("c").join("c.txt")).unwrap().contains(r"|recordings\tower.wav|") || cfg!(not(windows)));
    d.app().delete_nodes(&[src]);
    d.frames(2);
    assert_eq!(d.app().library.list.len(), 1, "the recording stays in the project");
    d.app().remove_recording(&id);
    assert!(d.app().library.list.is_empty());
}

/// The top bar: Project menu open, with the project's name and Recordings beside it.
#[test]
fn top_bar_shot() {
    let dir = fresh("top_bar");
    let clip = dir.join("field 03.wav");
    wav(&clip, 440.0, 0.5);
    let mut d = Drive::new();
    d.app().add(Kind::Sourcer { input: clip.to_string_lossy().into_owned() }, Pos2::new(40.0, 60.0));
    d.frames(2);
    d.app().save_project_as(dir.join("hillingdon.txt"));
    d.app().add(Kind::Map, Pos2::new(400.0, 60.0));
    d.frames(2);
    d.click("Project");
    d.shot("project_top_bar");
}
