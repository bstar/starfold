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
//!
//! ## Growing one
//!
//! `Resize::Fit`, which STAR/KIT encodes with, never upsizes -- its own
//! documentation says so. A picture smaller than the panel therefore sits in
//! the middle of it at whatever size it happens to be, and growing it is
//! this application's own work: scale the pixels first, to exactly the size
//! of the rectangle they will be placed over, and hand *that* picture to
//! [`Graphics::protocol`]. [`plan`] is the whole of the arithmetic -- which
//! rectangle, how many pixels into it, and with which filter -- and it is
//! pure, so the three modes can be read side by side and tested without a
//! terminal.

use std::sync::Arc;

use starkit::chrome::frame;
use starkit::chrome::scrollbar;
use starkit::graphics::{Graphics, ImageId};
use starkit::image::imageops::FilterType;
use starkit::image::RgbaImage;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::Style;

use super::{empty, fit, rgb, width_of, words, ModuleId};
use crate::config::Scale;
use crate::fold::preview::Preview;
use crate::fold::summary::DirSummary;
use crate::ui::theme::Theme;
use crate::ui::{Bar, Bars};

/// Where a picture the app must paint itself belongs, which one it is, and
/// what it has to be scaled to first -- see the module doc for why neither
/// the painting nor the scaling is done here.
pub struct Placement {
    pub area: Rect,
    /// The identity to build a protocol under. Not the source picture's --
    /// see [`scaled_id`].
    pub id: ImageId,
    /// The source picture's own identity, so the app can tell whether the
    /// scaled copy it is holding was made from this picture.
    pub source: ImageId,
    /// `None` hands the source picture over untouched -- either it is
    /// already the right size, or it is being scaled *down*, which
    /// `Resize::Fit` does perfectly well by itself.
    pub scale: Option<Scaling>,
}

/// The pixels to make, and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaling {
    pub mode: Scale,
    pub pixels: (u32, u32),
    pub filter: FilterType,
}

/// What [`plan`] decided: the rectangle, the pre-scaling if any, and the
/// factor the meta row reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub rect: Rect,
    pub scale: Option<Scaling>,
    /// How much bigger than itself the picture ends up. `1.0` when it did
    /// not grow, including when it was scaled down to fit.
    pub factor: f32,
}

/// The identity a scaled copy is cached under.
///
/// Not the source picture's: the same picture at two scale modes, or at one
/// mode over two panel sizes, is two different sets of pixels, and a cache
/// keyed on the source alone would serve the first of them for the second.
/// The source is in the hash, so these are distinct between pictures too,
/// but forgetting the source's own id does not forget them -- the app
/// forgets each derived id it has used alongside it.
pub fn scaled_id(source: ImageId, scaling: &Scaling) -> ImageId {
    ImageId::of(&(
        source.0,
        scaling.mode.name(),
        scaling.pixels.0,
        scaling.pixels.1,
    ))
}

/// Which rectangle a picture goes in, and how big to make it first.
///
/// `panel` is the space the picture has, in cells, and `cell` is how many
/// pixels one of those cells is -- `None` where the terminal never said, in
/// which case there is no honest way to talk about pixels at all and all
/// three modes fall back to fitting the panel, the behaviour that predates
/// them.
///
/// Every mode leaves a picture already bigger than the panel alone: the
/// rectangle is the fitted one and the encoder scales it down, which is what
/// `Resize::Fit` is for.
pub fn plan(mode: Scale, img: (u32, u32), panel: Rect, cell: Option<(u16, u16)>) -> Plan {
    let (img_w, img_h) = img;
    let aspect = cell.map_or(2.0, |(w, h)| f32::from(h) / f32::from(w));
    let fitted = Plan {
        rect: fit_aspect(panel, img_w, img_h, aspect),
        scale: None,
        factor: 1.0,
    };
    let Some((cell_w, cell_h)) = cell else {
        return fitted;
    };
    if img_w == 0 || img_h == 0 || cell_w == 0 || cell_h == 0 {
        return fitted;
    }
    if panel.width == 0 || panel.height == 0 {
        return fitted;
    }

    // What the picture covers at a whole-number magnification, in cells, or
    // `None` if that does not fit the panel. The last cell is usually part
    // empty -- a picture is not a whole number of cells wide -- and rounding
    // down instead would crop it, which `Fit` would then make up for by
    // shrinking it: a `1x` that is not 1x.
    let cells = |k: u32| -> Option<(u16, u16)> {
        let w = u16::try_from(img_w.checked_mul(k)?.div_ceil(u32::from(cell_w))).ok()?;
        let h = u16::try_from(img_h.checked_mul(k)?.div_ceil(u32::from(cell_h))).ok()?;
        (w > 0 && h > 0 && w <= panel.width && h <= panel.height).then_some((w, h))
    };

    match mode {
        Scale::One => match cells(1) {
            Some((w, h)) => Plan {
                rect: centre(panel, w, h),
                scale: None,
                factor: 1.0,
            },
            // Bigger than the panel: there is no natural size to place.
            None => fitted,
        },
        Scale::Pixels => {
            // The largest whole number that still fits, walked upwards
            // rather than solved for: the ceiling in `cells` is not a smooth
            // function of `k`, and a closed form is off by one at exactly
            // the sizes this is for. Bounded by the panel, since a factor
            // can never exceed the number of cells across it.
            let mut best = None;
            for k in 1..=u32::from(panel.width.max(panel.height)) + 1 {
                match cells(k) {
                    Some(wh) => best = Some((k, wh)),
                    None => break,
                }
            }
            match best {
                // One is the natural size, and nothing has to be made.
                Some((1, (w, h))) => Plan {
                    rect: centre(panel, w, h),
                    scale: None,
                    factor: 1.0,
                },
                Some((k, (w, h))) => Plan {
                    rect: centre(panel, w, h),
                    scale: Some(Scaling {
                        mode,
                        pixels: (img_w * k, img_h * k),
                        filter: FilterType::Nearest,
                    }),
                    factor: k as f32,
                },
                None => fitted,
            }
        }
        Scale::Smooth => {
            let rect = fitted.rect;
            let room = (
                u32::from(rect.width) * u32::from(cell_w),
                u32::from(rect.height) * u32::from(cell_h),
            );
            // The rectangle already has the picture's own shape, so the two
            // ratios are within a rounding of each other; the smaller is
            // taken so the result cannot spill past the rectangle and be
            // shrunk straight back by the encoder.
            let factor = (room.0 as f32 / img_w as f32).min(room.1 as f32 / img_h as f32);
            if !factor.is_finite() || factor <= 1.0 {
                return fitted;
            }
            let pixels = (
                ((img_w as f32 * factor).round() as u32).clamp(1, room.0),
                ((img_h as f32 * factor).round() as u32).clamp(1, room.1),
            );
            Plan {
                rect,
                scale: Some(Scaling {
                    mode,
                    pixels,
                    filter: SMOOTH_FILTER,
                }),
                factor,
            }
        }
    }
}

/// The filter `smooth` grows with.
///
/// Catmull-Rom rather than Lanczos3. Both are sharp enough at these sizes,
/// and Lanczos3's negative lobes ring on a hard edge -- a preview panel is
/// as likely to be showing a screenshot of text, or a diagram, all hard
/// edges, as a photograph. A halo around every letter is the worse failure
/// of the two, and the one somebody would report as a bug.
const SMOOTH_FILTER: FilterType = FilterType::CatmullRom;

/// A `w` by `h` rectangle in the middle of `area`.
fn centre(area: Rect, w: u16, h: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w.min(area.width),
        height: h.min(area.height),
    }
}

/// `pixels ×3`, `smooth ×2.4`, or just `1x` -- what the meta row says about
/// the mode once it has been applied.
fn scale_label(mode: Scale, factor: f32) -> String {
    match mode {
        // The name is already the factor.
        Scale::One => mode.name().to_string(),
        // At or below one it did not grow, and `×1.0` would only invite the
        // question of where the tenth went.
        _ if factor <= 1.0 => format!("{} \u{d7}1", mode.name()),
        Scale::Pixels => format!("{} \u{d7}{}", mode.name(), factor.round() as u32),
        Scale::Smooth => format!("{} \u{d7}{factor:.1}", mode.name()),
    }
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
    /// How much a picture smaller than the panel is grown by. See [`plan`].
    pub scale: Scale,
}

pub fn render(
    area: Rect,
    buf: &mut Buffer,
    v: &mut View<'_>,
    bars: &mut Bars,
) -> Option<Placement> {
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
            render_text(
                area, body, buf, v.theme, preview, *bytes, v.name, v.scroll, bars,
            );
            None
        }
        Preview::Binary {
            mime, truncated, ..
        } => {
            render_binary(
                area, body, buf, v.theme, preview, mime, *truncated, v.scroll, bars,
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
    bars: &mut Bars,
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
    render_scrollbar(outer, body, buf, t, scroll, rows.len(), bars);
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
    bars: &mut Bars,
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
    render_scrollbar(outer, body, buf, t, scroll, rows.len(), bars);
}

/// The scroll position of a text or binary preview, on the panel's own right
/// border -- drawn over the rows the content actually occupies, below the
/// meta line, the same mark every other scrolling list in the column draws.
/// Recorded through `bars` rather than drawn directly, so a press or a drag
/// on this same track next frame has something to answer to.
fn render_scrollbar(
    outer: Rect,
    body: Rect,
    buf: &mut Buffer,
    t: &Theme,
    scroll: usize,
    len: usize,
    bars: &mut Bars,
) {
    let track = scrollbar::track(outer, body);
    bars.draw(Bar::Preview, track, buf, t, len as u32, scroll as u32);
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
    if area.height == 1 {
        let meta = format!(
            "{format} \u{b7} {width} \u{d7} {height} \u{b7} {}",
            scale_label(v.scale, 1.0)
        );
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

    let cell = v.graphics.as_deref().and_then(Graphics::cell_size);
    let p = plan(v.scale, (width, height), pic, cell);
    let meta = format!(
        "{format} \u{b7} {width} \u{d7} {height} \u{b7} {}",
        scale_label(v.scale, p.factor)
    );

    let usable = v
        .graphics
        .as_deref()
        .map(Graphics::pictures_available)
        .unwrap_or(false);

    let placement = if usable {
        // Left for the app's second pass -- see the module doc.
        let source = ImageId::of_arc(data);
        Some(Placement {
            area: p.rect,
            id: match &p.scale {
                Some(scaling) => scaled_id(source, scaling),
                None => source,
            },
            source,
            scale: p.scale,
        })
    } else if width == 0 || height == 0 {
        starkit::graphics::placeholder(p.rect, buf, Style::default().fg(rgb(v.theme.dim)));
        None
    } else {
        // No pre-scaling: half blocks sample the picture themselves, at
        // whatever size the rectangle is, so growing the pixels first would
        // only cost the time and change nothing on screen. The rectangle
        // still follows the mode, so `1x` and `pixels` are honest about size
        // wherever a cell has been measured.
        starkit::graphics::halfblocks(data, p.rect, buf);
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
    use proptest::prelude::*;
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
            scale: Scale::default(),
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
        render(area, &mut buf, &mut v, &mut Bars::new());
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

    // -- the three picture scales ------------------------------------------

    /// A cell that is not square, because a square one hides the one bug
    /// worth having a test for.
    const CELL: Option<(u16, u16)> = Some((10, 20));

    /// A panel 40 cells across is 400 pixels; 20 rows is 400 pixels too.
    fn panel() -> Rect {
        Rect::new(3, 5, 40, 20)
    }

    /// 1x places the picture at exactly the cells its own pixels cover, in
    /// the middle of the panel, and grows nothing.
    #[test]
    fn one_x_centres_a_small_picture_at_its_own_size() {
        // 100x60 pixels is 10 by 3 cells.
        let p = plan(Scale::One, (100, 60), panel(), CELL);
        assert_eq!(p.scale, None, "1x never makes a new picture");
        assert_eq!(p.factor, 1.0);
        assert_eq!((p.rect.width, p.rect.height), (10, 3));
        assert_eq!(p.rect.x, 3 + (40 - 10) / 2);
        assert_eq!(p.rect.y, 5 + (20 - 3) / 2);
    }

    /// A picture that is not a whole number of cells across still gets a
    /// whole cell for its last few pixels -- rounding down would crop it.
    #[test]
    fn one_x_rounds_a_part_cell_up_rather_than_cropping() {
        let p = plan(Scale::One, (101, 61), panel(), CELL);
        assert_eq!((p.rect.width, p.rect.height), (11, 4));
    }

    /// The largest whole number that still fits, and never zero.
    #[test]
    fn pixels_takes_the_largest_whole_factor_that_fits() {
        // 100x60 in a 400x400 panel: 4 across, 6 down, so 4.
        let p = plan(Scale::Pixels, (100, 60), panel(), CELL);
        assert_eq!(p.factor, 4.0);
        let s = p.scale.expect("a whole-number growth makes a picture");
        assert_eq!(s.pixels, (400, 240));
        assert_eq!(s.filter, FilterType::Nearest);
        assert_eq!((p.rect.width, p.rect.height), (40, 12));

        // One pixel more than a quarter of the panel and 4 no longer fits.
        let p = plan(Scale::Pixels, (101, 60), panel(), CELL);
        assert_eq!(p.factor, 3.0);
    }

    /// Growing by one is the natural size, and nothing has to be made for
    /// it -- the same rectangle 1x would have chosen.
    #[test]
    fn a_factor_of_one_scales_nothing() {
        let p = plan(Scale::Pixels, (300, 300), panel(), CELL);
        assert_eq!(p.factor, 1.0);
        assert_eq!(p.scale, None);
        assert_eq!(p.rect, plan(Scale::One, (300, 300), panel(), CELL).rect);
    }

    /// Smooth fills the fitted rectangle: one of the two dimensions is the
    /// rectangle's own, to the pixel.
    #[test]
    fn smooth_fills_the_fitted_rectangle() {
        let p = plan(Scale::Smooth, (100, 60), panel(), CELL);
        let s = p.scale.expect("smooth grows a small picture");
        assert_eq!(s.filter, SMOOTH_FILTER);
        let room = (u32::from(p.rect.width) * 10, u32::from(p.rect.height) * 20);
        assert!(
            s.pixels.0 == room.0 || s.pixels.1 == room.1,
            "{:?} fills neither dimension of {room:?}",
            s.pixels
        );
        assert!(
            s.pixels.0 <= room.0 && s.pixels.1 <= room.1,
            "{:?}",
            s.pixels
        );
        assert!(p.factor > 1.0, "{}", p.factor);
    }

    /// A picture bigger than the panel is the encoder's job in every mode:
    /// same rectangle, no pre-scaling, and the meta row says it did not
    /// grow.
    #[test]
    fn a_picture_bigger_than_the_panel_is_the_same_in_all_three_modes() {
        let big = (4000, 3000);
        let one = plan(Scale::One, big, panel(), CELL);
        for mode in [Scale::Pixels, Scale::Smooth] {
            let p = plan(mode, big, panel(), CELL);
            assert_eq!(p.rect, one.rect, "{mode} chose a different rectangle");
            assert_eq!(p.scale, None, "{mode} scaled a picture it need not");
            assert_eq!(p.factor, 1.0);
            assert!(scale_label(mode, p.factor).ends_with("\u{d7}1"), "{mode}");
        }
        assert!(one.rect.width <= panel().width && one.rect.height <= panel().height);
    }

    /// With no cell measured there is nothing to say how many pixels a
    /// rectangle holds, so every mode is the fit that predates them.
    #[test]
    fn an_unmeasured_cell_leaves_all_three_modes_fitting() {
        let want = fit_aspect(panel(), 100, 60, 2.0);
        for mode in [Scale::One, Scale::Pixels, Scale::Smooth] {
            let p = plan(mode, (100, 60), panel(), None);
            assert_eq!(p.rect, want, "{mode}");
            assert_eq!(p.scale, None, "{mode}");
            assert_eq!(p.factor, 1.0, "{mode}");
        }
    }

    #[test]
    fn the_meta_row_names_the_mode_and_what_it_did() {
        assert_eq!(scale_label(Scale::One, 1.0), "1x");
        assert_eq!(scale_label(Scale::Pixels, 3.0), "pixels \u{d7}3");
        assert_eq!(scale_label(Scale::Smooth, 2.4375), "smooth \u{d7}2.4");
        assert_eq!(scale_label(Scale::Smooth, 1.0), "smooth \u{d7}1");
        assert_eq!(scale_label(Scale::Pixels, 1.0), "pixels \u{d7}1");
    }

    /// The whole meta row, drawn, for each mode -- headless graphics has no
    /// protocol, so the picture is half blocks and the row is the only
    /// visible difference between the three.
    #[test]
    fn each_mode_draws_its_own_meta_row() {
        let t = theme("terminal");
        let data = Arc::new(RgbaImage::from_pixel(
            4,
            4,
            starkit::image::Rgba([200, 100, 50, 255]),
        ));
        for (mode, want) in [
            (Scale::One, "png \u{b7} 320 \u{d7} 200 \u{b7} 1x"),
            (
                Scale::Pixels,
                "png \u{b7} 320 \u{d7} 200 \u{b7} pixels \u{d7}1",
            ),
            (
                Scale::Smooth,
                "png \u{b7} 320 \u{d7} 200 \u{b7} smooth \u{d7}1",
            ),
        ] {
            let mut v = view(&t, None, Some("harbour.png"));
            v.scale = mode;
            let body = Rect::new(0, 0, 60, 6);
            let mut buf = Buffer::empty(body);
            render_image(body, &mut buf, &mut v, &data, 320, 200, "png");
            let row: String = (0..body.width)
                .map(|x| buf[(x, body.height - 1)].symbol().to_string())
                .collect();
            assert_eq!(row.trim(), want);
        }
    }

    proptest! {
        /// Foreign pixels, a panel of any shape and a cell of any size: the
        /// rectangle stays inside the panel, the pixels asked for are never
        /// nothing, and nothing panics or overflows on the way.
        #[test]
        fn no_size_makes_a_plan_that_escapes_its_panel(
            img_w in 0u32..9000,
            img_h in 0u32..9000,
            w in 0u16..200,
            h in 0u16..200,
            cell in proptest::option::of((0u16..64, 0u16..64)),
            mode in prop_oneof![Just(Scale::One), Just(Scale::Pixels), Just(Scale::Smooth)],
        ) {
            let panel = Rect::new(7, 11, w, h);
            let p = plan(mode, (img_w, img_h), panel, cell);
            prop_assert!(p.rect.x >= panel.x && p.rect.y >= panel.y, "{:?}", p.rect);
            prop_assert!(
                p.rect.right() <= panel.right() && p.rect.bottom() <= panel.bottom(),
                "{:?} escapes {panel:?}",
                p.rect
            );
            prop_assert!(p.factor >= 1.0 && p.factor.is_finite(), "{}", p.factor);
            if let Some(s) = p.scale {
                prop_assert!(s.pixels.0 > 0 && s.pixels.1 > 0, "{:?}", s.pixels);
                // Nothing is ever made larger than the rectangle it goes in:
                // the encoder would only shrink it back.
                let (cw, ch) = cell.expect("no cell means no scaling");
                prop_assert!(s.pixels.0 <= u32::from(p.rect.width) * u32::from(cw));
                prop_assert!(s.pixels.1 <= u32::from(p.rect.height) * u32::from(ch));
            }
            // The label is a string for every factor, decimal or not.
            prop_assert!(!scale_label(mode, p.factor).is_empty());
        }
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
