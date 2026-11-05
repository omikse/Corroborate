//! The timeline: lanes of timed items on one shared clock.
//!
//! An item with an absolute time (UTC) sits at its real place, with its error as a margin.
//! An item with a relative time (unknown offset) is kept on its own clock until the solver
//! places it. The display strength of an item follows its error: a narrow margin is drawn
//! strong, a wide one faded. That is a display choice, not a measurement, and the margin is
//! always drawn too.

use crate::core::{Source, TimeRef};

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    pub time: TimeRef,
    pub source: Source,
}

impl Item {
    /// The UTC window (start, end) this item may lie in, if it has an absolute time.
    pub fn window(&self) -> Option<(f64, f64)> {
        match &self.time {
            TimeRef::Absolute { utc, error_s } => Some((utc - error_s, utc + error_s)),
            TimeRef::Relative { .. } => None,
        }
    }

    /// Display strength in (0, 1]: 1 for an error of zero, halving at a 1 s error,
    /// close to zero for a wide margin. Shown as colour saturation.
    pub fn strength(&self) -> f64 {
        1.0 / (1.0 + self.time.error_s())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub name: String,
    pub items: Vec<Item>,
}

#[derive(Default, Debug)]
pub struct Timeline {
    lanes: Vec<Lane>,
}

impl Timeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lanes(&self) -> &[Lane] {
        &self.lanes
    }

    /// Add a lane and return its index.
    pub fn add_lane(&mut self, name: impl Into<String>) -> usize {
        self.lanes.push(Lane { name: name.into(), items: Vec::new() });
        self.lanes.len() - 1
    }

    /// Put an item on a lane. Returns false when the lane does not exist.
    pub fn push(&mut self, lane: usize, item: Item) -> bool {
        match self.lanes.get_mut(lane) {
            Some(l) => {
                l.items.push(item);
                true
            }
            None => false,
        }
    }

    /// Smallest window start and largest window end over all absolute items.
    pub fn span_utc(&self) -> Option<(f64, f64)> {
        self.lanes
            .iter()
            .flat_map(|l| l.items.iter())
            .filter_map(Item::window)
            .fold(None, |acc, (a, b)| match acc {
                None => Some((a, b)),
                Some((x, y)) => Some((x.min(a), y.max(b))),
            })
    }

    /// Items that are not yet placed on UTC: they wait for the solver.
    pub fn unplaced(&self) -> Vec<(usize, &Item)> {
        self.lanes
            .iter()
            .enumerate()
            .flat_map(|(i, l)| l.items.iter().map(move |it| (i, it)))
            .filter(|(_, it)| !it.time.is_absolute())
            .collect()
    }

    /// Items (from any lanes) whose UTC windows overlap: candidates for "these lined up".
    pub fn overlapping(&self) -> Vec<(&Item, &Item)> {
        let all: Vec<&Item> = self.lanes.iter().flat_map(|l| l.items.iter()).collect();
        let mut out = Vec::new();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                if let (Some((a0, a1)), Some((b0, b1))) = (a.window(), b.window()) {
                    if a0 <= b1 && b0 <= a1 {
                        out.push((*a, *b));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(label: &str, utc: f64, err: f64) -> Item {
        Item {
            label: label.into(),
            time: TimeRef::Absolute { utc, error_s: err },
            source: Source::new("test", label),
        }
    }

    fn rel(label: &str, t: f64) -> Item {
        Item {
            label: label.into(),
            time: TimeRef::Relative { t, clock: "stream A".into(), error_s: 30.0 },
            source: Source::new("test", label),
        }
    }

    #[test]
    fn absolute_items_have_windows_and_relative_ones_do_not() {
        assert_eq!(abs("pass", 100.0, 2.0).window(), Some((98.0, 102.0)));
        assert_eq!(rel("stream", 5.0).window(), None);
    }

    #[test]
    fn strength_falls_as_error_grows() {
        assert_eq!(abs("sharp", 0.0, 0.0).strength(), 1.0);
        assert!(abs("loose", 0.0, 10.0).strength() < abs("tight", 0.0, 1.0).strength());
    }

    #[test]
    fn span_covers_absolute_windows_only() {
        let mut t = Timeline::new();
        let l = t.add_lane("a");
        t.push(l, abs("x", 100.0, 5.0));
        t.push(l, abs("y", 200.0, 1.0));
        t.push(l, rel("z", 9999.0));
        assert_eq!(t.span_utc(), Some((95.0, 201.0)));
    }

    #[test]
    fn unplaced_lists_relative_items_with_their_lane() {
        let mut t = Timeline::new();
        let a = t.add_lane("planes");
        let b = t.add_lane("streams");
        t.push(a, abs("pass", 100.0, 2.0));
        t.push(b, rel("stream B", 0.0));
        let un = t.unplaced();
        assert_eq!(un.len(), 1);
        assert_eq!(un[0].0, b);
        assert_eq!(un[0].1.label, "stream B");
    }

    #[test]
    fn overlapping_finds_items_that_could_be_the_same_moment() {
        let mut t = Timeline::new();
        let a = t.add_lane("a");
        let b = t.add_lane("b");
        t.push(a, abs("bang in stream 1", 100.0, 1.0));
        t.push(b, abs("bang in stream 2", 101.5, 1.0));
        t.push(b, abs("later", 300.0, 1.0));
        let pairs = t.overlapping();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0.label, "bang in stream 1");
        assert_eq!(pairs[0].1.label, "bang in stream 2");
    }

    #[test]
    fn push_to_missing_lane_fails() {
        let mut t = Timeline::new();
        assert!(!t.push(3, abs("x", 0.0, 0.0)));
    }
}
