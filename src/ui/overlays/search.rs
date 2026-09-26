//! Prompt for a recursive filename search.

use starkit::chrome::overlay::{self, Anchor};
use starkit::crossterm::event::KeyEvent;
use starkit::input::{Edit, TextInput};
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;

use crate::ui::panels::{fit, rgb};
use crate::ui::theme::Theme;

#[derive(Debug)]
pub struct Search {
    pub input: TextInput,
    pub error: Option<&'static str>,
}

impl Search {
    pub fn new(query: &str) -> Self {
        Self {
            input: TextInput::single().with_text(query),
            error: None,
        }
    }

    pub fn handle(&mut self, key: KeyEvent) -> Option<String> {
        match self.input.handle(key) {
            Edit::Submit if !self.input.text().trim().is_empty() => {
                Some(self.input.text().trim().to_string())
            }
            Edit::Submit => {
                self.error = Some("enter a filename");
                None
            }
            Edit::Consumed => {
                self.error = None;
                None
            }
            Edit::Cancel | Edit::Ignored => None,
        }
    }
}

pub fn rect(area: Rect) -> Rect {
    overlay::rect(area, (30, 64), 4, 3, Anchor::Centre)
}

pub fn render(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    form: &mut Search,
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
            title: "search filenames below here",
            detail: None,
            footer: Some("enter search · esc cancel"),
        },
    );
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let cursor = form.input.render(
        Rect { height: 1, ..inner },
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
