//! Deleting to the trash rather than for good.
//!
//! `// TODO(1c)`: the bootstrap stub. The real module wraps the `trash`
//! crate: the freedesktop specification on Linux, `NSFileManager` on macOS.

use std::path::PathBuf;

/// Whether a trash can be reached from here at all. Probed once at startup.
pub fn available() -> bool {
    false
}

/// Move every path to the trash. The error names the first path that could
/// not go, so the outcome can say which.
pub fn delete(paths: &[PathBuf]) -> Result<(), (PathBuf, String)> {
    let _ = paths;
    Ok(())
}
