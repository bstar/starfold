//! Name entry for a new empty file or directory.

use starkit::chrome::overlay::{self, Anchor};
use starkit::crossterm::event::KeyEvent;
use starkit::input::{Edit, TextInput};
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;
use std::path::PathBuf;

use crate::fold::create::{self, Kind};
use crate::ui::panels::{fit, rgb};
use crate::ui::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Taken,
    Close,
    Create {
        dir: PathBuf,
        kind: Kind,
        name: String,
    },
}

#[derive(Debug)]
pub struct Create {
    pub dir: PathBuf,
    pub kind: Kind,
    pub input: TextInput,
    pub error: Option<&'static str>,
}

impl Create {
    pub fn new(dir: PathBuf, kind: Kind) -> Self {
        Self {
            dir,
            kind,
            input: TextInput::single(),
            error: None,
        }
    }

    pub fn handle(&mut self, key: KeyEvent) -> Action {
        match self.input.handle(key) {
            Edit::Submit => match create::validate_name(self.input.text()) {
                Ok(()) => Action::Create {
                    dir: self.dir.clone(),
                    kind: self.kind,
                    name: self.input.text().to_owned(),
                },
                Err(error) => {
                    self.error = Some(error);
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

pub fn rect(area: Rect) -> Rect {
    overlay::rect(area, (24, 60), 4, 3, Anchor::Centre)
}

pub fn render(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    form: &mut Create,
) -> Option<(u16, u16)> {
    let rr = rect(area);
    if rr.width < 8 || rr.height < 3 {
        return None;
    }
    let core: &starkit::theme::Theme = theme;
    let inner = overlay::render(
        rr,
        buf,
        &overlay::Overlay {
            theme: core,
            title: match form.kind {
                Kind::File => "new file",
                Kind::Directory => "new directory",
            },
            detail: None,
            footer: Some("enter create · esc cancel"),
        },
    );
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let cursor = form.input.render(
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
        buf,
        Style::default().fg(rgb(theme.fg)),
    );
    if inner.height > 1 {
        if let Some(error) = form.error {
            buf.set_string(
                inner.x,
                inner.y + 1,
                fit(error, inner.width),
                Style::default().fg(rgb(theme.fold.error_fg)),
            );
        }
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::*;
    use starkit::crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn invalid_name_stays_open_and_valid_name_submits() {
        let mut form = Create::new("/tmp".into(), Kind::Directory);
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert_eq!(form.handle(key(KeyCode::Enter)), Action::Taken);
        assert!(form.error.is_some());
        for c in "new folder".chars() {
            form.handle(key(KeyCode::Char(c)));
        }
        assert_eq!(
            form.handle(key(KeyCode::Enter)),
            Action::Create {
                dir: "/tmp".into(),
                kind: Kind::Directory,
                name: "new folder".into(),
            }
        );
    }
}
