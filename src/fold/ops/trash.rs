//! Deleting to the trash rather than for good.
//!
//! Wraps the `trash` crate: the freedesktop trash specification in pure
//! Rust on Linux, `NSFileManager` through `objc2` bindings on macOS. One
//! call moves every path at once -- `trash::delete_all` batches whatever the
//! platform lets it batch, which on Linux is each item's own rename into a
//! `Trash/files` entry plus an `.trashinfo` write, done back to back rather
//! than through a slower one-call-per-path API.

use std::path::PathBuf;

/// Whether a trash can be reached from here at all. Probed once at startup
/// and shown in the queue before an op runs, so [`super::DeleteHow`] can be
/// decided up front rather than discovered mid-delete.
///
/// Always `true`. macOS always has `NSFileManager`'s trash. A correct probe
/// on Linux would have to know which mount every *future* delete will
/// target: the home trash (`$XDG_DATA_HOME/Trash`, falling back to
/// `~/.local/share/Trash`) is only the common case, and a path on a
/// different filesystem uses that filesystem's own top-level
/// `$topdir/.Trash-$uid` instead, which nothing available at startup has any
/// way to name. Reporting `true` unconditionally and letting a real failure
/// surface from [`delete`] -- which `state::apply` turns into a note plus
/// the permanent-delete confirmation, per this crate's `SECURITY.md` -- is
/// both simpler and cannot be wrong in the way a home-only probe can, since
/// it never claims "no trash" for a path the home probe just did not think
/// to check.
pub fn available() -> bool {
    true
}

/// Move every path to the trash in one call. `trash::delete_all` batches the
/// underlying platform operations; on failure the error names whichever path
/// it can identify, falling back to the first path in the batch when the
/// underlying error carries none of its own. Nothing here tries the rest of
/// the batch individually afterwards -- the caller stops and reports the one
/// failure, which is what "record the failure per path and stop" asks for.
pub fn delete(paths: &[PathBuf]) -> Result<(), (PathBuf, String)> {
    if paths.is_empty() {
        return Ok(());
    }
    trash::delete_all(paths).map_err(|e| (blame(&e, paths), e.to_string()))
}

/// Best-effort extraction of which path an error names. Falls back to the
/// first path in the batch for a variant that carries none (`Error::Os`,
/// `Error::Unknown`, `Error::TargetedRoot`, ...) -- still better than no
/// path at all for the status row to show.
fn blame(err: &trash::Error, paths: &[PathBuf]) -> PathBuf {
    match err {
        #[cfg(all(unix, not(target_os = "macos")))]
        trash::Error::FileSystem { path, .. } => path.clone(),
        trash::Error::CouldNotAccess { target } => PathBuf::from(target),
        trash::Error::CanonicalizePath { original } => original.clone(),
        _ => paths.first().cloned().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_nothing_is_fine() {
        assert!(delete(&[]).is_ok());
    }

    #[test]
    fn a_trash_is_always_reported_available() {
        assert!(available());
    }

    /// Touches the real trash, so it is gated behind an explicit opt-in
    /// rather than running on every `cargo test` -- a CI box or a sandboxed
    /// dev machine may have no trash daemon, no `~/.local/share`, or run as
    /// a user with no home directory at all.
    #[test]
    fn moving_a_real_file_to_the_trash_round_trips() {
        if std::env::var("STARFOLD_TEST_TRASH").as_deref() != Ok("1") {
            println!("skipping: set STARFOLD_TEST_TRASH=1 to exercise the real trash");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trash-me.txt");
        std::fs::write(&path, b"gone soon").unwrap();

        delete(std::slice::from_ref(&path)).expect("moving a real file to the real trash");
        assert!(!path.exists());
    }
}
