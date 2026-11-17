//! Playheads. There is one real-time playhead, in UTC (Coordinated Universal Time), shared by all
//! Timeline nodes. A recording follows it only when its start time (and, if known, its end time)
//! is wired into a Timeline: its moment t is then at start + t, or with an end,
//! t = (playhead − start) × length ÷ (end − start) (`Follow`, refreshed every frame by
//! `update_follow`). Every other recording keeps its own playhead, on its own clock, linked to
//! nothing. A view asks for the playhead on its own clock (`playhead_on`). The selected node's
//! clock has the active playhead. When a recording starts or stops following, its playhead moves
//! over at the same moment.
//!
//! Play on a SOURCER (or Space) plays and pauses: the active playhead moves in real time and
//! linked views follow. A live source's playhead stops at "now", the newest audio received, and
//! stays there while playing; paused, it falls behind while the audio keeps coming in, and Live
//! jumps back to now. A recording (a simulation) plays to its end and pauses there.

use super::*;
use std::collections::BTreeMap;

/// How a recording follows the real-time playhead: its moment `t0` is at `u0` (UTC), and it runs
/// `k` of its seconds per real second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follow {
    pub t0: f64,
    pub u0: f64,
    pub k: f64,
}

impl Follow {
    /// The moment on the recording at real time `utc`.
    pub fn t(&self, utc: f64) -> f64 {
        self.t0 + (utc - self.u0) * self.k
    }

    /// The real time of moment `t` on the recording.
    pub fn utc(&self, t: f64) -> f64 {
        self.u0 + (t - self.t0) / self.k
    }
}

#[derive(Default)]
pub struct Playheads {
    /// The real-time playhead, in UTC.
    pub utc: Option<f64>,
    /// Clock not following it → position on it.
    pub own: BTreeMap<String, f64>,
    /// Clocks following the real-time playhead, from the Timelines' wired times.
    pub follow: BTreeMap<String, Follow>,
    /// Counts the moves, so views that follow the playhead (Planes) know when it moved.
    pub rev: u64,
    pub playing: bool,
    /// When the last frame moved the playhead.
    pub last: Option<std::time::Instant>,
}

impl Playheads {
    /// Sets a playhead: the real-time one in UTC, a clock's own on that clock.
    pub fn set(&mut self, absolute: bool, clock: &str, pos: f64) {
        if absolute {
            self.utc = Some(pos);
        } else {
            self.own.insert(clock.to_string(), pos);
        }
        self.rev += 1;
    }

    /// A playhead, as `set` takes it.
    pub fn get(&self, absolute: bool, clock: &str) -> Option<f64> {
        if absolute {
            self.utc
        } else {
            self.own.get(clock).copied()
        }
    }
}

impl App {
    /// Which recordings follow the real-time playhead now: those whose start time is wired into
    /// a Timeline (the first Timeline that has it). Its start is its moment 0; an end gives its
    /// speed: its length over the time between start and end.
    pub(crate) fn follows_now(&self) -> BTreeMap<String, Follow> {
        let mut out = BTreeMap::new();
        for i in 0..self.nodes.len() {
            if self.nodes[i].hidden || !matches!(self.nodes[i].kind, Kind::Timeline { .. }) {
                continue;
            }
            for r in self.timeline_rows(i) {
                let (Some(c), Some((u0, _))) = (r.clock.clone(), r.at) else { continue };
                let k = match r.end {
                    Some((u1, _)) if r.len > 0.0 && u1 > u0 => r.len / (u1 - u0),
                    _ => 1.0,
                };
                out.entry(c).or_insert(Follow { t0: 0.0, u0, k });
            }
        }
        out
    }

    /// Refreshes which recordings follow the real-time playhead. One that starts following moves
    /// its playhead over at the same moment (the active one's wins the real-time playhead); one
    /// that stops keeps its place on its own clock.
    pub(crate) fn update_follow(&mut self) {
        let new = self.follows_now();
        if new == self.ph.follow {
            return;
        }
        let active = self.active_clock();
        let old = std::mem::take(&mut self.ph.follow);
        for (c, f) in &old {
            if !new.contains_key(c) {
                if let Some(u) = self.ph.utc {
                    self.ph.set(false, c, f.t(u));
                }
            }
        }
        let mut joined: Vec<(bool, f64, String)> = new
            .iter()
            .filter(|(c, _)| !old.contains_key(*c))
            .filter_map(|(c, f)| self.ph.own.get(c).map(|&t| (Some(c) == active.as_ref(), f.utc(t), c.clone())))
            .collect();
        joined.sort_by_key(|x| !x.0);
        for (k, (is_active, at, c)) in joined.into_iter().enumerate() {
            if k == 0 && (is_active || self.ph.utc.is_none()) {
                self.ph.set(true, "", at);
            }
            self.ph.own.remove(&c);
        }
        for c in new.keys() {
            self.ph.own.remove(c);
        }
        self.ph.follow = new;
    }

    /// The playhead in `clock`'s own time.
    pub(crate) fn playhead_on(&self, clock: &str) -> Option<f64> {
        match self.ph.follow.get(clock) {
            Some(f) => self.ph.utc.map(|u| f.t(u)),
            None => self.ph.own.get(clock).copied(),
        }
    }

    /// Moves `clock`'s playhead to `t` on that clock.
    pub(crate) fn set_playhead(&mut self, clock: &str, t: f64) {
        match self.ph.follow.get(clock).copied() {
            Some(f) => self.ph.set(true, "", f.utc(t)),
            None => self.ph.set(false, clock, t),
        }
    }

    /// Moves the real-time playhead (a click on a Timeline node).
    pub(crate) fn set_utc_playhead(&mut self, utc: f64) {
        self.ph.set(true, "", utc);
    }

    /// The clock of the selected node: its playhead is the active one.
    pub(crate) fn active_clock(&self) -> Option<String> {
        self.selected.and_then(|i| self.clock_of(i))
    }

    /// The active playhead as `Playheads` keys it: (absolute, clock); with nothing that has a
    /// clock selected, the real-time one when it has a position.
    pub(crate) fn active_key(&self) -> Option<(bool, String)> {
        match self.active_clock() {
            Some(c) if self.ph.follow.contains_key(&c) => Some((true, String::new())),
            Some(c) => Some((false, c)),
            None => self.ph.utc.is_some().then(|| (true, String::new())),
        }
    }

    /// Whether any source is live: the project is live, otherwise a simulation.
    #[cfg(test)]
    pub(crate) fn is_live(&self) -> bool {
        self.nodes.iter().any(|n| !n.hidden && n.state.lock().map(|s| s.live).unwrap_or(false))
    }

    /// "Now" for a playhead, as `Playheads` keys it: the newest audio of its live sources.
    pub(crate) fn live_now(&self, absolute: bool, clock: &str) -> Option<f64> {
        self.audio_end(absolute, clock, true)
    }

    /// The end of the audio received so far on a playhead (only live sources' with `live_only`).
    fn audio_end(&self, absolute: bool, clock: &str, live_only: bool) -> Option<f64> {
        (0..self.nodes.len())
            .filter(|&i| !self.nodes[i].hidden && self.nodes[i].kind.is_source())
            .filter_map(|i| {
                let (live, a) = self.nodes[i].state.lock().map(|s| (s.live, s.audio_s)).ok()?;
                let id = self.clock_of(i)?;
                let f = self.ph.follow.get(&id);
                let end = match (absolute, f) {
                    (true, Some(f)) => f.utc(a),
                    (false, None) if id == clock => a,
                    _ => return None,
                };
                ((live || !live_only) && a > 0.0).then_some(end)
            })
            .reduce(f64::max)
    }

    /// Moves the active playhead on by `dt` seconds when playing. Live groups' playheads stop at
    /// now and stay there while playing (following live); a recording's stops at its end and
    /// pauses.
    pub(crate) fn play_step(&mut self, dt: f64) {
        if self.ph.playing {
            if let Some((abs, g)) = self.active_key() {
                let start = self.ph.get(abs, &g).unwrap_or_else(|| if abs { self.live_now(true, "").unwrap_or(0.0) } else { 0.0 });
                let to = start + dt;
                if self.live_now(abs, &g).is_none() {
                    if let Some(end) = self.audio_end(abs, &g, false).filter(|e| to >= *e) {
                        self.ph.set(abs, &g, end);
                        self.ph.playing = false;
                        return;
                    }
                }
                self.ph.set(abs, &g, to);
            }
        }
        if let Some(u) = self.ph.utc {
            if let Some(now) = self.live_now(true, "").filter(|n| u > *n) {
                self.ph.utc = Some(now);
            }
        }
        let over: Vec<(String, f64)> = self.ph.own.iter().filter_map(|(g, &p)| self.live_now(false, g).filter(|n| p > *n).map(|n| (g.clone(), n))).collect();
        for (g, n) in over {
            self.ph.own.insert(g, n);
        }
    }

    /// Every source's clock gets a playhead as soon as it has audio: at the start of a recording,
    /// at now for a live source. So the views always show one.
    pub(crate) fn ensure_playheads(&mut self) {
        for i in 0..self.nodes.len() {
            if self.nodes[i].hidden || !self.nodes[i].kind.is_source() {
                continue;
            }
            let Some(clock) = self.clock_of(i) else { continue };
            let (live, a) = self.nodes[i].state.lock().map(|s| (s.live, s.audio_s)).unwrap_or((false, 0.0));
            if a > 0.0 && self.playhead_on(&clock).is_none() {
                self.set_playhead(&clock, if live { a } else { 0.0 });
            }
        }
    }

    /// Play or pause from a source node: its playhead becomes the active one (the node is selected).
    /// Playing a recording that is at its end starts it again from the beginning.
    pub(crate) fn play_source(&mut self, i: NodeId) {
        if self.selected != Some(i) {
            self.pick_only(i);
            if self.ph.playing {
                // Another clock was playing: this click moves the play here.
                return;
            }
        }
        if !self.ph.playing {
            if let Some(clock) = self.clock_of(i) {
                let (live, a) = self.nodes[i].state.lock().map(|s| (s.live, s.audio_s)).unwrap_or((false, 0.0));
                let at = self.playhead_on(&clock);
                if !live && at.is_none_or(|t| t >= a - 0.05) {
                    self.set_playhead(&clock, 0.0);
                }
            }
        }
        self.ph.playing = !self.ph.playing;
    }

    /// Each frame: which recordings follow the real-time playhead, Space plays or pauses (when
    /// no text box has the keys), the playhead moves on.
    pub(crate) fn playhead_tick(&mut self, ctx: &egui::Context) {
        self.update_follow();
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) && ctx.memory(|m| m.focused().is_none()) {
            self.ph.playing = !self.ph.playing;
        }
        let now = std::time::Instant::now();
        let dt = self.ph.last.map_or(0.0, |l| (now - l).as_secs_f64());
        self.ph.last = self.ph.playing.then_some(now);
        self.ensure_playheads();
        self.play_step(dt);
        if self.ph.playing {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    /// Jumps the active playhead to now (live projects).
    pub(crate) fn go_live(&mut self) {
        if let Some((abs, g)) = self.active_key().or(Some((true, String::new()))) {
            if let Some(n) = self.live_now(abs, &g) {
                self.ph.set(abs, &g, n);
            }
        }
    }

    /// Whether `clock` follows the active playhead (every clock following real time does
    /// together).
    pub(crate) fn is_active_clock(&self, clock: &str) -> bool {
        let Some(a) = self.active_clock() else { return false };
        let f = &self.ph.follow;
        (f.contains_key(&a) && f.contains_key(clock)) || a == clock
    }
}
