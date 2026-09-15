//! How far a running operation has got.
//!
//! Atomics rather than a value behind the state lock: the worker thread
//! updates this after every file it copies, and taking the write lock that
//! often -- for numbers the UI only reads once a frame -- would serialise the
//! copy against the thread drawing the status bar for no reason. `bar` and
//! `line` are pure formatting over whatever the atomics say at the moment
//! they are called.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Shared between the op that is running and whoever is drawing its status.
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
    cancelled: AtomicBool,
}

impl Progress {
    pub fn new(total: u64) -> Self {
        Self {
            done: AtomicU64::new(0),
            total: AtomicU64::new(total),
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    /// Add to the done count -- bytes copied, most often, or files for an
    /// operation that does not know its byte total until it has planned.
    pub fn add(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }

    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// `0.0` when nothing is known yet, so a status bar drawn before the plan
    /// finishes shows an empty rather than a full one.
    pub fn fraction(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        (self.done() as f64 / total as f64).min(1.0)
    }

    /// `████░░ 78%`, `width` characters of bar plus the percentage.
    pub fn bar(&self, width: usize) -> String {
        let fraction = self.fraction();
        let filled = ((fraction * width as f64) as usize).min(width);
        let mut s = String::with_capacity(width + 5);
        for _ in 0..filled {
            s.push('\u{2588}');
        }
        for _ in filled..width {
            s.push('\u{2591}');
        }
        s.push_str(&format!(" {}%", (fraction * 100.0).round() as u32));
        s
    }

    /// `COPYING ████████░░ 78%`, the line the status row shows while an
    /// operation is running.
    pub fn line(&self, verb: &str) -> String {
        format!("{verb} {}", self.bar(10))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fraction_is_zero_with_nothing_known() {
        let p = Progress::new(0);
        assert_eq!(p.fraction(), 0.0);
    }

    #[test]
    fn the_fraction_tracks_done_against_total() {
        let p = Progress::new(100);
        p.add(78);
        assert!((p.fraction() - 0.78).abs() < f64::EPSILON);
    }

    #[test]
    fn the_bar_matches_the_worked_example() {
        let p = Progress::new(100);
        p.add(78);
        assert_eq!(
            p.bar(6),
            "\u{2588}\u{2588}\u{2588}\u{2588}\u{2591}\u{2591} 78%"
        );
    }

    #[test]
    fn a_full_bar_has_no_empty_cells() {
        let p = Progress::new(10);
        p.add(10);
        assert_eq!(p.bar(4), "\u{2588}\u{2588}\u{2588}\u{2588} 100%");
    }

    #[test]
    fn the_line_names_the_verb_and_shows_the_bar() {
        let p = Progress::new(100);
        p.add(78);
        assert_eq!(
            p.line("COPYING"),
            "COPYING \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2591}\u{2591}\u{2591} 78%"
        );
    }

    #[test]
    fn cancelling_is_visible_to_every_holder() {
        let p = Progress::new(10);
        assert!(!p.is_cancelled());
        p.cancel();
        assert!(p.is_cancelled());
    }
}
