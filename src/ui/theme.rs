//! STAR/FOLD's theme: the shared one, plus the roles a file manager needs.
//!
//! The sixteen built-in theme files are STAR/KIT's and are shared with
//! STAR/AMP and STAR/CORD. None of them says anything about a directory
//! listing, and they should not have to: a theme is eight colours and a
//! base16 scheme, and everything else is derived from those. So `[fold]` is
//! derived here, out of the same palette the core resolved, and a file that
//! *does* state a `[fold]` table has the last word.
//!
//! Derivation rather than a table of literals is what keeps a theme honest,
//! as STAR/CORD's `[chat]` table puts it: one rule per role, run over sixteen
//! palettes and asserted legible by a test, is a few hundred colours that
//! are all correct.
//!
//! `// TODO(2a)`: this is the bootstrap shape -- the roles, the wrapper and
//! the `Resolve`. The derivation below is a first cut, and the legibility
//! test over every built-in is Phase 2a's.

use std::ops::Deref;

use serde::{Deserialize, Serialize};
use starkit::theme::color::Rgb;
use starkit::theme::{pick, Registry, Resolve, Theme as Core, ThemeFile};

/// The `[fold]` table, as a theme file may state it.
///
/// Every field optional: a theme states the two it cares about and lets the
/// rest fall out of its palette.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FoldColors {
    pub dir_fg: Option<Rgb>,
    pub symlink_fg: Option<Rgb>,
    pub exec_fg: Option<Rgb>,
    pub hidden_fg: Option<Rgb>,
    pub marked_fg: Option<Rgb>,
    pub marked_bg: Option<Rgb>,
    pub crumb_fg: Option<Rgb>,
    pub crumb_active_fg: Option<Rgb>,
    pub size_fg: Option<Rgb>,
    pub time_fg: Option<Rgb>,
    pub kind_fg: Option<Rgb>,
    pub progress_fg: Option<Rgb>,
    pub progress_bg: Option<Rgb>,
    pub conflict_fg: Option<Rgb>,
    pub error_fg: Option<Rgb>,
}

/// The same roles, resolved.
#[derive(Debug, Clone)]
pub struct Fold {
    /// A directory's name, and a symlink to one.
    pub dir_fg: Rgb,
    pub symlink_fg: Rgb,
    pub exec_fg: Rgb,
    /// A dotfile, when hidden files are shown at all.
    pub hidden_fg: Rgb,
    /// The mark glyph and the row of a marked entry.
    pub marked_fg: Rgb,
    pub marked_bg: Rgb,
    /// A folded level's line, and the active level's rule.
    pub crumb_fg: Rgb,
    pub crumb_active_fg: Rgb,
    /// The size, time and kind columns: metadata, quieter than the name.
    pub size_fg: Rgb,
    pub time_fg: Rgb,
    pub kind_fg: Rgb,
    /// The running operation's bar.
    pub progress_fg: Rgb,
    pub progress_bg: Rgb,
    /// A queued operation that is waiting on a conflict answer.
    pub conflict_fg: Rgb,
    /// A listing that could not be read, an operation that failed.
    pub error_fg: Rgb,
}

/// WCAG AA for normal text. Anything carrying words clears this against
/// whatever it is drawn on.
const TEXT_CONTRAST: f64 = 4.5;

/// WCAG AA for a graphical mark: the progress bar, a mark glyph.
const MARK_CONTRAST: f64 = 3.0;

/// How far a marked row's background is pulled from the panel toward the
/// accent. Low: the name still has to be read on it.
const TINT: f64 = 0.20;

/// What the file said, or what the palette implies held to a contrast floor.
///
/// The floor is only ever applied to a *derived* colour. A theme that names a
/// role names it.
fn stated_or(stated: Option<Rgb>, derived: Rgb, against: Rgb, target: f64) -> Rgb {
    stated.unwrap_or_else(|| derived.ensure_contrast(against, target))
}

impl Fold {
    fn derive(core: &Core, f: &FoldColors, b16: Option<&starkit::theme::schema::Base16>) -> Self {
        let bg = core.panel_bg;

        // Every role names the base16 slot it comes from. The spec's own
        // meanings: 08 red, 09 orange, 0A yellow, 0B green, 0C cyan, 0D blue,
        // 0E magenta. Directories are blue because every `ls --color` since
        // 1996 has drawn them blue, and a file manager that chose otherwise
        // would be arguing with the reader's reflexes.
        let dir_fg = stated_or(
            f.dir_fg,
            pick(None, b16.map(|b| b.base0D), core.accent),
            bg,
            TEXT_CONTRAST,
        );
        let marked_base = pick(f.marked_fg, b16.map(|b| b.base0A), core.warn);
        let marked_bg = stated_or(
            f.marked_bg,
            bg.mix(core.accent, TINT),
            core.fg,
            TEXT_CONTRAST,
        );
        let marked_fg = stated_or(f.marked_fg, marked_base, marked_bg, TEXT_CONTRAST);
        let progress_bg = stated_or(f.progress_bg, bg.mix(core.fg, 0.15), bg, 1.0);

        Self {
            dir_fg,
            symlink_fg: stated_or(
                f.symlink_fg,
                pick(None, b16.map(|b| b.base0C), core.accent),
                bg,
                TEXT_CONTRAST,
            ),
            exec_fg: stated_or(
                f.exec_fg,
                pick(None, b16.map(|b| b.base0B), core.ok),
                bg,
                TEXT_CONTRAST,
            ),
            hidden_fg: stated_or(f.hidden_fg, core.dim, bg, TEXT_CONTRAST),
            marked_fg,
            marked_bg,
            crumb_fg: stated_or(f.crumb_fg, core.dim, bg, TEXT_CONTRAST),
            crumb_active_fg: stated_or(f.crumb_active_fg, core.header_fg, bg, TEXT_CONTRAST),
            size_fg: stated_or(f.size_fg, core.row_meta_fg, bg, TEXT_CONTRAST),
            time_fg: stated_or(f.time_fg, core.row_meta_fg, bg, TEXT_CONTRAST),
            kind_fg: stated_or(f.kind_fg, core.dim, bg, TEXT_CONTRAST),
            progress_fg: stated_or(f.progress_fg, core.accent, progress_bg, MARK_CONTRAST),
            progress_bg,
            conflict_fg: stated_or(
                f.conflict_fg,
                pick(None, b16.map(|b| b.base09), core.warn),
                bg,
                TEXT_CONTRAST,
            ),
            error_fg: stated_or(
                f.error_fg,
                pick(None, b16.map(|b| b.base08), core.error),
                bg,
                TEXT_CONTRAST,
            ),
        }
    }
}

/// The theme STAR/FOLD draws with: the shared one, plus `[fold]`.
///
/// `Deref` rather than a hundred delegating accessors, so `theme.accent` and
/// `theme.fold.dir_fg` read the same way and every STAR/KIT widget takes
/// `&*theme`.
#[derive(Debug, Clone)]
pub struct Theme {
    core: Core,
    pub fold: Fold,
}

impl Deref for Theme {
    type Target = Core;

    fn deref(&self) -> &Core {
        &self.core
    }
}

impl Resolve for Theme {
    fn resolve(file: &ThemeFile) -> Self {
        let core = Core::resolve(file);
        // A malformed `[fold]` table costs the table, not the theme.
        let stated: FoldColors = file.table("fold").unwrap_or_else(|e| {
            tracing::warn!("{}: the [fold] table was ignored: {e}", core.id);
            FoldColors::default()
        });
        let fold = Fold::derive(&core, &stated, file.base16.as_ref());
        Self { core, fold }
    }

    fn core(&self) -> &Core {
        &self.core
    }
}

/// Where the themes come from: the same directory as the rest of STAR/FOLD's
/// files, spelled with STAR/KIT's `Paths` because that is what [`Registry`]
/// takes.
pub const THEME_PATHS: starkit::paths::Paths = crate::paths::PATHS;

pub fn registry() -> Registry<Theme> {
    Registry::new(THEME_PATHS)
}

/// A resolved built-in, for the tests in every other module that need
/// something to draw with.
#[cfg(test)]
pub mod tests_support {
    use super::{Resolve as _, Theme, ThemeFile};

    pub fn theme(id: &str) -> Theme {
        let builtin = starkit::theme::builtin::BUILTINS
            .iter()
            .find(|b| b.id == id)
            .unwrap_or_else(|| panic!("no built-in theme {id}"));
        Theme::resolve(&ThemeFile::parse(builtin.toml).expect("a built-in parses"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_resolves_with_a_fold_table() {
        for b in starkit::theme::builtin::BUILTINS {
            let t = Theme::resolve(&ThemeFile::parse(b.toml).unwrap());
            assert_ne!(t.fold.dir_fg, t.panel_bg, "{}: directories vanish", b.id);
        }
    }
}
