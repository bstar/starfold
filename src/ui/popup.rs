//! Compact context menus. Drawing and pointer input use the same geometry.
use super::{panels::rgb, theme::Theme};
use starkit::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    ratatui::{
        buffer::Buffer,
        layout::Rect,
        style::{Modifier, Style},
    },
    text::fit,
    wrap::width_of,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum Entry<A> {
    Action {
        label: String,
        action: A,
        enabled: bool,
        danger: bool,
    },
    Submenu {
        label: String,
        entries: Vec<Entry<A>>,
        enabled: bool,
    },
    Separator,
}
impl<A> Entry<A> {
    pub fn action(label: impl Into<String>, action: A) -> Self {
        Self::Action {
            label: label.into(),
            action,
            enabled: true,
            danger: false,
        }
    }
    pub fn submenu(label: impl Into<String>, entries: Vec<Self>) -> Self {
        Self::Submenu {
            label: label.into(),
            entries,
            enabled: true,
        }
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        match &mut self {
            Self::Action { enabled: e, .. } | Self::Submenu { enabled: e, .. } => *e = enabled,
            _ => {}
        }
        self
    }
    pub fn dangerous(mut self) -> Self {
        if let Self::Action { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }
    fn selectable(&self) -> bool {
        matches!(
            self,
            Self::Action { enabled: true, .. } | Self::Submenu { enabled: true, .. }
        )
    }
    fn label(&self) -> &str {
        match self {
            Self::Action { label, .. } | Self::Submenu { label, .. } => label,
            Self::Separator => "",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer<A> {
    Selected(A),
    Dismissed,
    Consumed,
}
#[derive(Debug)]
struct Level<A> {
    entries: Vec<Entry<A>>,
    cursor: usize,
    scroll: usize,
}
impl<A> Level<A> {
    fn new(entries: Vec<Entry<A>>) -> Self {
        let cursor = entries.iter().position(Entry::selectable).unwrap_or(0);
        Self {
            entries,
            cursor,
            scroll: 0,
        }
    }
    fn width(&self) -> u16 {
        self.entries
            .iter()
            .map(|e| {
                width_of(e.label()).saturating_add(if matches!(e, Entry::Submenu { .. }) {
                    2
                } else {
                    0
                })
            })
            .max()
            .unwrap_or(0)
            .saturating_add(6)
            .max(14)
    }
}
#[derive(Debug, Clone, Copy)]
struct Panel {
    level: usize,
    rect: Rect,
}
#[derive(Debug)]
pub struct Popup<A> {
    levels: Vec<Level<A>>,
    pub anchor: (u16, u16),
    pending: Option<(usize, usize, Instant)>,
    pointer: Option<(u16, u16)>,
}
impl<A: Copy> Popup<A> {
    pub fn new(entries: Vec<Entry<A>>, anchor: (u16, u16)) -> Self {
        Self {
            levels: vec![Level::new(entries)],
            anchor,
            pending: None,
            pointer: None,
        }
    }
    pub fn root_rect(&self, area: Rect) -> Rect {
        let w = self.levels[0].width().min(area.width);
        let h = (self.levels[0]
            .entries
            .len()
            .saturating_add(2)
            .clamp(3, u16::MAX as usize) as u16)
            .min(area.height);
        Rect::new(
            self.anchor.0.clamp(area.x, area.right().saturating_sub(w)),
            self.anchor.1.clamp(area.y, area.bottom().saturating_sub(h)),
            w,
            h,
        )
    }
    pub fn center_in(&mut self, area: Rect) {
        let r = self.root_rect(area);
        self.anchor = (
            area.x + (area.width - r.width) / 2,
            area.y + (area.height - r.height) / 2,
        );
    }
    fn panels(&mut self, area: Rect) -> Vec<Panel> {
        if area.width < 3 || area.height < 3 {
            return vec![];
        }
        let mut panels = vec![Panel {
            level: 0,
            rect: self.root_rect(area),
        }];
        let count = self.levels.len();
        for n in 0..count {
            let r = panels.last().unwrap().rect;
            let level = &mut self.levels[n];
            let visible = usize::from(r.height.saturating_sub(2));
            if level.cursor < level.scroll {
                level.scroll = level.cursor;
            }
            if level.cursor >= level.scroll + visible {
                level.scroll = level.cursor + 1 - visible;
            }
            level.scroll = level
                .scroll
                .min(level.entries.len().saturating_sub(visible));
            if n + 1 == count {
                break;
            }
            let row_y = r.y + 1 + (level.cursor - level.scroll) as u16;
            let child = &self.levels[n + 1];
            let w = child.width().min(area.width);
            let h = (child
                .entries
                .len()
                .saturating_add(2)
                .clamp(3, u16::MAX as usize) as u16)
                .min(area.height);
            let x = if u32::from(r.right() - 1) + u32::from(w) <= u32::from(area.right()) {
                r.right() - 1
            } else if r.x.saturating_sub(area.x) + 1 >= w {
                r.x + 1 - w
            } else {
                panels.clear();
                r.x.min(area.right() - w)
            };
            let y = row_y.saturating_sub(1).clamp(area.y, area.bottom() - h);
            panels.push(Panel {
                level: n + 1,
                rect: Rect::new(x, y, w, h),
            });
        }
        panels
    }
    fn open(&mut self) {
        let level = self.levels.last().unwrap();
        if let Some(Entry::Submenu {
            entries,
            enabled: true,
            ..
        }) = level.entries.get(level.cursor)
        {
            self.levels.push(Level::new(entries.clone()));
        }
    }
    fn activate(&mut self) -> Answer<A> {
        let level = self.levels.last().unwrap();
        match level.entries.get(level.cursor) {
            Some(Entry::Action {
                action,
                enabled: true,
                ..
            }) => Answer::Selected(*action),
            Some(Entry::Submenu { enabled: true, .. }) => {
                self.open();
                Answer::Consumed
            }
            _ => Answer::Consumed,
        }
    }
    pub fn back(&mut self) -> bool {
        self.pending = None;
        if self.levels.len() > 1 {
            self.levels.pop();
            true
        } else {
            false
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Answer<A> {
        self.pending = None;
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Answer::Dismissed;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                if self.back() {
                    Answer::Consumed
                } else {
                    Answer::Dismissed
                }
            }
            KeyCode::Right => {
                self.open();
                Answer::Consumed
            }
            KeyCode::Enter => self.activate(),
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                let down = matches!(key.code, KeyCode::Down | KeyCode::Char('j'));
                let l = self.levels.last_mut().unwrap();
                let next = if down {
                    (l.cursor + 1..l.entries.len()).find(|i| l.entries[*i].selectable())
                } else {
                    (0..l.cursor).rev().find(|i| l.entries[*i].selectable())
                };
                if let Some(next) = next {
                    l.cursor = next;
                }
                Answer::Consumed
            }
            _ => Answer::Consumed,
        }
    }
    fn hit(&mut self, area: Rect, x: u16, y: u16) -> Option<(usize, Option<usize>)> {
        for p in self.panels(area).into_iter().rev() {
            if !p.rect.contains((x, y).into()) {
                continue;
            }
            let row = if y > p.rect.y
                && y < p.rect.bottom() - 1
                && x > p.rect.x
                && x < p.rect.right() - 1
            {
                let index = self.levels[p.level].scroll + usize::from(y - p.rect.y - 1);
                (index < self.levels[p.level].entries.len()).then_some(index)
            } else {
                None
            };
            return Some((p.level, row));
        }
        None
    }
    pub fn hover(&mut self, area: Rect, x: u16, y: u16) {
        if self.pointer == Some((x, y)) {
            return;
        }
        self.pointer = Some((x, y));
        self.pending = None;
        let Some((level, Some(row))) = self.hit(area, x, y) else {
            return;
        };
        if !self.levels[level].entries[row].selectable() {
            return;
        }
        if self.levels[level].cursor != row {
            self.levels.truncate(level + 1);
        }
        self.levels[level].cursor = row;
        if self.levels.len() == level + 1
            && matches!(self.levels[level].entries[row], Entry::Submenu { .. })
        {
            self.pending = Some((level, row, Instant::now()));
        }
    }
    pub fn tick(&mut self, now: Instant) {
        if let Some((level, row, since)) = self.pending {
            if now.saturating_duration_since(since) >= Duration::from_millis(200) {
                self.pending = None;
                if self.levels.len() == level + 1 && self.levels[level].cursor == row {
                    self.open();
                }
            }
        }
    }
    pub fn click(&mut self, area: Rect, x: u16, y: u16) -> Answer<A> {
        self.pending = None;
        let Some((level, row)) = self.hit(area, x, y) else {
            return Answer::Dismissed;
        };
        let Some(row) = row else {
            return Answer::Consumed;
        };
        if !self.levels[level].entries[row].selectable() {
            return Answer::Consumed;
        }
        self.levels.truncate(level + 1);
        self.levels[level].cursor = row;
        self.activate()
    }
    pub fn contains(&mut self, area: Rect, x: u16, y: u16) -> bool {
        self.hit(area, x, y).is_some()
    }
    pub fn wheel(&mut self, area: Rect, x: u16, y: u16, down: bool) {
        if let Some((level, _)) = self.hit(area, x, y) {
            self.levels.truncate(level + 1);
            self.key(KeyEvent::new(
                if down { KeyCode::Down } else { KeyCode::Up },
                KeyModifiers::NONE,
            ));
        }
    }
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let normal = Style::default()
            .fg(rgb(theme.fg.ensure_contrast(theme.panel_bg, 4.5)))
            .bg(rgb(theme.panel_bg));
        let border = normal.fg(rgb(theme.accent.ensure_contrast(theme.panel_bg, 3.0)));
        let selected = normal
            .fg(rgb(theme.bg.ensure_contrast(theme.accent, 4.5)))
            .bg(rgb(theme.accent))
            .add_modifier(Modifier::BOLD);
        for p in self.panels(area) {
            let r = p.rect;
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    buf[(x, y)].reset();
                    buf[(x, y)].set_symbol(" ").set_style(normal);
                }
            }
            for x in r.x..r.right() {
                buf[(x, r.y)]
                    .set_symbol(if x == r.x {
                        "┌"
                    } else if x == r.right() - 1 {
                        "┐"
                    } else {
                        "─"
                    })
                    .set_style(border);
                buf[(x, r.bottom() - 1)]
                    .set_symbol(if x == r.x {
                        "└"
                    } else if x == r.right() - 1 {
                        "┘"
                    } else {
                        "─"
                    })
                    .set_style(border);
            }
            let l = &self.levels[p.level];
            for y in r.y + 1..r.bottom() - 1 {
                buf[(r.x, y)].set_symbol("│").set_style(border);
                buf[(r.right() - 1, y)].set_symbol("│").set_style(border);
                let index = l.scroll + usize::from(y - r.y - 1);
                let Some(entry) = l.entries.get(index) else {
                    continue;
                };
                if matches!(entry, Entry::Separator) {
                    buf.set_string(
                        r.x + 1,
                        y,
                        "─".repeat(usize::from(r.width - 2)),
                        normal.fg(rgb(theme.border)),
                    );
                    continue;
                }
                let style = if index == l.cursor && entry.selectable() {
                    selected
                } else if !entry.selectable() {
                    normal.fg(rgb(theme.dim))
                } else if matches!(entry, Entry::Action { danger: true, .. }) {
                    normal.fg(rgb(theme.fold.error_fg))
                } else {
                    normal
                };
                buf.set_string(r.x + 1, y, " ".repeat(usize::from(r.width - 2)), style);
                if r.width > 6 {
                    buf.set_string(r.x + 3, y, fit(entry.label(), r.width - 6), style);
                    if matches!(entry, Entry::Submenu { .. }) {
                        buf.set_string(r.right() - 3, y, "›", style);
                    }
                }
            }
            if l.scroll > 0 {
                buf[(r.right() - 2, r.y)].set_symbol("↑");
            }
            if l.scroll + usize::from(r.height - 2) < l.entries.len() {
                buf[(r.right() - 2, r.bottom() - 1)].set_symbol("↓");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn menu(anchor: (u16, u16)) -> Popup<u8> {
        Popup::new(
            vec![
                Entry::action("Open", 1),
                Entry::Separator,
                Entry::action("Disabled", 2).enabled(false),
                Entry::submenu(
                    "New",
                    vec![Entry::action("File", 3), Entry::action("Directory", 4)],
                ),
                Entry::action("Delete", 5).dangerous(),
            ],
            anchor,
        )
    }
    #[test]
    fn keyboard_skips_separators_and_disabled_rows_and_returns_from_submenus() {
        let mut m = menu((0, 0));
        m.key(key(KeyCode::Down));
        assert_eq!(m.levels[0].cursor, 3);
        m.key(key(KeyCode::Right));
        assert_eq!(m.levels.len(), 2);
        assert_eq!(m.key(key(KeyCode::Enter)), Answer::Selected(3));
        assert_eq!(m.key(key(KeyCode::Esc)), Answer::Consumed);
        assert_eq!(m.levels.len(), 1);
        assert_eq!(m.key(key(KeyCode::Left)), Answer::Dismissed);
    }
    #[test]
    fn hover_waits_before_opening_and_stationary_pointer_does_not_override_keys() {
        let area = Rect::new(0, 0, 60, 21);
        let mut m = menu((4, 3));
        m.hover(area, 7, 7);
        let since = m.pending.unwrap().2;
        m.tick(since + Duration::from_millis(199));
        assert_eq!(m.levels.len(), 1);
        m.tick(since + Duration::from_millis(200));
        assert_eq!(m.levels.len(), 2);
        m.key(key(KeyCode::Left));
        m.key(key(KeyCode::Down));
        m.hover(area, 7, 7);
        assert_eq!(m.levels[0].cursor, 4);
        m.tick(since + Duration::from_secs(1));
        assert_eq!(m.levels.len(), 1);
    }
    #[test]
    fn hover_and_click_share_geometry_and_disabled_actions_never_fire() {
        let area = Rect::new(0, 0, 60, 21);
        let mut m = menu((4, 3));
        assert_eq!(m.click(area, 7, 6), Answer::Consumed);
        m.hover(area, 7, 8);
        assert_eq!(m.levels[0].cursor, 4);
        assert_eq!(m.click(area, 7, 8), Answer::Selected(5));
        assert_eq!(m.click(area, 59, 20), Answer::Dismissed);
    }
    #[test]
    fn submenu_flips_left_at_the_screen_edge_and_uses_its_own_hit_map() {
        let area = Rect::new(2, 2, 60, 21);
        let mut m = menu((61, 22));
        m.key(key(KeyCode::Down));
        m.key(key(KeyCode::Right));
        let panels = m.panels(area);
        assert_eq!(panels.len(), 2);
        assert!(panels[1].rect.x < panels[0].rect.x);
        for p in &panels {
            assert!(
                p.rect.x >= area.x
                    && p.rect.y >= area.y
                    && p.rect.right() <= area.right()
                    && p.rect.bottom() <= area.bottom()
            );
        }
        let r = panels[1].rect;
        assert_eq!(m.click(area, r.x + 3, r.y + 2), Answer::Selected(4));
    }
    #[test]
    fn narrow_screen_replaces_parent_and_left_restores_it() {
        let area = Rect::new(0, 0, 20, 8);
        let mut m = menu((10, 4));
        m.key(key(KeyCode::Down));
        m.key(key(KeyCode::Right));
        let panels = m.panels(area);
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].level, 1);
        m.key(key(KeyCode::Left));
        assert_eq!(m.panels(area)[0].level, 0);
    }
    #[test]
    fn long_menus_scroll_without_hiding_the_keyboard_selection() {
        let area = Rect::new(0, 0, 60, 6);
        let mut m = Popup::new(
            (0..20)
                .map(|n| Entry::action(format!("項目 {n}"), n))
                .collect(),
            (59, 5),
        );
        for _ in 0..19 {
            m.key(key(KeyCode::Down));
        }
        let r = m.panels(area)[0].rect;
        let l = &m.levels[0];
        assert!(l.scroll > 0 && l.cursor < l.scroll + usize::from(r.height - 2));
        assert_eq!(m.click(area, r.x + 3, r.bottom() - 2), Answer::Selected(19));
        m.wheel(area, r.x + 3, r.y + 1, false);
        assert_eq!(m.key(key(KeyCode::Enter)), Answer::Selected(18));
    }
    #[test]
    fn unicode_width_and_theme_contrast_are_preserved() {
        for b in starkit::theme::builtin::BUILTINS {
            let t = crate::ui::theme::tests_support::theme(b.id);
            let area = Rect::new(0, 0, 60, 21);
            let mut m = Popup::new(vec![Entry::action("資料 🗂", 1)], (0, 0));
            assert!(m.root_rect(area).width >= width_of("資料 🗂") + 6);
            let mut buf = Buffer::empty(area);
            m.render(area, &mut buf, &t);
            assert_eq!(buf[(0, 0)].symbol(), "┌");
            assert!(t.panel_bg.contrast(t.fg.ensure_contrast(t.panel_bg, 4.5)) >= 4.49);
            assert!(t.accent.contrast(t.bg.ensure_contrast(t.accent, 4.5)) >= 4.49);
        }
    }
}
