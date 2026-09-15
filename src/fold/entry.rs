//! One entry in a directory listing.
//!
//! `stat` reads a path exactly once, through `symlink_metadata`, so a listing
//! never silently follows a link into a loop or off onto a slower filesystem.
//! A symlink's *own* kind is always [`EntryKind::Symlink`]; what it points at
//! is a second, optional lookup, because a broken link is common enough in a
//! real home directory that failing to draw a row for it would be a bug.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What kind of thing a path is, without following a symlink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    /// A device node, a socket, a FIFO -- something `stat` can name but a file
    /// manager has no business opening.
    Other,
}

/// One row of a listing.
///
/// Everything a panel needs to draw a row and everything `sort` needs to order
/// one, read once at listing time rather than re-derived from `path` on every
/// frame: a 50,000-entry directory re-running `file_name()` and a lossy
/// conversion every redraw is the difference between a list that scrolls and
/// one that does not.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    /// The file name, lossily converted for drawing. `path` keeps the exact
    /// bytes; this is only ever shown, never opened.
    pub display: String,
    pub kind: EntryKind,
    /// What a symlink resolves to, one hop through `metadata`. `None` for
    /// anything that is not a symlink, and also for a broken one -- the two
    /// are told apart by `kind`, not by this field, since a row still has to
    /// draw for a link whose target is gone.
    pub link_kind: Option<EntryKind>,
    /// Zero for a directory: nothing here sizes one eagerly, because doing so
    /// is exactly the walk `summary::summarize` exists to budget.
    pub len: u64,
    pub modified: Option<SystemTime>,
    /// The raw Unix mode bits, for the permissions column. `0` on a platform
    /// or filesystem that does not report them.
    pub mode: u32,
    pub executable: bool,
    /// Whether the display name starts with `.`, cached rather than
    /// recomputed every time a listing is filtered.
    pub hidden: bool,
}

impl Entry {
    /// The extension, without the dot, lowercased: `"toml"` for both
    /// `Cargo.toml` and `CARGO.TOML`, `""` when there is none.
    /// `Path::extension` already treats a dotfile's leading dot as part of
    /// the stem rather than a separator, so `.gitignore` comes back with no
    /// extension the same way `Path::new(".gitignore").extension()` does.
    ///
    /// Returns an owned `String` rather than a `&str` borrowed from
    /// `display`: `Entry` has no field to cache a lowercased copy in without
    /// breaking every other place in this crate that builds one with a
    /// struct literal (`selection.rs`'s test fixtures, outside this unit's
    /// files, among them), and a lowercased slice cannot borrow from a
    /// differently-cased original without allocating somewhere. `sort`
    /// pays that allocation once per comparison rather than storing it.
    pub fn ext(&self) -> String {
        Path::new(&self.display)
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default()
    }

    /// Whether stepping into this row would take you into a directory: it
    /// is one, or it is a symlink whose target is one. `sort::order` uses
    /// this to decide what "directories first" means, because someone
    /// drilling down with `l` cares where the row takes them, not the row's
    /// own type -- a symlink to a directory belongs with the directories.
    pub fn is_dir_like(&self) -> bool {
        matches!(self.kind, EntryKind::Dir)
            || matches!(self.kind, EntryKind::Symlink if self.link_kind == Some(EntryKind::Dir))
    }
}

fn kind_of(ft: std::fs::FileType) -> EntryKind {
    if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

#[cfg(unix)]
fn mode_of(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    meta.permissions().mode()
}

#[cfg(not(unix))]
fn mode_of(_meta: &std::fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
fn executable_of(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable_of(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Read one path. Never fails: a path that has vanished or cannot be read
/// comes back as an [`EntryKind::Other`] with everything else at its zero
/// value, because a listing draws a row for every name `read_dir` handed it
/// and a stat that raced a delete is not a reason to drop the row silently.
pub fn stat(path: &Path) -> Entry {
    let name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| path.as_os_str().to_os_string());
    let display = name.to_string_lossy().into_owned();
    let hidden = display.starts_with('.');

    let meta = std::fs::symlink_metadata(path);
    let (kind, mode, len, modified, executable) = match &meta {
        Ok(m) => (
            kind_of(m.file_type()),
            mode_of(m),
            m.len(),
            m.modified().ok(),
            executable_of(m),
        ),
        Err(_) => (EntryKind::Other, 0, 0, None, false),
    };

    let link_kind = (kind == EntryKind::Symlink)
        .then(|| std::fs::metadata(path).ok().map(|m| kind_of(m.file_type())))
        .flatten();

    Entry {
        path: path.to_path_buf(),
        display,
        kind,
        link_kind,
        len,
        modified,
        mode,
        executable,
        hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_file_is_stat_ed_as_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readme.txt");
        std::fs::write(&path, b"hello").unwrap();

        let e = stat(&path);
        assert_eq!(e.kind, EntryKind::File);
        assert_eq!(e.len, 5);
        assert_eq!(e.display, "readme.txt");
        assert!(!e.hidden);
        assert!(e.link_kind.is_none());
    }

    #[test]
    fn a_dot_file_is_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        std::fs::write(&path, b"").unwrap();
        assert!(stat(&path).hidden);
    }

    #[test]
    fn a_directory_is_read_as_a_directory() {
        // `len` is whatever the filesystem reports for the directory inode
        // itself -- 0 on some, a block size on others (APFS) -- and is never
        // read as a byte count for a directory's contents, so nothing here
        // asserts a value for it.
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(stat(&sub).kind, EntryKind::Dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinks_own_kind_is_symlink_and_its_target_is_resolved_separately() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, b"x").unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let e = stat(&link);
        assert_eq!(e.kind, EntryKind::Symlink);
        assert_eq!(e.link_kind, Some(EntryKind::File));
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_symlink_still_gets_a_row() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("broken.txt");
        std::os::unix::fs::symlink(dir.path().join("nowhere"), &link).unwrap();

        let e = stat(&link);
        assert_eq!(e.kind, EntryKind::Symlink);
        assert_eq!(e.link_kind, None, "a broken target resolves to nothing");
    }

    #[cfg(unix)]
    #[test]
    fn an_executable_bit_is_read_from_the_mode() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.sh");
        std::fs::write(&path, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(stat(&path).executable);
    }

    #[test]
    fn a_path_that_does_not_exist_still_produces_a_row() {
        let dir = tempfile::tempdir().unwrap();
        let e = stat(&dir.path().join("nothing-here"));
        assert_eq!(e.kind, EntryKind::Other);
        assert_eq!(e.len, 0);
    }

    #[test]
    fn ext_is_lowercased_and_has_no_dot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("REPORT.PDF");
        std::fs::write(&path, b"x").unwrap();
        assert_eq!(stat(&path).ext(), "pdf");
    }

    #[test]
    fn a_dotfile_has_no_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(stat(&path).ext(), "");
    }

    #[test]
    fn a_name_with_no_dot_has_no_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("README");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(stat(&path).ext(), "");
    }

    #[test]
    fn a_directory_is_dir_like() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert!(stat(&sub).is_dir_like());
    }

    #[test]
    fn a_plain_file_is_not_dir_like() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.txt");
        std::fs::write(&path, b"x").unwrap();
        assert!(!stat(&path).is_dir_like());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_directory_is_dir_like() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&sub, &link).unwrap();
        assert!(stat(&link).is_dir_like());
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_symlink_is_not_dir_like() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("broken");
        std::os::unix::fs::symlink(dir.path().join("nowhere"), &link).unwrap();
        assert!(!stat(&link).is_dir_like());
    }
}
