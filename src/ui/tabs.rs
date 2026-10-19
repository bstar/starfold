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

    #[cfg(feature = "terminal-graphics")]
    pub fn pointer_regions(&mut self, area: Rect) -> Vec<Rect> {
        self.menu
            .as_mut()
            .map_or_else(Vec::new, |m| m.pointer_regions(area))
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

#[derive(Debug, Clone, Copy)]
pub enum Hit {
    Tab(TabId),
    Close(TabId),
    New,
    Manage,
    NavigationMode,
    Previous,
    Next,
}
/// Geometry shared by native painting and pointer handling. Keep the active
/// workspace reachable when a restored session contains more items than fit.
pub fn numbered(
    area: Rect,
    items: &[Item],
    active: TabId,
    offset: &mut usize,
    _buf: &mut Buffer,
    _theme: &Theme,
) -> Vec<(Rect, Hit)> {
    let active_index = items.iter().position(|i| i.id == active).unwrap_or(0);
    *offset = (*offset).min(active_index);
    let available = area.width.saturating_sub(24);
    let width = |item: &Item| {
        (starkit::wrap::width_of(&item.label) + 14)
            .clamp(20, 48)
            .min(available)
    };
    while *offset < active_index
        && items[*offset..=active_index]
            .iter()
            .map(|i| u32::from(width(i)))
            .sum::<u32>()
            > u32::from(available)
    {
        *offset += 1;
    }
    let mut hits = vec![];
    let mut x = area.x;
    for item in items.iter().skip(*offset) {
        let w = width(item);
        if w == 0 || x + w > area.x + available {
            break;
        }
        hits.push((Rect::new(x, area.y, w, area.height), Hit::Tab(item.id)));
        x += w;
    }
    if area.width >= 24 {
        hits.push((
            Rect::new(area.right() - 24, area.y, 12, area.height),
            Hit::NavigationMode,
        ));
        hits.push((
            Rect::new(area.right() - 12, area.y, 4, area.height),
            Hit::New,
        ));
        hits.push((
            Rect::new(area.right() - 8, area.y, 8, area.height),
            Hit::Manage,
        ));
    }
    hits
}

pub fn sidebar(
    area: Rect,
    items: &[Item],
    active: TabId,
    offset: &mut usize,
    _buf: &mut Buffer,
    _theme: &Theme,
) -> Vec<(Rect, Hit)> {
    let capacity = usize::from(area.height.saturating_sub(6) / 2).max(1);
    let active_index = items.iter().position(|i| i.id == active).unwrap_or(0);
    *offset = (*offset).min(active_index);
    if active_index >= *offset + capacity {
        *offset = active_index + 1 - capacity;
    }
    let mut hits = vec![];
    for (row, item) in items.iter().skip(*offset).take(capacity).enumerate() {
        hits.push((
            Rect::new(area.x, area.y + 2 + row as u16 * 2, area.width, 2),
            Hit::Tab(item.id),
        ));
    }
    hits.push((
        Rect::new(area.x, area.bottom() - 4, area.width, 2),
        Hit::NavigationMode,
    ));
    hits.push((
        Rect::new(area.x, area.bottom() - 2, area.width / 2, 2),
        Hit::New,
    ));
    hits.push((
        Rect::new(
            area.x + area.width / 2,
            area.bottom() - 2,
            area.width - area.width / 2,
            2,
        ),
        Hit::Manage,
    ));
    hits
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
    let native = area.height > 1;
    let controls_width = if native { 3 } else { 7 };
    let tab_width = |item: &Item| {
        if native {
            (starkit::wrap::width_of(&item.label) + 8).clamp(14, 36)
        } else {
            24
        }
    };
    if native {
        while *offset < active_index
            && items[*offset..=active_index]
                .iter()
                .map(|item| u32::from(tab_width(item)))
                .sum::<u32>()
                > u32::from(area.width.saturating_sub(controls_width))
        {
            *offset += 1;
        }
    }
    let capacity = if native {
        let mut used: u16 = 0;
        items
            .iter()
            .skip(*offset)
            .take_while(|item| {
                used = used.saturating_add(tab_width(item));
                used <= area.width.saturating_sub(controls_width)
            })
            .count()
            .max(1)
    } else {
        usize::from(area.width.saturating_sub(controls_width) / 24).max(1)
    };
    if !native && active_index >= *offset + capacity {
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
        let w = (area.right().saturating_sub(x + controls_width)).min(tab_width(item));
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
        let rect = Rect::new(x, area.y, w, area.height);
        if item.id == active {
            buf.set_string(x, area.y, " ".repeat((w - 1) as usize), active_style);
        }
        let label = fit(&text, w - 7);
        buf.set_string(x + 3, area.y, label.trim_end(), style);
        buf.set_string(x + w - 3, area.y, "×", style);
        // The close target must win over the tab's surrounding click area.
        hits.push((
            Rect::new(x + w - 4, area.y, 3, area.height),
            Hit::Close(item.id),
        ));
        hits.push((rect, Hit::Tab(item.id)));
        x += w;
    }
    for (text, hit) in [("‹ ", Hit::Previous), ("› ", Hit::Next), (" + ", Hit::New)] {
        if native && matches!(hit, Hit::Previous | Hit::Next) {
            continue;
        }
        let w = if native {
            3
        } else {
            text.chars().count() as u16
        };
        if x + w <= area.right() {
            buf.set_string(x, area.y, text, normal);
            hits.push((Rect::new(x, area.y, w, area.height), hit));
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
    fn native_navigation_keeps_active_workspace_and_management_reachable() {
        let (theme, _) = crate::ui::theme::registry().resolve_named("kanagawa");
        for (area, render) in [
            (
                Rect::new(0, 0, 60, 2),
                numbered
                    as fn(
                        Rect,
                        &[Item],
                        TabId,
                        &mut usize,
                        &mut Buffer,
                        &Theme,
                    ) -> Vec<(Rect, Hit)>,
            ),
            (
                Rect::new(0, 0, 22, 40),
                sidebar
                    as fn(
                        Rect,
                        &[Item],
                        TabId,
                        &mut usize,
                        &mut Buffer,
                        &Theme,
                    ) -> Vec<(Rect, Hit)>,
            ),
        ] {
            let mut buf = Buffer::empty(area);
            let mut offset = 0;
            for active in [TabId(19), TabId(0), TabId(10)] {
                let hits = render(area, &items(), active, &mut offset, &mut buf, &theme);
                assert!(hits
                    .iter()
                    .any(|(_, h)| matches!(h, Hit::Tab(id) if *id == active)));
                assert!(hits.iter().any(|(_, h)| matches!(h, Hit::New)));
                assert!(hits.iter().any(|(_, h)| matches!(h, Hit::Manage)));
                assert!(hits.iter().any(|(_, h)| matches!(h, Hit::NavigationMode)));
                assert!(!hits.iter().any(|(_, h)| matches!(h, Hit::Close(_))));
                for (i, (r, _)) in hits.iter().enumerate() {
                    assert!(
                        area.contains((r.x, r.y).into())
                            && r.right() <= area.right()
                            && r.bottom() <= area.bottom()
                    );
                    assert!(hits[i + 1..]
                        .iter()
                        .all(|(other, _)| r.intersection(*other).is_empty()));
                }
            }
        }
    }
    #[test]
    fn native_tabs_keep_active_and_close_targets_visible_with_mixed_widths() {
        let mut items = items();
        for (i, item) in items.iter_mut().enumerate() {
            item.label = if i % 2 == 0 {
                "x".into()
            } else {
                "a very long workspace name that must truncate".into()
            };
        }
        let (theme, _) = crate::ui::theme::registry().resolve_named("catppuccin-latte");
        for width in [60, 90, 180] {
            let area = Rect::new(0, 0, width, 2);
            let mut offset = 0;
            for active in &items {
                let mut buffer = Buffer::empty(area);
                let hits = rail(area, &items, active.id, &mut offset, &mut buffer, &theme);
                assert!(hits
                    .iter()
                    .any(|(_, hit)| matches!(hit, Hit::Tab(id) if *id == active.id)));
                assert!(hits
                    .iter()
                    .any(|(_, hit)| matches!(hit, Hit::Close(id) if *id == active.id)));
                assert!(hits.iter().all(|(rect, _)| rect.right() <= area.right()));
            }
        }
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
