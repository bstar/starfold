//! "Are you sure?" -- three shapes of it: deleting for good, dropping the
//! queue, and stopping something that is running (plus quitting while it
//! still is).
//!
//! The box itself is STAR/KIT's `chrome::confirm` now -- raw keys, not the
//! key table, because while this is open the keyboard means exactly one
//! thing, and `y`/`n` rather than `Enter` for the same reason the shared
//! widget documents: a dialogue whose default key is the one a reader's
//! thumb is already resting on is a dialogue that answers a keystroke meant
//! for whatever came before it. Unlike a dialogue that always answers "yes"
//! on the same button, STAR/FOLD's four questions do not all want the same
//! word on them -- clearing a queue is not deleting a file, and stopping a
//! copy is not either -- so `yes`/`no` are carried on this struct, the same
//! as the shared one underneath it.
//!
//! This module keeps only what is STAR/FOLD's business: the four questions
//! themselves and the [`Pending`] each answers into. [`layout`] and
//! [`render`] are thin covers over `chrome::confirm`'s own, so the box a
//! person sees and the box [`super::Overlays::click`] tests against can
//! never drift from what the shared widget actually draws.

use starkit::chrome::confirm;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;

use crate::fold::ops::OpId;
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

/// This question, as the shared widget spells it: a title lower-cased here
/// (`chrome::frame` capitals every panel's own title, this one included) and
/// the body and answers copied across as they are.
fn kit(c: &Confirm) -> confirm::Confirm {
    confirm::Confirm {
        title: c.title.clone(),
        body: c.body.clone(),
        yes: c.yes,
        no: c.no,
    }
}

/// Where the box lands and where its two answers sit, so a click can be
/// tested against the same geometry [`render`] draws -- both read straight
/// through to `chrome::confirm`'s own, so the two can never disagree.
pub(super) fn layout(area: Rect, c: &Confirm) -> Option<confirm::Layout> {
    confirm::layout(area, &kit(c))
}

pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, c: &Confirm) {
    confirm::render(area, buf, theme, &kit(c));
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
        assert!(text.contains("DELETE PERMANENTLY"), "{text}");
        assert!(text.contains("4 items"), "{text}");
        assert!(text.contains("y delete"), "{text}");
        assert!(text.contains("n keep"), "{text}");
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
            assert!(starkit::wrap::width_of(row) < l.inner.width);
        }
    }
}
