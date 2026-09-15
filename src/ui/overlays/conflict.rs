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
//! [`layout`] is, as in [`super::confirm`], the one computation [`render`]
//! and [`super::Overlays::click`] both read.

use starkit::crossterm::event::{KeyCode, KeyEvent};
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Modifier, Style};
use starkit::ratatui::text::Span;
use starkit::ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

use crate::fold::ops::{Conflict, ConflictPolicy, OpId};
use crate::ui::panels::{fit, rgb, width_of};
use crate::ui::theme::Theme;

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
/// right -- `layout` walks this once to place each word, and `render` joins
/// it with two spaces to draw the same line back.
const WORDS: [&str; 4] = [
    "[o] overwrite",
    "[s] skip",
    "[r] rename new",
    "[esc] leave queued",
];

pub(super) struct Layout {
    pub rect: Rect,
    pub inner: Rect,
    pub footer_y: u16,
    pub o: (u16, u16),
    pub s: (u16, u16),
    pub r: (u16, u16),
    pub esc: (u16, u16),
}

pub(super) fn layout(area: Rect, p: &Prompt) -> Option<Layout> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let w = area.width.saturating_sub(4).clamp(30, 70).min(area.width);
    let rows = p.conflicts.len().max(1) as u16;
    // Two borders, one row per conflict (or one, empty, if there somehow are
    // none), and the footer.
    let h = (rows + 3).clamp(5, area.height);
    if w < 12 || h < 5 {
        return None;
    }
    let rect = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    let inner = Rect {
        x: rect.x + 1,
        y: rect.y + 1,
        width: rect.width.saturating_sub(2),
        height: rect.height.saturating_sub(2),
    };
    if inner.height == 0 {
        return None;
    }
    let footer_y = inner.y + inner.height - 1;
    let mut x = inner.x;
    let mut spans = [(0u16, 0u16); 4];
    for (i, word) in WORDS.iter().enumerate() {
        let start = x;
        let end = x + width_of(word);
        spans[i] = (start, end);
        x = end + 2;
    }
    Some(Layout {
        rect,
        inner,
        footer_y,
        o: spans[0],
        s: spans[1],
        r: spans[2],
        esc: spans[3],
    })
}

pub(super) fn hit_footer(l: &Layout, x: u16, y: u16) -> Option<ConflictPolicy> {
    if y != l.footer_y {
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
    y == l.footer_y && x >= l.esc.0 && x < l.esc.1
}

/// Which row in [`Prompt::conflicts`] a click landed on, accounting for
/// [`Prompt::scroll`]. `None` outside the list, below it, or past the end of
/// what is actually there.
pub(super) fn hit_row(l: &Layout, x: u16, y: u16, p: &Prompt) -> Option<usize> {
    if x < l.inner.x || x >= l.inner.x + l.inner.width {
        return None;
    }
    if y < l.inner.y || y >= l.footer_y {
        return None;
    }
    let row = p.scroll + usize::from(y - l.inner.y);
    (row < p.conflicts.len()).then_some(row)
}

pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, p: &Prompt) {
    let Some(l) = layout(area, p) else {
        return;
    };
    Clear.render(l.rect, buf);

    let title = format!("CONFLICTS \u{2014} {} already exist", p.conflicts.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(rgb(theme.border_focused)))
        .title(Span::styled(
            format!("{}{title} ", starkit::chrome::frame::TITLE_LEAD),
            Style::default()
                .fg(rgb(theme.header_fg))
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(rgb(theme.panel_bg)));
    block.render(l.rect, buf);
    starkit::chrome::frame::render_corners(l.rect, buf, theme, true);

    let rows_visible = usize::from(l.footer_y.saturating_sub(l.inner.y));
    for (i, conflict) in p
        .conflicts
        .iter()
        .enumerate()
        .skip(p.scroll)
        .take(rows_visible)
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

    let footer = WORDS.join("  ");
    buf.set_string(
        l.inner.x,
        l.footer_y,
        fit(&footer, l.inner.width),
        Style::default().fg(rgb(theme.dim)),
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
        let area = Rect::new(0, 0, 60, 21);
        let mut buf = Buffer::empty(area);
        let p = Prompt::new(OpId(1), vec![conflict("a.txt"), conflict("b.txt")]);
        render(area, &mut buf, &t, &p);
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
}
