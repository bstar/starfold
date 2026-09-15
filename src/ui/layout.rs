//! Where everything is, worked out once a frame.
//!
//! [`LayoutState::regions`] is the only place in the program that decides a
//! rectangle. It is called at the top of `draw`, its answer is kept in
//! `layout.last`, and `handle_mouse` reads that rather than recomputing
//! anything. Two pieces of code that have to agree about geometry, one of
//! which is only ever exercised by a pointer, is where a layout bug lives;
//! there is one here.
//!
//! ## One column
//!
//! ```text
//! STACK          the crumbs and the active level's listing, takes the slack
//! PREVIEW        folds to one line, or opens beside the stack
//! OPERATIONS     folds to one line, or opens beside the stack
//! status         one row, never anything else
//! ```
//!
//! Every module is always there, in the order you drill through them, and the
//! arithmetic below is the whole of it: a plain top-down stack, no tree and no
//! `Layout`. Unlike STAR/CORD, where at most one list is open at a time, here
//! the stack never folds -- it is always the thing being drilled through --
//! and PREVIEW and OPERATIONS each fold independently of the other.
//!
//! ## What each module gets
//!
//! The stack keeps at least [`STACK_MIN_ROWS`] and takes whatever is left
//! over once the other two have what they want. PREVIEW is [`COLLAPSED_ROWS`]
//! folded; open and unfocused it grows by `[ui] preview_rows`; open and
//! focused it grows to as much as half the body. OPERATIONS is
//! [`COLLAPSED_ROWS`] unless it is focused, in which case it grows to show up
//! to `[ui] ops_rows` of the queue (one entry already shows in its folded
//! line, so the extra is `min(queued, ops_rows) - 1`). An op running in the
//! background changes nothing here -- the module stays folded and its one
//! line shows the bar instead of a queue count, which is the panel's business
//! and not this one's.
//!
//! ## Short terminals
//!
//! There is no degradation ladder. Below [`MIN_COLS`] or [`MIN_ROWS`] the
//! caller draws one line saying so. Above the floor, PREVIEW's extra rows are
//! served first (so they are the last thing lost as the terminal shrinks),
//! whatever is left goes to OPERATIONS' extra, and whatever remains after
//! both goes to the stack.

use starkit::chrome::header;
use starkit::ratatui::layout::Rect;

use super::panels::{ModuleId, COLUMN};

/// Below this the layout is not drawn at all.
///
/// Sixty columns is the narrowest a row of metadata beside a file name is
/// worth drawing. Twenty-one rows is arithmetic rather than judgement: it is
/// the sum of the constants below, which is what makes it the height at
/// which every module still has a row of content -- the same 60x21 floor as
/// STAR/CORD's.
pub const MIN_COLS: u16 = 60;

/// A module with nothing open in it: two borders, the header row the action
/// words sit on, and one row of content. A module with no content row is a
/// box with a title.
pub const COLLAPSED_ROWS: u16 = 2 + header::ROWS + 1;

/// The fewest rows the active level's listing is worth drawing at.
pub const LIST_ROWS_MIN: u16 = 7;

/// The stack's floor: two borders, the header row, one crumb row (parent
/// levels squeezed to a single `▸ ~ › projects › …` line), the active
/// level's rule, and its listing at [`LIST_ROWS_MIN`].
pub const STACK_MIN_ROWS: u16 = 2 + header::ROWS + 1 + 1 + LIST_ROWS_MIN;

/// The stack at its floor, plus PREVIEW and OPERATIONS both folded, plus the
/// status row.
pub const MIN_ROWS: u16 = STACK_MIN_ROWS + 2 * COLLAPSED_ROWS + 1;

/// One frame's geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Regions {
    /// The whole of it, padding already taken off.
    pub area: Rect,
    /// One rect per module, indexed by [`ModuleId::index`], in [`COLUMN`]
    /// order. Every one of them is the full width of the area.
    modules: [Rect; 3],
    pub status: Rect,
}

impl Regions {
    pub fn rect_of(&self, m: ModuleId) -> Rect {
        self.modules[m.index()]
    }

    /// Which module a cell is in.
    pub fn hit(&self, x: u16, y: u16) -> Option<ModuleId> {
        COLUMN.into_iter().find(|m| {
            let r = self.rect_of(*m);
            x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
        })
    }
}

/// Which module has the keyboard, whether PREVIEW is open, and what each
/// module is configured to grow to.
pub struct LayoutState {
    focus: ModuleId,
    /// Whether PREVIEW draws more than its folded line. Unlike STAR/CORD's
    /// lists, this is not an accordion against OPERATIONS: the two fold
    /// independently, so this is its own flag rather than an `Option` shared
    /// with anything else.
    pub preview_open: bool,
    /// `[ui] preview_rows`: how far PREVIEW grows when it is open but not
    /// focused.
    pub preview_rows: u16,
    /// `[ui] ops_rows`: how far OPERATIONS grows while focused, up to its own
    /// queue length.
    pub ops_rows: u16,
    /// `[ui] fold_rows`: how many rows a folded parent level may draw before
    /// the stack squeezes them into one crumb row. The stack panel reads
    /// this to lay out its own content within whatever height it was given;
    /// it plays no part in the arithmetic below, because [`STACK_MIN_ROWS`]
    /// already assumes the squeezed case.
    pub fold_rows: u16,
    pub last: Option<Regions>,
}

impl LayoutState {
    pub fn new(preview_rows: u16, ops_rows: u16, fold_rows: u16) -> Self {
        Self {
            focus: ModuleId::Stack,
            preview_open: true,
            preview_rows,
            ops_rows,
            fold_rows,
            last: None,
        }
    }

    /// The whole geometry of one frame, or `None` when the terminal is too
    /// small to draw anything honest in.
    ///
    /// `queued` is how many entries are in the operations queue, for
    /// OPERATIONS' open height. Whether an op is currently running changes
    /// no height: the module stays at whatever height focus already gives
    /// it, and draws the progress bar on its one summary line instead of a
    /// queue count.
    pub fn regions(&mut self, full: Rect, pad: (u16, u16), queued: u16) -> Option<&Regions> {
        let area = inset(full, pad);
        if area.width < MIN_COLS || area.height < MIN_ROWS {
            self.last = None;
            return None;
        }

        // The status line first, off the bottom, because it is never hidden,
        // never focused and never resized.
        let status = Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        };
        let body = Rect {
            height: area.height - 1,
            ..area
        };

        // Everyone at their floor is the baseline; `room` is how much taller
        // than that the body is. The `MIN_ROWS` check above guarantees this
        // does not underflow.
        let room = body.height - STACK_MIN_ROWS - 2 * COLLAPSED_ROWS;

        // PREVIEW's extra is served first, so it is the last thing lost as
        // the terminal shrinks.
        let preview_desired = if !self.preview_open {
            0
        } else if self.focus == ModuleId::Preview {
            // Up to half the body while focused, not capped by
            // `preview_rows` -- the reader asked to look at it.
            (body.height / 2).saturating_sub(COLLAPSED_ROWS)
        } else {
            self.preview_rows
        };
        let preview_extra = preview_desired.min(room);

        // OPERATIONS only grows while focused, and gives up its extra before
        // PREVIEW does: whatever room PREVIEW left is all it can have.
        let ops_desired = if self.focus == ModuleId::Operations {
            // One entry of the queue already shows on the folded line.
            queued.min(self.ops_rows).saturating_sub(1)
        } else {
            0
        };
        let ops_extra = ops_desired.min(room - preview_extra);

        // Whatever is left over goes to the stack.
        let stack_extra = room - preview_extra - ops_extra;

        let height = |m: ModuleId| match m {
            ModuleId::Stack => STACK_MIN_ROWS + stack_extra,
            ModuleId::Preview => COLLAPSED_ROWS + preview_extra,
            ModuleId::Operations => COLLAPSED_ROWS + ops_extra,
        };

        let mut modules = [Rect::new(0, 0, 0, 0); 3];
        let mut y = body.y;
        for m in COLUMN {
            let h = height(m);
            modules[m.index()] = Rect {
                y,
                height: h,
                ..body
            };
            y += h;
        }

        self.last = Some(Regions {
            area,
            modules,
            status,
        });
        self.last.as_ref()
    }

    pub fn focus(&self) -> ModuleId {
        self.focus
    }

    /// Focus a module. Focusing PREVIEW opens it -- a module you are about to
    /// read is a module you can see.
    pub fn focus_set(&mut self, m: ModuleId) {
        if m == ModuleId::Preview {
            self.preview_open = true;
        }
        self.focus = m;
    }

    pub fn focus_next(&mut self) {
        let i = (self.focus.index() + 1) % COLUMN.len();
        self.focus_set(COLUMN[i]);
    }

    pub fn focus_prev(&mut self) {
        let i = (self.focus.index() + COLUMN.len() - 1) % COLUMN.len();
        self.focus_set(COLUMN[i]);
    }

    /// `i`: open PREVIEW if it was folded; fold it, landing focus on the
    /// stack, if it was open and focused. Folding it while it is open but
    /// not the one in focus leaves focus wherever it already was -- there is
    /// nothing to land, because nothing was looking at it.
    pub fn toggle_preview(&mut self) {
        if self.preview_open {
            self.preview_open = false;
            if self.focus == ModuleId::Preview {
                self.focus = ModuleId::Stack;
            }
        } else {
            self.preview_open = true;
        }
    }

    /// Whether `m` is drawing more than its one folded line right now.
    pub fn is_open(&self, m: ModuleId) -> bool {
        match m {
            ModuleId::Stack => true,
            ModuleId::Preview => self.preview_open,
            ModuleId::Operations => self.focus == ModuleId::Operations,
        }
    }
}

/// Shrink a rect by the configured padding, never past nothing.
fn inset(area: Rect, pad: (u16, u16)) -> Rect {
    let (x, y) = pad;
    let width = area.width.saturating_sub(x.saturating_mul(2));
    let height = area.height.saturating_sub(y.saturating_mul(2));
    if width == 0 || height == 0 {
        return Rect {
            width: 0,
            height: 0,
            ..area
        };
    }
    Rect {
        x: area.x + x,
        y: area.y + y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn state() -> LayoutState {
        LayoutState::new(10, 6, 6)
    }

    fn min_height(m: ModuleId) -> u16 {
        match m {
            ModuleId::Stack => STACK_MIN_ROWS,
            ModuleId::Preview | ModuleId::Operations => COLLAPSED_ROWS,
        }
    }

    /// The property the whole module rests on: the modules cover the body
    /// exactly, top to bottom, each of them the full width. A gap is a cell
    /// nothing redraws, which keeps whatever was there last; an overlap is
    /// two modules writing the same cell in an order nobody chose.
    #[test]
    fn the_modules_tile_the_body_from_top_to_bottom() {
        for width in [59u16, 60, 61, 100, 200] {
            for height in [20u16, 21, 22, 30, 60] {
                for focus in COLUMN {
                    for preview_open in [false, true] {
                        for queued in [0u16, 2, 9] {
                            let mut s = state();
                            s.focus = focus;
                            s.preview_open = preview_open;
                            let full = Rect::new(0, 0, width, height);
                            let Some(r) = s.regions(full, (0, 0), queued).cloned() else {
                                assert!(
                                    width < MIN_COLS || height < MIN_ROWS,
                                    "{width}x{height} refused to lay out"
                                );
                                continue;
                            };

                            let mut y = r.area.y;
                            for m in COLUMN {
                                let rect = r.rect_of(m);
                                assert_eq!(rect.x, r.area.x, "{m:?} at {width}x{height}");
                                assert_eq!(rect.width, r.area.width, "{m:?} at {width}x{height}");
                                assert_eq!(rect.y, y, "{m:?} at {width}x{height} is not stacked");
                                assert!(
                                    rect.height >= min_height(m),
                                    "{m:?} is {rect:?}, below its floor"
                                );
                                y += rect.height;
                            }
                            assert_eq!(y, r.status.y, "the stack does not reach the status line");
                            assert_eq!(r.status.height, 1);
                            assert_eq!(r.status.y, r.area.y + r.area.height - 1);
                        }
                    }
                }
            }
        }
    }

    /// Below the floor there is no layout at all, and the caller draws one
    /// line saying so.
    #[test]
    fn a_terminal_below_the_floor_gets_nothing() {
        for (w, h) in [(59u16, 30u16), (60, 20), (40, 8), (0, 0)] {
            let mut s = state();
            let r = s.regions(Rect::new(0, 0, w, h), (0, 0), 3);
            assert!(r.is_none(), "{w}x{h} laid out");
            assert!(s.last.is_none());
            // And the next frame at a workable size recovers.
            assert!(s.regions(Rect::new(0, 0, 100, 30), (0, 0), 3).is_some());
        }
    }

    /// Padding comes off the outside and the floor is measured after it, so
    /// a padded sixty-four-column terminal is a sixty-column layout.
    #[test]
    fn padding_is_taken_before_the_floor_is_measured() {
        let mut s = state();
        let r = s
            .regions(Rect::new(0, 0, MIN_COLS + 4, MIN_ROWS + 2), (2, 1), 3)
            .cloned()
            .expect("60x21 left");
        assert_eq!(r.area, Rect::new(2, 1, MIN_COLS, MIN_ROWS));

        let mut s = state();
        assert!(
            s.regions(Rect::new(0, 0, MIN_COLS + 3, MIN_ROWS + 2), (2, 1), 3)
                .is_none(),
            "59 columns after padding is below the floor"
        );
    }

    /// Whatever anybody asks for, the stack keeps its minimum.
    #[test]
    fn the_stack_never_drops_below_its_minimum() {
        for height in MIN_ROWS..=60 {
            for focus in COLUMN {
                for queued in [0u16, 5, 200] {
                    let mut s = state();
                    s.focus = focus;
                    let r = s
                        .regions(Rect::new(0, 0, 100, height), (0, 0), queued)
                        .cloned()
                        .unwrap();
                    assert!(
                        r.rect_of(ModuleId::Stack).height >= STACK_MIN_ROWS,
                        "{height} rows, focus {focus:?}, queued {queued}"
                    );
                }
            }
        }
    }

    /// At the floor every module is exactly its own minimum.
    #[test]
    fn at_the_floor_every_module_is_its_minimum() {
        assert_eq!(MIN_ROWS, 21);
        let mut s = state();
        let r = s
            .regions(Rect::new(0, 0, MIN_COLS, MIN_ROWS), (0, 0), 50)
            .cloned()
            .unwrap();
        assert_eq!(r.rect_of(ModuleId::Stack).height, STACK_MIN_ROWS);
        assert_eq!(r.rect_of(ModuleId::Preview).height, COLLAPSED_ROWS);
        assert_eq!(r.rect_of(ModuleId::Operations).height, COLLAPSED_ROWS);
        assert_eq!(COLLAPSED_ROWS, 4);
        assert_eq!(STACK_MIN_ROWS, 12);
    }

    /// OPERATIONS only grows while it has focus, whatever else is going on.
    #[test]
    fn operations_grows_only_while_focused() {
        let mut s = state();
        s.focus = ModuleId::Stack;
        let r = s
            .regions(Rect::new(0, 0, 100, 40), (0, 0), 9)
            .cloned()
            .unwrap();
        assert_eq!(
            r.rect_of(ModuleId::Operations).height,
            COLLAPSED_ROWS,
            "unfocused with nine queued still folded"
        );
        assert!(!s.is_open(ModuleId::Operations));

        s.focus_set(ModuleId::Operations);
        let r = s
            .regions(Rect::new(0, 0, 100, 40), (0, 0), 9)
            .cloned()
            .unwrap();
        assert_eq!(
            r.rect_of(ModuleId::Operations).height,
            COLLAPSED_ROWS + (9u16.min(s.ops_rows) - 1),
            "focused, it opens up to ops_rows"
        );
        assert!(s.is_open(ModuleId::Operations));

        // A queue of one is already shown on the folded line: no extra.
        let mut s = state();
        s.focus_set(ModuleId::Operations);
        let r = s
            .regions(Rect::new(0, 0, 100, 40), (0, 0), 1)
            .cloned()
            .unwrap();
        assert_eq!(r.rect_of(ModuleId::Operations).height, COLLAPSED_ROWS);
    }

    /// PREVIEW open and unfocused grows to `preview_rows`; open and focused
    /// it grows to as much as half the body.
    #[test]
    fn the_preview_takes_half_the_body_when_focused() {
        let mut s = state();
        s.focus = ModuleId::Stack;
        let full = Rect::new(0, 0, 100, 60);
        let r = s.regions(full, (0, 0), 0).cloned().unwrap();
        assert_eq!(
            r.rect_of(ModuleId::Preview).height,
            COLLAPSED_ROWS + s.preview_rows,
            "unfocused and open, capped at preview_rows"
        );

        s.focus_set(ModuleId::Preview);
        let r = s.regions(full, (0, 0), 0).cloned().unwrap();
        let body_height = full.height - 1;
        assert_eq!(
            r.rect_of(ModuleId::Preview).height,
            body_height / 2,
            "focused, it takes half the body"
        );
        assert!(r.rect_of(ModuleId::Preview).height > COLLAPSED_ROWS + s.preview_rows);
    }

    /// Focusing PREVIEW opens it; folding it while it has focus lands focus
    /// on the stack.
    #[test]
    fn focusing_preview_opens_it_and_toggling_it_lands_on_the_stack() {
        let mut s = state();
        s.preview_open = false;
        s.focus_set(ModuleId::Preview);
        assert!(s.preview_open);
        assert_eq!(s.focus(), ModuleId::Preview);

        s.toggle_preview();
        assert!(!s.preview_open);
        assert_eq!(s.focus(), ModuleId::Stack);

        // Toggling it open again does not move focus.
        s.focus_set(ModuleId::Operations);
        s.toggle_preview();
        assert!(s.preview_open);
        assert_eq!(s.focus(), ModuleId::Operations);
    }

    #[test]
    fn focus_next_and_prev_cycle_the_column() {
        let mut s = state();
        assert_eq!(s.focus(), ModuleId::Stack);
        s.focus_next();
        assert_eq!(s.focus(), ModuleId::Preview);
        s.focus_next();
        assert_eq!(s.focus(), ModuleId::Operations);
        s.focus_next();
        assert_eq!(s.focus(), ModuleId::Stack);
        s.focus_prev();
        assert_eq!(s.focus(), ModuleId::Operations);
    }

    #[test]
    fn hit_testing_answers_the_module_that_was_drawn() {
        let mut s = state();
        let r = s
            .regions(Rect::new(0, 0, 100, 30), (0, 0), 3)
            .cloned()
            .unwrap();
        for m in COLUMN {
            let rect = r.rect_of(m);
            assert_eq!(r.hit(rect.x, rect.y), Some(m));
            assert_eq!(
                r.hit(rect.x + rect.width - 1, rect.y + rect.height - 1),
                Some(m)
            );
        }
        assert_eq!(
            r.hit(r.status.x, r.status.y),
            None,
            "the status is not a module"
        );
    }

    /// What the state's invariants are, stated once: focus is always one of
    /// the three modules, and a focused PREVIEW is always open -- there is no
    /// sequence of focus changes and toggles that leaves the reader looking
    /// at a fold with the keyboard behind it.
    #[derive(Debug, Clone, Copy)]
    enum Op {
        FocusSet(usize),
        FocusNext,
        FocusPrev,
        TogglePreview,
    }

    proptest! {
        #[test]
        fn focus_and_toggle_sequences_hold_the_invariants(ops in proptest::collection::vec(
            prop_oneof![
                (0usize..3).prop_map(Op::FocusSet),
                Just(Op::FocusNext),
                Just(Op::FocusPrev),
                Just(Op::TogglePreview),
            ],
            0..40,
        )) {
            let mut s = state();
            for op in ops {
                match op {
                    Op::FocusSet(i) => s.focus_set(COLUMN[i]),
                    Op::FocusNext => s.focus_next(),
                    Op::FocusPrev => s.focus_prev(),
                    Op::TogglePreview => s.toggle_preview(),
                }
                prop_assert!(COLUMN.contains(&s.focus()), "{:?} is not a module", s.focus());
                if s.focus() == ModuleId::Preview {
                    prop_assert!(s.preview_open, "a focused preview is folded");
                }
            }
        }
    }
}
