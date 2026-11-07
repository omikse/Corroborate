//! Linked playheads. Every absolute group shares one position (in UTC); each floating group has its
//! own (in the time of the group's reference clock). A view asks for the playhead on its own clock
//! and the solved clocks convert it, so moving it in one view moves it in every linked one.
//! Unanchored groups follow nobody. The selected node's group has the active playhead. When new
//! evidence merges or splits groups, each playhead moves to its clocks' new group at the same
//! moment; on a clash the active group's wins (evidence.rs `carry_playheads`).
//!
//! Play on a SOURCER (or Space) plays and pauses: the active playhead moves in real time and
//! linked views follow. A live source's playhead stops at "now", the newest audio received, and
//! stays there while playing; paused, it falls behind while the audio keeps coming in, and Live
//! jumps back to now. A recording (a simulation) plays to its end and pauses there.

use super::*;
use corroborate::clock::Kind as Ck;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Playheads {
    /// Where every absolute group is, in UTC.
    pub utc: Option<f64>,
    /// Floating group id → position on its reference clock.
    pub floating: BTreeMap<String, f64>,
    /// Counts the moves, so views that follow the playhead (Planes) know when it moved.
    pub rev: u64,
    pub playing: bool,
    /// When the last frame moved the playhead.
    pub last: Option<std::time::Instant>,
}

impl Playheads {
    /// Sets the playhead of a group: absolute ones in UTC, floating ones on the reference clock.
    pub fn set(&mut self, absolute: bool, group: &str, pos: f64) {
        if absolute {
            self.utc = Some(pos);
        } else {
            self.floating.insert(group.to_string(), pos);
        }
        self.rev += 1;
    }

    /// The playhead of a group, as `set` takes it.
    pub fn get(&self, absolute: bool, group: &str) -> Option<f64> {
        if absolute {
            self.utc
        } else {
            self.floating.get(group).copied()
        }
    }
}

impl App {
    /// The playhead in `clock`'s own time.
    pub(crate) fn playhead_on(&self, clock: &str) -> Option<f64> {
        let c = self.ev.clocks.get(clock);
        self.ph.get(c.kind == Ck::Absolute, &c.group).map(|p| p - c.offset)
    }

    /// Moves the playhead of `clock`'s group to `t` on that clock.
    pub(crate) fn set_playhead(&mut self, clock: &str, t: f64) {
        let c = self.ev.clocks.get(clock);
        self.ph.set(c.kind == Ck::Absolute, &c.group, t + c.offset);
    }

    /// The clock of the selected node: its group's playhead is the active one.
    pub(crate) fn active_clock(&self) -> Option<String> {
        self.selected.and_then(|i| self.clock_of(i))
    }

    /// The active group as `Playheads` keys it: (absolute, group); with nothing selected, the
    /// absolute one when it has a playhead.
    pub(crate) fn active_key(&self) -> Option<(bool, String)> {
        match self.active_clock() {
            Some(c) => {
                let i = self.ev.clocks.get(&c);
                Some((i.kind == Ck::Absolute, i.group))
            }
            None => self.ph.utc.is_some().then(|| (true, String::new())),
        }
    }

    /// Whether any source is live: the project is live, otherwise a simulation.
    #[cfg(test)]
    pub(crate) fn is_live(&self) -> bool {
        self.nodes.iter().any(|n| !n.hidden && n.state.lock().map(|s| s.live).unwrap_or(false))
    }

    /// "Now" for a group, as `Playheads` keys it: the newest audio of its live sources.
    pub(crate) fn live_now(&self, absolute: bool, group: &str) -> Option<f64> {
        self.audio_end(absolute, group, true)
    }

    /// The end of the audio received so far in a group (only live sources' with `live_only`).
    fn audio_end(&self, absolute: bool, group: &str, live_only: bool) -> Option<f64> {
        (0..self.nodes.len())
            .filter(|&i| !self.nodes[i].hidden && self.nodes[i].kind.is_source())
            .filter_map(|i| {
                let (live, a) = self.nodes[i].state.lock().map(|s| (s.live, s.audio_s)).ok()?;
                let c = self.ev.clocks.get(&self.clock_of(i)?);
                let same = if absolute { c.kind == Ck::Absolute } else { c.kind != Ck::Absolute && c.group == group };
                ((live || !live_only) && same && a > 0.0).then_some(a + c.offset)
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
        let over: Vec<(String, f64)> = self.ph.floating.iter().filter_map(|(g, &p)| self.live_now(false, g).filter(|n| p > *n).map(|n| (g.clone(), n))).collect();
        for (g, n) in over {
            self.ph.floating.insert(g, n);
        }
    }

    /// Every source's group gets a playhead as soon as it has audio: at the start of a recording,
    /// at now for a live source. So the timeline and the views always show one.
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

    /// Play or pause from a source node: its group becomes the active one (the node is selected).
    /// Playing a recording that is at its end starts it again from the beginning.
    pub(crate) fn play_source(&mut self, i: NodeId) {
        if self.selected != Some(i) {
            self.pick_only(i);
            if self.ph.playing {
                // Another group was playing: this click moves the play here.
                self.follow = true;
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
            self.follow = true;
        }
        self.ph.playing = !self.ph.playing;
    }

    /// Each frame: Space plays or pauses (when no text box has the keys), the playhead moves on.
    pub(crate) fn playhead_tick(&mut self, ctx: &egui::Context) {
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

    /// Jumps the active group's playhead to now (live projects).
    pub(crate) fn go_live(&mut self) {
        if let Some((abs, g)) = self.active_key().or(Some((true, String::new()))) {
            if let Some(n) = self.live_now(abs, &g) {
                self.ph.set(abs, &g, n);
            }
        }
    }

    /// Whether `clock` is in the active playhead's group (every absolute clock is one group).
    pub(crate) fn is_active_clock(&self, clock: &str) -> bool {
        let Some(a) = self.active_clock() else { return false };
        let (x, y) = (self.ev.clocks.get(&a), self.ev.clocks.get(clock));
        (x.kind == Ck::Absolute && y.kind == Ck::Absolute) || x.group == y.group
    }
}
