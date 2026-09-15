//! The persistent selection.
//!
//! Marking a file does not depend on where the cursor is or which frame is
//! open: `space` marks it, and it stays marked while the fold moves elsewhere,
//! which is what lets `y`/`m` mean "copy or move the marked files *here*"
//! from any level. The set is paths rather than indices into a listing for
//! exactly that reason -- an index means nothing once the frame has changed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::entry::{Entry, EntryKind};
use super::format::size;

/// What is marked, and the running total the status row shows beside it.
///
/// The byte total is kept incrementally rather than summed on every draw,
/// because summing it means knowing the size of every marked directory, and
/// that is the budgeted walk [`super::summary::summarize`] exists to avoid
/// running on every frame. A marked directory instead bumps
/// [`Selection::unsized_dirs`], and the status row's `+` says the total is a
/// lower bound until it is measured.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    marked: BTreeSet<PathBuf>,
    bytes: u64,
    unsized_dirs: usize,
}

impl Selection {
    pub fn is_marked(&self, path: &Path) -> bool {
        self.marked.contains(path)
    }

    pub fn len(&self) -> usize {
        self.marked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marked.is_empty()
    }

    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.marked.iter().map(PathBuf::as_path)
    }

    /// Mark or unmark one entry, keeping the byte total in step.
    pub fn toggle(&mut self, entry: &Entry) {
        if self.marked.remove(&entry.path) {
            self.forget_size(entry);
        } else {
            self.marked.insert(entry.path.clone());
            self.add_size(entry);
        }
    }

    /// Mark every entry given, skipping ones already marked so the total is
    /// not double-counted.
    pub fn mark_all(&mut self, entries: &[Entry]) {
        for entry in entries {
            if self.marked.insert(entry.path.clone()) {
                self.add_size(entry);
            }
        }
    }

    /// Flip every entry given: marked becomes unmarked and back again.
    pub fn invert(&mut self, entries: &[Entry]) {
        for entry in entries {
            self.toggle(entry);
        }
    }

    /// Unmark everything, everywhere -- not just the entries on screen.
    pub fn forget(&mut self) {
        self.marked.clear();
        self.bytes = 0;
        self.unsized_dirs = 0;
    }

    fn add_size(&mut self, entry: &Entry) {
        if entry.kind == EntryKind::Dir {
            self.unsized_dirs += 1;
        } else {
            self.bytes += entry.len;
        }
    }

    fn forget_size(&mut self, entry: &Entry) {
        if entry.kind == EntryKind::Dir {
            self.unsized_dirs = self.unsized_dirs.saturating_sub(1);
        } else {
            self.bytes = self.bytes.saturating_sub(entry.len);
        }
    }

    /// The status row's summary, e.g. `2 marked · 14.2 MB` or, while a marked
    /// directory's size is not yet known, `14.2 MB+`. Empty when nothing is
    /// marked, so the status row can drop the field rather than show a zero.
    pub fn summary(&self) -> String {
        if self.marked.is_empty() {
            return String::new();
        }
        let plus = if self.unsized_dirs > 0 { "+" } else { "" };
        format!(
            "{} marked \u{b7} {}{plus}",
            self.marked.len(),
            size(self.bytes)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, len: u64) -> Entry {
        Entry {
            path: path.into(),
            display: path.to_string(),
            kind: EntryKind::File,
            link_kind: None,
            len,
            modified: None,
            mode: 0,
            executable: false,
            hidden: false,
        }
    }

    fn dir(path: &str) -> Entry {
        Entry {
            kind: EntryKind::Dir,
            ..file(path, 0)
        }
    }

    #[test]
    fn toggling_marks_and_unmarks() {
        let mut s = Selection::default();
        let a = file("/a", 10);
        s.toggle(&a);
        assert!(s.is_marked(&a.path));
        assert_eq!(s.len(), 1);
        s.toggle(&a);
        assert!(!s.is_marked(&a.path));
        assert!(s.is_empty());
    }

    #[test]
    fn the_byte_total_follows_marks_and_unmarks() {
        let mut s = Selection::default();
        let a = file("/a", 1000);
        let b = file("/b", 2000);
        s.toggle(&a);
        s.toggle(&b);
        assert_eq!(s.bytes, 3000);
        s.toggle(&a);
        assert_eq!(s.bytes, 2000);
    }

    #[test]
    fn marking_a_directory_does_not_size_it() {
        let mut s = Selection::default();
        s.toggle(&dir("/projects"));
        assert_eq!(s.bytes, 0);
        assert_eq!(s.unsized_dirs, 1);
    }

    #[test]
    fn mark_all_skips_what_is_already_marked() {
        let mut s = Selection::default();
        let entries = vec![file("/a", 5), file("/b", 5)];
        s.toggle(&entries[0]);
        s.mark_all(&entries);
        assert_eq!(s.len(), 2);
        assert_eq!(s.bytes, 10, "the already-marked file was not counted twice");
    }

    #[test]
    fn invert_flips_every_entry_given() {
        let mut s = Selection::default();
        let entries = vec![file("/a", 1), file("/b", 1), file("/c", 1)];
        s.toggle(&entries[0]);
        s.invert(&entries);
        assert!(!s.is_marked(&entries[0].path));
        assert!(s.is_marked(&entries[1].path));
        assert!(s.is_marked(&entries[2].path));
    }

    #[test]
    fn forget_clears_everything_including_the_totals() {
        let mut s = Selection::default();
        s.toggle(&file("/a", 500));
        s.toggle(&dir("/b"));
        s.forget();
        assert!(s.is_empty());
        assert_eq!(s.bytes, 0);
        assert_eq!(s.unsized_dirs, 0);
        assert_eq!(s.summary(), "");
    }

    #[test]
    fn the_summary_marks_an_unsized_total_with_a_plus() {
        let mut s = Selection::default();
        s.toggle(&file("/a.txt", 14_200_000));
        assert_eq!(s.summary(), "1 marked \u{b7} 14.2 MB");
        s.toggle(&dir("/projects"));
        assert_eq!(s.summary(), "2 marked \u{b7} 14.2 MB+");
    }
}
