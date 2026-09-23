//! What is drawn over the column: help, a confirmation, a rename, a
//! conflict.
//!
//! One rule holds the four together, copied from STAR/CORD's own overlays
//! module: **only one is ever open**, and while one is, it takes every key --
//! a dialogue drawn over a panel that lets a key through to the panel
//! underneath is a dialogue you can type through, which is the bug keeping
//! them behind [`Overlays`] makes impossible to write. `esc` always closes
//! whichever one is open rather than doing anything panel-specific, and
//! `ctrl+c` quits from inside any of them, the same as it does everywhere
//! else -- a modal box is not a place the one true way out should stop
//! working.
//!
//! [`Overlays::handle`] and [`Overlays::click`] are the only entry points a
//! caller needs: the app's key and mouse dispatch check
//! [`Overlays::is_open`] first (as STAR/CORD's `Overlays::open` is checked
//! first in both of its own dispatchers) and, while it answers true, hand
//! every key and click here instead of to the focused module.
//!
//! Closing an overlay can change what is behind it -- a rename that just
//! renamed the very entry the cursor was on, say -- so the caller repaints
//! the whole frame after any [`Answer`] that is not [`Answer::Consumed`],
//! the way STAR/CORD's `repaint` flag does. Nothing in this module needs to
//! know that; it only ever draws what is open right now.

pub mod confirm;
pub mod conflict;
pub mod context;
pub mod rename;

use std::path::PathBuf;

use starkit::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use starkit::keymap::{help_rect, HelpView};
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::widgets::Widget;

use crate::fold::ops::{ConflictPolicy, OpId};
use crate::ui::keymap::{BINDINGS, MOUSE};
use crate::ui::theme::Theme;
use crate::ui::Bars;

/// What a [`confirm::Confirm`] is asking about, carried through unopened so
/// the caller learns it again only once the answer is yes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    DeletePermanently(OpId),
    ClearQueue,
    CancelRunning(OpId),
    Quit,
}

/// The one overlay that may be open, and what it needs to keep drawing
/// itself between frames.
#[derive(Debug)]
pub enum Overlay {
    Context(context::Menu),
    Destination(context::Destination),
    Help { scroll: u16 },
    Confirm(confirm::Confirm),
    Rename(rename::Rename),
    Conflict(conflict::Prompt),
}

/// Modal things drawn over everything else. See the module doc for the one
/// rule they share.
#[derive(Debug, Default)]
pub struct Overlays {
    current: Option<Overlay>,
}

/// What handling a key or a click did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Context(context::Target, context::Action),
    Operation(context::Request),
    /// The overlay took the key or click; nothing else happened.
    Consumed,
    /// The overlay closed without deciding anything -- `n`, `esc`, a click
    /// outside the box.
    Closed,
    /// `y`/`enter` on a [`confirm::Confirm`].
    Confirmed(Pending),
    /// A [`rename::Rename`] was submitted with a name that passed
    /// validation.
    Renamed {
        from: PathBuf,
        to: PathBuf,
    },
    /// `o`/`s`/`r` on a [`conflict::Prompt`], applied to the whole op.
    Policy {
        op: OpId,
        policy: ConflictPolicy,
    },
    /// `ctrl+c`, which quits from inside an overlay the same as it does
    /// everywhere else.
    Quit,
}

impl Overlays {
    pub fn new() -> Self {
        Self { current: None }
    }

    /// Checked first by both the key and the mouse dispatch, so a key or a
    /// click reaches an open overlay rather than the panel under it.
    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    pub fn current(&self) -> Option<&Overlay> {
        self.current.as_ref()
    }

    /// The same overlay, mutably -- for a dragged scrollbar to move the one
    /// piece of it ([`conflict::Prompt::scroll`]) a mouse event ever touches
    /// straight, rather than through [`Answer`].
    pub fn current_mut(&mut self) -> Option<&mut Overlay> {
        self.current.as_mut()
    }

    /// Opening any overlay replaces whatever was open; see the module doc
    /// for why there is never more than one.
    pub fn open_help(&mut self) {
        self.current = Some(Overlay::Help { scroll: 0 });
    }

    pub fn open_confirm(&mut self, c: confirm::Confirm) {
        self.current = Some(Overlay::Confirm(c));
    }

    pub fn open_context(&mut self, target: context::Target, anchor: (u16, u16)) {
        self.current = Some(Overlay::Context(context::Menu::new(target, anchor)));
    }
    pub fn open_destination(&mut self, request: context::Request) {
        self.current = Some(Overlay::Destination(context::Destination::new(request)));
    }

    pub fn open_rename(&mut self, from: PathBuf) {
        self.current = Some(Overlay::Rename(rename::Rename::new(from)));
    }

    pub fn open_conflict(&mut self, p: conflict::Prompt) {
        self.current = Some(Overlay::Conflict(p));
    }

    pub fn close(&mut self) {
        self.current = None;
    }

    /// Route a key to the open overlay, if any. `esc` and `ctrl+c` are
    /// handled once, here, ahead of every overlay's own keys -- see the
    /// module doc.
    pub fn handle(&mut self, k: KeyEvent) -> Answer {
        if self.current.is_none() {
            return Answer::Closed;
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.current = None;
            return Answer::Quit;
        }
        if k.code == KeyCode::Esc {
            self.current = None;
            return Answer::Closed;
        }

        let overlay = self.current.as_mut().expect("checked above");
        let (close, answer) = match overlay {
            Overlay::Context(m) => match m.key(k) {
                Some(a) => (true, Answer::Context(m.target.clone(), a)),
                None => (false, Answer::Consumed),
            },
            Overlay::Destination(d) => match d.handle(k) {
                Some(r) => (true, Answer::Operation(r)),
                None => (false, Answer::Consumed),
            },
            Overlay::Help { scroll } => match k.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    *scroll = scroll.saturating_add(1);
                    (false, Answer::Consumed)
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    *scroll = scroll.saturating_sub(1);
                    (false, Answer::Consumed)
                }
                KeyCode::PageDown => {
                    *scroll = scroll.saturating_add(10);
                    (false, Answer::Consumed)
                }
                KeyCode::PageUp => {
                    *scroll = scroll.saturating_sub(10);
                    (false, Answer::Consumed)
                }
                // Anything else closes it, `?`/`F1` included -- the help
                // overlay has no other use for a key.
                _ => (true, Answer::Closed),
            },
            Overlay::Confirm(c) => match starkit::chrome::confirm::answer(k) {
                starkit::chrome::confirm::Answer::Yes => (true, Answer::Confirmed(c.pending)),
                starkit::chrome::confirm::Answer::No => (true, Answer::Closed),
                // `ctrl+c` is caught above, ahead of every overlay's own
                // keys, so this arm is unreached in practice; kept exhaustive
                // rather than assumed away.
                starkit::chrome::confirm::Answer::Quit => (true, Answer::Quit),
                starkit::chrome::confirm::Answer::Waiting => (false, Answer::Consumed),
            },
            Overlay::Rename(r) => match r.handle(k) {
                rename::Action::Taken => (false, Answer::Consumed),
                rename::Action::Close => (true, Answer::Closed),
                rename::Action::Renamed(to) => (
                    true,
                    Answer::Renamed {
                        from: r.from.clone(),
                        to,
                    },
                ),
            },
            Overlay::Conflict(p) => match p.handle(k) {
                conflict::Action::Taken => (false, Answer::Consumed),
                conflict::Action::Close => (true, Answer::Closed),
                conflict::Action::Policy(policy) => (true, Answer::Policy { op: p.op, policy }),
            },
        };
        if close {
            self.current = None;
        }
        answer
    }

    /// A click, while something is open. Outside the box it closes,
    /// whichever overlay it is; inside, the help ignores it, a confirmation's
    /// two footer words answer, and a conflict prompt's rows move the
    /// reading cursor while its own footer words answer.
    pub fn click(&mut self, x: u16, y: u16, area: Rect) -> Answer {
        if self.current.is_none() {
            return Answer::Closed;
        }
        let overlay = self.current.as_mut().expect("checked above");
        let (close, answer) = match overlay {
            Overlay::Context(m) => {
                let r = m.rect(area);
                if !inside(r, x, y) {
                    (true, Answer::Closed)
                } else if y > r.y && y < r.bottom() - 1 {
                    match m.actions.get((y - r.y - 1) as usize) {
                        Some(a) => (true, Answer::Context(m.target.clone(), *a)),
                        None => (false, Answer::Consumed),
                    }
                } else {
                    (false, Answer::Consumed)
                }
            }
            Overlay::Destination(_) => {
                if inside(context::Destination::rect(area), x, y) {
                    (false, Answer::Consumed)
                } else {
                    (true, Answer::Closed)
                }
            }
            Overlay::Help { .. } => {
                let r = help_rect(area);
                if inside(r, x, y) {
                    (false, Answer::Consumed)
                } else {
                    (true, Answer::Closed)
                }
            }
            Overlay::Confirm(c) => match confirm::layout(area, c) {
                Some(l) if !inside(l.rect, x, y) => (true, Answer::Closed),
                Some(l) if in_word(l.yes, l.footer_y, x, y) => (true, Answer::Confirmed(c.pending)),
                Some(l) if in_word(l.no, l.footer_y, x, y) => (true, Answer::Closed),
                Some(_) => (false, Answer::Consumed),
                None => (true, Answer::Closed),
            },
            Overlay::Rename(_) => {
                let r = rename::rect(area);
                if inside(r, x, y) {
                    (false, Answer::Consumed)
                } else {
                    (true, Answer::Closed)
                }
            }
            Overlay::Conflict(p) => match conflict::layout(area, p) {
                Some(l) if !inside(l.rect, x, y) => (true, Answer::Closed),
                Some(l) => {
                    if let Some(policy) = conflict::hit_footer(&l, x, y) {
                        (true, Answer::Policy { op: p.op, policy })
                    } else if conflict::hit_esc(&l, x, y) {
                        (true, Answer::Closed)
                    } else {
                        if let Some(row) = conflict::hit_row(&l, x, y, p) {
                            p.cursor = row;
                        }
                        (false, Answer::Consumed)
                    }
                }
                None => (true, Answer::Closed),
            },
        };
        if close {
            self.current = None;
        }
        answer
    }

    /// The wheel, while something is open. Only the help and a conflict
    /// prompt hold a list long enough to scroll.
    pub fn scroll(&mut self, up: bool) {
        match self.current.as_mut() {
            Some(Overlay::Help { scroll }) => {
                *scroll = if up {
                    scroll.saturating_sub(3)
                } else {
                    scroll.saturating_add(3)
                };
            }
            Some(Overlay::Conflict(p)) => {
                p.scroll = if up {
                    p.scroll.saturating_sub(3)
                } else {
                    p.scroll.saturating_add(3)
                };
            }
            _ => {}
        }
    }

    /// Draw whatever is open. Returns where the terminal's own cursor
    /// belongs -- only the rename field ever wants it there. `bars` is only
    /// read by the conflict prompt's own list, but every overlay takes it so
    /// the caller need not know which one that is.
    pub fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        theme: &Theme,
        bars: &mut Bars,
    ) -> Option<(u16, u16)> {
        match self.current.as_mut()? {
            Overlay::Context(m) => {
                m.render(area, buf, theme);
                None
            }
            Overlay::Destination(d) => d.render(area, buf, theme),
            Overlay::Help { scroll } => {
                // `HelpView` takes STAR/KIT's own `Theme`, which this crate's
                // wrapper derefs to; a struct literal is not a coercion site,
                // so the target type is spelled out to reach it explicitly.
                let core: &starkit::theme::Theme = theme;
                HelpView {
                    theme: core,
                    bindings: BINDINGS,
                    mouse: MOUSE,
                    scroll: *scroll,
                    title: "keys",
                }
                .render(area, buf);
                None
            }
            Overlay::Confirm(c) => {
                confirm::render(area, buf, theme, c);
                None
            }
            Overlay::Rename(r) => rename::render(area, buf, theme, r),
            Overlay::Conflict(p) => {
                conflict::render(area, buf, theme, p, bars);
                None
            }
        }
    }
}

fn inside(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

fn in_word((start, end): (u16, u16), row_y: u16, x: u16, y: u16) -> bool {
    y == row_y && x >= start && x < end
}

#[cfg(test)]
mod tests {
    use super::confirm::Confirm;
    use super::*;
    use crate::fold::ops::Conflict;
    use crate::ui::theme::tests_support::theme;
    use starkit::crossterm::event::KeyModifiers;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn code(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn one_conflict() -> conflict::Prompt {
        conflict::Prompt::new(
            OpId(1),
            vec![Conflict {
                source: PathBuf::from("/src/a.txt"),
                dest: PathBuf::from("/dest/a.txt"),
                both_dirs: false,
            }],
        )
    }

    fn every_overlay() -> Vec<fn(&mut Overlays)> {
        vec![
            |o: &mut Overlays| o.open_help(),
            |o: &mut Overlays| o.open_confirm(Confirm::clear_queue(2)),
            |o: &mut Overlays| o.open_rename(PathBuf::from("/tmp/a.txt")),
            |o: &mut Overlays| o.open_conflict(one_conflict()),
        ]
    }

    #[test]
    fn nothing_is_open_to_begin_with() {
        let o = Overlays::new();
        assert!(!o.is_open());
        assert!(o.current().is_none());
    }

    #[test]
    fn the_help_opens_and_the_help_key_closes_it() {
        let mut o = Overlays::new();
        o.open_help();
        assert!(o.is_open());
        assert_eq!(o.handle(key('?')), Answer::Closed);
        assert!(!o.is_open());
    }

    /// Escape closes every one of the four, and never quits -- the rule the
    /// key table holds everywhere else, held again here because an overlay
    /// is the place it is most tempting to break.
    #[test]
    fn escape_closes_every_overlay() {
        for open in every_overlay() {
            let mut o = Overlays::new();
            open(&mut o);
            let answer = o.handle(code(KeyCode::Esc));
            assert_eq!(answer, Answer::Closed);
            assert!(!o.is_open());
        }
    }

    #[test]
    fn ctrl_c_quits_from_inside_any_overlay() {
        for open in every_overlay() {
            let mut o = Overlays::new();
            open(&mut o);
            let answer = o.handle(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
            assert_eq!(answer, Answer::Quit);
            assert!(!o.is_open());
        }
    }

    #[test]
    fn a_confirmation_answers_on_y_and_on_n() {
        let mut o = Overlays::new();
        o.open_confirm(Confirm::delete_permanently(OpId(5), 3));
        assert_eq!(
            o.handle(key('y')),
            Answer::Confirmed(Pending::DeletePermanently(OpId(5)))
        );
        assert!(!o.is_open());

        o.open_confirm(Confirm::delete_permanently(OpId(5), 3));
        assert_eq!(o.handle(key('n')), Answer::Closed);
        assert!(!o.is_open());
    }

    #[test]
    fn a_confirmation_answers_on_a_click_of_either_word() {
        let area = Rect::new(0, 0, 60, 21);
        let mut o = Overlays::new();
        o.open_confirm(Confirm::delete_permanently(OpId(5), 3));
        let l = match o.current() {
            Some(Overlay::Confirm(c)) => confirm::layout(area, c).unwrap(),
            _ => unreachable!(),
        };
        let answer = o.click(l.yes.0, l.footer_y, area);
        assert_eq!(
            answer,
            Answer::Confirmed(Pending::DeletePermanently(OpId(5)))
        );
        assert!(!o.is_open());

        o.open_confirm(Confirm::delete_permanently(OpId(5), 3));
        let l = match o.current() {
            Some(Overlay::Confirm(c)) => confirm::layout(area, c).unwrap(),
            _ => unreachable!(),
        };
        let answer = o.click(l.no.0, l.footer_y, area);
        assert_eq!(answer, Answer::Closed);
        assert!(!o.is_open());
    }

    #[test]
    fn a_click_outside_the_box_closes_whatever_is_open() {
        // Large enough that even the help overlay -- capped at 80x38, but
        // otherwise happy to fill a smaller area corner to corner -- leaves a
        // margin outside itself for (0, 0) to land in.
        let area = Rect::new(0, 0, 100, 40);
        for open in every_overlay() {
            let mut o = Overlays::new();
            open(&mut o);
            let answer = o.click(0, 0, area);
            assert_eq!(answer, Answer::Closed, "at (0, 0) in a {area:?} box");
            assert!(!o.is_open());
        }
    }

    #[test]
    fn a_rename_submits_a_changed_name() {
        let mut o = Overlays::new();
        o.open_rename(PathBuf::from("/tmp/a.txt"));
        // The cursor starts before the extension, not at the end -- move to
        // the end and clear the whole line before typing the new name.
        o.handle(code(KeyCode::End));
        o.handle(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "b.txt".chars() {
            o.handle(key(c));
        }
        let answer = o.handle(code(KeyCode::Enter));
        assert_eq!(
            answer,
            Answer::Renamed {
                from: PathBuf::from("/tmp/a.txt"),
                to: PathBuf::from("/tmp/b.txt"),
            }
        );
        assert!(!o.is_open());
    }

    #[test]
    fn a_conflict_prompt_answers_on_o_s_and_r() {
        let mut o = Overlays::new();
        o.open_conflict(one_conflict());
        assert_eq!(
            o.handle(key('o')),
            Answer::Policy {
                op: OpId(1),
                policy: ConflictPolicy::Overwrite
            }
        );
        assert!(!o.is_open());

        o.open_conflict(one_conflict());
        assert_eq!(
            o.handle(key('s')),
            Answer::Policy {
                op: OpId(1),
                policy: ConflictPolicy::Skip
            }
        );

        o.open_conflict(one_conflict());
        assert_eq!(
            o.handle(key('r')),
            Answer::Policy {
                op: OpId(1),
                policy: ConflictPolicy::RenameNew
            }
        );
    }

    #[test]
    fn a_conflict_prompt_stays_queued_on_escape() {
        let mut o = Overlays::new();
        o.open_conflict(one_conflict());
        assert_eq!(o.handle(code(KeyCode::Esc)), Answer::Closed);
        assert!(!o.is_open());
    }

    /// Opening a second overlay replaces the first rather than stacking on
    /// top of it -- `current` is a single `Option`, so this is really a test
    /// that each `open_*` call reaches it.
    #[test]
    fn opening_one_overlay_replaces_whatever_was_open() {
        let mut o = Overlays::new();
        o.open_help();
        assert!(matches!(o.current(), Some(Overlay::Help { .. })));
        o.open_confirm(Confirm::clear_queue(2));
        assert!(matches!(o.current(), Some(Overlay::Confirm(_))));
        o.open_rename(PathBuf::from("/tmp/a.txt"));
        assert!(matches!(o.current(), Some(Overlay::Rename(_))));
        o.open_conflict(one_conflict());
        assert!(matches!(o.current(), Some(Overlay::Conflict(_))));
        o.open_help();
        assert!(matches!(o.current(), Some(Overlay::Help { .. })));
    }

    #[test]
    fn render_draws_each_overlays_title_and_does_not_panic() {
        let t = theme("terminal");
        for (open, title) in [
            (
                (|o: &mut Overlays| o.open_help()) as fn(&mut Overlays),
                "KEYS",
            ),
            (
                |o: &mut Overlays| o.open_confirm(Confirm::clear_queue(2)),
                "CLEAR THE QUEUE",
            ),
            (
                |o: &mut Overlays| o.open_rename(PathBuf::from("/tmp/Cargo.toml")),
                "RENAME",
            ),
            (
                |o: &mut Overlays| o.open_conflict(one_conflict()),
                "CONFLICTS",
            ),
        ] {
            for area in [Rect::new(0, 0, 60, 21), Rect::new(0, 0, 200, 60)] {
                let mut o = Overlays::new();
                open(&mut o);
                let mut buf = Buffer::empty(area);
                o.render(area, &mut buf, &t, &mut Bars::new());
                let text: String = (0..area.height)
                    .map(|y| {
                        (0..area.width)
                            .map(|x| buf[(x, y)].symbol().to_string())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains(title), "{area:?}: {text}");
            }
        }
    }

    #[test]
    fn a_closed_overlay_renders_nothing_and_no_cursor() {
        let t = theme("terminal");
        let area = Rect::new(0, 0, 60, 21);
        let mut buf = Buffer::empty(area);
        let before = buf.clone();
        let cursor = Overlays::new().render(area, &mut buf, &t, &mut Bars::new());
        assert_eq!(buf, before);
        assert_eq!(cursor, None);
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        fn arb_key() -> impl Strategy<Value = KeyEvent> {
            let code = prop_oneof![
                Just(KeyCode::Esc),
                Just(KeyCode::Enter),
                Just(KeyCode::Up),
                Just(KeyCode::Down),
                Just(KeyCode::Left),
                Just(KeyCode::Right),
                Just(KeyCode::PageUp),
                Just(KeyCode::PageDown),
                Just(KeyCode::Home),
                Just(KeyCode::End),
                Just(KeyCode::Backspace),
                Just(KeyCode::Delete),
                Just(KeyCode::F(1)),
                prop_oneof![
                    Just('a'),
                    Just('y'),
                    Just('n'),
                    Just('o'),
                    Just('s'),
                    Just('r'),
                    Just('j'),
                    Just('k'),
                    Just('c'),
                    Just('?'),
                    Just('.'),
                    Just('/'),
                    Just(' '),
                ]
                .prop_map(KeyCode::Char),
            ];
            let modifiers = prop_oneof![
                Just(KeyModifiers::NONE),
                Just(KeyModifiers::CONTROL),
                Just(KeyModifiers::SHIFT),
                Just(KeyModifiers::ALT),
            ];
            (code, modifiers).prop_map(|(code, modifiers)| KeyEvent::new(code, modifiers))
        }

        fn open_nth(o: &mut Overlays, n: usize) {
            match n % 4 {
                0 => o.open_help(),
                1 => o.open_confirm(Confirm::clear_queue(2)),
                2 => o.open_rename(PathBuf::from("/tmp/a.txt")),
                _ => o.open_conflict(one_conflict()),
            }
        }

        proptest! {
            #[test]
            fn random_keys_never_panic_whichever_overlay_is_open(
                opener in 0..4usize,
                keys in proptest::collection::vec(arb_key(), 0..60),
            ) {
                let mut o = Overlays::new();
                open_nth(&mut o, opener);
                for k in keys {
                    if !o.is_open() {
                        // Re-open once the last key closed it, so the run
                        // keeps exercising the overlay rather than idling on
                        // `Answer::Closed` for the rest of the sequence.
                        open_nth(&mut o, opener);
                    }
                    let _ = o.handle(k);
                }
            }
        }
    }
}
