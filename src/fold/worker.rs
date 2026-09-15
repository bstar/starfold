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
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use super::handle::{Event, EventSink, Note};
use super::listing::{self, Listing};
use super::ops::progress::Progress;
use super::ops::{self, ConflictPolicy, OpId, OpKind, Outcome, Plan};
use super::preview::{self, Preview};
use super::state::{self, Change, State};
use super::summary::{self, Budget, DirSummary};
use super::watch::Watch;
use super::{open, FoldConfig};

/// How often the io thread polls a frame's directory for changes, absent a
/// filesystem-notification crate. Two or three watched directories, and the
/// app already re-lists after its own operations, is not worth pulling in
/// `notify` for; `POLL` is the seam if that changes.
pub const POLL: Duration = Duration::from_secs(1);

/// Both worker threads' job senders.
///
/// A job finished on either thread folds its result into `State` with
/// `state::apply`, and that fold can produce more jobs of *either* kind --
/// a `Done::Planned` handled while running the queue can hand back a
/// `Job::Run` for the very thread reporting it, and nothing stops a future
/// change from having `Done::Listed` imply a summarise job. Rather than wire
/// a special case for "the other thread" through both worker loops, each
/// thread is simply given clones of both senders and routes a job by its
/// kind the same way [`super::handle::Handle::send`] does.
#[derive(Clone)]
pub struct Senders {
    pub io: crossbeam_channel::Sender<Job>,
    pub ops: crossbeam_channel::Sender<Job>,
}

impl Senders {
    /// Send `job` to whichever worker owns its kind of work.
    pub fn dispatch(&self, job: Job) {
        let sender = match &job {
            Job::Plan { .. } | Job::Run { .. } => &self.ops,
            _ => &self.io,
        };
        if sender.try_send(job).is_err() {
            tracing::warn!("a job did not fit its worker's queue");
        }
    }
}

/// Work handed to a worker thread.
///
/// A job carries everything the worker needs to do it, copied out of `State`
/// by `apply` under the write lock, so the worker never takes the lock while
/// the filesystem work is in progress. A preview job also carries a
/// `generation`, compared against `State::preview_generation` when the job is
/// picked up, so a build for a file the cursor has already left is discarded
/// rather than overwriting the newer one.
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

/// Every directory currently open in any frame of any stack of any tab, with
/// no duplicates -- what the mtime poll watches.
///
/// Milestone 1 is one tab and one stack, but this walks the whole of `State`
/// rather than assuming that, so a later tab or a forked stack is watched for
/// free. `Stack::frames` is *every* level the stack remembers, not only the
/// crumb trail up to the active one: a directory `jump_to` stepped away from
/// (but `forward` could still step back into) is still on screen nowhere,
/// but it is one `alt+down` from being drawn again, and a change to it while
/// it is out of view should not go unnoticed until then.
fn watched_dirs(state: &Arc<RwLock<State>>) -> Vec<PathBuf> {
    let s = state.read().unwrap_or_else(|e| e.into_inner());
    let mut dirs: Vec<PathBuf> = s
        .tabs
        .tabs
        .iter()
        .flat_map(|tab| tab.stacks.iter())
        .flat_map(|stack| stack.frames().iter().map(|frame| frame.dir.clone()))
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Whether a preview job was built for a file the cursor has since left.
///
/// A separate, directly testable function rather than inline in the job
/// loop: the comparison itself -- "behind", not "different from" -- is the
/// one thing here worth pinning down with a test that does not also depend
/// on a real preview being built.
fn is_stale_preview(state: &Arc<RwLock<State>>, generation: u64) -> bool {
    let s = state.read().unwrap_or_else(|e| e.into_inner());
    generation < s.preview_generation
}

/// Fold one [`Done`] into `state` and dispatch whatever it implies, taking
/// the write lock only for the fold itself -- the rule the module doc
/// describes.
///
/// A free function rather than the closure each thread loop used to build
/// for itself, so a fixture that folds a `Done` on the test thread with no
/// worker behind it at all -- the UI's `fake` module -- goes through exactly
/// the same code as `starfold-io` and `starfold-ops` do.
pub fn finish(done: Done, state: &Arc<RwLock<State>>, events: &EventSink, senders: &Senders) {
    let effects = {
        let mut s = state.write().unwrap_or_else(|e| e.into_inner());
        state::apply(&mut s, Change::Done(done))
    };
    for job in effects.jobs {
        senders.dispatch(job);
    }
    for event in effects.events {
        events.send(event);
    }
}

/// What running one `starfold-io` job produced, before [`finish`] folds it
/// into `State`.
///
/// Not simply `Option<Done>`: `Job::Open` never produces a `Done` at all --
/// opening a file changes nothing this program tracks -- but its failure is
/// still worth a line in the status bar, and a stale preview job (superseded
/// by a newer generation before it was even picked up) is worth neither a
/// `Done` nor a `Note`.
pub enum IoOutcome {
    Done(Done),
    Note(Note),
    None,
}

/// Do the filesystem work for one io job: read a directory, size one,
/// build a preview, or ask the desktop to open a file.
///
/// Pulled out of [`spawn_io`]'s loop so the UI's `fake` fixture can run the
/// very same code inline, on the test thread, with no channel or thread
/// between the job and its result -- see the module doc.
pub fn perform_io(
    job: Job,
    cfg: &FoldConfig,
    state: &Arc<RwLock<State>>,
    cancel: &AtomicBool,
) -> IoOutcome {
    match job {
        Job::List(dir) => {
            let listing = listing::read(&dir, &cfg.list);
            IoOutcome::Done(Done::Listed(listing))
        }
        Job::Summarize(dir) => {
            let budget = Budget {
                max_entries: cfg.preview.dir_budget,
                max_depth: 64,
            };
            let summary = summary::summarize(&dir, &budget, cancel);
            IoOutcome::Done(Done::Summarized { dir, summary })
        }
        Job::Preview { path, generation } => {
            // Builds are synchronous on this one thread, so a preview cannot
            // be cancelled *mid*-build the way a running op can be; what
            // matters is not starting a build for a file the cursor has
            // already left, which this check catches before any bytes are
            // read.
            if is_stale_preview(state, generation) {
                return IoOutcome::None;
            }
            let preview = preview::build(&path, &cfg.preview, cancel);
            IoOutcome::Done(Done::Previewed {
                path,
                generation,
                preview,
            })
        }
        Job::Open(path) => match open::open_external(&path, &cfg.open) {
            Ok(()) => IoOutcome::None,
            Err(e) => IoOutcome::Note(Note::error(
                "open",
                format!("opening {}: {e}", path.display()),
            )),
        },
        // Not io work: `spawn_io`'s loop handles `Shutdown` and forwards
        // `Plan`/`Run` to the ops queue before this is ever called. Kept
        // here, rather than assumed away, so this match stays exhaustive if
        // `Job` grows another kind.
        Job::Shutdown | Job::Plan { .. } | Job::Run { .. } => IoOutcome::None,
    }
}

/// Do the filesystem work for one `starfold-ops` job: plan a queued
/// operation's sources and conflicts, or run one that is already planned.
///
/// Pulled out of [`spawn_ops`]'s loop for the same reason as [`perform_io`].
/// `cfg.preserve_times` is the one setting a run needs that a plan does not.
pub fn perform_ops(job: Job, cfg: &FoldConfig) -> Option<Done> {
    match job {
        Job::Plan {
            op,
            kind,
            sources,
            dest,
        } => {
            let result =
                ops::plan::plan(kind, &sources, dest.as_deref()).map_err(|e| e.to_string());
            Some(Done::Planned { op, result })
        }
        Job::Run {
            op,
            kind,
            plan,
            policy,
            progress,
        } => {
            let options = ops::exec::RunOptions {
                preserve_times: cfg.preserve_times,
                force_copy: false,
            };
            let outcome = ops::exec::run(kind, &plan, policy, &options, &progress);
            Some(Done::Finished { op, outcome })
        }
        // Not ops work: `spawn_ops`'s loop forwards everything else before
        // this is called.
        Job::List(_) | Job::Summarize(_) | Job::Preview { .. } | Job::Open(_) | Job::Shutdown => {
            None
        }
    }
}

/// Start the io thread: listings, directory summaries, previews, external
/// opens, and the mtime poll that notices a change this program did not make
/// itself.
pub fn spawn_io(
    jobs: crossbeam_channel::Receiver<Job>,
    events: EventSink,
    state: Arc<RwLock<State>>,
    cfg: FoldConfig,
    senders: Senders,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("starfold-io".into())
        .spawn(move || {
            let mut watch = Watch::new();

            loop {
                let job = match jobs.recv_timeout(POLL) {
                    Ok(job) => job,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        let dirs = watched_dirs(&state);
                        watch.watch(&dirs);
                        let changed = watch.changed();
                        if !changed.is_empty() {
                            finish(Done::Changed(changed), &state, &events, &senders);
                        }
                        continue;
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                };

                match job {
                    Job::Shutdown => break,
                    other @ (Job::Plan { .. } | Job::Run { .. }) => {
                        // `Senders::dispatch` always routes these to `ops`;
                        // arriving here would mean something upstream sent a
                        // job to the wrong queue. Forward it rather than
                        // drop it silently.
                        senders.dispatch(other);
                    }
                    other => {
                        let cancel = AtomicBool::new(false);
                        match perform_io(other, &cfg, &state, &cancel) {
                            IoOutcome::Done(done) => finish(done, &state, &events, &senders),
                            IoOutcome::Note(note) => events.send(Event::Note(note)),
                            IoOutcome::None => {}
                        }
                    }
                }
            }
        })
        .expect("spawning the io thread")
}

/// Start the ops thread: the queue, one entry at a time.
pub fn spawn_ops(
    jobs: crossbeam_channel::Receiver<Job>,
    events: EventSink,
    state: Arc<RwLock<State>>,
    cfg: FoldConfig,
    senders: Senders,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("starfold-ops".into())
        .spawn(move || {
            for job in jobs.iter() {
                match job {
                    Job::Shutdown => break,
                    other @ (Job::Plan { .. } | Job::Run { .. }) => {
                        if let Some(done) = perform_ops(other, &cfg) {
                            finish(done, &state, &events, &senders);
                        }
                    }
                    other => {
                        // As in `spawn_io`: not this thread's work, but worth
                        // forwarding rather than dropping.
                        senders.dispatch(other);
                    }
                }
            }
        })
        .expect("spawning the ops thread")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::handle::EventSink;
    use std::sync::atomic::AtomicU64;

    fn senders() -> (
        Senders,
        crossbeam_channel::Receiver<Job>,
        crossbeam_channel::Receiver<Job>,
    ) {
        let (io_tx, io_rx) = crossbeam_channel::unbounded();
        let (ops_tx, ops_rx) = crossbeam_channel::unbounded();
        (
            Senders {
                io: io_tx,
                ops: ops_tx,
            },
            io_rx,
            ops_rx,
        )
    }

    #[test]
    fn the_poll_interval_is_one_second() {
        assert_eq!(POLL, Duration::from_secs(1));
    }

    #[test]
    fn both_worker_threads_stop_on_shutdown() {
        let (io_tx, io_rx) = crossbeam_channel::unbounded();
        let (ops_tx, ops_rx) = crossbeam_channel::unbounded();
        let (event_tx, _event_rx) = crossbeam_channel::unbounded();
        let sink = EventSink::new(event_tx, Arc::new(AtomicU64::new(0)));
        let state = Arc::new(RwLock::new(State::new(
            &FoldConfig::default(),
            PathBuf::from("/"),
            PathBuf::from("/"),
            false,
        )));
        let senders = Senders {
            io: io_tx.clone(),
            ops: ops_tx.clone(),
        };

        let cfg = FoldConfig::default();
        let io = spawn_io(
            io_rx,
            sink.clone(),
            Arc::clone(&state),
            cfg.clone(),
            senders.clone(),
        );
        let ops = spawn_ops(ops_rx, sink, state, cfg, senders);

        io_tx.send(Job::Shutdown).unwrap();
        ops_tx.send(Job::Shutdown).unwrap();
        io.join().unwrap();
        ops.join().unwrap();
    }

    /// Drives `spawn_io` directly with its own channels -- no `Handle`
    /// involved -- and checks that a `Job::List` ends up folded into `State`
    /// with an `Event::Listing` drained for it.
    ///
    /// This depends on Phase 1b's `state::apply` being real: today it is
    /// still the bootstrap stub that touches nothing and returns no events,
    /// so this assertion fails until that lands, not because anything here
    /// is wrong.
    #[test]
    fn a_list_job_folds_its_listing_into_state_and_emits_an_event() {
        let dir = tempfile::tempdir().unwrap();
        let (jobs_tx, jobs_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let sink = EventSink::new(event_tx, Arc::new(AtomicU64::new(0)));
        let state = Arc::new(RwLock::new(State::new(
            &FoldConfig::default(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            false,
        )));
        let senders = Senders {
            io: jobs_tx.clone(),
            ops: crossbeam_channel::unbounded::<Job>().0,
        };

        let io = spawn_io(
            jobs_rx,
            sink,
            Arc::clone(&state),
            FoldConfig::default(),
            senders,
        );

        jobs_tx.send(Job::List(dir.path().to_path_buf())).unwrap();
        jobs_tx.send(Job::Shutdown).unwrap();
        io.join().unwrap();

        assert!(
            state.read().unwrap().listing_of(dir.path()).is_some(),
            "state::apply (Phase 1b) folds Done::Listed into State::listings; \
             until it is real this stays empty"
        );
        assert!(
            matches!(event_rx.try_recv(), Ok(Event::Listing(_))),
            "state::apply (Phase 1b) is what emits Event::Listing"
        );
    }

    #[test]
    fn a_preview_job_behind_the_current_generation_is_stale() {
        let state = Arc::new(RwLock::new(State::new(
            &FoldConfig::default(),
            "/".into(),
            "/".into(),
            false,
        )));
        state.write().unwrap().preview_generation = 5;

        assert!(is_stale_preview(&state, 0));
        assert!(is_stale_preview(&state, 4));
        assert!(
            !is_stale_preview(&state, 5),
            "the current generation is not stale"
        );
        assert!(
            !is_stale_preview(&state, 6),
            "a newer generation is not stale"
        );
    }

    #[test]
    fn senders_dispatch_routes_plan_and_run_to_ops_and_everything_else_to_io() {
        let (senders, io_rx, ops_rx) = senders();

        senders.dispatch(Job::List("/a".into()));
        senders.dispatch(Job::Summarize("/a".into()));
        senders.dispatch(Job::Open("/a".into()));
        senders.dispatch(Job::Preview {
            path: "/a".into(),
            generation: 0,
        });
        assert_eq!(io_rx.try_iter().count(), 4);
        assert_eq!(ops_rx.try_iter().count(), 0);

        senders.dispatch(Job::Plan {
            op: OpId(0),
            kind: OpKind::Copy,
            sources: vec!["/a".into()],
            dest: Some("/b".into()),
        });
        senders.dispatch(Job::Run {
            op: OpId(0),
            kind: OpKind::Copy,
            plan: Plan::default(),
            policy: ConflictPolicy::Ask,
            progress: Arc::new(Progress::new(0)),
        });
        assert_eq!(ops_rx.try_iter().count(), 2);
    }
}
