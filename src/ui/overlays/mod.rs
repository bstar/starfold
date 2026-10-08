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
pub mod create;
pub mod failure;
pub mod rename;
pub mod search;
pub mod sort;
pub mod trash_warning;
pub mod unsaved;

use std::path::PathBuf;

use starkit::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use starkit::keymap::{help_rect, HelpView};
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;

use crate::fold::create::Kind as CreateKind;
use crate::fold::ops::{ConflictPolicy, OpId};
use crate::fold::sort::SortOrder;
use crate::ui::keymap::{BINDINGS, MOUSE};
use crate::ui::theme::Theme;
use crate::ui::Bars;

/// What a [`confirm::Confirm`] is asking about, carried through unopened so
/// the caller learns it again only once the answer is yes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    CloseTab(crate::fold::tab::TabId),
    DeletePermanently(Vec<PathBuf>),
    ElevatedDelete(OpId),
    QueueDelete(Vec<PathBuf>),
    ClearQueue,
    CancelRunning(OpId),
    Quit,
}

/// The one overlay that may be open, and what it needs to keep drawing
/// itself between frames.
#[derive(Debug)]
pub enum Overlay {
    Context(context::Menu),
    Drop(context::Menu),
    Destination(context::Destination),
    Help { scroll: u16 },
    Failure(failure::Failure),
    Update(failure::Failure),
    Confirm(confirm::Confirm),
    Unsaved(String),
    TrashWarning(trash_warning::Prompt),
    Create(create::Create),
    Rename(rename::Rename),
    Search(search::Search),
    Sort(sort::Picker),
    Conflict(conflict::Prompt),
    ConflictRename(conflict::RenameSequence),
}

/// Modal things drawn over everything else. See the module doc for the one
/// rule they share.
#[derive(Debug, Default)]
pub struct Overlays {
    current: Option<Overlay>,
    help_shortcuts: crate::config::Shortcuts,
}

#[cfg(feature = "terminal-graphics")]
impl Overlays {
    pub fn pointer_regions(&mut self, area: Rect) -> Vec<Rect> {
        if let Some(Overlay::Context(menu) | Overlay::Drop(menu)) = self.current.as_mut() {
            return menu.popup.pointer_regions(area);
        }
        let Some(rect) = self.graphical_rect(area) else {
            return Vec::new();
        };
        let Some(overlay) = self.current.as_ref() else {
            return Vec::new();
        };
        let footer = match overlay {
            Overlay::Destination(d) => Some(d.footer()),
            Overlay::Rename(_) | Overlay::ConflictRename(_) => Some(rename::FOOTER),
            Overlay::Create(_) => Some(create::FOOTER),
            Overlay::Search(_) => Some(search::FOOTER),
            Overlay::Failure(f) | Overlay::Update(f) => Some(failure::footer(f)),
            _ => None,
        };
        let mut result = Vec::new();
        for y in rect.y..rect.bottom() {
            let mut start = None;
            for x in rect.x..=rect.right() {
                let hit = x < rect.right()
                    && (footer.is_some_and(|f| footer_key(rect, f, x, y).is_some())
                        || match overlay {
                            Overlay::Confirm(c) => confirm::layout(area, c).is_some_and(|l| {
                                in_word(l.yes, l.footer_y, x, y) || in_word(l.no, l.footer_y, x, y)
                            }),
                            Overlay::Unsaved(_) => unsaved::click(rect, x, y).is_some(),
                            Overlay::TrashWarning(_) => trash_warning::click(rect, x, y).is_some(),
                            Overlay::Sort(p) => {
                                y > rect.y && p.choose(usize::from(y - rect.y - 1)).is_some()
                            }
                            Overlay::Conflict(p) => conflict::layout(area, p).is_some_and(|l| {
                                conflict::hit_footer(&l, x, y).is_some()
                                    || conflict::hit_esc(&l, x, y)
                                    || conflict::hit_row(&l, x, y, p).is_some()
                            }),
                            _ => false,
                        });
                if hit && start.is_none() {
                    start = Some(x);
                }
                if !hit {
                    if let Some(left) = start.take() {
                        result.push(Rect::new(left, y, x - left, 1));
                    }
                }
            }
        }
        result
    }
    pub fn graphical_rects(&mut self, area: Rect) -> Vec<Rect> {
        if let Some(Overlay::Context(menu) | Overlay::Drop(menu)) = self.current.as_mut() {
            return menu.popup.graphical_rects(area);
        }
        self.graphical_rect(area).into_iter().collect()
    }
    pub fn graphical_rect(&self, area: Rect) -> Option<Rect> {
        Some(match self.current.as_ref()? {
            Overlay::Context(menu) | Overlay::Drop(menu) => menu.popup.root_rect(area),
            Overlay::Destination(d) => d.bounds(area),
            Overlay::Help { .. } => help_rect(area),
            Overlay::Failure(_) | Overlay::Update(_) => failure::rect(area),
            Overlay::Confirm(prompt) => confirm::layout(area, prompt)?.rect,
            Overlay::Unsaved(_) => unsaved::layout(area)?,
            Overlay::TrashWarning(_) => trash_warning::layout(area)?,
            Overlay::Create(_) => create::rect(area),
            Overlay::Rename(_) | Overlay::ConflictRename(_) => rename::rect(area),
            Overlay::Search(_) => search::rect(area),
            Overlay::Sort(_) => sort::Picker::rect(area),
            Overlay::Conflict(prompt) => conflict::layout(area, prompt)?.rect,
        })
    }
}

/// What handling a key or a click did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Context(context::Target, context::Action),
    Drop(crate::fold::ops::OpKind),
    Operation(context::Request),
    /// The overlay took the key or click; nothing else happened.
    Consumed,
    /// The overlay closed without deciding anything -- `n`, `esc`, a click
    /// outside the box.
    Closed,
    /// `y`/`enter` on a [`confirm::Confirm`].
    EditorChanges(bool),
    Confirmed(Pending),
    RetryFailedDelete(Vec<PathBuf>),
    RetryFailedDeleteWithSudo(OpId),
    TrashDelete {
        sources: Vec<PathBuf>,
        drive: trash_warning::DriveKey,
        suppress_for_session: bool,
    },
    /// A [`rename::Rename`] was submitted with a name that passed
    /// validation.
    Renamed {
        from: PathBuf,
        to: PathBuf,
    },
    Created {
        dir: PathBuf,
        kind: CreateKind,
        name: String,
    },
    Search(String, crate::fold::search::Mode),
    Sort(SortOrder),
    /// `o`/`s`/`r` on a [`conflict::Prompt`], applied to the whole op.
    Policy {
        op: OpId,
        policy: ConflictPolicy,
    },
    ConflictNames {
        op: OpId,
        targets: Vec<(PathBuf, PathBuf)>,
    },
    /// `ctrl+c`, which quits from inside an overlay the same as it does
    /// everywhere else.
    Quit,
}

impl Overlays {
    pub fn new() -> Self {
        Self {
            current: None,
            help_shortcuts: crate::config::Shortcuts::default(),
        }
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
    pub fn open_update(&mut self, lines: Vec<String>) {
        self.current = Some(Overlay::Update(failure::Failure {
            lines,
            scroll: 0,
            retry: None,
            sudo_retry: None,
        }));
    }

    pub fn open_help_with_shortcuts(&mut self, shortcuts: &crate::config::Shortcuts) {
        self.help_shortcuts = shortcuts.clone();
        self.open_help();
    }
    pub fn open_help(&mut self) {
        self.current = Some(Overlay::Help { scroll: 0 });
    }

    pub fn open_failure(&mut self, failure: failure::Failure) {
        self.current = Some(Overlay::Failure(failure));
    }

    pub fn open_sort(&mut self, order: SortOrder) {
        self.current = Some(Overlay::Sort(sort::Picker::new(order)));
    }

    pub fn open_unsaved(&mut self, filename: String) {
        self.current = Some(Overlay::Unsaved(filename));
    }
    pub fn open_confirm(&mut self, c: confirm::Confirm) {
        self.current = Some(Overlay::Confirm(c));
    }

    pub fn open_trash_warning(&mut self, prompt: trash_warning::Prompt) {
        self.current = Some(Overlay::TrashWarning(prompt));
    }

    pub fn open_context(&mut self, target: context::Target, anchor: (u16, u16)) {
        self.current = Some(Overlay::Context(context::Menu::new(target, anchor)));
    }
    pub fn open_drop(&mut self, anchor: (u16, u16)) {
        self.current = Some(Overlay::Drop(context::Menu::for_drop(anchor)));
    }
    pub fn open_destination(&mut self, request: context::Request) {
        self.current = Some(Overlay::Destination(context::Destination::new(request)));
    }

    pub fn open_rename(&mut self, from: PathBuf) {
        self.current = Some(Overlay::Rename(rename::Rename::new(from)));
    }

    pub fn open_create(&mut self, dir: PathBuf, kind: CreateKind) {
        self.current = Some(Overlay::Create(create::Create::new(dir, kind)));
    }

    pub fn open_search(&mut self, query: &str, mode: crate::fold::search::Mode) {
        self.current = Some(Overlay::Search(search::Search::new(query, mode)));
    }

    pub fn paste(&mut self, text: &str) -> bool {
        let input = match self.current.as_mut() {
            Some(Overlay::Search(form)) => {
                form.error = None;
                &mut form.input
            }
            Some(Overlay::Rename(form)) => {
                form.error = None;
                &mut form.input
            }
            Some(Overlay::Create(form)) => {
                form.error = None;
                &mut form.input
            }
            Some(Overlay::Destination(form)) => {
                form.error = None;
                if form.field == 3 {
                    &mut form.password
                } else {
                    &mut form.input
                }
            }
            Some(Overlay::ConflictRename(sequence)) => {
                sequence.form.error = None;
                &mut sequence.form.input
            }
            _ => return false,
        };
        input.paste(text);
        true
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
        if matches!(k.code, KeyCode::Esc | KeyCode::Left) {
            if let Some(Overlay::Context(m) | Overlay::Drop(m)) = self.current.as_mut() {
                if m.popup.back() {
                    return Answer::Consumed;
                }
                self.current = None;
                return Answer::Closed;
            }
        }
        if k.code == KeyCode::Esc {
            self.current = None;
            return Answer::Closed;
        }

        let overlay = self.current.as_mut().expect("checked above");
        let mut start_rename = None;
        let (close, answer) = match overlay {
            Overlay::Drop(m) => match k.code {
                KeyCode::Char('c') => (true, Answer::Drop(crate::fold::ops::OpKind::Copy)),
                KeyCode::Char('m') => (true, Answer::Drop(crate::fold::ops::OpKind::Move)),
                _ => match m.key(k) {
                    Some(context::Action::Copy) => {
                        (true, Answer::Drop(crate::fold::ops::OpKind::Copy))
                    }
                    Some(context::Action::Move) => {
                        (true, Answer::Drop(crate::fold::ops::OpKind::Move))
                    }
                    _ => (false, Answer::Consumed),
                },
            },
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
            Overlay::Failure(failure) | Overlay::Update(failure) => match k.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    failure.scroll = failure
                        .scroll
                        .saturating_add(1)
                        .min(failure.lines.len().saturating_sub(1));
                    (false, Answer::Consumed)
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    failure.scroll = failure.scroll.saturating_sub(1);
                    (false, Answer::Consumed)
                }
                KeyCode::PageDown => {
                    failure.scroll = failure
                        .scroll
                        .saturating_add(10)
                        .min(failure.lines.len().saturating_sub(1));
                    (false, Answer::Consumed)
                }
                KeyCode::PageUp => {
                    failure.scroll = failure.scroll.saturating_sub(10);
                    (false, Answer::Consumed)
                }
                KeyCode::Char('r') => match &failure.retry {
                    Some(paths) => (true, Answer::RetryFailedDelete(paths.clone())),
                    None => (false, Answer::Consumed),
                },
                KeyCode::Char('s') => match failure.sudo_retry {
                    Some(id) => (true, Answer::RetryFailedDeleteWithSudo(id)),
                    None => (false, Answer::Consumed),
                },
                _ => (false, Answer::Consumed),
            },
            Overlay::Confirm(c) => match starkit::chrome::confirm::answer(k) {
                starkit::chrome::confirm::Answer::Yes => {
                    (true, Answer::Confirmed(c.pending.clone()))
                }
                starkit::chrome::confirm::Answer::No => (true, Answer::Closed),
                // `ctrl+c` is caught above, ahead of every overlay's own
                // keys, so this arm is unreached in practice; kept exhaustive
                // rather than assumed away.
                starkit::chrome::confirm::Answer::Quit => (true, Answer::Quit),
                starkit::chrome::confirm::Answer::Waiting => (false, Answer::Consumed),
            },
            Overlay::Unsaved(_) => match k.code {
                KeyCode::Char('s') => (true, Answer::EditorChanges(true)),
                KeyCode::Char('d') => (true, Answer::EditorChanges(false)),
                _ => (false, Answer::Consumed),
            },
            Overlay::TrashWarning(prompt) => match trash_warning::Prompt::key(k) {
                Some(suppress_for_session) => (
                    true,
                    Answer::TrashDelete {
                        sources: prompt.sources.clone(),
                        drive: prompt.drive.clone(),
                        suppress_for_session,
                    },
                ),
                None => (false, Answer::Consumed),
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
            Overlay::Create(form) => match form.handle(k) {
                create::Action::Taken => (false, Answer::Consumed),
                create::Action::Close => (true, Answer::Closed),
                create::Action::Create { dir, kind, name } => {
                    (true, Answer::Created { dir, kind, name })
                }
            },
            Overlay::Search(form) => match form.handle(k) {
                Some((query, mode)) => (true, Answer::Search(query, mode)),
                None => (false, Answer::Consumed),
            },
            Overlay::Sort(picker) => match picker.key(k) {
                Some(order) => (true, Answer::Sort(order)),
                None => (false, Answer::Consumed),
            },
            Overlay::Conflict(p) => match p.handle(k) {
                conflict::Action::Taken => (false, Answer::Consumed),
                conflict::Action::Close => (true, Answer::Closed),
                conflict::Action::Policy(ConflictPolicy::RenameNew) => {
                    start_rename = Some(conflict::RenameSequence::new(p.op, p.conflicts.clone()));
                    (false, Answer::Consumed)
                }
                conflict::Action::Policy(policy) => (true, Answer::Policy { op: p.op, policy }),
            },
            Overlay::ConflictRename(sequence) => match sequence.form.handle(k) {
                rename::Action::Taken => (false, Answer::Consumed),
                rename::Action::Close => (true, Answer::Closed),
                rename::Action::Renamed(target) => match sequence.accept(target) {
                    Some(targets) => (
                        true,
                        Answer::ConflictNames {
                            op: sequence.op,
                            targets,
                        },
                    ),
                    None => (false, Answer::Consumed),
                },
            },
        };
        if let Some(sequence) = start_rename {
            self.current = Some(Overlay::ConflictRename(sequence));
            return Answer::Consumed;
        }
        if close {
            self.current = None;
        }
        answer
    }

    pub fn hover(&mut self, area: Rect, x: u16, y: u16) {
        if let Some(Overlay::Context(m) | Overlay::Drop(m)) = self.current.as_mut() {
            m.popup.hover(area, x, y);
        }
    }
    pub fn dismiss_outside(&mut self, area: Rect, x: u16, y: u16) -> Answer {
        if let Some(Overlay::Context(m) | Overlay::Drop(m)) = self.current.as_mut() {
            if !m.popup.contains(area, x, y) {
                self.current = None;
                return Answer::Closed;
            }
        }
        Answer::Consumed
    }
    pub fn tick(&mut self, now: std::time::Instant) {
        if let Some(Overlay::Context(m) | Overlay::Drop(m)) = self.current.as_mut() {
            m.popup.tick(now);
        }
    }
    pub fn menu_wheel(&mut self, area: Rect, x: u16, y: u16, down: bool) -> bool {
        if let Some(Overlay::Context(m) | Overlay::Drop(m)) = self.current.as_mut() {
            m.popup.wheel(area, x, y, down);
            true
        } else {
            false
        }
    }
    /// A click, while something is open. Outside the box it closes,
    /// whichever overlay it is; inside, the help ignores it, a confirmation's
    /// two footer words answer, and a conflict prompt's rows move the
    /// reading cursor while its own footer words answer.
    pub fn click(&mut self, x: u16, y: u16, area: Rect) -> Answer {
        if self.current.is_none() {
            return Answer::Closed;
        }
        // Route visible footer actions through the same validation and state
        // transitions as keys. No duplicate operation/confirmation logic.
        let footer = match self.current.as_ref().unwrap() {
            Overlay::Destination(d) => Some((d.bounds(area), d.footer())),
            Overlay::Rename(_) | Overlay::ConflictRename(_) => {
                Some((rename::rect(area), rename::FOOTER))
            }
            Overlay::Create(_) => Some((create::rect(area), create::FOOTER)),
            Overlay::Search(_) => Some((search::rect(area), search::FOOTER)),
            Overlay::Failure(f) | Overlay::Update(f) => {
                Some((failure::rect(area), failure::footer(f)))
            }
            _ => None,
        };
        if let Some((rect, footer)) = footer {
            if let Some(code) = footer_key(rect, footer, x, y) {
                return self.handle(KeyEvent::new(code, KeyModifiers::NONE));
            }
        }
        let overlay = self.current.as_mut().expect("checked above");
        let mut start_rename = None;
        let (close, answer) = match overlay {
            Overlay::Drop(m) => match m.popup.click(area, x, y) {
                super::popup::Answer::Selected(context::Action::Copy) => {
                    (true, Answer::Drop(crate::fold::ops::OpKind::Copy))
                }
                super::popup::Answer::Selected(context::Action::Move) => {
                    (true, Answer::Drop(crate::fold::ops::OpKind::Move))
                }
                super::popup::Answer::Dismissed => (true, Answer::Closed),
                _ => (false, Answer::Consumed),
            },
            Overlay::Context(m) => match m.popup.click(area, x, y) {
                super::popup::Answer::Selected(a) => (true, Answer::Context(m.target.clone(), a)),
                super::popup::Answer::Dismissed => (true, Answer::Closed),
                _ => (false, Answer::Consumed),
            },
            Overlay::Destination(d) => {
                if inside(d.bounds(area), x, y) {
                    match d.click(area, x, y) {
                        Some(r) => (true, Answer::Operation(r)),
                        None => (false, Answer::Consumed),
                    }
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
            Overlay::Failure(_) | Overlay::Update(_) => {
                if inside(failure::rect(area), x, y) {
                    (false, Answer::Consumed)
                } else {
                    (true, Answer::Closed)
                }
            }
            Overlay::Confirm(c) => match confirm::layout(area, c) {
                Some(l) if !inside(l.rect, x, y) => (true, Answer::Closed),
                Some(l) if in_word(l.yes, l.footer_y, x, y) => {
                    (true, Answer::Confirmed(c.pending.clone()))
                }
                Some(l) if in_word(l.no, l.footer_y, x, y) => (true, Answer::Closed),
                Some(_) => (false, Answer::Consumed),
                None => (true, Answer::Closed),
            },
            Overlay::Unsaved(_) => match unsaved::layout(area) {
                Some(rect) if !inside(rect, x, y) => (true, Answer::Closed),
                Some(rect) => match unsaved::click(rect, x, y) {
                    Some(Some(save)) => (true, Answer::EditorChanges(save)),
                    Some(None) => (true, Answer::Closed),
                    None => (false, Answer::Consumed),
                },
                None => (true, Answer::Closed),
            },
            Overlay::TrashWarning(prompt) => match trash_warning::layout(area) {
                Some(rect) if !inside(rect, x, y) => (true, Answer::Closed),
                Some(rect) => match trash_warning::click(rect, x, y) {
                    Some(Some(suppress_for_session)) => (
                        true,
                        Answer::TrashDelete {
                            sources: prompt.sources.clone(),
                            drive: prompt.drive.clone(),
                            suppress_for_session,
                        },
                    ),
                    Some(None) => (true, Answer::Closed),
                    None => (false, Answer::Consumed),
                },
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
            Overlay::Create(_) => {
                let r = create::rect(area);
                if inside(r, x, y) {
                    (false, Answer::Consumed)
                } else {
                    (true, Answer::Closed)
                }
            }
            Overlay::Search(_) => {
                let r = search::rect(area);
                if inside(r, x, y) {
                    (false, Answer::Consumed)
                } else {
                    (true, Answer::Closed)
                }
            }
            Overlay::Sort(picker) => {
                let rect = sort::Picker::rect(area);
                if !inside(rect, x, y) {
                    (true, Answer::Closed)
                } else if y > rect.y && y < rect.bottom().saturating_sub(1) {
                    match picker.choose((y - rect.y - 1) as usize) {
                        Some(order) => (true, Answer::Sort(order)),
                        None => (false, Answer::Consumed),
                    }
                } else {
                    (false, Answer::Consumed)
                }
            }
            Overlay::Conflict(p) => match conflict::layout(area, p) {
                Some(l) if !inside(l.rect, x, y) => (true, Answer::Closed),
                Some(l) => {
                    if let Some(policy) = conflict::hit_footer(&l, x, y) {
                        if policy == ConflictPolicy::RenameNew {
                            start_rename =
                                Some(conflict::RenameSequence::new(p.op, p.conflicts.clone()));
                            (false, Answer::Consumed)
                        } else {
                            (true, Answer::Policy { op: p.op, policy })
                        }
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
            Overlay::ConflictRename(_) => {
                if inside(rename::rect(area), x, y) {
                    (false, Answer::Consumed)
                } else {
                    (true, Answer::Closed)
                }
            }
        };
        if let Some(sequence) = start_rename {
            self.current = Some(Overlay::ConflictRename(sequence));
            return Answer::Consumed;
        }
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
            Some(Overlay::Failure(failure) | Overlay::Update(failure)) => {
                failure.scroll = if up {
                    failure.scroll.saturating_sub(3)
                } else {
                    failure
                        .scroll
                        .saturating_add(3)
                        .min(failure.lines.len().saturating_sub(1))
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
            Overlay::Drop(m) => {
                m.render(area, buf, theme);
                None
            }
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
                .render_with_keys(area, buf, |b| match b.action {
                    crate::ui::keymap::Action::NextTheme => self.help_shortcuts.next_theme.clone(),
                    crate::ui::keymap::Action::PrevTheme => {
                        self.help_shortcuts.previous_theme.clone()
                    }
                    crate::ui::keymap::Action::TogglePresentation => {
                        self.help_shortcuts.presentation.clone()
                    }
                    _ => b.keys.to_owned(),
                });
                None
            }
            Overlay::Update(notice) => {
                failure::render_titled(area, buf, theme, notice, "STAR/FOLD update");
                None
            }
            Overlay::Failure(failure) => {
                failure::render(area, buf, theme, failure);
                None
            }
            Overlay::Confirm(c) => {
                confirm::render(area, buf, theme, c);
                None
            }
            Overlay::Unsaved(filename) => {
                unsaved::render(area, buf, theme, filename);
                None
            }
            Overlay::TrashWarning(prompt) => {
                trash_warning::render(area, buf, theme, prompt);
                None
            }
            Overlay::Rename(r) => rename::render(area, buf, theme, r),
            Overlay::Create(form) => create::render(area, buf, theme, form),
            Overlay::Search(form) => search::render(area, buf, theme, form),
            Overlay::Sort(picker) => {
                picker.render(area, buf, theme);
                None
            }
            Overlay::Conflict(p) => {
                conflict::render(area, buf, theme, p, bars);
                None
            }
            Overlay::ConflictRename(sequence) => {
                rename::render(area, buf, theme, &mut sequence.form)
            }
        }
    }
}

/// Footer labels use the exact right alignment and fit rule of KIT's frame.
/// Separators, clipped labels and body text must never submit an operation.
fn footer_key(rect: Rect, footer: &str, x: u16, y: u16) -> Option<KeyCode> {
    let width = starkit::wrap::width_of(footer).saturating_add(2);
    // Native chrome reserves two extra cells; the ordinary frame reserves two.
    let inset = if starkit::chrome::frame::extra_rows() > 0 {
        4
    } else {
        2
    };
    if rect.height == 0 || y != rect.bottom() - 1 || width > rect.width.saturating_sub(inset) {
        return None;
    }
    let mut start = rect.right() - width;
    for word in footer.split(" · ") {
        let end = start + starkit::wrap::width_of(word);
        if x >= start && x < end {
            return match word.split_whitespace().next()? {
                "enter" => Some(KeyCode::Enter),
                "esc" => Some(KeyCode::Esc),
                "tab" => Some(KeyCode::Tab),
                "r" => Some(KeyCode::Char('r')),
                "s" => Some(KeyCode::Char('s')),
                _ => None,
            };
        }
        start = end + 3;
    }
    None
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

    fn trash_warning() -> trash_warning::Prompt {
        trash_warning::Prompt::new(
            vec![PathBuf::from("/mnt/drive/image.png")],
            trash_warning::DriveKey {
                path: PathBuf::from("/mnt/drive"),
                source: "/dev/sdb1".into(),
                uuid: Some("ABC".into()),
            },
        )
    }

    fn every_overlay() -> Vec<fn(&mut Overlays)> {
        vec![
            |o: &mut Overlays| o.open_help(),
            |o: &mut Overlays| o.open_confirm(Confirm::clear_queue(2)),
            |o: &mut Overlays| o.open_trash_warning(trash_warning()),
            |o: &mut Overlays| o.open_unsaved("notes.txt".into()),
            |o: &mut Overlays| o.open_rename(PathBuf::from("/tmp/a.txt")),
            |o: &mut Overlays| o.open_conflict(one_conflict()),
        ]
    }

    // Locate the actual drawn label, rather than duplicating footer geometry.
    fn click_label(o: &mut Overlays, label: &str) -> Answer {
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = Buffer::empty(area);
        let theme = crate::ui::theme::tests_support::theme("terminal");
        o.render(area, &mut buf, &theme, &mut Bars::default());
        for y in 0..area.height {
            for x in 0..area.width {
                let row: String = (x..area.width).map(|col| buf[(col, y)].symbol()).collect();
                if row.starts_with(label) {
                    return o.click(x, y, area);
                }
            }
        }
        panic!("missing action {label}");
    }

    #[test]
    fn mouse_submits_destination_and_conflict_names_in_both_presentations() {
        for native in [false, true] {
            let _scope = starkit::chrome::frame::padding_scope(native);
            let mut o = Overlays::new();
            for kind in [
                crate::fold::ops::OpKind::Copy,
                crate::fold::ops::OpKind::Move,
            ] {
                o.current = Some(Overlay::Destination(context::Destination::new(
                    context::Request {
                        archive_options: Default::default(),
                        kind,
                        sources: vec!["/src/a.txt".into()],
                        destination: "/dest".into(),
                    },
                )));
                match click_label(&mut o, "enter ") {
                    Answer::Operation(request) => {
                        assert_eq!(request.kind, kind);
                        assert_eq!(request.sources, vec![PathBuf::from("/src/a.txt")]);
                        assert_eq!(request.destination, PathBuf::from("/dest"));
                    }
                    answer => panic!("{answer:?}"),
                }
                assert!(!o.is_open());
            }
            o.open_conflict(one_conflict());
            assert_eq!(click_label(&mut o, "r edit name"), Answer::Consumed);
            // The editable suggestion must remain visible beneath the native title.
            let area = Rect::new(0, 0, 120, 40);
            let mut buf = Buffer::empty(area);
            o.render(
                area,
                &mut buf,
                &crate::ui::theme::tests_support::theme("terminal"),
                &mut Bars::default(),
            );
            assert!(buf
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .contains("a (1).txt"));
            match click_label(&mut o, "enter rename") {
                Answer::ConflictNames { targets, .. } => assert_eq!(targets.len(), 1),
                answer => panic!("{answer:?}"),
            }
            assert!(!o.is_open());
        }
    }

    #[test]
    fn mouse_submit_keeps_invalid_form_open_and_cancel_closes_it() {
        let mut o = Overlays::new();
        o.open_rename("/tmp/a.txt".into());
        assert_eq!(click_label(&mut o, "enter rename"), Answer::Consumed);
        assert!(matches!(o.current(), Some(Overlay::Rename(r)) if r.error.is_some()));
        assert_eq!(click_label(&mut o, "esc cancel"), Answer::Closed);
        assert!(!o.is_open());
    }

    #[test]
    fn footer_separators_and_hidden_actions_do_not_submit() {
        let rect = Rect::new(10, 10, 60, 4);
        let text = rename::FOOTER;
        let start = rect.right() - starkit::wrap::width_of(text) - 2;
        assert_eq!(footer_key(rect, text, start + 12, rect.bottom() - 1), None);
        assert_eq!(footer_key(Rect::new(0, 0, 10, 4), text, 1, 3), None);
        assert_eq!(footer_key(rect, text, start, rect.y + 1), None);
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
        o.open_confirm(Confirm::delete_permanently(vec![PathBuf::from("/a"); 3]));
        assert_eq!(
            o.handle(key('y')),
            Answer::Confirmed(Pending::DeletePermanently(vec![PathBuf::from("/a"); 3]))
        );
        assert!(!o.is_open());

        o.open_confirm(Confirm::delete_permanently(vec![PathBuf::from("/a"); 3]));
        assert_eq!(o.handle(key('n')), Answer::Closed);
        assert!(!o.is_open());
    }

    #[test]
    fn a_confirmation_answers_on_a_click_of_either_word() {
        let area = Rect::new(0, 0, 60, 21);
        let mut o = Overlays::new();
        o.open_confirm(Confirm::delete_permanently(vec![PathBuf::from("/a"); 3]));
        let l = match o.current() {
            Some(Overlay::Confirm(c)) => confirm::layout(area, c).unwrap(),
            _ => unreachable!(),
        };
        let answer = o.click(l.yes.0, l.footer_y, area);
        assert_eq!(
            answer,
            Answer::Confirmed(Pending::DeletePermanently(vec![PathBuf::from("/a"); 3]))
        );
        assert!(!o.is_open());

        o.open_confirm(Confirm::delete_permanently(vec![PathBuf::from("/a"); 3]));
        let l = match o.current() {
            Some(Overlay::Confirm(c)) => confirm::layout(area, c).unwrap(),
            _ => unreachable!(),
        };
        let answer = o.click(l.no.0, l.footer_y, area);
        assert_eq!(answer, Answer::Closed);
        assert!(!o.is_open());
    }

    #[test]
    fn trash_warning_keys_and_clicks_have_distinct_delete_and_cancel_actions() {
        let area = Rect::new(0, 0, 60, 21);
        let rect = trash_warning::layout(area).unwrap();
        let expected = trash_warning();
        let mut overlays = Overlays::new();

        overlays.open_trash_warning(trash_warning());
        assert_eq!(overlays.handle(key('x')), Answer::Consumed);
        assert_eq!(
            overlays.handle(key('d')),
            Answer::TrashDelete {
                sources: expected.sources.clone(),
                drive: expected.drive.clone(),
                suppress_for_session: false,
            }
        );
        overlays.open_trash_warning(trash_warning());
        assert_eq!(
            overlays.handle(key('s')),
            Answer::TrashDelete {
                sources: expected.sources.clone(),
                drive: expected.drive.clone(),
                suppress_for_session: true,
            }
        );
        overlays.open_trash_warning(trash_warning());
        assert_eq!(
            overlays.click(rect.x + 2, rect.y + 2, area),
            Answer::TrashDelete {
                sources: expected.sources.clone(),
                drive: expected.drive.clone(),
                suppress_for_session: false,
            }
        );
        overlays.open_trash_warning(trash_warning());
        assert_eq!(
            overlays.click(rect.x + 2, rect.y + 3, area),
            Answer::TrashDelete {
                sources: expected.sources.clone(),
                drive: expected.drive.clone(),
                suppress_for_session: true,
            }
        );
        overlays.open_trash_warning(trash_warning());
        assert_eq!(overlays.click(rect.x + 2, rect.y + 4, area), Answer::Closed);
        assert!(!overlays.is_open());
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
        assert_eq!(o.handle(key('r')), Answer::Consumed);
        assert!(matches!(o.current(), Some(Overlay::ConflictRename(_))));
        assert_eq!(
            o.handle(code(KeyCode::Enter)),
            Answer::ConflictNames {
                op: OpId(1),
                targets: vec![(
                    PathBuf::from("/dest/a.txt"),
                    PathBuf::from("/dest/a (1).txt")
                )],
            }
        );
    }

    #[test]
    fn rename_edits_each_conflict_before_resuming() {
        let mut o = Overlays::new();
        o.open_conflict(conflict::Prompt::new(
            OpId(7),
            vec![
                Conflict {
                    source: "/src/a.txt".into(),
                    dest: "/dest/a.txt".into(),
                    both_dirs: false,
                },
                Conflict {
                    source: "/src/b.txt".into(),
                    dest: "/dest/b.txt".into(),
                    both_dirs: false,
                },
            ],
        ));
        assert_eq!(o.handle(key('r')), Answer::Consumed);
        assert_eq!(o.handle(code(KeyCode::Enter)), Answer::Consumed);
        let Some(Overlay::ConflictRename(sequence)) = o.current() else {
            panic!("rename editor")
        };
        assert_eq!(sequence.index, 1);
        o.handle(code(KeyCode::End));
        o.handle(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "other.txt".chars() {
            o.handle(key(c));
        }
        assert_eq!(
            o.handle(code(KeyCode::Enter)),
            Answer::ConflictNames {
                op: OpId(7),
                targets: vec![
                    ("/dest/a.txt".into(), "/dest/a (1).txt".into()),
                    ("/dest/b.txt".into(), "/dest/other.txt".into()),
                ]
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
                "REMOVE WAITING OPERATIONS",
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
