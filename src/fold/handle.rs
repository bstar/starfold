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
    /// Drill into the entry under the cursor.
    Enter,
    /// Back one level.
    Back,
    /// Jump to a level by its position in `Stack::crumbs`.
    JumpTo(usize),
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
    MarkAll,
    InvertMarks,
    ClearMarks,
    QueueCopyHere,
    QueueMoveHere,
    /// Queue a delete of the marked entries, or the one under the cursor when
    /// nothing is marked.
    QueueDelete,
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
    io: crossbeam_channel::Sender<Job>,
    ops: crossbeam_channel::Sender<Job>,
    events: crossbeam_channel::Receiver<Event>,
    state: Arc<RwLock<State>>,
    sink: EventSink,
    /// The generation of the newest preview the UI has asked for. Bumped on
    /// every `Command::Preview`; a `Done::Previewed` whose generation has
    /// fallen behind this is a stale build and is discarded rather than
    /// overwriting a newer one.
    latest_preview: Arc<AtomicU64>,
    /// Shared with the ops thread: `Command::Cancel` sets it, and the running
    /// op checks it between files. One flag is enough because the ops thread
    /// runs one op at a time.
    cancel: Arc<AtomicBool>,
    threads: (
        Option<std::thread::JoinHandle<()>>,
        Option<std::thread::JoinHandle<()>>,
    ),
    dropped: Arc<AtomicU64>,
}

/// Everything a [`Handle`] needs, so a fixture can build one with no real
/// worker threads behind it.
pub struct HandleParts {
    pub io: crossbeam_channel::Sender<Job>,
    pub ops: crossbeam_channel::Sender<Job>,
    pub events: crossbeam_channel::Receiver<Event>,
    pub state: Arc<RwLock<State>>,
    pub sink: EventSink,
    pub latest_preview: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
    pub threads: (
        Option<std::thread::JoinHandle<()>>,
        Option<std::thread::JoinHandle<()>>,
    ),
    pub dropped: Arc<AtomicU64>,
}

impl Handle {
    /// Start the two worker threads.
    ///
    /// `// TODO(1e)`: this is the bootstrap stub. It builds the state and
    /// spawns `worker::spawn_io`/`spawn_ops`, which today do nothing but wait
    /// for `Job::Shutdown`; the directory that is `start` is not actually
    /// listed until Phase 1a and 1e are both in.
    pub fn spawn(cfg: FoldConfig, start: PathBuf, home: PathBuf, trash_available: bool) -> Handle {
        let (io_tx, io_rx) = crossbeam_channel::bounded(JOB_CAPACITY);
        let (ops_tx, ops_rx) = crossbeam_channel::bounded(JOB_CAPACITY);
        let (event_tx, event_rx) = crossbeam_channel::bounded(EVENT_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let sink = EventSink::new(event_tx, Arc::clone(&dropped));
        let cancel = Arc::new(AtomicBool::new(false));

        let state = Arc::new(RwLock::new(State::new(&cfg, start, home, trash_available)));

        let io_thread = worker::spawn_io(io_rx, sink.clone(), Arc::clone(&state), cfg.clone());
        let ops_thread = worker::spawn_ops(ops_rx, sink.clone(), Arc::clone(&state), cfg);

        Handle::from_parts(HandleParts {
            io: io_tx,
            ops: ops_tx,
            events: event_rx,
            state,
            sink,
            latest_preview: Arc::new(AtomicU64::new(0)),
            cancel,
            threads: (Some(io_thread), Some(ops_thread)),
            dropped,
        })
    }

    /// Build a handle around something that is not the real core -- the UI's
    /// tests, and (Phase 2·fake) a fixture that runs the real, pure core
    /// functions inline with no threads behind them at all.
    pub fn from_parts(parts: HandleParts) -> Handle {
        Handle {
            io: parts.io,
            ops: parts.ops,
            events: parts.events,
            state: parts.state,
            sink: parts.sink,
            latest_preview: parts.latest_preview,
            cancel: parts.cancel,
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
    ///
    pub fn send(&self, command: Command) {
        if matches!(command, Command::Preview(_)) {
            self.latest_preview.fetch_add(1, Ordering::Relaxed);
        }
        let effects = {
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            state::apply(&mut state, Change::Command(command))
        };
        for job in effects.jobs {
            self.dispatch(job);
        }
        for event in effects.events {
            self.sink.send(event);
        }
    }

    fn dispatch(&self, job: Job) {
        let sender = match &job {
            Job::Plan { .. } | Job::Run { .. } => &self.ops,
            _ => &self.io,
        };
        if sender.try_send(job).is_err() {
            tracing::warn!("a job did not fit its worker's queue");
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
        // `try_send`, not a blocking send: if a worker's queue is full it is
        // busy, and closing the channel below stops it once it drains.
        let _ = self.io.try_send(Job::Shutdown);
        let _ = self.ops.try_send(Job::Shutdown);

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
            io: io_tx,
            ops: ops_tx,
            events: event_rx,
            state: Arc::new(RwLock::new(State::new(
                &FoldConfig::default(),
                "/".into(),
                "/".into(),
                false,
            ))),
            sink: EventSink::new(event_tx.clone(), Arc::new(AtomicU64::new(0))),
            latest_preview: Arc::new(AtomicU64::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
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
    fn sending_a_command_emits_whatever_apply_implied() {
        let (handle, _events, _io) = parts();
        handle.send(Command::Reload);
        // `apply` is a stub until Phase 1b; the point here is that `send`
        // forwards exactly what it returned, which today is nothing.
        assert_eq!(handle.drain().count(), 0);
    }

    #[test]
    fn previewing_bumps_the_latest_generation() {
        let (handle, _events, _io) = parts();
        assert_eq!(handle.latest_preview.load(Ordering::Relaxed), 0);
        handle.send(Command::Preview("/a".into()));
        assert_eq!(handle.latest_preview.load(Ordering::Relaxed), 1);
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
