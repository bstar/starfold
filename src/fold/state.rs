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

use super::entry::{Entry, EntryKind};
use super::filter;
use super::handle::{Command, Event, Note};
use super::listing::Listing;
use super::ops::{ConflictPolicy, DeleteHow, OpId, OpKind, OpStatus, Queue};
use super::places::{self, Bookmark};
use super::preview::Preview;
use super::selection::Selection;
use super::sort::{self, SortOrder};
use super::stack::{Frame, ParentMove, Stack};
use super::summary::DirSummary;
use super::tab::Tabs;
use super::worker::{Done, Job};
use super::{FoldConfig, TrashMode};

/// The whole of what the program knows, right now.
pub struct State {
    pub places: places::PlacesState,
    pub commander: bool,
    pub commander_pane: usize,
    parked_selections: [Selection; 3],
    pub tabs: Tabs,
    /// Every directory read so far, keyed by path. A frame's own listing is
    /// looked up here rather than carried on the frame, so two frames open on
    /// the same directory -- a fold and its own child, briefly -- share one
    /// read rather than paying for it twice.
    pub listings: HashMap<PathBuf, Arc<Listing>>,
    pub selection: Selection,
    pub queue: Queue,
    /// What the PREVIEW panel shows, and which path it was built for -- the
    /// panel's title says the name, and a preview that arrived for a file the
    /// cursor has since left is told apart from the current one by the path
    /// rather than trusted.
    pub preview: Option<(PathBuf, Arc<Preview>)>,
    pub sort: SortOrder,
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
    /// Set until the start directory's first listing lands.
    pub loading: bool,
}

impl State {
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
            tabs: Tabs::single(Stack::new(start)),
            listings: HashMap::new(),
            selection: Selection::default(),
            queue: Queue::new(),
            preview: None,
            sort: cfg.list.sort,
            show_hidden: cfg.list.show_hidden,
            trash_available,
            trash: cfg.trash,
            conflicts: cfg.conflicts,
            preserve_times: cfg.preserve_times,
            home,
            version: 0,
            preview_generation: 0,
            loading: true,
        }
    }

    pub fn active_frame(&self) -> &Frame {
        self.tabs.active().active_stack().active()
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
/// `QueueCopyHere` command plans a copy, and the `Done::Planned` that
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
    let old_stack = state.tabs.active().active_stack;
    let old_dir = state.active_frame().dir.clone();
    let effects = apply_inner(state, change);
    if state.commander
        && old_stack == state.tabs.active().active_stack
        && old_dir != state.active_frame().dir
    {
        state.selection.forget();
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
        Command::SaveBookmark { name, path } => cmd_save_bookmark(state, name, path),
        Command::RenameBookmark { path, name } => cmd_rename_bookmark(state, path, name),
        Command::RemoveBookmark(path) => cmd_remove_bookmark(state, path),
        Command::Notify(message) => Effects {
            events: vec![Event::Note(Note::warning("startup", message))],
            ..Effects::default()
        },
        Command::ToggleView => {
            if state.tabs.active().stacks.len() == 1 {
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
            )
        }
        Command::FocusPane(pane) => {
            if !state.commander || pane > 1 {
                return Effects::default();
            }
            state.commander_pane = pane;
            switch_stack(state, pane + 1)
        }
        Command::RestoreCommander {
            dirs,
            active,
            enabled,
        } => {
            state.tabs.active_mut().stacks.truncate(1);
            state
                .tabs
                .active_mut()
                .stacks
                .extend(dirs.iter().cloned().map(Stack::new));
            state.commander_pane = active.min(1);
            state.commander = enabled;
            let mut effects =
                switch_stack(state, if enabled { state.commander_pane + 1 } else { 0 });
            effects.jobs.extend(dirs.into_iter().map(Job::List));
            effects
        }
        Command::Enter => cmd_enter(state),
        Command::Back => cmd_back(state),
        Command::JumpTo(index) => cmd_jump_to(state, index),
        Command::Forward => cmd_forward(state),
        Command::Push(dir) => push_dir(state, dir),
        Command::Reload => cmd_reload(state),
        Command::CursorTo(row) => set_cursor(state, row),
        Command::CursorBy(delta) => cmd_cursor_by(state, delta),
        Command::SetFilter(query) => cmd_set_filter(state, query),
        Command::ClearFilter => cmd_clear_filter(state),
        Command::SetSort(order) => cmd_set_sort(state, order),
        Command::SetHidden(hidden) => cmd_set_hidden(state, hidden),
        Command::ToggleMark => cmd_toggle_mark(state),
        Command::ToggleMarkPath(path) => {
            let entry = path
                .parent()
                .and_then(|dir| state.listing_of(dir))
                .and_then(|l| l.entries.iter().find(|e| e.path == path))
                .cloned();
            let Some(entry) = entry else {
                return Effects::default();
            };
            state.selection.toggle(&entry);
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
        Command::QueueOperation {
            kind,
            sources,
            dest,
        } => {
            if sources.is_empty() {
                return Effects::default();
            }
            let id = state.queue.enqueue(kind, sources, dest, state.conflicts);
            Effects {
                jobs: vec![],
                events: vec![Event::Queue(id)],
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
                    path,
                    generation,
                    page,
                }],
                events: vec![Event::Preview],
            }
        }
        Command::QueueCopyHere => cmd_queue_copy_or_move(state, OpKind::Copy),
        Command::QueueMoveHere => cmd_queue_copy_or_move(state, OpKind::Move),
        Command::QueueDelete => cmd_queue_delete(state),
        Command::QueueDeleteSources(sources) => queue_delete_sources(state, sources),
        Command::QueueRename { from, to } => cmd_queue_rename(state, from, to),
        Command::RemoveOp(id) => cmd_remove_op(state, id),
        Command::ClearQueue => cmd_clear_queue(state),
        Command::Run => run_next(state),
        Command::SetPolicy(id, policy) => cmd_set_policy(state, id, policy),
        Command::Cancel(id) => cmd_cancel(state, id),
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
            state.preview_generation += 1;
            for op in state.queue.iter() {
                op.progress.cancel();
            }
            Effects::default()
        }
    }
}

fn switch_stack(state: &mut State, index: usize) -> Effects {
    let old = state.tabs.active().active_stack;
    std::mem::swap(&mut state.selection, &mut state.parked_selections[old]);
    state.tabs.active_mut().active_stack = index;
    std::mem::swap(&mut state.selection, &mut state.parked_selections[index]);
    state.preview_generation += 1;
    state.preview = None;
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
        Done::Listed(listing) => done_listed(state, listing),
        Done::Summarized { dir, summary } => done_summarized(state, dir, summary),
        Done::Previewed {
            path,
            generation,
            preview,
        } => done_previewed(state, path, generation, preview),
        Done::Planned { op, result } => done_planned(state, op, result),
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
    let sort = state.sort;
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
    let sort = state.sort;
    let hidden = state.show_hidden;
    if let Some(listing) = state.listings.get(&dir).cloned() {
        let frame = state.active_frame_mut();
        rebuild_rows(frame, &listing, sort, hidden, landing);
    }
}

/// Rebuild every frame's rows, in every stack of every tab, against whatever
/// listing each one's own directory has cached -- `SetSort` and `SetHidden`
/// change how *every* level reads, not just the active one.
///
/// The listings map is cloned up front (an `Arc` clone per entry, not a deep
/// copy of any `Listing`) rather than looked up per frame while `state.tabs`
/// is borrowed mutably: `HashMap<PathBuf, Arc<Listing>>` and `Tabs` are
/// different fields of `State`, but threading a live borrow of one through
/// three nested loops mutating the other is more ceremony than a cheap clone
/// is worth.
fn rebuild_all_frames(state: &mut State) {
    let sort = state.sort;
    let hidden = state.show_hidden;
    let listings = state.listings.clone();
    for tab in &mut state.tabs.tabs {
        for stack in &mut tab.stacks {
            for frame in stack.frames_mut() {
                if let Some(listing) = listings.get(&frame.dir) {
                    rebuild_rows(frame, listing, sort, hidden, Landing::Refind);
                }
            }
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
    state.listings.retain(|dir, _| referenced.contains(dir));
}

/// Move the active frame's cursor down one row, clamped to the last one --
/// what `space` does after marking, so the next entry lands under the cursor
/// without a separate `j`.
fn bump_cursor_down(state: &mut State) {
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
    state.tabs.active_mut().active_stack_mut().push(dir);
    let jobs = populate_active_frame(state);
    Effects {
        jobs,
        events: vec![Event::Stack],
    }
}

fn cmd_back(state: &mut State) -> Effects {
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
    let current = state.active_frame().cursor as i64;
    let target = (current + delta as i64).max(0) as usize;
    set_cursor(state, target)
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
    state.sort = order;
    rebuild_all_frames(state);
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Stack],
    }
}

fn cmd_set_hidden(state: &mut State, hidden: bool) -> Effects {
    state.show_hidden = hidden;
    rebuild_all_frames(state);
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
    state.selection.forget();
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Selection],
    }
}

/// `QueueCopyHere`/`QueueMoveHere`: everything marked, into the active
/// frame's directory. A queue entry is never built with an empty source
/// list -- an empty selection is a note, not a no-op `Op` sitting in the
/// panel.
fn cmd_queue_copy_or_move(state: &mut State, kind: OpKind) -> Effects {
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
    let id = state
        .queue
        .enqueue(kind, sources, Some(dest), state.conflicts);
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Queue(id)],
    }
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
    let id = state
        .queue
        .enqueue(OpKind::Delete(how), sources, None, state.conflicts);
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Queue(id)],
    }
}

fn cmd_queue_rename(state: &mut State, from: PathBuf, to: PathBuf) -> Effects {
    let id = state
        .queue
        .enqueue(OpKind::Rename, vec![from], Some(to), state.conflicts);
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Queue(id)],
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
        .filter(|op| op.is_pending())
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
        return Effects {
            jobs: Vec::new(),
            events: vec![Event::Note(Note::info("an operation is already running"))],
        };
    }
    let Some(id) = state.queue.first_runnable() else {
        return Effects::default();
    };
    let Some(op) = state.queue.get_mut(id) else {
        return Effects::default();
    };

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
                progress: Arc::clone(&op.progress),
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
    if op.status == OpStatus::NeedsPolicy {
        op.status = OpStatus::Queued;
    }
    Effects {
        jobs: Vec::new(),
        events: vec![Event::Queue(id)],
    }
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
            path,
            generation: state.preview_generation,
        }],
        events: vec![Event::Preview],
    }
}

// ---------------------------------------------------------------------
// Worker results.
// ---------------------------------------------------------------------

fn done_listed(state: &mut State, listing: Listing) -> Effects {
    let dir = listing.dir.clone();
    let error = listing.error.clone();
    let listing = Arc::new(listing);
    state.listings.insert(dir.clone(), Arc::clone(&listing));

    let sort = state.sort;
    let hidden = state.show_hidden;
    for tab in &mut state.tabs.tabs {
        for stack in &mut tab.stacks {
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
            if !state.commander && err.to_lowercase().contains("no such") {
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
    if let Preview::Document(new) = &mut preview {
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

fn done_planned(
    state: &mut State,
    op_id: OpId,
    result: Result<super::ops::Plan, String>,
) -> Effects {
    match result {
        Err(message) => {
            if let Some(op) = state.queue.get_mut(op_id) {
                op.status = OpStatus::Failed;
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
                        progress: Arc::clone(&op.progress),
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
        op.status = if outcome.cancelled {
            OpStatus::Cancelled
        } else if !outcome.failed.is_empty() {
            OpStatus::Failed
        } else {
            OpStatus::Done
        };

        if op.status == OpStatus::Failed {
            let total = outcome.done + outcome.skipped + outcome.failed.len();
            events.push(Event::Note(Note::error(
                "op-failed",
                format!("{} of {} failed", outcome.failed.len(), total),
            )));
        }

        // Every kind consumes its marks. A move, a delete and a rename have
        // taken the paths away; a copy has not, but the marks were the thing
        // being copied, and a `d` pressed afterwards that deleted the
        // originals because they were still marked is a mistake nobody
        // should be able to make.
        if !outcome.cancelled {
            for src in &op.sources {
                if !outcome.failed.iter().any(|(p, _)| p == src) {
                    moved_or_deleted.push(src.clone());
                }
            }
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
        state.selection.forget_paths(&moved_or_deleted);
        for selection in &mut state.parked_selections {
            selection.forget_paths(&moved_or_deleted);
        }
        events.push(Event::Selection);
    }

    let frame_dirs = all_frame_dirs(state);
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

    fn entry_named(dir: &str, name: &str, kind: EntryKind) -> Entry {
        Entry {
            path: PathBuf::from(dir).join(name),
            display: name.to_string(),
            kind,
            link_kind: None,
            len: 0,
            modified: None,
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
    fn queuecopyhere_with_nothing_marked_notes_and_queues_nothing() {
        let mut s = state_at("/home");
        let effects = apply(&mut s, Change::Command(Command::QueueCopyHere));
        assert!(s.queue.is_empty());
        assert!(matches!(
            effects.events.as_slice(),
            [Event::Note(n)] if n.key == Some("nothing-marked")
        ));
    }

    #[test]
    fn queuecopyhere_with_marks_enqueues_a_copy_to_the_active_directory() {
        let mut s = state_at("/dest");
        s.selection
            .toggle(&entry_named("/src", "a.txt", EntryKind::File));

        let effects = apply(&mut s, Change::Command(Command::QueueCopyHere));
        assert_eq!(s.queue.len(), 1);
        let op = s.queue.iter().next().unwrap();
        assert_eq!(op.dest.as_deref(), Some(Path::new("/dest")));
        assert_eq!(op.sources, vec![PathBuf::from("/src/a.txt")]);
        assert!(matches!(effects.events.as_slice(), [Event::Queue(_)]));
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
    fn run_plans_the_first_runnable_op() {
        let mut s = state_at("/home");
        s.selection
            .toggle(&entry_named("/home", "a.txt", EntryKind::File));
        apply(&mut s, Change::Command(Command::QueueCopyHere));

        let effects = apply(&mut s, Change::Command(Command::Run));
        assert!(matches!(effects.jobs.as_slice(), [Job::Plan { .. }]));
        assert_eq!(s.queue.iter().next().unwrap().status, OpStatus::Planning);
    }

    /// Sets up one queued copy, runs it, and answers its plan with one
    /// conflict -- the shared starting point for the `NeedsPolicy` and
    /// `SetPolicy` tests below.
    fn queued_copy_with_a_conflict(s: &mut State) -> OpId {
        s.selection
            .toggle(&entry_named("/home", "a.txt", EntryKind::File));
        apply(s, Change::Command(Command::QueueCopyHere));
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
    fn setpolicy_then_run_proceeds_to_job_run() {
        let mut s = state_at("/home");
        let id = queued_copy_with_a_conflict(&mut s);

        apply(
            &mut s,
            Change::Command(Command::SetPolicy(id, ConflictPolicy::Overwrite)),
        );
        assert_eq!(s.queue.iter().next().unwrap().status, OpStatus::Queued);

        let effects = apply(&mut s, Change::Command(Command::Run));
        assert!(matches!(effects.jobs.as_slice(), [Job::Run { .. }]));
        assert_eq!(s.queue.iter().next().unwrap().status, OpStatus::Running);
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
    fn a_marked_directory_size_does_not_replace_its_tree() {
        let mut s = state_at("/home");
        let path = PathBuf::from("/home/folder");
        apply(&mut s, Change::Command(Command::Preview(path.clone())));
        apply(
            &mut s,
            Change::Done(Done::Previewed {
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
