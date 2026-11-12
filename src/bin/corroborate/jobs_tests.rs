//! Job states (docs/run-queue/plan.md, stage 1): the dot before each job node's name, its hover
//! text, and the queue list (Queue in the top bar).
//! Run: cargo test --profile fast --bin corroborate jobs_tests -- --nocapture

use super::*;
use crate::cut_tests::four_shots;
use crate::drive::{tone_wav, Drive};
use crate::jobs::JobState;
use std::collections::HashSet;

/// A clip and a tone, each in a SOURCER; a Cut detector on the clip, an Audio match on both, a
/// Planes on the tone, and a Cut detector with nothing wired. Before Run every job is not run;
/// while running, each passes through queued or working; at the end the ones that can are done
/// (with how long they took) and the unwired Cut detector has failed, saying why. The Queue button
/// counts them and its list shows each in the order they started.
#[test]
fn every_job_shows_its_state() {
    let clip = four_shots().to_string_lossy().into_owned();
    let tone = tone_wav().to_string_lossy().into_owned();
    let mut d = Drive::new();
    let app = d.app();
    let s1 = app.add(Kind::Sourcer { input: clip }, Pos2::new(20.0, 40.0));
    let s2 = app.add(Kind::Sourcer { input: tone }, Pos2::new(20.0, 330.0));
    let c = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(330.0, 40.0));
    let lone = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(760.0, 40.0));
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(330.0, 330.0));
    let p = app.add(Kind::Planes, Pos2::new(760.0, 400.0));
    let t = app.add(Kind::Time { rows: Vec::new() }, Pos2::new(760.0, 600.0));
    app.wire(s1, "video", c, "video");
    app.wire(s1, "audio", c, "audio");
    for (k, s) in [s1, s2].into_iter().enumerate() {
        app.wire(s, "audio", m, &format!("audio {}", k + 1));
        app.sync_many_ports();
    }
    app.wire(s2, "audio", p, "audio");
    let jobs = [s1, s2, c, lone, m, p];
    d.frames(3);
    let app = d.app();
    for &j in &jobs {
        assert_eq!(app.job_view(j).map(|v| v.state), Some(JobState::NotRun), "{}", app.nodes[j].label);
    }
    assert!(app.job_view(t).is_none(), "a Time node does no work: no dot");
    assert_eq!(app.queue_counts(), "nothing run");
    d.shot("jobs_before_run");

    d.app().run();
    let mut seen: HashMap<NodeId, HashSet<&'static str>> = HashMap::new();
    let ended = d.wait_until(300.0, |a| {
        for &j in &jobs {
            if let Some(v) = a.job_view(j) {
                seen.entry(j).or_default().insert(v.state.word());
            }
        }
        jobs.iter().all(|&j| a.job_view(j).is_some_and(|v| !matches!(v.state, JobState::Queued | JobState::Working | JobState::NotRun)))
    });
    let app = d.app();
    let states: Vec<String> = jobs.iter().map(|&j| format!("{}: {}", app.nodes[j].label, app.job_view(j).unwrap().text())).collect();
    println!("seen {seen:?}\nend {states:?}");
    assert!(ended, "every job ended: {states:?}");
    for &j in &[s1, s2, c, m, p] {
        let v = app.job_view(j).unwrap();
        assert_eq!(v.state, JobState::Done, "{}: {}", app.nodes[j].label, v.text());
        assert!(v.took.is_some() && v.text().starts_with("done in "), "{}", v.text());
        assert!(seen[&j].iter().any(|w| *w == "working" || *w == "queued"), "{} was seen working or queued: {:?}", app.nodes[j].label, seen[&j]);
    }
    let v = app.job_view(lone).unwrap();
    assert_eq!(v.state, JobState::Failed);
    assert!(v.text().contains("wire a recording"), "{}", v.text());
    assert_eq!(app.queue_counts(), "1 failed · 5 done");
    // The list: started first, in the order they started; Time is not in it.
    let order: Vec<NodeId> = app.jobs().into_iter().map(|(i, _)| i).collect();
    assert_eq!(order.len(), 6, "{order:?}");
    assert!(!order.contains(&t));

    d.click("Queue");
    d.frames(3);
    let screen = d.dump();
    assert!(screen.contains("\"Queue\""), "the list opened: {screen}");
    assert!(screen.contains("failed: wire a recording"), "{screen}");
    d.shot("jobs_queue_list");
}

/// Stop while a run is under way: what has not ended shows cancelled, at once.
#[test]
fn stop_cancels_what_runs() {
    let clip = four_shots().to_string_lossy().into_owned();
    let mut d = Drive::new();
    let app = d.app();
    let s = app.add(Kind::Sourcer { input: clip }, Pos2::new(20.0, 40.0));
    let c = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(330.0, 40.0));
    app.wire(s, "video", c, "video");
    app.run();
    assert_eq!(app.job_view(c).map(|v| v.state), Some(JobState::Queued), "it waits for a turn to decode first");
    app.stop_run();
    for j in [s, c] {
        let v = d.app().job_view(j).unwrap();
        assert!(matches!(v.state, JobState::Cancelled | JobState::Done), "{}", v.text());
    }
    assert!(d.wait_until(30.0, |a| !a.nodes[c].state.lock().unwrap().running), "the Cut detector's run ends");
    assert_eq!(d.app().nodes[c].state.lock().unwrap().progress.end, Some(crate::jobs::End::Cancelled));
}

/// yt-dlp's progress lines: bytes so far, the total or its estimate.
#[test]
fn download_percent_from_ytdlp() {
    use crate::sourcer_node::progress_of;
    assert_eq!(progress_of("corroborate-progress 500 1000 NA"), Some(0.5));
    assert_eq!(progress_of("corroborate-progress 250 NA 1000.5"), Some(250.0 / 1000.5));
    assert_eq!(progress_of("corroborate-progress 250 NA NA"), None);
    assert_eq!(progress_of("[download] Destination: x.mp4"), None);
}
