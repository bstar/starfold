//! Reading one directory.
//!
//! A [`Listing`] is the whole of what `read_dir` found, sorted by nobody:
//! ordering is [`super::sort::order`]'s job, run over the same entries as many
//! times as the view changes without reading the directory again. A
//! permission error becomes [`Listing::error`] rather than a `Result`, because
//! the frame still has to draw *something* for the level the user is on --
//! an empty panel with no explanation is worse than one that says why.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::entry::Entry;
use super::sort::SortOrder;

/// What `read` is told about how big a directory it may return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListConfig {
    /// A directory bigger than this is truncated rather than read whole --
    /// `/proc`, a build's `target/`, a photo library -- because a `Vec` of
    /// every entry in one of those is a multi-second pause and a panel that
    /// cannot scroll to the bottom of it faster than the eye moves anyway.
    pub max_entries: usize,
    /// Where a freshly opened frame starts, before the user has touched `s`,
    /// `S` or `.` -- carried here rather than read from `Config` a second
    /// time in `state::apply`.
    pub sort: SortOrder,
    pub show_hidden: bool,
}

impl Default for ListConfig {
    fn default() -> Self {
        Self {
            max_entries: 50_000,
            sort: SortOrder::default(),
            show_hidden: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ListError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// One directory's contents, as read once.
#[derive(Debug, Clone)]
pub struct Listing {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    /// `true` when `max_entries` cut the read short. The panel says so; it
    /// does not pretend the directory has fewer files than it does.
    pub truncated: bool,
    /// Set when the directory could not be read at all -- permission denied,
    /// most often. The entries are then empty rather than absent, so a panel
    /// that draws `listing.entries` needs no separate case for "no listing".
    pub error: Option<String>,
    /// The directory's own modification time, read once alongside the
    /// listing rather than re-stat-ed by the watcher on every poll.
    /// `watch.rs` compares this against a fresh `stat` to notice that a
    /// level changed underneath the stack; `None` when the directory itself
    /// could not be stat-ed (which also means `error` is set).
    pub dir_mtime: Option<SystemTime>,
}

impl Listing {
    pub fn error(dir: &Path, message: impl Into<String>) -> Self {
        Self {
            dir: dir.to_path_buf(),
            entries: Vec::new(),
            truncated: false,
            error: Some(message.into()),
            dir_mtime: None,
        }
    }
}

/// Read a directory. Never fails: whatever went wrong is carried in
/// [`Listing::error`] instead, because the caller always needs a `Listing` to
/// put in `State::listings` and a `Result` would only be unwrapped straight
/// back into one.
///
/// Ordering is left exactly as `read_dir` produced it -- [`super::sort::order`]
/// is what a panel actually draws, and sorting here would just be thrown away
/// the first time the view changes.
pub fn read(dir: &Path, cfg: &ListConfig) -> Listing {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(err) => return Listing::error(dir, describe(&err)),
    };

    // Read after `read_dir` succeeds rather than before: a directory that
    // cannot be opened has no useful mtime to report either, and
    // `Listing::error` already carries `None`.
    let dir_mtime = std::fs::metadata(dir).ok().and_then(|m| m.modified().ok());

    let mut entries = Vec::new();
    let mut truncated = false;
    for item in read_dir {
        let Ok(item) = item else {
            // One name in the middle of the walk raced a delete or turned
            // out unreadable; the directory as a whole is still good, so the
            // row is dropped rather than the entire read aborted.
            continue;
        };
        if entries.len() >= cfg.max_entries {
            truncated = true;
            break;
        }
        entries.push(super::entry::stat(&item.path()));
    }

    Listing {
        dir: dir.to_path_buf(),
        entries,
        truncated,
        error: None,
        dir_mtime,
    }
}

/// A short, human reason `read_dir` failed. `io::Error`'s own `Display`
/// spells the path and the OS errno text, which is too long for a panel row;
/// this picks out the handful of cases anyone opening a directory actually
/// hits and only falls back to the raw error for the rest.
fn describe(err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        std::io::ErrorKind::NotADirectory => "not a directory".to_string(),
        std::io::ErrorKind::NotFound => "no such directory".to_string(),
        _ => format!("cannot read directory: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::entry::EntryKind;
    use crate::fold::testing::Fixture;

    #[test]
    fn a_default_config_has_a_generous_but_finite_ceiling() {
        assert_eq!(ListConfig::default().max_entries, 50_000);
    }

    #[test]
    fn an_error_listing_has_no_entries_and_carries_its_reason() {
        let l = Listing::error(Path::new("/root/secret"), "permission denied");
        assert!(l.entries.is_empty());
        assert_eq!(l.error.as_deref(), Some("permission denied"));
        assert!(l.dir_mtime.is_none());
    }

    #[test]
    fn reading_the_fixture_home_finds_every_top_level_entry_with_the_right_kind() {
        let f = Fixture::tree();
        let l = read(f.home(), &ListConfig::default());
        assert!(l.error.is_none());

        let kind_of = |name: &str| l.entries.iter().find(|e| e.display == name).map(|e| e.kind);
        assert_eq!(kind_of("projects"), Some(EntryKind::Dir));
        assert_eq!(kind_of("pictures"), Some(EntryKind::Dir));
        assert_eq!(kind_of("blob.bin"), Some(EntryKind::File));
        assert_eq!(kind_of("empty"), Some(EntryKind::Dir));
        #[cfg(unix)]
        {
            assert_eq!(kind_of("notes.txt"), Some(EntryKind::Symlink));
            assert_eq!(kind_of("dangling"), Some(EntryKind::Symlink));
        }
    }

    #[test]
    fn a_hidden_dotfile_is_present_in_entries_filtering_is_sorts_job() {
        let f = Fixture::tree();
        let l = crate::fold::testing::listing(&f, "projects/starwire");
        assert!(
            l.entries.iter().any(|e| e.display == ".gitignore"),
            "read never drops the hidden entries; sort::order does that"
        );
    }

    #[test]
    fn max_entries_truncates_a_big_directory_and_flags_it() {
        let f = Fixture::tree();
        let cfg = ListConfig {
            max_entries: 3,
            ..ListConfig::default()
        };
        let l = read(f.home(), &cfg);
        assert_eq!(l.entries.len(), 3);
        assert!(l.truncated);
    }

    #[test]
    fn a_directory_within_the_limit_is_not_flagged_truncated() {
        let f = Fixture::tree();
        let l = read(&f.path("empty"), &ListConfig::default());
        assert!(l.entries.is_empty());
        assert!(!l.truncated);
    }

    #[test]
    fn a_missing_path_yields_an_error_listing() {
        let f = Fixture::tree();
        let l = read(&f.path("nowhere-at-all"), &ListConfig::default());
        assert!(l.entries.is_empty());
        assert!(l.error.is_some());
    }

    #[test]
    fn a_file_path_is_reported_as_not_a_directory() {
        let f = Fixture::tree();
        let l = read(&f.path("blob.bin"), &ListConfig::default());
        assert!(l.entries.is_empty());
        assert_eq!(l.error.as_deref(), Some("not a directory"));
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_with_no_permissions_becomes_an_error_listing_with_no_entries() {
        if running_as_root() {
            eprintln!("skipping: root ignores a directory's own permission bits");
            return;
        }
        use std::os::unix::fs::PermissionsExt;

        let f = Fixture::tree();
        let locked = f.path("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let l = read(&locked, &ListConfig::default());

        // Restore access before the fixture's `TempDir` tries to remove it.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(l.entries.is_empty());
        assert_eq!(l.error.as_deref(), Some("permission denied"));
    }

    #[test]
    fn dir_mtime_is_set_for_a_readable_directory() {
        let f = Fixture::tree();
        let l = read(f.home(), &ListConfig::default());
        assert!(l.dir_mtime.is_some());
    }

    /// A heuristic rather than a real `geteuid()` check: this crate has no
    /// `libc` dependency (see the plan's "Not added" list), and root ignores
    /// a directory's own mode bits, which would otherwise fail the
    /// no-permissions test above under a root-run CI container.
    #[cfg(unix)]
    fn running_as_root() -> bool {
        std::env::var_os("USER").as_deref() == Some(std::ffi::OsStr::new("root"))
            || std::env::var_os("LOGNAME").as_deref() == Some(std::ffi::OsStr::new("root"))
    }
}
