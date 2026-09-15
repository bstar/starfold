//! Running a planned operation: the bytes actually moving.
//!
//! `// TODO(1c)`: the bootstrap stub. The real `run` copies file by file with
//! `std::fs::copy`, recreates symlinks rather than following them, renames
//! where it can and copies-then-deletes across devices, applies the conflict
//! policy, and checks `progress.is_cancelled()` between items.

use super::progress::Progress;
use super::{ConflictPolicy, OpKind, Outcome, Plan};

/// Knobs `run` takes from the configuration, and one for the tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunOptions {
    pub preserve_times: bool,
    /// Copy-then-delete even where a rename would do, so the cross-device
    /// path is exercised on one filesystem.
    pub force_copy: bool,
}

/// Run `plan`. Never panics on a bad file; what failed is in the `Outcome`.
pub fn run(
    kind: OpKind,
    plan: &Plan,
    policy: ConflictPolicy,
    options: &RunOptions,
    progress: &Progress,
) -> Outcome {
    let _ = (kind, plan, policy, options, progress);
    Outcome::default()
}
