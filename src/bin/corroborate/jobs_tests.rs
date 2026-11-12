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
    assert_eq!(app.job_view(c).map(|v| v.state), Some(JobState::Queued), "it waits for its SOURCER");
    app.stop_run();
    for j in [s, c] {
        let v = d.app().job_view(j).unwrap();
        assert!(matches!(v.state, JobState::Cancelled | JobState::Done), "{}", v.text());
    }
    assert!(d.wait_until(30.0, |a| !a.nodes[s].state.lock().unwrap().running), "the SOURCER's run ends");
    assert_eq!(d.app().nodes[s].state.lock().unwrap().progress.end, Some(crate::jobs::End::Cancelled));
    // The Cut detector never started: the queue says it was cancelled, and Auto leaves it so.
    d.wait(1.0);
    assert_eq!(d.app().job_view(c).unwrap().state, JobState::Cancelled);
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

/// When job `j` ran: from its start to its end.
fn span(app: &App, j: NodeId) -> (std::time::Instant, std::time::Instant) {
    let s = app.nodes[j].state.lock().unwrap();
    let t0 = s.progress.started.expect("it ran");
    (t0, t0 + std::time::Duration::from_secs_f64(s.progress.took.unwrap_or(0.0)))
}

fn run_id(app: &App, j: NodeId) -> u64 {
    app.nodes[j].state.lock().unwrap().run_id
}

/// Nothing queued or working.
fn idle(a: &App) -> bool {
    a.queue.waiting.is_empty() && (0..a.nodes.len()).all(|i| !a.nodes[i].state.lock().unwrap().running)
}

/// Stage 2's test (docs/run-queue/plan.md): ▶ on an Audio match fed a Cut detector's piece runs
/// the SOURCERs, then the Cut detector, then the match, and nothing else: a SOURCER and a Planes
/// wired elsewhere stay not run. As in a project just opened: stopped, so Auto runs nothing.
#[test]
fn play_runs_what_it_needs() {
    let mut d = Drive::new();
    let app = d.app();
    app.stopped = true;
    let s1 = app.add(Kind::Sourcer { input: four_shots().to_string_lossy().into_owned() }, Pos2::new(20.0, 40.0));
    let s2 = app.add(Kind::Sourcer { input: tone_wav().to_string_lossy().into_owned() }, Pos2::new(20.0, 330.0));
    let s3 = app.add(Kind::Sourcer { input: crate::drive::gps_clip().to_string_lossy().into_owned() }, Pos2::new(20.0, 620.0));
    let c = app.add(Kind::CutDetector { rows: vec![("pieces".into(), "4".into())] }, Pos2::new(330.0, 40.0));
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(760.0, 40.0));
    let p = app.add(Kind::Planes, Pos2::new(330.0, 620.0));
    app.wire(s1, "video", c, "video");
    app.wire(s1, "audio", c, "audio");
    app.wire_many(c, "audio 1", m);
    app.wire_many(s2, "audio", m);
    app.wire(s3, "audio", p, "audio");
    d.frames(3);
    let mut want = vec![c, s2];
    want.sort();
    assert_eq!(d.app().needs(m), want);
    assert_eq!(d.app().needs(c), vec![s1]);
    d.app().run_node(m);
    let app = d.app();
    for (j, want) in [(s1, JobState::Queued), (s2, JobState::Queued), (c, JobState::Queued), (m, JobState::Queued), (s3, JobState::NotRun), (p, JobState::NotRun)] {
        let v = app.job_view(j).unwrap();
        // A SOURCER may already be working.
        assert!(v.state == want || (want == JobState::Queued && v.state == JobState::Working), "{}: {}", app.nodes[j].label, v.text());
    }
    let why = app.job_view(m).unwrap().text();
    assert!(why.starts_with("queued: for "), "{why}");
    d.shot("queue_play_asked");
    assert!(d.wait_until(300.0, idle), "it ends");
    let app = d.app();
    for j in [s1, s2, c, m] {
        let v = app.job_view(j).unwrap();
        println!("{}: {}", app.nodes[j].label, v.text());
        assert_eq!(v.state, JobState::Done, "{}: {}", app.nodes[j].label, v.text());
    }
    for j in [s3, p] {
        assert_eq!(app.job_view(j).unwrap().state, JobState::NotRun, "{} is not needed", app.nodes[j].label);
    }
    // In order: the SOURCERs, then the Cut detector, then the match.
    let (sp1, sp2, cp, mp) = (span(app, s1), span(app, s2), span(app, c), span(app, m));
    assert!(sp1.1 <= cp.0, "the Cut detector started after its SOURCER ended");
    assert!(cp.1 <= mp.0 && sp2.1 <= mp.0, "the match started after the Cut detector and the tone");
    d.click("Queue");
    d.frames(3);
    d.shot("queue_play_done");
}

/// With Auto on, a change to one SOURCER redoes only it and what follows: the other SOURCERs
/// and the Cut detector on another recording are not run again; the Audio match is.
#[test]
fn a_change_redoes_only_what_it_touches() {
    let mut d = Drive::new();
    let app = d.app();
    let tone = tone_wav().to_string_lossy().into_owned();
    let s1 = app.add(Kind::Sourcer { input: tone.clone() }, Pos2::new(20.0, 40.0));
    let s2 = app.add(Kind::Sourcer { input: four_shots().to_string_lossy().into_owned() }, Pos2::new(20.0, 330.0));
    let s3 = app.add(Kind::Sourcer { input: tone.clone() }, Pos2::new(20.0, 620.0));
    let c = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(330.0, 330.0));
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(760.0, 40.0));
    app.wire(s2, "video", c, "video");
    for s in [s1, s2, s3] {
        app.wire_many(s, "audio", m);
    }
    app.run();
    assert!(d.wait_until(300.0, |a| idle(a) && a.job_view(m).is_some_and(|v| v.state == JobState::Done)), "the first run ends");
    let app = d.app();
    let before: Vec<u64> = [s1, s2, s3, c, m].iter().map(|&j| run_id(app, j)).collect();
    // Another file in the first SOURCER: it turns stale, and the match after it.
    let other = crate::drive::gps_clip().to_string_lossy().into_owned();
    if let Kind::Sourcer { input } = &mut app.nodes[s1].kind {
        *input = other;
    }
    d.frames(2);
    let app = d.app();
    assert_eq!(app.job_view(s1).unwrap().state, JobState::Stale, "{}", app.job_view(s1).unwrap().text());
    assert_eq!(app.job_view(m).unwrap().state, JobState::Stale);
    assert_eq!(app.job_view(s2).unwrap().state, JobState::Done);
    d.shot("queue_stale");
    assert!(d.wait_until(300.0, |a| run_id(a, m) != before[4] && idle(a)), "Auto runs it again");
    let app = d.app();
    let after: Vec<u64> = [s1, s2, s3, c, m].iter().map(|&j| run_id(app, j)).collect();
    assert_ne!(after[0], before[0], "the changed SOURCER ran again");
    assert_eq!(&after[1..4], &before[1..4], "the others did not: {before:?} {after:?}");
    for j in [s1, s2, s3, c, m] {
        assert_eq!(app.job_view(j).unwrap().state, JobState::Done, "{}", app.nodes[j].label);
    }
}

/// The mockup's Difficult project, built for real (files instead of downloads where a download
/// would need the network): three links that fail to download (nothing can be reached at
/// `.invalid`), one with a Cut detector after it; two Cut detectors sharing a recording; an Audio
/// match needing seven jobs; an ENF on a grid log; a plugin; a Cut detector with nothing wired.
/// A cancelled Cut detector ends the match that needs it, at once, and nothing else; a failed
/// download ends its Cut detector and nothing else. ▶ on the match then runs only what it lacks;
/// a new grid log redoes only the ENF (and the sound it reads). Not here: a live microphone (no
/// device in a test).
#[test]
fn the_difficult_project() {
    let clip = four_shots().to_string_lossy().into_owned();
    let tone = tone_wav().to_string_lossy().into_owned();
    let gps = crate::drive::gps_clip().to_string_lossy().into_owned();
    let grid = crate::drive::out_dir().join("queue_grid_log.txt");
    std::fs::write(&grid, "grid log 1").unwrap();
    let mut d = Drive::new();
    let app = d.app();
    app.stopped = true;
    let links: Vec<NodeId> = (1..=3).map(|k| app.add(Kind::Sourcer { input: format!("https://example.invalid/watch?v={k}") }, Pos2::new(20.0, 40.0 + 120.0 * k as f32))).collect();
    let cl = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(330.0, 160.0));
    app.wire(links[0], "video", cl, "video");
    let f1 = app.add(Kind::Sourcer { input: clip }, Pos2::new(20.0, 600.0));
    let ca = app.add(Kind::CutDetector { rows: vec![("pieces".into(), "4".into())] }, Pos2::new(330.0, 600.0));
    let cb = app.add(Kind::CutDetector { rows: vec![("pieces".into(), "4".into())] }, Pos2::new(330.0, 900.0));
    app.wire(f1, "video", ca, "video");
    app.wire(f1, "audio", ca, "audio");
    app.wire(f1, "audio", cb, "audio");
    let files: Vec<NodeId> = [&tone, &gps, &tone, &gps].iter().enumerate().map(|(k, f)| app.add(Kind::Sourcer { input: f.to_string() }, Pos2::new(20.0, 1200.0 + 300.0 * k as f32))).collect();
    let m = app.add(Kind::AudioMatch { rows: Vec::new() }, Pos2::new(800.0, 600.0));
    app.wire_many(ca, "audio 1", m);
    app.wire_many(cb, "audio 2", m);
    for &f in &files {
        app.wire_many(f, "audio", m);
    }
    let enf = app.add(Kind::Enf { grid: grid.to_string_lossy().into_owned() }, Pos2::new(800.0, 1200.0));
    app.wire(files[0], "audio", enf, "audio");
    let plug = app.add(Kind::Plugin { path: crate::drive::out_dir().join("no such plugin.exe").to_string_lossy().into_owned() }, Pos2::new(800.0, 1600.0));
    app.wire(files[1], "audio", plug, "audio");
    let lone = app.add(Kind::CutDetector { rows: Vec::new() }, Pos2::new(1300.0, 40.0));
    d.frames(3);
    let app = d.app();
    let mut needs = app.needs(m);
    needs.extend(needs.clone().iter().flat_map(|&n| app.needs(n)));
    needs.sort();
    needs.dedup();
    assert_eq!(needs.len(), 7, "the match needs seven jobs: {needs:?}");

    // Run, and cancel one Cut detector while it waits for its recording: the match that needs it
    // ends at once; the other Cut detector on the same recording goes on.
    app.run();
    assert_eq!(app.job_view(cb).unwrap().state, JobState::Queued);
    app.cancel_job(cb);
    let v = app.job_view(m).unwrap();
    assert_eq!(v.state, JobState::Cancelled, "{}", v.text());
    assert!(v.why.contains("which was cancelled"), "{}", v.text());
    assert!(matches!(app.job_view(ca).unwrap().state, JobState::Queued | JobState::Working));
    let v = app.job_view(lone).unwrap();
    assert_eq!(v.state, JobState::Failed, "nothing wired: it fails at once");
    d.shot("queue_difficult_running");
    assert!(d.wait_until(300.0, idle), "it all ends");
    let app = d.app();
    let text = |a: &App, j: NodeId| format!("{}: {}", a.nodes[j].label, a.job_view(j).unwrap().text());
    for j in (0..app.nodes.len()).filter(|&j| app.job_view(j).is_some()) {
        println!("{}", text(app, j));
    }
    for &l in &links {
        assert_eq!(app.job_view(l).unwrap().state, JobState::Failed, "{}", text(app, l));
    }
    let v = app.job_view(cl).unwrap();
    assert!(v.state == JobState::Failed && v.why.contains("which failed"), "{}", text(app, cl));
    for j in [f1, ca, enf, plug].into_iter().chain(files.iter().copied()) {
        assert_eq!(app.job_view(j).unwrap().state, JobState::Done, "no further: {}", text(app, j));
    }
    assert_eq!(app.job_view(cb).unwrap().state, JobState::Cancelled);
    assert_eq!(app.job_view(m).unwrap().state, JobState::Cancelled, "Auto does not ask again what was cancelled");
    d.click("Queue");
    d.frames(3);
    d.shot("queue_difficult_list");

    // ▶ on the match: only the Cut detector it lacks, then the match.
    let ran: Vec<u64> = [f1, ca].iter().chain(files.iter()).map(|&j| run_id(d.app(), j)).collect();
    d.app().run_node(m);
    assert!(d.wait_until(300.0, |a| idle(a) && a.job_view(m).is_some_and(|v| v.state == JobState::Done)), "{}", text(d.app(), m));
    let app = d.app();
    assert_eq!(app.job_view(cb).unwrap().state, JobState::Done);
    let again: Vec<u64> = [f1, ca].iter().chain(files.iter()).map(|&j| run_id(app, j)).collect();
    assert_eq!(ran, again, "nothing it had was run again");

    // A new grid log: only the ENF is stale; with Auto on it runs again (with the sound it reads).
    let (m_run, plug_run, enf_run) = (run_id(app, m), run_id(app, plug), run_id(app, enf));
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&grid, "grid log 2, a longer one").unwrap();
    assert!(d.wait_until(300.0, |a| run_id(a, enf) != enf_run && idle(a)), "the ENF runs again");
    let app = d.app();
    assert_eq!(app.job_view(enf).unwrap().state, JobState::Done);
    assert_eq!((run_id(app, m), run_id(app, plug)), (m_run, plug_run), "the match and the plugin are not run again");
    assert_eq!(app.job_view(m).unwrap().state, JobState::Done, "the match is up to date: its SOURCER's sound is the same");
}

/// Turns: three for two turns; the third waits until one gives its turn back.
#[test]
fn turns_limit_how_many_at_once() {
    static T: corroborate::ingest::Turns = corroborate::ingest::Turns::new(|| 2);
    let stop = std::sync::atomic::AtomicBool::new(false);
    let a = T.take(&stop).unwrap();
    let _b = T.take(&stop).unwrap();
    assert_eq!(T.now(), 2);
    let started = std::time::Instant::now();
    let h = std::thread::spawn(|| {
        let stop = std::sync::atomic::AtomicBool::new(false);
        let _c = T.take(&stop).unwrap();
        std::time::Instant::now()
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(a);
    let got = h.join().unwrap();
    assert!(got.duration_since(started) >= std::time::Duration::from_millis(250), "the third waited for a turn");
    let stop = std::sync::atomic::AtomicBool::new(true);
    assert!(T.take(&stop).is_none(), "stopped: no turn");
}
