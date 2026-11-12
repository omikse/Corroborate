//! Kept results (docs/run-queue/plan.md, stage 3): what a job worked out is kept in the project's
//! cache\ folder and loads when the project opens.
//! Run: cargo test --profile fast --bin corroborate kept_tests -- --nocapture

use super::*;
use crate::cut_tests::four_shots;
use crate::drive::{out_dir, Drive};
use crate::jobs::JobState;

/// Nothing queued or working.
fn idle(a: &App) -> bool {
    a.queue.waiting.is_empty() && (0..a.nodes.len()).all(|i| !a.nodes[i].state.lock().unwrap().running)
}

fn state(a: &App, j: NodeId) -> JobState {
    a.job_view(j).map_or(JobState::NotRun, |v| v.state)
}

fn steps(a: &App, j: NodeId) -> Vec<String> {
    a.nodes[j].state.lock().unwrap().progress.steps.iter().map(|s| s.name.clone()).collect()
}

fn kept_files(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".bin")).collect();
    v.sort();
    v
}

/// Stage 3's test, on the clip in place of the 44 minute video: a SOURCER on the clip with a Cut
/// detector, an Audio match of the clip and a copy of it, an ENF on the copy. Run: each result is
/// kept in the project's cache\. Opened again: each job is kept (loaded at once, nothing decodes
/// or reads), the cuts are there, and Run runs nothing. The clip changes: its SOURCER, the Cut
/// detector and the match turn stale and read it again; the copy's SOURCER and the ENF stay kept.
/// Run again works everything out anew; Forget empties the folder.
#[test]
fn kept_results_load_when_the_project_opens() {
    let dir = out_dir().join("kept_project");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("clip.mp4");
    let copy = dir.join("copy.mp4");
    std::fs::copy(four_shots(), &clip).unwrap();
    std::fs::copy(four_shots(), &copy).unwrap();
    let grid = dir.join("grid log.txt");
    std::fs::write(&grid, "grid log 1").unwrap();
    let file = dir.join("project.txt");
    let cache = dir.join("cache");

    let mut d = Drive::new();
    let app = d.app();
    app.project_path = Some(file.clone());
    let s1 = app.add(Kind::Sourcer { input: clip.to_string_lossy().into_owned() }, Pos2::new(20.0, 40.0));
    let s2 = app.add(Kind::Sourcer { input: copy.to_string_lossy().into_owned() }, Pos2::new(20.0, 400.0));
    let c = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(330.0, 40.0));
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(330.0, 400.0));
    let e = app.add(Kind::Enf { grid: grid.to_string_lossy().into_owned() }, Pos2::new(760.0, 400.0));
    app.wire(s1, "video", c, "video");
    app.wire(s1, "audio", c, "audio");
    app.wire_many(s1, "audio", m);
    app.wire_many(s2, "audio", m);
    app.wire(s2, "audio", e, "audio");
    let jobs = [s1, s2, c, m, e];
    app.run();
    assert!(d.wait_until(300.0, |a| idle(a) && jobs.iter().all(|&j| state(a, j) == JobState::Done)), "{:?}", jobs.map(|j| d.app().job_view(j).map(|v| v.text())));
    // Each result is kept: the sound (the clip and its copy are the same file: one kept sound
    // for both), the scan, the match, the hum track.
    assert!(d.wait_until(30.0, |_| kept_files(&cache).len() == 4), "{:?}", kept_files(&cache));
    let names = kept_files(&cache);
    for kind in ["sourcer-", "cut-", "match-", "enf-"] {
        assert!(names.iter().any(|n| n.starts_with(kind)), "{kind} in {names:?}");
    }
    let cuts = d.app().cut_view(c).cuts.iter().filter(|x| x.on()).count();
    assert_eq!(cuts, 3);
    // What the match found (a steady tone gives it little to link) and its line.
    let found = |a: &App| {
        let s = a.nodes[m].state.lock().unwrap();
        s.found.as_ref().map(|f| (f.links.len(), f.links_json.clone(), s.status.clone()))
    };
    let matched = found(d.app());
    assert!(matched.is_some(), "the match ended with a result");
    assert!(d.app().save_project_to(&file));

    // Opened again: everything is kept, at once; nothing decodes or reads.
    let mut d = Drive::new();
    d.app().open_project(file.clone());
    let app = d.app();
    assert!(app.stopped);
    let (s1, s2, c, m, e) = {
        let find = |f: &dyn Fn(&Kind) -> bool| app.nodes.iter().position(|n| !n.hidden && f(&n.kind)).unwrap();
        let s1 = app.nodes.iter().position(|n| matches!(&n.kind, Kind::Sourcer { input } if input.ends_with("clip.mp4"))).unwrap();
        let s2 = app.nodes.iter().position(|n| matches!(&n.kind, Kind::Sourcer { input } if input.ends_with("copy.mp4"))).unwrap();
        (s1, s2, find(&|k| matches!(k, Kind::CutDetector { .. })), find(&|k| matches!(k, Kind::AudioMatch { .. })), find(&|k| matches!(k, Kind::Enf { .. })))
    };
    let jobs = [s1, s2, c, m, e];
    assert!(app.nodes[c].state.lock().unwrap().cut.is_some(), "the scan is there as the project opens");
    assert!(d.wait_until(60.0, |a| idle(a) && jobs.iter().all(|&j| state(a, j) == JobState::Kept)), "{:?}", jobs.map(|j| d.app().job_view(j).map(|v| v.text())));
    d.frames(3);
    let app = d.app();
    assert_eq!(steps(app, s1), vec!["read the kept sound".to_string()], "the SOURCER read its kept sound, it did not decode");
    assert!(steps(app, c).is_empty(), "the Cut detector read nothing: {:?}", steps(app, c));
    assert_eq!(app.cut_view(c).cuts.iter().filter(|x| x.on()).count(), 3);
    assert_eq!(found(app), matched, "the same match, kept");
    assert!(app.nodes[s1].state.lock().unwrap().source.peaks.len() > 100, "the waveform is there");
    assert_eq!(app.job_view(c).unwrap().text(), "kept: loaded from cache\\, not worked out again");
    d.shot("kept_opened");
    d.app().queue_open = true;
    d.frames(3);
    d.shot("kept_queue_list");
    let ids: Vec<u64> = jobs.iter().map(|&j| d.app().nodes[j].state.lock().unwrap().run_id).collect();
    d.app().run();
    d.frames(5);
    let app = d.app();
    assert!(idle(app), "Run runs nothing: all is up to date");
    assert_eq!(jobs.iter().map(|&j| app.nodes[j].state.lock().unwrap().run_id).collect::<Vec<_>>(), ids, "nothing ran again");

    // The clip changes (its first 7.5 s): what reads it turns stale and runs again (Auto is
    // on and the project runs); the copy's SOURCER and the ENF on it stay kept.
    let short = out_dir().join("kept_clip_7s.mp4");
    let ok = corroborate::no_window(&mut std::process::Command::new("ffmpeg")).args(["-v", "error", "-y", "-i"]).arg(four_shots()).args(["-t", "7.5", "-c:v", "libx264", "-c:a", "aac"]).arg(&short).status().is_ok_and(|s| s.success());
    assert!(ok, "ffmpeg cut the clip");
    std::fs::copy(&short, &clip).unwrap();
    d.frames(2);
    let app = d.app();
    for j in [s1, c, m] {
        assert!(matches!(state(app, j), JobState::Stale | JobState::Queued | JobState::Working), "{}: {:?}", app.nodes[j].label, app.job_view(j).map(|v| v.text()));
    }
    assert!(d.wait_until(300.0, |a| idle(a) && [s1, c, m].iter().all(|&j| state(a, j) == JobState::Done)), "{:?}", jobs.map(|j| d.app().job_view(j).map(|v| v.text())));
    let app = d.app();
    assert!(steps(app, c).iter().any(|s| s == "every frame"), "the Cut detector read the clip again: {:?}", steps(app, c));
    assert_eq!(app.cut_view(c).cuts.iter().filter(|x| x.on()).count(), 2, "7.5 s: two cuts");
    assert_eq!(state(app, s2), JobState::Kept);
    assert_eq!(state(app, e), JobState::Kept);
    assert!(d.wait_until(30.0, |_| kept_files(&cache).len() == 7), "the new results are kept beside the old: {:?}", kept_files(&cache));

    // Run again: everything anew, without the kept results.
    d.app().run_again();
    assert!(d.wait_until(300.0, |a| idle(a) && jobs.iter().all(|&j| state(a, j) == JobState::Done)), "{:?}", jobs.map(|j| d.app().job_view(j).map(|v| v.text())));
    assert!(steps(d.app(), s2).iter().any(|s| s == "decode"), "the copy decoded anew: {:?}", steps(d.app(), s2));

    // Forget: one node's, then all.
    let before = kept_files(&cache).len();
    d.app().forget_kept(c);
    assert_eq!(kept_files(&cache).len(), before - 1);
    d.app().forget_all_kept();
    assert!(kept_files(&cache).is_empty());
}
