//! What an operation would do, worked out before anything is touched.
//!
//! `// TODO(1c)`: the bootstrap stub. The real `plan` expands directories
//! without following symlinks, totals the bytes, finds every name that already
//! exists at the destination, and refuses a copy or move into itself.

use std::path::{Path, PathBuf};

use super::{OpKind, Plan};

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("{0} is inside itself")]
    IntoItself(PathBuf),
    #[error("{0} is gone")]
    Missing(PathBuf),
    #[error("a copy or move needs somewhere to go")]
    NoDestination,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Work out `Plan` for `kind` over `sources` into `dest`.
pub fn plan(kind: OpKind, sources: &[PathBuf], dest: Option<&Path>) -> Result<Plan, PlanError> {
    let _ = (kind, sources, dest);
    Ok(Plan::default())
}
