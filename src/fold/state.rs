//! The truth, and the one function that is allowed to change it.
//!
//! Everything the UI draws and everything a worker reads comes out of
//! [`State`], held behind the one `RwLock` [`super::handle::Handle`] owns.
//! [`apply`] is the *only* writer, and it is a pure function: given the state
//! as it was and a [`Change`], it returns the state as it now is and the
//! [`super::worker::Job`]s that change implies -- no filesystem call, no
//! thread spawned, nothing that can block the caller holding the write lock.
//! That is what lets a worker take the lock just long enough to fold its
//! result in and let go again.
//!
//! `apply` is one `match` per [`Command`] and one per [`Done`], each in its
//! own small function below rather than inline: the arms do not share logic
//! so much as they share a handful of small moves -- rebuild a frame's rows,
//! ask for a listing if one is not cached, find every frame open on a
//! directory -- and those moves are named functions of their own so an arm
//! reads as what it does rather than how.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::create::{self, Kind as CreateKind, Origin as CreateOrigin};
use super::entry::{Entry, EntryKind};
use super::filter;
use super::handle::{Command, Event, Note};
use super::listing::Listing;
use super::ops::{ConflictPolicy, DeleteHow, OpId, OpKind, OpStatus, Queue};
use super::places::{self, Bookmark};
use super::preview::Preview;
use super::search::{self, Search, Status as SearchStatus};
use super::selection::Selection;
use super::sort::{self, SortOrder};
use super::stack::{Frame, ParentMove, Stack};
use super::summary::DirSummary;
use super::tab::{Context as TabContext, Tab, TabId, Tabs};
mod workspace;
use super::worker::{Done, Job};
use super::{FoldConfig, TrashMode};

/// The whole of what the program knows, right now.
pub struct State {
    pub places: places::PlacesState,
    pub commander: bool,
    pub commander_pane: usize,
    parked_selections: [Selection; 3],
    closed_tabs: Vec<crate::session::TabSession>,
    durable_tabs: bool,
    pending_search: Option<crate::session::SearchSession>,
    pub tabs: Tabs,
    /// Every directory read so far, keyed by path. A frame's own listing is
    /// looked up here rather than carried on the frame, so two frames open on
    /// the same directory -- a fold and its own child, briefly -- share one
    /// read rather than paying for it twice.
    pub listings: HashMap<PathBuf, Arc<Listing>>,
    pub selection: Selection,
    /// Paths saved by `y` for repeated `p` copies across navigation.
    pub yanked: Vec<PathBuf>,
    /// Search identities travel with a yank even after results and marks close.
    yanked_search_identities: HashMap<PathBuf, search::Identity>,
    /// Search identities retained for marked results after the search view
    /// closes, until those marks are removed or the operation finishes.
    marked_search_identities: HashMap<PathBuf, search::Identity>,
    pub search: Option<Search>,
    search_generation: u64,
    pub queue: Queue,
    /// What the PREVIEW panel shows, and which path it was built for -- the
    /// panel's title says the name, and a preview that arrived for a file the
    /// cursor has since left is told apart from the current one by the path
    /// rather than trusted.
    pub preview: Option<(PathBuf, Arc<Preview>)>,
    pub archive_materialized: HashMap<PathBuf, PathBuf>,
    pub archive_editable: std::collections::HashSet<PathBuf>,
    pub sort: SortOrder,
    /// Commander panes keep independent ordering from Fold and each other.
    pub pane_sorts: [SortOrder; 2],
    pub show_hidden: bool,
    /// Probed once at [`Handle::spawn`](super::handle::Handle::spawn); a
    /// delete's confirmation and the trash-or-permanent question both read
    /// this rather than re-checking the filesystem on every `d`.
    pub trash_available: bool,
    /// `[ops] trash` from the config: whether a delete reaches for the trash
    /// at all, combined with `trash_available` in `cmd_queue_delete` to
    /// decide `DeleteHow` once, at queue time.
    pub trash: TrashMode,
    /// `[ops] conflicts`: the policy a freshly queued copy, move or rename
    /// starts with, until `Command::SetPolicy` answers a conflict `plan`
    /// found.
    pub conflicts: ConflictPolicy,
    /// `[ops] preserve_times`. Not read by anything in this file -- `exec`
    /// takes it from the `FoldConfig` the ops thread was started with -- but
    /// kept here too so a later settings panel has one place that holds
    /// every `[ops]` value the session is running with.
    pub preserve_times: bool,
    pub home: PathBuf,
    /// Bumped by [`apply`] on every change the UI's drawn view depends on, so
    /// the render loop can tell "nothing happened this frame" from "go copy a
    /// fresh `ViewData` out" without comparing the whole structure.
    pub version: u64,
    /// Bumped on every `Command::Preview`; a `Done::Previewed` carrying an
    /// older generation was built for a file the cursor has since left and is
    /// dropped rather than shown.
    pub preview_generation: u64,
    /// Explicit browser commands, excluding pane focus and worker completions.
    navigation_generation: u64,
    /// Set until the start directory's first listing lands.
    pub loading: bool,
}

impl State {
    pub fn navigation_generation(&self) -> u64 {
        self.navigation_generation
    }

    pub fn selection_for_stack(&self, index: usize) -> &Selection {
        if index == self.tabs.active().active_stack {
            &self.selection
        } else {
            &self.parked_selections[index]
        }
    }
    /// One tab, one stack, one frame open on `start`.
    pub fn new(cfg: &FoldConfig, start: PathBuf, home: PathBuf, trash_available: bool) -> Self {
        Self {
            places: places::PlacesState::default(),
            commander: false,
            commander_pane: 0,
            parked_selections: Default::default(),
            closed_tabs: Vec::new(),
            durable_tabs: false,
            pending_search: None,
            tabs: Tabs::single(Stack::new(start)),
            listings: HashMap::new(),
            selection: Selection::default(),
            yanked: Vec::new(),
            yanked_search_identities: HashMap::new(),
            marked_search_identities: HashMap::new(),
            search: None,
            search_generation: 0,
            queue: Queue::new(),
            preview: None,
            archive_materialized: HashMap::new(),
            archive_editable: Default::default(),
            sort: cfg.list.sort,
            pane_sorts: [cfg.list.sort; 2],
            show_hidden: cfg.list.show_hidden,
            trash_available,
            trash: cfg.trash,
            conflicts: cfg.conflicts,
            preserve_times: cfg.preserve_times,
            home,
            version: 0,
            preview_generation: 0,
            navigation_generation: 0,
            loading: true,
        }
    }

    pub fn active_frame(&self) -> &Frame {
        self.tabs.active().active_stack().active()
    }

    pub fn active_sort(&self) -> SortOrder {
        sort_for_stack(self.tabs.active().active_stack, self.sort, self.pane_sorts)
    }

    pub fn active_frame_mut(&mut self) -> &mut Frame {
        self.tabs.active_mut().active_stack_mut().active_mut()
    }

    pub fn listing_of(&self, dir: &Path) -> Option<&Arc<Listing>> {
        self.listings.get(dir)
    }

    /// The active stack's levels, root to the level currently open -- what
    /// the column's folded rows and the active one are drawn from.
    pub fn crumbs(&self) -> &[Frame] {
        self.tabs.active().active_stack().crumbs()
    }

    /// The active frame's own listing, if it has one yet. `None` means the
    /// frame is loading -- the panel draws that state from `Frame::loading`,
    /// not by treating a missing listing as an empty directory.
    pub fn active_listing(&self) -> Option<&Arc<Listing>> {
        self.listings.get(&self.active_frame().dir)
    }

    /// The entries `frame` currently draws, in row order: `frame.rows`
    /// resolved against its directory's listing. Empty when the frame has no
    /// listing yet -- the same "loading, not empty" distinction as
    /// `active_listing`.
    pub fn rows<'a>(&'a self, frame: &Frame) -> Vec<&'a Entry> {
        match self.listings.get(&frame.dir) {
            Some(listing) => frame
                .rows
                .iter()
                .filter_map(|&i| listing.entries.get(i))
                .collect(),
            None => Vec::new(),
        }
    }

    /// The entry the active frame's cursor sits on, if there is one -- `None`
    /// for an empty directory, a still-loading frame, or a query that
    /// matched nothing.
    pub fn cursor_entry(&self) -> Option<&Entry> {
        if let Some(search) = &self.search {
            return search.results.get(search.cursor);
        }
        let frame = self.active_frame();
        let listing = self.listings.get(&frame.dir)?;
        let row = *frame.rows.get(frame.cursor)?;
        listing.entries.get(row)
    }

    /// `(files, dirs, bytes)` for the active level's rule row -- e.g.
    /// `14 files · 3 dirs · 84.2 MB`. Counts the whole directory under the
    /// current hidden-files setting, not narrowed by a `/` filter: the rule
    /// describes the level, not the search. `None` while the frame has no
    /// listing yet.
    pub fn dir_stats(&self) -> Option<(usize, usize, u64)> {
        let listing = self.active_listing()?;
        let mut files = 0usize;
        let mut dirs = 0usize;
        let mut bytes = 0u64;
        for entry in &listing.entries {
            if entry.hidden && !self.show_hidden {
                continue;
            }
            if entry.kind == EntryKind::Dir {
                dirs += 1;
            } else {
                files += 1;
                bytes += entry.len;
            }
        }
        Some((files, dirs, bytes))
    }
}

/// What can change the truth: a command the UI asked for, or a worker
/// reporting that a job finished.
///
/// One enum for both rather than two entry points, because a command and a
/// `Done` are handled the same way from here down: both are folded into
/// `State` under the one write lock and both can produce more `Job`s: a
/// `PasteHere` command queues a copy, and the `Done::Planned` that
/// eventually comes back from planning it is what actually enqueues the
/// files to run.
pub enum Change {
    Command(Command),
    Done(Done),
}

/// What one [`apply`] implied: work for the threads, and notifications for
/// the window. Both are returned rather than sent from inside `apply`, so the
/// write lock is released before anything touches a channel.
#[derive(Debug, Default)]
pub struct Effects {
    pub jobs: Vec<Job>,
    pub events: Vec<Event>,
}

/// Fold one [`Change`] into `state`, and say what it implies.
///
/// `state.version` is bumped exactly when `events` comes back non-empty --
/// the same rule STAR/CORD's `apply` uses (`touch()` there): an event is
/// never emitted for a change that left the drawn view alone, so "something
/// worth telling the UI about happened" and "the version moved" are the same
/// question asked twice, and it is cheaper to ask it once here than to have
/// every arm below decide for itself.
pub fn apply(state: &mut State, change: Change) -> Effects {
    let old_tab = state.tabs.active().id;
    let old_stack = state.tabs.active().active_stack;
    let old_dir = state.active_frame().dir.clone();
    let effects = apply_inner(state, change);
    if state.commander
        && old_tab == state.tabs.active().id
        && old_stack == state.tabs.active().active_stack
        && old_dir != state.active_frame().dir
    {
        for path in state.selection.paths() {
            state.marked_search_identities.remove(path);
        }
        state.selection.forget();
    }
    let origin_name = state.tab_label(old_tab);
    for op in state.queue.iter_mut() {
        if op.origin_tab.is_none() {
            op.origin_tab = Some(old_tab);
            op.origin_name = origin_name.clone();
        }
    }
    if !effects.events.is_empty() {
        state.version += 1;
    }
    effects
}

fn apply_inner(state: &mut State, change: Change) -> Effects {
    match change {
        Change::Command(command) => apply_command(state, command),
        Change::Done(done) => apply_done(state, done),
    }
}

fn apply_command(state: &mut State, command: Command) -> Effects {
    if state
        .preview
        .as_ref()
        .and_then(|(_, p)| p.extension())
        .is_some_and(|info| info.interactive && info.modified)
        && (matches!(
            &command,
            Command::Preview(_)
                | Command::ClosePreview
                | Command::NewTab { .. }
                | Command::SwitchTab(_)
                | Command::CloseTab(_)
                | Command::ReopenTab
                | Command::RestoreTabs(..)
        ) || matches!(&command, Command::UnmountPlace {path,..} if state.preview.as_ref().is_some_and(|(file,_)| file.starts_with(path))))
    {
        return Effects {
            jobs: vec![],
            events: vec![Event::Note(Note::error(
                "editor",
                "Resolve unsaved editor changes before replacing Preview or switching tabs",
            ))],
        };
    }
    if matches!(
        &command,
        Command::CursorTo(_)
            | Command::CursorBy(_)
            | Command::Enter
            | Command::Back
            | Command::JumpTo(_)
            | Command::Forward
            | Command::Push(_)
            | Command::Reload
            | Command::SetFilter(_)
            | Command::ClearFilter
            | Command::StartSearch(_)
            | Command::StartContentSearch(_)
            | Command::CloseSearch
            | Command::SetSort(_)
            | Command::SetHidden(_)
            | Command::ToggleMark
            | Command::ToggleView
            | Command::NewTab { .. }
            | Command::SwitchTab(_)
            | Command::CloseTab(_)
            | Command::ReopenTab
            | Command::RestoreTabs(..)
            | Command::RestoreCommander { .. }
    ) {
        state.navigation_generation = state.navigation_generation.wrapping_add(1);
    }
    match command {
        Command::LoadPlaces(path) => {
            state.places.requested_path = Some(path.clone());
            state.places.loading = true;
            state.places.file_path = None;
            state.places.bookmarks_error = None;
            state.places.locations_error = None;
            state.places.sync_error();
            Effects {
                jobs: vec![Job::LoadPlaces(path)],
                events: vec![Event::Places],
            }
        }
        Command::RefreshPlaces => {
            if state.places.loading {
                return Effects::default();
            }
            state.places.loading = true;
            let job = if state.places.file_path.is_none() {
                state
                    .places
                    .requested_path
                    .clone()
                    .map(Job::LoadPlaces)
                    .unwrap_or(Job::RefreshPlaces)
            } else {
                Job::RefreshPlaces
            };
            Effects {
                jobs: vec![job],
                events: vec![Event::Places],
            }
        }
        Command::UnmountPlace { path, source } => {
            if state.places.loading
                || !state.places.locations.iter().any(|location| {
                    location.path == path && location.unmount_source.as_ref() == Some(&source)
                })
            {
                return Effects::default();
            }
            state.places.loading = true;
            state.places.unmounting = Some(path.clone());
            state.places.locations_error = None;
            state.places.sync_error();
            Effects {
                jobs: vec![Job::UnmountPlace { path, source }],
                events: vec![Event::Places],
            }
        }
        Command::SaveBookmark { name, path } => cmd_save_bookmark(state, name, path),
        Command::RenameBookmark { path, name } => cmd_rename_bookmark(state, path, name),
        Command::RemoveBookmark(path) => cmd_remove_bookmark(state, path),
        Command::Notify(message) => Effects {
            events: vec![Event::Note(Note::warning("startup", message))],
            ..Effects::default()
        },
        Command::RememberTabScroll { tab, rows } => {
            let search = if tab == state.tabs.active().id {
                state.search.as_mut()
            } else {
                state
                    .tabs
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tab)
                    .and_then(|t| t.context.as_mut())
                    .and_then(|c| c.search.as_mut())
            };
            if let Some(search) = search {
                if let Some((_, _, scroll)) = rows
                    .iter()
                    .find(|(stack, frame, _)| *stack == 3 && frame.0 == search.generation)
                {
                    search.view = *scroll;
                }
            }
            if let Some(tab) = state.tabs.tabs.iter_mut().find(|t| t.id == tab) {
                for (stack, frame, scroll) in rows {
                    if let Some(frame) = tab
                        .stacks
                        .get_mut(stack)
                        .and_then(|s| s.frames_mut().find(|f| f.id == frame))
                    {
                        frame.view = scroll;
                    }
                }
            }
            Effects::default()
        }
        Command::NewTab { duplicate } => workspace::new_tab(state, duplicate),
        Command::SwitchTab(id) => workspace::switch_tab(state, id),
        Command::CloseTab(id) => workspace::close_tab(state, id),
        Command::RenameTab(id, name) => {
            if let Some(tab) = state.tabs.tabs.iter_mut().find(|t| t.id == id) {
                tab.name = bookmark_name(name);
            }
            Effects {
                jobs: vec![],
                events: vec![Event::Stack],
            }
        }
        Command::MoveTab(id, delta) => {
            if let Some(index) = state.tabs.tabs.iter().position(|t| t.id == id) {
                let active = state.tabs.active().id;
                let target = (index as i64 + i64::from(delta))
                    .clamp(0, state.tabs.tabs.len() as i64 - 1)
                    as usize;
                state.tabs.tabs.swap(index, target);
                let active_index = state.tabs.tabs.iter().position(|t| t.id == active).unwrap();
                state.tabs.activate(active_index);
            }
            Effects {
                jobs: vec![],
                events: vec![Event::Stack],
            }
        }
        Command::ReopenTab => workspace::reopen_tab(state),
        Command::RestoreTabs(tabs, active) => workspace::restore_tabs(state, tabs, active),
        Command::ToggleView => {
            if state.tabs.active().stacks.len() == 1 {
                state.pane_sorts = [state.sort; 2];
                let dir = state.active_frame().dir.clone();
                let loading = !state.listings.contains_key(&dir);
                state
                    .tabs
                    .active_mut()
                    .stacks
                    .extend([Stack::new(dir.clone()), Stack::new(dir)]);
                rebuild_all_frames(state);
                for stack in state.tabs.active_mut().stacks.iter_mut().skip(1) {
                    stack.active_mut().loading = loading;
                }
            }
            state.commander = !state.commander;
            switch_stack(
                state,
                if state.commander {
                    state.commander_pane + 1
                } else {
                    0
                },
                false,
            )
        }
        Command::FocusPane(pane) => {
            if !state.commander || pane > 1 {
                return Effects::default();
            }
            state.commander_pane = pane;
            switch_stack(state, pane + 1, true)
        }
        Command::RestoreCommander {
            dirs,
            active,
            enabled,
        } => {
            state.pane_sorts = [state.sort; 2];
            state.tabs.active_mut().stacks.truncate(1);
            state
                .tabs
                .active_mut()
                .stacks
                .extend(dirs.iter().cloned().map(Stack::new));
            state.commander_pane = active.min(1);
            state.commander = enabled;
            let mut effects = switch_stack(
                state,
                if enabled { state.commander_pane + 1 } else { 0 },
                false,
            );
            effects.jobs.extend(dirs.into_iter().map(Job::List));
            effects
        }
        Command::Enter => cmd_enter(state),
        Command::Back => cmd_back(state),
        Command::JumpTo(index) => cmd_jump_to(state, index),
        Command::Forward => cmd_forward(state),
        Command::Push(dir) => push_dir(state, dir),
        Command::Reload => cmd_reload(state),
        Command::Create { dir, kind, name } => {
            if let Err(error) = create::validate_name(&name) {
                return Effects {
                    jobs: vec![],
                    events: vec![Event::Note(Note::error("create", error))],
                };
            }
            if let Some(effects) =
                copy_lock_note(copy_lock_conflict_paths(state, &[], &[dir.join(&name)]))
            {
                return effects;
            }
            let tab = state.tabs.active();
            let origin = CreateOrigin {
                tab: tab.id,
                stack: tab.active_stack,
                frame: tab.active_stack().active().id,
            };
            Effects {
                jobs: vec![Job::Create {
                    dir,
                    name,
                    kind,
                    origin,
                }],
                events: vec![],
            }
        }
        Command::CursorTo(row) => set_cursor(state, row),
        Command::CursorBy(delta) => cmd_cursor_by(state, delta),
        Command::SetFilter(query) => cmd_set_filter(state, query),
        Command::ClearFilter => cmd_clear_filter(state),
        Command::StartSearch(query) => cmd_start_search(state, query, search::Mode::Names),
        Command::StartContentSearch(query) => {
            cmd_start_search(state, query, search::Mode::Contents)
        }
        Command::CancelSearch => {
            if let Some(search) = &state.search {
                search
                    .progress
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Effects {
                jobs: vec![],
                events: vec![Event::Stack],
            }
        }
        Command::CloseSearch => close_search(state),
        Command::SetSort(order) => cmd_set_sort(state, order),
        Command::RestoreSorts { fold, panes } => {
            if let Some(order) = fold {
                state.sort = order;
            }
            state.pane_sorts = panes.map(|order| order.unwrap_or(state.sort));
            rebuild_all_frames(state);
            Effects {
                jobs: Vec::new(),
                events: vec![Event::Stack],
            }
        }
        Command::SetHidden(hidden) => cmd_set_hidden(state, hidden),
        Command::ToggleMark => cmd_toggle_mark(state),
        Command::ToggleMarkPath(path) => {
            let entry = state
                .search
                .as_ref()
                .and_then(|search| search.results.iter().find(|e| e.path == path))
                .cloned()
                .or_else(|| {
                    path.parent()
                        .and_then(|dir| state.listing_of(dir))
                        .and_then(|l| l.entries.iter().find(|e| e.path == path))
                        .cloned()
                });
            let Some(entry) = entry else {
                return Effects::default();
            };
            state.selection.toggle(&entry);
            sync_search_mark_identity(state, &entry);
            let jobs = if entry.kind == EntryKind::Dir && state.selection.is_marked(&path) {
                vec![Job::Summarize(path)]
            } else {
                vec![]
            };
            Effects {
                jobs,
                events: vec![Event::Selection],
            }
        }
        Command::MarkAll => cmd_mark_all(state),
        Command::InvertMarks => cmd_invert_marks(state),
        Command::ClearMarks => cmd_clear_marks(state),
        Command::SyncArchiveEdits => Effects {
            jobs: vec![Job::SyncArchiveEdits],
            events: vec![],
        },
        Command::UnlockArchive { location, options } => Effects {
            jobs: vec![Job::UnlockArchive { location, options }],
            events: vec![],
        },
        Command::QueueArchive {
            format,
            sources,
            destination,
            options,
        } => {
            let id = state.queue.enqueue(
                OpKind::Compress(format),
                sources,
                Some(destination),
                state.conflicts,
            );
            state.queue.get_mut(id).unwrap().archive_options = options;
            queued_effects(state, id)
        }
        Command::QueueOperation {
            kind,
            sources,
            dest,
        } => {
            if sources.is_empty() {
                return Effects::default();
            }
            if let Some(effects) =
                copy_lock_note(copy_lock_conflict(state, kind, &sources, dest.as_deref()))
            {
                return effects;
            }
            let id = state
                .queue
                .enqueue(kind, sources.clone(), dest, state.conflicts);
            record_search_expectations(state, id, &sources);
            queued_effects(state, id)
        }
        Command::QueueDrop {
            kind,
            sources,
            dest,
        } => {
            if sources.is_empty() || !matches!(kind, OpKind::Copy | OpKind::Move) {
                return Effects::default();
            }
            if let Some(effects) =
                copy_lock_note(copy_lock_conflict(state, kind, &sources, Some(&dest)))
            {
                return effects;
            }
            let id = state
                .queue
                .enqueue_drop(kind, sources.clone(), dest, state.conflicts);
            record_search_expectations(state, id, &sources);
            queued_effects(state, id)
        }
        Command::BeginExport(sources) => {
            if sources.is_empty() {
                return Effects::default();
            }
            if let Some(effects) = copy_lock_note(copy_lock_conflict_paths(state, &sources, &[])) {
                return effects;
            }
            let id = state.queue.begin_export(sources, state.conflicts);
            Effects {
                jobs: vec![],
                events: vec![Event::Queue(id)],
            }
        }
        Command::FinishExport { op, success } => {
            if let Some(entry) = state.queue.get_mut(op) {
                entry.status = if success {
                    OpStatus::Done
                } else {
                    OpStatus::Failed
                };
            }
            let mut effects = run_next(state);
            effects.events.insert(0, Event::Queue(op));
            effects
        }
        Command::BeginImport { sources, dest } => {
            if sources.is_empty() {
                return Effects::default();
            }
            let targets = copy_targets(&sources, &dest);
            if let Some(effects) = copy_lock_note(copy_lock_conflict_paths(state, &[], &targets)) {
                return effects;
            }
            let id = state
                .queue
                .begin_import(sources.clone(), dest.clone(), state.conflicts);
            Effects {
                jobs: vec![Job::CheckImport {
                    op: id,
                    sources,
                    dest,
                }],
                events: vec![Event::Queue(id)],
            }
        }
        Command::CompleteImport { op, sources } => {
            let Some(entry) = state.queue.get_mut(op) else {
                return Effects::default();
            };
            if entry.import_sources.is_some() && entry.progress.is_cancelled() {
                entry.status = OpStatus::Cancelled;
                return Effects {
                    jobs: vec![],
                    events: vec![Event::Queue(op)],
                };
            }
            if entry.import_sources.is_none() || entry.status != OpStatus::Running {
                return Effects::default();
            }
            if entry.progress.is_cancelled() || sources.is_empty() {
                entry.status = if entry.progress.is_cancelled() {
                    OpStatus::Cancelled
                } else {
                    OpStatus::Failed
                };
                let mut effects = run_next(state);
                effects.events.insert(0, Event::Queue(op));
                return effects;
            }
            entry.progress.finish_receiving();
            entry.sources = sources.clone();
            entry.kind = if entry
                .dest
                .as_deref()
                .is_some_and(super::location::is_archive)
            {
                OpKind::Copy
            } else {
                OpKind::Move
            };
            entry.status = OpStatus::Planning;
            Effects {
                jobs: vec![Job::Plan {
                    op,
                    kind: entry.kind,
                    sources,
                    dest: entry.dest.clone(),
                    expected: Vec::new(),
                    archive_options: Default::default(),
                }],
                events: vec![Event::Queue(op)],
            }
        }
        Command::FinishImport { op, success } => {
            if let Some(entry) = state.queue.get_mut(op) {
                entry.status = if entry.progress.is_cancelled() {
                    OpStatus::Cancelled
                } else if success {
                    OpStatus::Done
                } else {
                    OpStatus::Failed
                };
            }
            let mut effects = run_next(state);
            effects.events.insert(0, Event::Queue(op));
            effects
        }
        Command::ReportDropFailure { op, reason } => {
            if let Some(entry) = state
                .queue
                .get_mut(op)
                .filter(|entry| entry.import_sources.is_some())
            {
                entry.failure = Some(reason.clone());
                if let Some(source) = entry
                    .import_sources
                    .as_ref()
                    .and_then(|paths| paths.first())
                {
                    entry.failed.push((source.clone(), reason));
                }
                if !matches!(entry.status, OpStatus::Running | OpStatus::Planning) {
                    entry.status = OpStatus::Failed;
                }
            }
            Effects {
                jobs: vec![],
                events: vec![Event::Queue(op)],
            }
        }
        Command::PreviewAck { session, sequence } => {
            if let Some((_, preview)) = &mut state.preview {
                if let Some(info) = Arc::make_mut(preview)
                    .extension_mut()
                    .filter(|info| info.session == session)
                {
                    info.actions.retain(|action| action.sequence > sequence);
                }
            }
            Effects::default()
        }
        Command::PreviewInput {
            path,
            generation,
            input,
        } => {
            if generation != state.preview_generation
                || !state.preview.as_ref().is_some_and(|(p, _)| *p == path)
            {
                return Effects::default();
            }
            Effects {
                jobs: vec![Job::PreviewInput {
                    tab: state.tabs.active().id,
                    path,
                    generation,
                    input,
                }],
                events: vec![],
            }
        }
        Command::PreviewPage {
            path,
            generation,
            page,
        } => {
            if generation != state.preview_generation
                || !state.preview.as_ref().is_some_and(|(p, _)| *p == path)
            {
                return Effects::default();
            }
            if let Some((_, preview)) = &mut state.preview {
                if let Preview::Document(d) = Arc::make_mut(preview) {
                    d.notice = Some(format!("Loading pages {page}…"));
                }
            }
            Effects {
                jobs: vec![Job::PreviewPage {
                    tab: state.tabs.active().id,
                    path,
                    generation,
                    page,
                }],
                events: vec![Event::Preview],
            }
        }
        Command::Yank => cmd_yank(state),
        Command::PasteHere => cmd_paste_here(state),
        Command::QueueMoveHere => cmd_queue_move(state),
        Command::QueueDelete => cmd_queue_delete(state),
        Command::QueueDeleteSources(sources) => queue_delete_sources(state, sources),
        Command::QueuePermanentDeleteSources(sources) => {
            queue_delete_sources_with_how(state, sources, DeleteHow::Permanent)
        }
        Command::QueueElevatedDelete(id) => cmd_queue_elevated_delete(state, id),
        Command::EmptyDriveTrash(path) => cmd_empty_drive_trash(state, &path),
        Command::QueueRename { from, to } => cmd_queue_rename(state, from, to),
        Command::RemoveOp(id) => cmd_remove_op(state, id),
        Command::ClearQueue => cmd_clear_queue(state),
        Command::Run => {
            state.queue.resume();
            run_next(state)
        }
        Command::SetPolicy(id, policy) => cmd_set_policy(state, id, policy),
        Command::SetConflictNames(id, names) => {
            let Some(op) = state.queue.get_mut(id) else {
                return Effects::default();
            };
            if op.status != OpStatus::NeedsPolicy {
                return Effects::default();
            }
            op.rename_targets = names;
            cmd_set_policy(state, id, ConflictPolicy::RenameNew)
        }
        Command::Cancel(id) => cmd_cancel(state, id),
        Command::StopActive(id) => {
            if state
                .queue
                .get_mut(id)
                .is_some_and(|op| matches!(op.status, OpStatus::Planning | OpStatus::Running))
            {
                state.queue.pause();
                cmd_cancel(state, id)
            } else {
                Effects::default()
            }
        }
        Command::Preview(path) => cmd_preview(state, path),
        Command::ClosePreview => {
            state.preview_generation += 1;
            Effects {
                jobs: vec![Job::ClosePreview],
                events: vec![],
            }
        }
        Command::OpenExternal(path) => Effects {
            jobs: vec![Job::Open(path)],
            events: Vec::new(),
        },
        Command::Shutdown => {
            for tab in &state.tabs.tabs {
                if let Some(search) = tab.context.as_ref().and_then(|c| c.search.as_ref()) {
                    search
                        .progress
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            if let Some(search) = &state.search {
                search
                    .progress
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            state.preview_generation += 1;
            for op in state.queue.iter() {
                op.progress.cancel();
            }
            Effects::default()
        }
    }
}

fn switch_stack(state: &mut State, index: usize, preserve_preview: bool) -> Effects {
    let old = state.tabs.active().active_stack;
    std::mem::swap(&mut state.selection, &mut state.parked_selections[old]);
    state.tabs.active_mut().active_stack = index;
    std::mem::swap(&mut state.selection, &mut state.parked_selections[index]);
    if !preserve_preview
        && !state
            .preview
            .as_ref()
            .and_then(|(_, p)| p.extension())
            .is_some_and(|info| info.interactive)
    {
        state.preview_generation += 1;
        state.preview = None;
    }
    let jobs = ensure_listed(state);
    Effects {
        jobs,
        events: vec![Event::Stack],
    }
}

fn apply_done(state: &mut State, done: Done) -> Effects {
    match done {
        Done::PlacesLoaded {
            path,
            bookmarks,
            locations,
        } => {
            state.places.loading = false;
            match bookmarks {
                Ok(bookmarks) => {
                    state.places.bookmarks = bookmarks;
                    state.places.file_path = Some(path);
                    state.places.bookmarks_error = None;
                }
                Err(error) => state.places.bookmarks_error = Some(error),
            }
            match locations {
                Ok(locations) => {
                    state.places.locations = locations;
                    state.places.locations_error = None;
                }
                Err(error) => state.places.locations_error = Some(error),
            }
            state.places.sync_error();
            let mut events = vec![Event::Places];
            if let Some(error) = &state.places.error {
                events.push(Event::Note(Note::error("places", error.clone())));
            }
            Effects {
                jobs: vec![],
                events,
            }
        }
        Done::PlacesRefreshed(result) => {
            state.places.loading = false;
            let mut events = vec![Event::Places];
            match result {
                Ok(locations) => {
                    state.places.locations = locations;
                    state.places.locations_error = None;
                }
                Err(error) => {
                    state.places.locations_error = Some(error.clone());
                    events.push(Event::Note(Note::error("places", error)));
                }
            }
            state.places.sync_error();
            Effects {
                jobs: vec![],
                events,
            }
        }
        Done::PlaceUnmounted { path, result } => {
            state.places.loading = false;
            state.places.unmounting = None;
            match result {
                Ok(locations) => {
                    state.places.locations = locations;
                    state.places.locations_error = None;
                    state.places.sync_error();
                    let jobs = leave_unmounted_place(state, &path);
                    Effects {
                        jobs,
                        events: vec![
                            Event::Places,
                            Event::Stack,
                            Event::Note(Note::info(format!("Unmounted {}", path.display()))),
                        ],
                    }
                }
                Err(error) => {
                    state.places.locations_error = Some(error.clone());
                    state.places.sync_error();
                    Effects {
                        jobs: vec![],
                        events: vec![Event::Places, Event::Note(Note::error("places", error))],
                    }
                }
            }
        }
        Done::BookmarksSaved { revision, result } => {
            if revision != state.places.revision {
                return Effects::default();
            }
            match result {
                Ok(()) => {
                    state.places.bookmarks_error = None;
                    state.places.sync_error();
                    Effects {
                        jobs: vec![],
                        events: vec![Event::Places],
                    }
                }
                Err(error) => {
                    state.places.bookmarks_error = Some(error.clone());
                    state.places.sync_error();
                    Effects {
                        jobs: vec![],
                        events: vec![Event::Places, Event::Note(Note::error("bookmarks", error))],
                    }
                }
            }
        }
        Done::StartupListed {
            requested,
            listing,
            notice,
        } => {
            let actual = listing.dir.clone();
            if requested != actual && !state.durable_tabs {
                for tab in &mut state.tabs.tabs {
                    for stack in &mut tab.stacks {
                        // A user may have navigated away while a drive woke
                        // up. Only replace an untouched startup location.
                        if stack.len() == 1 && stack.active().dir == requested {
                            *stack = Stack::new(actual.clone());
                        }
                    }
                }
            }
            let mut effects = done_listed(state, listing);
            if let Some(notice) = notice {
                effects
                    .events
                    .push(Event::Note(Note::warning("startup", notice)));
            }
            effects
        }
        Done::Listed(listing) => done_listed(state, listing),
        Done::Searched {
            tab,
            generation,
            found,
        } => {
            if tab != state.tabs.active().id {
                let Some(context) = state
                    .tabs
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tab)
                    .and_then(|t| t.context.as_mut())
                else {
                    return Effects::default();
                };
                let sort = sort_for_stack(
                    if context.commander {
                        context.commander_pane + 1
                    } else {
                        0
                    },
                    context.sort,
                    context.pane_sorts,
                );
                let Some(search) = context
                    .search
                    .as_mut()
                    .filter(|s| s.generation == generation)
                else {
                    return Effects::default();
                };
                let indices = sort::order(&found.entries, sort, true);
                search.results = indices
                    .into_iter()
                    .map(|i| found.entries[i].clone())
                    .collect();
                search.identities = found.identities.clone();
                search.excerpts = found.excerpts.clone();
                search.skipped_binary = found.skipped_binary;
                search.skipped_large = found.skipped_large;
                search.cursor = search
                    .restore_cursor_path
                    .take()
                    .and_then(|path| search.results.iter().position(|entry| entry.path == path))
                    .unwrap_or(search.cursor)
                    .min(search.results.len().saturating_sub(1));
                search.status = found.status;
                search.errors = found.errors.clone();
                return Effects {
                    jobs: vec![],
                    events: vec![Event::Stack],
                };
            }
            let active_sort = state.active_sort();
            let Some(search) = state.search.as_mut().filter(|s| s.generation == generation) else {
                return Effects::default();
            };
            let indices = sort::order(&found.entries, active_sort, true);
            search.results = indices
                .into_iter()
                .map(|i| found.entries[i].clone())
                .collect();
            search.identities = found.identities.clone();
            search.excerpts = found.excerpts.clone();
            search.skipped_binary = found.skipped_binary;
            search.skipped_large = found.skipped_large;
            search.cursor = search
                .restore_cursor_path
                .take()
                .and_then(|path| search.results.iter().position(|entry| entry.path == path))
                .unwrap_or(search.cursor)
                .min(search.results.len().saturating_sub(1));
            search.status = found.status;
            search.errors = found.errors.clone();
            let mut events = vec![Event::Stack];
            if let Some(error) = search.errors.first() {
                events.push(Event::Note(Note::warning(
                    "search",
                    format!("Search skipped unreadable paths: {error}"),
                )));
            }
            Effects {
                jobs: vec![],
                events,
            }
        }
        Done::Created {
            path,
            kind,
            origin,
            result,
        } => done_created(state, path, kind, origin, result),
        Done::Summarized { dir, summary } => done_summarized(state, dir, summary),
        Done::ArchiveOpened { path, location } => {
            if state.cursor_entry().is_some_and(|e| e.path == path) {
                push_dir(state, location)
            } else {
                Effects::default()
            }
        }
        Done::ArchiveEditsSynced { directories, error } => Effects {
            jobs: directories.into_iter().map(Job::List).collect(),
            events: error
                .into_iter()
                .map(|e| Event::Note(Note::error("archive editor", e)))
                .collect(),
        },
        Done::ArchivePreviewed {
            tab,
            path,
            local,
            editable,
            generation,
            preview,
        } => {
            if tab == state.tabs.active().id && generation == state.preview_generation {
                if editable {
                    state.archive_editable.insert(path.clone());
                } else {
                    state.archive_editable.remove(&path);
                }
                state.archive_materialized.insert(path.clone(), local);
                done_previewed(state, path, generation, preview)
            } else {
                Effects::default()
            }
        }
        Done::Previewed {
            tab,
            path,
            generation,
            preview,
        } => {
            if tab == state.tabs.active().id {
                done_previewed(state, path, generation, preview)
            } else {
                Effects::default()
            }
        }
        Done::Planned { op, result } => done_planned(state, op, result),
        Done::ImportChecked { op, result } => done_import_checked(state, op, result),
        Done::Finished { op, outcome } => done_finished(state, op, outcome),
        Done::Changed(dirs) => done_changed(state, dirs),
    }
}

fn bookmark_name(name: String) -> Option<String> {
    let trimmed = name.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn bookmark_error(text: impl Into<String>) -> Effects {
    Effects {
        jobs: vec![],
        events: vec![Event::Note(Note::error("bookmarks", text))],
    }
}

fn persist_bookmarks(state: &mut State) -> Effects {
    let Some(path) = state.places.file_path.clone() else {
        return bookmark_error("bookmarks have not loaded; cannot save");
    };
    state.places.revision += 1;
    Effects {
        jobs: vec![Job::SaveBookmarks {
            path,
            bookmarks: state.places.bookmarks.clone(),
            revision: state.places.revision,
        }],
        events: vec![Event::Places],
    }
}

fn cmd_save_bookmark(state: &mut State, name: String, path: PathBuf) -> Effects {
    let Some(name) = bookmark_name(name) else {
        return bookmark_error("bookmark name cannot be empty");
    };
    if !path.is_absolute() {
        return bookmark_error("bookmark path must be absolute");
    }
    if state.places.file_path.is_none() {
        return bookmark_error("bookmarks have not loaded; cannot save");
    }
    if let Some(mark) = state
        .places
        .bookmarks
        .iter_mut()
        .find(|mark| mark.path == path)
    {
        mark.name = name;
    } else {
        state.places.bookmarks.push(Bookmark { name, path });
    }
    persist_bookmarks(state)
}

fn cmd_rename_bookmark(state: &mut State, path: PathBuf, name: String) -> Effects {
    let Some(name) = bookmark_name(name) else {
        return bookmark_error("bookmark name cannot be empty");
    };
    if state.places.file_path.is_none() {
        return bookmark_error("bookmarks have not loaded; cannot save");
    }
    let Some(mark) = state
        .places
        .bookmarks
        .iter_mut()
        .find(|mark| mark.path == path)
    else {
        return bookmark_error("bookmark no longer exists");
    };
    mark.name = name;
    persist_bookmarks(state)
}

fn cmd_remove_bookmark(state: &mut State, path: PathBuf) -> Effects {
    if state.places.file_path.is_none() {
        return bookmark_error("bookmarks have not loaded; cannot save");
    }
    let previous = state.places.bookmarks.len();
    state.places.bookmarks.retain(|mark| mark.path != path);
    if state.places.bookmarks.len() == previous {
        return bookmark_error("bookmark no longer exists");
    }
    persist_bookmarks(state)
}

// ---------------------------------------------------------------------
// Rebuilding a frame's rows.
// ---------------------------------------------------------------------

/// Whether landing on a fresh row should put the cursor back at the top, or
/// try to find the entry it was already on. A push or a fresh filter has no
/// previous position worth keeping; a reload, a sort change or a hidden-files
/// toggle does.
#[derive(Clone, Copy)]
enum Landing {
    Reset,
    Refind,
}

/// Recompute `frame.rows` from `listing` under the current sort and filter,
/// then place the cursor per `landing`. The one place `sort::order` and
/// `filter::rank` are called from -- everything above this line decides
/// *when* rows need rebuilding, and this is the only function that knows
/// *how*.
fn rebuild_rows(
    frame: &mut Frame,
    listing: &Listing,
    sort: SortOrder,
    hidden: bool,
    landing: Landing,
) {
    let visible = sort::order(&listing.entries, sort, hidden);
    frame.rows = filter::rank(&frame.filter, &listing.entries, &visible);
    match landing {
        Landing::Reset => reset_cursor(frame, listing),
        Landing::Refind => refind_cursor(frame, listing),
    }
}

fn reset_cursor(frame: &mut Frame, listing: &Listing) {
    frame.cursor = 0;
    sync_cursor_name(frame, listing);
}

/// Look for `frame.cursor_name` among the freshly built `frame.rows`; land on
/// it if it is still there (a reload that inserted a row above it, a sort
/// change, `.` toggled) and fall back to a clamped index if it is gone (the
/// file itself vanished).
fn refind_cursor(frame: &mut Frame, listing: &Listing) {
    if let Some(name) = frame.cursor_name.clone() {
        if let Some(pos) = frame.rows.iter().position(|&i| {
            listing.entries.get(i).and_then(|e| e.path.file_name()) == Some(name.as_os_str())
        }) {
            frame.cursor = pos;
            return;
        }
    }
    frame.cursor = frame.cursor.min(frame.rows.len().saturating_sub(1));
    if frame.rows.is_empty() {
        frame.cursor = 0;
    }
    sync_cursor_name(frame, listing);
}

/// Set `frame.cursor_name` from whatever row the cursor now sits on, so the
/// next reload or sort change can find it again.
fn sync_cursor_name(frame: &mut Frame, listing: &Listing) {
    frame.cursor_name = frame
        .rows
        .get(frame.cursor)
        .and_then(|&i| listing.entries.get(i))
        .and_then(|e| e.path.file_name())
        .map(|n| n.to_os_string());
}

/// After landing on a frame (a fresh push), fill its rows from a cached
/// listing or ask the io thread to read one. "Enter on a cached dir asks for
/// nothing" is this function finding the listing already in `state.listings`.
fn populate_active_frame(state: &mut State) -> Vec<Job> {
    let dir = state.active_frame().dir.clone();
    let sort = state.active_sort();
    let hidden = state.show_hidden;
    match state.listings.get(&dir).cloned() {
        Some(listing) => {
            let frame = state.active_frame_mut();
            frame.loading = false;
            rebuild_rows(frame, &listing, sort, hidden, Landing::Reset);
            Vec::new()
        }
        None => {
            let frame = state.active_frame_mut();
            frame.loading = true;
            vec![Job::List(dir)]
        }
    }
}

/// Ask for a listing if the active frame landed on has none cached -- backing
/// into, or jumping to, a frame whose listing was evicted while it was not
/// the active one. Its `rows` are left exactly as they were: nothing about
/// this frame's own content changed, only which frame is active.
fn ensure_listed(state: &mut State) -> Vec<Job> {
    let dir = state.active_frame().dir.clone();
    if state.listings.contains_key(&dir) {
        state.active_frame_mut().loading = false;
        Vec::new()
    } else {
        state.active_frame_mut().loading = true;
        vec![Job::List(dir)]
    }
}

/// Rebuild the active frame's rows from its own cached listing, if it has
/// one -- what `SetFilter` and `ClearFilter` do, since only the level being
/// looked at can have its filter changed.
fn rebuild_active(state: &mut State, landing: Landing) {
    let dir = state.active_frame().dir.clone();
    let sort = state.active_sort();
    let hidden = state.show_hidden;
    if let Some(listing) = state.listings.get(&dir).cloned() {
        let frame = state.active_frame_mut();
        rebuild_rows(frame, &listing, sort, hidden, landing);
    }
}

/// Rebuild every frame's rows, in every stack of every tab, against whatever
/// listing each one's own directory has cached. `SetHidden` and session
/// restoration use this; ordinary sort changes rebuild one stack only.
///
/// The listings map is cloned up front (an `Arc` clone per entry, not a deep
/// copy of any `Listing`) rather than looked up per frame while `state.tabs`
/// is borrowed mutably: `HashMap<PathBuf, Arc<Listing>>` and `Tabs` are
/// different fields of `State`, but threading a live borrow of one through
/// three nested loops mutating the other is more ceremony than a cheap clone
/// is worth.
fn rebuild_all_frames(state: &mut State) {
    let fold_sort = state.sort;
    let pane_sorts = state.pane_sorts;
    let hidden = state.show_hidden;
    let listings = state.listings.clone();
    for tab in &mut state.tabs.tabs {
        let (fold_sort, pane_sorts, hidden) = tab
            .context
            .as_ref()
            .map(|c| (c.sort, c.pane_sorts, c.show_hidden))
            .unwrap_or((fold_sort, pane_sorts, hidden));
        for (index, stack) in tab.stacks.iter_mut().enumerate() {
            let sort = sort_for_stack(index, fold_sort, pane_sorts);
            for frame in stack.frames_mut() {
                if let Some(listing) = listings.get(&frame.dir) {
                    rebuild_rows(frame, listing, sort, hidden, Landing::Refind);
                }
            }
        }
    }
}

fn sort_for_stack(index: usize, fold: SortOrder, panes: [SortOrder; 2]) -> SortOrder {
    match index {
        1 => panes[0],
        2 => panes[1],
        _ => fold,
    }
}

fn rebuild_current_stack(state: &mut State) {
    let sort = state.active_sort();
    let hidden = state.show_hidden;
    let listings = state.listings.clone();
    for frame in state.tabs.active_mut().active_stack_mut().frames_mut() {
        if let Some(listing) = listings.get(&frame.dir) {
            rebuild_rows(frame, listing, sort, hidden, Landing::Refind);
        }
    }
}

/// Every directory open in some frame, in any stack of any tab, no
/// duplicates -- what a listing eviction and a post-operation re-list both
/// check against.
fn all_frame_dirs(state: &State) -> BTreeSet<PathBuf> {
    state
        .tabs
        .tabs
        .iter()
        .flat_map(|tab| tab.stacks.iter())
        .flat_map(|stack| stack.frames().iter())
        .map(|frame| frame.dir.clone())
        .collect()
}

/// Drop a cached listing that no frame is looking at any more -- a level
/// that was popped, or a forward trail a fresh push just truncated.
fn evict_unreferenced_listings(state: &mut State) {
    let referenced = all_frame_dirs(state);
    state
        .listings
        .retain(|dir, listing| referenced.contains(dir) || listing.archive_changes > 0);
}

/// Once a volume is gone, no pane may keep drawing its cached directory or
/// return to it through an old Fold crumb. Move affected panes to the mount's
/// parent and list any destination that is not already cached.
fn leave_unmounted_place(state: &mut State, mount: &Path) -> Vec<Job> {
    let parent = mount.parent().unwrap_or(Path::new("/")).to_path_buf();
    let mut destinations = BTreeSet::new();
    for tab in &mut state.tabs.tabs {
        for stack in &mut tab.stacks {
            if stack
                .frames()
                .iter()
                .any(|frame| frame.dir.starts_with(mount))
            {
                let active = &stack.active().dir;
                let destination = if active.starts_with(mount) {
                    parent.clone()
                } else {
                    active.clone()
                };
                *stack = Stack::new(destination.clone());
                destinations.insert(destination);
            }
        }
    }
    state.listings.retain(|dir, _| !dir.starts_with(mount));
    state.selection.forget_under(mount);
    for selection in &mut state.parked_selections {
        selection.forget_under(mount);
    }
    state
        .marked_search_identities
        .retain(|path, _| !path.starts_with(mount));
    if state
        .preview
        .as_ref()
        .is_some_and(|(path, _)| path.starts_with(mount))
    {
        state.preview_generation += 1;
        state.preview = None;
    }
    let fold_sort = state.sort;
    let pane_sorts = state.pane_sorts;
    let hidden = state.show_hidden;
    let mut jobs = Vec::new();
    for destination in destinations {
        if let Some(listing) = state.listings.get(&destination).cloned() {
            for tab in &mut state.tabs.tabs {
                for (index, stack) in tab.stacks.iter_mut().enumerate() {
                    let sort = sort_for_stack(index, fold_sort, pane_sorts);
                    if stack.active().dir == destination {
                        let frame = stack.active_mut();
                        frame.loading = false;
                        rebuild_rows(frame, &listing, sort, hidden, Landing::Reset);
                    }
                }
            }
        } else {
            jobs.push(Job::List(destination));
        }
    }
    jobs
}

/// Move the active frame's cursor down one row, clamped to the last one --
/// what `space` does after marking, so the next entry lands under the cursor
/// without a separate `j`.
fn bump_cursor_down(state: &mut State) {
    if let Some(search) = state.search.as_mut() {
        search.cursor = (search.cursor + 1).min(search.results.len().saturating_sub(1));
        return;
    }
    let dir = state.active_frame().dir.clone();
    let listing = state.listings.get(&dir).cloned();
    let frame = state.active_frame_mut();
    if frame.rows.is_empty() {
        return;
    }
    frame.cursor = (frame.cursor + 1).min(frame.rows.len() - 1);
    if let Some(listing) = &listing {
        sync_cursor_name(frame, listing);
    }
}

/// The active frame's currently visible entries, in row order -- what
/// `MarkAll` and `InvertMarks` work over. A `Vec<Entry>` rather than
/// `Vec<&Entry>`: `Selection::mark_all`/`invert` need to outlive the borrow
/// of `state.listings` this reads from.
fn visible_entries(state: &State) -> Vec<Entry> {
    if let Some(search) = &state.search {
        return search.results.clone();
    }
    let frame = state.active_frame();
    match state.listings.get(&frame.dir) {
        Some(listing) => frame
            .rows
            .iter()
            .filter_map(|&i| listing.entries.get(i).cloned())
            .collect(),
        None => Vec::new(),
    }
}

/// `Job::Summarize` for every directory among `entries` that is now marked --
/// what fills in a freshly marked directory's size.
fn summarize_marked_dirs(state: &State, entries: &[Entry]) -> Vec<Job> {
    entries
        .iter()
        .filter(|e| e.kind == EntryKind::Dir && state.selection.is_marked(&e.path))
        .map(|e| Job::Summarize(e.path.clone()))
        .collect()
}

fn is_dir_like(entry: &Entry) -> bool {
    matches!(entry.kind, EntryKind::Dir)
        || matches!(entry.kind, EntryKind::Symlink if entry.link_kind == Some(EntryKind::Dir))
}

// ---------------------------------------------------------------------
// Commands.
// ---------------------------------------------------------------------

fn cmd_enter(state: &mut State) -> Effects {
    let Some(entry) = state.cursor_entry() else {
        return Effects::default();
    };
    if !is_dir_like(entry) && super::archive::Format::from_path(&entry.path).is_some() {
        let path = entry.path.clone();
        match super::location::Location::from_key(&path).and_then(|l| l.enter_archive()) {
            Ok(location) => return push_dir(state, location.key()),
            Err(error) => {
                return Effects {
                    jobs: vec![],
                    events: vec![Event::Note(Note::error("archive", error.to_string()))],
                }
            }
        }
    }
    if is_dir_like(entry) {
        push_dir(state, entry.path.clone())
    } else {
        Effects {
            jobs: vec![Job::Open(entry.path.clone())],
            events: Vec::new(),
        }
    }
}

/// Push `dir` onto the active stack and fill the new frame's rows, from
/// cache or a fresh `Job::List`. Shared by `Command::Enter` on a directory
/// and `Command::Push` (`gh`, `gr`, a crumb-adjacent path, a typed path).
fn push_dir(state: &mut State, dir: PathBuf) -> Effects {
    if let Some(search) = state.search.take() {
        search
            .progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    state.tabs.active_mut().active_stack_mut().push(dir);
    let jobs = populate_active_frame(state);
    Effects {
        jobs,
        events: vec![Event::Stack],
    }
}

fn cmd_back(state: &mut State) -> Effects {
    if state.search.is_some() {
        return close_search(state);
    }
    let Some(movement) = state.tabs.active_mut().active_stack_mut().go_parent() else {
        return Effects::default();
    };
    if movement == ParentMove::New {
        // Another pane may already have listed this parent. A fresh frame
        // needs those cached rows, with the departed child highlighted.
        rebuild_active(state, Landing::Refind);
    }
    let jobs = ensure_listed(state);
    evict_unreferenced_listings(state);
    Effects {
        jobs,
        events: vec![Event::Stack],
    }
}

fn cmd_jump_to(state: &mut State, index: usize) -> Effects {
    if !state.tabs.active_mut().active_stack_mut().jump_to(index) {
        return Effects::default();
    }
    let jobs = ensure_listed(state);
    Effects {
        jobs,
        events: vec![Event::Stack],
    }
}

/// `Command::Forward` (`alt+down`): step into a child a `JumpTo` left behind
/// without popping it, the mirror of `cmd_jump_to` calling `Stack::forward`
/// instead of `Stack::jump_to`.
fn cmd_forward(state: &mut State) -> Effects {
    if !state.tabs.active_mut().active_stack_mut().forward() {
        return Effects::default();
    }
    let jobs = ensure_listed(state);
    Effects {
        jobs,
        events: vec![Event::Stack],
    }
}

fn cmd_reload(state: &mut State) -> Effects {
    if let Some(search) = &state.search {
        return cmd_start_search(state, search.query.clone(), search.mode);
    }
    let dir = state.active_frame().dir.clone();
    state.active_frame_mut().loading = true;
    Effects {
        jobs: vec![Job::List(dir)],
        events: vec![Event::Stack],
    }
}

/// Move the active frame's cursor to `row`, clamped into `rows`. Returns no
/// event, and therefore does not bump `state.version`, when the cursor did
/// not actually move -- a `CursorBy` that tries to walk off either end of the
/// list is a no-op, not a redraw.
fn set_cursor(state: &mut State, row: usize) -> Effects {
    if let Some(search) = state.search.as_mut() {
        let cursor = row.min(search.results.len().saturating_sub(1));
        if cursor == search.cursor {
            return Effects::default();
        }
        search.cursor = cursor;
        return Effects {
            jobs: vec![],
            events: vec![Event::Stack],
        };
    }
    let dir = state.active_frame().dir.clone();
    let listing = state.listings.get(&dir).cloned();
    let frame = state.active_frame_mut();
    let clamped = if frame.rows.is_empty() {
        0
    } else {
        row.min(frame.rows.len() - 1)
    };
    if clamped == frame.cursor {
        return Effects::default();
    }
    frame.cursor = clamped;
    if let Some(listing) = &listing {
        sync_cursor_name(frame, listing);
    }
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Stack],
    }
}

fn cmd_cursor_by(state: &mut State, delta: i32) -> Effects {
    let current = state
        .search
        .as_ref()
        .map(|search| search.cursor)
        .unwrap_or(state.active_frame().cursor) as i64;
    let target = (current + delta as i64).max(0) as usize;
    set_cursor(state, target)
}

fn cmd_start_search(state: &mut State, query: String, mode: search::Mode) -> Effects {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Effects {
            jobs: vec![],
            events: vec![Event::Note(Note::warning("search", "enter a search term"))],
        };
    }
    let root = state
        .search
        .as_ref()
        .map(|search| search.root.clone())
        .unwrap_or_else(|| state.active_frame().dir.clone());
    if let Some(search) = state.search.take() {
        search
            .progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    state.search_generation = state.search_generation.wrapping_add(1);
    let generation = state.search_generation;
    let progress = Arc::new(search::Progress::default());
    let include_hidden = state.show_hidden;
    state.search = Some(Search {
        generation,
        root: root.clone(),
        query: query.clone(),
        mode,
        include_hidden,
        results: vec![],
        identities: HashMap::new(),
        excerpts: HashMap::new(),
        skipped_binary: 0,
        skipped_large: 0,
        cursor: 0,
        view: 0,
        restore_cursor_path: None,
        status: SearchStatus::Running,
        errors: vec![],
        progress: Arc::clone(&progress),
    });
    if !state
        .preview
        .as_ref()
        .and_then(|(_, p)| p.extension())
        .is_some_and(|info| info.interactive)
    {
        state.preview_generation += 1;
        state.preview = None;
    }
    Effects {
        jobs: vec![Job::Search {
            tab: state.tabs.active().id,
            generation,
            root,
            query,
            mode,
            include_hidden,
            progress,
        }],
        events: vec![Event::Stack],
    }
}

fn close_search(state: &mut State) -> Effects {
    let Some(search) = state.search.take() else {
        return Effects::default();
    };
    search
        .progress
        .cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    if !state
        .preview
        .as_ref()
        .and_then(|(_, p)| p.extension())
        .is_some_and(|info| info.interactive)
    {
        state.preview_generation += 1;
        state.preview = None;
    }
    Effects {
        jobs: vec![],
        events: vec![Event::Stack],
    }
}

fn cmd_set_filter(state: &mut State, query: String) -> Effects {
    state.active_frame_mut().filter = query;
    rebuild_active(state, Landing::Reset);
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Stack],
    }
}

fn cmd_clear_filter(state: &mut State) -> Effects {
    state.active_frame_mut().filter.clear();
    rebuild_active(state, Landing::Refind);
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Stack],
    }
}

fn cmd_set_sort(state: &mut State, order: SortOrder) -> Effects {
    match state.tabs.active().active_stack {
        0 => state.sort = order,
        index => state.pane_sorts[index - 1] = order,
    }
    rebuild_current_stack(state);
    if let Some(search) = state.search.as_mut() {
        let selected = search
            .results
            .get(search.cursor)
            .map(|entry| entry.path.clone());
        let indices = sort::order(&search.results, order, true);
        let sorted: Vec<_> = indices
            .into_iter()
            .map(|i| search.results[i].clone())
            .collect();
        search.cursor = selected
            .and_then(|path| sorted.iter().position(|entry| entry.path == path))
            .unwrap_or(0);
        search.results = sorted;
    }
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Stack],
    }
}

fn cmd_set_hidden(state: &mut State, hidden: bool) -> Effects {
    state.show_hidden = hidden;
    rebuild_all_frames(state);
    if let Some(search) = &state.search {
        return cmd_start_search(state, search.query.clone(), search.mode);
    }
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Stack],
    }
}

fn cmd_toggle_mark(state: &mut State) -> Effects {
    let Some(entry) = state.cursor_entry().cloned() else {
        return Effects::default();
    };
    state.selection.toggle(&entry);
    sync_search_mark_identity(state, &entry);
    let mut jobs = Vec::new();
    if entry.kind == EntryKind::Dir && state.selection.is_marked(&entry.path) {
        jobs.push(Job::Summarize(entry.path));
    }
    bump_cursor_down(state);
    Effects {
        jobs,
        events: vec![Event::Selection, Event::Stack],
    }
}

fn cmd_mark_all(state: &mut State) -> Effects {
    let entries = visible_entries(state);
    if entries.is_empty() {
        return Effects::default();
    }
    state.selection.mark_all(&entries);
    for entry in &entries {
        sync_search_mark_identity(state, entry);
    }
    let jobs = summarize_marked_dirs(state, &entries);
    Effects {
        jobs,
        events: vec![Event::Selection],
    }
}

fn cmd_invert_marks(state: &mut State) -> Effects {
    let entries = visible_entries(state);
    if entries.is_empty() {
        return Effects::default();
    }
    state.selection.invert(&entries);
    for entry in &entries {
        sync_search_mark_identity(state, entry);
    }
    let jobs = summarize_marked_dirs(state, &entries);
    Effects {
        jobs,
        events: vec![Event::Selection],
    }
}

fn cmd_clear_marks(state: &mut State) -> Effects {
    if state.selection.is_empty() {
        return Effects::default();
    }
    for path in state.selection.paths() {
        state.marked_search_identities.remove(path);
    }
    state.selection.forget();
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Selection],
    }
}

fn cmd_yank(state: &mut State) -> Effects {
    let mut sources: Vec<PathBuf> = state.selection.paths().map(Path::to_path_buf).collect();
    if sources.is_empty() {
        sources.extend(state.cursor_entry().map(|entry| entry.path.clone()));
    }
    if sources.is_empty() {
        return Effects {
            jobs: Vec::new(),
            events: vec![Event::Note(Note::warning(
                "nothing-to-yank",
                "nothing to yank",
            ))],
        };
    }
    let count = sources.len();
    state.yanked_search_identities = sources
        .iter()
        .filter_map(|path| {
            state
                .marked_search_identities
                .get(path)
                .or_else(|| {
                    state
                        .search
                        .as_ref()
                        .and_then(|search| search.identities.get(path))
                })
                .map(|identity| (path.clone(), *identity))
        })
        .collect();
    state.yanked = sources;
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Note(Note::info(format!(
            "yanked {count} {}",
            if count == 1 { "item" } else { "items" }
        )))],
    }
}

fn cmd_paste_here(state: &mut State) -> Effects {
    if state.yanked.is_empty() {
        return Effects {
            jobs: Vec::new(),
            events: vec![Event::Note(Note::warning(
                "nothing-yanked",
                "nothing yanked yet",
            ))],
        };
    }
    let sources = state.yanked.clone();
    let dest = state.active_frame().dir.clone();
    if let Some(effects) = copy_lock_note(copy_lock_conflict(
        state,
        OpKind::Copy,
        &sources,
        Some(&dest),
    )) {
        return effects;
    }
    let id = state
        .queue
        .enqueue(OpKind::Copy, sources.clone(), Some(dest), state.conflicts);
    record_search_expectations(state, id, &sources);
    if let Some(op) = state.queue.get_mut(id) {
        for path in &sources {
            if let Some(identity) = state.yanked_search_identities.get(path) {
                op.expected
                    .retain(|(expected_path, _)| expected_path != path);
                op.expected.push((path.clone(), *identity));
            }
        }
    }
    queued_effects(state, id)
}

/// `QueueMoveHere`: everything marked, into the opposite Commander pane or
/// the active Fold directory. A queue entry is never built with an empty source
/// list -- an empty selection is a note, not a no-op `Op` sitting in the
/// panel.
fn cmd_queue_move(state: &mut State) -> Effects {
    let mut sources: Vec<PathBuf> = state.selection.paths().map(Path::to_path_buf).collect();
    if state.commander && sources.is_empty() {
        sources.extend(state.cursor_entry().map(|entry| entry.path.clone()));
    }
    if sources.is_empty() {
        return nothing_marked_note();
    }
    let dest = if state.commander {
        state.tabs.active().stacks[2 - state.commander_pane]
            .active()
            .dir
            .clone()
    } else {
        state.active_frame().dir.clone()
    };
    if let Some(effects) = copy_lock_note(copy_lock_conflict(
        state,
        OpKind::Move,
        &sources,
        Some(&dest),
    )) {
        return effects;
    }
    let id = state
        .queue
        .enqueue(OpKind::Move, sources.clone(), Some(dest), state.conflicts);
    record_search_expectations(state, id, &sources);
    queued_effects(state, id)
}

/// `QueueDelete`: the marked entries, or the entry under the cursor when
/// nothing is marked -- the one queueing command that still has something to
/// do with an empty selection. `DeleteHow` is decided once, here, from
/// `[ops] trash` and whether a trash was found at startup, so the queue
/// entry's own title (`TRASH` vs `DELETE`) is right before anything runs.
fn cmd_queue_delete(state: &mut State) -> Effects {
    let mut sources: Vec<PathBuf> = state.selection.paths().map(Path::to_path_buf).collect();
    if sources.is_empty() {
        if let Some(entry) = state.cursor_entry() {
            sources.push(entry.path.clone());
        }
    }
    if sources.is_empty() {
        return nothing_marked_note();
    }
    queue_delete_sources(state, sources)
}
fn queue_delete_sources(state: &mut State, sources: Vec<PathBuf>) -> Effects {
    if sources.is_empty() {
        return Effects::default();
    }
    let how = if state.trash == TrashMode::Always
        || (state.trash == TrashMode::Auto && state.trash_available)
    {
        DeleteHow::Trash
    } else {
        DeleteHow::Permanent
    };
    queue_delete_sources_with_how(state, sources, how)
}

fn queue_delete_sources_with_how(
    state: &mut State,
    sources: Vec<PathBuf>,
    how: DeleteHow,
) -> Effects {
    if sources.is_empty() {
        return Effects::default();
    }
    if let Some(effects) = copy_lock_note(copy_lock_conflict(
        state,
        OpKind::Delete(how),
        &sources,
        None,
    )) {
        return effects;
    }
    let id = state
        .queue
        .enqueue(OpKind::Delete(how), sources.clone(), None, state.conflicts);
    record_search_expectations(state, id, &sources);
    queued_effects(state, id)
}

fn cmd_queue_elevated_delete(state: &mut State, failed_id: OpId) -> Effects {
    let Some(failed) = state.queue.iter().find(|op| op.id == failed_id) else {
        return Effects::default();
    };
    if failed.status != OpStatus::Failed
        || failed.kind != OpKind::Delete(DeleteHow::Permanent)
        || failed.label.is_some()
    {
        return Effects::default();
    }
    let sources: Vec<PathBuf> = failed
        .failed
        .iter()
        .filter(|(path, reason)| {
            failed.sources.contains(path) && super::elevated::is_permission_error(reason)
        })
        .map(|(path, _)| path.clone())
        .collect();
    if sources.is_empty() {
        return Effects::default();
    }
    if let Some(effects) = copy_lock_note(copy_lock_conflict(
        state,
        OpKind::Delete(DeleteHow::Permanent),
        &sources,
        None,
    )) {
        return effects;
    }
    let id = state.queue.enqueue(
        OpKind::Delete(DeleteHow::Permanent),
        sources,
        None,
        state.conflicts,
    );
    state.queue.get_mut(id).expect("just enqueued").elevated = true;
    queued_effects(state, id)
}

fn cmd_empty_drive_trash(state: &mut State, mount: &Path) -> Effects {
    let Some(location) = state
        .places
        .locations
        .iter()
        .find(|location| location.path == mount && location.info.is_some())
    else {
        return Effects {
            jobs: Vec::new(),
            events: vec![Event::Note(Note::warning(
                "drive-trash",
                "drive is no longer available",
            ))],
        };
    };
    if location
        .info
        .as_ref()
        .is_some_and(|info| info.trash_disabled)
    {
        return Effects {
            jobs: Vec::new(),
            events: vec![Event::Note(Note::warning(
                "drive-trash",
                "Trash is disabled on this drive",
            ))],
        };
    }
    let label = format!("EMPTY TRASH: {}", location.name);
    let sources = places::drive_trash_paths(mount);
    if let Some(effects) = copy_lock_note(copy_lock_conflict(
        state,
        OpKind::Delete(DeleteHow::Permanent),
        &sources,
        None,
    )) {
        return effects;
    }
    let id = state.queue.enqueue(
        OpKind::Delete(DeleteHow::Permanent),
        sources,
        Some(mount.to_path_buf()),
        state.conflicts,
    );
    state.queue.get_mut(id).expect("just enqueued").label = Some(label);
    queued_effects(state, id)
}

fn cmd_queue_rename(state: &mut State, from: PathBuf, to: PathBuf) -> Effects {
    if let Some(effects) = copy_lock_note(copy_lock_conflict(
        state,
        OpKind::Rename,
        std::slice::from_ref(&from),
        Some(&to),
    )) {
        return effects;
    }
    let id = state.queue.enqueue(
        OpKind::Rename,
        vec![from.clone()],
        Some(to),
        state.conflicts,
    );
    record_search_expectations(state, id, &[from]);
    queued_effects(state, id)
}

/// A copy reads its sources and may replace each named destination. Keep
/// conflicting writes out of every entry point, including the IO worker's
/// separate create path and remote drag and drop. The queue's own jobs run in
/// series, but a newly requested action should not silently wait and then
/// change a file the user saw being copied.
fn copy_lock_conflict(
    state: &State,
    kind: OpKind,
    sources: &[PathBuf],
    dest: Option<&Path>,
) -> Option<PathBuf> {
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    match kind {
        OpKind::Copy => reads.extend_from_slice(sources),
        OpKind::Move | OpKind::Delete(_) | OpKind::Rename => writes.extend_from_slice(sources),
        OpKind::ArchiveTest | OpKind::Compress(_) | OpKind::Extract => {
            reads.extend_from_slice(sources)
        }
        OpKind::ArchiveSave | OpKind::ArchiveDiscard => writes.extend_from_slice(sources),
    }
    if let Some(dest) = dest {
        match kind {
            OpKind::Copy | OpKind::Move => writes.extend(copy_targets(sources, dest)),
            OpKind::Rename | OpKind::Compress(_) | OpKind::Extract => {
                writes.push(dest.to_path_buf())
            }
            OpKind::ArchiveSave
            | OpKind::ArchiveDiscard
            | OpKind::ArchiveTest
            | OpKind::Delete(_) => {}
        }
    }
    copy_lock_conflict_paths(state, &reads, &writes)
}

fn copy_targets(sources: &[PathBuf], dest: &Path) -> Vec<PathBuf> {
    sources
        .iter()
        .map(|source| {
            source
                .file_name()
                .map_or_else(|| dest.to_path_buf(), |name| dest.join(name))
        })
        .collect()
}

fn paths_overlap(a: &Path, b: &Path) -> bool {
    fn physical(path: &Path) -> PathBuf {
        match super::location::Location::from_key(path) {
            Ok(super::location::Location::Archive { source, .. }) => source.file,
            _ => path.into(),
        }
    }
    if super::location::is_archive(a) || super::location::is_archive(b) {
        let a = physical(a);
        let b = physical(b);
        a.starts_with(&b) || b.starts_with(&a)
    } else {
        a.starts_with(b) || b.starts_with(a)
    }
}

fn copy_lock_conflict_paths(
    state: &State,
    reads: &[PathBuf],
    writes: &[PathBuf],
) -> Option<PathBuf> {
    for op in state.queue.iter().filter(|op| {
        (matches!(
            op.kind,
            OpKind::Copy
                | OpKind::Compress(_)
                | OpKind::Extract
                | OpKind::ArchiveSave
                | OpKind::ArchiveTest
        ) || op.import_sources.is_some())
            && matches!(
                op.status,
                OpStatus::Planning | OpStatus::NeedsPolicy | OpStatus::Running
            )
    }) {
        let source_locks = if op.import_sources.is_some() {
            &[][..]
        } else {
            op.sources.as_slice()
        };
        let targets = op
            .dest
            .as_deref()
            .map(|dest| copy_targets(op.import_sources.as_deref().unwrap_or(&op.sources), dest))
            .unwrap_or_default();
        let targets = targets
            .into_iter()
            .chain(op.rename_targets.iter().map(|(_, target)| target.clone()))
            .collect::<Vec<_>>();
        if let Some(path) = writes.iter().find(|path| {
            source_locks
                .iter()
                .chain(targets.iter())
                .any(|locked| paths_overlap(path, locked))
        }) {
            return Some(path.clone());
        }
        if let Some(path) = reads
            .iter()
            .find(|path| targets.iter().any(|locked| paths_overlap(path, locked)))
        {
            return Some(path.clone());
        }
    }
    None
}

fn copy_lock_note(path: Option<PathBuf>) -> Option<Effects> {
    path.map(|path| Effects {
        jobs: Vec::new(),
        events: vec![Event::Note(Note::warning(
            "copy-locked",
            format!("Copy in progress: {} is locked", path.display()),
        ))],
    })
}

fn queued_effects(state: &mut State, id: OpId) -> Effects {
    let mut effects = run_next(state);
    effects.events.insert(0, Event::Queue(id));
    effects
}

fn record_search_expectations(state: &mut State, id: OpId, sources: &[PathBuf]) {
    let expected: Vec<_> = sources
        .iter()
        .filter_map(|path| {
            state
                .marked_search_identities
                .get(path)
                .or_else(|| {
                    state
                        .search
                        .as_ref()
                        .and_then(|search| search.identities.get(path))
                })
                .map(|identity| (path.clone(), *identity))
        })
        .collect();
    if let Some(op) = state.queue.get_mut(id) {
        op.expected = expected;
    }
}

fn sync_search_mark_identity(state: &mut State, entry: &Entry) {
    if !state.selection.is_marked(&entry.path) {
        state.marked_search_identities.remove(&entry.path);
    } else if let Some(identity) = state
        .search
        .as_ref()
        .and_then(|search| search.identities.get(&entry.path))
    {
        state
            .marked_search_identities
            .insert(entry.path.clone(), *identity);
    }
}

fn nothing_marked_note() -> Effects {
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Note(Note::warning(
            "nothing-marked",
            "nothing is marked",
        ))],
    }
}

fn cmd_remove_op(state: &mut State, id: OpId) -> Effects {
    if state
        .queue
        .get_mut(id)
        .is_some_and(|op| matches!(op.status, OpStatus::Planning | OpStatus::Running))
    {
        return Effects::default();
    }
    if state.queue.remove(id) {
        Effects {
            jobs: Vec::new(),
            events: vec![Event::Queue(id)],
        }
    } else {
        Effects::default()
    }
}

/// `esc` on the queue: drop everything pending, one `Event::Queue` per entry
/// removed -- events carry no data, so a removal the UI has to notice is
/// named the same way whether it came from `esc` or from `x` on one row.
fn cmd_clear_queue(state: &mut State) -> Effects {
    let removed: Vec<OpId> = state
        .queue
        .iter()
        .filter(|op| matches!(op.status, OpStatus::Queued | OpStatus::NeedsPolicy))
        .map(|op| op.id)
        .collect();
    if removed.is_empty() {
        return Effects::default();
    }
    state.queue.clear_pending();
    Effects {
        jobs: Vec::new(),
        events: removed.into_iter().map(Event::Queue).collect(),
    }
}

/// `Run`: start the next runnable op if none is already in flight. Called
/// directly by `Command::Run`, and again by `Done::Planned`'s failure arm and
/// by `Done::Finished` to keep the queue moving on its own once it has
/// started -- "the queue runs to completion" is this function being called
/// one more time at the end of every op.
fn run_next(state: &mut State) -> Effects {
    let in_flight = state
        .queue
        .iter()
        .any(|op| matches!(op.status, OpStatus::Planning | OpStatus::Running));
    if in_flight {
        return Effects::default();
    }
    let Some(id) = state.queue.next_runnable() else {
        return Effects::default();
    };
    let Some(op) = state.queue.get_mut(id) else {
        return Effects::default();
    };

    if op.elevated {
        op.status = OpStatus::Running;
        op.progress.set_total(op.sources.len() as u64);
        return Effects {
            jobs: vec![Job::RunElevated {
                op: id,
                sources: op.sources.clone(),
                progress: Arc::clone(&op.progress),
            }],
            events: vec![Event::Queue(id)],
        };
    }

    // A `Queued` op with a `Plan` already on it was `NeedsPolicy` a moment
    // ago: `plan` was worked out before the conflict was found, and
    // `Command::SetPolicy` is what moved it back to `Queued`. Re-planning it
    // now would throw that work away and ask the conflict question a second
    // time; running the plan that is already there is what "SetPolicy then
    // Run proceeds to Job::Run" means.
    if let Some(plan) = op.plan.clone() {
        let policy = op.policy;
        let kind = op.kind;
        op.status = OpStatus::Running;
        op.progress.set_total(plan.total_bytes);
        return Effects {
            jobs: vec![Job::Run {
                op: id,
                kind,
                plan,
                policy,
                rename_targets: op.rename_targets.clone(),
                progress: Arc::clone(&op.progress),
                expected: op.expected.clone(),
            }],
            events: vec![Event::Queue(id)],
        };
    }

    op.status = OpStatus::Planning;
    let job = Job::Plan {
        op: id,
        kind: op.kind,
        sources: op.sources.clone(),
        dest: op.dest.clone(),
        expected: op.expected.clone(),
        archive_options: op.archive_options.clone(),
    };
    Effects {
        jobs: vec![job],
        events: vec![Event::Queue(id)],
    }
}

fn cmd_set_policy(state: &mut State, id: OpId, policy: ConflictPolicy) -> Effects {
    let Some(op) = state.queue.get_mut(id) else {
        return Effects::default();
    };
    op.policy = policy;
    if op.import_sources.is_some() && op.kind == OpKind::Copy && op.status == OpStatus::NeedsPolicy
    {
        op.status = OpStatus::Running;
        if policy == ConflictPolicy::Skip {
            op.skipped = op.plan.as_ref().map_or(0, |plan| plan.conflicts.len());
        }
        return Effects {
            jobs: vec![],
            events: vec![Event::Queue(id), Event::ImportReady(id)],
        };
    }
    if op.status == OpStatus::NeedsPolicy {
        op.status = OpStatus::Queued;
    }
    let mut effects = run_next(state);
    effects.events.insert(0, Event::Queue(id));
    effects
}

fn cmd_cancel(state: &mut State, id: OpId) -> Effects {
    let Some(op) = state.queue.get_mut(id) else {
        return Effects::default();
    };
    op.progress.cancel();
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Queue(id)],
    }
}

fn cmd_preview(state: &mut State, path: PathBuf) -> Effects {
    state.preview_generation += 1;
    let mut loading = super::preview::model::Document::new("Loading preview…");
    loading.notice = Some("Loading…".into());
    state.preview = Some((path.clone(), Arc::new(Preview::Document(loading))));
    Effects {
        jobs: vec![Job::Preview {
            tab: state.tabs.active().id,
            path,
            generation: state.preview_generation,
        }],
        events: vec![Event::Preview],
    }
}

// ---------------------------------------------------------------------
// Worker results.
// ---------------------------------------------------------------------

fn done_created(
    state: &mut State,
    path: PathBuf,
    kind: CreateKind,
    origin: CreateOrigin,
    result: Result<(), String>,
) -> Effects {
    if let Err(error) = result {
        return Effects {
            jobs: vec![],
            events: vec![Event::Note(Note::error("create", error))],
        };
    }

    let Some(dir) = path.parent().map(Path::to_path_buf) else {
        return Effects::default();
    };
    let name = path.file_name().map(|name| name.to_os_string());
    let mut events = vec![Event::Note(Note::info(format!(
        "Created {} {}",
        kind.label(),
        path.display()
    )))];

    // A newly created dotfile must be visible if the cursor is to land on it.
    // Rebuild first: doing it after setting cursor_name would replace that
    // name with an old entry because the fresh listing has not arrived yet.
    if path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
    {
        if origin.tab == state.tabs.active().id {
            state.show_hidden = true;
        } else if let Some(context) = state
            .tabs
            .tabs
            .iter_mut()
            .find(|t| t.id == origin.tab)
            .and_then(|t| t.context.as_mut())
        {
            context.show_hidden = true;
        }
        rebuild_all_frames(state);
        events.push(Event::Stack);
    }

    // The user may have switched panes or jumped to another Fold level while
    // the IO worker was busy. Only the frame that requested this creation
    // should move its cursor when its listing refreshes.
    if let Some(frame) = state
        .tabs
        .tabs
        .iter_mut()
        .find(|tab| tab.id == origin.tab)
        .and_then(|tab| tab.stacks.get_mut(origin.stack))
        .and_then(|stack| stack.frames_mut().find(|frame| frame.id == origin.frame))
        .filter(|frame| frame.dir == dir)
    {
        frame.cursor_name = name;
        frame.filter.clear();
        events.push(Event::Stack);
    }

    let jobs = if all_frame_dirs(state).contains(&dir) {
        vec![Job::List(dir)]
    } else {
        vec![]
    };
    Effects { jobs, events }
}

fn done_listed(state: &mut State, listing: Listing) -> Effects {
    let dir = listing.dir.clone();
    let error = listing.error.clone();
    let listing = Arc::new(listing);
    state.listings.insert(dir.clone(), Arc::clone(&listing));

    let fold_sort = state.sort;
    let pane_sorts = state.pane_sorts;
    let hidden = state.show_hidden;
    for tab in &mut state.tabs.tabs {
        let (fold_sort, pane_sorts, hidden) = tab
            .context
            .as_ref()
            .map(|c| (c.sort, c.pane_sorts, c.show_hidden))
            .unwrap_or((fold_sort, pane_sorts, hidden));
        for (index, stack) in tab.stacks.iter_mut().enumerate() {
            let sort = sort_for_stack(index, fold_sort, pane_sorts);
            for frame in stack.frame_mut_by_dir(&dir) {
                frame.loading = false;
                rebuild_rows(frame, &listing, sort, hidden, Landing::Refind);
            }
        }
    }

    if state.listings.contains_key(&state.active_frame().dir) {
        state.loading = false;
    }

    let mut events = vec![Event::Listing(dir.clone())];
    let mut jobs = Vec::new();

    if let Some(err) = error {
        if state.active_frame().dir == dir {
            events.push(Event::Note(Note::warning("listing", err.clone())));
            // "no such directory" (see `listing::read`'s `describe`) is the
            // one failure that means the level itself is gone rather than
            // merely unreadable -- a permission wall still draws a frame for
            // the directory the user is looking at, but a directory that no
            // longer exists cannot be looked at at all.
            if !state.durable_tabs && !state.commander && err.to_lowercase().contains("no such") {
                let popped = state.tabs.active_mut().active_stack_mut().pop();
                if popped {
                    events.push(Event::Stack);
                    jobs.extend(ensure_listed(state));
                }
            }
        }
    }

    evict_unreferenced_listings(state);

    Effects { jobs, events }
}

fn done_summarized(state: &mut State, dir: PathBuf, summary: DirSummary) -> Effects {
    let mut events = Vec::new();
    for tab in &mut state.tabs.tabs {
        if let Some(c) = &mut tab.context {
            if c.selection.is_marked(&dir) {
                c.selection.sized(&dir, summary.bytes);
            }
            for selection in &mut c.parked_selections {
                if selection.is_marked(&dir) {
                    selection.sized(&dir, summary.bytes);
                }
            }
        }
    }
    if state.selection.is_marked(&dir) {
        state.selection.sized(&dir, summary.bytes);
        events.push(Event::Selection);
    }
    for selection in &mut state.parked_selections {
        if selection.is_marked(&dir) {
            selection.sized(&dir, summary.bytes);
            events.push(Event::Selection);
        }
    }
    Effects {
        jobs: Vec::new(),
        events,
    }
}

fn done_previewed(
    state: &mut State,
    path: PathBuf,
    generation: u64,
    mut preview: Preview,
) -> Effects {
    if generation != state.preview_generation {
        return Effects::default();
    }
    if let Some(new) = preview.extension_mut() {
        if let Some(old) = state
            .preview
            .as_ref()
            .and_then(|(_, p)| p.extension())
            .filter(|old| old.session == new.session)
        {
            if old.actions.len() + new.actions.len() <= 256 {
                let mut actions = old.actions.clone();
                actions.append(&mut new.actions);
                new.actions = actions;
            } else {
                return Effects {
                    jobs: vec![],
                    events: vec![Event::Note(Note::warning(
                        "preview",
                        "Preview input backlog exceeded; wait for controls to finish",
                    ))],
                };
            }
        }
    }
    if let Preview::Document(new) = &mut preview {
        if new.extension.is_some() {
            state.preview = Some((path, Arc::new(preview)));
            return Effects {
                jobs: vec![],
                events: vec![Event::Preview],
            };
        }
        if let Some((old_path, old)) = &state.preview {
            if *old_path == path {
                if let Preview::Document(old) = old.as_ref() {
                    use super::preview::model::Content;
                    if matches!(old.content, Content::Pages(_))
                        && matches!(new.content, Content::Metadata)
                    {
                        new.content = old.content.clone();
                        new.total_pages = old.total_pages;
                    }
                    if let (Content::Pages(previous), Content::Pages(incoming)) =
                        (&old.content, &mut new.content)
                    {
                        if previous
                            .last()
                            .zip(incoming.first())
                            .is_some_and(|(a, b)| a.number + 1 == b.number)
                        {
                            let mut pages = previous.clone();
                            pages.append(incoming);
                            if pages.len() > 24 {
                                pages.drain(..pages.len() - 24);
                            }
                            *incoming = pages;
                        } else if incoming
                            .last()
                            .zip(previous.first())
                            .is_some_and(|(a, b)| a.number + 1 == b.number)
                        {
                            incoming.extend(previous.iter().cloned());
                            incoming.truncate(24);
                            new.next_page = incoming.last().and_then(|p| {
                                (Some(p.number) < new.total_pages).then_some(p.number + 1)
                            });
                        }
                    }
                }
            }
        }
    }
    state.preview = Some((path, Arc::new(preview)));
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Preview],
    }
}

fn done_import_checked(
    state: &mut State,
    op_id: OpId,
    result: Result<Vec<super::ops::Conflict>, String>,
) -> Effects {
    let Some(op) = state.queue.get_mut(op_id) else {
        return Effects::default();
    };
    if op.import_sources.is_none() || op.status != OpStatus::Planning {
        return Effects::default();
    }
    if op.progress.is_cancelled() {
        op.status = OpStatus::Cancelled;
        return Effects {
            jobs: vec![],
            events: vec![Event::Queue(op_id)],
        };
    }
    match result {
        Err(error) => {
            op.status = OpStatus::Failed;
            op.failure = Some(error.clone());
            Effects {
                jobs: vec![],
                events: vec![
                    Event::Queue(op_id),
                    Event::Note(Note::error("import-check", error)),
                ],
            }
        }
        Ok(conflicts) if !conflicts.is_empty() && op.policy == ConflictPolicy::Ask => {
            op.plan = Some(super::ops::Plan {
                conflicts,
                ..Default::default()
            });
            op.status = OpStatus::NeedsPolicy;
            Effects {
                jobs: vec![],
                events: vec![Event::Queue(op_id), Event::Conflicts(op_id)],
            }
        }
        Ok(conflicts) => {
            if op.policy == ConflictPolicy::Skip {
                op.skipped = conflicts.len();
            }
            op.plan = Some(super::ops::Plan {
                conflicts,
                ..Default::default()
            });
            op.status = OpStatus::Running;
            Effects {
                jobs: vec![],
                events: vec![Event::Queue(op_id), Event::ImportReady(op_id)],
            }
        }
    }
}

fn done_planned(
    state: &mut State,
    op_id: OpId,
    result: Result<super::ops::Plan, String>,
) -> Effects {
    if state
        .queue
        .get_mut(op_id)
        .is_some_and(|op| op.progress.is_cancelled())
    {
        if let Some(op) = state.queue.get_mut(op_id) {
            op.status = OpStatus::Cancelled;
        }
        let mut effects = run_next(state);
        effects.events.insert(0, Event::Queue(op_id));
        return effects;
    }
    match result {
        Err(message) => {
            if let Some(op) = state.queue.get_mut(op_id) {
                op.status = OpStatus::Failed;
                op.failure = Some(message.clone());
            }
            let mut effects = run_next(state);
            effects.events.insert(0, Event::Queue(op_id));
            effects
                .events
                .insert(0, Event::Note(Note::error("op-plan", message)));
            effects
        }
        Ok(plan) => {
            let has_conflicts = !plan.conflicts.is_empty();
            let mut jobs = Vec::new();
            let mut events = vec![Event::Queue(op_id)];
            if let Some(op) = state.queue.get_mut(op_id) {
                let policy = op.policy;
                let kind = op.kind;
                op.plan = Some(plan.clone());
                if has_conflicts && policy == ConflictPolicy::Ask {
                    op.status = OpStatus::NeedsPolicy;
                    events.push(Event::Conflicts(op_id));
                } else {
                    op.status = OpStatus::Running;
                    op.progress.set_total(plan.total_bytes);
                    jobs.push(Job::Run {
                        op: op_id,
                        kind,
                        plan,
                        policy,
                        rename_targets: op.rename_targets.clone(),
                        progress: Arc::clone(&op.progress),
                        expected: op.expected.clone(),
                    });
                }
            }
            Effects { jobs, events }
        }
    }
}

/// `Done::Finished`: settle the op's status, forget whatever it moved or
/// deleted from the selection, ask for a fresh listing wherever a frame is
/// sitting on the destination or a source's parent, and start the next
/// runnable op -- the queue does not wait for `Command::Run` again once it
/// has been told to go.
fn done_finished(state: &mut State, op_id: OpId, outcome: super::ops::Outcome) -> Effects {
    let mut events = vec![Event::Queue(op_id)];
    let mut touch_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    let mut moved_or_deleted: Vec<PathBuf> = Vec::new();

    if let Some(op) = state.queue.get_mut(op_id) {
        op.skipped += outcome.skipped;
        op.status = if outcome.cancelled {
            OpStatus::Cancelled
        } else if !outcome.failed.is_empty() {
            OpStatus::Failed
        } else {
            OpStatus::Done
        };

        if op.status == OpStatus::Failed {
            let total = outcome.done + outcome.skipped + outcome.failed.len();
            let reason = outcome.failed.first().map(|(_, reason)| reason.clone());
            op.failure = reason.clone();
            op.failed = outcome.failed.clone();
            let detail = reason.map_or_else(String::new, |reason| format!(": {reason}"));
            events.push(Event::Note(Note::error(
                "op-failed",
                format!("{} of {} failed{detail}", outcome.failed.len(), total),
            )));
            if op.kind == OpKind::Delete(DeleteHow::Trash)
                && state.trash == TrashMode::Auto
                && op.sources.len() == 1
                && !op.sources.iter().any(|p| super::location::is_archive(p))
            {
                op.trash_failure = Some(outcome.failed[0].1.clone());
            }
        }

        // Every kind consumes its marks. A move, a delete and a rename have
        // taken the paths away; a copy has not, but the marks were the thing
        // being copied, and a `d` pressed afterwards that deleted the
        // originals because they were still marked is a mistake nobody
        // should be able to make.
        // The trash library may report the unusable trash directory as the
        // failing path rather than the requested source. A failed batch does
        // not identify which sources moved, so retain its marks.
        if !outcome.cancelled
            && !(op.kind == OpKind::Delete(DeleteHow::Trash) && !outcome.failed.is_empty())
        {
            for src in &op.sources {
                if !outcome.failed.iter().any(|(p, _)| p == src) {
                    moved_or_deleted.push(src.clone());
                }
            }
        }

        if matches!(op.kind, OpKind::ArchiveSave | OpKind::ArchiveDiscard)
            && op.status == OpStatus::Done
        {
            state.archive_materialized.clear();
            state.archive_editable.clear();
            state.preview = None;
            state.preview_generation += 1;
        }
        if let Some(dest) = &op.dest {
            touch_dirs.insert(dest.clone());
            if matches!(op.kind, OpKind::Compress(_) | OpKind::Extract) {
                if let Some(parent) = dest.parent() {
                    touch_dirs.insert(parent.to_path_buf());
                }
            }
        }
        for src in &op.sources {
            if let Some(parent) = src.parent() {
                touch_dirs.insert(parent.to_path_buf());
            }
        }
    }

    if !moved_or_deleted.is_empty() {
        for tab in &mut state.tabs.tabs {
            if let Some(context) = &mut tab.context {
                context.selection.forget_paths(&moved_or_deleted);
                for selection in &mut context.parked_selections {
                    selection.forget_paths(&moved_or_deleted);
                }
                for path in &moved_or_deleted {
                    context.marked_search_identities.remove(path);
                }
            }
        }
        state.selection.forget_paths(&moved_or_deleted);
        for selection in &mut state.parked_selections {
            selection.forget_paths(&moved_or_deleted);
        }
        for path in &moved_or_deleted {
            state.marked_search_identities.remove(path);
        }
        events.push(Event::Selection);
    }

    let mut frame_dirs = all_frame_dirs(state);
    frame_dirs.extend(
        state
            .listings
            .values()
            .filter(|l| l.archive_changes > 0)
            .map(|l| l.dir.clone()),
    );
    touch_dirs.extend(
        frame_dirs
            .iter()
            .filter(|d| super::location::is_archive(d))
            .cloned(),
    );
    let jobs: Vec<Job> = touch_dirs
        .into_iter()
        .filter(|dir| frame_dirs.contains(dir))
        .map(Job::List)
        .collect();

    let mut next = run_next(state);
    let mut jobs = jobs;
    jobs.append(&mut next.jobs);
    events.append(&mut next.events);

    Effects { jobs, events }
}

fn done_changed(state: &mut State, dirs: Vec<PathBuf>) -> Effects {
    let frame_dirs = all_frame_dirs(state);
    let jobs: Vec<Job> = dirs
        .into_iter()
        .filter(|d| frame_dirs.contains(d))
        .map(Job::List)
        .collect();
    Effects {
        jobs,
        events: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn cfg() -> FoldConfig {
        FoldConfig::default()
    }

    fn state_at(dir: &str) -> State {
        State::new(&cfg(), PathBuf::from(dir), PathBuf::from(dir), true)
    }

    #[test]
    fn remote_import_keeps_one_operation_and_one_progress_counter() {
        let mut state = state_at("/home");
        apply(
            &mut state,
            Change::Command(Command::BeginImport {
                sources: vec!["/remote/photo.jpg".into()],
                dest: "/home/photos".into(),
            }),
        );
        let op = state.queue.iter().last().unwrap();
        let id = op.id;
        let checked = apply(
            &mut state,
            Change::Done(Done::ImportChecked {
                op: id,
                result: Ok(vec![]),
            }),
        );
        assert!(checked
            .events
            .iter()
            .any(|event| matches!(event, Event::ImportReady(found) if *found == id)));
        let op = state.queue.get_mut(id).unwrap();
        op.progress.add(100);
        assert_eq!(op.progress.done(), 100);
        assert_eq!(op.progress.total(), 0);

        let effects = apply(
            &mut state,
            Change::Command(Command::CompleteImport {
                op: id,
                sources: vec!["/home/photos/.starfold-drop/photo.jpg".into()],
            }),
        );
        assert!(
            matches!(effects.jobs.as_slice(), [Job::Plan { op, kind: OpKind::Move, .. }] if *op == id)
        );
        let op = state.queue.get_mut(id).unwrap();
        assert_eq!(op.title(), "COPY 1 item → /home/photos");
        assert_eq!(
            op.import_sources.as_ref().unwrap()[0],
            Path::new("/remote/photo.jpg")
        );
        assert_eq!(op.progress.percent(), "50.0%");
        assert_eq!(state.queue.len(), 1);

        apply(
            &mut state,
            Change::Done(Done::Planned {
                op: id,
                result: Ok(super::super::ops::Plan {
                    total_bytes: 100,
                    ..Default::default()
                }),
            }),
        );
        let op = state.queue.get_mut(id).unwrap();
        assert_eq!(op.progress.percent(), "50.0%");
        op.progress.add(100);
        assert_eq!(op.progress.percent(), "100.0%");
        assert_eq!(state.queue.len(), 1);
    }

    #[test]
    fn remote_zip_import_plans_staged_copy_instead_of_move() {
        let mut state = state_at("/home");
        let dest = super::super::location::Location::Filesystem("/home/data.zip".into())
            .enter_archive()
            .unwrap()
            .key();
        apply(
            &mut state,
            Change::Command(Command::BeginImport {
                sources: vec!["/remote/photo.jpg".into()],
                dest,
            }),
        );
        let id = state.queue.iter().last().unwrap().id;
        apply(
            &mut state,
            Change::Done(Done::ImportChecked {
                op: id,
                result: Ok(vec![]),
            }),
        );
        let completed = apply(
            &mut state,
            Change::Command(Command::CompleteImport {
                op: id,
                sources: vec!["/home/.starfold-drop/photo.jpg".into()],
            }),
        );
        assert!(matches!(
            completed.jobs.as_slice(),
            [Job::Plan {
                kind: OpKind::Copy,
                ..
            }]
        ));
        assert_eq!(state.queue.get_mut(id).unwrap().kind, OpKind::Copy);
    }
    #[test]
    fn remote_collision_waits_for_policy_before_receiving() {
        let mut state = state_at("/home");
        let source = PathBuf::from("/remote/photo.jpg");
        let dest = PathBuf::from("/home/photos");
        let started = apply(
            &mut state,
            Change::Command(Command::BeginImport {
                sources: vec![source.clone()],
                dest: dest.clone(),
            }),
        );
        let id = state.queue.iter().last().unwrap().id;
        assert!(matches!(started.jobs.as_slice(), [Job::CheckImport { .. }]));
        assert_eq!(state.queue.get_mut(id).unwrap().status, OpStatus::Planning);
        let checked = apply(
            &mut state,
            Change::Done(Done::ImportChecked {
                op: id,
                result: Ok(vec![super::super::ops::Conflict {
                    source,
                    dest: dest.join("photo.jpg"),
                    both_dirs: false,
                }]),
            }),
        );
        assert!(checked
            .events
            .iter()
            .any(|event| matches!(event, Event::Conflicts(found) if *found == id)));
        assert!(!checked
            .events
            .iter()
            .any(|event| matches!(event, Event::ImportReady(_))));
        let op = state.queue.get_mut(id).unwrap();
        assert_eq!(op.status, OpStatus::NeedsPolicy);
        assert_eq!(
            op.plan.as_ref().unwrap().conflicts[0].dest,
            dest.join("photo.jpg")
        );
        let answered = apply(
            &mut state,
            Change::Command(Command::SetPolicy(id, ConflictPolicy::RenameNew)),
        );
        assert!(answered
            .events
            .iter()
            .any(|event| matches!(event, Event::ImportReady(found) if *found == id)));
        assert_eq!(state.queue.get_mut(id).unwrap().status, OpStatus::Running);
    }

    #[test]
    fn remote_conflict_keeps_entered_name_after_staging() {
        let mut state = state_at("/home");
        let dest = PathBuf::from("/home/photos");
        let original = dest.join("photo.jpg");
        apply(
            &mut state,
            Change::Command(Command::BeginImport {
                sources: vec!["/remote/photo.jpg".into()],
                dest: dest.clone(),
            }),
        );
        let id = state.queue.iter().last().unwrap().id;
        let conflict = super::super::ops::Conflict {
            source: "/remote/photo.jpg".into(),
            dest: original.clone(),
            both_dirs: false,
        };
        apply(
            &mut state,
            Change::Done(Done::ImportChecked {
                op: id,
                result: Ok(vec![conflict]),
            }),
        );
        let names = vec![(original.clone(), dest.join("my photo.jpg"))];
        let ready = apply(
            &mut state,
            Change::Command(Command::SetConflictNames(id, names.clone())),
        );
        assert!(ready
            .events
            .iter()
            .any(|event| matches!(event, Event::ImportReady(found) if *found == id)));
        let staged = PathBuf::from("/home/photos/.starfold-drop/photo.jpg");
        apply(
            &mut state,
            Change::Command(Command::CompleteImport {
                op: id,
                sources: vec![staged.clone()],
            }),
        );
        let planned = apply(
            &mut state,
            Change::Done(Done::Planned {
                op: id,
                result: Ok(super::super::ops::Plan {
                    sources: vec![staged.clone()],
                    dest,
                    conflicts: vec![super::super::ops::Conflict {
                        source: staged,
                        dest: original,
                        both_dirs: false,
                    }],
                    ..Default::default()
                }),
            }),
        );
        assert!(
            matches!(planned.jobs.as_slice(), [Job::Run { rename_targets, .. }] if *rename_targets == names)
        );
    }

    #[test]
    fn stopped_remote_receipt_never_plans_the_staged_move() {
        let mut state = state_at("/home");
        apply(
            &mut state,
            Change::Command(Command::BeginImport {
                sources: vec!["/remote/photo.jpg".into()],
                dest: "/home/photos".into(),
            }),
        );
        let id = state.queue.iter().last().unwrap().id;
        apply(&mut state, Change::Command(Command::StopActive(id)));
        let effects = apply(
            &mut state,
            Change::Command(Command::CompleteImport {
                op: id,
                sources: vec!["/home/photos/.starfold-drop/photo.jpg".into()],
            }),
        );
        assert!(effects.jobs.is_empty());
        assert_eq!(state.queue.get_mut(id).unwrap().status, OpStatus::Cancelled);
    }

    fn entry_named(dir: &str, name: &str, kind: EntryKind) -> Entry {
        Entry {
            path: PathBuf::from(dir).join(name),
            display: name.to_string(),
            kind,
            link_kind: None,
            len: 0,
            modified: None,
            created: None,
            accessed: None,
            mode: 0,
            executable: false,
            hidden: name.starts_with('.'),
        }
    }

    fn listing_at(dir: &str, entries: Vec<Entry>) -> Listing {
        Listing {
            dir: PathBuf::from(dir),
            entries,
            truncated: false,
            error: None,
            dir_mtime: None,
            space: None,
            archive_changes: 0,
            archive_writable: false,
        }
    }

    #[test]
    fn a_fresh_state_has_one_tab_one_stack_one_frame_and_is_loading() {
        let s = state_at("/home/bob");
        assert_eq!(s.tabs.tabs.len(), 1);
        assert_eq!(s.tabs.active().stacks.len(), 1);
        assert_eq!(s.active_frame().dir, PathBuf::from("/home/bob"));
        assert!(s.loading);
        assert!(s.trash_available);
        assert_eq!(s.version, 0);
    }

    #[test]
    fn active_frame_mut_reaches_the_same_frame_active_frame_reads() {
        let mut s = state_at("/a");
        s.active_frame_mut().cursor = 3;
        assert_eq!(s.active_frame().cursor, 3);
    }

    #[test]
    fn crumbs_starts_as_the_one_root_frame() {
        let s = state_at("/a");
        assert_eq!(s.crumbs().len(), 1);
    }

    #[test]
    fn enter_on_a_directory_pushes_a_level_and_asks_for_a_listing() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![entry_named("/home", "projects", EntryKind::Dir)],
            ))),
        );

        let effects = apply(&mut s, Change::Command(Command::Enter));
        assert_eq!(s.active_frame().dir, PathBuf::from("/home/projects"));
        assert!(
            matches!(effects.jobs.as_slice(), [Job::List(p)] if p == Path::new("/home/projects")),
            "got {:?}",
            effects.jobs
        );
        assert!(matches!(effects.events.as_slice(), [Event::Stack]));
    }

    #[test]
    fn enter_on_a_cached_directory_asks_for_nothing() {
        // A `Job::List` only ever exists because some frame's own directory
        // needed one, so "already cached" is demonstrated by keeping a frame
        // alive with `JumpTo` -- which, unlike `Back`, does not throw the
        // frame's listing away -- rather than by handing `apply` a `Listed`
        // for a directory nothing has asked for yet.
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![entry_named("/home", "projects", EntryKind::Dir)],
            ))),
        );
        apply(&mut s, Change::Command(Command::Enter));
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/home/projects",
                vec![entry_named("/home/projects", "a.txt", EntryKind::File)],
            ))),
        );
        apply(&mut s, Change::Command(Command::JumpTo(0)));
        assert_eq!(s.active_frame().dir, PathBuf::from("/home"));

        let effects = apply(&mut s, Change::Command(Command::Enter));
        assert!(effects.jobs.is_empty(), "got {:?}", effects.jobs);
        assert_eq!(s.active_frame().dir, PathBuf::from("/home/projects"));
        assert_eq!(s.active_frame().rows.len(), 1);
    }

    #[test]
    fn enter_on_a_file_asks_to_open_it() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![entry_named("/home", "a.txt", EntryKind::File)],
            ))),
        );
        let effects = apply(&mut s, Change::Command(Command::Enter));
        assert!(matches!(effects.jobs.as_slice(), [Job::Open(p)] if p == Path::new("/home/a.txt")));
    }

    #[test]
    fn enter_with_nothing_under_the_cursor_does_nothing() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at("/home", Vec::new()))),
        );
        let effects = apply(&mut s, Change::Command(Command::Enter));
        assert!(effects.jobs.is_empty());
        assert!(effects.events.is_empty());
    }

    #[test]
    fn back_pops_and_discards_the_frame() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Command(Command::Push("/home/projects".into())),
        );
        assert_eq!(s.tabs.active().active_stack().len(), 2);

        apply(&mut s, Change::Command(Command::Back));
        assert_eq!(s.tabs.active().active_stack().len(), 1);
        assert_eq!(s.active_frame().dir, PathBuf::from("/home"));
    }

    #[test]
    fn back_walks_parent_paths_past_the_launch_directory_in_fold() {
        let mut s = state_at("/a/b");
        for expected in ["/a", "/"] {
            assert!(matches!(
                apply(&mut s, Change::Command(Command::Back))
                    .events
                    .as_slice(),
                [Event::Stack]
            ));
            assert_eq!(s.active_frame().dir, PathBuf::from(expected));
        }
        assert!(apply(&mut s, Change::Command(Command::Back))
            .events
            .is_empty());
        assert_eq!(s.active_frame().dir, PathBuf::from("/"));
    }

    #[test]
    fn back_after_unrelated_jump_uses_the_real_parent_in_commander() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Command(Command::RestoreCommander {
                dirs: [PathBuf::from("/home/a"), PathBuf::from("/network/share")],
                active: 0,
                enabled: true,
            }),
        );
        apply(
            &mut s,
            Change::Command(Command::Push("/media/usb/work".into())),
        );
        apply(&mut s, Change::Command(Command::Back));
        assert_eq!(s.active_frame().dir, PathBuf::from("/media/usb"));
        assert_eq!(s.tabs.active().stacks[1].len(), 1);
        assert_eq!(
            s.tabs.active().stacks[2].active().dir,
            PathBuf::from("/network/share")
        );
        apply(&mut s, Change::Command(Command::Back));
        assert_eq!(s.active_frame().dir, PathBuf::from("/media"));
    }

    #[test]
    fn forward_steps_into_a_child_a_jump_left_behind() {
        let mut s = state_at("/a");
        apply(&mut s, Change::Command(Command::Push("/a/b".into())));
        apply(&mut s, Change::Command(Command::JumpTo(0)));
        assert_eq!(s.active_frame().dir, PathBuf::from("/a"));

        let effects = apply(&mut s, Change::Command(Command::Forward));
        assert_eq!(s.active_frame().dir, PathBuf::from("/a/b"));
        assert!(matches!(effects.events.as_slice(), [Event::Stack]));

        let effects = apply(&mut s, Change::Command(Command::Forward));
        assert!(
            effects.events.is_empty(),
            "there is nothing past the last frame"
        );
    }

    #[test]
    fn jump_to_keeps_every_frame_the_way_back_does_not() {
        let mut s = state_at("/a");
        apply(&mut s, Change::Command(Command::Push("/a/b".into())));
        apply(&mut s, Change::Command(Command::Push("/a/b/c".into())));
        assert_eq!(s.tabs.active().active_stack().len(), 3);

        apply(&mut s, Change::Command(Command::JumpTo(0)));
        assert_eq!(s.active_frame().dir, PathBuf::from("/a"));
        assert_eq!(
            s.tabs.active().active_stack().len(),
            3,
            "jumping keeps every frame, unlike Back"
        );
    }

    #[test]
    fn listed_refinds_the_cursor_by_name_after_a_row_is_inserted_above_it() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![entry_named("/home", "b.txt", EntryKind::File)],
            ))),
        );
        assert_eq!(s.cursor_entry().unwrap().display, "b.txt");

        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![
                    entry_named("/home", "a.txt", EntryKind::File),
                    entry_named("/home", "b.txt", EntryKind::File),
                ],
            ))),
        );
        assert_eq!(
            s.cursor_entry().unwrap().display,
            "b.txt",
            "the cursor followed the name, not the index"
        );
        assert_eq!(s.active_frame().cursor, 1);
    }

    #[test]
    fn sethidden_reveals_a_dotfile() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![
                    entry_named("/home", ".gitignore", EntryKind::File),
                    entry_named("/home", "src", EntryKind::Dir),
                ],
            ))),
        );
        assert_eq!(s.active_frame().rows.len(), 1, "hidden by default");

        apply(&mut s, Change::Command(Command::SetHidden(true)));
        assert_eq!(s.active_frame().rows.len(), 2);
    }

    #[test]
    fn setfilter_narrows_the_active_frames_rows() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![
                    entry_named("/home", "apple.txt", EntryKind::File),
                    entry_named("/home", "banana.txt", EntryKind::File),
                ],
            ))),
        );

        apply(&mut s, Change::Command(Command::SetFilter("ban".into())));
        assert_eq!(s.active_frame().rows.len(), 1);
        assert_eq!(s.cursor_entry().unwrap().display, "banana.txt");
    }

    #[test]
    fn commander_panes_keep_distinct_sorts_and_filters_on_a_shared_listing() {
        let mut state = state_at("/home");
        apply(
            &mut state,
            Change::Command(Command::RestoreCommander {
                dirs: ["/home".into(), "/home".into()],
                active: 0,
                enabled: true,
            }),
        );
        apply(
            &mut state,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![
                    entry_named("/home", "apple.txt", EntryKind::File),
                    entry_named("/home", "banana.txt", EntryKind::File),
                    entry_named("/home", "cherry.txt", EntryKind::File),
                ],
            ))),
        );
        let visible = |state: &State, index: usize| -> Vec<String> {
            state
                .rows(state.tabs.active().stacks[index].active())
                .iter()
                .map(|entry| entry.display.clone())
                .collect()
        };
        let reversed = SortOrder {
            reverse: true,
            ..SortOrder::default()
        };
        apply(&mut state, Change::Command(Command::SetSort(reversed)));
        assert_eq!(
            visible(&state, 1),
            ["cherry.txt", "banana.txt", "apple.txt"]
        );
        assert_eq!(
            visible(&state, 2),
            ["apple.txt", "banana.txt", "cherry.txt"]
        );

        apply(
            &mut state,
            Change::Command(Command::SetFilter("app".into())),
        );
        assert_eq!(visible(&state, 1), ["apple.txt"]);
        assert_eq!(visible(&state, 2).len(), 3);
        apply(&mut state, Change::Command(Command::FocusPane(1)));
        assert_eq!(state.active_sort(), SortOrder::default());
        apply(
            &mut state,
            Change::Command(Command::SetFilter("ban".into())),
        );
        assert_eq!(visible(&state, 1), ["apple.txt"]);
        assert_eq!(visible(&state, 2), ["banana.txt"]);
        apply(&mut state, Change::Command(Command::ClearFilter));
        assert_eq!(visible(&state, 1), ["apple.txt"]);
        assert_eq!(
            visible(&state, 2),
            ["apple.txt", "banana.txt", "cherry.txt"]
        );

        apply(&mut state, Change::Command(Command::FocusPane(0)));
        apply(&mut state, Change::Command(Command::ClearFilter));
        apply(
            &mut state,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![
                    entry_named("/home", "apple.txt", EntryKind::File),
                    entry_named("/home", "banana.txt", EntryKind::File),
                    entry_named("/home", "cherry.txt", EntryKind::File),
                ],
            ))),
        );
        assert_eq!(
            visible(&state, 1),
            ["cherry.txt", "banana.txt", "apple.txt"]
        );
        assert_eq!(
            visible(&state, 2),
            ["apple.txt", "banana.txt", "cherry.txt"]
        );

        assert_eq!(state.active_sort(), reversed);
        assert_eq!(state.active_frame().filter, "");
        apply(&mut state, Change::Command(Command::ToggleView));
        assert_eq!(state.active_sort(), SortOrder::default());
        apply(&mut state, Change::Command(Command::ToggleView));
        assert_eq!(state.active_sort(), reversed);
        assert_eq!(
            visible(&state, 1),
            ["cherry.txt", "banana.txt", "apple.txt"]
        );
    }

    #[test]
    fn restored_commander_sort_orders_are_independent() {
        let mut state = state_at("/home");
        apply(
            &mut state,
            Change::Command(Command::RestoreCommander {
                dirs: ["/left".into(), "/right".into()],
                active: 1,
                enabled: true,
            }),
        );
        let fold = SortOrder::default();
        let left = SortOrder {
            reverse: true,
            ..fold
        };
        let right = SortOrder {
            key: super::super::sort::SortKey::Size,
            ..fold
        };
        apply(
            &mut state,
            Change::Command(Command::RestoreSorts {
                fold: Some(fold),
                panes: [Some(left), Some(right)],
            }),
        );
        assert_eq!(state.active_sort(), right);
        apply(&mut state, Change::Command(Command::FocusPane(0)));
        assert_eq!(state.active_sort(), left);
        apply(&mut state, Change::Command(Command::ToggleView));
        assert_eq!(state.active_sort(), fold);
    }

    #[test]
    fn active_copy_blocks_changes_to_its_source_tree_and_target() {
        let mut state = state_at("/home");
        let id = state.queue.enqueue(
            OpKind::Copy,
            vec!["/home/source".into()],
            Some("/backup".into()),
            ConflictPolicy::Ask,
        );
        state.queue.get_mut(id).unwrap().status = OpStatus::Running;

        let blocked = [
            Command::QueueDeleteSources(vec!["/home/source/child".into()]),
            Command::QueueRename {
                from: "/home".into(),
                to: "/renamed".into(),
            },
            Command::QueueOperation {
                kind: OpKind::Move,
                sources: vec!["/backup/source".into()],
                dest: Some("/elsewhere".into()),
            },
            Command::QueueDrop {
                kind: OpKind::Copy,
                sources: vec!["/other/source".into()],
                dest: "/backup".into(),
            },
            Command::Create {
                dir: "/backup".into(),
                kind: CreateKind::File,
                name: "source".into(),
            },
            Command::BeginImport {
                sources: vec!["/remote/source".into()],
                dest: "/backup".into(),
            },
            Command::BeginExport(vec!["/backup/source".into()]),
        ];
        for command in blocked {
            let effects = apply(&mut state, Change::Command(command));
            assert!(effects.jobs.is_empty());
            assert!(
                matches!(effects.events.as_slice(), [Event::Note(note)] if note.key == Some("copy-locked"))
            );
            assert_eq!(state.queue.len(), 1);
        }

        let unrelated = apply(
            &mut state,
            Change::Command(Command::QueueDeleteSources(vec!["/backup/other".into()])),
        );
        assert!(matches!(unrelated.events.first(), Some(Event::Queue(_))));
        assert_eq!(state.queue.len(), 2);

        state.queue.get_mut(id).unwrap().status = OpStatus::Done;
        let after = apply(
            &mut state,
            Change::Command(Command::QueueDeleteSources(vec!["/home/source".into()])),
        );
        assert!(matches!(after.events.first(), Some(Event::Queue(_))));
        assert_eq!(state.queue.len(), 3);
    }

    #[test]
    fn togglemark_marks_the_cursor_entry_and_moves_down() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![
                    entry_named("/home", "a.txt", EntryKind::File),
                    entry_named("/home", "b.txt", EntryKind::File),
                ],
            ))),
        );

        let effects = apply(&mut s, Change::Command(Command::ToggleMark));
        assert!(s.selection.is_marked(Path::new("/home/a.txt")));
        assert_eq!(s.active_frame().cursor, 1);
        assert!(effects.events.iter().any(|e| matches!(e, Event::Selection)));
        assert!(effects.events.iter().any(|e| matches!(e, Event::Stack)));
    }

    #[test]
    fn yank_with_no_entry_notes_and_paste_with_empty_register_queues_nothing() {
        let mut s = state_at("/home");
        let effects = apply(&mut s, Change::Command(Command::Yank));
        assert!(s.queue.is_empty());
        assert!(matches!(
            effects.events.as_slice(),
            [Event::Note(n)] if n.key == Some("nothing-to-yank")
        ));
        let effects = apply(&mut s, Change::Command(Command::PasteHere));
        assert!(s.queue.is_empty());
        assert!(matches!(
            effects.events.as_slice(),
            [Event::Note(n)] if n.key == Some("nothing-yanked")
        ));
    }

    #[test]
    fn yank_with_marks_pastes_a_copy_to_the_active_directory() {
        let mut s = state_at("/dest");
        s.selection
            .toggle(&entry_named("/src", "a.txt", EntryKind::File));

        apply(&mut s, Change::Command(Command::Yank));
        assert!(s.queue.is_empty());
        s.selection.forget();
        let effects = apply(&mut s, Change::Command(Command::PasteHere));
        assert_eq!(s.queue.len(), 1);
        let op = s.queue.iter().next().unwrap();
        assert_eq!(op.dest.as_deref(), Some(Path::new("/dest")));
        assert_eq!(op.sources, vec![PathBuf::from("/src/a.txt")]);
        assert!(effects.events.iter().any(|e| matches!(e, Event::Queue(_))));
        assert!(matches!(effects.jobs.as_slice(), [Job::Plan { .. }]));
    }

    #[test]
    fn yank_without_marks_uses_cursor_and_second_yank_replaces_register() {
        let mut s = state_at("/src");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/src",
                vec![
                    entry_named("/src", "a", EntryKind::Dir),
                    entry_named("/src", "b", EntryKind::File),
                ],
            ))),
        );
        apply(&mut s, Change::Command(Command::Yank));
        apply(&mut s, Change::Command(Command::Yank)); // yy
        assert_eq!(s.yanked, vec![PathBuf::from("/src/a")]);
        apply(&mut s, Change::Command(Command::CursorTo(1)));
        apply(&mut s, Change::Command(Command::Yank));
        assert_eq!(s.yanked, vec![PathBuf::from("/src/b")]);
    }

    #[test]
    fn queuedelete_picks_trash_or_permanent_from_the_mode_and_availability() {
        let mut s = state_at("/home");
        s.trash = TrashMode::Auto;
        s.trash_available = true;
        s.selection
            .toggle(&entry_named("/home", "a.txt", EntryKind::File));
        apply(&mut s, Change::Command(Command::QueueDelete));
        assert!(matches!(
            s.queue.iter().next().unwrap().kind,
            OpKind::Delete(DeleteHow::Trash)
        ));

        let mut s2 = state_at("/home");
        s2.trash = TrashMode::Auto;
        s2.trash_available = false;
        s2.selection
            .toggle(&entry_named("/home", "a.txt", EntryKind::File));
        apply(&mut s2, Change::Command(Command::QueueDelete));
        assert!(matches!(
            s2.queue.iter().next().unwrap().kind,
            OpKind::Delete(DeleteHow::Permanent)
        ));
    }

    #[test]
    fn paste_plans_the_first_runnable_op_immediately() {
        let mut s = state_at("/home");
        s.selection
            .toggle(&entry_named("/home", "a.txt", EntryKind::File));
        apply(&mut s, Change::Command(Command::Yank));
        let effects = apply(&mut s, Change::Command(Command::PasteHere));
        assert!(matches!(effects.jobs.as_slice(), [Job::Plan { .. }]));
        assert_eq!(s.queue.iter().next().unwrap().status, OpStatus::Planning);
    }

    /// Sets up one queued copy, runs it, and answers its plan with one
    /// conflict -- the shared starting point for the `NeedsPolicy` and
    /// `SetPolicy` tests below.
    fn queued_copy_with_a_conflict(s: &mut State) -> OpId {
        s.selection
            .toggle(&entry_named("/home", "a.txt", EntryKind::File));
        apply(s, Change::Command(Command::Yank));
        apply(s, Change::Command(Command::PasteHere));
        let id = s.queue.iter().next().unwrap().id;
        apply(s, Change::Command(Command::Run));

        let plan = super::super::ops::Plan {
            sources: vec!["/home/a.txt".into()],
            dest: "/home".into(),
            total_bytes: 10,
            conflicts: vec![super::super::ops::Conflict {
                source: "/home/a.txt".into(),
                dest: "/home/a.txt".into(),
                both_dirs: false,
            }],
            ..Default::default()
        };
        apply(
            s,
            Change::Done(Done::Planned {
                op: id,
                result: Ok(plan),
            }),
        );
        id
    }

    #[test]
    fn planned_with_conflicts_under_ask_stops_at_needspolicy_and_emits_conflicts() {
        let mut s = state_at("/home");
        let id = queued_copy_with_a_conflict(&mut s);
        let op = s.queue.iter().next().unwrap();
        assert_eq!(op.status, OpStatus::NeedsPolicy);
        assert_eq!(op.id, id);
    }

    #[test]
    fn setpolicy_resumes_the_started_operation() {
        let mut s = state_at("/home");
        let id = queued_copy_with_a_conflict(&mut s);

        let effects = apply(
            &mut s,
            Change::Command(Command::SetPolicy(id, ConflictPolicy::Overwrite)),
        );
        assert!(matches!(effects.jobs.as_slice(), [Job::Run { .. }]));
        assert_eq!(s.queue.iter().next().unwrap().status, OpStatus::Running);
    }

    #[test]
    fn sudo_retry_only_enqueues_failed_permission_delete_sources() {
        let mut s = state_at("/home");
        let original = s.queue.enqueue(
            OpKind::Delete(DeleteHow::Permanent),
            vec!["/home/denied".into(), "/home/other".into()],
            None,
            ConflictPolicy::Ask,
        );
        let op = s.queue.get_mut(original).unwrap();
        op.status = OpStatus::Failed;
        op.failed = vec![("/home/denied".into(), "Permission denied".into())];
        let effects = apply(
            &mut s,
            Change::Command(Command::QueueElevatedDelete(original)),
        );
        assert!(
            matches!(effects.jobs.as_slice(), [Job::RunElevated { sources, .. }] if sources == &vec![PathBuf::from("/home/denied")])
        );
        let retry = s.queue.iter().last().unwrap();
        assert!(retry.elevated);
        assert_eq!(retry.status, OpStatus::Running);
    }

    #[test]
    fn named_conflict_choice_reaches_the_worker() {
        let mut s = state_at("/home");
        let id = queued_copy_with_a_conflict(&mut s);
        let targets = vec![(
            PathBuf::from("/home/a.txt"),
            PathBuf::from("/home/my copy.txt"),
        )];
        let effects = apply(
            &mut s,
            Change::Command(Command::SetConflictNames(id, targets.clone())),
        );
        assert!(
            matches!(effects.jobs.as_slice(), [Job::Run { rename_targets, policy: ConflictPolicy::RenameNew, .. }] if *rename_targets == targets)
        );
        assert_eq!(s.queue.get_mut(id).unwrap().status, OpStatus::Running);
    }

    #[test]
    fn dropped_transfer_waits_for_the_earlier_operation() {
        let mut s = state_at("/dest");
        let first = apply(
            &mut s,
            Change::Command(Command::QueueOperation {
                kind: OpKind::Copy,
                sources: vec!["/old".into()],
                dest: Some("/dest".into()),
            }),
        );
        let old = s.queue.iter().next().unwrap().id;
        assert!(matches!(first.jobs.as_slice(), [Job::Plan { op, .. }] if *op == old));
        let started = apply(
            &mut s,
            Change::Command(Command::QueueDrop {
                kind: OpKind::Copy,
                sources: vec!["/new".into()],
                dest: "/dest".into(),
            }),
        );
        let drop_id = s.queue.iter().nth(1).unwrap().id;
        assert!(started.jobs.is_empty());
        assert_eq!(s.queue.iter().nth(1).unwrap().status, OpStatus::Queued);
        apply(
            &mut s,
            Change::Done(Done::Planned {
                op: old,
                result: Ok(super::super::ops::Plan::default()),
            }),
        );
        let finished = apply(
            &mut s,
            Change::Done(Done::Finished {
                op: old,
                outcome: super::super::ops::Outcome::default(),
            }),
        );
        assert!(finished
            .jobs
            .iter()
            .any(|job| matches!(job, Job::Plan { op, .. } if *op == drop_id)));
        assert_eq!(s.queue.iter().next().unwrap().status, OpStatus::Done);
    }

    #[test]
    fn stop_pauses_later_and_new_work_until_resume() {
        let mut s = state_at("/dest");
        let first = apply(
            &mut s,
            Change::Command(Command::QueueOperation {
                kind: OpKind::Copy,
                sources: vec!["/a".into()],
                dest: Some("/dest".into()),
            }),
        );
        let active = s.queue.iter().next().unwrap().id;
        assert!(matches!(first.jobs.as_slice(), [Job::Plan { op, .. }] if *op == active));
        apply(
            &mut s,
            Change::Command(Command::QueueOperation {
                kind: OpKind::Copy,
                sources: vec!["/b".into()],
                dest: Some("/dest".into()),
            }),
        );
        apply(&mut s, Change::Command(Command::StopActive(active)));
        assert!(s.queue.is_paused());
        let planned = apply(
            &mut s,
            Change::Done(Done::Planned {
                op: active,
                result: Ok(super::super::ops::Plan::default()),
            }),
        );
        assert!(planned.jobs.is_empty());
        assert_eq!(s.queue.iter().next().unwrap().status, OpStatus::Cancelled);
        let dropped = apply(
            &mut s,
            Change::Command(Command::QueueDrop {
                kind: OpKind::Copy,
                sources: vec!["/c".into()],
                dest: "/dest".into(),
            }),
        );
        assert!(dropped.jobs.is_empty());
        assert_eq!(
            s.queue
                .iter()
                .filter(|op| op.status == OpStatus::Queued)
                .count(),
            2
        );
        let resumed = apply(&mut s, Change::Command(Command::Run));
        assert!(!s.queue.is_paused());
        let second = s.queue.iter().nth(1).unwrap().id;
        assert!(matches!(resumed.jobs.as_slice(), [Job::Plan { op, .. }] if *op == second));
    }

    #[test]
    fn finished_forgets_moved_sources_relists_the_dest_and_starts_the_next_op() {
        let mut s = state_at("/dest");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at("/dest", Vec::new()))),
        );

        s.selection
            .toggle(&entry_named("/src", "a.txt", EntryKind::File));
        apply(&mut s, Change::Command(Command::QueueMoveHere));
        let id1 = s.queue.iter().next().unwrap().id;

        s.selection
            .toggle(&entry_named("/src", "b.txt", EntryKind::File));
        apply(&mut s, Change::Command(Command::QueueMoveHere));
        let id2 = s.queue.iter().nth(1).unwrap().id;

        apply(&mut s, Change::Command(Command::Run));
        apply(
            &mut s,
            Change::Done(Done::Planned {
                op: id1,
                result: Ok(super::super::ops::Plan {
                    sources: vec!["/src/a.txt".into()],
                    dest: "/dest".into(),
                    total_bytes: 0,
                    ..Default::default()
                }),
            }),
        );

        let outcome = super::super::ops::Outcome {
            done: 1,
            skipped: 0,
            failed: Vec::new(),
            cancelled: false,
        };
        let effects = apply(&mut s, Change::Done(Done::Finished { op: id1, outcome }));

        assert!(
            !s.selection.is_marked(Path::new("/src/a.txt")),
            "the moved source was forgotten"
        );
        assert!(
            effects
                .jobs
                .iter()
                .any(|j| matches!(j, Job::List(p) if p == Path::new("/dest"))),
            "the destination frame was re-listed: {:?}",
            effects.jobs
        );
        assert!(
            effects
                .jobs
                .iter()
                .any(|j| matches!(j, Job::Plan { op, .. } if *op == id2)),
            "the next op started: {:?}",
            effects.jobs
        );
    }

    #[test]
    fn failed_delete_reports_the_reason() {
        let mut s = state_at("/home");
        let source = PathBuf::from("/home/uploaded.txt");
        s.selection
            .toggle(&entry_named("/home", "uploaded.txt", EntryKind::File));
        apply(
            &mut s,
            Change::Command(Command::QueueDeleteSources(vec![source.clone()])),
        );
        let id = s.queue.iter().next().unwrap().id;
        let effects = apply(
            &mut s,
            Change::Done(Done::Finished {
                op: id,
                outcome: super::super::ops::Outcome {
                    failed: vec![(
                        "/volume/.Trash-1000".into(),
                        "trash directory is unavailable".into(),
                    )],
                    ..Default::default()
                },
            }),
        );
        assert!(effects.events.iter().any(|event| matches!(
            event,
            Event::Note(note) if note.text.contains("trash directory is unavailable")
        )));
        assert_eq!(
            s.queue.get_mut(id).unwrap().trash_failure.as_deref(),
            Some("trash directory is unavailable")
        );
        assert_eq!(s.queue.get_mut(id).unwrap().failed.len(), 1);
        assert!(s.selection.is_marked(&source));

        apply(
            &mut s,
            Change::Command(Command::QueuePermanentDeleteSources(vec![
                "/home/uploaded.txt".into(),
            ])),
        );
        assert_eq!(
            s.queue.iter().last().unwrap().kind,
            OpKind::Delete(DeleteHow::Permanent)
        );

        let mut s = state_at("/home");
        s.trash = TrashMode::Always;
        apply(
            &mut s,
            Change::Command(Command::QueueDeleteSources(vec![
                "/home/uploaded.txt".into()
            ])),
        );
        let id = s.queue.iter().next().unwrap().id;
        apply(
            &mut s,
            Change::Done(Done::Finished {
                op: id,
                outcome: super::super::ops::Outcome {
                    failed: vec![("/home/uploaded.txt".into(), "still unavailable".into())],
                    ..Default::default()
                },
            }),
        );
        assert!(s.queue.get_mut(id).unwrap().trash_failure.is_none());
    }

    #[test]
    fn empty_drive_trash_queues_only_this_users_trash() {
        let mut state = state_at("/home");
        let mount = PathBuf::from("/run/media/me/USB");
        state.places.locations.push(super::super::places::Location {
            name: "USB".into(),
            path: mount.clone(),
            kind: super::super::places::LocationKind::Device,
            unmount_source: Some("/dev/sdb1".into()),
            info: Some(super::super::places::LocationInfo::default()),
        });
        apply(
            &mut state,
            Change::Command(Command::EmptyDriveTrash(mount.clone())),
        );
        let op = state.queue.iter().next().expect("queued drive trash");
        assert_eq!(op.title(), "EMPTY TRASH: USB");
        assert_eq!(op.kind, OpKind::Delete(DeleteHow::Permanent));
        assert_eq!(op.sources, places::drive_trash_paths(&mount));
        assert!(!op.sources.iter().any(|path| path == &mount.join(".Trash")));
    }

    #[test]
    fn a_marked_directory_size_does_not_replace_its_tree() {
        let mut s = state_at("/home");
        let path = PathBuf::from("/home/folder");
        apply(&mut s, Change::Command(Command::Preview(path.clone())));
        apply(
            &mut s,
            Change::Done(Done::Previewed {
                tab: TabId(0),
                path: path.clone(),
                generation: 1,
                preview: Preview::Dir(super::super::preview::directory::Tree {
                    entries: vec![],
                    summary: DirSummary::default(),
                    notice: Some("Tree notice".into()),
                }),
            }),
        );
        apply(
            &mut s,
            Change::Done(Done::Summarized {
                dir: path,
                summary: DirSummary {
                    files: 100,
                    ..DirSummary::default()
                },
            }),
        );
        assert!(matches!(s.preview.as_ref().unwrap().1.as_ref(),
            Preview::Dir(tree) if tree.summary.files == 0 && tree.notice.as_deref() == Some("Tree notice")));
    }

    #[test]
    fn extension_actions_survive_coalescing_and_acknowledge_only_consumed_sequences() {
        use super::super::preview::{
            extensions::{Action, Info},
            model::Document,
        };
        use starfold_preview_protocol::MediaAction;
        let mut state = state_at("/home");
        let document = |sequence| {
            let mut d = Document::new("Test media");
            d.extension = Some(Box::new(Info {
                provider: "test".into(),
                revision: "1".into(),
                session: 9,
                sequence,
                keys: vec![],
                interactive: false,
                modified: false,
                actions: vec![Action {
                    sequence,
                    action: MediaAction::PlayPause,
                }],
            }));
            Preview::Document(d)
        };
        for sequence in [2, 3] {
            apply(
                &mut state,
                Change::Done(Done::Previewed {
                    tab: TabId(0),
                    path: "/home/movie".into(),
                    generation: 0,
                    preview: document(sequence),
                }),
            );
        }
        assert_eq!(
            state
                .preview
                .as_ref()
                .unwrap()
                .1
                .extension()
                .unwrap()
                .actions
                .len(),
            2
        );
        apply(
            &mut state,
            Change::Command(Command::PreviewAck {
                session: 8,
                sequence: 3,
            }),
        );
        assert_eq!(
            state
                .preview
                .as_ref()
                .unwrap()
                .1
                .extension()
                .unwrap()
                .actions
                .len(),
            2
        );
        apply(
            &mut state,
            Change::Command(Command::PreviewAck {
                session: 9,
                sequence: 2,
            }),
        );
        let pending = &state
            .preview
            .as_ref()
            .unwrap()
            .1
            .extension()
            .unwrap()
            .actions;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].sequence, 3);
    }

    #[test]
    fn a_stale_previewed_generation_is_ignored() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Command(Command::Preview("/home/a.txt".into())),
        );
        apply(
            &mut s,
            Change::Command(Command::Preview("/home/b.txt".into())),
        );

        let effects = apply(
            &mut s,
            Change::Done(Done::Previewed {
                tab: TabId(0),
                path: "/home/a.txt".into(),
                generation: 1,
                preview: Preview::Empty,
            }),
        );
        assert_eq!(s.preview.as_ref().unwrap().0, Path::new("/home/b.txt"));
        assert!(
            matches!(s.preview.as_ref().unwrap().1.as_ref(), Preview::Document(d) if d.kind == "Loading preview…")
        );
        assert!(effects.events.is_empty());
    }

    #[test]
    fn version_bumps_exactly_when_something_changed() {
        let mut s = state_at("/home");
        apply(
            &mut s,
            Change::Done(Done::Listed(listing_at(
                "/home",
                vec![
                    entry_named("/home", "a.txt", EntryKind::File),
                    entry_named("/home", "b.txt", EntryKind::File),
                ],
            ))),
        );
        let v0 = s.version;

        apply(&mut s, Change::Command(Command::CursorBy(-1)));
        assert_eq!(
            s.version, v0,
            "a no-op cursor move must not bump the version"
        );

        apply(&mut s, Change::Command(Command::CursorBy(1)));
        assert_eq!(
            s.version,
            v0 + 1,
            "an actual move must bump it exactly once"
        );
    }

    #[test]
    fn apply_does_no_io() {
        // NO-IO-HERE
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fold/state.rs"); // NO-IO-HERE
        let text = std::fs::read_to_string(&path).unwrap(); // NO-IO-HERE
        for (n, line) in text.lines().enumerate() {
            if line.contains("NO-IO-HERE") {
                continue;
            }
            assert!(
                !line.contains("std::fs::"), // NO-IO-HERE
                "state.rs:{}: {} -- apply must never touch a filesystem directly",
                n + 1,
                line
            );
        }
    }

    proptest! {
        #[test]
        fn applying_random_commands_never_panics_and_keeps_the_cursor_in_bounds(
            ops in proptest::collection::vec(0u8..12, 0..80)
        ) {
            let mut s = state_at("/home");
            apply(
                &mut s,
                Change::Done(Done::Listed(listing_at(
                    "/home",
                    vec![
                        entry_named("/home", "a.txt", EntryKind::File),
                        entry_named("/home", "b.txt", EntryKind::File),
                        entry_named("/home", ".hidden", EntryKind::File),
                        entry_named("/home", "sub", EntryKind::Dir),
                    ],
                ))),
            );
            apply(
                &mut s,
                Change::Done(Done::Listed(listing_at(
                    "/home/sub",
                    vec![entry_named("/home/sub", "c.txt", EntryKind::File)],
                ))),
            );

            for op in ops {
                let command = match op {
                    0 => Command::Enter,
                    1 => Command::Back,
                    2 => Command::JumpTo(0),
                    3 => Command::CursorTo(3),
                    4 => Command::CursorBy(1),
                    5 => Command::CursorBy(-1),
                    6 => Command::SetFilter("a".into()),
                    7 => Command::ClearFilter,
                    8 => Command::SetHidden(true),
                    9 => Command::SetHidden(false),
                    10 => Command::ToggleMark,
                    _ => Command::MarkAll,
                };
                apply(&mut s, Change::Command(command));
                let frame = s.active_frame();
                prop_assert!(frame.cursor < frame.rows.len() || frame.rows.is_empty());
            }
        }
    }
}
