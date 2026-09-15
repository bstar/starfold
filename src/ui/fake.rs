//! A core with no threads behind it, for deterministic frames.
//!
//! [`handle`] wires up a real [`Handle`] the way [`Handle::spawn`] does --
//! same `State::new`, same synchronous first listing -- but hands the two
//! job queues to a [`Fake`] instead of spawning `starfold-io` and
//! `starfold-ops`. A test calls [`Handle::send`] exactly as the real UI
//! would, then calls [`Fake::pump`] to run whatever that produced: `pump`
//! drains both queues through [`worker::perform_io`], [`worker::perform_ops`]
//! and [`worker::finish`] -- the very functions the real threads call --
//! synchronously, on the test thread. Nothing here re-implements what a job
//! does; it only removes the channel and the thread from between "a job was
//! queued" and "its result was folded in", so a drawn frame in a test needs
//! no `sleep` and no retry loop to be sure the core is done.
//!
//! This is *not* a second core the way STAR/CORD's fake is a second
//! implementation of the Discord client: `state::apply`, `listing::read`,
//! `preview::build`, `ops::plan::plan` and `ops::exec::run` all run for
//! real, against a real temporary directory tree ([`testing::Fixture`]).
//! What is fake is only the absence of threads.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::fold::handle::{Command, Event, EventSink, Handle, HandleParts};
use crate::fold::state::State;
use crate::fold::worker::{self, Done, IoOutcome, Job, Senders};
use crate::fold::{listing, testing, FoldConfig};

/// A [`Handle`] with no worker threads behind it, and the plumbing that
/// stands in for them.
///
/// `fixture` is kept here -- rather than dropped once [`handle`] returns --
/// so the `TempDir` it owns lives as long as the test does; dropping it
/// early would delete the tree a still-running test is reading rows out of.
pub struct Fake {
    pub fixture: testing::Fixture,
    io: crossbeam_channel::Receiver<Job>,
    ops: crossbeam_channel::Receiver<Job>,
    state: Arc<RwLock<State>>,
    events: EventSink,
    senders: Senders,
    cfg: FoldConfig,
}

/// Build a [`Handle`] over the fixture tree, with a [`Fake`] standing in for
/// its two worker threads.
///
/// Mirrors [`Handle::spawn`] line for line apart from the threads themselves:
/// the same `State::new`, the same start directory listed synchronously
/// before anything else runs. `home` is the fixture's own root, not some
/// directory under it, so a frame drawn from it shows `~` for the top crumb
/// the same way a real session does.
pub fn handle(cfg: FoldConfig) -> (Handle, Fake) {
    let fixture = testing::Fixture::tree();
    let home = fixture.home().to_path_buf();

    let (io_tx, io_rx) = crossbeam_channel::unbounded();
    let (ops_tx, ops_rx) = crossbeam_channel::unbounded();
    let (event_tx, event_rx) = crossbeam_channel::unbounded();
    let dropped = Arc::new(AtomicU64::new(0));
    let events = EventSink::new(event_tx, Arc::clone(&dropped));
    let senders = Senders {
        io: io_tx,
        ops: ops_tx,
    };

    let state = Arc::new(RwLock::new(State::new(
        &cfg,
        home.clone(),
        home.clone(),
        true,
    )));

    // List the start directory synchronously, exactly as `Handle::spawn`
    // does, so the first frame a test draws already has rows instead of an
    // empty panel waiting for a thread that will never run.
    let listing = listing::read(&home, &cfg.list);
    worker::finish(Done::Listed(listing), &state, &events, &senders);

    let handle = Handle::from_parts(HandleParts {
        senders: senders.clone(),
        events: event_rx,
        state: Arc::clone(&state),
        sink: events.clone(),
        threads: (None, None),
        dropped,
    });

    let fake = Fake {
        fixture,
        io: io_rx,
        ops: ops_rx,
        state,
        events,
        senders,
        cfg,
    };

    (handle, fake)
}

impl Fake {
    /// Run every job either queue holds, on this thread, until both are
    /// empty. Returns how many jobs ran.
    ///
    /// Loops rather than draining each queue once: a `Done` folded in by
    /// [`worker::finish`] can itself produce more jobs -- a copy's
    /// `Done::Planned` hands back the `Job::Run` that actually moves the
    /// bytes -- and a caller that only drained once would see the plan land
    /// but not the run. Never blocks: both queues are drained with
    /// `try_recv`, so a test that forgot to send a command that would ever
    /// produce a job simply gets `0` back rather than hanging.
    pub fn pump(&self) -> usize {
        let mut ran = 0;
        loop {
            let mut any = false;
            while let Ok(job) = self.io.try_recv() {
                any = true;
                ran += 1;
                self.run_io(job);
            }
            while let Ok(job) = self.ops.try_recv() {
                any = true;
                ran += 1;
                self.run_ops(job);
            }
            if !any {
                break;
            }
        }
        ran
    }

    /// Run one io job through [`worker::perform_io`] and fold its result in
    /// -- the inline equivalent of one turn through `spawn_io`'s loop body.
    fn run_io(&self, job: Job) {
        let cancel = AtomicBool::new(false);
        match worker::perform_io(job, &self.cfg, &self.state, &cancel) {
            IoOutcome::Done(done) => worker::finish(done, &self.state, &self.events, &self.senders),
            IoOutcome::Note(note) => self.events.send(Event::Note(note)),
            IoOutcome::None => {}
        }
    }

    /// Run one ops job through [`worker::perform_ops`] and fold its result
    /// in -- the inline equivalent of one turn through `spawn_ops`'s loop
    /// body.
    fn run_ops(&self, job: Job) {
        if let Some(done) = worker::perform_ops(job, &self.cfg) {
            worker::finish(done, &self.state, &self.events, &self.senders);
        }
    }

    /// The truth, read the same way [`Handle::state`] reads it. A separate
    /// accessor rather than routing every assertion through the `Handle`
    /// itself, since a test also wants to read `Fake::fixture`.
    pub fn state(&self) -> RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The truth, writable -- for a frame snapshot that needs an `Op` in a
    /// state no real run leaves it in for long enough to draw, such as
    /// `Running` at a chosen percentage. Bypasses `state::apply`, so a
    /// caller that changes anything a drawn frame reads must bump
    /// `State::version` itself (`state.version += 1`) for `App::tick`'s
    /// `refresh` to notice; everything under `src/fold/` still runs for
    /// real, only this one seam is synthetic.
    pub fn state_mut(&self) -> RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(|e| e.into_inner())
    }

    /// The fixture's home directory -- `Fixture::home`, spelled the way a
    /// frame test that only has a `Fake` in scope wants it.
    pub fn home(&self) -> &Path {
        self.fixture.home()
    }
}

/// Push into a directory under the fixture home and run whatever that
/// queues, in one call -- what a frame snapshot uses to reach a level like
/// `projects/starwire` before it draws.
///
/// A free function rather than a `Fake` or `Handle` method: it needs both --
/// the command goes to `handle`, the draining to `fake` -- and neither owns
/// the other, so a method on either would have to take the other as an
/// argument anyway.
pub fn open(handle: &Handle, fake: &Fake, relative: &str) {
    handle.send(Command::Push(fake.home().join(relative)));
    fake.pump();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::entry::EntryKind;
    use crate::fold::ops::{ConflictPolicy, OpStatus};
    use crate::fold::preview::Preview;
    use crate::fold::TrashMode;

    fn row_names(fake: &Fake) -> Vec<String> {
        let state = fake.state();
        let frame = state.active_frame();
        state
            .rows(frame)
            .into_iter()
            .map(|e| e.display.clone())
            .collect()
    }

    fn row_index(fake: &Fake, name: &str) -> usize {
        row_names(fake)
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("no row named {name} in {:?}", row_names(fake)))
    }

    #[test]
    fn building_a_handle_lists_the_home_directory_with_the_fixtures_names() {
        let (_handle, fake) = handle(FoldConfig::default());
        assert!(
            !fake.state().loading,
            "the start directory is listed synchronously"
        );
        let names = row_names(&fake);
        assert!(names.contains(&"projects".to_string()), "got {names:?}");
        assert!(names.contains(&"blob.bin".to_string()), "got {names:?}");
        assert!(names.contains(&"empty".to_string()), "got {names:?}");
    }

    #[test]
    fn entering_a_directory_then_pumping_lists_it() {
        let (handle, fake) = handle(FoldConfig::default());
        let row = row_index(&fake, "projects");
        handle.send(Command::CursorTo(row));
        handle.send(Command::Enter);

        assert_eq!(
            fake.state().active_frame().dir,
            fake.home().join("projects")
        );
        assert!(fake.state().active_frame().loading, "not listed yet");

        let ran = fake.pump();
        assert!(ran > 0, "the List job should have run");
        assert!(!fake.state().active_frame().loading);
        assert!(row_names(&fake).contains(&"starwire".to_string()));
    }

    #[test]
    fn opening_reaches_a_nested_directory_in_one_call() {
        let (handle, fake) = handle(FoldConfig::default());
        open(&handle, &fake, "projects/starwire");
        assert_eq!(
            fake.state().active_frame().dir,
            fake.home().join("projects/starwire")
        );
        assert!(row_names(&fake).contains(&"Cargo.toml".to_string()));
    }

    #[test]
    fn previewing_a_text_file_then_pumping_sets_a_text_preview() {
        let (handle, fake) = handle(FoldConfig::default());
        let readme = fake.home().join("projects/starwire/README.md");
        handle.send(Command::Preview(readme.clone()));
        let ran = fake.pump();
        assert!(ran > 0);

        let state = fake.state();
        let (path, preview) = state.preview.as_ref().expect("a preview was built");
        assert_eq!(path, &readme);
        assert!(
            matches!(preview.as_ref(), Preview::Text { .. }),
            "got {preview:?}"
        );
    }

    #[test]
    fn copying_a_marked_file_runs_through_the_queue_and_lands_at_the_destination() {
        let (handle, fake) = handle(FoldConfig::default());

        let row = row_index(&fake, "blob.bin");
        handle.send(Command::CursorTo(row));
        handle.send(Command::ToggleMark);
        assert!(fake
            .state()
            .selection
            .is_marked(&fake.home().join("blob.bin")));

        open(&handle, &fake, "empty");
        handle.send(Command::QueueCopyHere);
        handle.send(Command::Run);

        let planned = fake.pump();
        assert!(planned > 0, "planning should have produced a run");

        let dest = fake.home().join("empty/blob.bin");
        assert!(dest.exists(), "the copy has not landed yet: {dest:?}");

        let state = fake.state();
        let op = state.queue.iter().next().expect("one queued op");
        assert_eq!(op.status, OpStatus::Done, "got {:?}", op.status);
    }

    #[test]
    fn deleting_with_trash_mode_never_removes_the_file_permanently() {
        let (handle, fake) = handle(FoldConfig::default());
        // `Never` forces a permanent delete regardless of `trash_available`,
        // which `handle()` set to `true` above -- the same rule
        // `cmd_queue_delete` applies from `[ops] trash`. Setting it directly
        // on `State` rather than through a `Command`: there is no command
        // for it, `[ops] trash` is config `Handle::spawn` bakes in once at
        // startup.
        fake.state.write().unwrap().trash = TrashMode::Never;

        let row = row_index(&fake, "blob.bin");
        handle.send(Command::CursorTo(row));
        handle.send(Command::ToggleMark);
        handle.send(Command::QueueDelete);
        handle.send(Command::Run);
        fake.pump();

        assert!(!fake.home().join("blob.bin").exists());
        let state = fake.state();
        let op = state.queue.iter().next().expect("one queued op");
        assert_eq!(op.status, OpStatus::Done, "got {:?}", op.status);
    }

    #[test]
    fn renaming_a_file_moves_it_to_the_new_name() {
        let (handle, fake) = handle(FoldConfig::default());
        let from = fake.home().join("blob.bin");
        let to = fake.home().join("renamed.bin");

        handle.send(Command::QueueRename {
            from: from.clone(),
            to: to.clone(),
        });
        handle.send(Command::Run);
        fake.pump();

        assert!(!from.exists());
        assert!(to.exists());
    }

    #[test]
    fn pumping_an_empty_queue_runs_nothing() {
        let (_handle, fake) = handle(FoldConfig::default());
        assert_eq!(fake.pump(), 0);
    }

    #[test]
    fn conflict_policy_is_carried_from_the_config() {
        // Not a behavioural assertion on its own -- just confirms `handle`
        // reads `cfg.conflicts` into `State` the way `Handle::spawn` does,
        // rather than silently defaulting it.
        let cfg = FoldConfig {
            conflicts: ConflictPolicy::Overwrite,
            ..FoldConfig::default()
        };
        let (_handle, fake) = handle(cfg);
        assert_eq!(fake.state().conflicts, ConflictPolicy::Overwrite);
    }

    #[test]
    fn a_directory_row_reports_dir_kind() {
        let (_handle, fake) = handle(FoldConfig::default());
        let state = fake.state();
        let frame = state.active_frame();
        let rows = state.rows(frame);
        let projects = rows
            .iter()
            .find(|e| e.display == "projects")
            .expect("projects listed");
        assert_eq!(projects.kind, EntryKind::Dir);
    }
}
