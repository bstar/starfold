//! Reading one directory.
//!
//! A [`Listing`] is the whole of what `read_dir` found, sorted by nobody:
//! ordering is [`super::sort::order`]'s job, run over the same entries as many
//! times as the view changes without reading the directory again. A
//! permission error becomes [`Listing::error`] rather than a `Result`, because
//! the frame still has to draw *something* for the level the user is on --
//! an empty panel with no explanation is worse than one that says why.

use std::path::{Path, PathBuf};

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
}

impl Listing {
    pub fn error(dir: &Path, message: impl Into<String>) -> Self {
        Self {
            dir: dir.to_path_buf(),
            entries: Vec::new(),
            truncated: false,
            error: Some(message.into()),
        }
    }
}

/// Read a directory. Never fails: whatever went wrong is carried in
/// [`Listing::error`] instead, because the caller always needs a `Listing` to
/// put in `State::listings` and a `Result` would only be unwrapped straight
/// back into one.
///
/// `// TODO(1a)`: this is the bootstrap stub. It returns an empty listing for
/// every directory; the real walk over `read_dir`, `stat`-ing each entry and
/// stopping at `max_entries`, is Phase 1a's.
pub fn read(dir: &Path, cfg: &ListConfig) -> Listing {
    let _ = cfg;
    Listing {
        dir: dir.to_path_buf(),
        entries: Vec::new(),
        truncated: false,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_config_has_a_generous_but_finite_ceiling() {
        assert_eq!(ListConfig::default().max_entries, 50_000);
    }

    #[test]
    fn an_error_listing_has_no_entries_and_carries_its_reason() {
        let l = Listing::error(Path::new("/root/secret"), "permission denied");
        assert!(l.entries.is_empty());
        assert_eq!(l.error.as_deref(), Some("permission denied"));
    }

    #[test]
    fn the_stub_reads_nothing_yet() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        let l = read(dir.path(), &ListConfig::default());
        assert!(l.entries.is_empty(), "Phase 1a fills this in");
        assert!(l.error.is_none());
    }
}
