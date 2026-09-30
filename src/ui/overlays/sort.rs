//! A direct picker for the active listing order.

use starkit::chrome::overlay::{self};
use starkit::crossterm::event::{KeyCode, KeyEvent};
use starkit::ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Clear, Widget},
};

use crate::fold::sort::{SortKey, SortOrder};
use crate::ui::panels::{fit, rgb};
use crate::ui::theme::Theme;

const KEYS: [SortKey; 7] = [
    SortKey::Name,
    SortKey::Ext,
    SortKey::Type,
    SortKey::Size,
    SortKey::Time,
    SortKey::Created,
    SortKey::Accessed,
];
const ROWS: usize = KEYS.len() + 2;

#[derive(Debug)]
pub struct Picker {
    pub order: SortOrder,
    pub cursor: usize,
}

impl Picker {
    pub fn new(order: SortOrder) -> Self {
        let cursor = KEYS.iter().position(|&key| key == order.key).unwrap_or(0);
        Self { order, cursor }
    }

    pub fn rect(area: Rect) -> Rect {
        let width = 28.min(area.width);
        let height = (ROWS as u16 + 2).min(area.height);
        Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        )
    }

    pub fn choose(&self, row: usize) -> Option<SortOrder> {
        let mut order = self.order;
        if let Some(&key) = KEYS.get(row) {
            order.key = key;
        } else if row == KEYS.len() {
            order.reverse = !order.reverse;
        } else if row == KEYS.len() + 1 {
            order.dirs_first = !order.dirs_first;
        } else {
            return None;
        }
        Some(order)
    }

    pub fn key(&mut self, key: KeyEvent) -> Option<SortOrder> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(ROWS - 1),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = ROWS - 1,
            KeyCode::Char('S' | 'r') => return self.choose(KEYS.len()),
            KeyCode::Enter | KeyCode::Char(' ') => return self.choose(self.cursor),
            _ => {}
        }
        None
    }

    fn direction_label(&self) -> &'static str {
        match (self.order.key, self.order.reverse) {
            (SortKey::Size, false) => "Low → High",
            (SortKey::Size, true) => "High → Low",
            (SortKey::Time | SortKey::Created | SortKey::Accessed, false) => "Newest → Oldest",
            (SortKey::Time | SortKey::Created | SortKey::Accessed, true) => "Oldest → Newest",
            (SortKey::Name | SortKey::Ext, false) => "A → Z",
            (SortKey::Name | SortKey::Ext, true) => "Z → A",
            (SortKey::Type, false) => "Types ↑",
            (SortKey::Type, true) => "Types ↓",
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let rect = Self::rect(area);
        Clear.render(rect, buf);
        let inner = overlay::render(
            rect,
            buf,
            &overlay::Overlay {
                theme,
                title: "sort by",
                detail: None,
                footer: None,
            },
        );
        for row in 0..ROWS.min(inner.height as usize) {
            let label = if let Some(key) = KEYS.get(row) {
                format!(
                    "{} {}",
                    if self.order.key == *key { '●' } else { ' ' },
                    key.label()
                )
            } else if row == KEYS.len() {
                format!("↕ order: {}", self.direction_label())
            } else {
                format!(
                    "{} directories first",
                    if self.order.dirs_first { '✓' } else { ' ' }
                )
            };
            let style = if row == self.cursor {
                Style::default().fg(rgb(theme.bg)).bg(rgb(theme.row_fg))
            } else {
                Style::default().fg(rgb(theme.row_fg)).bg(rgb(theme.bg))
            };
            buf.set_string(
                inner.x,
                inner.y + row as u16,
                format!(
                    "{:<width$}",
                    fit(&label, inner.width),
                    width = inner.width as usize
                ),
                style,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starkit::crossterm::event::KeyModifiers;

    #[test]
    fn picker_selects_every_key_and_toggles_order_settings() {
        let mut picker = Picker::new(SortOrder::default());
        for (row, key) in KEYS.iter().enumerate() {
            assert_eq!(picker.choose(row).unwrap().key, *key);
        }
        assert!(picker.choose(KEYS.len()).unwrap().reverse);
        assert!(!picker.choose(KEYS.len() + 1).unwrap().dirs_first);
        picker.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(picker.cursor, ROWS - 1);
        assert!(
            !picker
                .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                .unwrap()
                .dirs_first
        );
    }

    #[test]
    fn direction_toggle_names_the_current_order() {
        let mut picker = Picker::new(SortOrder {
            key: SortKey::Size,
            ..SortOrder::default()
        });
        assert_eq!(picker.direction_label(), "Low → High");
        picker.order = picker
            .key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT))
            .unwrap();
        assert_eq!(picker.direction_label(), "High → Low");
        picker.order.key = SortKey::Time;
        assert_eq!(picker.direction_label(), "Oldest → Newest");
        picker.order.reverse = false;
        assert_eq!(picker.direction_label(), "Newest → Oldest");
    }
}
