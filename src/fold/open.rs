//! Handing a file to whatever the desktop opens it with.
//!
//! `// TODO(1d)`: the bootstrap stub. The real one runs the configured argv,
//! or `open` on macOS and `xdg-open` elsewhere, with every fd on `/dev/null`
//! and the child detached -- never through a shell.

use std::path::Path;

use super::OpenConfig;

pub fn open_external(path: &Path, cfg: &OpenConfig) -> std::io::Result<()> {
    let _ = (path, cfg);
    Ok(())
}
