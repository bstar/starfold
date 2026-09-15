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
        // A marked row's tint, checked against the panel rather than against
        // `fg` -- see STAR/CORD's `spoiler_bg`, which this is copied from.
        // The tint carries no letters of its own (the marked glyph does, and
        // is checked against *this* colour below), so what has to hold is
        // that a marked row can be told apart from an unmarked one, not that
        // some particular foreground reads on it -- a weak accent, as
        // winamp-classic's is, would otherwise leave a 20% mix a hair from
        // invisible.
        let marked_bg = stated_or(f.marked_bg, bg.mix(core.accent, TINT), bg, MARK_CONTRAST);
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

    /// The roles that carry words, and what each is drawn on. The legibility
    /// test walks this; naming it here is what stops a new role being added
    /// without one.
    #[cfg(test)]
    fn text_roles(&self, panel_bg: Rgb) -> Vec<(&'static str, Rgb, Rgb)> {
        vec![
            ("dir_fg", self.dir_fg, panel_bg),
            ("symlink_fg", self.symlink_fg, panel_bg),
            ("exec_fg", self.exec_fg, panel_bg),
            ("hidden_fg", self.hidden_fg, panel_bg),
            ("marked_fg", self.marked_fg, self.marked_bg),
            ("crumb_fg", self.crumb_fg, panel_bg),
            ("crumb_active_fg", self.crumb_active_fg, panel_bg),
            ("size_fg", self.size_fg, panel_bg),
            ("time_fg", self.time_fg, panel_bg),
            ("kind_fg", self.kind_fg, panel_bg),
            ("conflict_fg", self.conflict_fg, panel_bg),
            ("error_fg", self.error_fg, panel_bg),
        ]
    }

    /// Roles that carry no letters: the progress bar's fill against its own
    /// track, and the marked row's tint against the plain panel, which has to
    /// be told apart from an unmarked row at a glance.
    #[cfg(test)]
    fn mark_roles(&self, panel_bg: Rgb) -> Vec<(&'static str, Rgb, Rgb)> {
        vec![
            ("progress_fg", self.progress_fg, self.progress_bg),
            ("marked_bg", self.marked_bg, panel_bg),
        ]
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
    use super::tests_support::theme;
    use super::*;
    use starkit::theme::builtin::BUILTINS;

    #[test]
    fn every_builtin_resolves_with_a_fold_table() {
        for b in BUILTINS {
            let t = Theme::resolve(&ThemeFile::parse(b.toml).unwrap());
            assert_ne!(t.fold.dir_fg, t.panel_bg, "{}: directories vanish", b.id);
        }
    }

    /// The test the whole derivation exists to pass.
    ///
    /// Sixteen palettes, fourteen roles, and nobody looking at any of them.
    /// A rule that produces an unreadable colour on one scheme in sixteen is
    /// the normal outcome of writing rules for colours, and this is what
    /// catches it. A failure here is fixed in [`Fold::derive`], never by
    /// special-casing the theme that tripped it.
    #[test]
    fn every_builtin_fold_role_is_legible() {
        for b in BUILTINS {
            assert_legible(b.id, &theme(b.id));
        }

        // And the desktop's own palette, where there is one -- the one theme
        // nobody here chose, and exactly the case a rule written against
        // sixteen known palettes can fail on. Skipped rather than faked where
        // no desktop theme is set, because a synthesised one would be a
        // seventeenth builtin with a misleading name.
        if let Some((file, _)) = starkit::theme::system::theme() {
            assert_legible("system", &Theme::resolve(&file));
        }
    }

    fn assert_legible(id: &str, t: &Theme) {
        for (role, fg, bg) in t.fold.text_roles(t.panel_bg) {
            let c = bg.contrast(fg);
            assert!(
                c >= TEXT_CONTRAST,
                "{id}: {role} is {c:.2}:1 against its background"
            );
        }
        for (role, fg, bg) in t.fold.mark_roles(t.panel_bg) {
            let c = bg.contrast(fg);
            assert!(
                c >= MARK_CONTRAST,
                "{id}: {role} is {c:.2}:1 against its background"
            );
        }
    }

    /// A file that states a role gets that role, unchanged, whatever the
    /// derivation would have produced. Themes are allowed to be exact.
    #[test]
    fn a_stated_role_wins() {
        let f = ThemeFile::parse(
            r##"
            [meta]
            name = "Stated"
            variant = "dark"
            [app]
            bg = "#000000"
            fg = "#ffffff"
            [fold]
            dir_fg = "#ff0000"
            "##,
        )
        .unwrap();
        let t = Theme::resolve(&f);
        assert_eq!(t.fold.dir_fg, Rgb::new(0xff, 0, 0));
    }

    /// A `[fold]` table that is not a `[fold]` table costs the table and
    /// nothing else. The core tables still fail loudly; this one is
    /// decoration over a working palette.
    #[test]
    fn a_malformed_fold_table_does_not_lose_the_theme() {
        let f = ThemeFile::parse(
            r##"
            [meta]
            name = "Broken"
            variant = "dark"
            [app]
            bg = "#101010"
            fg = "#e0e0e0"
            [fold]
            dir_fg = "not a colour"
            "##,
        )
        .unwrap();
        let t = Theme::resolve(&f);
        assert_eq!(t.bg, Rgb::new(0x10, 0x10, 0x10));
        assert!(t.panel_bg.contrast(t.fold.dir_fg) >= TEXT_CONTRAST);
    }

    /// `[fold]` is TOML written by a person, so every field it can state has
    /// to survive being read back exactly.
    #[test]
    fn the_fold_table_round_trips_through_serde() {
        let stated = FoldColors {
            dir_fg: Some(Rgb::new(0x11, 0x22, 0x33)),
            symlink_fg: Some(Rgb::new(0x44, 0x55, 0x66)),
            exec_fg: None,
            hidden_fg: Some(Rgb::new(0x77, 0x88, 0x99)),
            marked_fg: None,
            marked_bg: Some(Rgb::new(0xaa, 0xbb, 0xcc)),
            crumb_fg: None,
            crumb_active_fg: None,
            size_fg: Some(Rgb::new(0x01, 0x02, 0x03)),
            time_fg: None,
            kind_fg: None,
            progress_fg: Some(Rgb::new(0x0a, 0x0b, 0x0c)),
            progress_bg: None,
            conflict_fg: Some(Rgb::new(0x1a, 0x2b, 0x3c)),
            error_fg: Some(Rgb::new(0xff, 0x00, 0xff)),
        };
        let text = toml::to_string(&stated).expect("a fold table serialises");
        let back: FoldColors = toml::from_str(&text).expect("it parses back");
        assert_eq!(back.dir_fg, stated.dir_fg);
        assert_eq!(back.symlink_fg, stated.symlink_fg);
        assert_eq!(back.exec_fg, stated.exec_fg);
        assert_eq!(back.hidden_fg, stated.hidden_fg);
        assert_eq!(back.marked_fg, stated.marked_fg);
        assert_eq!(back.marked_bg, stated.marked_bg);
        assert_eq!(back.crumb_fg, stated.crumb_fg);
        assert_eq!(back.crumb_active_fg, stated.crumb_active_fg);
        assert_eq!(back.size_fg, stated.size_fg);
        assert_eq!(back.time_fg, stated.time_fg);
        assert_eq!(back.kind_fg, stated.kind_fg);
        assert_eq!(back.progress_fg, stated.progress_fg);
        assert_eq!(back.progress_bg, stated.progress_bg);
        assert_eq!(back.conflict_fg, stated.conflict_fg);
        assert_eq!(back.error_fg, stated.error_fg);

        // And through a whole theme file, by round-tripping the resolved
        // table verbatim: what a person wrote in `[fold]` is what they get
        // back, not a derivation over it.
        let file = ThemeFile::parse(
            r##"
            [meta]
            name = "Round Trip"
            variant = "dark"
            [app]
            bg = "#101010"
            fg = "#e0e0e0"
            [fold]
            dir_fg = "#112233"
            "##,
        )
        .unwrap();
        let t = Theme::resolve(&file);
        assert_eq!(t.fold.dir_fg, Rgb::new(0x11, 0x22, 0x33));
    }
}
