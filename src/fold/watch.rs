//! Noticing that a directory changed behind the program's back.
//!
//! Polled by `worker::spawn_io` every [`super::worker::POLL`] rather than
//! watched through a filesystem-notification API: a couple of directories at
//! a time, and this program already re-lists after its own operations, is
//! not worth a `notify` dependency for. `Watch` itself does no polling and
//! knows nothing about a schedule -- it is just "remember what these
//! directories' mtimes were" and "which of them moved since", called once a
//! tick.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What `watch` remembers about each directory it was told to follow: `None`
/// when the directory could not be stat-ed, which is also what a directory
/// that has since been removed settles to, so a vanished directory is
/// reported exactly once rather than every tick after.
#[derive(Debug, Default)]
pub struct Watch {
    seen: HashMap<PathBuf, Option<SystemTime>>,
}

impl Watch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the watched set with `dirs`.
    ///
    /// A directory already being watched keeps the mtime remembered for it,
    /// so a level that has not moved since the last tick does not show up as
    /// "changed" just because the stack pushed a new frame beside it. A
    /// directory watched for the first time gets its mtime read now, without
    /// being reported: `changed` only ever compares against a *previous*
    /// call's answer, and there is not one yet for a directory that just
    /// joined the set.
    pub fn watch(&mut self, dirs: &[PathBuf]) {
        let mut next = HashMap::with_capacity(dirs.len());
        for dir in dirs {
            let mtime = match self.seen.get(dir) {
                Some(known) => *known,
                None => mtime_of(dir),
            };
            next.insert(dir.clone(), mtime);
        }
        self.seen = next;
    }

    /// The watched directories whose mtime moved, or which vanished, since
    /// the last call to `watch` or `changed` -- updating the remembered
    /// value as it goes, so the next call reports only what changed after
    /// this one.
    pub fn changed(&mut self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for (dir, last) in self.seen.iter_mut() {
            let now = mtime_of(dir);
            if now != *last {
                out.push(dir.clone());
                *last = now;
            }
        }
        out
    }
}

fn mtime_of(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).ok()?.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Sets a directory's own mtime forward explicitly, rather than sleeping
    /// past the filesystem's mtime resolution, so the test is not flaky on a
    /// filesystem with coarse (one-second, on some) granularity.
    fn bump_mtime(dir: &Path) {
        let file = std::fs::File::open(dir).expect("opening the directory to bump its mtime");
        let later = SystemTime::now() + Duration::from_secs(120);
        let times = std::fs::FileTimes::new().set_modified(later);
        file.set_times(times)
            .expect("setting the directory's mtime");
    }

    #[test]
    fn watching_a_directory_for_the_first_time_is_not_itself_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Watch::new();
        w.watch(&[dir.path().to_path_buf()]);
        assert!(w.changed().is_empty());
    }

    #[test]
    fn creating_a_file_moves_the_watched_directorys_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Watch::new();
        w.watch(&[dir.path().to_path_buf()]);
        assert!(w.changed().is_empty(), "nothing has happened yet");

        std::fs::write(dir.path().join("new.txt"), b"x").unwrap();
        bump_mtime(dir.path());

        assert_eq!(w.changed(), vec![dir.path().to_path_buf()]);
    }

    #[test]
    fn a_directory_nothing_touched_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Watch::new();
        w.watch(&[dir.path().to_path_buf()]);
        w.changed();
        assert!(
            w.changed().is_empty(),
            "a second call with nothing new must report nothing"
        );
    }

    #[test]
    fn a_removed_directory_is_reported_once_and_then_settles() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();

        let mut w = Watch::new();
        w.watch(std::slice::from_ref(&sub));
        std::fs::remove_dir(&sub).unwrap();

        assert_eq!(
            w.changed(),
            vec![sub.clone()],
            "the removal is noticed once"
        );
        assert!(
            w.changed().is_empty(),
            "gone stays gone; it does not keep reporting"
        );
    }

    #[test]
    fn watching_a_new_set_keeps_the_mtime_already_known_for_a_directory_still_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();

        let mut w = Watch::new();
        w.watch(&[dir.path().to_path_buf()]);
        std::fs::write(dir.path().join("new.txt"), b"x").unwrap();
        bump_mtime(dir.path());

        // Re-watching with an extra directory alongside it must not forget
        // that `dir` already moved -- only a fresh call to `changed` should
        // clear that.
        w.watch(&[dir.path().to_path_buf(), other.path().to_path_buf()]);
        assert_eq!(w.changed(), vec![dir.path().to_path_buf()]);
    }
}
