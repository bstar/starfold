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
use starkit::ratatui::style::Style;

use super::keymap::Module;

// The border, corners, titles and header row are STAR/KIT's now -- see
// `starkit::chrome::frame` -- and so are the small text helpers every panel
// used to keep its own copy of. Re-exported under their old names so the
// rest of this module's callers keep the imports they already have.
pub use starkit::chrome::{empty, rgb};
pub use starkit::text::fit;
pub use starkit::wrap::width_of;

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
    View,
    Places,
    Bookmark,
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
            Word::View => "view".into(),
            Word::Places => "places".into(),
            Word::Bookmark => "bookmark".into(),
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
        ModuleId::Stack => vec![
            Word::Back,
            Word::Hidden,
            Word::Sort,
            Word::Filter,
            Word::Bookmark,
            Word::Places,
            Word::View,
        ],
        ModuleId::Preview => vec![Word::Close],
        ModuleId::Operations => vec![Word::Run, Word::Clear],
    }
}

/// The application's name as the top of the window says it, letter-spaced,
/// exactly as STAR/AMP and STAR/CORD say their own.
pub const HEADING: &str = "S T A R / F O L D";

/// Cut a string to `width` columns by taking out its middle, so both ends
/// survive: `/private/tmp/…/scratchpad/tree` keeps the part that says where
/// and the part that says what.
pub fn elide_middle(text: &str, width: u16) -> String {
    let w = width_of(text);
    if w <= width {
        return text.to_string();
    }
    if width < 4 {
        return fit(text, width);
    }
    let keep = usize::from(width - 1);
    let head = keep / 2;
    let tail = keep - head;
    let clusters: Vec<&str> = starkit::wrap::clusters(text).map(|(_, c)| c).collect();
    let mut out = String::new();
    let mut used = 0usize;
    for c in &clusters {
        let cw = usize::from(width_of(c));
        if used + cw > head {
            break;
        }
        out.push_str(c);
        used += cw;
    }
    out.push('\u{2026}');
    let mut back = Vec::new();
    let mut used = 0usize;
    for c in clusters.iter().rev() {
        let cw = usize::from(width_of(c));
        if used + cw > tail {
            break;
        }
        back.push(*c);
        used += cw;
    }
    for c in back.iter().rev() {
        out.push_str(c);
    }
    out
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
                        matches!(
                            w,
                            Word::Back
                                | Word::Hidden
                                | Word::Sort
                                | Word::Filter
                                | Word::View
                                | Word::Places
                                | Word::Bookmark
                        )
                    }
                    ModuleId::Preview => matches!(w, Word::Close),
                    ModuleId::Operations => matches!(w, Word::Run | Word::Clear),
                };
                assert!(allowed, "{m:?} offers {w:?}");
            }
        }
    }
}
