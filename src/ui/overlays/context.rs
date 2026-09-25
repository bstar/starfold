//! Context menu and destination dialog. Targets are captured when opened,
//! never reconstructed from a cursor that may subsequently move.
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
    ratatui::{
        buffer::Buffer,
        layout::Rect,
        style::Style,
        widgets::{Clear, Widget},
    },
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
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Preview,
    Mark,
    Copy,
    Move,
    Rename,
    CreateFile,
    CreateDirectory,
    Delete,
    Compress,
    Extract,
}
impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Preview => "Preview",
            Self::Mark => "Mark / unmark",
            Self::Copy => "Copy…",
            Self::Move => "Move…",
            Self::Rename => "Rename…",
            Self::CreateFile => "New file…",
            Self::CreateDirectory => "New directory…",
            Self::Delete => "Delete (queue)",
            Self::Compress => "Compress…",
            Self::Extract => "Extract…",
        }
    }
}
#[derive(Debug)]
pub struct Menu {
    pub target: Target,
    pub actions: Vec<Action>,
    pub cursor: usize,
    pub anchor: (u16, u16),
    pub title: &'static str,
}
impl Menu {
    pub fn new(target: Target, anchor: (u16, u16)) -> Self {
        let empty = target.sources.is_empty();
        let mut actions = if target.sources.is_empty() {
            vec![Action::CreateFile, Action::CreateDirectory]
        } else {
            vec![
                Action::Open,
                Action::Preview,
                Action::Mark,
                Action::Copy,
                Action::Move,
                Action::Rename,
                Action::CreateFile,
                Action::CreateDirectory,
                Action::Delete,
                Action::Compress,
            ]
        };
        if !target.sources.is_empty()
            && target
                .sources
                .iter()
                .all(|p| Format::from_path(p).is_some())
        {
            actions.push(Action::Extract);
        }
        Self {
            target,
            actions,
            cursor: 0,
            anchor,
            title: if empty {
                "directory actions"
            } else {
                "file actions"
            },
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
            },
            actions: vec![Action::Copy, Action::Move],
            cursor: 0,
            anchor,
            title: "drop files",
        }
    }
    pub fn rect(&self, area: Rect) -> Rect {
        let w = 26.min(area.width);
        let h = (self.actions.len() as u16 + 2).min(area.height);
        Rect::new(
            self.anchor.0.clamp(area.x, area.right().saturating_sub(w)),
            self.anchor.1.clamp(area.y, area.bottom().saturating_sub(h)),
            w,
            h,
        )
    }
    pub fn key(&mut self, k: KeyEvent) -> Option<Action> {
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(self.actions.len() - 1)
            }
            KeyCode::Enter => return self.actions.get(self.cursor).copied(),
            _ => {}
        }
        None
    }
    pub fn render(&self, area: Rect, buf: &mut Buffer, t: &Theme) {
        let r = self.rect(area);
        Clear.render(r, buf);
        let inner = overlay::render(
            r,
            buf,
            &overlay::Overlay {
                theme: t,
                title: self.title,
                detail: None,
                footer: None,
            },
        );
        for (i, a) in self.actions.iter().take(inner.height as usize).enumerate() {
            let style = if i == self.cursor {
                Style::default().fg(rgb(t.bg)).bg(rgb(t.row_fg))
            } else {
                Style::default().fg(rgb(t.row_fg)).bg(rgb(t.bg))
            };
            buf.set_string(
                inner.x,
                inner.y + i as u16,
                format!(
                    "{:<width$}",
                    fit(a.label(), inner.width),
                    width = inner.width as usize
                ),
                style,
            );
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub kind: OpKind,
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
}
#[derive(Debug)]
pub struct Destination {
    pub request: Request,
    pub input: TextInput,
    pub error: Option<String>,
}
impl Destination {
    pub fn new(request: Request) -> Self {
        let input = TextInput::single().with_text(request.destination.to_string_lossy());
        Self {
            request,
            input,
            error: None,
        }
    }
    pub fn handle(&mut self, k: KeyEvent) -> Option<Request> {
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
            r.destination = if path.is_absolute() {
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
            return Some(r);
        }
        None
    }
    pub fn rect(area: Rect) -> Rect {
        overlay::rect(area, (24, 76), 6, 3, Anchor::Centre)
    }
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, t: &Theme) -> Option<(u16, u16)> {
        let r = Self::rect(area);
        let title = match self.request.kind {
            OpKind::Compress(_) => "compress — destination archive",
            OpKind::Extract => "extract — destination folder",
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
                footer: Some("enter queue · tab format · esc cancel"),
            },
        );
        if inner.width == 0 || inner.height == 0 {
            return None;
        }
        let caret = self.input.render(
            Rect::new(inner.x, inner.y, inner.width, 1),
            buf,
            Style::default().fg(rgb(t.row_fg)),
        );
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
        caret
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
        }
    }
    #[test]
    fn archive_menu_exposes_extract() {
        let m = Menu::new(target(), (98, 29));
        assert!(m.actions.contains(&Action::Extract));
        assert_eq!(m.target.sources.len(), 1);
    }
    #[test]
    fn format_cycle_changes_suffix_and_queued_format() {
        let mut d = Destination::new(Request {
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
