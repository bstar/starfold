//! The modules of the column, and the words on their headers.
//!
//! A module is a pure widget over a small view struct, copied from STAR/
//! CORD's `ui/panels/mod.rs` with three modules instead of five and
//! `is_list` renamed to [`ModuleId::folds`] -- every module here folds when
//! it is not the one in focus (STAR/CORD's stack panels are the only ones
//! that do; here all three are).
//!
//! `pub mod stack; pub mod preview; pub mod operations;` are Phase 2c's files
//! and are deliberately not declared yet.

pub mod operations;
pub mod preview;
pub mod stack;

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use starkit::chrome::header;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Modifier, Style};
use starkit::ratatui::text::{Line, Span};
use starkit::ratatui::widgets::{Block, BorderType, Borders, Widget};

use super::keymap::Module;

// The app's own `Theme` does not exist until Phase 2a resolves `[fold]`
// roles out of a theme file on top of this one, so every call in this file
// that would take it takes `starkit::theme::Theme` instead.
// TODO(2a): Phase 2a's `Theme` derefs to this one, so the call sites below
// are unchanged once `crate::ui::theme::Theme` replaces it here.

/// The three modules of the column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleId {
    Stack,
    Preview,
    Operations,
}

/// The column, top to bottom -- draw order, tab order and the order a fold is
/// drilled through, deliberately the same order.
pub const COLUMN: [ModuleId; 3] = [ModuleId::Stack, ModuleId::Preview, ModuleId::Operations];

impl ModuleId {
    pub fn index(self) -> usize {
        match self {
            ModuleId::Stack => 0,
            ModuleId::Preview => 1,
            ModuleId::Operations => 2,
        }
    }

    /// Whether this module folds to one row when it is not the one in focus.
    /// Unlike STAR/CORD, where only the list panels fold, every module here
    /// does: the stack never folds -- it is always the thing being drilled
    /// through -- and preview and operations both collapse to make room for
    /// it.
    pub fn folds(self) -> bool {
        matches!(self, ModuleId::Preview | ModuleId::Operations)
    }

    pub fn title(self) -> &'static str {
        match self {
            ModuleId::Stack => "stack",
            ModuleId::Preview => "preview",
            ModuleId::Operations => "operations",
        }
    }

    pub fn module(self) -> Module {
        match self {
            ModuleId::Stack => Module::Stack,
            ModuleId::Preview => Module::Preview,
            ModuleId::Operations => Module::Operations,
        }
    }
}

/// A word on a module's header row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Word {
    /// `‹`: back one level, on the STACK module -- where a browser keeps it.
    Back,
    Hidden,
    Sort,
    Filter,
    Run,
    Clear,
    Close,
}

impl header::Word for Word {
    fn word(self) -> Cow<'static, str> {
        match self {
            Word::Back => "\u{2039}".into(),
            Word::Hidden => "hidden".into(),
            Word::Sort => "sort".into(),
            Word::Filter => "filter".into(),
            Word::Run => "run".into(),
            Word::Clear => "clear".into(),
            Word::Close => "close".into(),
        }
    }
}

/// What each module offers, right-aligned, dropped from the left as it
/// narrows.
pub fn words(module: ModuleId) -> Vec<Word> {
    match module {
        ModuleId::Stack => vec![Word::Back, Word::Hidden, Word::Sort, Word::Filter],
        ModuleId::Preview => vec![Word::Close],
        ModuleId::Operations => vec![Word::Run, Word::Clear],
    }
}

/// The application's name as the top of the window says it, letter-spaced,
/// exactly as STAR/AMP and STAR/CORD say their own.
pub const HEADING: &str = "S T A R / F O L D";

/// Everything a module needs that is not its own contents.
pub struct Frame<'a> {
    pub theme: &'a starkit::theme::Theme,
    pub focused: bool,
    pub name: &'a str,
    pub detail: Option<&'a str>,
    pub heading: bool,
    pub words: &'a [Word],
}

/// Draw a module's border, corners, title and header row, and hand back what
/// is left for its contents. See STAR/CORD's `panels::frame`, which this is
/// copied from, for why the corners are drawn after the title rather than
/// before it.
pub fn frame(area: Rect, buf: &mut Buffer, f: &Frame<'_>) -> Rect {
    let t = f.theme;
    let border = if f.focused {
        t.border_focused
    } else {
        t.border
    };

    let (title, style) = if f.heading {
        (
            format!("{}{HEADING} ", starkit::chrome::frame::TITLE_LEAD),
            Style::default()
                .fg(rgb(t.titlebar_active_fg))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        let name = f.name.to_uppercase();
        let text = match f.detail {
            Some(detail) => format!(
                "{}{name} \u{2014} {detail} ",
                starkit::chrome::frame::TITLE_LEAD
            ),
            None => format!("{}{name} ", starkit::chrome::frame::TITLE_LEAD),
        };
        (text, Style::default().fg(rgb(t.header_fg)))
    };

    let room = area.width.saturating_sub(2);
    let left = if width_of(&title) <= room {
        width_of(&title)
    } else {
        0
    };
    let right = f
        .heading
        .then(|| format!(" {}{}", f.name, starkit::chrome::frame::TITLE_TRAIL))
        .filter(|right| left + 1 + width_of(right) <= room);

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(rgb(border)))
        .style(Style::default().bg(rgb(t.panel_bg)));
    if left > 0 {
        block = block.title(Span::styled(title, style));
    }
    if let Some(right) = right {
        block = block.title_top(
            Line::from(Span::styled(right, Style::default().fg(rgb(t.dim)))).right_aligned(),
        );
    }
    block.render(area, buf);

    if area.width >= 2 && area.height >= 2 {
        starkit::chrome::frame::render_corners(area, buf, t, f.focused);
    }

    header::render(area, f.words, buf, t);
    header::body(area)
}

/// A ratatui colour from a theme one.
pub fn rgb(c: starkit::theme::color::Rgb) -> starkit::ratatui::style::Color {
    starkit::ratatui::style::Color::Rgb(c.r, c.g, c.b)
}

/// Columns a string takes on screen.
pub fn width_of(text: &str) -> u16 {
    starkit::wrap::width_of(text)
}

/// Cut and pad a row to exactly `width` columns, by display width rather than
/// by character count -- see STAR/CORD's `panels::fit`, which this is copied
/// from, for why a file name cannot be formatted with `{:width$}`.
pub fn fit(text: &str, width: u16) -> String {
    let mut out = String::with_capacity(usize::from(width) + 4);
    let mut used = 0u16;
    for (_, cluster) in starkit::wrap::clusters(text) {
        let w = width_of(cluster);
        if used + w > width {
            break;
        }
        out.push_str(cluster);
        used += w;
    }
    for _ in used..width {
        out.push(' ');
    }
    out
}

/// One dim line in the middle of an empty module.
pub fn empty(area: Rect, buf: &mut Buffer, theme: &starkit::theme::Theme, text: &str) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let y = area.y + area.height / 2;
    let text = fit(text, area.width);
    let text = text.trim_end();
    let x = area.x + area.width.saturating_sub(width_of(text)) / 2;
    buf.set_string(x, y, text, Style::default().fg(rgb(theme.empty_fg)));
}

/// The one line a folded module draws: what is currently open in it.
pub fn summary_row(body: Rect, buf: &mut Buffer, text: &str, style: Style) {
    if body.height == 0 || body.width == 0 {
        return;
    }
    buf.set_string(body.x, body.y, fit(text, body.width), style);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A built-in theme, resolved through the core, for the tests in this
    /// module. `starkit::chrome::test_theme` is `pub(crate)` to that crate,
    /// so this reaches the same built-ins the way any application does: the
    /// public `Registry`/`ThemeFile` path.
    fn test_theme(id: &str) -> starkit::theme::Theme {
        starkit::theme::BUILTINS
            .iter()
            .find(|b| b.id == id)
            .map(|b| {
                starkit::theme::Theme::resolve(&starkit::theme::ThemeFile::parse(b.toml).unwrap())
            })
            .unwrap_or_else(|| panic!("no built-in {id}"))
    }

    #[test]
    fn every_module_is_in_the_column_once() {
        for m in [ModuleId::Stack, ModuleId::Preview, ModuleId::Operations] {
            assert_eq!(
                COLUMN.iter().filter(|&&q| q == m).count(),
                1,
                "{m:?} is not in the column exactly once"
            );
            assert_eq!(COLUMN[m.index()], m, "{m:?} indexes somebody else's rect");
        }
    }

    #[test]
    fn the_stack_never_folds_and_the_other_two_do() {
        assert!(!ModuleId::Stack.folds());
        assert!(ModuleId::Preview.folds());
        assert!(ModuleId::Operations.folds());
    }

    #[test]
    fn a_module_serialises_as_its_lowercase_name() {
        let json = serde_json_like(ModuleId::Operations);
        assert_eq!(json, "operations");
    }

    fn serde_json_like(id: ModuleId) -> String {
        // No serde_json dependency here; toml round-trips the same derive.
        #[derive(Serialize)]
        struct Wrap {
            id: ModuleId,
        }
        let text = toml::to_string(&Wrap { id }).unwrap();
        text.trim()
            .trim_start_matches("id = \"")
            .trim_end_matches('"')
            .to_string()
    }

    #[test]
    fn every_module_maps_to_its_own_key_module() {
        let mut seen = Vec::new();
        for m in COLUMN {
            let k = m.module();
            assert!(!seen.contains(&k), "{k:?} is claimed by two modules");
            seen.push(k);
        }
    }

    #[test]
    fn the_frame_is_drawn_in_double_lines() {
        let theme = test_theme("cosmic");
        let area = Rect::new(0, 0, 12, 5);
        let mut buf = Buffer::empty(area);
        frame(
            area,
            &mut buf,
            &Frame {
                theme: &theme,
                focused: false,
                name: "",
                detail: None,
                heading: false,
                words: &[],
            },
        );
        let at = |x: u16, y: u16| buf[(x, y)].symbol().to_string();
        assert_eq!(
            [at(0, 0), at(11, 0), at(11, 4), at(0, 4)],
            ["\u{2554}", "\u{2557}", "\u{255d}", "\u{255a}"],
            "the corners are not the double-line ones"
        );
        assert_eq!(at(6, 0), "\u{2550}", "the top edge is not double");
        assert_eq!(at(0, 2), "\u{2551}", "the left edge is not double");
    }

    #[test]
    fn a_summary_row_is_one_fitted_line() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        summary_row(area, &mut buf, "a rather long summary", Style::default());
        let drawn: String = (0..10).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert_eq!(drawn, "a rather l");
    }

    #[test]
    fn no_module_offers_a_word_it_cannot_honour() {
        for m in COLUMN {
            for w in words(m) {
                let allowed = match m {
                    ModuleId::Stack => {
                        matches!(w, Word::Back | Word::Hidden | Word::Sort | Word::Filter)
                    }
                    ModuleId::Preview => matches!(w, Word::Close),
                    ModuleId::Operations => matches!(w, Word::Run | Word::Clear),
                };
                assert!(allowed, "{m:?} offers {w:?}");
            }
        }
    }
}
