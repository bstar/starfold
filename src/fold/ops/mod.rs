//! The operations queue: what will happen to which files, and how far it has
//! got.
//!
//! An [`Op`] is queued the moment `y`, `m` or `d` is pressed -- the queue
//! *is* the confirmation, the way STAR/CORD's composer holds a draft rather
//! than sending on every keystroke. Nothing here runs anything: `plan`,
//! `exec` and `trash`, which turn a queued `Op` into bytes moving on disk,
//! are Phase 1c's files and are deliberately not declared yet, so that this
//! module compiles as the vocabulary the UI and the worker are both written
//! against before either of them exists.

pub mod exec;
pub mod plan;
pub mod progress;
pub mod trash;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use progress::Progress;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
    /// How is decided when the op is queued, from `[ops] trash` and whether a
    /// trash was found at startup, so the queue entry can say which it will
    /// be before anything runs.
    Delete(DeleteHow),
    Rename,
    Compress(super::archive::Format),
    Extract,
}

/// How a delete removes a file. Decided once, at plan time, from
/// [`super::TrashMode`] and whether a trash was found at startup -- not
/// re-decided per file, so a queue entry's title can say which it will be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteHow {
    Trash,
    Permanent,
}

/// What to do when a copy or a move would overwrite something already at the
/// destination.
///
/// Serialised in lowercase to match `config.toml`'s `[ops] conflicts` key;
/// `RenameNew` is spelled `"rename"` there, because "give the new one a new
/// name" is the reading a person configuring this wants, and "renamenew"
/// is not a word anybody would type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictPolicy {
    /// Stop and ask. The default: a queue that quietly overwrote something
    /// would not be trusted with a second file.
    #[default]
    Ask,
    Skip,
    Overwrite,
    #[serde(rename = "rename")]
    RenameNew,
}

/// One collision `plan` found between a source and what is already at the
/// destination.
#[derive(Debug, Clone)]
pub struct Conflict {
    pub source: PathBuf,
    pub dest: PathBuf,
    /// Both sides are directories, so `ConflictPolicy::Overwrite` merges into
    /// the existing one instead of removing it first -- removing it first
    /// would throw away whatever was already there under names that are not
    /// among `sources`.
    pub both_dirs: bool,
}

/// What kind of thing one planned [`Item`] is, read the same way
/// [`super::entry::stat`] reads a listing row: `symlink_metadata`, never
/// followed. A directory's contents are further [`Item`]s in the same
/// `Plan`; a symlink is never expanded, whatever it points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Dir,
    /// The regular file's length, read once at plan time so `exec` never has
    /// to stat a source again to total or report progress.
    File(u64),
    Symlink,
}

/// One thing `exec` will touch, in the walk order `plan` found it: parents
/// before children, so creating a directory always happens before anything
/// is written inside it.
#[derive(Debug, Clone)]
pub struct Item {
    pub from: PathBuf,
    /// Where this lands: `dest/<name>/...` for a copy or move, the `to` path
    /// itself for a rename. `None` for a delete, which sends nothing
    /// anywhere.
    pub to: Option<PathBuf>,
    pub kind: ItemKind,
}

/// What `plan` worked out before anything is touched: the full list of
/// sources after expanding any directories, the total bytes, and every
/// conflict found along the way.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub sources: Vec<PathBuf>,
    /// `dest/` for a copy or move, the rename target for a rename, and an
    /// empty path for a delete, which has nowhere to go.
    pub dest: PathBuf,
    pub total_bytes: u64,
    /// `items.len()`, kept separately so a delete's progress -- which counts
    /// items, not bytes -- does not have to re-count the vector every time
    /// the status row reads it.
    pub total_items: usize,
    pub conflicts: Vec<Conflict>,
    /// Every item `exec` will touch, in walk order, so `exec` never reads a
    /// directory itself -- it only ever replays what `plan` already found.
    pub items: Vec<Item>,
    /// Index into `items` where each `sources[i]`'s own subtree begins, in
    /// the same order as `sources`. `None` at `i` means `sources[i]` was
    /// missing (a delete only -- `missing` also names it) and there is
    /// nothing to walk for it. `exec` uses this to find one top-level
    /// source's whole subtree as a contiguous slice without re-deriving the
    /// boundary from paths.
    pub roots: Vec<Option<usize>>,
    /// Sources that were already gone when planning a delete. Not fatal --
    /// unlike a missing copy or move source, which aborts the whole plan --
    /// because "it's already not there" is the outcome a delete of it was
    /// asking for in the first place.
    pub missing: Vec<PathBuf>,
}

/// How an operation ended: what got done, what was skipped by policy, what
/// failed and why, and whether it was stopped before the end.
///
/// Counts rather than a single verdict, because a copy of fourteen files
/// where one was unreadable is thirteen files copied and one line in the
/// status row, not a failure that leaves the person wondering about the
/// other thirteen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub done: usize,
    pub skipped: usize,
    pub failed: Vec<(PathBuf, String)>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpStatus {
    /// Sitting in the queue, not yet planned.
    Queued,
    /// `plan` is running on the worker thread.
    Planning,
    /// `plan` found a conflict under [`ConflictPolicy::Ask`] and is waiting
    /// for `Command::SetPolicy`. No worker ever blocks on the user; it stops
    /// here and the queue moves on to whatever else is runnable.
    NeedsPolicy,
    Running,
    Done,
    Failed,
    Cancelled,
}

/// One entry in the queue: what to do, to which files, and how it is going.
pub struct Op {
    pub id: OpId,
    pub kind: OpKind,
    pub sources: Vec<PathBuf>,
    pub dest: Option<PathBuf>,
    pub status: OpStatus,
    pub policy: ConflictPolicy,
    pub plan: Option<Plan>,
    /// Items left at the source by a conflict policy. A drop must never
    /// report a completed Move to the desktop when this is nonzero.
    pub skipped: usize,
    /// Shared with the ops thread while the op runs: the same `Arc` goes out
    /// in `Job::Run`, and the status row reads the atomics through this one.
    pub progress: Arc<Progress>,
}

impl Op {
    /// `COPY 14 files → ~/Archive`, the line the OPERATIONS panel draws.
    pub fn title(&self) -> String {
        if self.kind == OpKind::Copy && self.dest.is_none() {
            let noun = if self.sources.len() == 1 {
                "item"
            } else {
                "items"
            };
            return format!("EXPORT {} {noun}", self.sources.len());
        }
        let verb = match self.kind {
            OpKind::Copy => "COPY",
            OpKind::Move => "MOVE",
            OpKind::Delete(DeleteHow::Trash) => "TRASH",
            OpKind::Delete(DeleteHow::Permanent) => "DELETE",
            OpKind::Rename => "RENAME",
            OpKind::Compress(_) => "COMPRESS",
            OpKind::Extract => "EXTRACT",
        };
        let count = self.sources.len();
        let noun = if count == 1 { "item" } else { "items" };
        match &self.dest {
            Some(dest) => format!("{verb} {count} {noun} \u{2192} {}", dest.display()),
            None => format!("{verb} {count} {noun}"),
        }
    }

    /// Whether this op is still waiting its turn -- queued, being planned, or
    /// stalled on a conflict decision. `Queue::first_runnable` skips
    /// everything that is not.
    pub fn is_pending(&self) -> bool {
        matches!(
            self.status,
            OpStatus::Queued | OpStatus::Planning | OpStatus::NeedsPolicy
        )
    }
}

/// The queue itself, in the order things were asked for.
#[derive(Default)]
pub struct Queue {
    ops: Vec<Op>,
    next_id: u64,
    auto_start: HashSet<OpId>,
    manual_running: bool,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(
        &mut self,
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: Option<PathBuf>,
        policy: ConflictPolicy,
    ) -> OpId {
        let id = OpId(self.next_id);
        self.next_id += 1;
        self.ops.push(Op {
            id,
            kind,
            sources,
            dest,
            status: OpStatus::Queued,
            policy,
            plan: None,
            skipped: 0,
            progress: Arc::new(Progress::new(0)),
        });
        id
    }

    /// A dropped transfer starts as soon as the operations worker is free.
    /// Pending manual entries remain drafts until Run is requested again.
    pub fn enqueue_drop(
        &mut self,
        kind: OpKind,
        sources: Vec<PathBuf>,
        dest: PathBuf,
        policy: ConflictPolicy,
    ) -> OpId {
        let id = self.enqueue(kind, sources, Some(dest), policy);
        self.auto_start.insert(id);
        self.manual_running = false;
        id
    }

    pub fn begin_export(&mut self, sources: Vec<PathBuf>, policy: ConflictPolicy) -> OpId {
        let id = self.enqueue(OpKind::Copy, sources, None, policy);
        if let Some(op) = self.get_mut(id) {
            op.status = OpStatus::Running;
        }
        id
    }

    pub fn begin_import(
        &mut self,
        sources: Vec<PathBuf>,
        dest: PathBuf,
        policy: ConflictPolicy,
    ) -> OpId {
        let id = self.enqueue(OpKind::Copy, sources, Some(dest), policy);
        if let Some(op) = self.get_mut(id) {
            op.status = OpStatus::Running;
        }
        self.manual_running = false;
        id
    }

    pub fn start_manual(&mut self) {
        self.manual_running = true;
    }

    pub fn next_runnable(&self) -> Option<OpId> {
        self.ops
            .iter()
            .find(|op| op.status == OpStatus::Queued && self.auto_start.contains(&op.id))
            .map(|op| op.id)
            .or_else(|| self.manual_running.then(|| self.first_runnable()).flatten())
    }

    /// One entry by id, to be changed in place: its status, its policy, its
    /// plan. `state::apply` is the only caller, and the fields it sets are
    /// the ones `Op` makes public.
    pub fn get_mut(&mut self, id: OpId) -> Option<&mut Op> {
        self.ops.iter_mut().find(|op| op.id == id)
    }

    /// Drop one entry, wherever it is in the queue. `false` if it was not
    /// there -- already finished and swept, most likely.
    pub fn remove(&mut self, id: OpId) -> bool {
        let before = self.ops.len();
        self.ops.retain(|op| op.id != id);
        self.auto_start.remove(&id);
        self.ops.len() != before
    }

    /// `esc` on the queue: drop everything that has not started. A running
    /// op is left to finish, or to be stopped with `Command::Cancel`.
    pub fn clear_pending(&mut self) {
        self.ops.retain(|op| !op.is_pending());
        self.auto_start
            .retain(|id| self.ops.iter().any(|op| op.id == *id));
        self.manual_running = false;
    }

    /// The next queued entry the worker should start, in queue order. Skips
    /// anything already running, finished or waiting on a conflict decision.
    pub fn first_runnable(&self) -> Option<OpId> {
        self.ops
            .iter()
            .find(|op| op.status == OpStatus::Queued)
            .map(|op| op.id)
    }

    /// The one op the ops thread is on, if any -- there is never more than
    /// one, since `worker.rs` runs the queue serially.
    pub fn running(&self) -> Option<&Op> {
        self.ops.iter().find(|op| op.status == OpStatus::Running)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Op> {
        self.ops.iter()
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(id: u64, status: OpStatus) -> Op {
        Op {
            id: OpId(id),
            kind: OpKind::Copy,
            sources: vec!["/a".into()],
            dest: Some("/dest".into()),
            status,
            policy: ConflictPolicy::Ask,
            plan: None,
            skipped: 0,
            progress: Arc::new(Progress::new(0)),
        }
    }

    #[test]
    fn a_title_names_the_verb_the_count_and_the_destination() {
        let queue_op = op(1, OpStatus::Queued);
        assert_eq!(queue_op.title(), "COPY 1 item \u{2192} /dest");
    }

    #[test]
    fn enqueue_hands_back_increasing_ids() {
        let mut q = Queue::new();
        let a = q.enqueue(
            OpKind::Copy,
            vec!["/a".into()],
            Some("/d".into()),
            ConflictPolicy::Ask,
        );
        let b = q.enqueue(
            OpKind::Move,
            vec!["/b".into()],
            Some("/d".into()),
            ConflictPolicy::Ask,
        );
        assert_ne!(a, b);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn remove_drops_the_named_entry_only() {
        let mut q = Queue::new();
        let a = q.enqueue(OpKind::Copy, vec!["/a".into()], None, ConflictPolicy::Ask);
        let b = q.enqueue(OpKind::Copy, vec!["/b".into()], None, ConflictPolicy::Ask);
        assert!(q.remove(a));
        assert_eq!(q.len(), 1);
        assert!(!q.remove(a), "it is already gone");
        assert!(q.iter().any(|op| op.id == b));
    }

    #[test]
    fn clear_pending_leaves_a_running_op_alone() {
        let mut q = Queue::new();
        q.ops.push(op(1, OpStatus::Queued));
        q.ops.push(op(2, OpStatus::Running));
        q.clear_pending();
        assert_eq!(q.len(), 1);
        assert_eq!(q.iter().next().unwrap().id, OpId(2));
    }

    #[test]
    fn first_runnable_skips_what_is_not_queued() {
        let mut q = Queue::new();
        q.ops.push(op(1, OpStatus::Done));
        q.ops.push(op(2, OpStatus::NeedsPolicy));
        q.ops.push(op(3, OpStatus::Queued));
        assert_eq!(q.first_runnable(), Some(OpId(3)));
    }

    #[test]
    fn running_finds_the_one_op_in_flight() {
        let mut q = Queue::new();
        q.ops.push(op(1, OpStatus::Queued));
        q.ops.push(op(2, OpStatus::Running));
        assert_eq!(q.running().map(|op| op.id), Some(OpId(2)));
    }

    #[test]
    fn conflict_policy_serialises_in_lowercase_and_rename_new_is_just_rename() {
        #[derive(Serialize, Deserialize)]
        struct Wrap {
            conflicts: ConflictPolicy,
        }
        let text = toml::to_string(&Wrap {
            conflicts: ConflictPolicy::Ask,
        })
        .unwrap();
        assert_eq!(text.trim(), "conflicts = \"ask\"");
        let text = toml::to_string(&Wrap {
            conflicts: ConflictPolicy::RenameNew,
        })
        .unwrap();
        assert_eq!(text.trim(), "conflicts = \"rename\"");
        let parsed: Wrap = toml::from_str("conflicts = \"overwrite\"").unwrap();
        assert_eq!(parsed.conflicts, ConflictPolicy::Overwrite);
    }
}
