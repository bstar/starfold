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

use super::create::{self, Kind as CreateKind, Origin as CreateOrigin};
use super::handle::{Event, EventSink, Note, StartupLocation};
use super::listing::{self, ListConfig, Listing};
use super::ops::progress::Progress;
use super::ops::{self, ConflictPolicy, OpId, OpKind, Outcome, Plan};
use super::places::{self, Bookmark, Location};
use super::preview::{self, Preview};
use super::search;
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
    /// Places actions and creation must report queue saturation to the UI.
    pub fn dispatch_checked(&self, job: Job, state: &Arc<RwLock<State>>, events: &EventSink) {
        if !matches!(
            job,
            Job::SaveBookmarks { .. }
                | Job::LoadPlaces(_)
                | Job::RefreshPlaces
                | Job::UnmountPlace { .. }
                | Job::Create { .. }
                | Job::LoadRecovery { .. }
        ) {
            self.dispatch(job);
            return;
        }
        let sender = if matches!(job, Job::UnmountPlace { .. }) {
            &self.ops
        } else {
            &self.io
        };
        if let Err(error) = sender.try_send(job) {
            let message = "Places worker is busy or unavailable; retry the action".to_string();
            let done = match error.into_inner() {
                Job::LoadRecovery { generation, .. } => Done::RecoveryLoaded {
                    generation,
                    result: Err("Recovery worker is busy or unavailable; refresh to retry".into()),
                },
                Job::SaveBookmarks { revision, .. } => Done::BookmarksSaved {
                    revision,
                    result: Err(message),
                },
                Job::LoadPlaces(path) => Done::PlacesLoaded {
                    path,
                    bookmarks: Err(message.clone()),
                    locations: Err(message),
                },
                Job::RefreshPlaces => Done::PlacesRefreshed(Err(message)),
                Job::UnmountPlace { path, .. } => Done::PlaceUnmounted {
                    path,
                    result: Err(message),
                },
                Job::Create {
                    dir,
                    name,
                    kind,
                    origin,
                } => Done::Created {
                    path: dir.join(name),
                    kind,
                    origin,
                    result: Err("creation worker is busy or unavailable; retry the action".into()),
                },
                _ => unreachable!(),
            };
            finish(done, state, events, self);
        }
    }
    /// Send `job` to whichever worker owns its kind of work.
    pub fn dispatch(&self, job: Job) {
        let sender = match &job {
            Job::CheckImport { .. }
            | Job::Plan { .. }
            | Job::Run { .. }
            | Job::RunElevated { .. }
            | Job::UnmountPlace { .. } => &self.ops,
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
    SyncArchiveEdits,
    UnlockArchive {
        location: PathBuf,
        options: starfold_archive_protocol::Options,
    },
    LoadRecovery {
        generation: u64,
        mode: super::recovery::Mode,
        undo: Vec<super::recovery::Record>,
    },
    ClosePreview,
    LoadPlaces(PathBuf),
    RefreshPlaces,
    UnmountPlace {
        path: PathBuf,
        source: PathBuf,
    },
    SaveBookmarks {
        path: PathBuf,
        bookmarks: Vec<Bookmark>,
        revision: u64,
    },
    List(PathBuf),
    Search {
        tab: super::tab::TabId,
        generation: u64,
        root: PathBuf,
        query: String,
        mode: search::Mode,
        include_hidden: bool,
        progress: Arc<search::Progress>,
    },
    Create {
        dir: PathBuf,
        name: String,
        kind: CreateKind,
        origin: CreateOrigin,
    },
    Summarize(PathBuf),
    PreviewInput {
        tab: super::tab::TabId,
        path: PathBuf,
        generation: u64,
        input: starfold_preview_protocol::Input,
    },
    PreviewPage {
        tab: super::tab::TabId,
        path: PathBuf,
        generation: u64,
        page: u32,
    },
    Preview {
        tab: super::tab::TabId,
        path: PathBuf,
        generation: u64,
    },
    /// Ask the desktop, or the configured argv, to open a file.
    Open(PathBuf),
    /// Check remote top-level names before requesting any file contents.
    CheckImport {
        op: OpId,
        sources: Vec<PathBuf>,
        dest: PathBuf,
    },
    /// Expand an op's sources, total their bytes, and find conflicts.
    Plan {
        archive_options: starfold_archive_protocol::Options,
        op: OpId,
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: Option<PathBuf>,
        expected: Vec<(PathBuf, search::Identity)>,
    },
    /// Execute a planned op. `progress` is the same `Arc` the `Op` in
    /// `State` holds, which is how the status row sees the bytes move.
    Run {
        recovery: Option<Arc<super::recovery::Guard>>,
        op: OpId,
        kind: OpKind,
        plan: Plan,
        policy: ConflictPolicy,
        rename_targets: Vec<(PathBuf, PathBuf)>,
        progress: Arc<Progress>,
        expected: Vec<(PathBuf, search::Identity)>,
    },
    RunElevated {
        op: OpId,
        sources: Vec<PathBuf>,
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
    ArchiveOpened {
        path: PathBuf,
        location: PathBuf,
    },
    ArchiveEditsSynced {
        directories: Vec<PathBuf>,
        error: Option<String>,
    },
    ArchivePreviewed {
        tab: super::tab::TabId,
        path: PathBuf,
        local: PathBuf,
        editable: bool,
        generation: u64,
        preview: Preview,
    },
    RecoveryLoaded {
        generation: u64,
        result: Result<Vec<super::recovery::Item>, String>,
    },
    StartupListed {
        requested: PathBuf,
        listing: Listing,
        notice: Option<String>,
    },
    PlacesLoaded {
        path: PathBuf,
        bookmarks: Result<Vec<Bookmark>, String>,
        locations: Result<Vec<Location>, String>,
    },
    PlacesRefreshed(Result<Vec<Location>, String>),
    PlaceUnmounted {
        path: PathBuf,
        result: Result<Vec<Location>, String>,
    },
    BookmarksSaved {
        revision: u64,
        result: Result<(), String>,
    },
    Listed(Listing),
    Searched {
        tab: super::tab::TabId,
        generation: u64,
        found: Arc<search::Found>,
    },
    Created {
        path: PathBuf,
        kind: CreateKind,
        origin: CreateOrigin,
        result: Result<(), String>,
    },
    Summarized {
        dir: PathBuf,
        summary: DirSummary,
    },
    Previewed {
        tab: super::tab::TabId,
        path: PathBuf,
        generation: u64,
        preview: Preview,
    },
    Planned {
        op: OpId,
        result: Result<Plan, String>,
    },
    ImportChecked {
        op: OpId,
        result: Result<Vec<ops::Conflict>, String>,
    },
    Finished {
        op: OpId,
        outcome: Outcome,
    },
    /// Something on disk changed under one of these paths, other than
    /// through this program's own operations -- the mtime poll's finding.
    Changed(Vec<PathBuf>),
}

/// Validate and read one startup location without delaying the first frame.
/// Saved paths may disappear while STAR/FOLD is closed; explicit paths retain
/// their ordinary listing error instead of silently changing destination.
pub fn read_startup(location: StartupLocation, cfg: &ListConfig) -> Done {
    let requested = location.path;
    if super::location::is_archive(&requested) {
        return Done::StartupListed {
            listing: listing::read(&requested, cfg),
            requested,
            notice: None,
        };
    }
    let (actual, notice) = if let Some(fallback) = location.fallback {
        if requested.is_dir() {
            (
                requested
                    .canonicalize()
                    .unwrap_or_else(|_| requested.clone()),
                None,
            )
        } else {
            let notice = format!(
                "saved {} location {} is unavailable; opened {}",
                location.label,
                requested.display(),
                fallback.display()
            );
            (fallback, Some(notice))
        }
    } else {
        (
            requested
                .canonicalize()
                .unwrap_or_else(|_| requested.clone()),
            None,
        )
    };
    let mut listing = listing::read(&actual, cfg);
    if !super::location::is_archive(&actual) {
        listing.space = places::filesystem_space(&actual);
    }
    Done::StartupListed {
        requested,
        listing,
        notice,
    }
}

/// Poll only the visible workspace. Switching tabs revalidates its cached
/// listings, so unavailable background volumes cannot block active browsing.
fn watched_dirs(state: &Arc<RwLock<State>>) -> Vec<PathBuf> {
    let s = state.read().unwrap_or_else(|e| e.into_inner());
    let mut dirs: Vec<PathBuf> = s
        .tabs
        .active()
        .stacks
        .iter()
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

fn is_stale_tab_preview(
    state: &Arc<RwLock<State>>,
    tab: super::tab::TabId,
    generation: u64,
) -> bool {
    let s = state.read().unwrap_or_else(|e| e.into_inner());
    s.tabs.active().id != tab || generation < s.preview_generation
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
        senders.dispatch_checked(job, state, events);
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
// Keep the worker’s owned result inline; all variants share one dispatch path.
#[allow(clippy::large_enum_variant)]
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
        Job::SyncArchiveEdits => {
            let result = super::archive::edit::sync_working();
            let (sources, error) = match result {
                Ok(s) => (s, None),
                Err(e) => (vec![], Some(e.to_string())),
            };
            let directories = {
                let state = state.read().unwrap_or_else(|e| e.into_inner());
                state.listings.keys().filter(|key| matches!(super::location::Location::from_key(key),Ok(super::location::Location::Archive{source,..}) if sources.iter().any(|changed|changed.file==source.file && changed.nested.starts_with(&source.nested)))).cloned().collect()
            };
            IoOutcome::Done(Done::ArchiveEditsSynced { directories, error })
        }
        Job::UnlockArchive { location, options } => {
            if let Ok(super::location::Location::Archive { source, .. }) =
                super::location::Location::from_key(&location)
            {
                super::archive::browser::unlock(
                    source.file.clone(),
                    options.password.unwrap_or_default(),
                );
                super::archive::browser::invalidate(&source);
            }
            IoOutcome::Done(Done::Listed(listing::read(&location, &cfg.list)))
        }
        Job::LoadRecovery {
            generation,
            mode,
            undo,
        } => IoOutcome::Done(Done::RecoveryLoaded {
            generation,
            result: super::recovery::list(mode, undo, cfg.recovery_dir.as_deref()),
        }),
        Job::LoadPlaces(path) => IoOutcome::Done(Done::PlacesLoaded {
            bookmarks: places::load_bookmarks(&path),
            locations: places::discover_locations(),
            path,
        }),
        Job::RefreshPlaces => IoOutcome::Done(Done::PlacesRefreshed(places::discover_locations())),
        Job::SaveBookmarks {
            path,
            bookmarks,
            revision,
        } => IoOutcome::Done(Done::BookmarksSaved {
            revision,
            result: places::save_bookmarks(&path, &bookmarks),
        }),
        Job::List(dir) => {
            let mut listing = listing::read(&dir, &cfg.list);
            if !super::location::is_archive(&dir) {
                listing.space = places::filesystem_space(&dir);
            }
            IoOutcome::Done(Done::Listed(listing))
        }
        Job::Search {
            tab,
            generation,
            root,
            query,
            mode,
            include_hidden,
            progress,
        } => IoOutcome::Done(Done::Searched {
            tab,
            generation,
            found: Arc::new(search::scan_mode(
                &root,
                &query,
                mode,
                include_hidden,
                &progress,
            )),
        }),
        Job::Create {
            dir,
            name,
            kind,
            origin,
        } => IoOutcome::Done(Done::Created {
            path: dir.join(&name),
            kind,
            origin,
            result: create::create(kind, &dir, &name).map(|_| ()),
        }),
        Job::Summarize(dir) => {
            let budget = Budget {
                max_entries: cfg.preview.dir_budget,
                max_depth: 64,
            };
            let summary = if super::location::is_archive(&dir) {
                super::archive::browser::summary(&dir).unwrap_or_default()
            } else {
                summary::summarize(&dir, &budget, cancel)
            };
            IoOutcome::Done(Done::Summarized { dir, summary })
        }
        Job::Preview {
            tab,
            path,
            generation,
        } => {
            // Builds are synchronous on this one thread, so a preview cannot
            // be cancelled *mid*-build the way a running op can be; what
            // matters is not starting a build for a file the cursor has
            // already left, which this check catches before any bytes are
            // read.
            if is_stale_tab_preview(state, tab, generation) {
                return IoOutcome::None;
            }
            let preview = preview::build(&path, &cfg.preview, cancel);
            IoOutcome::Done(Done::Previewed {
                tab,
                path,
                generation,
                preview,
            })
        }
        Job::PreviewInput {
            tab,
            path,
            generation,
            input,
        } => {
            if is_stale_tab_preview(state, tab, generation) {
                return IoOutcome::None;
            }
            let mut connection =
                preview::connection::Connection::with_registry(cfg.extensions.clone());
            let stale = || {
                cancel.load(std::sync::atomic::Ordering::Relaxed)
                    || is_stale_tab_preview(state, tab, generation)
            };
            match connection.input(&path, generation, input, &cfg.preview, &stale) {
                Some(preview) => IoOutcome::Done(Done::Previewed {
                    tab,
                    path,
                    generation,
                    preview,
                }),
                None => IoOutcome::None,
            }
        }
        Job::PreviewPage {
            tab,
            path,
            generation,
            page,
        } => {
            if is_stale_tab_preview(state, tab, generation) {
                return IoOutcome::None;
            }
            let mut head = [0; 512];
            use std::io::Read;
            let n = std::fs::File::open(&path)
                .and_then(|mut f| f.read(&mut head))
                .unwrap_or(0);
            let preview = preview::providers::build(&path, &head[..n], page, &cfg.preview)
                .map(Preview::Document)
                .unwrap_or(Preview::Empty);
            IoOutcome::Done(Done::Previewed {
                tab,
                path,
                generation,
                preview,
            })
        }
        Job::Open(path) => {
            let resolved = if super::location::is_archive(&path) {
                match super::archive::browser::materialize(&path, &Progress::new(0)) {
                    Ok(p) => p,
                    Err(e) => return IoOutcome::Note(Note::error("archive", e.to_string())),
                }
            } else {
                path.clone()
            };
            if super::archive::service::inspect(&resolved).is_ok() {
                if let Ok(location) =
                    super::location::Location::from_key(&path).and_then(|l| l.enter_archive())
                {
                    return IoOutcome::Done(Done::ArchiveOpened {
                        path,
                        location: location.key(),
                    });
                }
            }
            match open::open_external(&resolved, &cfg.open) {
                Ok(()) => IoOutcome::None,
                Err(e) => IoOutcome::Note(Note::error(
                    "open",
                    format!("opening {}: {e}", path.display()),
                )),
            }
        }
        // Not io work: `spawn_io`'s loop handles `Shutdown` and forwards
        // `Plan`/`Run` to the ops queue before this is ever called. Kept
        // here, rather than assumed away, so this match stays exhaustive if
        // `Job` grows another kind.
        Job::ClosePreview
        | Job::Shutdown
        | Job::CheckImport { .. }
        | Job::Plan { .. }
        | Job::Run { .. }
        | Job::RunElevated { .. }
        | Job::UnmountPlace { .. } => IoOutcome::None,
    }
}

/// Do the filesystem work for one `starfold-ops` job: plan a queued
/// operation's sources and conflicts, or run one that is already planned.
///
/// Pulled out of [`spawn_ops`]'s loop for the same reason as [`perform_io`].
/// `cfg.preserve_times` is the one setting a run needs that a plan does not.
fn validate_search_sources_for_run(
    expected: &[(PathBuf, search::Identity)],
) -> Result<(), (PathBuf, String)> {
    for (path, identity) in expected {
        if search::Identity::of(path).ok() != Some(*identity) {
            return Err((
                path.clone(),
                format!(
                    "search result changed since it was found: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

pub fn perform_ops(job: Job, cfg: &FoldConfig) -> Option<Done> {
    match job {
        Job::CheckImport { op, sources, dest } => {
            if super::location::is_archive(&dest) {
                return Some(Done::ImportChecked {
                    op,
                    result: super::archive::edit::import_conflicts(&sources, &dest)
                        .map_err(|error| error.to_string()),
                });
            }
            let result = sources
                .iter()
                .map(|source| {
                    let name = source
                        .file_name()
                        .ok_or_else(|| "drop has no file name".to_string())?;
                    let target = dest.join(name);
                    match std::fs::symlink_metadata(&target) {
                        Ok(_) => Ok(Some(ops::Conflict {
                            source: source.clone(),
                            dest: target,
                            both_dirs: false,
                        })),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                        Err(error) => Err(format!("{}: {error}", target.display())),
                    }
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|items| items.into_iter().flatten().collect());
            Some(Done::ImportChecked { op, result })
        }
        Job::Plan {
            op,
            kind,
            sources,
            dest,
            expected,
            archive_options,
        } => {
            let result = validate_search_sources_for_run(&expected)
                .map_err(|(_, e)| e)
                .and_then(|_| {
                    ops::plan::plan(kind, &sources, dest.as_deref())
                        .map(|mut plan| {
                            if matches!(kind, ops::OpKind::Compress(_))
                                && (archive_options.password.is_some()
                                    || archive_options.volume_bytes.is_some())
                            {
                                plan.total_bytes = plan.total_bytes.saturating_mul(2);
                            }
                            plan.archive_options = archive_options;
                            plan
                        })
                        .map_err(|e| e.to_string())
                });
            Some(Done::Planned { op, result })
        }
        Job::Run {
            recovery,
            op,
            kind,
            plan,
            policy,
            rename_targets,
            progress,
            expected,
        } => {
            if let Err((path, message)) = validate_search_sources_for_run(&expected) {
                return Some(Done::Finished {
                    op,
                    outcome: Outcome {
                        failed: vec![(path, message)],
                        ..Outcome::default()
                    },
                });
            }
            let options = ops::exec::RunOptions {
                preserve_times: cfg.preserve_times,
                force_copy: false,
            };
            let mut outcome = if let Some(guard) = &recovery {
                super::recovery::run(guard, &plan.dest, &progress)
            } else if kind == OpKind::Delete(ops::DeleteHow::Trash)
                && !plan
                    .sources
                    .iter()
                    .any(|path| super::location::is_archive(path))
            {
                let mut outcome = Outcome::default();
                progress.set_total(plan.sources.len() as u64);
                for source in &plan.sources {
                    if plan.missing.contains(source) {
                        outcome.skipped += 1;
                        continue;
                    }
                    if progress.is_cancelled() {
                        outcome.cancelled = true;
                        break;
                    }
                    match super::recovery::trash_one(source, cfg.recovery_dir.as_deref()) {
                        Ok(()) => {
                            outcome.done += 1;
                            progress.add(1);
                        }
                        Err(error) => outcome.failed.push((source.clone(), error)),
                    }
                }
                outcome
            } else {
                ops::exec::run_with_names(kind, &plan, policy, &options, &progress, &rename_targets)
            };
            if recovery.is_none() {
                outcome.reversible = super::recovery::receipt(kind, &plan, &outcome);
            }
            for (path, reason) in &outcome.failed {
                tracing::warn!(operation = ?kind, path = %path.display(), %reason, "file operation failed");
            }
            Some(Done::Finished { op, outcome })
        }
        Job::RunElevated {
            op,
            sources,
            progress,
        } => Some(Done::Finished {
            op,
            outcome: super::elevated::run(&sources, &progress),
        }),
        Job::UnmountPlace { path, source } => Some(Done::PlaceUnmounted {
            result: places::unmount_location(&path, &source),
            path,
        }),
        // Not ops work: `spawn_ops`'s loop forwards everything else before
        // this is called.
        Job::UnlockArchive { .. }
        | Job::LoadRecovery { .. }
        | Job::LoadPlaces(_)
        | Job::RefreshPlaces
        | Job::SaveBookmarks { .. }
        | Job::List(_)
        | Job::Search { .. }
        | Job::Create { .. }
        | Job::Summarize(_)
        | Job::Preview { .. }
        | Job::PreviewPage { .. }
        | Job::PreviewInput { .. }
        | Job::Open(_)
        | Job::ClosePreview
        | Job::SyncArchiveEdits
        | Job::Shutdown => None,
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
            let (preview_tx, preview_rx) = crossbeam_channel::bounded::<Job>(32);
            let stop = Arc::new(AtomicBool::new(false));
            let pending_previews = preview_rx.clone();
            let preview_thread = spawn_preview(
                preview_rx,
                state.clone(),
                events.clone(),
                senders.clone(),
                cfg.preview,
                cfg.extensions.clone(),
                stop.clone(),
            );

            // Searches retain their owning tab, but do not hold up directory
            // reads or operation requests when the user switches workspaces.
            let (search_tx, search_rx) = crossbeam_channel::bounded::<Job>(256);
            let search_state = state.clone();
            let search_events = events.clone();
            let search_senders = senders.clone();
            let search_cfg = cfg.clone();
            let search_stop = stop.clone();
            let search_thread = std::thread::Builder::new()
                .name("starfold-search".into())
                .spawn(move || {
                    while let Ok(job) = search_rx.recv() {
                        if search_stop.load(std::sync::atomic::Ordering::Relaxed) {
                            break;
                        }
                        match perform_io(job, &search_cfg, &search_state, &search_stop) {
                            IoOutcome::Done(done) => {
                                finish(done, &search_state, &search_events, &search_senders)
                            }
                            IoOutcome::Note(note) => search_events.send(Event::Note(note)),
                            IoOutcome::None => {}
                        }
                    }
                })
                .expect("spawning search thread");

            loop {
                let job = match jobs.recv_timeout(POLL) {
                    Ok(job) => job,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        let mut dirs = watched_dirs(&state);
                        let selected = state
                            .read()
                            .unwrap_or_else(|e| e.into_inner())
                            .cursor_entry()
                            .map(|e| e.path.clone());
                        if let Some(path) = &selected {
                            dirs.push(path.clone());
                        }
                        watch.watch(&dirs);
                        let mut changed = watch.changed();
                        for path in &mut changed {
                            if Some(&*path) == selected.as_ref() {
                                if let Some(parent) = path.parent() {
                                    *path = parent.to_path_buf();
                                }
                            }
                        }
                        changed.sort();
                        changed.dedup();
                        if !changed.is_empty() {
                            finish(Done::Changed(changed), &state, &events, &senders);
                        }
                        continue;
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                };

                match job {
                    Job::Shutdown => break,
                    job @ Job::Search { .. } => {
                        if let Err(error) = search_tx.try_send(job) {
                            if let Job::Search {
                                tab,
                                generation,
                                progress,
                                ..
                            } = error.into_inner()
                            {
                                progress
                                    .cancel
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                                finish(
                                    Done::Searched {
                                        tab,
                                        generation,
                                        found: Arc::new(search::Found {
                                            entries: vec![],
                                            identities: Default::default(),
                                            excerpts: Default::default(),
                                            skipped_binary: 0,
                                            skipped_large: 0,
                                            status: search::Status::Cancelled,
                                            errors: vec![
                                                "Search queue is full; retry the search".into()
                                            ],
                                        }),
                                    },
                                    &state,
                                    &events,
                                    &senders,
                                );
                            }
                        }
                    }
                    job @ (Job::Preview { .. }
                    | Job::PreviewPage { .. }
                    | Job::PreviewInput { .. }
                    | Job::ClosePreview) => {
                        // Obsolete requests are skipped by the preview worker.
                        if let Err(crossbeam_channel::TrySendError::Full(job)) =
                            preview_tx.try_send(job)
                        {
                            let _ = pending_previews.try_recv();
                            let _ = preview_tx.try_send(job);
                        }
                    }
                    other @ (Job::CheckImport { .. }
                    | Job::Plan { .. }
                    | Job::Run { .. }
                    | Job::RunElevated { .. }
                    | Job::UnmountPlace { .. }) => {
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
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            {
                let state = state.read().unwrap_or_else(|e| e.into_inner());
                if let Some(search) = &state.search {
                    search
                        .progress
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                for tab in &state.tabs.tabs {
                    if let Some(search) = tab.context.as_ref().and_then(|c| c.search.as_ref()) {
                        search
                            .progress
                            .cancel
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            drop(search_tx);
            drop(preview_tx);
            let _ = search_thread.join();
            let _ = preview_thread.join();
        })
        .expect("spawning the io thread")
}

/// Shared by the preview supervisor and the threadless UI fixture.
pub fn perform_preview(
    job: Job,
    connection: &mut preview::connection::Connection,
    cfg: &preview::PreviewConfig,
    state: &Arc<RwLock<State>>,
    stop: &AtomicBool,
) -> Option<Done> {
    if matches!(job, Job::ClosePreview) {
        connection.close();
        return None;
    }
    let (tab, path, generation, page, input) = match job {
        Job::Preview {
            tab,
            path,
            generation,
        } => (tab, path, generation, 1, None),
        Job::PreviewPage {
            tab,
            path,
            generation,
            page,
        } => (tab, path, generation, page, None),
        Job::PreviewInput {
            tab,
            path,
            generation,
            input,
        } => (tab, path, generation, 1, Some(input)),
        _ => return None,
    };
    let stale = || {
        stop.load(std::sync::atomic::Ordering::Relaxed)
            || is_stale_tab_preview(state, tab, generation)
    };
    let resolved = if super::location::is_archive(&path) {
        if matches!(
            super::location::Location::from_key(&path),
            Ok(super::location::Location::Archive { member: None, .. })
        ) {
            let listing =
                super::archive::browser::read(&path, &super::listing::ListConfig::default());
            let mut document = super::preview::model::Document::new("Archive folder");
            document.content = super::preview::model::Content::Archive(
                listing
                    .entries
                    .iter()
                    .map(|e| super::archive::Entry {
                        name: e.display.clone(),
                        bytes: Some(e.len),
                        directory: e.kind == super::entry::EntryKind::Dir,
                    })
                    .collect(),
            );
            document.notice = listing.error;
            return Some(Done::Previewed {
                tab,
                path,
                generation,
                preview: Preview::Document(document),
            });
        }
        match super::archive::browser::materialize_cancellable(&path, &stale) {
            Ok(file) => {
                use std::io::Read;
                let mut head = [0; 512];
                let n = std::fs::File::open(&file)
                    .and_then(|mut f| f.read(&mut head))
                    .unwrap_or(0);
                let text_member = super::preview::is_editable_content(&path, &head[..n]);
                if text_member
                    && super::archive::edit::validate(&match super::location::Location::from_key(
                        &path,
                    )
                    .ok()?
                    {
                        super::location::Location::Archive { source, .. } => source,
                        _ => unreachable!(),
                    })
                    .is_ok()
                {
                    match super::archive::edit::working_copy(&path, &file) {
                        Ok(p) => p,
                        Err(error) => {
                            return Some(Done::Previewed {
                                tab,
                                path,
                                generation,
                                preview: Preview::Error(error.to_string()),
                            })
                        }
                    }
                } else if text_member {
                    // A configured editor must never modify a read-only
                    // extraction cache. Show bounded text without starting
                    // an interactive provider; extraction remains available.
                    let mut document =
                        super::preview::model::Document::new("Archive member · read-only");
                    document.notice = Some("Extract this member to edit it".into());
                    let mut bytes = Vec::new();
                    if let Ok(input) = std::fs::File::open(&file) {
                        let _ = input.take(cfg.max_bytes).read_to_end(&mut bytes);
                    }
                    document.content =
                        super::preview::model::Content::Pages(vec![super::preview::model::Page {
                            number: 1,
                            text: String::from_utf8_lossy(&bytes)
                                .lines()
                                .take(cfg.max_lines)
                                .collect::<Vec<_>>()
                                .join("\n"),
                            truncated: std::fs::metadata(&file)
                                .is_ok_and(|m| m.len() > bytes.len() as u64),
                        }]);
                    let preview = preview::archive_member_preview(
                        Preview::Document(document),
                        &file,
                        &path,
                        cfg,
                    );
                    return Some(Done::ArchivePreviewed {
                        tab,
                        path,
                        local: file,
                        editable: false,
                        generation,
                        preview,
                    });
                } else {
                    file
                }
            }
            Err(error) => {
                return Some(Done::Previewed {
                    tab,
                    path,
                    generation,
                    preview: Preview::Error(error.to_string()),
                })
            }
        }
    } else {
        path.clone()
    };
    let result = match input {
        Some(input) => connection.input(&resolved, generation, input, cfg, &stale),
        None => connection.build_scoped(&resolved, page, generation, cfg, &stale),
    };
    result.map(|preview| {
        if super::location::is_archive(&path) {
            let preview = preview::archive_member_preview(preview, &resolved, &path, cfg);
            Done::ArchivePreviewed {
                tab,
                editable: super::archive::edit::is_working(&path),
                path,
                local: resolved,
                generation,
                preview,
            }
        } else {
            Done::Previewed {
                tab,
                path,
                generation,
                preview,
            }
        }
    })
}

fn spawn_preview(
    jobs: crossbeam_channel::Receiver<Job>,
    state: Arc<RwLock<State>>,
    events: EventSink,
    senders: Senders,
    cfg: preview::PreviewConfig,
    registry: preview::extensions::Registry,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("starfold-preview".into())
        .spawn(move || {
            let mut connection = preview::connection::Connection::with_registry(registry);
            while let Ok(job) = jobs.recv() {
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                if let Some(done) = perform_preview(job, &mut connection, &cfg, &state, &stop) {
                    finish(done, &state, &events, &senders);
                }
            }
        })
        .expect("spawning preview supervisor")
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
                    other @ (Job::CheckImport { .. }
                    | Job::Plan { .. }
                    | Job::Run { .. }
                    | Job::RunElevated { .. }
                    | Job::UnmountPlace { .. }) => {
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

    #[test]
    fn import_check_finds_destination_name_before_transfer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("photo.jpg"), b"old").unwrap();
        let source = PathBuf::from("/remote/photo.jpg");
        let done = perform_ops(
            Job::CheckImport {
                op: OpId(1),
                sources: vec![source.clone()],
                dest: dir.path().to_path_buf(),
            },
            &FoldConfig::default(),
        )
        .unwrap();
        let Done::ImportChecked {
            result: Ok(conflicts),
            ..
        } = done
        else {
            panic!("expected import check")
        };
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].source, source);
        assert_eq!(conflicts[0].dest, dir.path().join("photo.jpg"));
    }

    #[test]
    fn queued_search_result_is_rechecked_before_plan_and_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("found.txt");
        std::fs::write(&path, b"old").unwrap();
        let identity = search::Identity::of(&path).unwrap();
        let replacement = dir.path().join("replacement.tmp");
        std::fs::write(&replacement, b"replacement").unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let expected = vec![(path.clone(), identity)];
        let planned = perform_ops(
            Job::Plan {
                archive_options: Default::default(),
                op: OpId(1),
                kind: OpKind::Delete(ops::DeleteHow::Permanent),
                sources: vec![path.clone()],
                dest: None,
                expected: expected.clone(),
            },
            &FoldConfig::default(),
        );
        assert!(matches!(
            planned,
            Some(Done::Planned { result: Err(_), .. })
        ));
        let finished = perform_ops(
            Job::Run {
                recovery: None,
                op: OpId(1),
                kind: OpKind::Delete(ops::DeleteHow::Permanent),
                plan: Plan::default(),
                policy: ConflictPolicy::Ask,
                rename_targets: vec![],
                progress: Arc::new(Progress::new(0)),
                expected,
            },
            &FoldConfig::default(),
        );
        assert!(
            matches!(finished, Some(Done::Finished { outcome, .. }) if outcome.done == 0 && outcome.failed.len() == 1)
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
    }
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
    fn a_saturated_worker_reports_bookmark_save_failure() {
        let (io, _io_rx) = crossbeam_channel::bounded(0);
        let (ops, _ops_rx) = crossbeam_channel::bounded(1);
        let senders = Senders { io, ops };
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let events = EventSink::new(event_tx, Arc::new(AtomicU64::new(0)));
        let state = Arc::new(RwLock::new(State::new(
            &FoldConfig::default(),
            "/".into(),
            "/".into(),
            true,
        )));
        senders.dispatch_checked(
            Job::SaveBookmarks {
                path: "/unused/bookmarks.toml".into(),
                bookmarks: vec![],
                revision: 0,
            },
            &state,
            &events,
        );
        assert!(state
            .read()
            .unwrap()
            .places
            .error
            .as_deref()
            .unwrap()
            .contains("retry"));
        assert!(event_rx
            .try_iter()
            .any(|event| matches!(event, Event::Note(_))));
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
            state
                .read()
                .unwrap()
                .listing_of(dir.path())
                .and_then(|listing| listing.space)
                .is_some_and(|(total, available)| total > 0 && available <= total),
            "a listed pane receives its filesystem capacity"
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
    fn senders_dispatch_routes_operations_to_ops_and_everything_else_to_io() {
        let (senders, io_rx, ops_rx) = senders();

        senders.dispatch(Job::List("/a".into()));
        senders.dispatch(Job::Summarize("/a".into()));
        senders.dispatch(Job::Open("/a".into()));
        senders.dispatch(Job::Preview {
            tab: super::super::tab::TabId(0),
            path: "/a".into(),
            generation: 0,
        });
        assert_eq!(io_rx.try_iter().count(), 4);
        assert_eq!(ops_rx.try_iter().count(), 0);

        senders.dispatch(Job::Plan {
            archive_options: Default::default(),
            op: OpId(0),
            kind: OpKind::Copy,
            sources: vec!["/a".into()],
            dest: Some("/b".into()),
            expected: vec![],
        });
        senders.dispatch(Job::Run {
            recovery: None,
            op: OpId(0),
            kind: OpKind::Copy,
            plan: Plan::default(),
            policy: ConflictPolicy::Ask,
            rename_targets: vec![],
            progress: Arc::new(Progress::new(0)),
            expected: vec![],
        });
        senders.dispatch(Job::UnmountPlace {
            path: "/media/usb".into(),
            source: "/dev/sdb1".into(),
        });
        assert_eq!(ops_rx.try_iter().count(), 3);
    }

    #[test]
    fn ops_worker_reports_a_stale_unmount_instead_of_forwarding_it_back_to_itself() {
        let dir = tempfile::tempdir().unwrap();
        let (io, _io_rx) = crossbeam_channel::unbounded();
        let (ops, ops_rx) = crossbeam_channel::unbounded();
        let senders = Senders {
            io,
            ops: ops.clone(),
        };
        let (event_tx, _event_rx) = crossbeam_channel::unbounded();
        let events = EventSink::new(event_tx, Arc::new(std::sync::atomic::AtomicU64::new(0)));
        let state = Arc::new(RwLock::new(State::new(
            &FoldConfig::default(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            true,
        )));
        let thread = spawn_ops(
            ops_rx,
            events,
            Arc::clone(&state),
            FoldConfig::default(),
            senders,
        );
        ops.send(Job::UnmountPlace {
            path: dir.path().join("absent"),
            source: "/dev/sdb1".into(),
        })
        .unwrap();
        ops.send(Job::Shutdown).unwrap();
        thread.join().unwrap();
        assert!(state.read().unwrap().places.locations_error.is_some());
    }
}
