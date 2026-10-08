//! The contract between the fold core and the terminal.
//!
//! The core runs on two long-lived OS threads -- `starfold-io` and
//! `starfold-ops` -- plus short-lived startup listing threads. The UI loop is
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

use super::create::Kind as CreateKind;
use super::listing::ListConfig;
use super::ops::{ConflictPolicy, OpId};
use super::sort::SortOrder;
use super::state::{self, Change, State};
use super::worker::{self, Job};
use super::FoldConfig;

/// One location to restore without touching its filesystem on the UI thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupLocation {
    pub path: PathBuf,
    /// A saved location that disappeared falls back to the launch directory.
    /// Explicit CLI paths have no fallback.
    pub fallback: Option<PathBuf>,
    pub label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Startup {
    pub fold: StartupLocation,
    pub commander: Option<([StartupLocation; 2], usize, bool)>,
}

type StartupReader = Arc<dyn Fn(StartupLocation, &ListConfig) -> worker::Done + Send + Sync>;

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
/// Named for what it means rather than for the key that reaches it, so the
/// dispatcher in `ui/app.rs` is a `match` on meaning, not on input.
#[derive(Debug, Clone)]
pub enum Command {
    SyncArchiveEdits,
    LoadPlaces(PathBuf),
    RefreshPlaces,
    UnmountPlace {
        path: PathBuf,
        source: PathBuf,
    },
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
    RememberTabScroll {
        tab: super::tab::TabId,
        rows: Vec<(usize, super::stack::FrameId, usize)>,
    },
    NewTab {
        duplicate: bool,
    },
    SwitchTab(super::tab::TabId),
    CloseTab(super::tab::TabId),
    RenameTab(super::tab::TabId, String),
    MoveTab(super::tab::TabId, i32),
    ReopenTab,
    RestoreTabs(Vec<crate::session::TabSession>, usize),
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
    /// Create in the active directory immediately, without overwriting.
    Create {
        dir: PathBuf,
        kind: CreateKind,
        name: String,
    },
    /// Move the cursor to a row by index.
    CursorTo(usize),
    /// Move the cursor by a signed number of rows.
    CursorBy(i32),
    SetFilter(String),
    ClearFilter,
    StartSearch(String),
    StartContentSearch(String),
    CancelSearch,
    CloseSearch,
    SetSort(SortOrder),
    /// Restore Fold and both Commander sort orders from a session.
    RestoreSorts {
        fold: Option<SortOrder>,
        panes: [Option<SortOrder>; 2],
    },
    SetHidden(bool),
    /// Mark or unmark the entry under the cursor, and move down.
    ToggleMark,
    ToggleMarkPath(PathBuf),
    MarkAll,
    InvertMarks,
    ClearMarks,
    UnlockArchive {
        location: PathBuf,
        options: starfold_archive_protocol::Options,
    },
    QueueArchive {
        format: super::archive::Format,
        sources: Vec<PathBuf>,
        destination: PathBuf,
        options: starfold_archive_protocol::Options,
    },
    QueueOperation {
        kind: super::ops::OpKind,
        sources: Vec<PathBuf>,
        dest: Option<PathBuf>,
    },
    /// Queue a native drop in request order.
    QueueDrop {
        kind: super::ops::OpKind,
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    BeginExport(Vec<PathBuf>),
    FinishExport {
        op: OpId,
        success: bool,
    },
    BeginImport {
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    CompleteImport {
        op: OpId,
        sources: Vec<PathBuf>,
    },
    FinishImport {
        op: OpId,
        success: bool,
    },
    /// Preserve transport/source-cleanup errors in the operation history.
    ReportDropFailure {
        op: OpId,
        reason: String,
    },
    PreviewAck {
        session: u64,
        sequence: u64,
    },
    PreviewInput {
        path: PathBuf,
        generation: u64,
        input: starfold_preview_protocol::Input,
    },
    PreviewPage {
        path: PathBuf,
        generation: u64,
        page: u32,
    },
    /// Save the marked entries, or the highlighted entry, for later pastes.
    Yank,
    /// Queue a copy of the last yanked paths into the active directory.
    PasteHere,
    QueueMoveHere,
    /// Queue a delete of the marked entries, or the one under the cursor when
    /// nothing is marked.
    QueueDelete,
    QueueDeleteSources(Vec<PathBuf>),
    /// Explicit permanent delete after a separate confirmation in the UI.
    QueuePermanentDeleteSources(Vec<PathBuf>),
    /// Retry only permission-denied paths from a failed permanent delete.
    QueueElevatedDelete(OpId),
    /// Empty this mounted volume's trash after confirmation in Places.
    EmptyDriveTrash(PathBuf),
    QueueRename {
        from: PathBuf,
        to: PathBuf,
    },
    RemoveOp(OpId),
    ClearQueue,
    /// Resume a paused queue; new operations normally start automatically.
    Run,
    /// Answer a conflict an op's plan found under `ConflictPolicy::Ask`.
    SetPolicy(OpId, ConflictPolicy),
    /// Resolve Rename with explicit targets, one for each conflict.
    SetConflictNames(OpId, Vec<(PathBuf, PathBuf)>),
    Cancel(OpId),
    /// Stop active work and hold subsequent operations until Run resumes them.
    StopActive(OpId),
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
    /// The remote names were checked and the receiver may request bytes.
    ImportReady(OpId),
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
    startup_threads: Vec<std::thread::JoinHandle<()>>,
    startup_cancel: Arc<AtomicBool>,
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
    pub startup_threads: Vec<std::thread::JoinHandle<()>>,
    pub startup_cancel: Arc<AtomicBool>,
    pub dropped: Arc<AtomicU64>,
}

impl Handle {
    /// Start with a loading frame while the initial directory is read away
    /// from the UI thread.
    pub fn spawn(cfg: FoldConfig, start: PathBuf, home: PathBuf, trash_available: bool) -> Handle {
        Self::spawn_startup(
            cfg,
            Startup {
                fold: StartupLocation {
                    path: start,
                    fallback: None,
                    label: "Fold",
                },
                commander: None,
            },
            home,
            trash_available,
        )
    }

    pub fn spawn_startup(
        cfg: FoldConfig,
        startup: Startup,
        home: PathBuf,
        trash_available: bool,
    ) -> Handle {
        Self::spawn_with_reader(
            cfg,
            startup,
            home,
            trash_available,
            Arc::new(worker::read_startup),
        )
    }

    fn spawn_with_reader(
        cfg: FoldConfig,
        startup: Startup,
        home: PathBuf,
        trash_available: bool,
        reader: StartupReader,
    ) -> Handle {
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
            startup.fold.path.clone(),
            home,
            trash_available,
        )));

        let mut locations = vec![startup.fold];
        if let Some((dirs, active, enabled)) = startup.commander {
            // The transition creates the panes without doing IO. Its ordinary
            // listing jobs are replaced by independent startup reads below.
            let effects = {
                let mut s = state.write().unwrap_or_else(|e| e.into_inner());
                state::apply(
                    &mut s,
                    Change::Command(Command::RestoreCommander {
                        dirs: [dirs[0].path.clone(), dirs[1].path.clone()],
                        active,
                        enabled,
                    }),
                )
            };
            debug_assert!(effects.jobs.iter().all(|job| matches!(job, Job::List(_))));
            locations.extend(dirs);
        }

        let active_dir = state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .active_frame()
            .dir
            .clone();
        // An explicit CLI location wins if it happens to name the same path
        // as a saved location: it must keep its no-fallback semantics.
        locations
            .sort_by_key(|location| (location.path != active_dir, location.fallback.is_some()));
        let mut seen = std::collections::HashSet::new();
        locations.retain(|location| seen.insert(location.path.clone()));

        let list_cfg = cfg.list.clone();
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

        let startup_cancel = Arc::new(AtomicBool::new(false));
        let startup_threads = locations
            .into_iter()
            .map(|location| {
                let cfg = list_cfg.clone();
                let state = Arc::clone(&state);
                let sink = sink.clone();
                let senders = senders.clone();
                let cancel = Arc::clone(&startup_cancel);
                let reader = Arc::clone(&reader);
                std::thread::Builder::new()
                    .name("starfold-startup".into())
                    .spawn(move || {
                        let done = reader(location, &cfg);
                        if !cancel.load(Ordering::Relaxed) {
                            worker::finish(done, &state, &sink, &senders);
                        }
                    })
                    .expect("spawn startup reader")
            })
            .collect();

        Handle::from_parts(HandleParts {
            senders,
            events: event_rx,
            state,
            sink,
            threads: (Some(io_thread), Some(ops_thread)),
            startup_threads,
            startup_cancel,
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
            startup_threads: parts.startup_threads,
            startup_cancel: parts.startup_cancel,
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
        self.startup_cancel.store(true, Ordering::Relaxed);
        // A sleeping USB drive may keep a read in the kernel. Detach that
        // short-lived thread on exit instead of delaying terminal restore.
        for thread in self.startup_threads.drain(..) {
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
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
            startup_threads: Vec::new(),
            startup_cancel: Arc::new(AtomicBool::new(false)),
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

    /// A sleeping drive cannot hold up the caller. The test reader pauses
    /// before doing any filesystem work; the handle must still return with a
    /// drawable loading frame, then accept the eventual listing.
    #[test]
    fn spawning_returns_before_a_slow_startup_listing() {
        let fixture = crate::fold::testing::Fixture::tree();
        let (release, paused) = crossbeam_channel::bounded::<()>(0);
        let reader: StartupReader = Arc::new(move |location, cfg| {
            paused.recv().unwrap();
            worker::read_startup(location, cfg)
        });
        let started = Instant::now();
        let handle = Handle::spawn_with_reader(
            FoldConfig::default(),
            Startup {
                fold: StartupLocation {
                    path: fixture.home().to_path_buf(),
                    fallback: None,
                    label: "Fold",
                },
                commander: None,
            },
            fixture.home().to_path_buf(),
            false,
            reader,
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(handle.state().loading);
        assert!(handle.state().active_frame().loading);
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while handle.state().loading && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let resolved_home = fixture.home().canonicalize().unwrap();
        assert!(handle.state().listing_of(&resolved_home).is_some());
    }

    #[test]
    fn missing_saved_location_falls_back_after_startup_read() {
        let fixture = crate::fold::testing::Fixture::tree();
        let missing = fixture.path("missing-directory");
        let handle = Handle::spawn_startup(
            FoldConfig::default(),
            Startup {
                fold: StartupLocation {
                    path: missing,
                    fallback: Some(fixture.home().to_path_buf()),
                    label: "Fold",
                },
                commander: None,
            },
            fixture.home().to_path_buf(),
            false,
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while handle.state().loading && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let state = handle.state();
        assert_eq!(state.active_frame().dir, fixture.home());
        assert!(state.listing_of(fixture.home()).is_some());
        drop(state);
        assert!(handle.drain().any(|event| matches!(event, Event::Note(_))));
    }

    #[test]
    fn late_startup_listing_does_not_replace_user_navigation() {
        let fixture = crate::fold::testing::Fixture::tree();
        let original = fixture.path("projects");
        let destination = fixture.path("empty");
        let (release, paused) = crossbeam_channel::bounded::<()>(0);
        let reader: StartupReader = Arc::new(move |location, cfg| {
            paused.recv().unwrap();
            worker::read_startup(location, cfg)
        });
        let handle = Handle::spawn_with_reader(
            FoldConfig::default(),
            Startup {
                fold: StartupLocation {
                    path: original,
                    fallback: None,
                    label: "Fold",
                },
                commander: None,
            },
            fixture.home().to_path_buf(),
            false,
            reader,
        );
        handle.send(Command::Push(destination.clone()));
        let deadline = Instant::now() + Duration::from_secs(2);
        while handle.state().listing_of(&destination).is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(handle.state().listing_of(&destination).is_some());
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !handle.startup_threads[0].is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(handle.state().active_frame().dir, destination);
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
