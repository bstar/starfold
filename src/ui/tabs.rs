//! Workspace picker and compact tab rail. All geometry is shared with clicks.
use super::popup::{Answer as PopupAnswer, Entry, Popup};
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
    menu: Option<Popup<Action>>,
    menu_from_list: bool,
    reopen: bool,
}
pub enum Answer {
    Action(Action),
    Renamed(TabId, String),
    Consumed,
}

impl Picker {
    #[cfg(feature = "terminal-graphics")]
    pub fn graphical_rects(&mut self, area: Rect) -> Vec<Rect> {
        self.menu
            .as_mut()
            .map_or_else(|| vec![Self::rect(area)], |menu| menu.graphical_rects(area))
    }

    pub fn new(items: Vec<Item>, active: TabId) -> Self {
        let cursor = items.iter().position(|i| i.id == active).unwrap_or(0);
        Self {
            items,
            mode: Mode::Browse,
            input: TextInput::single(),
            cursor,
            menu: None,
            menu_from_list: true,
            reopen: false,
        }
    }
    pub fn menu(items: Vec<Item>, id: TabId) -> Self {
        let mut p = Self::new(items, id);
        p.menu_from_list = false;
        p.open_menu(id, (0, 0));
        p
    }
    pub fn configure(&mut self, anchor: (u16, u16), reopen: bool) {
        self.reopen = reopen;
        if let Mode::Menu(id) = self.mode {
            self.open_menu(id, anchor);
        }
    }
    pub fn set_anchor(&mut self, anchor: (u16, u16)) {
        if let Some(menu) = &mut self.menu {
            menu.anchor = anchor;
        }
    }
    fn open_menu(&mut self, id: TabId, anchor: (u16, u16)) {
        let index = self.items.iter().position(|i| i.id == id).unwrap_or(0);
        self.mode = Mode::Menu(id);
        self.menu = Some(Popup::new(
            vec![
                Entry::action("New tab", Action::New),
                Entry::action("Duplicate tab", Action::Duplicate(id)),
                Entry::action("Rename tab…", Action::Rename(id)),
                Entry::Separator,
                Entry::submenu(
                    "Position",
                    vec![
                        Entry::action("Move left", Action::Move(id, -1)).enabled(index > 0),
                        Entry::action("Move right", Action::Move(id, 1))
                            .enabled(index + 1 < self.items.len()),
                    ],
                )
                .enabled(self.items.len() > 1),
                Entry::Separator,
                Entry::action("Close tab", Action::Close(id)).enabled(self.items.len() > 1),
                Entry::action("Reopen closed tab", Action::Reopen).enabled(self.reopen),
            ],
            anchor,
        ));
    }
    fn menu_answer(&mut self, answer: PopupAnswer<Action>) -> Answer {
        match answer {
            PopupAnswer::Selected(Action::Rename(id)) => {
                self.mode = Mode::Rename(id);
                self.menu = None;
                let name = self
                    .items
                    .iter()
                    .find(|i| i.id == id)
                    .map(|i| i.label.clone())
                    .unwrap_or_default();
                self.input = TextInput::single().with_text(name);
                Answer::Consumed
            }
            PopupAnswer::Selected(a) => Answer::Action(a),
            PopupAnswer::Dismissed if self.menu_from_list => {
                self.mode = Mode::Browse;
                self.menu = None;
                Answer::Consumed
            }
            PopupAnswer::Dismissed => Answer::Action(Action::Dismiss),
            PopupAnswer::Consumed => Answer::Consumed,
        }
    }
    fn choose(&mut self) -> Answer {
        match self.mode {
            Mode::Browse => self
                .items
                .get(self.cursor)
                .map_or(Answer::Consumed, |i| Answer::Action(Action::Switch(i.id))),
            Mode::Rename(id) => Answer::Renamed(id, self.input.text().trim().to_owned()),
            Mode::Menu(_) => Answer::Consumed,
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Answer {
        if let Some(menu) = &mut self.menu {
            let answer = menu.key(key);
            return self.menu_answer(answer);
        }
        if key.code == KeyCode::Esc
            || (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c'))
        {
            return Answer::Action(Action::Dismiss);
        }
        if key.code == KeyCode::Enter {
            return self.choose();
        }
        if matches!(self.mode, Mode::Rename(_)) {
            self.input.handle(key);
            return Answer::Consumed;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(self.items.len().saturating_sub(1))
            }
            KeyCode::Right => {
                if let Some(item) = self.items.get(self.cursor) {
                    self.open_menu(item.id, (0, 0));
                }
            }
            _ => {}
        }
        Answer::Consumed
    }
    pub fn hover(&mut self, area: Rect, x: u16, y: u16) {
        if let Some(menu) = &mut self.menu {
            menu.hover(area, x, y);
        }
    }
    pub fn outside_click(&mut self, area: Rect, x: u16, y: u16) -> Answer {
        if let Some(menu) = &mut self.menu {
            if !menu.contains(area, x, y) {
                return self.menu_answer(PopupAnswer::Dismissed);
            }
        }
        Answer::Consumed
    }
    pub fn tick(&mut self, now: std::time::Instant) {
        if let Some(menu) = &mut self.menu {
            menu.tick(now);
        }
    }
    pub fn wheel(&mut self, area: Rect, x: u16, y: u16, down: bool) {
        if let Some(menu) = &mut self.menu {
            menu.wheel(area, x, y, down);
        } else if matches!(self.mode, Mode::Browse) {
            self.key(KeyEvent::new(
                if down { KeyCode::Down } else { KeyCode::Up },
                KeyModifiers::NONE,
            ));
        }
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
        if let Some(menu) = &mut self.menu {
            let answer = menu.click(area, x, y);
            return self.menu_answer(answer);
        }
        let r = Self::rect(area);
        if !r.contains((x, y).into()) {
            return Answer::Action(Action::Dismiss);
        }
        let start = r.y + 1;
        if y >= start && y < r.bottom().saturating_sub(1) && !matches!(self.mode, Mode::Rename(_)) {
            let visible = usize::from(r.height.saturating_sub(2));
            let first = self.cursor.saturating_sub(visible.saturating_sub(1));
            let row = first + usize::from(y - start);
            let count = self.items.len();
            if row < count {
                self.cursor = row;
                return self.choose();
            }
        }
        Answer::Consumed
    }
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let r = Self::rect(area);
        if !self.menu_from_list {
            if let Some(menu) = &mut self.menu {
                menu.render(area, buf, theme);
                return;
            }
        }
        Clear.render(r, buf);
        let inner = overlay::render(
            r,
            buf,
            &overlay::Overlay {
                theme,
                title: "tabs",
                detail: None,
                footer: Some(match self.mode {
                    Mode::Browse | Mode::Menu(_) => "enter switch · → actions · esc close",
                    _ => "enter rename · empty restores auto name",
                }),
            },
        );
        let normal = Style::default().fg(rgb(theme.fg));
        if matches!(self.mode, Mode::Rename(_)) {
            buf.set_string(inner.x, inner.y, fit("name: ", inner.width), normal);
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
            Mode::Browse | Mode::Menu(_) => self
                .items
                .iter()
                .map(|i| format!("{} {}  {}", i.label, i.badge, i.location))
                .collect(),
            _ => vec![],
        };
        let visible = usize::from(inner.height);
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
                inner.y + (row - start) as u16,
                fit(label, inner.width),
                style,
            );
        }
        if let Some(menu) = &mut self.menu {
            let visible = usize::from(inner.height);
            let start = self.cursor.saturating_sub(visible.saturating_sub(1));
            menu.anchor = (inner.x + 2, inner.y + (self.cursor - start) as u16);
            menu.render(area, buf, theme);
        }
    }
}

#[derive(Clone, Copy)]
pub enum Hit {
    Tab(TabId),
    Close(TabId),
    New,
    Previous,
    Next,
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
    let capacity = usize::from(area.width.saturating_sub(7) / 24).max(1);
    if active_index >= *offset + capacity {
        *offset = active_index + 1 - capacity;
    }
    let normal = Style::default().fg(rgb(theme.dim)).bg(rgb(theme.bg));
    let active_bg = theme.panel_bg.mix(theme.fg, 0.12);
    let active_style = normal
        .fg(rgb(theme.fg.ensure_contrast(active_bg, 4.5)))
        .bg(rgb(active_bg));
    buf.set_string(area.x, area.y, " ".repeat(area.width as usize), normal);
    let mut hits = vec![];
    let mut x = area.x;
    for (i, item) in items.iter().enumerate().skip(*offset).take(capacity) {
        let w = (area.right().saturating_sub(x + 7)).min(24);
        if w < 9 {
            break;
        }
        let badge = if item.badge.contains('!') {
            "!"
        } else {
            item.badge.as_str()
        };
        let badge: String = badge.chars().take(8).collect();
        let suffix = if badge.is_empty() {
            String::new()
        } else {
            format!(" {badge}")
        };
        let prefix = format!("{} ", i + 1);
        // Three cells of padding on each side, followed by a gap between tabs.
        let room =
            w.saturating_sub(7 + prefix.chars().count() as u16 + suffix.chars().count() as u16);
        let text = format!("{prefix}{}{suffix}", fit(&item.label, room));
        let style = if item.id == active {
            active_style
        } else {
            normal
        };
        let rect = Rect::new(x, area.y, w, 1);
        if item.id == active {
            buf.set_string(x, area.y, " ".repeat((w - 1) as usize), active_style);
        }
        let label = fit(&text, w - 7);
        buf.set_string(x + 3, area.y, label.trim_end(), style);
        buf.set_string(x + w - 3, area.y, "×", style);
        // The close target must win over the tab's surrounding click area.
        hits.push((Rect::new(x + w - 4, area.y, 3, 1), Hit::Close(item.id)));
        hits.push((rect, Hit::Tab(item.id)));
        x += w;
    }
    for (text, hit) in [("‹ ", Hit::Previous), ("› ", Hit::Next), (" + ", Hit::New)] {
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
    fn picker_switches_tabs_and_renames_through_actions() {
        let mut picker = Picker::new(items(), TabId(0));
        for _ in 0..19 {
            picker.key(key(KeyCode::Down));
        }
        // Typing in the list does not filter or reset its selection.
        picker.key(key(KeyCode::Char('x')));
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
        let (active_rect, _) = hits
            .iter()
            .find(|(_, hit)| matches!(hit, Hit::Tab(TabId(19))))
            .unwrap();
        assert_eq!(buf[(active_rect.x, 0)].symbol(), " ");
        assert_eq!(buf[(active_rect.x + 1, 0)].symbol(), " ");
        assert_eq!(buf[(active_rect.x + 2, 0)].symbol(), " ");
        assert_eq!(buf[(active_rect.x, 0)].bg, buf[(active_rect.x + 3, 0)].bg);
        assert_ne!(
            buf[(active_rect.x, 0)].bg,
            buf[(active_rect.right() - 1, 0)].bg
        );
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
