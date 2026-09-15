//! The stack of levels a fold is drilled through.
//!
//! `▸ ~` / `▸ projects/` / `─ starwire/ ──` is one [`Stack`]: every directory
//! visited on the way down, in order, with [`Stack::active`] saying which one
//! is open. [`Stack::pop`] ("back one level", `h`) *removes* the frame you
//! back out of -- the level is gone, and going back into it later starts
//! fresh, exactly as `docs/the-stack.md` describes it. [`Stack::jump_to`]
//! (`alt+up`, a crumb click) and [`Stack::forward`] (`alt+down`) are the
//! opposite: they only move [`Stack::active`], keeping every frame on both
//! sides of it, so a level you jumped away from rather than backed out of is
//! still there -- cursor, filter, scroll position and all -- to jump straight
//! back into.
//!
//! Pushing a *new* child still drops anything past the active frame first --
//! a forward trail left by `jump_to`/`forward` was a guess about where the
//! user was going, and a fresh navigation replaces it the way visiting a new
//! URL drops a browser's forward history. A `pop`, by contrast, never leaves
//! a forward trail to drop: there is nothing past the active frame once
//! backing out has already removed it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Identifies a [`Frame`] for the lifetime of its [`Stack`].
///
/// Not an index: an index shifts under a push or a pop and a stale one then
/// points at the wrong level. `worker.rs`'s jobs carry a directory instead of
/// a frame id for the same reason a listing job carries a path rather than an
/// index -- but a job's *cancellation* has to name the specific visit that
/// asked for it, which is what this is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameId(pub u64);

/// One level: a directory, and everything a fold remembers about how it was
/// being looked at, so popping back into it (or jumping straight back)
/// re-draws exactly what was left.
#[derive(Debug, Clone)]
pub struct Frame {
    pub id: FrameId,
    pub dir: PathBuf,
    /// Indices into the directory's `Listing::entries`, in the order this
    /// frame currently draws them: `sort::order` applied to the listing,
    /// then `filter::rank` over the result. A `Stack` does not know what a
    /// `Listing` is, so nothing here ever builds this -- `state::apply`
    /// rebuilds it whenever the listing, the sort, the hidden flag or the
    /// filter changes, and everything else just reads it.
    pub rows: Vec<usize>,
    /// The position in `rows` the cursor sits on. Kept in bounds by whoever
    /// rebuilds `rows`; a frame with no rows sits at `0`.
    pub cursor: usize,
    /// The entry name the cursor was on, kept alongside the index so a
    /// reload -- which can insert or remove rows above it -- can find the
    /// same file again rather than landing on whatever is now at that index.
    pub cursor_name: Option<OsString>,
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
            rows: Vec::new(),
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
    /// before the user jumped away and went somewhere else -- is dropped
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

    /// Back one level. Unlike `jump_to`, the frame you back out of is
    /// discarded outright: `frames.truncate(active)` drops it, `active -= 1`
    /// lands on the frame that is now the last one remaining. `false`,
    /// changing nothing, at the root -- there is nowhere to go.
    pub fn pop(&mut self) -> bool {
        if self.active == 0 {
            return false;
        }
        self.frames.truncate(self.active);
        self.active -= 1;
        true
    }

    /// Jump straight to a level by its position in [`frames`](Self::frames)
    /// (which for an index `<= active` is also its position in
    /// [`crumbs`](Self::crumbs)), keeping every frame on both sides of it.
    /// `false`, changing nothing, for an index past the end.
    pub fn jump_to(&mut self, index: usize) -> bool {
        if index >= self.frames.len() {
            return false;
        }
        self.active = index;
        true
    }

    /// Step one frame towards the end of the trail -- the opposite of
    /// `jump_to` moving towards the root, and how `alt+down` steps back into
    /// a child that `alt+up` left behind rather than popped. `false` when the
    /// active frame is already the last one remembered.
    pub fn forward(&mut self) -> bool {
        if self.active + 1 >= self.frames.len() {
            return false;
        }
        self.active += 1;
        true
    }

    pub fn active(&self) -> &Frame {
        &self.frames[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Frame {
        &mut self.frames[self.active]
    }

    /// Every level from the root to the active one, in drill-down order --
    /// what the column's folded rows and the active one are drawn from.
    pub fn crumbs(&self) -> &[Frame] {
        &self.frames[..=self.active]
    }

    /// How many levels deep the active frame is, counting from one.
    pub fn depth(&self) -> usize {
        self.active + 1
    }

    /// How many frames the stack remembers in total, including whatever is
    /// past the active one and still reachable with `forward`. Always at
    /// least one: a `Stack` is never empty.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Whether `forward` has somewhere to go.
    pub fn has_forward(&self) -> bool {
        self.active + 1 < self.frames.len()
    }

    /// Every frame the stack remembers, active or not, root to deepest --
    /// what `state::apply` walks to rebuild rows when a sort, a hidden-files
    /// toggle or a fresh listing touches more than just the one frame on
    /// screen.
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// The same, mutably.
    pub fn frames_mut(&mut self) -> impl Iterator<Item = &mut Frame> {
        self.frames.iter_mut()
    }

    /// Every frame open on `dir`, mutably -- how a fresh listing for `dir`
    /// refreshes every level that happens to be looking at it, not only
    /// whichever one is active.
    pub fn frame_mut_by_dir<'a>(
        &'a mut self,
        dir: &'a Path,
    ) -> impl Iterator<Item = &'a mut Frame> + 'a {
        self.frames.iter_mut().filter(move |f| f.dir == dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn pushing_truncates_the_forward_trail_left_by_a_jump() {
        let mut s = Stack::new("/home".into());
        s.push("/home/projects".into());
        s.push("/home/projects/starwire".into());
        assert_eq!(s.len(), 3);

        // Jump back to the root without popping: "starwire" is still there,
        // reachable with `forward`.
        s.jump_to(0);
        assert!(s.has_forward());

        // A fresh push from here must drop the "starwire" child, because the
        // user just went somewhere else.
        s.push("/home/projects/starfold".into());
        assert_eq!(s.len(), 2);
        assert_eq!(s.active().dir, PathBuf::from("/home/projects/starfold"));
        let dropped_child = Path::new("/home/projects/starwire");
        assert!(s.frames().iter().all(|f| f.dir != dropped_child));
    }

    #[test]
    fn back_removes_the_frame_it_leaves_rather_than_just_stepping_off_it() {
        let mut s = Stack::new("/home".into());
        s.push("/home/projects".into());
        s.push("/home/projects/starwire".into());
        assert_eq!(s.len(), 3);

        assert!(s.pop());
        assert_eq!(s.len(), 2, "the frame backed out of is gone, not kept");
        assert_eq!(s.active().dir, PathBuf::from("/home/projects"));
        assert!(
            !s.has_forward(),
            "there is nothing left to step forward into"
        );
    }

    #[test]
    fn popping_at_the_root_does_nothing() {
        let mut s = Stack::new("/home".into());
        assert!(!s.pop());
        assert_eq!(s.depth(), 1);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn jump_to_keeps_every_frame_on_both_sides() {
        let mut s = Stack::new("/a".into());
        s.push("/a/b".into());
        s.push("/a/b/c".into());
        assert!(s.jump_to(0));
        assert_eq!(s.active().dir, PathBuf::from("/a"));
        // The child trail is still there: `forward` can step forward again.
        assert_eq!(s.len(), 3);
        assert!(s.jump_to(2));
        assert_eq!(s.active().dir, PathBuf::from("/a/b/c"));
    }

    #[test]
    fn jump_to_past_the_end_fails_and_changes_nothing() {
        let mut s = Stack::new("/a".into());
        assert!(!s.jump_to(5));
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn forward_steps_into_a_child_a_jump_left_behind() {
        let mut s = Stack::new("/a".into());
        s.push("/a/b".into());
        assert!(s.jump_to(0));
        assert!(s.has_forward());

        assert!(s.forward());
        assert_eq!(s.active().dir, PathBuf::from("/a/b"));
        assert!(!s.forward(), "there is nothing past the last frame");
    }

    #[test]
    fn frame_mut_by_dir_reaches_every_frame_open_on_that_directory() {
        let mut s = Stack::new("/a".into());
        // Nothing stops the same directory from being pushed twice in a
        // row -- a symlink cycle, or just re-entering a place the trail
        // already passed through -- and both frames stay on the stack.
        s.push("/a/b".into());
        s.push("/a/b".into());
        assert_eq!(s.len(), 3);

        let matching = s.frame_mut_by_dir(Path::new("/a/b")).count();
        assert_eq!(matching, 2);
    }

    proptest! {
        #[test]
        fn the_active_frame_is_always_in_bounds_and_the_stack_never_empties(
            ops in proptest::collection::vec(0u8..4, 0..40)
        ) {
            let mut s = Stack::new("/root".into());
            for (i, op) in ops.iter().enumerate() {
                match op {
                    0 => { s.push(PathBuf::from(format!("/root/{i}"))); },
                    1 => { s.pop(); },
                    2 => { s.jump_to(i % 4); },
                    _ => { s.forward(); },
                }
                prop_assert!(s.active < s.frames.len());
                prop_assert!(!s.is_empty());
            }
        }
    }
}
