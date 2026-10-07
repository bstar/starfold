//! Unsaved preview editor choices, sharing layout between painting and input.
use crate::ui::{
    panels::{fit, rgb},
    theme::Theme,
};
use starkit::chrome::overlay::{self, Anchor};
use starkit::ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Clear, Widget},
};
pub fn layout(area: Rect) -> Option<Rect> {
    let rect = overlay::rect(area, (44, 58), 6, 6, Anchor::Centre);
    (rect.width > 2 && rect.height >= 6).then_some(rect)
}
pub fn click(rect: Rect, x: u16, y: u16) -> Option<Option<bool>> {
    if x <= rect.x || x >= rect.right().saturating_sub(1) {
        return None;
    }
    match y.checked_sub(rect.y + 2) {
        Some(0) => Some(Some(true)),
        Some(1) => Some(Some(false)),
        Some(2) => Some(None),
        _ => None,
    }
}
pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, filename: &str) {
    let Some(rect) = layout(area) else {
        return;
    };
    Clear.render(rect, buf);
    let inner = overlay::render(
        rect,
        buf,
        &overlay::Overlay {
            theme,
            title: "unsaved changes",
            detail: None,
            footer: None,
        },
    );
    let lines = [
        format!("Save changes to {filename}?"),
        "s  Save and continue".into(),
        "d  Discard and continue".into(),
        "Esc  Cancel".into(),
    ];
    for (row, line) in lines.iter().enumerate().take(inner.height as usize) {
        buf.set_string(
            inner.x,
            inner.y + row as u16,
            fit(line, inner.width),
            Style::default().fg(rgb(theme.fg)),
        );
    }
}
