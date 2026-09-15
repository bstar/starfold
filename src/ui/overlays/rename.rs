//! Renaming the entry under the cursor: a one-line field, pre-loaded with the
//! current name.
//!
//! Modelled on STAR/CORD's `overlays::attach`, which is the model's only
//! other bare [`TextInput`] in a box: `Edit::Submit`/`Edit::Cancel` map the
//! same way, and a refused submission stays open with a red hint rather than
//! closing, so a mistyped name is not the same as giving up on the rename.
//!
//! The one thing this adds that `attach` has no use for: the cursor starts
//! before the extension, not at the end. Most renames touch the stem and
//! leave `.toml` alone, and starting there means the common case is typed
//! without a single keystroke spent getting the cursor out of the way first.

use std::path::{Path, PathBuf};

use starkit::chrome::overlay::{self, Anchor};
use starkit::crossterm::event::KeyEvent;
use starkit::input::{Edit, TextInput};
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;

use crate::ui::panels::{fit, rgb};
use crate::ui::theme::Theme;

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Taken,
    Close,
    /// A name that passed [`validate`], paired with where it lands.
    Renamed(PathBuf),
}

#[derive(Debug)]
pub struct Rename {
    pub from: PathBuf,
    pub input: TextInput,
    /// Why the last submission was refused.
    pub error: Option<&'static str>,
}

impl Rename {
    pub fn new(from: PathBuf) -> Self {
        let name = from
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let cursor = extension_cursor(&name);
        let mut input = TextInput::single().with_text(name);
        input.set_cursor(cursor);
        Self {
            from,
            input,
            error: None,
        }
    }

    pub fn handle(&mut self, key: KeyEvent) -> Action {
        match self.input.handle(key) {
            Edit::Submit => match validate(self.input.text(), &self.from) {
                Ok(to) => Action::Renamed(to),
                Err(hint) => {
                    self.error = Some(hint);
                    Action::Taken
                }
            },
            Edit::Cancel => Action::Close,
            Edit::Consumed => {
                self.error = None;
                Action::Taken
            }
            Edit::Ignored => Action::Taken,
        }
    }
}

/// Where the cursor starts: right before the last `.`, unless that `.` opens
/// the name -- a dotfile like `.gitignore` has no extension to protect the
/// cursor from -- or there is no `.` at all.
fn extension_cursor(name: &str) -> usize {
    match name.rfind('.') {
        Some(0) | None => name.len(),
        Some(i) => i,
    }
}

/// A name worth submitting: not empty, not `.`/`..`, no path separator in it
/// (a rename stays inside its own directory; moving is what the queue is
/// for), and different from what is already there.
fn validate(name: &str, from: &Path) -> Result<PathBuf, &'static str> {
    if name.is_empty() {
        return Err("a name cannot be empty");
    }
    if name == "." || name == ".." {
        return Err("that is not a name");
    }
    if name.contains('/') {
        return Err("a name cannot contain /");
    }
    if Some(name) == from.file_name().and_then(|n| n.to_str()) {
        return Err("that is the name it already has");
    }
    Ok(from.with_file_name(name))
}

/// Where the box lands -- the same shape every overlay opens in, just tall
/// enough for the field and, when there is one, the row underneath it that
/// says why the last submission was refused.
pub fn rect(area: Rect) -> Rect {
    overlay::rect(area, (24, 60), 4, 3, Anchor::Centre)
}

/// Draw the field and hand back where the terminal's own cursor belongs, so
/// the caret the person sees is the real one rather than a drawn stand-in.
pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, r: &mut Rename) -> Option<(u16, u16)> {
    let rr = rect(area);
    if rr.width < 8 || rr.height < 3 {
        return None;
    }
    // The core theme type -- a struct literal is not a coercion site, so the
    // deref from this crate's own `Theme` is spelled out here.
    let core: &starkit::theme::Theme = theme;
    let inner = overlay::render(
        rr,
        buf,
        &overlay::Overlay {
            theme: core,
            title: "rename",
            detail: None,
            footer: Some("enter rename \u{b7} esc cancel"),
        },
    );
    if inner.width == 0 || inner.height == 0 {
        return None;
    }

    let cursor = r.input.render(
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
        buf,
        Style::default().fg(rgb(theme.fg)),
    );

    // Why the last submission was refused, if it was -- the key hints live
    // on the border footer now, so this row says nothing when there is
    // nothing to say.
    if inner.height > 1 {
        if let Some(hint) = &r.error {
            buf.set_string(
                inner.x,
                inner.y + 1,
                fit(hint, inner.width),
                Style::default().fg(rgb(theme.fold.error_fg)),
            );
        }
    }

    cursor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::tests_support::theme;
    use starkit::crossterm::event::{KeyCode, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn chars(r: &mut Rename, s: &str) {
        for c in s.chars() {
            r.handle(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn the_cursor_starts_before_the_extension() {
        let r = Rename::new(PathBuf::from("/tmp/Cargo.toml"));
        assert_eq!(r.input.cursor(), 5);
        assert_eq!(r.input.text(), "Cargo.toml");
    }

    #[test]
    fn a_dotfile_has_no_extension_to_protect() {
        let r = Rename::new(PathBuf::from("/tmp/.gitignore"));
        assert_eq!(r.input.cursor(), r.input.text().len());
    }

    #[test]
    fn a_name_with_no_dot_puts_the_cursor_at_the_end() {
        let r = Rename::new(PathBuf::from("/tmp/README"));
        assert_eq!(r.input.cursor(), r.input.text().len());
    }

    #[test]
    fn an_empty_name_is_refused_and_the_overlay_stays_open() {
        let mut r = Rename::new(PathBuf::from("/tmp/a.txt"));
        r.input.clear();
        assert_eq!(r.handle(key(KeyCode::Enter)), Action::Taken);
        assert!(r.error.is_some());
    }

    #[test]
    fn a_name_with_a_slash_is_refused() {
        let mut r = Rename::new(PathBuf::from("/tmp/a.txt"));
        r.input.clear();
        chars(&mut r, "b/c");
        assert_eq!(r.handle(key(KeyCode::Enter)), Action::Taken);
        assert!(r.error.is_some());
    }

    #[test]
    fn an_unchanged_name_is_refused() {
        let mut r = Rename::new(PathBuf::from("/tmp/a.txt"));
        assert_eq!(r.handle(key(KeyCode::Enter)), Action::Taken);
        assert!(r.error.is_some());
    }

    #[test]
    fn a_changed_name_submits_as_renamed() {
        let mut r = Rename::new(PathBuf::from("/tmp/a.txt"));
        r.input.clear();
        chars(&mut r, "b.txt");
        assert_eq!(
            r.handle(key(KeyCode::Enter)),
            Action::Renamed(PathBuf::from("/tmp/b.txt"))
        );
    }

    #[test]
    fn escape_cancels() {
        let mut r = Rename::new(PathBuf::from("/tmp/a.txt"));
        assert_eq!(r.handle(key(KeyCode::Esc)), Action::Close);
    }

    #[test]
    fn the_box_draws_its_title() {
        let t = theme("terminal");
        let area = Rect::new(0, 0, 60, 21);
        let mut buf = Buffer::empty(area);
        let mut r = Rename::new(PathBuf::from("/tmp/Cargo.toml"));
        render(area, &mut buf, &t, &mut r);
        let text: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("RENAME"), "{text}");
        assert!(text.contains("Cargo.toml"), "{text}");
    }
}
