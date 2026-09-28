//! One-time explanation for a drive that explicitly disables Trash.

use std::path::PathBuf;

use starkit::chrome::overlay::{self, Anchor};
use starkit::crossterm::event::{KeyCode, KeyEvent};
use starkit::ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Clear, Widget},
};

use crate::fold::places::Location;
use crate::ui::panels::{fit, rgb};
use crate::ui::theme::Theme;

/// Enough cached mount identity to keep a session choice on this drive.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DriveKey {
    pub path: PathBuf,
    pub source: String,
    pub uuid: Option<String>,
}

impl DriveKey {
    pub fn from_location(location: &Location) -> Self {
        let info = location
            .info
            .as_ref()
            .expect("disabled Trash has mount info");
        Self {
            path: location.path.clone(),
            source: info.source.clone(),
            uuid: info.uuid.clone(),
        }
    }
}

#[derive(Debug)]
pub struct Prompt {
    pub sources: Vec<PathBuf>,
    pub drive: DriveKey,
}

impl Prompt {
    pub fn new(sources: Vec<PathBuf>, drive: DriveKey) -> Self {
        Self { sources, drive }
    }

    pub fn key(key: KeyEvent) -> Option<bool> {
        match key.code {
            KeyCode::Char('d') => Some(false),
            KeyCode::Char('s') => Some(true),
            _ => None,
        }
    }
}

const ACTIONS: [&str; 3] = [
    "d  Delete",
    "s  Delete; skip warnings this session",
    "Esc  Cancel",
];

pub fn layout(area: Rect) -> Option<Rect> {
    let rect = overlay::rect(area, (44, 52), 6, 6, Anchor::Centre);
    (rect.width > 2 && rect.height >= 6).then_some(rect)
}

pub fn click(rect: Rect, x: u16, y: u16) -> Option<Option<bool>> {
    if x <= rect.x || x >= rect.right().saturating_sub(1) {
        return None;
    }
    match y.checked_sub(rect.y + 2) {
        Some(0) => Some(Some(false)),
        Some(1) => Some(Some(true)),
        Some(2) => Some(None),
        _ => None,
    }
}

pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, prompt: &Prompt) {
    let Some(rect) = layout(area) else {
        return;
    };
    Clear.render(rect, buf);
    let inner = overlay::render(
        rect,
        buf,
        &overlay::Overlay {
            theme,
            title: "delete permanently?",
            detail: None,
            footer: None,
        },
    );
    let source = if prompt.sources.len() == 1 {
        prompt.sources[0]
            .file_name()
            .unwrap_or(prompt.sources[0].as_os_str())
            .to_string_lossy()
            .into_owned()
    } else {
        format!("{} items", prompt.sources.len())
    };
    let lines = [
        format!("Trash is disabled on this drive: {source}"),
        ACTIONS[0].into(),
        ACTIONS[1].into(),
        ACTIONS[2].into(),
    ];
    for (row, line) in lines.iter().enumerate().take(inner.height as usize) {
        let color = if row == 0 { theme.fg } else { theme.row_fg };
        buf.set_string(
            inner.x,
            inner.y + row as u16,
            fit(line, inner.width),
            Style::default().fg(rgb(color)),
        );
    }
}
