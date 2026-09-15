//! What to do about files already at the destination.
//!
//! [`crate::fold::ops::ConflictPolicy`] is a property of an [`crate::fold::
//! ops::Op`], not of one file in it -- overwriting one collision and skipping
//! the next inside the same copy is not a choice this dialogue offers, and
//! [`Action::Policy`] is answered once for the whole queued operation. `j`/`k`
//! only move the reading cursor over the list of what collided; the three
//! real answers are `o`/`s`/`r`, the initials of the words on the footer, so
//! a hand that has already found those letters in the key table does not
//! have to relearn them here.
//!
//! The box is STAR/KIT's `chrome::overlay` now, the same shape every other
//! overlay opens in, with the count of what collided in the title's own
//! detail (`"conflicts — 3 already exist"`) rather than baked into the title
//! string, and the four answers moved from a body row onto the bottom
//! border's footer, where the rest of this crate's chrome keeps its own key
//! hints. [`layout`] is, as before, the one computation [`render`] and
//! [`super::Overlays::click`] both read.

use starkit::chrome::overlay::{self, Anchor};
use starkit::chrome::scrollbar;
use starkit::crossterm::event::{KeyCode, KeyEvent};
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;

use crate::fold::ops::{Conflict, ConflictPolicy, OpId};
use crate::ui::panels::{fit, rgb};
use crate::ui::theme::Theme;
use crate::ui::{Bar, Bars};

/// What a key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Taken,
    Close,
    /// Applies to every conflict in [`Prompt::conflicts`] at once.
    Policy(ConflictPolicy),
}

/// The conflicts one op's plan found, and where the reading cursor is.
#[derive(Debug)]
pub struct Prompt {
    pub op: OpId,
    pub conflicts: Vec<Conflict>,
    pub cursor: usize,
    pub scroll: usize,
}

impl Prompt {
    pub fn new(op: OpId, conflicts: Vec<Conflict>) -> Self {
        Self {
            op,
            conflicts,
            cursor: 0,
            scroll: 0,
        }
    }

    pub fn handle(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('o') => Action::Policy(ConflictPolicy::Overwrite),
            KeyCode::Char('s') => Action::Policy(ConflictPolicy::Skip),
            KeyCode::Char('r') => Action::Policy(ConflictPolicy::RenameNew),
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_cursor(1);
                Action::Taken
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_cursor(-1);
                Action::Taken
            }
            _ => Action::Taken,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.conflicts.is_empty() {
            return;
        }
        let last = (self.conflicts.len() - 1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }
}

/// The footer's four answers, read out in the order they appear left to
/// right -- [`layout`] walks this once to place each word's clickable span,
/// and [`render`] joins the same words with STAR/KIT's own middle dot to draw
/// the border's footer back.
const WORDS: [&str; 4] = ["o overwrite", "s skip", "r rename new", "esc leave queued"];

/// The footer text, exactly as [`overlay::render`] draws it wrapped in the
/// bottom border -- a function rather than a constant because [`layout`]
/// needs the same string [`render`] hands the frame, so the two cannot drift.
fn footer_text() -> String {
    WORDS.join(" \u{b7} ")
}

pub(super) struct Layout {
    pub rect: Rect,
    pub inner: Rect,
    /// The rows of `inner` the list actually gets -- every one of them, since
    /// an overlay's footer lives on the border rather than a body row.
    pub rows_visible: u16,
    pub footer_y: u16,
    /// Whether the footer actually fits `rect`'s width -- [`frame::frame`]
    /// drops a footer whole rather than clip it (see its own doc), so a box
    /// too narrow for all four answers draws none of them, and the spans
    /// below must agree there is nothing there to click.
    pub footer_fits: bool,
    pub o: (u16, u16),
    pub s: (u16, u16),
    pub r: (u16, u16),
    pub esc: (u16, u16),
}

/// Where the box lands, sized to the list: `None` when `area` cannot fit even
/// the smallest box this dialogue draws. Mirrors
/// [`starkit::chrome::confirm::layout`]'s own two-pass sizing -- a probe rect
/// picks the width, then the real rect is asked for with the height the list
/// actually needs.
pub(super) fn layout(area: Rect, p: &Prompt) -> Option<Layout> {
    let rows = p.conflicts.len().max(1) as u16;
    let want_h = rows + 2;
    let rect = overlay::rect(area, (30, 70), want_h, 5, Anchor::Centre);
    if rect.height < 5 {
        return None;
    }
    let inner = overlay::inner(rect);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let rows_visible = inner.height;
    let footer_y = rect.y + rect.height - 1;

    let drawn = format!(" {} ", footer_text());
    let drawn_w = starkit::wrap::width_of(&drawn);
    let room = rect.width.saturating_sub(2);
    let footer_fits = drawn_w <= room;
    let end_x = rect.x + rect.width.saturating_sub(2);
    let drawn_start = (end_x + 1).saturating_sub(drawn_w);
    let footer_start = drawn_start + 1; // past the frame's own leading space

    let sep_w = starkit::wrap::width_of(" \u{b7} ");
    let mut x = footer_start;
    let mut spans = [(0u16, 0u16); 4];
    for (i, word) in WORDS.iter().enumerate() {
        let start = x;
        let end = x + starkit::wrap::width_of(word);
        spans[i] = (start, end);
        x = end + sep_w;
    }

    Some(Layout {
        rect,
        inner,
        rows_visible,
        footer_y,
        footer_fits,
        o: spans[0],
        s: spans[1],
        r: spans[2],
        esc: spans[3],
    })
}

pub(super) fn hit_footer(l: &Layout, x: u16, y: u16) -> Option<ConflictPolicy> {
    if !l.footer_fits || y != l.footer_y {
        return None;
    }
    if x >= l.o.0 && x < l.o.1 {
        Some(ConflictPolicy::Overwrite)
    } else if x >= l.s.0 && x < l.s.1 {
        Some(ConflictPolicy::Skip)
    } else if x >= l.r.0 && x < l.r.1 {
        Some(ConflictPolicy::RenameNew)
    } else {
        None
    }
}

pub(super) fn hit_esc(l: &Layout, x: u16, y: u16) -> bool {
    l.footer_fits && y == l.footer_y && x >= l.esc.0 && x < l.esc.1
}

/// Which row in [`Prompt::conflicts`] a click landed on, accounting for
/// [`Prompt::scroll`]. `None` outside the list, or past the end of what is
/// actually there.
pub(super) fn hit_row(l: &Layout, x: u16, y: u16, p: &Prompt) -> Option<usize> {
    if x < l.inner.x || x >= l.inner.x + l.inner.width {
        return None;
    }
    if y < l.inner.y || y >= l.inner.y + l.rows_visible {
        return None;
    }
    let row = p.scroll + usize::from(y - l.inner.y);
    (row < p.conflicts.len()).then_some(row)
}

pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, p: &Prompt, bars: &mut Bars) {
    let Some(l) = layout(area, p) else {
        return;
    };

    let detail = format!("{} already exist", p.conflicts.len());
    let footer = footer_text();
    // The core theme type -- a struct literal is not a coercion site, so the
    // deref from this crate's own `Theme` is spelled out here.
    let core: &starkit::theme::Theme = theme;
    overlay::render(
        l.rect,
        buf,
        &overlay::Overlay {
            theme: core,
            title: "conflicts",
            detail: Some(&detail),
            footer: Some(&footer),
        },
    );

    for (i, conflict) in p
        .conflicts
        .iter()
        .enumerate()
        .skip(p.scroll)
        .take(usize::from(l.rows_visible))
    {
        let y = l.inner.y + (i - p.scroll) as u16;
        let name = conflict
            .dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // The name is what collided; the prompt's title already says where.
        // A directory says so, because "overwrite" means "merge into" for
        // one and "replace" for a file, and the answer is the same key.
        let text = if conflict.both_dirs {
            format!("{name}/  (directory)")
        } else {
            name
        };
        let style = if i == p.cursor {
            Style::default()
                .fg(rgb(theme.row_cursor_fg))
                .bg(rgb(theme.row_cursor_bg))
        } else {
            Style::default().fg(rgb(theme.fg))
        };
        buf.set_string(l.inner.x, y, fit(&text, l.inner.width), style);
    }

    // The reading cursor's position in the list, on the box's right border --
    // drawn after the frame and its corners, over the rows the list actually
    // occupies.
    let list = Rect {
        x: l.inner.x,
        y: l.inner.y,
        width: l.inner.width,
        height: l.rows_visible,
    };
    let track = scrollbar::track(l.rect, list);
    bars.draw(
        Bar::Conflict,
        track,
        buf,
        theme,
        p.conflicts.len() as u32,
        p.scroll as u32,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::tests_support::theme;
    use starkit::crossterm::event::KeyModifiers;
    use std::path::PathBuf;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn conflict(name: &str) -> Conflict {
        Conflict {
            source: PathBuf::from("/src").join(name),
            dest: PathBuf::from("/dest").join(name),
            both_dirs: false,
        }
    }

    #[test]
    fn o_s_r_answer_with_the_matching_policy() {
        let mut p = Prompt::new(OpId(1), vec![conflict("a")]);
        assert_eq!(
            p.handle(key('o')),
            Action::Policy(ConflictPolicy::Overwrite)
        );
        assert_eq!(p.handle(key('s')), Action::Policy(ConflictPolicy::Skip));
        assert_eq!(
            p.handle(key('r')),
            Action::Policy(ConflictPolicy::RenameNew)
        );
    }

    #[test]
    fn j_and_k_move_the_reading_cursor_without_answering() {
        let mut p = Prompt::new(OpId(1), vec![conflict("a"), conflict("b"), conflict("c")]);
        assert_eq!(p.handle(key('j')), Action::Taken);
        assert_eq!(p.cursor, 1);
        assert_eq!(p.handle(key('k')), Action::Taken);
        assert_eq!(p.cursor, 0);
        // Does not run off either end.
        assert_eq!(p.handle(key('k')), Action::Taken);
        assert_eq!(p.cursor, 0);
    }

    #[test]
    fn the_box_draws_its_title_and_the_rows() {
        let t = theme("terminal");
        // Wide enough that the footer's four answers fit -- see
        // `a_narrow_box_drops_the_footer_whole_rather_than_clip_it` for the
        // case where they do not.
        let area = Rect::new(0, 0, 100, 21);
        let mut buf = Buffer::empty(area);
        let p = Prompt::new(OpId(1), vec![conflict("a.txt"), conflict("b.txt")]);
        render(area, &mut buf, &t, &p, &mut Bars::new());
        let text: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("CONFLICTS"), "{text}");
        assert!(text.contains("2 already exist"), "{text}");
        assert!(text.contains("a.txt"), "{text}");
        assert!(text.contains("overwrite"), "{text}");
    }

    #[test]
    fn a_click_on_a_footer_word_answers_its_policy() {
        let area = Rect::new(0, 0, 100, 21);
        let p = Prompt::new(OpId(1), vec![conflict("a.txt")]);
        let l = layout(area, &p).unwrap();
        assert!(l.footer_fits, "the footer should fit at this width");
        assert_eq!(
            hit_footer(&l, l.o.0, l.footer_y),
            Some(ConflictPolicy::Overwrite)
        );
        assert_eq!(
            hit_footer(&l, l.s.0, l.footer_y),
            Some(ConflictPolicy::Skip)
        );
        assert_eq!(
            hit_footer(&l, l.r.0, l.footer_y),
            Some(ConflictPolicy::RenameNew)
        );
        assert!(hit_esc(&l, l.esc.0, l.footer_y));
        assert_eq!(hit_footer(&l, l.rect.x, l.rect.y), None);
    }

    /// `chrome::frame` drops a too-wide footer whole rather than clip it, so
    /// a box narrow enough to lose the footer must also lose its clickable
    /// spans -- otherwise a click could answer a policy nothing on screen
    /// named.
    #[test]
    fn a_narrow_box_drops_the_footer_and_its_clickable_spans_together() {
        let area = Rect::new(0, 0, 60, 21);
        let p = Prompt::new(OpId(1), vec![conflict("a.txt")]);
        let l = layout(area, &p).unwrap();
        assert!(!l.footer_fits, "the footer should not fit at this width");

        let text: String = {
            let t = theme("terminal");
            let mut buf = Buffer::empty(area);
            render(area, &mut buf, &t, &p, &mut Bars::new());
            (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(!text.contains("overwrite"), "{text}");

        for x in l.rect.x..l.rect.x + l.rect.width {
            assert_eq!(hit_footer(&l, x, l.footer_y), None);
            assert!(!hit_esc(&l, x, l.footer_y));
        }
    }
}
