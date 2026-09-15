//! The operations module: the transactional queue, and how far it has got.
//!
//! An [`OpRow`] is already worded by the caller -- `title`, `status` and the
//! optional `bar` are strings, not an `Op` to interpret -- because this
//! module never touches `fold::ops` directly; it only draws what it is
//! handed, the same separation `panels::stack` keeps from `fold::listing`.

use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;
use starkit::theme::color::Rgb;

use super::{empty, fit, frame, rgb, summary_row, width_of, words, Frame, ModuleId};
use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Pending,
    Running,
    Conflict,
    Done,
    Failed,
}

/// One queued operation, already worded: `title` is `COPY 2 items →
/// ~/Archive`, `status` is `queued`, `waiting: 3 conflicts`, `copying 78%`,
/// `done`, `3 of 14 failed` or `cancelled`, and `bar` is
/// `Progress::bar(10)`'s output while the op is actually running.
#[derive(Debug, Clone)]
pub struct OpRow {
    pub title: String,
    pub status: String,
    pub bar: Option<String>,
    pub tone: Tone,
}

pub struct View<'a> {
    pub theme: &'a Theme,
    pub focused: bool,
    pub folded: bool,
    pub rows: &'a [OpRow],
    pub cursor: usize,
    pub scroll: usize,
    /// `"enter run · esc clear"` -- shown beside the cursor row's own status
    /// while the module has focus, the same way a status bar's key hints
    /// only mean anything once you know where you are.
    pub hint: &'a str,
}

fn tone_fg(t: &Theme, tone: Tone) -> Rgb {
    match tone {
        Tone::Pending => t.dim,
        Tone::Running => t.fold.progress_fg,
        Tone::Conflict => t.fold.conflict_fg,
        Tone::Done => t.ok,
        Tone::Failed => t.fold.error_fg,
    }
}

/// The op the ops thread is actually on, if any -- there is never more than
/// one at a time. What the folded row and the title's `— copying 78%`
/// detail both come from.
fn running<'a>(v: &View<'a>) -> Option<&'a OpRow> {
    v.rows.iter().find(|r| r.tone == Tone::Running)
}

pub fn render(area: Rect, buf: &mut Buffer, v: &View<'_>) {
    let detail = running(v).map(|r| r.status.clone());
    let word_list = words(ModuleId::Operations);
    let body = frame(
        area,
        buf,
        &Frame {
            theme: v.theme,
            focused: v.focused,
            name: ModuleId::Operations.title(),
            detail: detail.as_deref(),
            heading: false,
            words: &word_list,
        },
    );

    if v.folded {
        render_folded(body, buf, v);
        return;
    }
    render_open(body, buf, v);
}

fn render_folded(area: Rect, buf: &mut Buffer, v: &View<'_>) {
    let row = running(v).or_else(|| v.rows.iter().find(|r| r.tone == Tone::Pending));
    match row {
        Some(r) => summary_row(
            area,
            buf,
            &format!("{} \u{b7} {}", r.title, r.status),
            Style::default().fg(rgb(tone_fg(v.theme, r.tone))),
        ),
        None => empty(area, buf, v.theme, "nothing queued"),
    }
}

fn render_open(area: Rect, buf: &mut Buffer, v: &View<'_>) {
    if v.rows.is_empty() {
        empty(area, buf, v.theme, "nothing queued");
        return;
    }
    let t = v.theme;
    for (i, row) in v
        .rows
        .iter()
        .enumerate()
        .skip(v.scroll)
        .take(usize::from(area.height))
    {
        let y = area.y + u16::try_from(i - v.scroll).unwrap_or(0);
        let cursor = v.focused && i == v.cursor;
        let fg = if cursor {
            t.row_cursor_fg
        } else {
            tone_fg(t, row.tone)
        };

        let mut style = Style::default().fg(rgb(fg));
        if cursor {
            let bg = rgb(t.row_cursor_bg);
            style = style.bg(bg);
            buf.set_string(area.x, y, " ".repeat(usize::from(area.width)), style);
        }

        let mut right = row.bar.clone().unwrap_or_else(|| row.status.clone());
        if cursor && !v.hint.is_empty() {
            right = format!("{right} \u{b7} {}", v.hint);
        }
        let right_w = width_of(&right).min(area.width);
        let left_w = area.width.saturating_sub(right_w + 1);
        buf.set_string(area.x, y, fit(&row.title, left_w), style);
        let rx = area.x + area.width.saturating_sub(right_w);
        buf.set_string(rx, y, &right, style);
    }
}

pub fn hit(area: Rect, v: &View<'_>, x: u16, y: u16) -> Option<usize> {
    if v.folded {
        return None;
    }
    let body = starkit::chrome::header::body(area);
    if x < body.x || x >= body.x + body.width || y < body.y || y >= body.y + body.height {
        return None;
    }
    let index = v.scroll + usize::from(y - body.y);
    (index < v.rows.len()).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::tests_support::theme;

    fn row(title: &str, status: &str, tone: Tone) -> OpRow {
        OpRow {
            title: title.to_string(),
            status: status.to_string(),
            bar: None,
            tone,
        }
    }

    fn view<'a>(theme: &'a Theme, rows: &'a [OpRow]) -> View<'a> {
        View {
            theme,
            focused: true,
            folded: false,
            rows,
            cursor: 0,
            scroll: 0,
            hint: "enter run \u{b7} esc clear",
        }
    }

    /// The whole buffer, row by row -- row-major, so a horizontal run of
    /// text stays contiguous rather than being interleaved with the column
    /// below it.
    fn dump(buf: &Buffer, area: Rect) -> String {
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn folded_shows_the_running_op_over_a_pending_one() {
        let t = theme("terminal");
        let rows = vec![
            row("COPY 2 items \u{2192} ~/Archive", "queued", Tone::Pending),
            row("MOVE 1 item \u{2192} ~/Docs", "copying 78%", Tone::Running),
        ];
        let mut v = view(&t, &rows);
        v.folded = true;
        // Border top, header row, one body row, border bottom.
        let area = Rect::new(0, 0, 60, 4);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v);
        assert!(dump(&buf, area).contains("copying 78%"));
    }

    #[test]
    fn folded_falls_back_to_the_first_pending_op_then_to_nothing_queued() {
        let t = theme("terminal");
        let rows = vec![row(
            "COPY 2 items \u{2192} ~/Archive",
            "queued",
            Tone::Pending,
        )];
        let mut v = view(&t, &rows);
        v.folded = true;
        let area = Rect::new(0, 0, 60, 4);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v);
        assert!(dump(&buf, area).contains("queued"));

        let empty_rows: Vec<OpRow> = Vec::new();
        let mut v2 = view(&t, &empty_rows);
        v2.folded = true;
        let mut buf2 = Buffer::empty(area);
        render(area, &mut buf2, &v2);
        assert!(dump(&buf2, area).contains("nothing queued"));
    }

    #[test]
    fn the_open_view_lists_every_row_and_hit_answers_the_pointer() {
        let t = theme("terminal");
        let rows = vec![
            row("COPY 2 items \u{2192} ~/Archive", "queued", Tone::Pending),
            row("MOVE 1 item \u{2192} ~/Docs", "done", Tone::Done),
        ];
        let v = view(&t, &rows);
        let area = Rect::new(0, 0, 60, 6);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v);
        let body = starkit::chrome::header::body(area);
        let text = dump(&buf, area);
        assert!(text.contains("COPY 2 items"), "{text:?}");
        assert!(text.contains("MOVE 1 item"), "{text:?}");

        assert_eq!(hit(area, &v, 5, body.y), Some(0));
        assert_eq!(hit(area, &v, 5, body.y + 1), Some(1));
        assert_eq!(hit(area, &v, 5, body.y + 2), None);
    }

    #[test]
    fn the_title_carries_the_running_percentage() {
        let t = theme("terminal");
        let rows = vec![row(
            "COPY 2 items \u{2192} ~/Archive",
            "copying 42%",
            Tone::Running,
        )];
        let v = view(&t, &rows);
        let area = Rect::new(0, 0, 60, 4);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v);
        let title: String = (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(title.contains("copying 42%"), "{title:?}");
    }
}
