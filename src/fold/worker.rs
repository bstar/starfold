//! The two worker threads: `starfold-io` and `starfold-ops`.
//!
//! Everything that touches a filesystem beyond the pure transition in
//! `state::apply` runs on one of these. `starfold-io` reads directories,
//! sizes them, builds previews and opens files externally; `starfold-ops`
//! runs the operations queue, one entry at a time. Both take the write lock
//! only inside `state::apply`, when a job finishes and its result has to be
//! folded back in -- never while the filesystem work itself is in progress,
//! which is what keeps a slow copy from stalling the panel that is drawing
//! the directory beside it.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use super::handle::EventSink;
use super::listing::Listing;
use super::ops::progress::Progress;
use super::ops::{ConflictPolicy, OpId, OpKind, Outcome, Plan};
use super::preview::Preview;
use super::state::State;
use super::summary::DirSummary;
use super::FoldConfig;

/// How often the io thread polls a frame's directory for changes, absent a
/// filesystem-notification crate. Two or three watched directories, and the
/// app already re-lists after its own operations, is not worth pulling in
/// `notify` for; `POLL` is the seam if that changes.
pub const POLL: Duration = Duration::from_secs(1);

/// Work handed to a worker thread.
///
/// A job carries everything the worker needs to do it, copied out of `State`
/// by `apply` under the write lock, so the worker never takes the lock while
/// the filesystem work is in progress. A preview job also carries a
/// `generation`, compared against `Handle::latest_preview` when it finishes,
/// so a build for a file the cursor has already left is discarded rather
/// than overwriting the newer one.
#[derive(Debug, Clone)]
pub enum Job {
    List(PathBuf),
    Summarize(PathBuf),
    Preview {
        path: PathBuf,
        generation: u64,
    },
    /// Ask the desktop, or the configured argv, to open a file.
    Open(PathBuf),
    /// Expand an op's sources, total their bytes, and find conflicts.
    Plan {
        op: OpId,
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: Option<PathBuf>,
    },
    /// Execute a planned op. `progress` is the same `Arc` the `Op` in
    /// `State` holds, which is how the status row sees the bytes move.
    Run {
        op: OpId,
        kind: OpKind,
        plan: Plan,
        policy: ConflictPolicy,
        progress: Arc<Progress>,
    },
    Shutdown,
}

/// What a worker reports back, to be folded into [`State`] through
/// [`super::state::apply`].
///
/// A result carries its data -- the listing, the summary, the plan -- rather
/// than naming what changed, because `apply` is the only thing allowed to
/// write to `State`: a worker that inserted its own listing and then sent a
/// notification would be a second writer.
#[derive(Debug, Clone)]
pub enum Done {
    Listed(Listing),
    Summarized {
        dir: PathBuf,
        summary: DirSummary,
    },
    Previewed {
        path: PathBuf,
        generation: u64,
        preview: Preview,
    },
    Planned {
        op: OpId,
        result: Result<Plan, String>,
    },
    Finished {
        op: OpId,
        outcome: Outcome,
    },
    /// Something on disk changed under one of these paths, other than
    /// through this program's own operations -- the mtime poll's finding.
    Changed(Vec<PathBuf>),
}

/// Start the io thread.
///
/// `// TODO(1e)`: this is the bootstrap stub. It does nothing but drain the
/// channel until [`Job::Shutdown`]; `read_dir`, directory summaries, preview
/// builds, the mtime poll and external opens are Phase 1e's.
pub fn spawn_io(
    jobs: crossbeam_channel::Receiver<Job>,
    events: EventSink,
    state: Arc<RwLock<State>>,
    cfg: FoldConfig,
) -> std::thread::JoinHandle<()> {
    let _ = (&events, &state, &cfg);
    std::thread::Builder::new()
        .name("starfold-io".into())
        .spawn(move || {
            for job in jobs.iter() {
                if matches!(job, Job::Shutdown) {
                    break;
                }
            }
        })
        .expect("spawning the io thread")
}

/// Start the ops thread.
///
/// `// TODO(1e)`: the bootstrap stub, as [`spawn_io`]. Running the queue --
/// `plan`, then `exec` or `trash`, one op at a time, checking `cancel`
/// between files -- is Phase 1e's, built on Phase 1c's `ops::{plan,exec,
/// trash}`.
pub fn spawn_ops(
    jobs: crossbeam_channel::Receiver<Job>,
    events: EventSink,
    state: Arc<RwLock<State>>,
    cfg: FoldConfig,
) -> std::thread::JoinHandle<()> {
    let _ = (&events, &state, &cfg);
    std::thread::Builder::new()
        .name("starfold-ops".into())
        .spawn(move || {
            for job in jobs.iter() {
                if matches!(job, Job::Shutdown) {
                    break;
                }
            }
        })
        .expect("spawning the ops thread")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::handle::EventSink;

    #[test]
    fn the_poll_interval_is_one_second() {
        assert_eq!(POLL, Duration::from_secs(1));
    }

    #[test]
    fn both_stub_threads_stop_on_shutdown() {
        let (io_tx, io_rx) = crossbeam_channel::unbounded();
        let (ops_tx, ops_rx) = crossbeam_channel::unbounded();
        let (event_tx, _event_rx) = crossbeam_channel::unbounded();
        let sink = EventSink::new(event_tx, Arc::new(std::sync::atomic::AtomicU64::new(0)));
        let state = Arc::new(RwLock::new(State::new(
            &crate::fold::FoldConfig::default(),
            PathBuf::from("/"),
            PathBuf::from("/"),
            false,
        )));

        let cfg = crate::fold::FoldConfig::default();
        let io = spawn_io(io_rx, sink.clone(), Arc::clone(&state), cfg.clone());
        let ops = spawn_ops(ops_rx, sink, state, cfg);

        io_tx.send(Job::Shutdown).unwrap();
        ops_tx.send(Job::Shutdown).unwrap();
        io.join().unwrap();
        ops.join().unwrap();
    }
}
