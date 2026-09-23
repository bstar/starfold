//! The stack module: the levels drilled through, and the one being looked at.
//!
//! Two things share the body: a handful of folded parent levels (crumbs,
//! oldest first) and the listing of the level currently open, under its own
//! rule line. [`split`] is the one function that decides how tall each part
//! is -- [`render`] draws from it and [`hit`] tests against it, so a click
//! can never land on a row the renderer did not draw.

use starkit::chrome::frame::{self, Badge, Tone};
use starkit::chrome::scrollbar;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;
use starkit::theme::color::Rgb;

use super::{empty, fit, rgb, width_of, words, ModuleId, HEADING};
use crate::ui::theme::Theme;
use crate::ui::{Bar, Bars};

/// Whether a row carries a mark glyph, and which.
///
/// `None` is not "unmarked" -- it is "the column does not apply here". A
/// listing with nothing marked anywhere draws with no glyph column at all,
/// per every `Row` in it being `None`; the moment one entry anywhere is
/// marked, every row everywhere becomes `Marked` or `Unmarked`, and the
/// column appears. That is the caller's job to decide (it knows the whole
/// selection, not just this listing); this module only reads the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Marked,
    Unmarked,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    Symlink { broken: bool },
    Exec,
    Hidden,
    File,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub mark: Mark,
    pub name: String,
    pub kind: Kind,
    /// `"toml"`, `"dir"`, `"link"` -- dropped below 70 columns.
    pub ext: String,
    /// `"1.2 KB"` or `"-"`.
    pub size: String,
    pub time: String,
}

/// One folded parent level: `▸ ~`, `▸ projects/`.
#[derive(Debug, Clone)]
pub struct Crumb {
    pub name: String,
    pub count: Option<usize>,
}

pub struct View<'a> {
    pub theme: &'a Theme,
    pub focused: bool,
    /// Parents only, root first -- the level currently open is not one of
    /// these, it is `rows`.
    pub crumbs: &'a [Crumb],
    /// `"starwire/ ── 14 files · 3 dirs · 84.2 MB"` or `"starwire/ ──
    /// permission denied"` -- already worded by the caller, who knows
    /// whether the level read cleanly.
    pub rule: String,
    pub rows: &'a [Row],
    pub cursor: usize,
    pub scroll: usize,
    pub fold_rows: u16,
    pub loading: bool,
    pub filter: Option<&'a str>,
    pub error: Option<&'a str>,
    pub truncated: bool,
}

/// Below this many list rows the crumb trail gives up its own height and
/// squeezes to one line. Mirrors the plan's `LIST_ROWS_MIN`.
pub const LIST_ROWS_MIN: u16 = 7;

pub struct Split {
    pub crumbs: Rect,
    pub squeezed: bool,
    pub rule: Rect,
    pub list: Rect,
}

/// Divide the body into the crumb trail, the active level's rule, and the
/// listing -- one row per crumb up to `fold_rows`, so long as that still
/// leaves [`LIST_ROWS_MIN`] rows for the listing; otherwise every crumb
/// folds into a single squeezed line so the listing keeps its floor.
pub fn split(body: Rect, depth: usize, fold_rows: u16) -> Split {
    let zero = |r: Rect| Rect {
        width: 0,
        height: 0,
        ..r
    };
    if body.height == 0 || body.width == 0 {
        return Split {
            crumbs: zero(body),
            squeezed: false,
            rule: zero(body),
            list: zero(body),
        };
    }

    let depth = u16::try_from(depth).unwrap_or(u16::MAX);
    let full_crumb_rows = depth.min(fold_rows);
    let rule_rows = 1u16.min(body.height);
    let list_if_full = body.height.saturating_sub(rule_rows + full_crumb_rows);
    // Squeezing a single crumb down to a single line buys nothing, so it is
    // only worth doing when there is more than one row of crumbs to save.
    let squeezed = depth > 1 && list_if_full < LIST_ROWS_MIN;
    let crumb_rows = if squeezed { 1 } else { full_crumb_rows };

    let crumbs = Rect {
        height: crumb_rows.min(body.height),
        ..body
    };
    let rule = Rect {
        y: body.y + crumbs.height,
        height: rule_rows.min(body.height.saturating_sub(crumbs.height)),
        ..body
    };
    let list = Rect {
        y: rule.y + rule.height,
        height: body.height.saturating_sub(crumbs.height + rule.height),
        ..body
    };
    Split {
        crumbs,
        squeezed,
        rule,
        list,
    }
}

/// How many list rows fit, for the app's scroll clamping and page keys.
pub fn visible_rows(area: Rect, depth: usize, fold_rows: u16) -> usize {
    let body = frame::body(area, &words(ModuleId::Stack));
    usize::from(split(body, depth, fold_rows).list.height)
}

const CRUMB_ARROW: char = '\u{25b8}';
const SQUEEZE_SEP: &str = " \u{203a} ";

/// The squeezed line's text, and the screen columns each crumb occupies in
/// it -- read by both [`render`] and [`hit`], so a click cannot disagree
/// with where a crumb was actually drawn.
fn squeeze_layout(x0: u16, crumbs: &[Crumb]) -> (String, Vec<(usize, u16, u16)>) {
    let mut line = format!("{CRUMB_ARROW} ");
    let mut boxes = Vec::with_capacity(crumbs.len());
    let mut x = x0 + width_of(&line);
    for (i, c) in crumbs.iter().enumerate() {
        if i > 0 {
            line.push_str(SQUEEZE_SEP);
            x += width_of(SQUEEZE_SEP);
        }
        let w = width_of(&c.name);
        boxes.push((i, x, w));
        line.push_str(&c.name);
        x += w;
    }
    (line, boxes)
}

/// Which crumbs get their own row when there is room for more than one:
/// the most recent `fold_rows - 1` (or all of them, under that), with
/// anything older folded into one row at the top rather than pushed off
/// screen unnamed. Returns how many were folded away and the slice shown.
fn visible_crumbs(crumbs: &[Crumb], fold_rows: u16) -> (usize, &[Crumb]) {
    let fold_rows = usize::from(fold_rows);
    if fold_rows == 0 || crumbs.len() <= fold_rows {
        return (0, crumbs);
    }
    let keep = fold_rows - 1;
    (crumbs.len() - keep, &crumbs[crumbs.len() - keep..])
}

pub fn render(area: Rect, buf: &mut Buffer, v: &View<'_>, bars: &mut Bars) {
    render_named(area, buf, v, bars, HEADING, None, Bar::Stack);
}

pub fn render_named(
    area: Rect,
    buf: &mut Buffer,
    v: &View<'_>,
    bars: &mut Bars,
    title: &str,
    detail: Option<&str>,
    bar: Bar,
) {
    let t = v.theme;
    let word_list = words(ModuleId::Stack);
    // The core theme type -- a struct literal is not a coercion site, so the
    // deref from this crate's own `Theme` is spelled out here.
    let core: &starkit::theme::Theme = t;
    let body = frame::frame(
        area,
        buf,
        &frame::Frame {
            theme: core,
            focused: v.focused,
            title,
            detail,
            heading: true,
            badge: Some(Badge {
                text: if matches!(bar, Bar::Commander(_)) {
                    "commander"
                } else {
                    ModuleId::Stack.title()
                },
                tone: Tone::Dim,
            }),
            footer: None,
            words: &word_list,
        },
    );

    let s = split(body, v.crumbs.len(), v.fold_rows);
    if s.crumbs.height > 0 {
        if s.squeezed {
            render_squeezed(s.crumbs, buf, t, v.crumbs);
        } else {
            render_crumb_rows(s.crumbs, buf, t, v.crumbs, v.fold_rows);
        }
    }
    if s.rule.height > 0 {
        render_rule(s.rule, buf, t, v, matches!(bar, Bar::Commander(_)));
    }
    render_list(s.list, buf, t, v);

    // The listing's own scroll position, on the panel's right border --
    // drawn last, over the corners, on the rows the list actually occupies
    // rather than the crumbs or the rule above it. Recorded through `bars`
    // rather than drawn directly, so a press or a drag on this same track
    // next frame has something to answer to.
    let track = scrollbar::track(area, s.list);
    bars.draw(bar, track, buf, t, v.rows.len() as u32, v.scroll as u32);
}

fn render_squeezed(area: Rect, buf: &mut Buffer, t: &Theme, crumbs: &[Crumb]) {
    let (line, _) = squeeze_layout(area.x, crumbs);
    buf.set_string(
        area.x,
        area.y,
        fit(&line, area.width),
        Style::default().fg(rgb(t.fold.crumb_fg)),
    );
}

fn render_crumb_rows(area: Rect, buf: &mut Buffer, t: &Theme, crumbs: &[Crumb], fold_rows: u16) {
    let (collapsed, shown) = visible_crumbs(crumbs, fold_rows);
    let style = Style::default().fg(rgb(t.fold.crumb_fg));
    for row in 0..area.height {
        let y = area.y + row;
        if collapsed > 0 && row == 0 {
            let text = format!("\u{2026} ({collapsed} more)");
            buf.set_string(area.x, y, fit(&text, area.width), style);
            continue;
        }
        let idx = usize::from(row) - usize::from(collapsed > 0);
        let Some(c) = shown.get(idx) else {
            continue;
        };
        let left = format!("{CRUMB_ARROW} {}", c.name);
        let right = c
            .count
            .map(|n| format!("{n} {}", if n == 1 { "item" } else { "items" }));
        draw_left_right(area.x, y, area.width, buf, &left, right.as_deref(), style);
    }
}

fn render_rule(area: Rect, buf: &mut Buffer, t: &Theme, v: &View<'_>, commander: bool) {
    let style = Style::default().fg(rgb(t.fold.crumb_active_fg));
    let reserved = 3u16
        .saturating_add(v.filter.map(|f| width_of(f).saturating_add(3)).unwrap_or(0))
        .saturating_add(if v.truncated { 12 } else { 0 });
    let rule = if commander {
        super::elide_middle(&v.rule, area.width.saturating_sub(reserved))
    } else {
        v.rule.clone()
    };
    let line = rule_line(area.width, &rule, v.truncated, v.filter);
    buf.set_string(area.x, area.y, line, style);
}

/// `─ name ── stats ───...─── /filter`: a leading rule, the caller's own
/// text, `(truncated)` if the read gave up early, and the live filter
/// flush against the right edge, with `─` filling whatever is left.
fn rule_line(width: u16, rule: &str, truncated: bool, filter: Option<&str>) -> String {
    let mut left = format!("\u{2500} {rule}");
    if truncated {
        left.push_str(" (truncated)");
    }
    left.push(' ');
    let right = filter.map(|f| format!(" /{f} ")).unwrap_or_default();
    let right_w = width_of(&right).min(width);
    let left_w = width_of(&left).min(width.saturating_sub(right_w));
    let fill = width.saturating_sub(left_w + right_w);
    let mut out = fit(&left, left_w);
    out.push_str(&"\u{2500}".repeat(usize::from(fill)));
    out.push_str(&fit(&right, right_w));
    out
}

fn draw_left_right(
    x: u16,
    y: u16,
    width: u16,
    buf: &mut Buffer,
    left: &str,
    right: Option<&str>,
    style: Style,
) {
    let right_w = right.map(width_of).unwrap_or(0);
    let left_w = width.saturating_sub(right_w + 1);
    buf.set_string(x, y, fit(left, left_w), style);
    if let Some(r) = right {
        let rx = x + width.saturating_sub(right_w);
        buf.set_string(rx, y, r, style);
    }
}

/// Fixed column widths, two spaces apart: `ext` drops below 70 columns,
/// `time` below 50. Chosen so a narrow split still shows a name, a size and
/// something to sort by, rather than an unreadable smear of every column
/// crushed to fit.
const MARK_W: u16 = 2;
const EXT_W: u16 = 6;
const SIZE_W: u16 = 9;
const TIME_W: u16 = 12;
const GAP: u16 = 2;

struct Cols {
    show_size: bool,
    show_ext: bool,
    show_time: bool,
    mark_w: u16,
    name_w: u16,
}

fn columns(width: u16) -> Cols {
    let show_ext = width >= 70;
    let show_time = width >= 50;
    let show_size = width >= 36;
    // The mark column is always there, blank until something is marked. A
    // column that appeared with the first mark moved every name two cells to
    // the right, which read as the whole list jumping.
    let mark_w = MARK_W;
    let mut fixed = MARK_W + GAP;
    if show_ext {
        fixed += EXT_W + GAP;
    }
    if show_size {
        fixed += SIZE_W + GAP;
    }
    if show_time {
        fixed += TIME_W + GAP;
    }
    Cols {
        show_size,
        show_ext,
        show_time,
        mark_w,
        name_w: width.saturating_sub(fixed),
    }
}

fn render_list(area: Rect, buf: &mut Buffer, t: &Theme, v: &View<'_>) {
    if v.loading {
        empty(area, buf, t, "reading\u{2026}");
        return;
    }
    if let Some(err) = v.error {
        draw_centered(area, buf, err, Style::default().fg(rgb(t.fold.error_fg)));
        return;
    }
    if v.rows.is_empty() {
        empty(area, buf, t, "empty");
        return;
    }

    let cols = columns(area.width);

    for (i, row) in v
        .rows
        .iter()
        .enumerate()
        .skip(v.scroll)
        .take(usize::from(area.height))
    {
        let y = area.y + u16::try_from(i - v.scroll).unwrap_or(0);
        render_row(
            area.x,
            y,
            area.width,
            buf,
            t,
            &cols,
            row,
            v.focused && i == v.cursor,
        );
    }
}

fn draw_centered(area: Rect, buf: &mut Buffer, text: &str, style: Style) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let y = area.y + area.height / 2;
    let fitted = fit(text, area.width);
    let trimmed = fitted.trim_end();
    let x = area.x + area.width.saturating_sub(width_of(trimmed)) / 2;
    buf.set_string(x, y, trimmed, style);
}

fn kind_fg(t: &Theme, kind: Kind) -> Rgb {
    match kind {
        Kind::Dir => t.fold.dir_fg,
        Kind::Symlink { broken: true } => t.fold.error_fg,
        Kind::Symlink { broken: false } => t.fold.symlink_fg,
        Kind::Exec => t.fold.exec_fg,
        Kind::Hidden => t.fold.hidden_fg,
        Kind::File => t.row_fg,
    }
}

fn fit_right(text: &str, width: u16) -> String {
    let fitted = fit(text, width);
    let trimmed = fitted.trim_end();
    let pad = width.saturating_sub(width_of(trimmed));
    format!("{}{trimmed}", " ".repeat(usize::from(pad)))
}

#[allow(clippy::too_many_arguments)]
fn render_row(
    x0: u16,
    y: u16,
    width: u16,
    buf: &mut Buffer,
    t: &Theme,
    cols: &Cols,
    row: &Row,
    cursor: bool,
) {
    let marked = row.mark == Mark::Marked;
    // A marked cursor row draws in the cursor's colours -- the glyph still
    // shows the mark, but which row is under the cursor is the more urgent
    // fact, and two competing highlights would fight for the reader's eye.
    let highlight_bg = if cursor {
        Some(rgb(t.row_cursor_bg))
    } else if marked {
        Some(rgb(t.fold.marked_bg))
    } else {
        None
    };
    let highlight_fg = if cursor {
        Some(rgb(t.row_cursor_fg))
    } else if marked {
        Some(rgb(t.fold.marked_fg))
    } else {
        None
    };

    if let Some(bg) = highlight_bg {
        buf.set_string(
            x0,
            y,
            " ".repeat(usize::from(width)),
            Style::default().bg(bg),
        );
    }

    let style_for = |fallback: Rgb| {
        let mut s = Style::default().fg(highlight_fg.unwrap_or_else(|| rgb(fallback)));
        if let Some(bg) = highlight_bg {
            s = s.bg(bg);
        }
        s
    };

    let mut x = x0;
    if cols.mark_w > 0 {
        let glyph = match row.mark {
            Mark::Marked => "\u{25cf}",
            Mark::Unmarked => "\u{25cb}",
            Mark::None => " ",
        };
        let glyph_fg = if marked { t.fold.marked_fg } else { t.dim };
        buf.set_string(x, y, fit(glyph, cols.mark_w), style_for(glyph_fg));
        x += cols.mark_w + GAP;
    }

    buf.set_string(
        x,
        y,
        fit(
            &format!(
                "{} {}",
                match row.kind {
                    Kind::Dir => "▸",
                    Kind::Symlink { .. } => "↗",
                    Kind::Exec => "◆",
                    _ => crate::fold::file_type::classify(std::path::Path::new(&row.name), &[])
                        .icon(),
                },
                row.name
            ),
            cols.name_w,
        ),
        style_for(kind_fg(t, row.kind)),
    );
    x += cols.name_w;

    if cols.show_ext {
        x += GAP;
        buf.set_string(x, y, fit(&row.ext, EXT_W), style_for(t.fold.kind_fg));
        x += EXT_W;
    }

    if cols.show_size {
        x += GAP;
        buf.set_string(
            x,
            y,
            fit_right(&row.size, SIZE_W),
            style_for(t.fold.size_fg),
        );
        x += SIZE_W;
    }

    if cols.show_time {
        x += GAP;
        buf.set_string(x, y, fit(&row.time, TIME_W), style_for(t.fold.time_fg));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Crumb(usize),
    Row(usize),
}

pub fn hit(area: Rect, v: &View<'_>, x: u16, y: u16) -> Option<Hit> {
    let body = frame::body(area, &words(ModuleId::Stack));
    let s = split(body, v.crumbs.len(), v.fold_rows);

    if y >= s.crumbs.y && y < s.crumbs.y + s.crumbs.height {
        return hit_crumbs(s.crumbs, v.crumbs, v.fold_rows, s.squeezed, x, y);
    }
    if y >= s.list.y && y < s.list.y + s.list.height {
        let row = v.scroll + usize::from(y - s.list.y);
        return (row < v.rows.len()).then_some(Hit::Row(row));
    }
    None
}

fn hit_crumbs(
    area: Rect,
    crumbs: &[Crumb],
    fold_rows: u16,
    squeezed: bool,
    x: u16,
    y: u16,
) -> Option<Hit> {
    if squeezed {
        let (_, boxes) = squeeze_layout(area.x, crumbs);
        return boxes
            .into_iter()
            .find(|&(_, bx, bw)| x >= bx && x < bx + bw)
            .map(|(idx, _, _)| Hit::Crumb(idx));
    }
    let (collapsed, shown) = visible_crumbs(crumbs, fold_rows);
    let row = y - area.y;
    if collapsed > 0 && row == 0 {
        return Some(Hit::Crumb(0));
    }
    let idx = usize::from(row) - usize::from(collapsed > 0);
    shown.get(idx).map(|_| Hit::Crumb(collapsed + idx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::tests_support::theme;

    fn crumb(name: &str, count: usize) -> Crumb {
        Crumb {
            name: name.to_string(),
            count: Some(count),
        }
    }

    fn row(name: &str, mark: Mark) -> Row {
        Row {
            mark,
            name: name.to_string(),
            kind: Kind::File,
            ext: "toml".into(),
            size: "1.2 KB".into(),
            time: "09:41".into(),
        }
    }

    fn line(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    /// The whole buffer, row by row -- row-major, so a horizontal run of
    /// text (a word, a message) stays contiguous in the result rather than
    /// being interleaved with the column below it.
    fn dump(buf: &Buffer, area: Rect) -> String {
        (0..area.height).map(|y| line(buf, area.y + y)).collect()
    }

    fn view<'a>(theme: &'a Theme, crumbs: &'a [Crumb], rows: &'a [Row]) -> View<'a> {
        View {
            theme,
            focused: true,
            crumbs,
            rule: "starwire/ \u{2500}\u{2500} 2 files".to_string(),
            rows,
            cursor: 0,
            scroll: 0,
            fold_rows: 6,
            loading: false,
            filter: None,
            error: None,
            truncated: false,
        }
    }

    #[test]
    fn split_gives_one_row_per_crumb_up_to_fold_rows() {
        let body = Rect::new(0, 0, 40, 30);
        let s = split(body, 3, 6);
        assert!(!s.squeezed);
        assert_eq!(s.crumbs.height, 3);
        assert_eq!(s.rule.height, 1);
        assert_eq!(s.list.height, 26);

        let capped = split(body, 9, 6);
        assert!(!capped.squeezed);
        assert_eq!(capped.crumbs.height, 6, "capped at fold_rows");
    }

    #[test]
    fn split_squeezes_when_the_list_would_fall_under_seven_rows() {
        // Height 12: a five-deep trail at one row each plus the rule leaves
        // only 6 rows for the list, under LIST_ROWS_MIN.
        let body = Rect::new(0, 0, 40, 12);
        let s = split(body, 5, 6);
        assert!(s.squeezed);
        assert_eq!(s.crumbs.height, 1);
        assert_eq!(s.list.height, 10);
    }

    #[test]
    fn hit_agrees_with_render_for_a_crumb_and_a_row() {
        let t = theme("terminal");
        let crumbs = vec![crumb("~", 12), crumb("projects/", 9)];
        let rows = vec![row("Cargo.toml", Mark::None), row("README.md", Mark::None)];
        let v = view(&t, &crumbs, &rows);
        let area = Rect::new(0, 0, 60, 30);

        let s = split(
            frame::body(area, &words(ModuleId::Stack)),
            v.crumbs.len(),
            v.fold_rows,
        );
        assert_eq!(hit(area, &v, 5, s.crumbs.y), Some(Hit::Crumb(0)));
        assert_eq!(hit(area, &v, 5, s.crumbs.y + 1), Some(Hit::Crumb(1)));
        assert_eq!(
            hit(area, &v, 5, s.list.y),
            Some(Hit::Row(0)),
            "the first list row lands where render put it"
        );
        assert_eq!(hit(area, &v, 5, s.list.y + 1), Some(Hit::Row(1)));
    }

    #[test]
    fn a_marked_row_shows_the_filled_glyph() {
        let t = theme("terminal");
        let crumbs: Vec<Crumb> = Vec::new();
        let rows = vec![
            row("Cargo.toml", Mark::Marked),
            row("README.md", Mark::Unmarked),
        ];
        let mut v = view(&t, &crumbs, &rows);
        v.cursor = 5; // off the marked row, so its own highlight shows through
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v, &mut Bars::new());
        let body = frame::body(area, &words(ModuleId::Stack));
        let s = split(body, 0, v.fold_rows);
        let first = line(&buf, s.list.y);
        assert!(first.contains('\u{25cf}'), "{first:?}");
        let second = line(&buf, s.list.y + 1);
        assert!(second.contains('\u{25cb}'), "{second:?}");
    }

    #[test]
    fn the_cursor_row_is_highlighted() {
        let t = theme("terminal");
        let crumbs: Vec<Crumb> = Vec::new();
        let rows = vec![row("a", Mark::None), row("b", Mark::None)];
        let mut v = view(&t, &crumbs, &rows);
        v.cursor = 0;
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v, &mut Bars::new());
        let body = frame::body(area, &words(ModuleId::Stack));
        let s = split(body, 0, v.fold_rows);
        let style = buf[(s.list.x, s.list.y)].style();
        assert_eq!(style.bg, Some(rgb(t.row_cursor_bg)));
    }

    #[test]
    fn a_sixty_column_width_drops_the_extension_column() {
        let cols = columns(60);
        assert!(!cols.show_ext);
        assert!(cols.show_time);
        let narrow = columns(40);
        assert!(!narrow.show_time);
    }

    #[test]
    fn loading_draws_its_own_text() {
        let t = theme("terminal");
        let crumbs: Vec<Crumb> = Vec::new();
        let rows: Vec<Row> = Vec::new();
        let mut v = view(&t, &crumbs, &rows);
        v.loading = true;
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v, &mut Bars::new());
        assert!(dump(&buf, area).contains("reading"));
    }

    #[test]
    fn an_error_draws_its_message() {
        let t = theme("terminal");
        let crumbs: Vec<Crumb> = Vec::new();
        let rows: Vec<Row> = Vec::new();
        let mut v = view(&t, &crumbs, &rows);
        v.error = Some("permission denied");
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v, &mut Bars::new());
        assert!(dump(&buf, area).contains("permission denied"));
    }

    #[test]
    fn an_empty_directory_draws_empty() {
        let t = theme("terminal");
        let crumbs: Vec<Crumb> = Vec::new();
        let rows: Vec<Row> = Vec::new();
        let v = view(&t, &crumbs, &rows);
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v, &mut Bars::new());
        assert!(dump(&buf, area).contains("empty"));
    }

    #[test]
    fn the_rule_shows_the_filter_and_truncation() {
        let line = rule_line(50, "starwire/ \u{2500}\u{2500} 2 files", true, Some("car"));
        assert!(line.contains("(truncated)"));
        assert!(line.contains("/car"));
        assert_eq!(width_of(&line), 50);
    }

    #[test]
    fn commander_keeps_the_end_of_a_long_directory_visible() {
        let t = theme("terminal");
        let mut v = view(&t, &[], &[]);
        v.rule = "/mnt/server/company/archive/photos".into();
        let area = Rect::new(0, 0, 28, 1);
        let mut buf = Buffer::empty(area);
        render_rule(area, &mut buf, &t, &v, true);
        let line = dump(&buf, area);
        assert!(line.contains("photos"), "{line}");
        assert!(line.contains('…'), "{line}");
    }

    proptest::proptest! {
        #[test]
        fn long_paths_and_filters_stay_inside_the_pane(width in 0u16..100, path in ".*", filter in ".*") {
            let line = rule_line(width, &path, true, Some(&filter));
            proptest::prop_assert!(width_of(&line) <= width);
        }
    }

    /// The listing's scroll position shows as a run of `█` on the panel's
    /// right border, over the list rows only, capped at a quarter of the
    /// list's height -- and does not appear at all once everything fits.
    #[test]
    fn the_list_shows_its_own_scrollbar_only_when_it_overflows() {
        let t = theme("terminal");
        let crumbs: Vec<Crumb> = Vec::new();
        let area = Rect::new(0, 0, 60, 20);
        let body = frame::body(area, &words(ModuleId::Stack));
        let s = split(body, 0, 6);
        let right_x = area.x + area.width - 1;

        let many: Vec<Row> = (0..40)
            .map(|i| row(&format!("file{i}"), Mark::None))
            .collect();
        let v = view(&t, &crumbs, &many);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &v, &mut Bars::new());

        let filled: Vec<u16> = (s.list.y..s.list.y + s.list.height)
            .filter(|&y| buf[(right_x, y)].symbol() == "\u{2588}")
            .collect();
        assert!(!filled.is_empty(), "no thumb drawn on an overflowing list");
        assert!(
            filled.len() as u16 <= s.list.height / 4,
            "thumb of {} rows is more than a quarter of {}",
            filled.len(),
            s.list.height
        );
        for pair in filled.windows(2) {
            assert_eq!(pair[1], pair[0] + 1, "the thumb is not one contiguous run");
        }
        for y in s.list.y..s.list.y + s.list.height {
            if !filled.contains(&y) {
                assert_eq!(
                    buf[(right_x, y)].symbol(),
                    "\u{2551}",
                    "row {y} is not plain border"
                );
            }
        }

        let few: Vec<Row> = (0..3)
            .map(|i| row(&format!("file{i}"), Mark::None))
            .collect();
        let v2 = view(&t, &crumbs, &few);
        let mut buf2 = Buffer::empty(area);
        render(area, &mut buf2, &v2, &mut Bars::new());
        for y in s.list.y..s.list.y + s.list.height {
            assert_ne!(
                buf2[(right_x, y)].symbol(),
                "\u{2588}",
                "a list that fits should draw no thumb"
            );
        }
    }
}
