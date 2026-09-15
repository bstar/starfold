//! Sizing a directory for the preview panel.
//!
//! A directory has no size of its own; showing one means walking it, which is
//! unbounded on a filesystem the user does not control the shape of. `Budget`
//! is the same caution `listing::read` applies to a single level, extended to
//! a whole subtree: a walk stops at `max_entries` and at `max_depth`, and
//! nothing here ever follows a symlink out of the tree it started in.

use std::path::Path;
use std::sync::atomic::AtomicBool;

/// What a directory preview shows: how much is in it, and whether the walk
/// that found out had to give up early.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirSummary {
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
    pub truncated: bool,
}

/// How far `summarize` is allowed to walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub max_entries: usize,
    /// Symlink loops are not the only way a walk runs long; a bind mount or a
    /// deliberately deep tree needs a hard floor under it too.
    pub max_depth: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_entries: 20_000,
            max_depth: 64,
        }
    }
}

/// Walk `path` and add up what is under it, within `budget`, checking
/// `cancel` as it goes so a preview nobody is looking at any more stops
/// promptly.
///
/// `// TODO(1d)`: this is the bootstrap stub. It reports an empty, untruncated
/// summary for every directory; the real walk -- depth-limited, symlink-safe,
/// checking `cancel` between entries -- is Phase 1d's.
pub fn summarize(path: &Path, budget: &Budget, cancel: &AtomicBool) -> DirSummary {
    let _ = (path, budget, cancel);
    DirSummary::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_budget_is_generous_but_finite() {
        let b = Budget::default();
        assert_eq!(b.max_entries, 20_000);
        assert_eq!(b.max_depth, 64);
    }

    #[test]
    fn the_stub_reports_nothing_yet() {
        let dir = tempfile::tempdir().unwrap();
        let s = summarize(dir.path(), &Budget::default(), &AtomicBool::new(false));
        assert_eq!(s, DirSummary::default());
    }
}
