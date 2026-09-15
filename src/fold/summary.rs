//! Sizing a directory for the preview panel.
//!
//! A directory has no size of its own; showing one means walking it, which is
//! unbounded on a filesystem the user does not control the shape of. `Budget`
//! is the same caution `listing::read` applies to a single level, extended to
//! a whole subtree: a walk stops at `max_entries` and at `max_depth`, and
//! nothing here ever follows a symlink out of the tree it started in.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

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

/// A queued directory, with the depth it was found at so children never
/// exceed `max_depth`.
struct Pending {
    path: PathBuf,
    depth: u32,
}

/// Walk `path` and add up what is under it, within `budget`, checking
/// `cancel` as it goes so a preview nobody is looking at any more stops
/// promptly.
///
/// An explicit stack rather than recursion: a tree deep enough to matter
/// (`max_depth` is 64, but a hostile or just very old home directory can be
/// deeper) should not risk this thread's own stack. `symlink_metadata` only --
/// a directory entry that is itself a symlink is counted as a file with zero
/// bytes and never descended into, which is what keeps a loop (`a -> a`, or
/// `a/b -> a`) from being a hang: nothing here ever calls `read_dir` on a
/// path reached by following a link. The `(dev, ino)` of every directory
/// actually descended into is remembered too, in case a bind mount or a hard
/// link makes the same directory reachable twice without a symlink in sight.
pub fn summarize(path: &Path, budget: &Budget, cancel: &AtomicBool) -> DirSummary {
    let mut out = DirSummary::default();
    let mut stack = vec![Pending {
        path: path.to_path_buf(),
        depth: 0,
    }];
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut checked = 0u64;

    if let Some(id) = dir_id(path) {
        seen.insert(id);
    }

    while let Some(Pending { path: dir, depth }) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // An unreadable subdirectory was already counted as a dir by
            // whichever iteration found it; skipping it here just means its
            // own contents are not added.
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            checked += 1;
            if checked.is_multiple_of(256) && cancel.load(Ordering::Relaxed) {
                out.truncated = true;
                return out;
            }
            if out.files + out.dirs >= budget.max_entries {
                out.truncated = true;
                return out;
            }

            let entry_path = entry.path();
            let meta = match std::fs::symlink_metadata(&entry_path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let ft = meta.file_type();

            if ft.is_dir() {
                out.dirs += 1;
                if depth + 1 > budget.max_depth {
                    continue;
                }
                let id = dir_id_from(&meta);
                if let Some(id) = id {
                    if !seen.insert(id) {
                        continue;
                    }
                }
                stack.push(Pending {
                    path: entry_path,
                    depth: depth + 1,
                });
            } else {
                // Regular files, symlinks (counted with zero bytes, never
                // resolved) and anything else `stat` can name (devices,
                // sockets, FIFOs) are all "not a directory" here.
                out.files += 1;
                if ft.is_file() {
                    out.bytes += meta.len();
                }
            }
        }

        if cancel.load(Ordering::Relaxed) {
            out.truncated = true;
            return out;
        }
    }

    out
}

#[cfg(unix)]
fn dir_id(path: &Path) -> Option<(u64, u64)> {
    std::fs::symlink_metadata(path)
        .ok()
        .and_then(|m| dir_id_from(&m))
}

#[cfg(unix)]
fn dir_id_from(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt as _;
    Some((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn dir_id(_path: &Path) -> Option<(u64, u64)> {
    None
}

#[cfg(not(unix))]
fn dir_id_from(_meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::testing::Fixture;

    #[test]
    fn the_default_budget_is_generous_but_finite() {
        let b = Budget::default();
        assert_eq!(b.max_entries, 20_000);
        assert_eq!(b.max_depth, 64);
    }

    #[test]
    fn an_empty_directory_summarises_to_zeros() {
        let f = Fixture::tree();
        let s = summarize(
            &f.path("empty"),
            &Budget::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(s, DirSummary::default());
    }

    #[test]
    fn starwire_counts_its_files_and_directories() {
        let f = Fixture::tree();
        let s = summarize(
            &f.path("projects/starwire"),
            &Budget::default(),
            &AtomicBool::new(false),
        );
        // src/ (dir) + src/main.rs, Cargo.toml, README.md, .gitignore, target/ (dir)
        assert_eq!(s.dirs, 2, "src/ and target/");
        assert_eq!(s.files, 4, "main.rs, Cargo.toml, README.md, .gitignore");
        assert!(!s.truncated);
        let expected_bytes = std::fs::metadata(f.path("projects/starwire/src/main.rs"))
            .unwrap()
            .len()
            + std::fs::metadata(f.path("projects/starwire/Cargo.toml"))
                .unwrap()
                .len()
            + std::fs::metadata(f.path("projects/starwire/README.md"))
                .unwrap()
                .len()
            + std::fs::metadata(f.path("projects/starwire/.gitignore"))
                .unwrap()
                .len();
        assert_eq!(s.bytes, expected_bytes);
    }

    #[test]
    fn a_tight_budget_truncates_and_stops() {
        let f = Fixture::tree();
        let budget = Budget {
            max_entries: 2,
            max_depth: 64,
        };
        let s = summarize(
            &f.path("projects/starwire"),
            &budget,
            &AtomicBool::new(false),
        );
        assert!(s.truncated);
        assert!(s.files + s.dirs <= 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_does_not_hang() {
        let f = Fixture::tree();
        let looped = f.path("looped");
        std::fs::create_dir(&looped).unwrap();
        std::os::unix::fs::symlink(&looped, looped.join("self")).unwrap();

        let s = summarize(&looped, &Budget::default(), &AtomicBool::new(false));
        // The symlink is a file, never descended into.
        assert_eq!(s.files, 1);
        assert_eq!(s.dirs, 0);
        assert!(!s.truncated);
    }

    #[test]
    fn a_set_cancel_flag_stops_the_walk() {
        let f = Fixture::tree();
        let s = summarize(f.home(), &Budget::default(), &AtomicBool::new(true));
        assert!(s.truncated);
    }

    #[test]
    fn a_missing_directory_summarises_to_zeros() {
        let f = Fixture::tree();
        let s = summarize(
            &f.path("nowhere"),
            &Budget::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(s, DirSummary::default());
    }
}
