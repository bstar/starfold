//! The preview module: what the entry under the cursor is, before it is
//! opened.
//!
//! Most of this is text -- a head, a hexdump, a directory summary -- and
//! [`lines`] is the one function that turns a [`Preview`] into rows of it,
//! shared between what [`render`] draws and what the app clamps `scroll`
//! against, so the two can never disagree about how tall a preview is.
//!
//! ## Painting a picture
//!
//! A protocol image is not a cell: it is an escape sequence the terminal
//! places over a rectangle, and encoding one takes a `&mut Graphics`. This
//! module is handed one (see [`View::graphics`]) but, like STAR/CORD's
//! message list, never calls [`Graphics::protocol`] itself -- it only
//! decides *where* the picture goes and hands that rectangle back as a
//! [`Placement`] for the app to paint in a second pass, after every panel
//! and before the overlays. The reason is the same as STAR/CORD's: an
//! overlay drawn afterwards clears the cells under it, and a protocol image
//! painted before that would be wiped the instant something opens on top of
//! it. `halfblocks` and the quiet `placeholder` need no protocol and no
//! second pass, so those two are painted here directly and `render` answers
//! `None` for them.

use std::sync::Arc;

use starkit::chrome::frame;
use starkit::chrome::scrollbar;
use starkit::graphics::{Graphics, ImageId};
use starkit::image::RgbaImage;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;

use super::{empty, fit, rgb, width_of, words, ModuleId};
use crate::fold::preview::Preview;
use crate::fold::summary::DirSummary;
use crate::ui::theme::Theme;

/// Where a picture the app must paint itself belongs, and which one it is --
/// see the module doc for why painting is not done here.
pub struct Placement {
    pub area: Rect,
    pub id: ImageId,
}

pub struct View<'a> {
    pub theme: &'a Theme,
    pub focused: bool,
    pub folded: bool,
    /// The title detail: `Some("Cargo.toml")` draws as `PREVIEW —
    /// Cargo.toml` once [`frame`] composes it.
    pub name: Option<&'a str>,
    pub preview: Option<&'a Preview>,
    pub scroll: usize,
    /// `None` means draw with half blocks or a quiet placeholder rather
    /// than a real protocol -- see the module doc.
    pub graphics: Option<&'a mut Graphics>,
}

pub fn render(area: Rect, buf: &mut Buffer, v: &mut View<'_>) -> Option<Placement> {
    let word_list = words(ModuleId::Preview);
    // The core theme type -- a struct literal is not a coercion site, so the
    // deref from this crate's own `Theme` is spelled out here.
    let core: &starkit::theme::Theme = v.theme;
    let body = frame::frame(
        area,
        buf,
        &frame::Frame {
            theme: core,
            focused: v.focused,
            title: ModuleId::Preview.title(),
            detail: v.name,
            heading: false,
            badge: None,
            footer: None,
            words: &word_list,
        },
    );

    if v.folded {
        render_folded(body, buf, v);
        return None;
    }

    let Some(preview) = v.preview else {
        empty(body, buf, v.theme, "nothing to preview");
        return None;
    };

    match preview {
        Preview::Empty => {
            empty(body, buf, v.theme, "empty file");
            None
        }
        Preview::Text { bytes, .. } => {
            render_text(area, body, buf, v.theme, preview, *bytes, v.name, v.scroll);
            None
        }
        Preview::Binary {
            mime, truncated, ..
        } => {
            render_binary(
                area, body, buf, v.theme, preview, mime, *truncated, v.scroll,
            );
            None
        }
        Preview::Dir(summary) => {
            render_dir(body, buf, v.theme, summary, v.name);
            None
        }
        Preview::Symlink { broken, .. } => {
            let style = if *broken {
                Style::default().fg(rgb(v.theme.fold.error_fg))
            } else {
                Style::default().fg(rgb(v.theme.row_fg))
            };
            render_plain(body, buf, preview, style);
            None
        }
        Preview::Error(_) => {
            let style = Style::default().fg(rgb(v.theme.fold.error_fg));
            render_plain(body, buf, preview, style);
            None
        }
        Preview::Image {
            data,
            width,
            height,
            format,
        } => {
            let data = Arc::clone(data);
            render_image(body, buf, v, &data, *width, *height, format)
        }
    }
}

/// Text lines for `Text`, `Binary`, `Dir`, `Symlink` and `Error`, fitted to
/// `width` -- what `render` draws from and what the app clamps `scroll`
/// against, so a preview never scrolls further than it is actually tall.
pub fn lines(preview: &Preview, width: u16) -> Vec<String> {
    match preview {
        // `lines()`, not `split('\n')`: a text file almost always ends in a
        // newline, and `split` turns that into a phantom empty last line --
        // one more row than a reader can see, which used to draw a
        // scrollbar on a preview that did not actually overflow. `lines()`
        // drops exactly the one optional trailing terminator (and handles
        // `\r\n` besides); a genuine blank line, in the middle or doubled at
        // the end, still comes through.
        Preview::Text { head, .. } => head
            .lines()
            .map(|l| fit(&l.replace('\t', "    "), width))
            .collect(),
        Preview::Binary { rows, .. } => rows.iter().map(|r| fit(r, width)).collect(),
        Preview::Dir(summary) => dir_lines(summary).iter().map(|l| fit(l, width)).collect(),
        Preview::Symlink { target, broken } => {
            let mut s = format!("\u{2192} {}", target.display());
            if *broken {
                s.push_str(" (broken)");
            }
            vec![fit(&s, width)]
        }
        Preview::Error(e) => vec![fit(e, width)],
        Preview::Empty | Preview::Image { .. } => Vec::new(),
    }
}

fn dir_lines(s: &DirSummary) -> Vec<String> {
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let mut out = vec![
        plural(s.files, "file", "files"),
        plural(s.dirs, "directory", "directories"),
        crate::fold::format::size(s.bytes),
    ];
    if s.truncated {
        out.push("(at least)".to_string());
    }
    out
}

/// The extension a folded summary and a text meta line both show, from the
/// name alone -- `"Cargo.toml"` gives `"toml"`.
fn ext_of(name: &str) -> Option<&str> {
    name.rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|e| !e.is_empty())
}

fn render_plain(area: Rect, buf: &mut Buffer, preview: &Preview, style: Style) {
    for (row, line) in lines(preview, area.width)
        .into_iter()
        .enumerate()
        .take(usize::from(area.height))
    {
        buf.set_string(area.x, area.y + row as u16, line, style);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_text(
    outer: Rect,
    area: Rect,
    buf: &mut Buffer,
    t: &Theme,
    preview: &Preview,
    bytes: u64,
    name: Option<&str>,
    scroll: usize,
) {
    if area.height > 0 {
        let meta = format!(
            "{} \u{b7} {}",
            name.and_then(ext_of).unwrap_or("-"),
            crate::fold::format::size(bytes)
        );
        draw_meta(area, buf, t, &meta);
    }
    let rows = lines(preview, area.width);
    let style = Style::default().fg(rgb(t.row_fg));
    let body = below_meta(area);
    for (row, text) in rows
        .iter()
        .skip(scroll)
        .take(usize::from(body.height))
        .enumerate()
    {
        buf.set_string(body.x, body.y + row as u16, fit(text, body.width), style);
    }
    render_scrollbar(outer, body, buf, t, scroll, rows.len());
}

#[allow(clippy::too_many_arguments)]
fn render_binary(
    outer: Rect,
    area: Rect,
    buf: &mut Buffer,
    t: &Theme,
    preview: &Preview,
    mime: &str,
    truncated: bool,
    scroll: usize,
) {
    if area.height > 0 {
        let meta = if truncated {
            format!("{mime} \u{b7} truncated")
        } else {
            mime.to_string()
        };
        draw_meta(area, buf, t, &meta);
    }
    let rows = lines(preview, area.width);
    let style = Style::default().fg(rgb(t.row_fg));
    let body = below_meta(area);
    for (row, text) in rows
        .iter()
        .skip(scroll)
        .take(usize::from(body.height))
        .enumerate()
    {
        buf.set_string(body.x, body.y + row as u16, fit(text, body.width), style);
    }
    render_scrollbar(outer, body, buf, t, scroll, rows.len());
}

/// The scroll position of a text or binary preview, on the panel's own right
/// border -- drawn over the rows the content actually occupies, below the
/// meta line, the same mark every other scrolling list in the column draws.
fn render_scrollbar(
    outer: Rect,
    body: Rect,
    buf: &mut Buffer,
    t: &Theme,
    scroll: usize,
    len: usize,
) {
    let thumb = scrollbar::rows(scroll, len, body.height);
    let track = scrollbar::track(outer, body);
    scrollbar::render(track, buf, t, thumb);
}

fn render_dir(area: Rect, buf: &mut Buffer, t: &Theme, summary: &DirSummary, name: Option<&str>) {
    let mut all: Vec<String> = Vec::new();
    if let Some(n) = name {
        all.push(n.to_string());
        all.push(String::new());
    }
    all.extend(dir_lines(summary));
    let style = Style::default().fg(rgb(t.row_fg));
    for (row, text) in all.iter().enumerate().take(usize::from(area.height)) {
        buf.set_string(area.x, area.y + row as u16, fit(text, area.width), style);
    }
}

/// What is left under the meta line. The line has a row of its own rather
/// than the right end of the first content row: a hexdump row is seventy
/// columns wide and the mime type drawn over its tail was unreadable both
/// ways.
fn below_meta(area: Rect) -> Rect {
    Rect {
        y: area.y + 1.min(area.height),
        height: area.height.saturating_sub(1),
        ..area
    }
}

/// A dim, right-aligned line on the preview's first row: `toml · 1.2 KB`.
fn draw_meta(area: Rect, buf: &mut Buffer, t: &Theme, meta: &str) {
    let w = width_of(meta).min(area.width);
    let x = area.x + area.width - w;
    buf.set_string(x, area.y, meta, Style::default().fg(rgb(t.dim)));
}

#[allow(clippy::too_many_arguments)]
fn render_image(
    area: Rect,
    buf: &mut Buffer,
    v: &mut View<'_>,
    data: &Arc<RgbaImage>,
    width: u32,
    height: u32,
    format: &str,
) -> Option<Placement> {
    if area.height == 0 || area.width == 0 {
        return None;
    }
    let meta = format!("{format} \u{b7} {width} \u{d7} {height}");
    if area.height == 1 {
        draw_meta(area, buf, v.theme, &meta);
        return None;
    }
    let pic = Rect {
        height: area.height - 1,
        ..area
    };
    let meta_row = Rect {
        y: pic.y + pic.height,
        height: 1,
        ..area
    };

    let aspect = v
        .graphics
        .as_deref()
        .and_then(Graphics::cell_aspect)
        .unwrap_or(2.0);
    let fitted = fit_aspect(pic, width, height, aspect);

    let usable = v
        .graphics
        .as_deref()
        .map(Graphics::pictures_available)
        .unwrap_or(false);

    let placement = if usable {
        // Left for the app's second pass -- see the module doc.
        Some(Placement {
            area: fitted,
            id: ImageId::of_arc(data),
        })
    } else if width == 0 || height == 0 {
        starkit::graphics::placeholder(fitted, buf, Style::default().fg(rgb(v.theme.dim)));
        None
    } else {
        starkit::graphics::halfblocks(data, fitted, buf);
        None
    };

    draw_meta(meta_row, buf, v.theme, &meta);
    placement
}

/// Fit an `img_w`×`img_h` picture into `area` keeping its aspect, accounting
/// for a terminal cell being `cell_aspect` times taller than it is wide --
/// without that correction a square picture would draw as a tall rectangle.
fn fit_aspect(area: Rect, img_w: u32, img_h: u32, cell_aspect: f32) -> Rect {
    if area.width == 0 || area.height == 0 || img_w == 0 || img_h == 0 {
        return area;
    }
    let img_aspect = (img_h as f32 / img_w as f32) / cell_aspect;
    let area_aspect = area.height as f32 / area.width as f32;
    let (w, h) = if img_aspect > area_aspect {
        let h = area.height;
        let w = ((h as f32) / img_aspect).round().max(1.0) as u16;
        (w.min(area.width), h)
    } else {
        let w = area.width;
        let h = ((w as f32) * img_aspect).round().max(1.0) as u16;
        (w, h.min(area.height))
    };
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn folded_summary(v: &View<'_>) -> Option<String> {
    let preview = v.preview?;
    let name = v.name.unwrap_or("-");
    Some(match preview {
        Preview::Empty => "empty file".to_string(),
        Preview::Text { bytes, lines, .. } => format!(
            "{name} \u{b7} {} \u{b7} {} \u{b7} {lines} lines",
            ext_of(name).unwrap_or("-"),
            crate::fold::format::size(*bytes)
        ),
        Preview::Image {
            width,
            height,
            format,
            ..
        } => format!("{name} \u{b7} {format} \u{b7} {width} \u{d7} {height}"),
        Preview::Dir(summary) => format!(
            "{name} \u{b7} {} files \u{b7} {} dirs \u{b7} {}",
            summary.files,
            summary.dirs,
            crate::fold::format::size(summary.bytes)
        ),
        Preview::Binary {
            mime, truncated, ..
        } => {
            if *truncated {
                format!("{name} \u{b7} {mime} \u{b7} truncated")
            } else {
                format!("{name} \u{b7} {mime}")
            }
        }
        Preview::Symlink { target, broken } => {
            let arrow = format!("\u{2192} {}", target.display());
            if *broken {
                format!("{arrow} (broken)")
            } else {
                arrow
            }
        }
        Preview::Error(e) => e.clone(),
    })
}

fn render_folded(area: Rect, buf: &mut Buffer, v: &View<'_>) {
    match folded_summary(v) {
        Some(text) => {
            super::summary_row(area, buf, &text, Style::default().fg(rgb(v.theme.row_fg)))
        }
        None => empty(area, buf, v.theme, "nothing to preview"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::tests_support::theme;
    use std::path::PathBuf;

    fn view<'a>(theme: &'a Theme, preview: Option<&'a Preview>, name: Option<&'a str>) -> View<'a> {
        View {
            theme,
            focused: true,
            folded: false,
            name,
            preview,
            scroll: 0,
            graphics: None,
        }
    }

    #[test]
    fn a_text_file_folds_to_name_extension_size_and_line_count() {
        let t = theme("terminal");
        let p = Preview::Text {
            head: "abc\n".into(),
            truncated: false,
            bytes: 1200,
            lines: 31,
        };
        let mut v = view(&t, Some(&p), Some("Cargo.toml"));
        v.folded = true;
        assert_eq!(
            folded_summary(&v).unwrap(),
            "Cargo.toml \u{b7} toml \u{b7} 1.2 KB \u{b7} 31 lines"
        );
    }

    #[test]
    fn an_image_folds_to_name_format_and_dimensions() {
        let t = theme("terminal");
        let img = Arc::new(RgbaImage::new(1, 1));
        let p = Preview::Image {
            data: img,
            width: 64,
            height: 48,
            format: "png",
        };
        let v = view(&t, Some(&p), Some("harbour.png"));
        assert_eq!(
            folded_summary(&v).unwrap(),
            "harbour.png \u{b7} png \u{b7} 64 \u{d7} 48"
        );
    }

    #[test]
    fn a_directory_folds_to_its_name_and_counts() {
        let t = theme("terminal");
        let s = DirSummary {
            files: 14,
            dirs: 3,
            bytes: 428_000,
            truncated: false,
        };
        let p = Preview::Dir(s);
        let v = view(&t, Some(&p), Some("src/"));
        assert_eq!(
            folded_summary(&v).unwrap(),
            "src/ \u{b7} 14 files \u{b7} 3 dirs \u{b7} 428.0 KB"
        );
    }

    #[test]
    fn a_symlink_folds_to_an_arrow_and_its_target() {
        let t = theme("terminal");
        let p = Preview::Symlink {
            target: PathBuf::from("target"),
            broken: false,
        };
        let v = view(&t, Some(&p), Some("build"));
        assert_eq!(folded_summary(&v).unwrap(), "\u{2192} target");

        let broken = Preview::Symlink {
            target: PathBuf::from("gone"),
            broken: true,
        };
        let v2 = view(&t, Some(&broken), Some("dangling"));
        assert_eq!(folded_summary(&v2).unwrap(), "\u{2192} gone (broken)");
    }

    #[test]
    fn an_error_folds_to_its_message() {
        let t = theme("terminal");
        let p = Preview::Error("permission denied".into());
        let v = view(&t, Some(&p), Some("secret"));
        assert_eq!(folded_summary(&v).unwrap(), "permission denied");
    }

    #[test]
    fn text_scrolls_from_the_given_offset() {
        let t = theme("terminal");
        let p = Preview::Text {
            head: "one\ntwo\nthree\nfour\n".into(),
            truncated: false,
            bytes: 20,
            lines: 4,
        };
        let mut v = view(&t, Some(&p), Some("f.txt"));
        v.scroll = 2;
        let area = Rect::new(0, 0, 30, 10);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &mut v);
        // The first body row is the meta line; the text starts under it.
        let body = frame::body(area, &words(ModuleId::Preview));
        let row1: String = (0..10)
            .map(|x| buf[(body.x + x, body.y + 1)].symbol().to_string())
            .collect();
        assert!(row1.starts_with("three"), "{row1:?}");
    }

    #[test]
    fn lines_renders_a_hexdump_row_per_row() {
        let p = Preview::Binary {
            rows: vec!["00000000  48 65 6c 6c 6f  |Hello|".to_string()],
            truncated: false,
            mime: "application/octet-stream".into(),
        };
        let out = lines(&p, 80);
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with("00000000"));
    }

    /// A text file's row count is the file's own line count, not one more
    /// than it -- almost every text file ends in a newline, and counting
    /// `split('\n')`'s phantom empty last element used to overflow a preview
    /// that had nothing left to scroll to.
    #[test]
    fn lines_counts_a_text_files_rows_the_way_the_file_does() {
        let text = |head: &str| Preview::Text {
            head: head.to_string(),
            truncated: false,
            bytes: head.len() as u64,
            lines: 0,
        };
        assert_eq!(
            lines(&text("a\nb\n"), 80).len(),
            2,
            "a single trailing newline is not a phantom third line"
        );
        assert_eq!(
            lines(&text("a\n\nb"), 80).len(),
            3,
            "a genuine blank line in the middle still counts"
        );
        assert_eq!(
            lines(&text("a\n\n"), 80).len(),
            2,
            "a blank line doubled at the end still counts, once"
        );
    }

    #[test]
    fn an_image_with_no_graphics_draws_half_blocks() {
        let t = theme("terminal");
        let mut img = RgbaImage::new(4, 4);
        for p in img.pixels_mut() {
            *p = starkit::image::Rgba([200, 100, 50, 255]);
        }
        let data = Arc::new(img);
        let mut v = view(&t, None, Some("harbour.png"));
        let body = Rect::new(0, 0, 10, 5);
        let mut buf = Buffer::empty(body);
        let placement = render_image(body, &mut buf, &mut v, &data, 4, 4, "png");
        assert!(placement.is_none(), "no protocol means nothing deferred");
        let text: String = (0..body.width)
            .flat_map(|x| (0..body.height).map(move |y| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(text.contains('\u{2580}'), "{text:?}");
    }
}
