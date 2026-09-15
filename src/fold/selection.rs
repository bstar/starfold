//! The persistent selection.
//!
//! Marking a file does not depend on where the cursor is or which frame is
//! open: `space` marks it, and it stays marked while the fold moves elsewhere,
//! which is what lets `y`/`m` mean "copy or move the marked files *here*"
//! from any level. The set is keyed by path rather than by an index into a
//! listing for exactly that reason -- an index means nothing once the frame
//! has changed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::entry::{Entry, EntryKind};
use super::format::size;

/// What one marked path contributes to the running totals, captured at mark
/// time (or the last time `sized` measured it) rather than looked up again
/// by path every time it might unmark -- a path can vanish between being
/// marked and being forgotten, and the total still has to come out right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Marked {
    /// How many bytes this path contributes to `Selection::bytes`. `0` for a
    /// directory whose size is not known yet -- see `unsized_`.
    bytes: u64,
    /// A directory marked before `Done::Summarized` measured it. While this
    /// is `true`, the path counts against `unsized_dirs` instead of `bytes`;
    /// `Selection::sized` is what flips it.
    unsized_: bool,
}

impl Marked {
    fn file(len: u64) -> Self {
        Self {
            bytes: len,
            unsized_: false,
        }
    }

    fn unsized_dir() -> Self {
        Self {
            bytes: 0,
            unsized_: true,
        }
    }
}

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
    marked: BTreeMap<PathBuf, Marked>,
    bytes: u64,
    unsized_dirs: usize,
}

impl Selection {
    pub fn is_marked(&self, path: &Path) -> bool {
        self.marked.contains_key(path)
    }

    pub fn len(&self) -> usize {
        self.marked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marked.is_empty()
    }

    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.marked.keys().map(PathBuf::as_path)
    }

    /// Mark or unmark one entry, keeping the byte total in step.
    pub fn toggle(&mut self, entry: &Entry) {
        if let Some(m) = self.marked.remove(&entry.path) {
            self.forget_size(m);
        } else {
            let m = mark_of(entry);
            self.add_size(m);
            self.marked.insert(entry.path.clone(), m);
        }
    }

    /// Mark every entry given, skipping ones already marked so the total is
    /// not double-counted.
    pub fn mark_all(&mut self, entries: &[Entry]) {
        for entry in entries {
            if !self.marked.contains_key(&entry.path) {
                let m = mark_of(entry);
                self.add_size(m);
                self.marked.insert(entry.path.clone(), m);
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

    /// Remove every path in `gone` from the selection -- called once a move
    /// or a delete finishes, so what remains marked does not still count
    /// paths that are no longer there. Exact, because each path's own
    /// contribution was captured when it was marked rather than re-derived
    /// from a listing that may no longer have a row for it.
    pub fn forget_paths(&mut self, gone: &[PathBuf]) {
        for path in gone {
            if let Some(m) = self.marked.remove(path) {
                self.forget_size(m);
            }
        }
    }

    /// A `Done::Summarized` arrived for a marked directory: fold its byte
    /// count into the total and move it out of `unsized_dirs`. A no-op for a
    /// path that is not marked, or not a directory -- a summary racing an
    /// unmark, or one built for a plain file by mistake, changes nothing.
    /// Safe to call again for a directory that was already sized (a later
    /// refresh): the previous measurement is replaced, not added to.
    pub fn sized(&mut self, dir: &Path, bytes: u64) {
        let Some(m) = self.marked.get_mut(dir) else {
            return;
        };
        if m.unsized_ {
            self.unsized_dirs = self.unsized_dirs.saturating_sub(1);
            m.unsized_ = false;
            m.bytes = bytes;
            self.bytes += bytes;
        } else {
            self.bytes = self.bytes.saturating_sub(m.bytes) + bytes;
            m.bytes = bytes;
        }
    }

    fn add_size(&mut self, m: Marked) {
        if m.unsized_ {
            self.unsized_dirs += 1;
        } else {
            self.bytes += m.bytes;
        }
    }

    fn forget_size(&mut self, m: Marked) {
        if m.unsized_ {
            self.unsized_dirs = self.unsized_dirs.saturating_sub(1);
        } else {
            self.bytes = self.bytes.saturating_sub(m.bytes);
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

fn mark_of(entry: &Entry) -> Marked {
    if entry.kind == EntryKind::Dir {
        Marked::unsized_dir()
    } else {
        Marked::file(entry.len)
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

    #[test]
    fn forget_paths_removes_exactly_the_named_paths_and_fixes_the_totals() {
        let mut s = Selection::default();
        s.toggle(&file("/a", 100));
        s.toggle(&file("/b", 200));
        s.forget_paths(&[PathBuf::from("/a")]);
        assert!(!s.is_marked(Path::new("/a")));
        assert!(s.is_marked(Path::new("/b")));
        assert_eq!(s.bytes, 200);
    }

    #[test]
    fn forget_paths_ignores_a_path_that_was_never_marked() {
        let mut s = Selection::default();
        s.toggle(&file("/a", 100));
        s.forget_paths(&[PathBuf::from("/nowhere")]);
        assert_eq!(s.len(), 1);
        assert_eq!(s.bytes, 100);
    }

    #[test]
    fn sizing_a_marked_directory_moves_it_out_of_the_unsized_total() {
        let mut s = Selection::default();
        s.toggle(&dir("/projects"));
        assert_eq!(s.unsized_dirs, 1);
        s.sized(Path::new("/projects"), 14_200_000);
        assert_eq!(s.unsized_dirs, 0);
        assert_eq!(s.bytes, 14_200_000);
        assert_eq!(s.summary(), "1 marked \u{b7} 14.2 MB");
    }

    #[test]
    fn resizing_an_already_sized_directory_replaces_rather_than_adds() {
        let mut s = Selection::default();
        s.toggle(&dir("/projects"));
        s.sized(Path::new("/projects"), 1_000);
        s.sized(Path::new("/projects"), 2_000);
        assert_eq!(s.bytes, 2_000, "the second measurement replaced the first");
    }

    #[test]
    fn sizing_an_unmarked_directory_does_nothing() {
        let mut s = Selection::default();
        s.sized(Path::new("/projects"), 1_000);
        assert_eq!(s.bytes, 0);
        assert!(s.is_empty());
    }
}
