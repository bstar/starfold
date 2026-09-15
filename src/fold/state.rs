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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::handle::{Command, Event};
use super::listing::Listing;
use super::ops::Queue;
use super::preview::Preview;
use super::selection::Selection;
use super::sort::SortOrder;
use super::stack::{Frame, Stack};
use super::tab::Tabs;
use super::worker::{Done, Job};
use super::FoldConfig;

/// The whole of what the program knows, right now.
pub struct State {
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
    /// One tab, one stack, one frame open on `start`.
    pub fn new(cfg: &FoldConfig, start: PathBuf, home: PathBuf, trash_available: bool) -> Self {
        Self {
            tabs: Tabs::single(Stack::new(start)),
            listings: HashMap::new(),
            selection: Selection::default(),
            queue: Queue::new(),
            preview: None,
            sort: cfg.list.sort,
            show_hidden: cfg.list.show_hidden,
            trash_available,
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
/// `// TODO(1b)`: this is the bootstrap stub. It touches nothing and returns
/// nothing; the real state machine -- one arm of a `match` per `Command` and
/// per `Done`, described in the plan's "Command / Event contract" -- is Phase
/// 1b's.
pub fn apply(state: &mut State, change: Change) -> Effects {
    let _ = (state, change);
    Effects::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> FoldConfig {
        FoldConfig::default()
    }

    #[test]
    fn a_fresh_state_has_one_tab_one_stack_one_frame_and_is_loading() {
        let s = State::new(&cfg(), "/home/bob".into(), "/home/bob".into(), true);
        assert_eq!(s.tabs.tabs.len(), 1);
        assert_eq!(s.tabs.active().stacks.len(), 1);
        assert_eq!(s.active_frame().dir, PathBuf::from("/home/bob"));
        assert!(s.loading);
        assert!(s.trash_available);
        assert_eq!(s.version, 0);
    }

    #[test]
    fn active_frame_mut_reaches_the_same_frame_active_frame_reads() {
        let mut s = State::new(&cfg(), "/a".into(), "/a".into(), false);
        s.active_frame_mut().cursor = 3;
        assert_eq!(s.active_frame().cursor, 3);
    }

    #[test]
    fn crumbs_starts_as_the_one_root_frame() {
        let s = State::new(&cfg(), "/a".into(), "/a".into(), false);
        assert_eq!(s.crumbs().len(), 1);
    }

    #[test]
    fn apply_is_a_pure_stub_for_now() {
        let mut s = State::new(&cfg(), "/a".into(), "/a".into(), false);
        let effects = apply(&mut s, Change::Command(Command::Reload));
        assert!(effects.jobs.is_empty());
    }
}
