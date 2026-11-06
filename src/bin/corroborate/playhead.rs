//! Linked playheads. Every absolute group shares one position (in UTC); each floating group has its
//! own (in the time of the group's reference clock). A view asks for the playhead on its own clock
//! and the solved clocks convert it, so moving it in one view moves it in every linked one.
//! Unanchored groups follow nobody. The selected node's group has the active playhead.

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

    /// Whether `clock` is in the active playhead's group (every absolute clock is one group).
    pub(crate) fn is_active_clock(&self, clock: &str) -> bool {
        let Some(a) = self.active_clock() else { return false };
        let (x, y) = (self.ev.clocks.get(&a), self.ev.clocks.get(clock));
        (x.kind == Ck::Absolute && y.kind == Ck::Absolute) || x.group == y.group
    }
}
