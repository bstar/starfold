//! What STAR/FOLD remembers between runs.
//!
//! Not settings -- those are `config.toml`, which a person edits. This is
//! where the fold was left, so the next run opens where this one closed
//! rather than back at the current directory every time. A session file that
//! will not parse is not an error worth reporting: it is a cache of one
//! convenience, and losing it costs a directory.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::fold::sort::SortOrder;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<SortOrder>,
}

/// Read the session, or the defaults if there is none, or it will not parse.
/// Either is logged, not returned as an error -- nothing in the caller would
/// do anything with one but fall back to the same defaults anyway.
pub fn load(path: &Path) -> Session {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Session::default();
    };
    match toml::from_str(&text) {
        Ok(session) => session,
        Err(e) => {
            tracing::warn!("ignoring an unreadable session file: {e}");
            Session::default()
        }
    }
}

impl Session {
    /// Write it, through a temporary file in the same directory so an
    /// interrupted write cannot leave a truncated session behind.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).context("serialising the session")?;
        starkit::fs::write_atomic(path, text.as_bytes())
            .with_context(|| format!("writing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");

        let session = Session {
            last_dir: Some(PathBuf::from("/home/bob/projects")),
            show_hidden: Some(true),
            sort: Some(SortOrder::default()),
        };
        session.save(&path).unwrap();

        let read = load(&path);
        assert_eq!(read, session);
    }

    #[test]
    fn a_missing_file_loads_as_the_default_session() {
        let dir = tempfile::tempdir().unwrap();
        let session = load(&dir.path().join("nothing.toml"));
        assert_eq!(session, Session::default());
    }

    #[test]
    fn an_unparsable_file_is_ignored_rather_than_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "this is not [ toml").unwrap();
        assert_eq!(load(&path), Session::default());
    }

    #[test]
    fn a_missing_parent_directory_is_created_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("session.toml");
        Session::default().save(&path).unwrap();
        assert!(path.is_file());
    }
}
