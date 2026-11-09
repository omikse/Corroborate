//! Scenes for judging Clean up: a copy of a real project and hand-made stress cases, each measured
//! before and after, with a screenshot of the result in target/drive/ (lab_*.png). Nothing of yours
//! is written: tests have no project file.
//!
//! cargo test --profile fast --bin corroborate cleanup_lab -- --ignored --nocapture

use super::drive::{self, Drive};
use super::*;
use corroborate::node_layout::{measure, Item, Link, Report};

const JIKU: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/app/projects/Jiku GT_090912/Jiku GT_090912.txt");

/// The canvas as the placer sees it, in canvas units.
fn sheet(app: &mut App) -> (Vec<Item>, Vec<Link>, Vec<(f32, f32)>) {
    let ids: Vec<NodeId> = (0..app.nodes.len()).filter(|&i| app.on_canvas(i)).collect();
    let (pan, zoom) = (app.pan, app.zoom);
    (app.pan, app.zoom) = (Vec2::ZERO, 1.0);
    let items: Vec<Item> = ids
        .iter()
        .map(|&i| {
            let r = app.node_rect(Pos2::ZERO, i);
            Item { x: r.left(), y: r.top(), w: r.width(), h: r.height() }
        })
        .collect();
    let slot = |n: NodeId| ids.iter().position(|&i| i == n);
    let links = app
        .shown_lines()
        .iter()
        .filter_map(|s| {
            let (from, to) = (slot(s.from)?, slot(s.to)?);
            Some(Link { from, from_y: app.port_place(Pos2::ZERO, s.from, s.from_port, true).0.y - items[from].y, to, to_y: app.port_place(Pos2::ZERO, s.to, s.to_port, false).0.y - items[to].y })
        })
        .collect();
    (app.pan, app.zoom) = (pan, zoom);
    let at = items.iter().map(|i| (i.x, i.y)).collect();
    (items, links, at)
}

fn line(name: &str, n: usize, w: usize, r: &Report) {
    println!(
        "{name:34} {n:3} nodes {w:4} wires | {:6.0} x {:6.0} (w/h {:.2}) | cross {:5} behind {:4} back {:3} overlap {:3} | wire {:8.0}",
        r.width,
        r.height,
        r.width / r.height.max(1.0),
        r.crossings,
        r.behind,
        r.backward,
        r.overlaps,
        r.wire
    );
}

/// Measures the canvas, presses Clean up, measures again, takes a picture, presses it again to see
/// that a second press settles.
fn judge(d: &mut Drive, name: &str) -> Report {
    d.frames(3);
    let (items, links, at) = sheet(d.app());
    line(&format!("{name}: as it stood"), items.len(), links.len(), &measure(&items, &links, &at));
    let before = items.clone();
    d.click("Clean up");
    d.frames(3);
    let (items, links, at) = sheet(d.app());
    let changed = before.iter().zip(&items).filter(|(a, b)| (a.w - b.w).abs() > 0.5 || (a.h - b.h).abs() > 0.5).inspect(|(a, b)| println!("   size changed: {} x {} -> {} x {}", a.w, a.h, b.w, b.h)).count();
    if changed > 0 {
        println!("   {changed} nodes changed size after the press");
    }
    let after = measure(&items, &links, &at);
    line(&format!("{name}: cleaned"), items.len(), links.len(), &after);
    d.shot(&format!("lab_{name}"));
    d.click("Clean up");
    d.frames(3);
    let (_, _, again) = sheet(d.app());
    let moved = at.iter().zip(&again).map(|(a, b)| (a.0 - b.0).abs().max((a.1 - b.1).abs())).fold(0.0, f32::max);
    println!("{:34} second press moved a node by at most {moved:.1}", "");
    let (a2, l2, b2) = sheet(d.app());
    d.click("Clean up");
    d.frames(3);
    let (a3, l3, b3) = sheet(d.app());
    let third = b2.iter().zip(&b3).map(|(a, b)| (a.0 - b.0).abs().max((a.1 - b.1).abs())).fold(0.0, f32::max);
    let sized = a2.iter().zip(&a3).filter(|(a, b)| (a.w - b.w).abs() > 0.5 || (a.h - b.h).abs() > 0.5).count();
    let ported = l2.iter().zip(&l3).filter(|(a, b)| (a.from_y - b.from_y).abs() > 0.5 || (a.to_y - b.to_y).abs() > 0.5).count();
    let first_ports = links.iter().zip(&l2).filter(|(a, b)| (a.from_y - b.from_y).abs() > 0.5 || (a.to_y - b.to_y).abs() > 0.5).count();
    println!("{:34} third press moved {third:.1}; sizes changed {sized}, ports changed {ported}; ports differing after the first press {first_ports}", "");
    after
}

fn jiku() -> Drive {
    let text = std::fs::read_to_string(JIKU).expect("the Jiku project");
    let mut d = Drive::new();
    d.app().load_project_text(&text);
    d.app().stop_run();
    d
}

#[test]
#[ignore]
fn cleanup_lab_jiku() {
    let mut d = jiku();
    judge(&mut d, "jiku");
    // How far each node that feeds nothing stands from what feeds it, and what the wire passes.
    let app = d.app();
    let ids: Vec<NodeId> = (0..app.nodes.len()).filter(|&i| app.on_canvas(i)).collect();
    let (items, links, at) = sheet(app);
    for (n, &id) in ids.iter().enumerate() {
        if links.iter().any(|l| l.from == n) || !links.iter().any(|l| l.to == n) || links.iter().filter(|l| l.to == n).count() > 3 {
            continue;
        }
        for l in links.iter().filter(|l| l.to == n) {
            let gap = at[n].0 - (at[l.from].0 + items[l.from].w);
            let (y0, y1) = (at[l.from].1 + l.from_y, at[n].1 + l.to_y);
            let passed = (0..items.len()).filter(|&m| m != n && m != l.from && at[m].0 + items[m].w > at[l.from].0 + items[l.from].w && at[m].0 < at[n].0 && at[m].1 < y0.max(y1) && at[m].1 + items[m].h > y0.min(y1)).count();
            println!("   end node {:18} gap to its feeder {gap:6.0}, height apart {:6.0}, nodes passed {passed}", app.nodes[id].label, (y1 - y0).abs());
        }
    }
}

/// Every source expanded, as a project looks before anyone collapses them.
#[test]
#[ignore]
fn cleanup_lab_jiku_expanded() {
    let mut d = jiku();
    for n in &mut d.app().nodes {
        n.collapsed = false;
    }
    // A source's size follows what its file turns out to hold, read in the background.
    d.wait(15.0);
    judge(&mut d, "jiku_expanded");
}

/// The real project with more added: a Timeline fed by every source and yellow output, Time nodes
/// in front of some rows, a Result record taking everything.
#[test]
#[ignore]
fn cleanup_lab_jiku_grown() {
    let mut d = jiku();
    let app = d.app();
    let m = (0..app.nodes.len()).find(|&i| matches!(app.nodes[i].kind, Kind::AudioMatch { .. })).unwrap();
    let srcs: Vec<(usize, NodeId)> = (1..60).filter_map(|n| app.graph.feeder(m, &format!("audio {n}")).map(|(s, _)| (n, s))).collect();
    let tl = app.add(Kind::Timeline { rows: Vec::new() }, Pos2::new(-900.0, -400.0));
    let rec = app.add(Kind::ResultRecord { folder: String::new() }, Pos2::new(3000.0, -900.0));
    for &(n, s) in &srcs {
        app.wire_many(s, "audio", tl);
        app.wire(m, &format!("synced {n}"), tl, &format!("time {n}"));
        app.sync_many_ports();
        app.wire_many(s, "audio", rec);
        app.sync_many_ports();
    }
    for &(n, s) in srcs.iter().step_by(6) {
        let t = app.add(Kind::Time { rows: corroborate::modules::time::defaults() }, Pos2::new(50.0, n as f32 * 77.0));
        app.graph.disconnect(m, &format!("time {n}"));
        app.wire(s, "time", t, "time");
        app.wire(t, "result", m, &format!("time {n}"));
    }
    judge(&mut d, "jiku_grown");
}

/// A long chain with skips over it, several pieces of different sizes, many unwired nodes, one
/// source feeding many sinks and many sources into one sink.
#[test]
#[ignore]
fn cleanup_lab_shapes() {
    let mut d = Drive::new();
    let app = d.app();
    let at = |k: usize| Pos2::new((k as f32 * 733.0) % 2300.0 - 300.0, (k as f32 * 491.0) % 1500.0 - 200.0);
    let mut k = 0;
    let mut next = || {
        k += 1;
        at(k)
    };
    // A chain, with a skip from its first node to its last.
    let p = app.add(drive::place(52.0, 21.0, 300.0), next());
    let i1 = app.add(Kind::InvertLocation, next());
    let i2 = app.add(Kind::InvertLocation, next());
    let i3 = app.add(Kind::InvertLocation, next());
    let c = app.add(Kind::CombineLocations { any: false }, next());
    let map = app.add(Kind::Map, next());
    app.wire(p, "value", i1, "location");
    app.wire(i1, "location", i2, "location");
    app.wire(i2, "location", i3, "location");
    app.wire(i3, "location", c, "location 1");
    app.sync_many_ports();
    app.wire(p, "value", c, "location 2");
    app.sync_many_ports();
    app.wire(c, "location", map, "location 1");
    // One source into many sinks.
    let hub = app.add(drive::place(10.0, 10.0, 100.0), next());
    for _ in 0..7 {
        let s = app.add(Kind::InvertLocation, next());
        app.wire(hub, "value", s, "location");
    }
    // Many sources into one sink.
    let sink = app.add(Kind::Map, next());
    for n in 1..=8 {
        let s = app.add(drive::place(n as f64, 0.0, 10.0), next());
        app.wire(s, "value", sink, &format!("location {n}"));
        app.sync_many_ports();
    }
    // Unwired.
    for n in 0..11 {
        app.add(drive::place(n as f64, 1.0, 10.0), next());
    }
    judge(&mut d, "shapes");
}
