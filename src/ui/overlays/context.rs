//! Context menu and destination dialog. Targets are captured when opened,
//! never reconstructed from a cursor that may subsequently move.
use crate::ui::popup::{Answer as PopupAnswer, Entry, Popup};
use crate::{
    fold::{archive::Format, ops::OpKind},
    ui::{
        panels::{fit, rgb},
        theme::Theme,
    },
};
use starkit::{
    chrome::overlay::{self, Anchor},
    crossterm::event::{KeyCode, KeyEvent},
    input::{Edit, TextInput},
    ratatui::{buffer::Buffer, layout::Rect, style::Style},
};
use std::path::PathBuf;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub clicked: PathBuf,
    pub directory: bool,
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
    /// The directory receiving a newly created item, independent of the
    /// opposite-pane destination used for copy and move.
    pub create_dir: PathBuf,
    /// Edit appears only for regular, text-like files.
    pub editable: bool,
    pub archive_writable: Option<bool>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NextTheme,
    PrevTheme,
    TogglePresentation,
    Tabs,
    Trash,
    Undo,
    Open,
    Preview,
    Edit,
    Mark,
    Copy,
    CopyCurrentPath,
    Move,
    Rename,
    CreateFile,
    CreateDirectory,
    Delete,
    Compress,
    Extract,
    ArchiveSave,
    ArchiveSaveAs,
    ArchiveDiscard,
    ArchiveTest,
    ArchiveUnlock,
}
impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::NextTheme => "Next theme",
            Self::PrevTheme => "Previous theme",
            Self::TogglePresentation => "Graphical / cells",
            Self::Trash => "Trash…",
            Self::Undo => "Undo…",
            Self::Tabs => "Tabs…",
            Self::Open => "Open",
            Self::Preview => "Preview",
            Self::Edit => "Edit",
            Self::Mark => "Mark / unmark",
            Self::Copy => "Copy…",
            Self::CopyCurrentPath => "Copy current path",
            Self::Move => "Move…",
            Self::Rename => "Rename…",
            Self::CreateFile => "New file…",
            Self::CreateDirectory => "New directory…",
            Self::Delete => "Delete",
            Self::Compress => "Compress…",
            Self::Extract => "Extract…",
            Self::ArchiveSave => "Save ZIP changes",
            Self::ArchiveSaveAs => "Save archive as…",
            Self::ArchiveDiscard => "Discard ZIP changes",
            Self::ArchiveTest => "Test archive",
            Self::ArchiveUnlock => "Unlock archive…",
        }
    }
}
#[derive(Debug)]
pub struct Menu {
    pub target: Target,
    #[cfg(test)]
    pub actions: Vec<Action>,
    pub popup: Popup<Action>,
}
impl Menu {
    pub fn new(target: Target, anchor: (u16, u16)) -> Self {
        let mut actions = if target.sources.is_empty() {
            vec![
                Action::CopyCurrentPath,
                Action::CreateFile,
                Action::CreateDirectory,
            ]
        } else {
            vec![
                Action::Open,
                Action::Preview,
                Action::Mark,
                Action::Copy,
                Action::CopyCurrentPath,
                Action::Move,
                Action::Rename,
                Action::CreateFile,
                Action::CreateDirectory,
                Action::Delete,
                Action::Compress,
            ]
        };
        if target.editable {
            actions.insert(2, Action::Edit);
        }
        if !target.sources.is_empty()
            && target
                .sources
                .iter()
                .all(|p| Format::from_path(p).is_some())
        {
            actions.push(Action::Extract);
        }
        let archive_location = crate::fold::location::is_archive(&target.create_dir);
        if archive_location {
            actions.retain(|a| {
                !matches!(
                    a,
                    Action::Move
                        | Action::CreateFile
                        | Action::CreateDirectory
                        | Action::Compress
                        | Action::Extract
                )
            });
            if target.archive_writable == Some(false) {
                actions.retain(|a| !matches!(a, Action::Edit | Action::Rename | Action::Delete));
            }
            actions.extend([Action::ArchiveTest, Action::ArchiveUnlock]);
            if let Ok(crate::fold::location::Location::Archive { source, .. }) =
                crate::fold::location::Location::from_key(&target.create_dir)
            {
                if target.archive_writable.unwrap_or_else(|| {
                    Format::from_path(&source.file) == Some(Format::Zip)
                        && source
                            .nested
                            .iter()
                            .all(|m| Format::from_path(&m.name) == Some(Format::Zip))
                }) {
                    actions.extend([
                        Action::ArchiveSave,
                        Action::ArchiveSaveAs,
                        Action::ArchiveDiscard,
                    ]);
                } else {
                    actions.retain(|a| !matches!(a, Action::Rename | Action::Delete));
                    actions.push(Action::ArchiveSaveAs);
                }
            }
        }
        actions.push(Action::Tabs);
        let action = |a: Action| Entry::action(a.label(), a);
        let mut entries = Vec::new();
        if !target.sources.is_empty() {
            for a in [Action::Open, Action::Preview, Action::Edit, Action::Mark] {
                if actions.contains(&a) {
                    entries.push(action(a));
                }
            }
            entries.push(Entry::Separator);
            for a in [
                Action::Copy,
                Action::Move,
                Action::Rename,
                Action::CopyCurrentPath,
            ] {
                if actions.contains(&a) {
                    entries.push(action(a));
                }
            }
            entries.push(Entry::Separator);
        } else {
            entries.push(action(Action::CopyCurrentPath));
            entries.push(Entry::Separator);
        }
        if !archive_location {
            entries.push(Entry::submenu(
                "New",
                vec![action(Action::CreateFile), action(Action::CreateDirectory)],
            ));
        }
        let archive = [
            Action::Compress,
            Action::Extract,
            Action::ArchiveSave,
            Action::ArchiveSaveAs,
            Action::ArchiveDiscard,
            Action::ArchiveTest,
            Action::ArchiveUnlock,
        ]
        .into_iter()
        .filter(|a| actions.contains(a))
        .map(action)
        .collect::<Vec<_>>();
        if !archive.is_empty() {
            entries.push(Entry::submenu("Archive", archive));
        }
        if actions.contains(&Action::Delete) {
            entries.push(Entry::Separator);
            entries.push(action(Action::Delete).dangerous());
        }
        entries.push(Entry::Separator);
        entries.push(action(Action::Tabs));
        entries.push(Entry::submenu(
            "Appearance",
            vec![
                action(Action::NextTheme),
                action(Action::PrevTheme),
                action(Action::TogglePresentation),
            ],
        ));
        entries.push(Entry::submenu(
            "Recovery",
            vec![action(Action::Trash), action(Action::Undo)],
        ));
        Self {
            target,
            #[cfg(test)]
            actions,
            popup: Popup::new(entries, anchor),
        }
    }
    pub fn for_drop(anchor: (u16, u16)) -> Self {
        Self {
            target: Target {
                clicked: PathBuf::new(),
                directory: true,
                sources: Vec::new(),
                destination: PathBuf::new(),
                create_dir: PathBuf::new(),
                editable: false,
                archive_writable: None,
            },
            #[cfg(test)]
            actions: vec![Action::Copy, Action::Move],
            popup: Popup::new(
                vec![
                    Entry::action("Copy", Action::Copy),
                    Entry::action("Move", Action::Move),
                ],
                anchor,
            ),
        }
    }
    #[cfg(test)]
    pub fn rect(&self, area: Rect) -> Rect {
        self.popup.root_rect(area)
    }
    pub fn center_in(&mut self, area: Rect) {
        self.popup.center_in(area);
    }
    pub fn key(&mut self, k: KeyEvent) -> Option<Action> {
        match self.popup.key(k) {
            PopupAnswer::Selected(a) => Some(a),
            _ => None,
        }
    }
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, t: &Theme) {
        self.popup.render(area, buf, t);
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub archive_options: starfold_archive_protocol::Options,
    pub kind: OpKind,
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
}
pub struct Destination {
    pub request: Request,
    pub input: TextInput,
    pub error: Option<String>,
    pub field: usize,
    pub password: TextInput,
}
impl std::fmt::Debug for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Destination")
            .field("request", &self.request)
            .field("field", &self.field)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
impl Destination {
    pub fn new(request: Request) -> Self {
        let input =
            TextInput::single().with_text(crate::fold::location::display(&request.destination));
        let field = if request.kind == OpKind::ArchiveTest {
            3
        } else {
            0
        };
        Self {
            request,
            input,
            error: None,
            field,
            password: TextInput::single(),
        }
    }
    pub fn handle(&mut self, k: KeyEvent) -> Option<Request> {
        if self.request.kind == OpKind::ArchiveTest {
            if self.password.handle(k) == Edit::Submit {
                let mut r = self.request.clone();
                r.archive_options.password = Some(self.password.text().into());
                return Some(r);
            }
            return None;
        }
        let compress = matches!(self.request.kind, OpKind::Compress(_));
        if compress && matches!(k.code, KeyCode::Up | KeyCode::Down) {
            self.field = if k.code == KeyCode::Up {
                (self.field + 6) % 7
            } else {
                (self.field + 1) % 7
            };
            return None;
        }
        if compress && self.field > 0 && k.code != KeyCode::Enter && k.code != KeyCode::Tab {
            if self.field == 3 {
                self.password.handle(k);
                return None;
            }
            if matches!(k.code, KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right) {
                self.cycle_field();
            }
            return None;
        }
        if k.code == KeyCode::Tab && matches!(self.request.kind, OpKind::Compress(_)) {
            let formats = [
                ("zip", Format::Zip),
                ("tar", Format::Tar),
                ("tar.gz", Format::TarGz),
                ("tar.zst", Format::TarZst),
                ("7z", Format::SevenZip),
            ];
            let current = Format::from_path(std::path::Path::new(self.input.text()));
            let next = (formats
                .iter()
                .position(|(_, f)| Some(*f) == current)
                .unwrap_or(0)
                + 1)
                % formats.len();
            let base = crate::fold::archive::destination(std::path::Path::new(self.input.text()));
            self.input =
                TextInput::single().with_text(format!("{}.{}", base.display(), formats[next].0));
            return None;
        }
        if self.input.handle(k) == Edit::Submit {
            let text = self.input.text();
            if text.trim().is_empty() {
                self.error = Some("Enter a destination".into());
                return None;
            }
            let mut r = self.request.clone();
            let path = PathBuf::from(text);
            r.destination = if crate::fold::location::is_archive(&r.destination)
                && text == crate::fold::location::display(&r.destination)
            {
                r.destination.clone()
            } else if path.is_absolute() {
                path
            } else {
                r.destination
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join(path)
            };
            if matches!(r.kind, OpKind::Compress(_)) {
                match Format::from_path(&r.destination).filter(|f| f.writable()) {
                    Some(f) => r.kind = OpKind::Compress(f),
                    None => {
                        self.error = Some("Use .zip, .tar, .tar.gz, .tar.zst or .7z".into());
                        return None;
                    }
                }
            }
            if compress {
                r.archive_options = self.request.archive_options.clone();
                r.archive_options.password =
                    (!self.password.text().is_empty()).then(|| self.password.text().to_string());
                if (r.archive_options.password.is_some()
                    || r.archive_options.volume_bytes.is_some())
                    && !matches!(r.kind, OpKind::Compress(Format::Zip | Format::SevenZip))
                {
                    self.error = Some("Passwords and split volumes require ZIP or 7z".into());
                    return None;
                }
            }
            return Some(r);
        }
        None
    }
    pub fn bounds(&self, area: Rect) -> Rect {
        if matches!(self.request.kind, OpKind::Compress(_)) {
            overlay::rect(area, (38, 82), 13, 3, Anchor::Centre)
        } else {
            Self::rect(area)
        }
    }
    fn cycle_field(&mut self) {
        use starfold_archive_protocol::Preset;
        match self.field {
            1 => {
                let _ = self.handle(KeyEvent::new(
                    KeyCode::Tab,
                    starkit::crossterm::event::KeyModifiers::NONE,
                ));
            }
            2 => {
                self.request.archive_options.preset = match self.request.archive_options.preset {
                    Preset::Store => Preset::Fast,
                    Preset::Fast => Preset::Balanced,
                    Preset::Balanced => Preset::Maximum,
                    Preset::Maximum => Preset::Store,
                }
            }
            4 => {
                let sizes = [
                    None,
                    Some(100 * 1024 * 1024),
                    Some(700 * 1024 * 1024),
                    Some(2 * 1024 * 1024 * 1024),
                    Some(4 * 1024 * 1024 * 1024),
                ];
                let index = sizes
                    .iter()
                    .position(|v| *v == self.request.archive_options.volume_bytes)
                    .unwrap_or(0);
                self.request.archive_options.volume_bytes = sizes[(index + 1) % sizes.len()];
            }
            5 => {
                self.request.archive_options.exclude_mac_metadata =
                    !self.request.archive_options.exclude_mac_metadata
            }
            _ => {}
        }
    }
    pub fn click(&mut self, area: Rect, x: u16, y: u16) -> Option<Request> {
        let r = self.bounds(area);
        let row = y.saturating_sub(r.y + 1);
        if x <= r.x || x >= r.right().saturating_sub(1) {
            return None;
        }
        if matches!(self.request.kind, OpKind::Compress(_)) {
            if let Some(index) = [0, 2, 3, 4, 5, 6, 8]
                .iter()
                .position(|offset| *offset == row)
            {
                self.field = index;
                if index == 6 {
                    return self.handle(KeyEvent::new(
                        KeyCode::Enter,
                        starkit::crossterm::event::KeyModifiers::NONE,
                    ));
                }
                self.cycle_field();
            }
        } else if row == r.height.saturating_sub(3) {
            return self.handle(KeyEvent::new(
                KeyCode::Enter,
                starkit::crossterm::event::KeyModifiers::NONE,
            ));
        }
        None
    }
    pub fn footer(&self) -> &'static str {
        match self.request.kind {
            OpKind::Compress(_) => {
                "enter start · ↑/↓ field · space change · tab format · esc cancel"
            }
            OpKind::Move => "enter move · esc cancel",
            OpKind::Copy => "enter copy · esc cancel",
            _ => "enter start · esc cancel",
        }
    }
    pub fn rect(area: Rect) -> Rect {
        overlay::rect(area, (24, 76), 6, 3, Anchor::Centre)
    }
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, t: &Theme) -> Option<(u16, u16)> {
        let r = self.bounds(area);
        let title = match self.request.kind {
            OpKind::Compress(_) => "compress — destination archive",
            OpKind::Extract => "extract — destination folder",
            OpKind::ArchiveTest => "archive password",
            OpKind::ArchiveSave => "save archive as",
            OpKind::Copy => "copy — destination folder",
            _ => "move — destination folder",
        };
        let inner = overlay::render(
            r,
            buf,
            &overlay::Overlay {
                theme: t,
                title,
                detail: None,
                footer: Some(self.footer()),
            },
        );
        if inner.width == 0 || inner.height == 0 {
            return None;
        }
        if self.request.kind == OpKind::ArchiveTest {
            let text = "•".repeat(self.password.text().chars().count());
            buf.set_string(
                inner.x,
                inner.y,
                fit(&text, inner.width),
                Style::default().fg(rgb(t.row_fg)),
            );
            return Some((
                inner.x
                    + text
                        .chars()
                        .count()
                        .min(inner.width.saturating_sub(1) as usize) as u16,
                inner.y,
            ));
        }
        let caret = self.input.render(
            Rect::new(inner.x, inner.y, inner.width, 1),
            buf,
            Style::default().fg(rgb(t.row_fg)),
        );
        if matches!(self.request.kind, OpKind::Compress(_)) {
            let o = &self.request.archive_options;
            let format = Format::from_path(std::path::Path::new(self.input.text()));
            let labels = [
                format!("Format        {:?}", format.unwrap_or(Format::Zip)),
                format!("Compression   {:?}", o.preset),
                format!(
                    "Password      {}",
                    if self.password.text().is_empty() {
                        "off".into()
                    } else {
                        "•".repeat(self.password.text().chars().count())
                    }
                ),
                format!(
                    "Split volumes {}",
                    o.volume_bytes
                        .map(crate::fold::format::size)
                        .unwrap_or_else(|| "off".into())
                ),
                format!(
                    "Mac metadata  {}",
                    if o.exclude_mac_metadata {
                        "exclude"
                    } else {
                        "include"
                    }
                ),
                "[ Create archive ]".into(),
            ];
            for (i, (row, label)) in [2, 3, 4, 5, 6, 8].into_iter().zip(labels).enumerate() {
                if row < inner.height {
                    buf.set_string(
                        inner.x,
                        inner.y + row,
                        fit(&label, inner.width),
                        Style::default().fg(rgb(if self.field == i + 1 {
                            t.accent
                        } else {
                            t.row_fg
                        })),
                    );
                }
            }
        }
        if let Some(e) = &self.error {
            if inner.height > 1 {
                buf.set_string(
                    inner.x,
                    inner.y + 1,
                    fit(e, inner.width),
                    Style::default().fg(rgb(t.fold.error_fg)),
                );
            }
        }
        if matches!(self.request.kind, OpKind::Compress(_)) && self.field == 3 {
            Some((
                inner.x
                    + 14
                    + (self.password.text().chars().count() as u16)
                        .min(inner.width.saturating_sub(15)),
                inner.y + 4,
            ))
        } else {
            caret
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use starkit::crossterm::event::KeyModifiers;
    fn target() -> Target {
        Target {
            clicked: "/tmp/book.zip".into(),
            directory: false,
            sources: vec!["/tmp/book.zip".into()],
            destination: "/tmp".into(),
            create_dir: "/tmp".into(),
            editable: false,
            archive_writable: None,
        }
    }
    #[test]
    fn archive_menu_exposes_extract() {
        let m = Menu::new(target(), (98, 29));
        assert!(m.actions.contains(&Action::Extract));
        assert_eq!(m.target.sources.len(), 1);
    }
    #[test]
    fn edit_appears_only_for_an_editable_target() {
        let mut text = target();
        text.editable = true;
        let menu = Menu::new(text, (0, 0));
        assert_eq!(menu.actions[2], Action::Edit);
        assert!(!Menu::new(target(), (0, 0)).actions.contains(&Action::Edit));
    }
    #[test]
    fn format_cycle_changes_suffix_and_queued_format() {
        let mut d = Destination::new(Request {
            archive_options: Default::default(),
            kind: OpKind::Compress(Format::Zip),
            sources: vec!["a".into()],
            destination: "/tmp/a.zip".into(),
        });
        d.handle(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        let r = d
            .handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(r.kind, OpKind::Compress(Format::Tar));
        assert_eq!(r.destination, PathBuf::from("/tmp/a.tar"));
    }
    proptest! { #[test] fn menu_stays_inside_terminal(w in 1u16..200,h in 1u16..100,x in 0u16..300,y in 0u16..200){let m=Menu::new(target(),(x,y));let area=Rect::new(0,0,w,h);let r=m.rect(area);prop_assert!(r.right()<=area.right());prop_assert!(r.bottom()<=area.bottom());} }
}
