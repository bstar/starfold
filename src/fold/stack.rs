//! The stack of levels a fold is drilled through.
//!
//! `▸ ~` / `▸ projects/` / `─ starwire/ ──` is one [`Stack`]: every directory
//! visited on the way down, in order, with [`Stack::active`] saying which one
//! is open. Popping does not delete a frame -- it only moves `active` back --
//! so `alt+down` can step forward into a child that is still there, the way a
//! browser's forward button works after back. Pushing a *new* child does
//! delete anything past the current frame, the way visiting a fresh URL does:
//! the old child trail was a guess about where you were going, and you just
//! went somewhere else.

use std::path::PathBuf;

/// Identifies a [`Frame`] for the lifetime of its [`Stack`].
///
/// Not an index: an index shifts under a push or a pop and a stale one then
/// points at the wrong level. `worker.rs`'s jobs carry a directory instead of
/// a frame id for the same reason a listing job carries a path rather than an
/// index -- but a job's *cancellation* has to name the specific visit that
/// asked for it, which is what this is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameId(pub u64);

/// One level: a directory, and the cursor position a fold remembers so that
/// popping back into it looks exactly as it was left.
#[derive(Debug, Clone)]
pub struct Frame {
    pub id: FrameId,
    pub dir: PathBuf,
    /// The row index the cursor sits on, in the sorted, filtered view.
    pub cursor: usize,
    /// The entry name the cursor was on, kept alongside the index so a
    /// reload -- which can insert or remove rows above it -- can find the
    /// same file again rather than landing on whatever is now at that index.
    pub cursor_name: Option<std::ffi::OsString>,
    pub filter: String,
    /// The topmost row drawn, so paging remembers where the view was scrolled
    /// to rather than re-centring on the cursor every frame.
    pub view: usize,
    /// Set while a listing job for this frame's directory is in flight.
    pub loading: bool,
}

impl Frame {
    fn new(id: FrameId, dir: PathBuf) -> Self {
        Self {
            id,
            dir,
            cursor: 0,
            cursor_name: None,
            filter: String::new(),
            view: 0,
            loading: true,
        }
    }
}

/// The levels of one fold, from the root of this drill-down to the deepest
/// child still remembered.
#[derive(Debug, Clone)]
pub struct Stack {
    frames: Vec<Frame>,
    active: usize,
    next_id: u64,
}

impl Stack {
    /// A stack with one frame, open on `dir`.
    pub fn new(dir: PathBuf) -> Self {
        Self {
            frames: vec![Frame::new(FrameId(0), dir)],
            active: 0,
            next_id: 1,
        }
    }

    /// Drill into `dir`. Anything past the active frame -- a child trail from
    /// before the user backed out and went somewhere else -- is dropped
    /// first, the way a fresh navigation replaces a browser's forward
    /// history.
    pub fn push(&mut self, dir: PathBuf) -> FrameId {
        self.frames.truncate(self.active + 1);
        let id = FrameId(self.next_id);
        self.next_id += 1;
        self.frames.push(Frame::new(id, dir));
        self.active = self.frames.len() - 1;
        id
    }

    /// Back one level. The popped frame is not removed -- `jump_to` or
    /// `alt+down` can still reach it -- only `active` moves. `false` at the
    /// root, where there is nowhere to go.
    pub fn pop(&mut self) -> bool {
        if self.active == 0 {
            return false;
        }
        self.active -= 1;
        true
    }

    /// Jump straight to a level by its position in [`crumbs`](Self::crumbs),
    /// keeping every frame on both sides of it. `false` for an index past the
    /// end.
    pub fn jump_to(&mut self, index: usize) -> bool {
        if index >= self.frames.len() {
            return false;
        }
        self.active = index;
        true
    }

    pub fn active(&self) -> &Frame {
        &self.frames[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Frame {
        &mut self.frames[self.active]
    }

    /// Every level from the root to the active one, in drill-down order --
    /// what the column's folded rows are drawn from.
    pub fn crumbs(&self) -> &[Frame] {
        &self.frames[..=self.active]
    }

    /// How many levels deep the active frame is, counting from one.
    pub fn depth(&self) -> usize {
        self.active + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn pushing_truncates_the_child_trail() {
        let mut s = Stack::new("/home".into());
        s.push("/home/projects".into());
        s.push("/home/projects/starwire".into());
        assert_eq!(s.depth(), 3);

        s.pop();
        assert_eq!(s.active().dir, PathBuf::from("/home/projects"));

        // A fresh push from here must drop the "starwire" child that is no
        // longer where the user is.
        s.push("/home/projects/starfold".into());
        assert_eq!(s.depth(), 3);
        assert_eq!(s.active().dir, PathBuf::from("/home/projects/starfold"));
        let dropped_child = std::path::Path::new("/home/projects/starwire");
        assert!(s.crumbs().iter().all(|f| f.dir != dropped_child));
    }

    #[test]
    fn popping_at_the_root_does_nothing() {
        let mut s = Stack::new("/home".into());
        assert!(!s.pop());
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn jump_to_keeps_every_frame_on_both_sides() {
        let mut s = Stack::new("/a".into());
        s.push("/a/b".into());
        s.push("/a/b/c".into());
        assert!(s.jump_to(0));
        assert_eq!(s.active().dir, PathBuf::from("/a"));
        // The child trail is still there: alt+down can step forward again.
        assert_eq!(s.crumbs().len(), 1);
        assert!(s.jump_to(2));
        assert_eq!(s.active().dir, PathBuf::from("/a/b/c"));
    }

    #[test]
    fn jump_to_past_the_end_fails_and_changes_nothing() {
        let mut s = Stack::new("/a".into());
        assert!(!s.jump_to(5));
        assert_eq!(s.depth(), 1);
    }

    proptest! {
        #[test]
        fn the_active_frame_is_always_in_bounds(
            ops in proptest::collection::vec(0u8..3, 0..30)
        ) {
            let mut s = Stack::new("/root".into());
            for (i, op) in ops.iter().enumerate() {
                match op {
                    0 => { s.push(PathBuf::from(format!("/root/{i}"))); },
                    1 => { s.pop(); },
                    _ => { s.jump_to(i % 4); },
                }
                prop_assert!(s.active < s.frames.len());
            }
        }
    }
}
