//! Noticing that a directory changed behind the program's back.
//!
//! `// TODO(1e)`: the bootstrap stub. The real one remembers each watched
//! directory's mtime and reports the ones that moved, polled by the io thread
//! every `worker::POLL`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Default)]
pub struct Watch {
    seen: HashMap<PathBuf, Option<SystemTime>>,
}

impl Watch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the watched set with `dirs`, keeping what is known about the
    /// ones that were already watched.
    pub fn watch(&mut self, dirs: &[PathBuf]) {
        let _ = dirs;
    }

    /// The watched directories whose mtime moved since the last call.
    pub fn changed(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
}
