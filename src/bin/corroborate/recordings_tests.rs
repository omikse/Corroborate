//! Projects as folders and the recordings they keep: Save and the unsaved mark, the recovery
//! file, the question about unsaved changes, recordings found again after the project folder
//! moved, missing and changed recordings, copies into the project.
//! Run: cargo test --profile fast --bin corroborate recordings_tests -- --nocapture

use super::*;
use crate::drive::{out_dir, Drive};
use project::{bad_name, drop_recent, project_file_of, project_in, push_recent, recent_in, recovery_of, taken_by, Answer, Then};
use egui_kittest::kittest::Queryable;
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
    assert_eq!(project_in(&p(r"C:\cases\case one")), p(r"C:\cases\case one\project.txt"));
    assert_eq!(recovery_of(&p(r"C:\a\project.txt")), p(r"C:\a\project.recovery.txt"));
    if cfg!(windows) {
        assert_eq!(relative(&p(r"C:\cases\one"), &p(r"C:\cases\one\recordings\a.wav")), Some(p(r"recordings\a.wav")));
        assert_eq!(relative(&p(r"C:\cases\one"), &p(r"C:\media\a.wav")), Some(p(r"..\..\media\a.wav")));
        assert_eq!(relative(&p(r"C:\Cases\One"), &p(r"c:\cases\one\a.wav")), Some(p("a.wav")), "any case on Windows");
        assert_eq!(relative(&p(r"C:\cases"), &p(r"D:\a.wav")), None, "another drive");
    }
}

/// Save as picks a folder: the project is that folder's project.txt, and the folder's name is the
/// project's, even when the folder is renamed outside the program.
#[test]
fn folders_are_projects() {
    let d = fresh("folders").join("case two");
    std::fs::create_dir_all(&d).unwrap();
    assert_eq!(project_file_of(&d), None, "an empty folder holds no project");
    let mut a = App::empty();
    a.add(Kind::Map, Pos2::new(10.0, 10.0));
    a.save_project_as(d.clone());
    assert_eq!(a.project_path.as_deref(), Some(d.join("project.txt").as_path()), "no folder inside the picked one");
    assert_eq!(a.project_name(), "case two");
    assert_eq!(project_file_of(&d), Some(d.join("project.txt")));
    let other = d.parent().unwrap().join("renamed");
    std::fs::rename(&d, &other).unwrap();
    let mut b = App::empty();
    b.open_project(project_file_of(&other).unwrap());
    assert_eq!(b.project_name(), "renamed", "the name is the folder's");
    let notes = d.parent().unwrap().join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("list.txt"), "node|map|0|0||\n").unwrap();
    assert_eq!(project_file_of(&notes), None, "only project.txt is a project");
}

/// Renaming moves the folder: recordings inside it are read from the new place, those outside
/// stay, the name and the project file follow, and it opens clean afterwards.
#[test]
fn rename_moves_the_folder_and_what_is_in_it() {
    let root = fresh("rename");
    let (d, moved) = (root.join("harbour"), root.join("harbour blast"));
    let (inside, beside) = (d.join("recordings").join("inside.wav"), root.join("beside.wav"));
    wav(&inside, 300.0, 0.5);
    wav(&beside, 400.0, 0.5);
    let mut a = App::empty();
    for f in [&inside, &beside] {
        a.add(Kind::Sourcer { input: f.to_string_lossy().into_owned() }, Pos2::new(40.0, 60.0));
    }
    a.sync_recordings();
    a.save_project_as(d.clone());
    assert!(a.rename_project("harbour blast"), "{}", a.msg);
    assert!(!d.exists() && moved.join("recordings").join("inside.wav").is_file(), "the folder moved with what is in it");
    assert_eq!(a.project_name(), "harbour blast");
    assert_eq!(a.results_dir(), moved.join("results"));
    assert!(!a.dirty(), "a saved project stays saved");
    let inputs = source_inputs(&a);
    assert_eq!(recordings::key(Path::new(&inputs[0])), recordings::key(&moved.join("recordings").join("inside.wav")));
    assert_eq!(recordings::key(Path::new(&inputs[1])), recordings::key(&beside), "one outside the folder stays");
    let mut b = App::empty();
    b.open_project(project_in(&moved));
    assert!(b.library.list.iter().all(|r| r.state == State::Ok), "{:?}", b.library.list);
    assert!(!b.dirty());
    assert_eq!(graph_of(&b), graph_of(&a));
}

/// Unsaved changes stay unsaved through a rename, and come back from the recovery file.
#[test]
fn rename_keeps_unsaved_changes_unsaved() {
    let root = fresh("rename_unsaved");
    let mut a = App::empty();
    a.add(Kind::Map, Pos2::new(10.0, 10.0));
    a.save_project_as(root.join("before"));
    a.add(Kind::Map, Pos2::new(300.0, 10.0));
    a.save_project();
    assert!(a.rename_project("after"), "{}", a.msg);
    assert!(a.dirty());
    let file = project_in(&root.join("after"));
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().filter(|l| l.starts_with("node|")).count(), 1, "Save has not happened");
    assert!(recovery_of(&file).is_file());
    let mut b = App::empty();
    b.open_project(file);
    assert_eq!(b.nodes.len(), 2, "the unsaved node came back");
}

/// What a rename refuses, leaving the project where it was; changing only the letter case works.
#[test]
fn rename_refusals() {
    let root = fresh("rename_refused");
    std::fs::create_dir_all(root.join("taken")).unwrap();
    let mut a = App::empty();
    a.add(Kind::Map, Pos2::new(10.0, 10.0));
    assert!(!a.rename_project("x"), "untitled has no folder");
    a.save_project_as(root.join("case"));
    for name in ["", "   ", "a/b", r"a\b", "a:b", "what?", "dots.", "CON", "com1.backup", "taken", "case"] {
        assert!(!a.rename_project(name), "{name:?}");
        assert_eq!(a.project_name(), "case");
        assert!(root.join("case").join("project.txt").is_file());
    }
    assert!(a.msg.contains("not renamed") || a.msg.contains("saved"), "{}", a.msg);
    assert!(bad_name("Jiku GT_090912 (2)").is_none());
    assert!(bad_name("console").is_none(), "only the exact device names");
    assert!(a.rename_project("Case"), "{}", a.msg);
    assert_eq!(a.project_name(), "Case");
}

/// An undo step from before a rename reads a recording inside the folder from the new place.
#[test]
fn undo_after_a_rename_finds_the_recording() {
    let root = fresh("rename_undo");
    let inside = root.join("old").join("recordings").join("inside.wav");
    wav(&inside, 300.0, 0.5);
    let mut d = Drive::new();
    d.app().add(Kind::Sourcer { input: inside.to_string_lossy().into_owned() }, Pos2::new(40.0, 60.0));
    d.app().sync_recordings();
    d.app().save_project_as(root.join("old"));
    // A change becomes an undo step once it has held for a moment.
    d.wait(1.5);
    d.app().add(Kind::Map, Pos2::new(400.0, 60.0));
    assert!(d.app().rename_project("new"));
    assert!(d.app().undo());
    assert_eq!(d.app().nodes.iter().filter(|n| !n.hidden).count(), 1, "the Map is undone");
    let want = recordings::key(&root.join("new").join("recordings").join("inside.wav"));
    assert_eq!(recordings::key(Path::new(&source_inputs(d.app())[0])), want);
}

/// Clicking the name in the top bar makes it a box: Enter renames, Esc leaves it.
#[test]
fn the_name_in_the_top_bar_renames_the_project() {
    let root = fresh("rename_bar");
    let mut d = Drive::new();
    d.app().add(Kind::Map, Pos2::new(40.0, 60.0));
    d.app().save_project_as(root.join("first name"));
    d.frames(2);
    d.click("first name ⏷");
    d.click("Rename…");
    assert_eq!(d.app().renaming.as_deref(), Some("first name"));
    d.type_into(0, "typed then left");
    d.press(egui::Key::Escape);
    assert!(d.app().renaming.is_none());
    assert_eq!(d.app().project_name(), "first name", "Esc leaves it");
    d.click("first name ⏷");
    d.click("Rename…");
    d.type_into(0, "second name");
    d.shot("project_rename");
    d.press(egui::Key::Enter);
    assert!(d.app().renaming.is_none());
    assert_eq!(d.app().project_name(), "second name");
    assert!(root.join("second name").join("project.txt").is_file() && !root.join("first name").exists());
    d.shot("project_renamed");
}

#[test]
fn renamed_projects_leave_recent() {
    let d = fresh("recent_drop");
    for k in [1, 2] {
        push_recent(&d, &PathBuf::from(format!(r"C:\cases\p{k}\project.txt")));
    }
    drop_recent(&d, &PathBuf::from(r"C:\cases\p2\project.txt"));
    assert_eq!(recent_in(&d), [PathBuf::from(r"C:\cases\p1\project.txt")]);
}

/// New makes the project in the picked folder at once, and Open on that folder finds it, nodes
/// added since and all. An empty project is still a project.
#[test]
fn new_makes_a_project_in_its_folder() {
    let d = fresh("new").join("harbour case");
    let mut a = App::empty();
    a.add(Kind::Map, Pos2::new(10.0, 10.0));
    a.new_project(&d);
    let file = d.join("project.txt");
    assert_eq!(a.project_path.as_deref(), Some(file.as_path()));
    assert_eq!(a.project_name(), "harbour case", "the name is the folder's");
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
    a.save_project_as(d.clone());
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
    d.app().save_project_as(dir.join("case one"));
    let file = dir.join("case one").join("project.txt");
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
    a.save_project_as(dir.join("r"));
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
    d.app().save_project_as(dir.join("ask"));
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
    a.save_project_as(old.join("case"));
    let want = graph_of(&a).len();
    drop(a);

    let new = root.join("new place");
    std::fs::rename(&old, &new).unwrap();
    std::fs::remove_file(new.join("clips").join("gone.wav")).unwrap();
    wav(&new.join("clips").join("edited.wav"), 999.0, 0.8);
    std::fs::rename(new.join("case").join("renamed.wav"), new.join("case").join("recordings").join("now here.wav")).unwrap();

    let mut b = App::empty();
    b.open_project(new.join("case").join("project.txt"));
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
    d.click_contains("⏷");
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
    d.app().save_project_as(dir.join("c"));
    assert!(d.app().copy_into_project(&id));
    assert!(d.wait_until(10.0, |a| !a.copying()), "the copy finished");
    let copy = dir.join("c").join("recordings").join("tower.wav");
    assert!(copy.is_file());
    assert!(clip.is_file(), "the original stays");
    assert_eq!(recordings::key(Path::new(&source_inputs(d.app())[0])), recordings::key(&copy), "the SOURCER reads the copy");
    assert!(d.app().save());
    assert!(std::fs::read_to_string(dir.join("c").join("project.txt")).unwrap().contains(r"|recordings\tower.wav|") || cfg!(not(windows)));
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
    d.app().save_project_as(dir.join("hillingdon"));
    d.app().add(Kind::Map, Pos2::new(400.0, 60.0));
    d.frames(2);
    d.click("hillingdon • ⏷");
    d.shot("project_top_bar");
}

/// The top bar: the window title is the program and its build; Run, Stop, Run again and Auto are
/// always there; Run runs only what is not up to date, Run again everything anew; a missing
/// recording shows a warning that opens the recordings.
#[test]
fn top_bar_run_stop_restart_and_warning() {
    let dir = fresh("bar");
    let clip = dir.join("north.wav");
    wav(&clip, 440.0, 0.5);
    let mut d = Drive::new();
    let s = d.app().add(Kind::Sourcer { input: clip.to_string_lossy().into_owned() }, Pos2::new(40.0, 60.0));
    d.app().add(Kind::Map, Pos2::new(400.0, 60.0));
    d.frames(2);
    d.app().save_project_as(dir.join("bar"));
    d.app().stop_run();
    d.frames(2);
    assert!(d.app().title.starts_with("Corroborate · "), "{}", d.app().title);
    for b in ["▶ Run", "■ Stop", "↻ Run again", "Auto", "Arrange", "+ Add node", "bar ⏷"] {
        assert!(d.h.query_by_label(b).is_some(), "{b} is there while stopped");
    }
    assert!(d.h.query_by_label_contains("missing or changed").is_none(), "no warning while all is well");
    d.shot("top_bar_stopped");
    d.click("▶ Run");
    assert!(!d.app().stopped);
    let run_of = |d: &mut Drive| d.app().nodes[s].state.lock().unwrap().run_id;
    assert!(d.wait_until(30.0, |a| a.job_view(s).is_some_and(|v| v.state == crate::jobs::JobState::Done)), "the SOURCER runs");
    let first = run_of(&mut d);
    d.click("▶ Run");
    d.frames(2);
    assert_eq!(run_of(&mut d), first, "Run leaves what is up to date");
    d.click("↻ Run again");
    d.frames(2);
    assert_ne!(run_of(&mut d), first, "Run again runs it anew");
    for b in ["▶ Run", "■ Stop", "↻ Run again", "Auto"] {
        assert!(d.h.query_by_label(b).is_some(), "{b} is there while running");
    }
    d.shot("top_bar_running");
    d.click("■ Stop");
    assert!(d.app().stopped);
    std::fs::remove_file(&clip).unwrap();
    d.app().library.list[0].check();
    d.frames(2);
    d.click_contains("1 recording missing or changed");
    assert!(d.h.query_all_by_label_contains("missing").count() >= 2, "the warning and the recording marked missing");
    d.shot("top_bar_warning");
}
