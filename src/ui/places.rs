//! A searchable picker for bookmarks and mounted locations.
//!
//! The core supplies saved bookmarks and mounted paths; the app also adds Home
//! and Root. This module owns only transient dialog state and returns actions
//! for the app to send to the core.

use std::path::PathBuf;

use starkit::chrome::overlay::{self, Anchor};
use starkit::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use starkit::input::{Edit, TextInput};
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;

use crate::ui::panels::{elide_middle, fit, rgb, width_of};
use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlaceGroup {
    Bookmarks,
    Devices,
    Volumes,
    Network,
    Standard,
}

impl PlaceGroup {
    const ORDER: [Self; 5] = [
        Self::Bookmarks,
        Self::Devices,
        Self::Volumes,
        Self::Network,
        Self::Standard,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Bookmarks => "BOOKMARKS",
            Self::Devices => "DEVICES",
            Self::Volumes => "VOLUMES",
            Self::Network => "NETWORK",
            Self::Standard => "LOCATIONS",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceItem {
    pub group: PlaceGroup,
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceAction {
    Consumed,
    Close,
    Quit,
    Open(PathBuf),
    SaveBookmark { path: PathBuf, name: String },
    RenameBookmark { path: PathBuf, name: String },
    RemoveBookmark(PathBuf),
    Refresh,
}

#[derive(Debug)]
enum Mode {
    Browse,
    Edit {
        path: PathBuf,
        input: TextInput,
        rename: bool,
        error: Option<&'static str>,
    },
    ConfirmRemove {
        path: PathBuf,
        name: String,
    },
}

#[derive(Debug)]
pub struct Places {
    items: Vec<PlaceItem>,
    search: TextInput,
    selected: usize,
    scroll: usize,
    mode: Mode,
    loading: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum Row {
    Heading(PlaceGroup),
    Item(usize),
}

const REMOVE_YES: &str = "y remove";
const REMOVE_NO: &str = "n keep";
const REMOVE_GAP: u16 = 3;

impl Places {
    pub fn new(items: Vec<PlaceItem>) -> Self {
        Self {
            items,
            search: TextInput::single(),
            selected: 0,
            scroll: 0,
            mode: Mode::Browse,
            loading: false,
            error: None,
        }
    }

    /// Replace the list after discovery or a bookmark change. Keep the
    /// selected path if it remains in the filtered list.
    pub fn set_items(&mut self, items: Vec<PlaceItem>) {
        if self.items == items {
            return;
        }
        let previous = self.selected_item().map(|i| i.path.clone());
        self.items = items;
        self.selected = previous
            .and_then(|path| {
                self.filtered_items()
                    .iter()
                    .position(|&i| self.items[i].path == path)
            })
            .unwrap_or(0);
    }

    pub fn set_status(&mut self, loading: bool, error: Option<String>) {
        self.loading = loading;
        self.error = error;
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) -> Option<(u16, u16)> {
        render(area, buf, theme, self)
    }

    /// Open the name field directly for `B` on the current directory.
    pub fn begin_bookmark(&mut self, path: PathBuf, name: String) {
        self.mode = Mode::Edit {
            path,
            input: TextInput::single().with_text(name),
            rename: false,
            error: None,
        };
    }

    pub fn handle(&mut self, key: KeyEvent) -> PlaceAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return PlaceAction::Quit;
        }
        match &mut self.mode {
            Mode::Edit {
                path,
                input,
                rename,
                error,
            } => match input.handle(key) {
                Edit::Cancel => {
                    self.mode = Mode::Browse;
                    PlaceAction::Consumed
                }
                Edit::Submit => {
                    let name = input.text().trim();
                    if name.is_empty() {
                        *error = Some("enter a name");
                        return PlaceAction::Consumed;
                    }
                    let action = if *rename {
                        PlaceAction::RenameBookmark {
                            path: path.clone(),
                            name: name.to_owned(),
                        }
                    } else {
                        PlaceAction::SaveBookmark {
                            path: path.clone(),
                            name: name.to_owned(),
                        }
                    };
                    self.mode = Mode::Browse;
                    action
                }
                Edit::Consumed => {
                    *error = None;
                    PlaceAction::Consumed
                }
                Edit::Ignored => PlaceAction::Consumed,
            },
            Mode::ConfirmRemove { path, .. } => match key.code {
                KeyCode::Char('y') if key.modifiers.is_empty() => {
                    let path = path.clone();
                    self.mode = Mode::Browse;
                    PlaceAction::RemoveBookmark(path)
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.mode = Mode::Browse;
                    PlaceAction::Consumed
                }
                _ => PlaceAction::Consumed,
            },
            Mode::Browse => self.handle_browse(key),
        }
    }

    fn handle_browse(&mut self, key: KeyEvent) -> PlaceAction {
        match key.code {
            KeyCode::Esc => return PlaceAction::Close,
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down | KeyCode::Tab => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-8),
            KeyCode::PageDown => self.move_selection(8),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.filtered_items().len().saturating_sub(1),
            KeyCode::Enter => {
                return self
                    .selected_item()
                    .map(|item| PlaceAction::Open(item.path.clone()))
                    .unwrap_or(PlaceAction::Consumed);
            }
            KeyCode::F(2) => return self.start_rename(),
            KeyCode::Delete => return self.start_remove(),
            KeyCode::F(5) => return PlaceAction::Refresh,
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.clear();
                self.selected = 0;
                self.scroll = 0;
            }
            _ => {
                if self.search.handle(key) == Edit::Consumed {
                    self.selected = 0;
                    self.scroll = 0;
                }
            }
        }
        PlaceAction::Consumed
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.filtered_items().len();
        if len > 0 {
            self.selected = self.selected.saturating_add_signed(delta).min(len - 1);
        }
    }

    fn selected_item(&self) -> Option<&PlaceItem> {
        let index = *self.filtered_items().get(self.selected)?;
        self.items.get(index)
    }

    fn start_rename(&mut self) -> PlaceAction {
        let Some(item) = self
            .selected_item()
            .filter(|i| i.group == PlaceGroup::Bookmarks)
        else {
            return PlaceAction::Consumed;
        };
        self.mode = Mode::Edit {
            path: item.path.clone(),
            input: TextInput::single().with_text(item.name.clone()),
            rename: true,
            error: None,
        };
        PlaceAction::Consumed
    }

    fn start_remove(&mut self) -> PlaceAction {
        let Some(item) = self
            .selected_item()
            .filter(|i| i.group == PlaceGroup::Bookmarks)
        else {
            return PlaceAction::Consumed;
        };
        self.mode = Mode::ConfirmRemove {
            path: item.path.clone(),
            name: item.name.clone(),
        };
        PlaceAction::Consumed
    }

    fn filtered_items(&self) -> Vec<usize> {
        let query = self.search.text().to_lowercase();
        PlaceGroup::ORDER
            .iter()
            .flat_map(|&group| {
                self.items.iter().enumerate().filter_map({
                    let query = query.clone();
                    move |(i, item)| {
                        (item.group == group
                            && (query.is_empty()
                                || item.name.to_lowercase().contains(&query)
                                || item.path.to_string_lossy().to_lowercase().contains(&query)))
                        .then_some(i)
                    }
                })
            })
            .collect()
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for group in PlaceGroup::ORDER {
            let mut matching = self
                .filtered_items()
                .into_iter()
                .filter(|&i| self.items[i].group == group)
                .peekable();
            if matching.peek().is_some() {
                rows.push(Row::Heading(group));
                rows.extend(matching.map(Row::Item));
            }
        }
        rows
    }

    fn list_rect(inner: Rect) -> Rect {
        Rect {
            x: inner.x,
            y: inner.y.saturating_add(2),
            width: inner.width,
            height: inner.height.saturating_sub(4),
        }
    }

    fn keep_selected_visible(&mut self, rows: &[Row], height: usize) {
        if height == 0 {
            return;
        }
        let selected = self.selected_item().and_then(|item| {
            rows.iter().position(|row| match row {
                Row::Item(i) => {
                    self.items[*i].path == item.path && self.items[*i].group == item.group
                }
                Row::Heading(_) => false,
            })
        });
        if let Some(at) = selected {
            if at < self.scroll {
                self.scroll = at;
            } else if at >= self.scroll + height {
                self.scroll = at + 1 - height;
            }
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
    }

    pub fn scroll(&mut self, down: bool) {
        self.move_selection(if down { 3 } else { -3 });
    }

    /// Click a row to open it; a footer button edits the selected bookmark.
    pub fn click(&mut self, x: u16, y: u16, area: Rect) -> PlaceAction {
        let rr = rect(area);
        if !contains(rr, x, y) {
            return PlaceAction::Close;
        }
        let inner = overlay::inner(rr);
        match &mut self.mode {
            Mode::Edit { .. } => return PlaceAction::Consumed,
            Mode::ConfirmRemove { path, .. } => {
                if y == inner.y.saturating_add(2) {
                    let yes_width = width_of(REMOVE_YES).min(inner.width);
                    let no_start = width_of(REMOVE_YES).saturating_add(REMOVE_GAP);
                    let no_end = no_start
                        .saturating_add(width_of(REMOVE_NO))
                        .min(inner.width);
                    let left = x.saturating_sub(inner.x);
                    if x >= inner.x && left < yes_width {
                        let path = path.clone();
                        self.mode = Mode::Browse;
                        return PlaceAction::RemoveBookmark(path);
                    }
                    if left >= no_start && left < no_end {
                        self.mode = Mode::Browse;
                    }
                }
                return PlaceAction::Consumed;
            }
            Mode::Browse => {}
        }
        let actions_y = inner.y + inner.height.saturating_sub(1);
        if y == actions_y {
            let left = x.saturating_sub(inner.x);
            return match left {
                0..=6 => self.start_rename(),
                9..=18 => self.start_remove(),
                21..=27 => PlaceAction::Refresh,
                _ => PlaceAction::Consumed,
            };
        }
        let list = Self::list_rect(inner);
        if !contains(list, x, y) {
            return PlaceAction::Consumed;
        }
        let rows = self.rows();
        let Some(Row::Item(index)) = rows.get(self.scroll + usize::from(y - list.y)) else {
            return PlaceAction::Consumed;
        };
        let path = self.items[*index].path.clone();
        self.selected = self
            .filtered_items()
            .iter()
            .position(|&i| i == *index)
            .unwrap_or(self.selected);
        PlaceAction::Open(path)
    }
}

fn contains(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x.saturating_add(r.width) && y >= r.y && y < r.y.saturating_add(r.height)
}

pub fn rect(area: Rect) -> Rect {
    overlay::rect(area, (38, 78), 24, 9, Anchor::Centre)
}

/// Render the picker and return the terminal cursor for its active text field.
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    places: &mut Places,
) -> Option<(u16, u16)> {
    let rr = rect(area);
    if rr.width < 8 || rr.height < 5 {
        return None;
    }
    let footer = match places.mode {
        Mode::Browse if rr.width < 70 => "enter open · F2 edit · del remove · esc close",
        Mode::Browse => "enter open · F2 rename · del remove · F5 refresh · esc close",
        Mode::Edit { .. } => "enter save · esc back",
        Mode::ConfirmRemove { .. } => "y remove · n keep",
    };
    let inner = overlay::render(
        rr,
        buf,
        &overlay::Overlay {
            theme,
            title: "places",
            detail: None,
            footer: Some(footer),
        },
    );
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    match &mut places.mode {
        Mode::Edit {
            path,
            input,
            rename,
            error,
        } => {
            let label = if *rename {
                "RENAME BOOKMARK"
            } else {
                "BOOKMARK DIRECTORY"
            };
            buf.set_string(
                inner.x,
                inner.y,
                fit(label, inner.width),
                Style::default().fg(rgb(theme.accent)),
            );
            if inner.height > 1 {
                buf.set_string(
                    inner.x,
                    inner.y + 1,
                    elide_middle(&path.to_string_lossy(), inner.width),
                    Style::default().fg(rgb(theme.dim)),
                );
            }
            let cursor = if inner.height > 3 {
                input.render(
                    Rect::new(inner.x, inner.y + 3, inner.width, 1),
                    buf,
                    Style::default().fg(rgb(theme.fg)),
                )
            } else {
                None
            };
            if let Some(error) = error {
                if inner.height > 4 {
                    buf.set_string(
                        inner.x,
                        inner.y + 4,
                        fit(error, inner.width),
                        Style::default().fg(rgb(theme.fold.error_fg)),
                    );
                }
            }
            cursor
        }
        Mode::ConfirmRemove { name, path } => {
            buf.set_string(
                inner.x,
                inner.y,
                fit("REMOVE BOOKMARK?", inner.width),
                Style::default().fg(rgb(theme.fold.error_fg)),
            );
            if inner.height > 1 {
                buf.set_string(
                    inner.x,
                    inner.y + 1,
                    fit(&format!("{name}  {}", path.display()), inner.width),
                    Style::default().fg(rgb(theme.fg)),
                );
            }
            if inner.height > 2 {
                buf.set_string(
                    inner.x,
                    inner.y + 2,
                    fit(
                        &format!(
                            "{REMOVE_YES}{}{REMOVE_NO}",
                            " ".repeat(usize::from(REMOVE_GAP))
                        ),
                        inner.width,
                    ),
                    Style::default().fg(rgb(theme.accent)),
                );
            }
            None
        }
        Mode::Browse => {
            let style = Style::default().fg(rgb(theme.fg));
            buf.set_string(
                inner.x,
                inner.y,
                fit("Search:", inner.width),
                Style::default().fg(rgb(theme.dim)),
            );
            let cursor = if inner.width > 8 {
                places.search.render(
                    Rect::new(inner.x + 8, inner.y, inner.width - 8, 1),
                    buf,
                    style,
                )
            } else {
                None
            };
            if inner.height > 1 {
                let status = if let Some(error) = &places.error {
                    error.as_str()
                } else if places.loading {
                    "Finding mounted locations…"
                } else {
                    "Type to search names and paths"
                };
                let color = if places.error.is_some() {
                    theme.fold.error_fg
                } else {
                    theme.dim
                };
                buf.set_string(
                    inner.x,
                    inner.y + 1,
                    fit(status, inner.width),
                    Style::default().fg(rgb(color)),
                );
            }
            let rows = places.rows();
            let list = Places::list_rect(inner);
            places.keep_selected_visible(&rows, usize::from(list.height));
            let selected_index = places.selected_item().and_then(|item| {
                rows.iter().position(|row| match row {
                    Row::Item(i) => {
                        places.items[*i].path == item.path && places.items[*i].group == item.group
                    }
                    Row::Heading(_) => false,
                })
            });
            if rows.is_empty() && list.height > 0 {
                buf.set_string(
                    list.x,
                    list.y,
                    fit("No matching places", list.width),
                    Style::default().fg(rgb(theme.dim)),
                );
            }
            for (line, row) in rows
                .iter()
                .skip(places.scroll)
                .take(usize::from(list.height))
                .enumerate()
            {
                let y = list.y + line as u16;
                match row {
                    Row::Heading(group) => {
                        buf.set_string(
                            list.x,
                            y,
                            fit(group.title(), list.width),
                            Style::default().fg(rgb(theme.accent)),
                        );
                    }
                    Row::Item(i) => {
                        let item = &places.items[*i];
                        let selected = selected_index == Some(places.scroll + line);
                        let bg = if selected {
                            theme.fold.marked_bg
                        } else {
                            theme.panel_bg
                        };
                        let fg = if selected {
                            theme.fold.marked_fg
                        } else {
                            theme.fg
                        };
                        let row_style = Style::default().fg(rgb(fg)).bg(rgb(bg));
                        buf.set_string(list.x, y, " ".repeat(usize::from(list.width)), row_style);
                        let marker = if selected { "> " } else { "  " };
                        buf.set_string(list.x, y, marker, row_style);
                        let available = list.width.saturating_sub(2);
                        let name_width = available.min((available / 3).max(14));
                        buf.set_string(list.x + 2, y, fit(&item.name, name_width), row_style);
                        let path_x = list.x + 2 + name_width;
                        let path_width = list.width.saturating_sub(2 + name_width);
                        if path_width > 0 {
                            buf.set_string(
                                path_x,
                                y,
                                elide_middle(&item.path.to_string_lossy(), path_width),
                                row_style,
                            );
                        }
                    }
                }
            }
            if inner.height > 0 {
                let action_y = inner.y + inner.height - 1;
                buf.set_string(
                    inner.x,
                    action_y,
                    fit("F2 edit  del remove  F5 scan", inner.width),
                    Style::default().fg(rgb(theme.dim)),
                );
            }
            cursor
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::tests_support::theme;
    use proptest::prelude::*;
    use starkit::crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn item(group: PlaceGroup, name: &str, path: &str) -> PlaceItem {
        PlaceItem {
            group,
            name: name.into(),
            path: path.into(),
        }
    }

    fn sample() -> Places {
        Places::new(vec![
            item(PlaceGroup::Bookmarks, "Projects", "/home/user/projects"),
            item(PlaceGroup::Bookmarks, "Writing", "/home/user/documents"),
            item(PlaceGroup::Devices, "Camera", "/run/media/user/CAMERA"),
            item(PlaceGroup::Volumes, "Scratch", "/mnt/scratch"),
            item(PlaceGroup::Network, "Office", "/mnt/office"),
            item(PlaceGroup::Standard, "Home", "/home/user"),
            item(PlaceGroup::Standard, "Root", "/"),
        ])
    }

    fn drawn(places: &mut Places, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        let mut buffer = Buffer::empty(area);
        places.render(area, &mut buffer, &theme("terminal"));
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn search_matches_path_and_groups_stay_ordered() {
        let mut places = Places::new(vec![
            item(PlaceGroup::Network, "Server", "/mnt/work"),
            item(PlaceGroup::Bookmarks, "Working", "/home/me/work"),
        ]);
        for c in "work".chars() {
            places.handle(key(KeyCode::Char(c)));
        }
        assert_eq!(places.filtered_items(), vec![1, 0]);
        assert_eq!(
            places.handle(key(KeyCode::Enter)),
            PlaceAction::Open("/home/me/work".into())
        );
        places.handle(key(KeyCode::Down));
        assert_eq!(
            places.handle(key(KeyCode::Enter)),
            PlaceAction::Open("/mnt/work".into())
        );
    }

    #[test]
    fn removing_a_bookmark_needs_confirmation() {
        let mut places = Places::new(vec![item(PlaceGroup::Bookmarks, "Home", "/home/me")]);
        assert_eq!(places.handle(key(KeyCode::Delete)), PlaceAction::Consumed);
        assert_eq!(
            places.handle(key(KeyCode::Char('n'))),
            PlaceAction::Consumed
        );
        assert_eq!(places.handle(key(KeyCode::Delete)), PlaceAction::Consumed);
        assert_eq!(
            places.handle(key(KeyCode::Char('y'))),
            PlaceAction::RemoveBookmark("/home/me".into())
        );
    }

    #[test]
    fn removal_prompt_clicks_match_the_visible_labels() {
        let mut places = Places::new(vec![item(PlaceGroup::Bookmarks, "Home", "/home/me")]);
        let area = Rect::new(0, 0, 60, 21);
        let inner = overlay::inner(rect(area));
        let y = inner.y + 2;
        places.handle(key(KeyCode::Delete));

        // The separator does nothing; the final character of each label
        // still answers the same way as its first character.
        assert_eq!(places.click(inner.x + 8, y, area), PlaceAction::Consumed);
        assert_eq!(
            places.click(inner.x + 6, y, area),
            PlaceAction::RemoveBookmark("/home/me".into())
        );

        places.handle(key(KeyCode::Delete));
        assert_eq!(places.click(inner.x + 16, y, area), PlaceAction::Consumed);
        assert_eq!(
            places.handle(key(KeyCode::Enter)),
            PlaceAction::Open("/home/me".into())
        );
    }

    #[test]
    fn bookmark_editor_rejects_blank_names() {
        let mut places = Places::new(Vec::new());
        places.begin_bookmark("/tmp".into(), " ".into());
        assert_eq!(places.handle(key(KeyCode::Enter)), PlaceAction::Consumed);
        places.handle(key(KeyCode::Backspace));
        places.handle(key(KeyCode::Char('X')));
        assert_eq!(
            places.handle(key(KeyCode::Enter)),
            PlaceAction::SaveBookmark {
                path: "/tmp".into(),
                name: "X".into()
            }
        );
    }

    #[test]
    fn draw_and_click_use_the_same_rows_at_the_minimum_window_size() {
        let mut places = Places::new(vec![
            item(PlaceGroup::Network, "Office", "/mnt/office"),
            item(PlaceGroup::Bookmarks, "Projects", "/home/me/projects"),
        ]);
        let area = Rect::new(0, 0, 60, 21);
        let mut buffer = Buffer::empty(area);
        places.render(area, &mut buffer, &theme("terminal"));
        let inner = overlay::inner(rect(area));
        let rows = Places::list_rect(inner);
        assert_eq!(
            places.click(rows.x + 3, rows.y + 1, area),
            PlaceAction::Open("/home/me/projects".into())
        );
        assert_eq!(
            places.click(rows.x + 3, rows.y + 3, area),
            PlaceAction::Open("/mnt/office".into())
        );
    }

    #[test]
    fn mount_entries_cannot_be_renamed_or_removed() {
        let mut places = Places::new(vec![item(PlaceGroup::Devices, "USB", "/media/usb")]);
        assert_eq!(places.handle(key(KeyCode::F(2))), PlaceAction::Consumed);
        assert_eq!(places.handle(key(KeyCode::Delete)), PlaceAction::Consumed);
        assert_eq!(
            places.handle(key(KeyCode::Enter)),
            PlaceAction::Open("/media/usb".into())
        );
    }

    #[test]
    fn picker_snapshots() {
        let mut places = sample();
        insta::assert_snapshot!("places-100x30", drawn(&mut places, 100, 30));

        for c in "office".chars() {
            places.handle(key(KeyCode::Char(c)));
        }
        insta::assert_snapshot!("places-search-60x21", drawn(&mut places, 60, 21));

        places.begin_bookmark("/home/user/projects".into(), "Projects".into());
        insta::assert_snapshot!("places-bookmark-editor-100x30", drawn(&mut places, 100, 30));

        let mut places = sample();
        places.handle(key(KeyCode::Delete));
        insta::assert_snapshot!("places-remove-60x21", drawn(&mut places, 60, 21));

        let mut places = sample();
        places.set_status(false, Some("Network mount list unavailable".into()));
        insta::assert_snapshot!("places-error-60x21", drawn(&mut places, 60, 21));
    }

    proptest! {
        #[test]
        fn arbitrary_unicode_names_and_paths_fit_in_any_overlay(
            name in proptest::collection::vec(any::<char>(), 0..50).prop_map(|v| v.into_iter().collect::<String>()),
            path in proptest::collection::vec(any::<char>(), 0..80).prop_map(|v| v.into_iter().collect::<String>()),
            width in 0u16..80,
            height in 0u16..30,
        ) {
            let mut places = Places::new(vec![PlaceItem {
                group: PlaceGroup::Bookmarks,
                name,
                path: PathBuf::from(format!("/tmp/{path}")),
            }]);
            let area = Rect::new(0, 0, width, height);
            let mut buffer = Buffer::empty(area);
            places.render(area, &mut buffer, &theme("terminal"));
        }
    }
}
