//! The contract between the fold core and the terminal.
//!
//! The core runs on two OS threads -- `starfold-io` and `starfold-ops`,
//! spawned by [`Handle::spawn`] -- with no async runtime. The UI loop is
//! synchronous: it sends [`Command`]s through [`Handle::send`], drains
//! [`Event`]s once a frame through [`Handle::drain`], and reads the truth out
//! of [`State`] behind an `RwLock`.
//!
//! **Events carry no state.** `Event::Stack` means "the stack changed", not
//! what it changed to. So an event may be coalesced, delayed or dropped
//! outright and the next frame still draws the truth out of `State`. That is
//! what makes the bounded event channel safe: when it fills, [`EventSink`]
//! counts the drop and the next send that fits is preceded by
//! [`Event::Refresh`], which tells the UI to stop trusting its incremental
//! picture and re-read everything. Copied from STAR/CORD's
//! `discord::handle`, with tokio's command channel replaced by a second
//! `crossbeam` one -- nothing under `src/fold/` waits on a socket, so there
//! is nothing here for an async runtime to do.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard};
use std::time::{Duration, Instant};

use super::ops::{ConflictPolicy, OpId};
use super::sort::SortOrder;
use super::state::{self, Change, State};
use super::worker::{self, Job};
use super::FoldConfig;

/// How many commands may be in flight before the UI is told to slow down.
const COMMAND_CAPACITY: usize = 256;
/// How many jobs may be queued for a worker before the same applies.
const JOB_CAPACITY: usize = 256;
/// How many events may be queued before they start being dropped.
const EVENT_CAPACITY: usize = 4096;
/// How long `Drop` waits for the two worker threads to finish.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Everything the terminal can ask the core to do.
///
/// Named for what it means rather than for the key that reaches it -- `y` and
/// `p` both send [`Command::QueueCopyHere`] -- so the dispatcher in
/// `ui/app.rs` (Phase 3a) is a `match` on meaning, not on input.
#[derive(Debug, Clone)]
pub enum Command {
    LoadPlaces(PathBuf),
    RefreshPlaces,
    SaveBookmark {
        name: String,
        path: PathBuf,
    },
    RenameBookmark {
        path: PathBuf,
        name: String,
    },
    RemoveBookmark(PathBuf),
    /// Surface a startup warning after the terminal has entered its window.
    Notify(String),
    ToggleView,
    FocusPane(usize),
    RestoreCommander {
        dirs: [PathBuf; 2],
        active: usize,
        enabled: bool,
    },
    /// Drill into the entry under the cursor.
    Enter,
    /// Back one level.
    Back,
    /// Jump to a level by its position in `Stack::crumbs`.
    JumpTo(usize),
    /// Step one frame towards the end of the trail -- see `Stack::forward`.
    /// `alt+down`: back into a child a `JumpTo` left behind rather than
    /// popped.
    Forward,
    /// Push a directory that did not come from the cursor -- `gh`, `gr`, a
    /// crumb click, a path typed on the command line.
    Push(PathBuf),
    Reload,
    /// Move the cursor to a row by index.
    CursorTo(usize),
    /// Move the cursor by a signed number of rows.
    CursorBy(i32),
    SetFilter(String),
    ClearFilter,
    SetSort(SortOrder),
    SetHidden(bool),
    /// Mark or unmark the entry under the cursor, and move down.
    ToggleMark,
    ToggleMarkPath(PathBuf),
    MarkAll,
    InvertMarks,
    ClearMarks,
    QueueOperation {
        kind: super::ops::OpKind,
        sources: Vec<PathBuf>,
        dest: Option<PathBuf>,
    },
    PreviewPage {
        path: PathBuf,
        generation: u64,
        page: u32,
    },
    QueueCopyHere,
    QueueMoveHere,
    /// Queue a delete of the marked entries, or the one under the cursor when
    /// nothing is marked.
    QueueDelete,
    QueueDeleteSources(Vec<PathBuf>),
    QueueRename {
        from: PathBuf,
        to: PathBuf,
    },
    RemoveOp(OpId),
    ClearQueue,
    /// Run the queue: plan whatever has not been planned, then run whatever
    /// is runnable.
    Run,
    /// Answer a conflict an op's plan found under `ConflictPolicy::Ask`.
    SetPolicy(OpId, ConflictPolicy),
    Cancel(OpId),
    Preview(PathBuf),
    ClosePreview,
    OpenExternal(PathBuf),
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteLevel {
    Info,
    Warning,
    Error,
}

/// Something to tell the user in the status line.
#[derive(Debug, Clone)]
pub struct Note {
    pub level: NoteLevel,
    /// Repeats of the same key replace rather than stack, so a directory that
    /// keeps failing to read does not fill the status line with the same
    /// line over and over.
    pub key: Option<&'static str>,
    pub text: String,
}

impl Note {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            level: NoteLevel::Info,
            key: None,
            text: text.into(),
        }
    }

    pub fn warning(key: &'static str, text: impl Into<String>) -> Self {
        Self {
            level: NoteLevel::Warning,
            key: Some(key),
            text: text.into(),
        }
    }

    pub fn error(key: &'static str, text: impl Into<String>) -> Self {
        Self {
            level: NoteLevel::Error,
            key: Some(key),
            text: text.into(),
        }
    }
}

/// A notification that something changed. Never the change itself -- see the
/// module doc.
#[derive(Debug, Clone)]
pub enum Event {
    Places,
    Listing(PathBuf),
    Stack,
    Selection,
    Queue(OpId),
    Progress(OpId),
    Preview,
    /// An op's plan found a conflict under `ConflictPolicy::Ask` and is
    /// waiting for `Command::SetPolicy`.
    Conflicts(OpId),
    Note(Note),
    /// Events were dropped. Whatever the UI believes about its incremental
    /// state is now suspect; re-read everything.
    Refresh,
}

/// The sending half of the event channel, with the drop bookkeeping.
///
/// Copied from STAR/CORD's `EventSink`: a dropped event means the UI's
/// incremental picture may be wrong, so the next one that fits is preceded by
/// a [`Event::Refresh`], and sending that eagerly would just be another event
/// to drop.
#[derive(Clone)]
pub struct EventSink {
    tx: crossbeam_channel::Sender<Event>,
    dropped: Arc<AtomicU64>,
    needs_refresh: Arc<AtomicBool>,
}

impl EventSink {
    pub fn new(tx: crossbeam_channel::Sender<Event>, dropped: Arc<AtomicU64>) -> Self {
        Self {
            tx,
            dropped,
            needs_refresh: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn send(&self, event: Event) {
        if self.needs_refresh.swap(false, Ordering::Relaxed)
            && self.tx.try_send(Event::Refresh).is_err()
        {
            self.needs_refresh.store(true, Ordering::Relaxed);
        }

        if self.tx.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            self.needs_refresh.store(true, Ordering::Relaxed);
        }
    }
}

/// The UI's view of the core.
pub struct Handle {
    senders: worker::Senders,
    events: crossbeam_channel::Receiver<Event>,
    state: Arc<RwLock<State>>,
    sink: EventSink,
    threads: (
        Option<std::thread::JoinHandle<()>>,
        Option<std::thread::JoinHandle<()>>,
    ),
    dropped: Arc<AtomicU64>,
}

/// Everything a [`Handle`] needs, so a fixture can build one with no real
/// worker threads behind it.
///
/// No `latest_preview` or `cancel` field here: the generation a stale
/// preview is compared against now lives on `State` itself
/// (`State::preview_generation`), and `Command::Cancel` reaches the running
/// op's own `Progress` through `state::apply` rather than through a flag
/// `Handle` holds on the side. Both were vestigial once `State` grew a real
/// place for what they stood in for.
pub struct HandleParts {
    pub senders: worker::Senders,
    pub events: crossbeam_channel::Receiver<Event>,
    pub state: Arc<RwLock<State>>,
    pub sink: EventSink,
    pub threads: (
        Option<std::thread::JoinHandle<()>>,
        Option<std::thread::JoinHandle<()>>,
    ),
    pub dropped: Arc<AtomicU64>,
}

impl Handle {
    /// Start the two worker threads.
    ///
    /// `start` is listed synchronously, before either thread is spawned, so
    /// the first frame the UI draws already has content instead of an empty
    /// panel for however long it takes `starfold-io` to pick the job up.
    pub fn spawn(cfg: FoldConfig, start: PathBuf, home: PathBuf, trash_available: bool) -> Handle {
        let (io_tx, io_rx) = crossbeam_channel::bounded(JOB_CAPACITY);
        let (ops_tx, ops_rx) = crossbeam_channel::bounded(JOB_CAPACITY);
        let (event_tx, event_rx) = crossbeam_channel::bounded(EVENT_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let sink = EventSink::new(event_tx, Arc::clone(&dropped));
        let senders = worker::Senders {
            io: io_tx,
            ops: ops_tx,
        };

        let state = Arc::new(RwLock::new(State::new(
            &cfg,
            start.clone(),
            home,
            trash_available,
        )));

        {
            let listing = super::listing::read(&start, &cfg.list);
            let effects = {
                let mut s = state.write().unwrap_or_else(|e| e.into_inner());
                state::apply(&mut s, Change::Done(worker::Done::Listed(listing)))
            };
            for job in effects.jobs {
                senders.dispatch(job);
            }
            for event in effects.events {
                sink.send(event);
            }
        }

        let io_thread = worker::spawn_io(
            io_rx,
            sink.clone(),
            Arc::clone(&state),
            cfg.clone(),
            senders.clone(),
        );
        let ops_thread = worker::spawn_ops(
            ops_rx,
            sink.clone(),
            Arc::clone(&state),
            cfg,
            senders.clone(),
        );

        Handle::from_parts(HandleParts {
            senders,
            events: event_rx,
            state,
            sink,
            threads: (Some(io_thread), Some(ops_thread)),
            dropped,
        })
    }

    /// Whether a trash can be reached from here at all. Probed once, before
    /// [`spawn`](Self::spawn) is even called, so `main` can decide the
    /// answer before there is a `State` to put it in -- `spawn` only takes
    /// it as a `bool`, not a way to find one out.
    pub fn probe_trash() -> bool {
        super::ops::trash::available()
    }

    /// Build a handle around something that is not the real core -- the UI's
    /// tests, and (Phase 2·fake) a fixture that runs the real, pure core
    /// functions inline with no threads behind them at all.
    pub fn from_parts(parts: HandleParts) -> Handle {
        Handle {
            senders: parts.senders,
            events: parts.events,
            state: parts.state,
            sink: parts.sink,
            threads: parts.threads,
            dropped: parts.dropped,
        }
    }

    /// Ask the core to do something.
    ///
    /// Applies the pure transition under the write lock, dispatches whatever
    /// `Job`s it produced to the right worker, and lets the lock go before
    /// anything is sent -- a worker taking the lock to report a result must
    /// never wait on a channel send this function is blocked on.
    pub fn send(&self, command: Command) {
        let effects = {
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            state::apply(&mut state, Change::Command(command))
        };
        for job in effects.jobs {
            self.senders.dispatch_checked(job, &self.state, &self.sink);
        }
        for event in effects.events {
            self.sink.send(event);
        }
    }

    /// Everything queued, for the once-a-frame drain.
    pub fn drain(&self) -> crossbeam_channel::TryIter<'_, Event> {
        self.events.try_iter()
    }

    /// The truth. Copy what the frame needs and drop the guard before
    /// drawing: a worker takes the write lock to fold its result in, and a
    /// read guard held across a render stalls it.
    pub fn state(&self) -> RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn events_dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.send(Command::Shutdown);
        // `try_send`, not a blocking send: if a worker's queue is full it is
        // busy, and closing the channel below stops it once it drains.
        let _ = self.senders.io.try_send(Job::Shutdown);
        let _ = self.senders.ops.try_send(Job::Shutdown);

        let deadline = Instant::now() + SHUTDOWN_GRACE;
        for thread in [self.threads.0.take(), self.threads.1.take()]
            .into_iter()
            .flatten()
        {
            while !thread.is_finished() {
                if Instant::now() >= deadline {
                    tracing::warn!("a fold worker did not stop within {SHUTDOWN_GRACE:?}");
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if let Err(e) = thread.join() {
                tracing::error!("a fold worker panicked: {e:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> (
        Handle,
        crossbeam_channel::Sender<Event>,
        crossbeam_channel::Receiver<Job>,
    ) {
        let (io_tx, io_rx) = crossbeam_channel::bounded(4);
        let (ops_tx, _ops_rx) = crossbeam_channel::bounded(4);
        let (event_tx, event_rx) = crossbeam_channel::bounded(8);
        let handle = Handle::from_parts(HandleParts {
            senders: worker::Senders {
                io: io_tx,
                ops: ops_tx,
            },
            events: event_rx,
            state: Arc::new(RwLock::new(State::new(
                &FoldConfig::default(),
                "/".into(),
                "/".into(),
                false,
            ))),
            sink: EventSink::new(event_tx.clone(), Arc::new(AtomicU64::new(0))),
            threads: (None, None),
            dropped: Arc::new(AtomicU64::new(0)),
        });
        (handle, event_tx, io_rx)
    }

    #[test]
    fn the_drain_yields_everything_queued_and_then_stops() {
        let (handle, events, _io) = parts();
        events.send(Event::Stack).unwrap();
        events.send(Event::Selection).unwrap();
        assert_eq!(handle.drain().count(), 2);
        assert_eq!(handle.drain().count(), 0);
    }

    #[test]
    fn sending_a_command_forwards_the_jobs_and_events_apply_implied() {
        let (handle, _events, io) = parts();
        // A reload of the root frame is a listing job for the io thread and
        // a stack notification for the window -- one of each, in that order
        // of importance: the job goes out before the event, so a window that
        // reacts to the event finds the work already queued.
        handle.send(Command::Reload);
        assert!(matches!(io.try_recv(), Ok(Job::List(dir)) if dir == std::path::Path::new("/")));
        assert!(handle.drain().any(|e| matches!(e, Event::Stack)));
    }

    /// `Handle::spawn` lists `start` synchronously before either worker
    /// thread runs, so the first frame is never empty for want of a job
    /// being picked up. This depends on Phase 1b's `state::apply` folding
    /// `Done::Listed` into `State::listings` and clearing `State::loading`;
    /// today `apply` is still the bootstrap stub, so this fails for that
    /// reason alone until 1b lands, not because `listing::read` or
    /// `Handle::spawn` themselves are wrong.
    #[test]
    fn spawning_lists_the_start_directory_before_either_thread_runs() {
        let fixture = crate::fold::testing::Fixture::tree();
        let handle = Handle::spawn(
            FoldConfig::default(),
            fixture.home().to_path_buf(),
            fixture.home().to_path_buf(),
            false,
        );
        assert!(
            handle.state().listing_of(fixture.home()).is_some(),
            "state::apply (Phase 1b) is what inserts the listing"
        );
        assert!(
            !handle.state().loading,
            "state::apply (Phase 1b) is what clears `loading`"
        );
    }

    /// `Drop` sends `Job::Shutdown` to both real worker threads and waits for
    /// them to join; this only exercises the mechanics -- both threads are
    /// today idle stubs waiting on their channel -- so it holds regardless of
    /// which other phases have landed.
    #[test]
    fn dropping_a_handle_joins_both_worker_threads_promptly() {
        let dir = tempfile::tempdir().unwrap();
        let handle = Handle::spawn(
            FoldConfig::default(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            false,
        );
        let started = Instant::now();
        drop(handle);
        assert!(
            started.elapsed() < SHUTDOWN_GRACE,
            "Drop must join both threads well within the grace period, not merely by it"
        );
    }

    #[test]
    fn probing_the_trash_does_not_panic_and_answers_a_plain_bool() {
        // `ops::trash::available` is still Phase 1c's stub, which always
        // answers `false`; this just confirms the plumbing reaches it.
        let _: bool = Handle::probe_trash();
    }

    /// Copied from STAR/CORD's `a_dropped_event_becomes_a_refresh_on_the_next_one_that_fits`:
    /// an event that does not fit the bounded channel is counted, and the
    /// next one that does is preceded by a `Refresh` so the UI knows to
    /// re-read everything rather than trust what it missed.
    #[test]
    fn events_that_do_not_fit_are_dropped_and_followed_by_a_refresh() {
        let (event_tx, event_rx) = crossbeam_channel::bounded(2);
        let dropped = Arc::new(AtomicU64::new(0));
        let sink = EventSink::new(event_tx, Arc::clone(&dropped));

        sink.send(Event::Stack);
        sink.send(Event::Stack);
        // Full now.
        sink.send(Event::Stack);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);

        let _ = event_rx.try_recv();
        let _ = event_rx.try_recv();
        sink.send(Event::Selection);

        assert!(
            matches!(event_rx.try_recv(), Ok(Event::Refresh)),
            "a UI that missed an event was not told to re-read"
        );
        assert!(matches!(event_rx.try_recv(), Ok(Event::Selection)));
    }

    #[test]
    fn a_command_that_names_a_file_can_still_be_debug_printed() {
        // Nothing here is a secret the way a Discord token is; this just
        // confirms the derive is in place for the log line `send` could grow.
        let command = Command::Push(PathBuf::from("/home/bob"));
        assert!(format!("{command:?}").contains("bob"));
    }
}
