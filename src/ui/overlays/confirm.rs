//! "Are you sure?" -- three shapes of it: deleting for good, dropping the
//! queue, and stopping something that is running (plus quitting while it
//! still is).
//!
//! Modelled on STAR/CORD's `overlays::confirm`: raw keys, not the key table,
//! because while this is open the keyboard means exactly one thing. Unlike
//! STAR/CORD's dialogue, which always answers "yes" to "delete" on the same
//! button, STAR/FOLD's four questions do not all want the same word on them --
//! clearing a queue is not deleting a file, and stopping a copy is not either
//! -- so `yes`/`no` are carried on the struct instead of fixed in the widget.
//!
//! [`layout`] is the one computation both [`render`] and
//! [`super::Overlays::click`] read, so the box a person sees and the box a
//! click is tested against can never drift apart.

use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Modifier, Style};
use starkit::ratatui::text::Span;
use starkit::ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};
use starkit::wrap;

use crate::fold::ops::OpId;
use crate::ui::panels::{fit, rgb, width_of};
use crate::ui::theme::Theme;

use super::Pending;

/// A question, what its two answers are called, and what saying yes leads to.
#[derive(Debug)]
pub struct Confirm {
    pub title: String,
    /// Logical lines; [`layout`] wraps each one to the box's width, so a
    /// caller writes a sentence and never a column count.
    pub body: Vec<String>,
    /// The word on the `y` answer: "delete", "clear", "stop", "quit".
    pub yes: &'static str,
    /// The word on the `n` answer: "keep", "continue", "stay" -- whatever
    /// refusing this particular question actually means.
    pub no: &'static str,
    pub pending: Pending,
}

impl Confirm {
    /// No trash to fall back on, or the trash was bypassed on purpose --
    /// either way, this is the last confirmation before the bytes are gone.
    pub fn delete_permanently(op: OpId, n_items: usize) -> Self {
        let noun = if n_items == 1 { "item" } else { "items" };
        Self {
            title: "delete permanently".into(),
            body: vec![
                format!("{n_items} {noun} will be permanently deleted."),
                "This cannot be undone.".into(),
            ],
            yes: "delete",
            no: "keep",
            pending: Pending::DeletePermanently(op),
        }
    }

    /// `esc` on the OPERATIONS module already drops everything that has not
    /// started without asking -- this is for the day that stops being true.
    pub fn clear_queue(n: usize) -> Self {
        let noun = if n == 1 { "operation" } else { "operations" };
        Self {
            title: "clear the queue".into(),
            body: vec![format!("{n} queued {noun} will be dropped.")],
            yes: "clear",
            no: "keep",
            pending: Pending::ClearQueue,
        }
    }

    /// `title` is the queued op's own [`crate::fold::ops::Op::title`] --
    /// "COPY 14 items → ~/Archive" -- so the question names the thing being
    /// stopped rather than just saying "the running operation".
    pub fn cancel_running(op: OpId, title: &str) -> Self {
        Self {
            title: "stop the running operation".into(),
            body: vec![format!("{title} is still running.")],
            yes: "stop",
            no: "continue",
            pending: Pending::CancelRunning(op),
        }
    }

    /// Quitting while an op is in flight stops it rather than orphaning it --
    /// there is no daemon to hand it off to -- and that is worth a question
    /// of its own rather than folding it into `cancel_running`.
    pub fn quit_with_running(title: &str) -> Self {
        Self {
            title: "quit".into(),
            body: vec![
                format!("{title} is still running."),
                "It will be stopped.".into(),
            ],
            yes: "quit",
            no: "stay",
            pending: Pending::Quit,
        }
    }
}

/// Where the box lands, the body already wrapped, and where the two words on
/// the footer are -- everything [`render`] draws and a click is tested
/// against, worked out once so the two cannot disagree.
pub(super) struct Layout {
    pub rect: Rect,
    pub inner: Rect,
    pub body_rows: Vec<String>,
    pub footer_y: u16,
    /// `[start, end)` columns of the `yes` word.
    pub yes: (u16, u16),
    /// `[start, end)` columns of the `no` word.
    pub no: (u16, u16),
}

pub(super) fn layout(area: Rect, c: &Confirm) -> Option<Layout> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let w = area.width.saturating_sub(4).clamp(24, 56).min(area.width);
    let inner_w = w.saturating_sub(2);
    let mut body_rows = Vec::new();
    for line in &c.body {
        for row in wrap::wrap(line, inner_w) {
            body_rows.push(row.drawn(line).to_string());
        }
    }
    // Two borders, every wrapped body row, and the footer row that carries
    // the two answers.
    let h = (body_rows.len() as u16 + 3).clamp(4, area.height);
    if w < 8 || h < 4 {
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
    let yes_label = format!("[y] {}", c.yes);
    let no_label = format!("[n] {}", c.no);
    let yes = (inner.x, inner.x + width_of(&yes_label));
    let no = (yes.1 + 3, yes.1 + 3 + width_of(&no_label));
    Some(Layout {
        rect,
        inner,
        body_rows,
        footer_y,
        yes,
        no,
    })
}

pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, c: &Confirm) {
    let Some(l) = layout(area, c) else {
        return;
    };
    Clear.render(l.rect, buf);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(rgb(theme.border_focused)))
        .title(Span::styled(
            format!("{}{} ", starkit::chrome::frame::TITLE_LEAD, c.title),
            Style::default()
                .fg(rgb(theme.header_fg))
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(rgb(theme.panel_bg)));
    block.render(l.rect, buf);
    starkit::chrome::frame::render_corners(l.rect, buf, theme, true);

    for (i, row) in l.body_rows.iter().enumerate() {
        let y = l.inner.y + i as u16;
        if y >= l.footer_y {
            break;
        }
        buf.set_string(
            l.inner.x,
            y,
            fit(row, l.inner.width),
            Style::default().fg(rgb(theme.fg)),
        );
    }

    let footer = format!("[y] {}   [n] {}", c.yes, c.no);
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

    #[test]
    fn a_single_item_reads_as_singular() {
        let c = Confirm::delete_permanently(OpId(1), 1);
        assert!(c.body[0].contains("1 item "), "{:?}", c.body);
        let c = Confirm::delete_permanently(OpId(1), 3);
        assert!(c.body[0].contains("3 items "), "{:?}", c.body);
    }

    #[test]
    fn each_question_names_its_own_answers() {
        assert_eq!(Confirm::delete_permanently(OpId(1), 1).yes, "delete");
        assert_eq!(Confirm::clear_queue(2).yes, "clear");
        assert_eq!(Confirm::cancel_running(OpId(1), "COPY 1 item").yes, "stop");
        assert_eq!(Confirm::quit_with_running("COPY 1 item").yes, "quit");
    }

    #[test]
    fn the_box_draws_its_title_and_body() {
        let t = theme("terminal");
        let area = Rect::new(0, 0, 60, 21);
        let mut buf = Buffer::empty(area);
        let c = Confirm::delete_permanently(OpId(1), 4);
        render(area, &mut buf, &t, &c);
        let text: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("delete permanently"), "{text}");
        assert!(text.contains("4 items"), "{text}");
        assert!(text.contains("[y] delete"), "{text}");
        assert!(text.contains("[n] keep"), "{text}");
    }

    #[test]
    fn a_long_body_wraps_rather_than_overruns_the_box() {
        let long = "x ".repeat(80);
        let c = Confirm {
            title: "t".into(),
            body: vec![long],
            yes: "y",
            no: "n",
            pending: Pending::ClearQueue,
        };
        let area = Rect::new(0, 0, 60, 21);
        let l = layout(area, &c).expect("it fits");
        assert!(l.body_rows.len() > 1, "a long line should wrap");
        for row in &l.body_rows {
            assert!(width_of(row) <= l.inner.width);
        }
    }
}
