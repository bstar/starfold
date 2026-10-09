//! Worker-backed recovery picker. The UI never inspects Trash on disk.
use super::rename::{self, Rename};
use crate::{
    fold::{
        recovery::{Item, Mode, Record},
        State,
    },
    ui::{
        panels::{fit, rgb},
        theme::Theme,
    },
};
use starkit::{
    chrome::overlay::{self, Anchor},
    crossterm::event::{KeyCode, KeyEvent},
    input::TextInput,
    ratatui::{buffer::Buffer, layout::Rect, style::Style},
};
use std::path::PathBuf;
#[derive(Debug)]
pub struct Browser {
    pub home: PathBuf,
    pub mode: Mode,
    pub items: Vec<Item>,
    pub cursor: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub rename: Option<Rename>,
}
pub enum Action {
    Taken,
    Refresh,
    Submit(Record, PathBuf),
}
impl Browser {
    pub fn bounds(&self, area: Rect) -> Rect {
        if self.rename.is_some() {
            rename::rect(area)
        } else {
            rect(area)
        }
    }

    pub fn footer(&self) -> &'static str {
        self.rename.as_ref().map_or(
            "enter recover · r refresh · j/k select · esc close",
            Rename::footer,
        )
    }

    pub fn new(mode: Mode) -> Self {
        Self {
            home: PathBuf::new(),
            mode,
            items: vec![],
            cursor: 0,
            loading: true,
            error: None,
            rename: None,
        }
    }
    pub fn sync(&mut self, state: &State) {
        self.home = state.home.clone();
        self.loading = state.recovery_loading;
        self.error = state.recovery_error.clone();
        self.items = state.recovery_items.clone();
        self.cursor = self.cursor.min(self.items.len().saturating_sub(1));
    }
    pub fn handle(&mut self, key: KeyEvent) -> Action {
        if let Some(form) = self.rename.as_mut() {
            return match form.handle(key) {
                rename::Action::Renamed(target) => {
                    Action::Submit(self.items[self.cursor].record.clone(), target)
                }
                rename::Action::Close => {
                    self.rename = None;
                    Action::Taken
                }
                _ => Action::Taken,
            };
        }
        match key.code {
            KeyCode::Char('r') => Action::Refresh,
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                Action::Taken
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(self.items.len().saturating_sub(1));
                Action::Taken
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(5);
                Action::Taken
            }
            KeyCode::PageDown => {
                self.cursor = (self.cursor + 5).min(self.items.len().saturating_sub(1));
                Action::Taken
            }
            KeyCode::Enter if !self.loading => {
                if let Some(item) = self.items.get(self.cursor) {
                    if item.collision && self.mode == Mode::Trash {
                        let mut form = Rename::new(item.record.original.clone());
                        form.recovery = true;
                        form.input = TextInput::single().with_text(item.suggestion.clone());
                        form.detail =
                            Some("Original name exists. Restore with another name.".into());
                        self.rename = Some(form);
                        Action::Taken
                    } else {
                        Action::Submit(item.record.clone(), item.record.original.clone())
                    }
                } else {
                    Action::Taken
                }
            }
            _ => Action::Taken,
        }
    }
}
pub fn rect(area: Rect) -> Rect {
    overlay::rect(area, (44, 96), 18, 8, Anchor::Centre)
}
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    browser: &mut Browser,
) -> Option<(u16, u16)> {
    if let Some(form) = browser.rename.as_mut() {
        return rename::render(area, buf, theme, form);
    }
    let title = if browser.mode == Mode::Trash {
        "Trash · restore"
    } else {
        "Undo · this session"
    };
    let inner = overlay::render(
        rect(area),
        buf,
        &overlay::Overlay {
            theme,
            title,
            detail: None,
            footer: Some(browser.footer()),
        },
    );
    let ordinary = Style::default().fg(rgb(theme.fg));
    if browser.loading {
        buf.set_string(inner.x, inner.y, fit("Reading…", inner.width), ordinary);
    } else if let Some(error) = &browser.error {
        buf.set_string(inner.x, inner.y, fit(error, inner.width), ordinary);
    } else if browser.items.is_empty() {
        let empty = if browser.mode == Mode::Trash {
            if cfg!(target_os = "macos") {
                "No STAR/FOLD Trash records."
            } else {
                "No recoverable Trash items."
            }
        } else {
            "No moves or renames available to undo."
        };
        buf.set_string(inner.x, inner.y, fit(empty, inner.width), ordinary);
    } else {
        let rows = (inner.height as usize / 2).max(1);
        let start = browser.cursor.saturating_sub(rows - 1);
        for (index, item) in browser.items.iter().enumerate().skip(start).take(rows) {
            let selected = index == browser.cursor;
            let style = if selected {
                ordinary
                    .bg(rgb(theme.fold.marked_bg))
                    .fg(rgb(theme.fold.marked_fg))
            } else {
                ordinary
            };
            let y = inner.y + ((index - start) * 2) as u16;
            let name = item
                .record
                .original
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let suffix = if item.collision {
                " · original name exists"
            } else {
                ""
            };
            buf.set_string(
                inner.x,
                y,
                fit(
                    &format!("{} {name}{suffix}", if selected { "›" } else { " " }),
                    inner.width,
                ),
                style,
            );
            if y + 1 < inner.bottom() {
                buf.set_string(
                    inner.x,
                    y + 1,
                    fit(
                        &format!(
                            "  → {}",
                            match item.record.original.strip_prefix(&browser.home) {
                                Ok(relative) if !browser.home.as_os_str().is_empty() =>
                                    format!("~/{}", relative.display()),
                                _ => item.record.original.display().to_string(),
                            }
                        ),
                        inner.width,
                    ),
                    style,
                );
            }
        }
    }
    None
}
