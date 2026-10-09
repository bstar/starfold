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
//! folded; open it grows by `[ui] preview_rows`, independent of focus.
//! OPERATIONS grows to show up to `[ui] ops_rows` of the queue (one entry already shows in its folded
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

/// Shared geometry for drawing and hit-testing Commander's two file panes.
pub fn pane_rect(area: Rect, pane: usize) -> Rect {
    let gap = starkit::chrome::frame::extra_rows().min(area.width);
    let width = area.width.saturating_sub(gap);
    let left = width / 2;
    if pane == 0 {
        Rect {
            width: left,
            ..area
        }
    } else {
        Rect {
            x: area.x + left + gap,
            width: width - left,
            ..area
        }
    }
}

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
    pub tabs: Option<Rect>,
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
    pub tabs_visible: bool,
    /// Pixel presentation reserves extra vertical padding for tabs.
    pub tab_rows: u16,
    /// Native rack layout uses a shared lower deck on roomy viewports.
    pub native_rack: bool,
    /// An embedded player reserves ten body rows when possible, five at
    /// the terminal floor, temporarily borrowing rows from the file list.
    pub audio_active: bool,
    /// An embedded terminal editor takes the available vertical room in
    /// Preview while it owns keyboard focus.
    pub editor_active: bool,
    /// Grow OPERATIONS while a file operation is active without taking focus.
    pub ops_active: bool,
    /// `[ui] preview_rows`: how far PREVIEW grows when it is open but not
    /// focused.
    pub preview_rows: u16,
    /// User-sized native preview; focus never changes this allocation.
    pub native_preview_rows: Option<u16>,
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
            tabs_visible: false,
            tab_rows: 1,
            native_rack: false,
            audio_active: false,
            editor_active: false,
            ops_active: false,
            preview_rows,
            native_preview_rows: None,
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
        let extra = starkit::chrome::frame::extra_rows();
        let gap = extra / 2;
        let gutters = gap * 2;
        let min_rows = MIN_ROWS + 3 * extra + gutters;
        let collapsed_rows = COLLAPSED_ROWS + extra;
        if area.width < MIN_COLS || area.height < min_rows {
            self.last = None;
            return None;
        }

        // The status line first, off the bottom, because it is never hidden,
        // never focused and never resized.
        let status_rows = if extra > 0 { 2 } else { 1 };
        let status = Rect {
            y: area.y + area.height - status_rows,
            height: status_rows,
            ..area
        };
        let tab_rows = if self.tabs_visible {
            self.tab_rows
                .clamp(1, area.height.saturating_sub(min_rows).saturating_add(1))
        } else {
            0
        };
        let tabs = self
            .tabs_visible
            .then(|| Rect::new(area.x, area.y, area.width, tab_rows));
        let body = Rect {
            y: area.y + tab_rows,
            height: area.height - status_rows - tab_rows,
            ..area
        };

        if self.native_rack
            && extra > 0
            && body.width >= 100
            && body.height >= 32
            && !self.audio_active
            && !self.editor_active
            && self.native_preview_rows != Some(u16::MAX)
        {
            // The lower modules share a row and keep their real logical
            // widths. Rendering and mouse projection use these same regions.
            let deck_floor = collapsed_rows + 2;
            let preview_rows = if self.preview_open {
                self.native_preview_rows.unwrap_or(self.preview_rows)
            } else {
                0
            };
            let deck = collapsed_rows
                .saturating_add(preview_rows)
                .max(deck_floor)
                .min(body.height.saturating_sub(STACK_MIN_ROWS + extra + gap));
            let stack = Rect::new(body.x, body.y, body.width, body.height - deck - gap);
            let deck_area = Rect::new(body.x, stack.bottom() + gap, body.width, deck);
            self.last = Some(Regions {
                area,
                modules: [stack, pane_rect(deck_area, 0), pane_rect(deck_area, 1)],
                status,
                tabs,
            });
            return self.last.as_ref();
        }

        // Everyone at their floor is the baseline; `room` is how much taller
        // than that the body is. The `MIN_ROWS` check above guarantees this
        // does not underflow.
        let stack_min = if self.audio_active || self.editor_active {
            8 + extra
        } else {
            STACK_MIN_ROWS + extra
        }
        .saturating_sub(tab_rows);
        let ops_floor = if extra > 0 { 2 } else { collapsed_rows };
        let room = body
            .height
            .saturating_sub(gutters + stack_min + collapsed_rows + ops_floor);

        // OPERATIONS only grows while focused, and then it is served first:
        // focusing it is asking to see the queue, and a queue that could not
        // open because the preview had the room was the first thing a real
        // terminal showed to be wrong.
        let ops_desired = if extra == 0 && (queued > 1 || self.ops_active) {
            // One entry of the queue already shows on the folded line.
            queued
                .max(if self.ops_active { 2 } else { 1 })
                .min(self.ops_rows)
                .saturating_sub(1)
        } else {
            0
        };
        let audio_reserved = if self.audio_active { room.min(9) } else { 0 };
        let ops_extra = ops_desired.min(room - audio_reserved);

        // PREVIEW takes what it asked for, but never more than half the room
        // while the stack has focus: at thirty rows the room is nine, and a
        // preview of ten pinned the listing to its seven-row floor, which
        // made a roomy terminal feel like the smallest one allowed.
        let preview_desired = if extra > 0 {
            if self.preview_open {
                self.native_preview_rows
                    .unwrap_or_else(|| self.preview_rows.min(room / 2))
            } else {
                0
            }
        } else if self.editor_active {
            room
        } else if self.audio_active {
            9 // 13 outer rows: border + header + ten-row player body.
        } else if !self.preview_open {
            0
        } else {
            self.native_preview_rows
                .unwrap_or_else(|| self.preview_rows.min(room / 2))
        };
        let preview_extra = preview_desired.min(room - ops_extra);

        // Whatever is left over goes to the stack.
        let stack_extra = room - preview_extra - ops_extra;

        let height = |m: ModuleId| match m {
            ModuleId::Stack => stack_min + stack_extra,
            ModuleId::Preview => collapsed_rows + preview_extra,
            ModuleId::Operations => ops_floor + ops_extra,
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
            y += h + if m == ModuleId::Operations { 0 } else { gap };
        }

        self.last = Some(Regions {
            area,
            modules,
            status,
            tabs,
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
        self.focus_set(match self.focus {
            ModuleId::Stack => ModuleId::Preview,
            ModuleId::Preview | ModuleId::Operations => ModuleId::Stack,
        });
    }

    pub fn focus_prev(&mut self) {
        self.focus_set(match self.focus {
            ModuleId::Stack | ModuleId::Operations => ModuleId::Preview,
            ModuleId::Preview => ModuleId::Stack,
        });
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
            ModuleId::Operations => {
                self.focus == ModuleId::Operations
                    || self.ops_active
                    || self.last.as_ref().is_some_and(|r| {
                        r.rect_of(ModuleId::Operations).height
                            > COLLAPSED_ROWS + starkit::chrome::frame::extra_rows()
                    })
            }
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

    #[test]
    fn native_deck_collapses_and_expanded_content_keeps_full_width() {
        let _padding = starkit::chrome::frame::padding_scope(true);
        let mut layout = LayoutState::new(10, 8, 3);
        layout.native_rack = true;
        layout.tabs_visible = true;
        layout.tab_rows = 5;
        let area = Rect::new(0, 0, 160, 60);
        let open = layout.regions(area, (0, 0), 3).unwrap().clone();
        assert_eq!(
            open.rect_of(ModuleId::Preview).y,
            open.rect_of(ModuleId::Operations).y
        );
        layout.preview_open = false;
        let closed = layout.regions(area, (0, 0), 3).unwrap().clone();
        assert!(closed.rect_of(ModuleId::Preview).height < open.rect_of(ModuleId::Preview).height);
        layout.preview_open = true;
        layout.native_preview_rows = Some(u16::MAX);
        let expanded = layout.regions(area, (0, 0), 3).unwrap();
        assert_eq!(expanded.rect_of(ModuleId::Preview).width, area.width);
        assert!(expanded.rect_of(ModuleId::Operations).y > expanded.rect_of(ModuleId::Preview).y);
        layout.native_preview_rows = None;
        layout.audio_active = true;
        assert_eq!(
            layout
                .regions(area, (0, 0), 3)
                .unwrap()
                .rect_of(ModuleId::Preview)
                .width,
            area.width
        );
        layout.audio_active = false;
        layout.editor_active = true;
        assert_eq!(
            layout
                .regions(area, (0, 0), 3)
                .unwrap()
                .rect_of(ModuleId::Preview)
                .width,
            area.width
        );
    }
    use proptest::prelude::*;

    fn state() -> LayoutState {
        LayoutState::new(10, 6, 6)
    }

    #[test]
    fn native_preview_focus_never_changes_height_and_manual_size_survives_clamping() {
        let _padding = starkit::chrome::frame::padding_scope(true);
        let mut layout = state();
        for rows in [None, Some(2), Some(20), Some(500)] {
            layout.native_preview_rows = rows;
            for height in [40, 60, 100] {
                let area = Rect::new(0, 0, 120, height);
                layout.focus_set(ModuleId::Stack);
                let before = layout.regions(area, (0, 0), 0).unwrap().clone();
                layout.focus_set(ModuleId::Preview);
                assert_eq!(&before, layout.regions(area, (0, 0), 0).unwrap());
                assert_eq!(layout.native_preview_rows, rows);
            }
        }
    }

    #[test]
    fn native_operations_do_not_resize_panes_as_work_starts_or_finishes() {
        let _chrome = starkit::chrome::frame::padding_scope(true);
        let mut layout = state();
        let area = Rect::new(0, 0, 160, 60);
        let idle = layout.regions(area, (1, 0), 0).cloned().unwrap();
        for (active, count) in [(true, 1), (true, 12), (false, 12), (false, 0)] {
            layout.ops_active = active;
            assert_eq!(layout.regions(area, (1, 0), count).unwrap(), &idle);
        }
        layout.focus_set(ModuleId::Operations);
        assert_eq!(layout.regions(area, (1, 0), 12).unwrap(), &idle);
    }

    #[test]
    fn native_gutters_separate_panes_and_do_not_capture_clicks() {
        let _chrome = starkit::chrome::frame::padding_scope(true);
        let mut layout = state();
        let regions = layout
            .regions(Rect::new(0, 0, 100, MIN_ROWS + 8), (0, 0), 0)
            .cloned()
            .unwrap();
        let stack = regions.rect_of(ModuleId::Stack);
        let preview = regions.rect_of(ModuleId::Preview);
        let operations = regions.rect_of(ModuleId::Operations);
        assert_eq!(preview.y, stack.bottom() + 1);
        assert_eq!(operations.y, preview.bottom() + 1);
        assert_eq!(operations.bottom(), regions.status.y);
        assert_eq!(regions.hit(stack.x, stack.bottom()), None);
        let left = pane_rect(stack, 0);
        let right = pane_rect(stack, 1);
        assert_eq!(right.x, left.right() + 2);
        assert_eq!(right.right(), stack.right());
        assert!(!left.contains((left.right(), left.y).into()));
        assert!(!right.contains((left.right(), left.y).into()));
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

    #[test]
    fn operations_allocation_is_independent_of_focus() {
        let mut s = state();
        let area = Rect::new(0, 0, 100, 40);
        let original = s.regions(area, (0, 0), 9).unwrap().clone();
        assert!(s.is_open(ModuleId::Operations));
        for focus in COLUMN {
            s.focus_set(focus);
            let r = s.regions(area, (0, 0), 9).unwrap();
            for pane in COLUMN {
                assert_eq!(r.rect_of(pane), original.rect_of(pane));
            }
        }
    }

    #[test]
    fn active_work_expands_operations_without_changing_focus() {
        let mut s = state();
        s.ops_active = true;
        let area = Rect::new(0, 0, 100, 30);
        let r = s.regions(area, (0, 0), 1).cloned().unwrap();
        assert_eq!(s.focus(), ModuleId::Stack);
        assert!(s.is_open(ModuleId::Operations));
        assert!(r.rect_of(ModuleId::Operations).height > COLLAPSED_ROWS);
        s.ops_active = false;
        let r = s.regions(area, (0, 0), 1).cloned().unwrap();
        assert_eq!(r.rect_of(ModuleId::Operations).height, COLLAPSED_ROWS);
        s.ops_active = true;
        let r = s
            .regions(Rect::new(0, 0, MIN_COLS, MIN_ROWS), (0, 0), 1)
            .cloned()
            .unwrap();
        assert_eq!(r.rect_of(ModuleId::Operations).height, COLLAPSED_ROWS);
    }

    /// At thirty rows the room is nine: the preview gets four of it, the
    /// stack the other five, and a focused queue of three opens fully.
    #[test]
    fn a_thirty_row_terminal_shares_the_room_between_the_stack_and_the_preview() {
        let mut s = state();
        s.focus = ModuleId::Stack;
        let full = Rect::new(0, 0, 100, 30);
        let r = s.regions(full, (0, 0), 0).cloned().unwrap();
        assert_eq!(r.rect_of(ModuleId::Preview).height, COLLAPSED_ROWS + 4);
        assert_eq!(r.rect_of(ModuleId::Stack).height, STACK_MIN_ROWS + 5);

        s.focus_set(ModuleId::Operations);
        let r = s.regions(full, (0, 0), 3).cloned().unwrap();
        assert_eq!(
            r.rect_of(ModuleId::Operations).height,
            COLLAPSED_ROWS + 2,
            "a focused queue opens before the preview is served"
        );
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
        let body_height = COLLAPSED_ROWS + s.preview_rows;
        assert_eq!(
            r.rect_of(ModuleId::Preview).height,
            body_height,
            "focus preserves the allocated preview height"
        );
        assert_eq!(
            r.rect_of(ModuleId::Preview).height,
            COLLAPSED_ROWS + s.preview_rows
        );
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
    fn focus_next_and_prev_cycle_browser_and_preview() {
        let mut s = state();
        assert_eq!(s.focus(), ModuleId::Stack);
        s.focus_next();
        assert_eq!(s.focus(), ModuleId::Preview);
        s.focus_next();
        assert_eq!(s.focus(), ModuleId::Stack);
        s.focus_prev();
        assert_eq!(s.focus(), ModuleId::Preview);
        s.focus_prev();
        assert_eq!(s.focus(), ModuleId::Stack);
        s.focus_set(ModuleId::Operations);
        s.focus_next();
        assert_eq!(s.focus(), ModuleId::Stack);
        s.focus_set(ModuleId::Operations);
        s.focus_prev();
        assert_eq!(s.focus(), ModuleId::Preview);
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

    #[test]
    fn audio_reserves_compact_or_full_body_without_hiding_browser() {
        let mut s = state();
        s.audio_active = true;
        for focus in COLUMN {
            s.focus_set(focus);
            for (height, body_height) in [(21, 5), (26, 10), (40, 10)] {
                let r = s.regions(Rect::new(0, 0, 60, height), (0, 0), 100).unwrap();
                assert_eq!(
                    header::body(r.rect_of(ModuleId::Preview)).height,
                    body_height
                );
                assert!(r.rect_of(ModuleId::Stack).height >= 8);
                assert!(r.rect_of(ModuleId::Operations).height >= COLLAPSED_ROWS);
                assert_eq!(r.status.bottom(), height);
            }
        }
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
