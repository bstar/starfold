//! Workspace picker and compact tab rail. All geometry is shared with clicks.
use crate::fold::tab::TabId;
use crate::ui::{
    panels::{fit, rgb},
    theme::Theme,
};
use starkit::{
    chrome::overlay,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    input::TextInput,
    ratatui::{
        buffer::Buffer,
        layout::Rect,
        style::Style,
        widgets::{Clear, Widget},
    },
};

#[derive(Debug, Clone)]
pub struct Item {
    pub id: TabId,
    pub label: String,
    pub location: String,
    pub badge: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Switch(TabId),
    New,
    Duplicate(TabId),
    Rename(TabId),
    Move(TabId, i32),
    Close(TabId),
    Reopen,
    Dismiss,
}
#[derive(Debug)]
enum Mode {
    Browse,
    Menu(TabId),
    Rename(TabId),
}
pub struct Picker {
    items: Vec<Item>,
    mode: Mode,
    input: TextInput,
    cursor: usize,
}
pub enum Answer {
    Action(Action),
    Renamed(TabId, String),
    Consumed,
}

impl Picker {
    pub fn new(items: Vec<Item>, active: TabId) -> Self {
        let cursor = items.iter().position(|i| i.id == active).unwrap_or(0);
        Self {
            items,
            mode: Mode::Browse,
            input: TextInput::single(),
            cursor,
        }
    }
    pub fn menu(items: Vec<Item>, id: TabId) -> Self {
        let mut p = Self::new(items, id);
        p.mode = Mode::Menu(id);
        p.cursor = 0;
        p
    }
    fn matches(&self) -> Vec<&Item> {
        let query = self.input.text().to_lowercase();
        self.items
            .iter()
            .filter(|i| {
                format!("{} {}", i.label, i.location)
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }
    fn menu_actions(id: TabId) -> [(Action, &'static str); 7] {
        [
            (Action::New, "New tab"),
            (Action::Duplicate(id), "Duplicate tab"),
            (Action::Rename(id), "Rename tab…"),
            (Action::Move(id, -1), "Move left"),
            (Action::Move(id, 1), "Move right"),
            (Action::Close(id), "Close tab"),
            (Action::Reopen, "Reopen closed tab"),
        ]
    }
    fn choose(&mut self) -> Answer {
        match self.mode {
            Mode::Browse => self
                .matches()
                .get(self.cursor)
                .map_or(Answer::Consumed, |i| Answer::Action(Action::Switch(i.id))),
            Mode::Menu(id) => {
                let action = Self::menu_actions(id)[self.cursor.min(6)].0;
                if action == Action::Rename(id) {
                    self.mode = Mode::Rename(id);
                    let name = self
                        .items
                        .iter()
                        .find(|i| i.id == id)
                        .map(|i| i.label.clone())
                        .unwrap_or_default();
                    self.input = TextInput::single().with_text(name);
                    Answer::Consumed
                } else {
                    Answer::Action(action)
                }
            }
            Mode::Rename(id) => Answer::Renamed(id, self.input.text().trim().to_owned()),
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Answer {
        if key.code == KeyCode::Esc {
            return Answer::Action(Action::Dismiss);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Answer::Action(Action::Dismiss);
        }
        if key.code == KeyCode::Enter {
            return self.choose();
        }
        if matches!(self.mode, Mode::Rename(_)) {
            self.input.handle(key);
            return Answer::Consumed;
        }
        let count = match self.mode {
            Mode::Browse => self.matches().len(),
            _ => 7,
        };
        match key.code {
            KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down => self.cursor = (self.cursor + 1).min(count.saturating_sub(1)),
            KeyCode::Right if matches!(self.mode, Mode::Browse) => {
                if let Some(item) = self.matches().get(self.cursor) {
                    self.mode = Mode::Menu(item.id);
                    self.cursor = 0;
                }
            }
            _ if matches!(self.mode, Mode::Browse) => {
                self.input.handle(key);
                self.cursor = 0;
            }
            _ => {}
        }
        Answer::Consumed
    }
    fn rect(area: Rect) -> Rect {
        let w = area.width.min(72);
        let h = area.height.min(14);
        Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
            w,
            h,
        )
    }
    pub fn click(&mut self, area: Rect, x: u16, y: u16) -> Answer {
        let r = Self::rect(area);
        if !r.contains((x, y).into()) {
            return Answer::Action(Action::Dismiss);
        }
        let start = r.y
            + if matches!(self.mode, Mode::Browse) {
                3
            } else {
                1
            };
        if y >= start && y < r.bottom().saturating_sub(1) && !matches!(self.mode, Mode::Rename(_)) {
            let visible = usize::from(r.height.saturating_sub(
                if matches!(self.mode, Mode::Browse) {
                    4
                } else {
                    2
                },
            ));
            let first = self.cursor.saturating_sub(visible.saturating_sub(1));
            let row = first + usize::from(y - start);
            let count = if matches!(self.mode, Mode::Browse) {
                self.matches().len()
            } else {
                7
            };
            if row < count {
                self.cursor = row;
                return self.choose();
            }
        }
        Answer::Consumed
    }
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let r = Self::rect(area);
        Clear.render(r, buf);
        let inner = overlay::render(
            r,
            buf,
            &overlay::Overlay {
                theme,
                title: "tabs",
                detail: None,
                footer: Some(match self.mode {
                    Mode::Browse => "enter switch · → actions · esc close",
                    Mode::Menu(_) => "enter choose · esc close",
                    _ => "enter rename · empty restores auto name",
                }),
            },
        );
        let normal = Style::default().fg(rgb(theme.fg));
        if matches!(self.mode, Mode::Browse | Mode::Rename(_)) {
            let prefix = if matches!(self.mode, Mode::Browse) {
                "find: "
            } else {
                "name: "
            };
            buf.set_string(inner.x, inner.y, fit(prefix, inner.width), normal);
            if let Some((x, y)) = self.input.render(
                Rect::new(inner.x + 6, inner.y, inner.width.saturating_sub(6), 1),
                buf,
                normal,
            ) {
                if buf.area.contains((x, y).into()) {
                    buf[(x, y)].set_style(
                        Style::default()
                            .fg(rgb(theme.row_cursor_fg))
                            .bg(rgb(theme.row_cursor_bg)),
                    );
                }
            }
        }
        let labels: Vec<String> = match self.mode {
            Mode::Browse => self
                .matches()
                .iter()
                .map(|i| format!("{} {}  {}", i.label, i.badge, i.location))
                .collect(),
            Mode::Menu(id) => Self::menu_actions(id)
                .iter()
                .map(|(_, name)| name.to_string())
                .collect(),
            _ => vec![],
        };
        let offset = usize::from(matches!(self.mode, Mode::Browse)) * 2;
        let visible = usize::from(inner.height).saturating_sub(offset);
        let start = self.cursor.saturating_sub(visible.saturating_sub(1));
        for (row, label) in labels.iter().enumerate().skip(start).take(visible) {
            let style = if row == self.cursor {
                normal
                    .fg(rgb(theme.row_cursor_fg))
                    .bg(rgb(theme.row_cursor_bg))
            } else {
                normal
            };
            buf.set_string(
                inner.x,
                inner.y + offset as u16 + (row - start) as u16,
                fit(label, inner.width),
                style,
            );
        }
    }
}

#[derive(Clone, Copy)]
pub enum Hit {
    Tab(TabId),
    New,
    Previous,
    Next,
    Picker,
}
pub fn rail(
    area: Rect,
    items: &[Item],
    active: TabId,
    offset: &mut usize,
    buf: &mut Buffer,
    theme: &Theme,
) -> Vec<(Rect, Hit)> {
    let active_index = items.iter().position(|i| i.id == active).unwrap_or(0);
    *offset = (*offset).min(active_index);
    let capacity = usize::from(area.width.saturating_sub(13) / 18).max(1);
    if active_index >= *offset + capacity {
        *offset = active_index + 1 - capacity;
    }
    let normal = Style::default().fg(rgb(theme.dim));
    buf.set_string(area.x, area.y, "─".repeat(area.width as usize), normal);
    let mut hits = vec![];
    let mut x = area.x;
    for (i, item) in items.iter().enumerate().skip(*offset).take(capacity) {
        let w = (area.right().saturating_sub(x + 13)).min(18);
        if w < 5 {
            break;
        }
        let badge = if item.badge.contains('!') {
            "!"
        } else {
            item.badge.as_str()
        };
        let badge: String = badge.chars().take(8).collect();
        let suffix = if badge.is_empty() {
            "]".to_string()
        } else {
            format!(" {badge}]")
        };
        let prefix = format!("[{} ", i + 1);
        let room = w.saturating_sub(prefix.chars().count() as u16 + suffix.chars().count() as u16);
        let text = format!("{prefix}{}{suffix}", fit(&item.label, room));
        let style = if item.id == active {
            normal
                .fg(rgb(theme.row_cursor_fg))
                .bg(rgb(theme.row_cursor_bg))
        } else {
            normal
        };
        let rect = Rect::new(x, area.y, w, 1);
        buf.set_string(x, area.y, fit(&text, w), style);
        hits.push((rect, Hit::Tab(item.id)));
        x += w;
    }
    for (text, hit) in [
        ("‹ ", Hit::Previous),
        ("› ", Hit::Next),
        ("… ", Hit::Picker),
        ("[+]", Hit::New),
    ] {
        let w = text.chars().count() as u16;
        if x + w <= area.right() {
            buf.set_string(x, area.y, text, normal);
            hits.push((Rect::new(x, area.y, w, 1), hit));
            x += w;
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    fn items() -> Vec<Item> {
        (0..20)
            .map(|n| Item {
                id: TabId(n),
                label: format!("project {n}"),
                location: format!("/projects/{n}"),
                badge: if n == 19 { "!".into() } else { String::new() },
            })
            .collect()
    }
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    #[test]
    fn picker_searches_paths_and_renames_through_actions() {
        let mut picker = Picker::new(items(), TabId(0));
        for c in "/projects/19".chars() {
            picker.key(key(KeyCode::Char(c)));
        }
        assert!(matches!(
            picker.key(key(KeyCode::Enter)),
            Answer::Action(Action::Switch(TabId(19)))
        ));
        picker.key(key(KeyCode::Right));
        picker.key(key(KeyCode::Down));
        picker.key(key(KeyCode::Down));
        assert!(matches!(picker.key(key(KeyCode::Enter)), Answer::Consumed));
        assert!(
            matches!(picker.key(key(KeyCode::Enter)), Answer::Renamed(TabId(19), name) if name == "project 19")
        );
    }
    #[test]
    fn overflow_keeps_active_tab_and_failure_badge_visible_at_floor() {
        let area = Rect::new(0, 0, 60, 1);
        let mut buf = Buffer::empty(area);
        let mut offset = 0;
        let (theme, _) = crate::ui::theme::registry().resolve_named("catppuccin-latte");
        let hits = rail(area, &items(), TabId(19), &mut offset, &mut buf, &theme);
        assert!(offset > 0);
        assert!(hits
            .iter()
            .any(|(_, hit)| matches!(hit, Hit::Tab(TabId(19)))));
        assert!(hits.iter().any(|(_, hit)| matches!(hit, Hit::New)));
        let text: String = (0..60).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(text.contains('!'), "{text}");
        for (rect, _) in hits {
            assert!(rect.right() <= area.right());
        }
    }
    #[test]
    fn picker_clicks_the_same_scrolled_row_it_draws() {
        let area = Rect::new(0, 0, 60, 21);
        let mut picker = Picker::new(items(), TabId(19));
        let mut buf = Buffer::empty(area);
        let (theme, _) = crate::ui::theme::registry().resolve_named("terminal");
        picker.render(area, &mut buf, &theme);
        let row = (0..21)
            .find(|y| {
                (0..60)
                    .map(|x| buf[(x, *y)].symbol())
                    .collect::<String>()
                    .contains("project 19")
            })
            .unwrap();
        assert!(matches!(
            picker.click(area, 10, row),
            Answer::Action(Action::Switch(TabId(19)))
        ));
    }
}
