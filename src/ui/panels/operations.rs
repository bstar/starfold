//! The operations module: the transactional queue, and how far it has got.
//!
//! An [`OpRow`] is already worded by the caller -- `title`, `status` and the
//! optional `bar` are strings, not an `Op` to interpret -- because this
//! module never touches `fold::ops` directly; it only draws what it is
//! handed, the same separation `panels::stack` keeps from `fold::listing`.

use starkit::chrome::frame;
use starkit::chrome::scrollbar;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;
use starkit::theme::color::Rgb;

use super::{elide_middle, empty, rgb, summary_row, width_of, words, ModuleId};
use crate::ui::theme::Theme;
use crate::ui::{Bar, Bars};

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
    /// A volume unmount is worker activity, not a removable queue entry.
    pub unmounting: Option<&'a str>,
    pub spinner: &'a str,
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

pub fn render(area: Rect, buf: &mut Buffer, v: &View<'_>, bars: &mut Bars) {
    let detail = v
        .unmounting
        .map(|_| activity_status(v).to_string())
        .or_else(|| running(v).map(|r| r.status.clone()));
    let word_list = words(ModuleId::Operations);
    // The core theme type -- a struct literal is not a coercion site, so the
    // deref from this crate's own `Theme` is spelled out here.
    let core: &starkit::theme::Theme = v.theme;
    let body = frame::frame(
        area,
        buf,
        &frame::Frame {
            theme: core,
            focused: v.focused,
            title: ModuleId::Operations.title(),
            detail: detail.as_deref(),
            heading: false,
            badge: None,
            footer: None,
            words: &word_list,
        },
    );

    if v.folded {
        render_folded(body, buf, v);
        return;
    }
    render_open(area, body, buf, v, bars);
}

fn render_folded(area: Rect, buf: &mut Buffer, v: &View<'_>) {
    if let Some(name) = v.unmounting {
        let status = format!(" · {}", activity_status(v));
        let title_w = area.width.saturating_sub(width_of(&status));
        summary_row(
            area,
            buf,
            &format!(
                "{}{status}",
                elide_middle(&format!("{} UNMOUNT {name}", v.spinner), title_w)
            ),
            Style::default().fg(rgb(v.theme.fold.progress_fg)),
        );
        return;
    }
    let row = running(v).or_else(|| v.rows.iter().find(|r| r.tone == Tone::Pending));
    match row {
        Some(r) => {
            // The status is the part worth reading on a folded row; a long
            // destination gives way in the middle so it stays.
            let status = format!(" \u{b7} {}", r.status);
            let title_w = area.width.saturating_sub(width_of(&status));
            summary_row(
                area,
                buf,
                &format!("{}{status}", elide_middle(&r.title, title_w)),
                Style::default().fg(rgb(tone_fg(v.theme, r.tone))),
            )
        }
        None => empty(area, buf, v.theme, "nothing queued"),
    }
}

fn render_open(outer: Rect, area: Rect, buf: &mut Buffer, v: &View<'_>, bars: &mut Bars) {
    let mut area = area;
    if let Some(name) = v.unmounting {
        if area.height > 0 {
            let status = activity_status(v);
            let right_w = width_of(status).min(area.width);
            let left_w = area.width.saturating_sub(right_w + 1);
            let style = Style::default().fg(rgb(v.theme.fold.progress_fg));
            buf.set_string(
                area.x,
                area.y,
                elide_middle(&format!("{} UNMOUNT {name}", v.spinner), left_w),
                style,
            );
            buf.set_string(area.x + area.width - right_w, area.y, status, style);
            area.y += 1;
            area.height -= 1;
        }
    }
    if v.rows.is_empty() {
        if v.unmounting.is_none() {
            empty(area, buf, v.theme, "nothing queued");
        }
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
        buf.set_string(area.x, y, elide_middle(&row.title, left_w), style);
        let rx = area.x + area.width.saturating_sub(right_w);
        buf.set_string(rx, y, &right, style);
    }

    let track = scrollbar::track(outer, area);
    bars.draw(
        Bar::Operations,
        track,
        buf,
        t,
        v.rows.len() as u32,
        v.scroll as u32,
    );
}

pub fn hit(area: Rect, v: &View<'_>, x: u16, y: u16) -> Option<usize> {
    if v.folded {
        return None;
    }
    let body = frame::body(area, &words(ModuleId::Operations));
    if x < body.x || x >= body.x + body.width || y < body.y || y >= body.y + body.height {
        return None;
    }
    let offset = usize::from(v.unmounting.is_some());
    let row = usize::from(y - body.y);
    if row < offset {
        return None;
    }
    let index = v.scroll + row - offset;
    (index < v.rows.len()).then_some(index)
}

fn activity_status(v: &View<'_>) -> &'static str {
    if running(v).is_some() {
        "waiting"
    } else {
        "unmounting"
    }
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
            unmounting: None,
            spinner: "⠋",
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
        render(area, &mut buf, &v, &mut Bars::new());
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
        render(area, &mut buf, &v, &mut Bars::new());
        assert!(dump(&buf, area).contains("queued"));

        let empty_rows: Vec<OpRow> = Vec::new();
        let mut v2 = view(&t, &empty_rows);
        v2.folded = true;
        let mut buf2 = Buffer::empty(area);
        render(area, &mut buf2, &v2, &mut Bars::new());
        assert!(dump(&buf2, area).contains("nothing queued"));
    }

    #[test]
    fn unmount_activity_is_visible_but_not_a_selectable_queue_row() {
        let t = theme("terminal");
        let rows = vec![row("COPY 1 item → ~/Work", "queued", Tone::Pending)];
        let mut v = view(&t, &rows);
        v.unmounting = Some("Camera");
        v.spinner = "⠹";
        let area = Rect::new(0, 0, 60, 7);
        let body = frame::body(area, &words(ModuleId::Operations));
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v, &mut Bars::new());
        let text = dump(&buf, area);
        assert!(text.contains("⠹ UNMOUNT Camera"), "{text:?}");
        assert!(text.contains("COPY 1 item"), "{text:?}");
        assert_eq!(hit(area, &v, 5, body.y), None);
        assert_eq!(hit(area, &v, 5, body.y + 1), Some(0));

        v.folded = true;
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v, &mut Bars::new());
        assert!(dump(&buf, area).contains("⠹ UNMOUNT Camera"));
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
        render(area, &mut buf, &v, &mut Bars::new());
        let body = frame::body(area, &words(ModuleId::Operations));
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
        render(area, &mut buf, &v, &mut Bars::new());
        let title: String = (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(title.contains("copying 42%"), "{title:?}");
    }
}
