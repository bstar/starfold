//! Prompt for recursive filename or text-content search.

use starcore::chrome::overlay::{self, Anchor};
use starcore::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use starcore::input::{Edit, TextInput};
use starcore::ratatui::buffer::Buffer;
use starcore::ratatui::layout::Rect;
use starcore::ratatui::style::Style;

use crate::fold::search::Mode;
use crate::ui::panels::{fit, rgb};
use crate::ui::theme::Theme;

#[derive(Debug)]
pub struct Search {
    pub input: TextInput,
    pub mode: Mode,
    pub error: Option<&'static str>,
}

impl Search {
    pub fn new(query: &str, mode: Mode) -> Self {
        Self {
            input: TextInput::single().with_text(query),
            mode,
            error: None,
        }
    }

    pub fn handle(&mut self, key: KeyEvent) -> Option<(String, Mode)> {
        if key.code == KeyCode::Tab && key.modifiers == KeyModifiers::NONE {
            self.mode = match self.mode {
                Mode::Names => Mode::Contents,
                Mode::Contents => Mode::Names,
            };
            self.error = None;
            return None;
        }
        match self.input.handle(key) {
            Edit::Submit if !self.input.text().trim().is_empty() => {
                Some((self.input.text().trim().to_string(), self.mode))
            }
            Edit::Submit => {
                self.error = Some("enter a search term");
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

pub const FOOTER: &str = "tab names/contents · enter search · esc cancel";

pub fn rect(area: Rect) -> Rect {
    overlay::rect(
        area,
        (30, 64),
        4 + starcore::chrome::frame::extra_rows(),
        3,
        Anchor::Centre,
    )
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
    let core: &starcore::theme::Theme = theme;
    let inner = overlay::render(
        rr,
        buf,
        &overlay::Overlay {
            theme: core,
            title: match form.mode {
                Mode::Names => "search filenames below here",
                Mode::Contents => "search file contents below here",
            },
            detail: None,
            footer: Some(FOOTER),
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
