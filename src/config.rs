//! `config.toml`, and the template written beside it on a first run.
//!
//! Every field has a default, and a file that omits a table gets the whole
//! table's defaults -- the file is hand-edited, and a key nobody has typed
//! yet should cost a preference rather than a startup. Nothing here is a
//! secret, so this file is safe to copy between machines and safe to paste
//! into a bug report.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::fold;

/// The whole of `config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ui: Ui,
    pub ops: Ops,
    pub preview: Preview,
    pub open: Open,
}

/// How the column looks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    /// A theme id, or `"system"` to follow the desktop.
    pub theme: String,
    /// `auto`, `kitty`, `blocks` or `off`.
    pub graphics: String,
    /// Blank columns and rows kept around the whole layout, for terminals
    /// whose window has no padding of its own.
    pub padding_x: u16,
    pub padding_y: u16,
    pub show_hidden: bool,
    pub sort: fold::sort::SortKey,
    pub sort_reverse: bool,
    pub dirs_first: bool,
    /// How many rows a folded parent level may draw before the stack
    /// squeezes them into one crumb row.
    pub fold_rows: u16,
    /// How many rows the PREVIEW panel gets when it is open but not focused.
    pub preview_rows: u16,
    /// How many rows OPERATIONS grows to while focused, up to its own queue
    /// length.
    pub ops_rows: u16,
    /// A directory bigger than this is truncated rather than read whole.
    pub max_entries: usize,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            theme: "catppuccin-mocha".into(),
            graphics: "auto".into(),
            padding_x: 0,
            padding_y: 0,
            show_hidden: false,
            sort: fold::sort::SortKey::Name,
            sort_reverse: false,
            dirs_first: true,
            fold_rows: 6,
            preview_rows: 10,
            ops_rows: 6,
            max_entries: 50_000,
        }
    }
}

/// How operations run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ops {
    pub trash: fold::TrashMode,
    /// Ask before a delete that cannot go to a trash.
    pub confirm_delete: bool,
    pub conflicts: fold::ops::ConflictPolicy,
    /// Keep a copied or moved file's modification time rather than stamping
    /// it with the time of the copy.
    pub preserve_times: bool,
}

impl Default for Ops {
    fn default() -> Self {
        Self {
            trash: fold::TrashMode::Auto,
            confirm_delete: true,
            conflicts: fold::ops::ConflictPolicy::Ask,
            preserve_times: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preview {
    /// Prefer a separately installed compatible STAR/AMP for audio playback.
    pub audio_player: AudioPlayer,
    /// Embedded transport pictures where supported, or always text.
    pub audio_buttons: AudioButtons,
    /// The most a text preview reads, in bytes.
    pub max_bytes: u64,
    /// The most a text preview shows, in lines.
    pub max_lines: usize,
    /// A picture wider or taller than this, in pixels, is not decoded.
    pub max_image_dimension: u32,
    /// The most entries a directory preview's summary walk counts before it
    /// gives up and says so.
    pub dir_budget: usize,
    /// How a picture smaller than the panel is grown to fill it. `z` cycles
    /// through the three while it is running.
    pub image_scale: Scale,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            audio_player: AudioPlayer::default(),
            audio_buttons: AudioButtons::default(),
            max_bytes: 262_144,
            max_lines: 400,
            max_image_dimension: 4096,
            dir_budget: 20_000,
            image_scale: Scale::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioPlayer {
    #[default]
    Auto,
    Staramp,
    External,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioButtons {
    #[default]
    Auto,
    Text,
}

impl AudioButtons {
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Text => "text",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Text,
            Self::Text => Self::Auto,
        }
    }
}

impl AudioPlayer {
    pub fn embeds(self, custom_opener: &str) -> bool {
        self == Self::Staramp || (self == Self::Auto && custom_opener.trim().is_empty())
    }
}

/// How much bigger than itself a picture may be drawn.
///
/// The terminal is handed pixels and places them over cells, and
/// `Resize::Fit` -- what STAR/KIT encodes with -- never upsizes. So a picture
/// smaller than the panel sits in the middle of it at whatever size it
/// happens to be, unless something scales it first, and that is a question
/// of taste rather than of correctness: a screenshot wants its own pixels
/// back, a piece of pixel art wants them square and enormous, and a
/// photograph wants the panel filled and no jagged edges. None of the three
/// touches a picture already bigger than the panel, which is scaled down to
/// fit in every mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scale {
    /// Natural size, centred. Never grown.
    // `1x` is not an identifier, so the name and the setting differ and the
    // rename is what keeps `config.toml` readable.
    #[serde(rename = "1x")]
    #[default]
    One,
    /// Grown by whole numbers only, nearest-neighbour: every source pixel
    /// becomes an exact k by k block, and a hard edge stays hard.
    Pixels,
    /// Grown to fill the fitted rectangle at any factor, smoothly.
    Smooth,
}

impl Scale {
    /// The name in `config.toml`, and the one the status note says.
    pub fn name(self) -> &'static str {
        match self {
            Self::One => "1x",
            Self::Pixels => "pixels",
            Self::Smooth => "smooth",
        }
    }

    /// The next mode `z` moves to, wrapping.
    pub fn next(self) -> Self {
        match self {
            Self::One => Self::Pixels,
            Self::Pixels => Self::Smooth,
            Self::Smooth => Self::One,
        }
    }
}

impl std::fmt::Display for Scale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for Scale {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "1x" => Ok(Self::One),
            "pixels" => Ok(Self::Pixels),
            "smooth" => Ok(Self::Smooth),
            other => Err(format!("not a picture scale: {other}")),
        }
    }
}

/// What opens a file that is not opened by this program.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Open {
    /// argv, whitespace-split -- never a shell line. Empty is the desktop's
    /// own opener: `open` on macOS, `xdg-open` elsewhere.
    pub command: String,
}

impl Config {
    /// Read the file, or the defaults if there is not one.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Write a commented starting file, if there is not one already. A
    /// template rather than a serialised `Config`, which would be correct
    /// and teach nobody anything. Returns whether it created the file.
    pub fn write_template(path: &Path) -> Result<bool> {
        if path.exists() {
            return Ok(false);
        }
        starkit::fs::write_atomic(path, TEMPLATE.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(true)
    }

    /// What the file says, translated into the core's own settings. The two
    /// structs are separate on purpose -- nothing under `src/fold/` reads a
    /// config file -- and this is the one place a key added to `config.toml`
    /// and not carried across here is a key that silently does nothing.
    pub fn core(&self) -> fold::FoldConfig {
        fold::FoldConfig {
            list: fold::listing::ListConfig {
                max_entries: self.ui.max_entries,
                sort: fold::sort::SortOrder {
                    key: self.ui.sort,
                    reverse: self.ui.sort_reverse,
                    dirs_first: self.ui.dirs_first,
                },
                show_hidden: self.ui.show_hidden,
            },
            preview: fold::preview::PreviewConfig {
                max_bytes: self.preview.max_bytes,
                max_lines: self.preview.max_lines,
                max_image_dimension: self.preview.max_image_dimension,
                dir_budget: self.preview.dir_budget,
            },
            trash: self.ops.trash,
            conflicts: self.ops.conflicts,
            preserve_times: self.ops.preserve_times,
            open: fold::OpenConfig {
                // A simple whitespace split rather than a shell-quoting
                // parser: there is no `shell-words`-equivalent dependency
                // here, and `[open] command` is meant for a bare `argv0 --flag`
                // line, not for anything that would need quoting.
                command: self
                    .open
                    .command
                    .split_whitespace()
                    .map(str::to_string)
                    .collect(),
            },
        }
    }
}

const TEMPLATE: &str = r#"# STAR/FOLD configuration.
#
# Everything STAR/FOLD keeps lives under one directory -- this file, the
# session and the log. $STARFOLD_DIR relocates all of it.

[ui]
# "system" follows the desktop. Or name one of the built-in themes; `t` and
# `T` cycle through them while it is running.
theme = "catppuccin-mocha"
# How pictures are drawn in the preview: auto, kitty, blocks, or off.
graphics = "auto"
# Blank cells around the whole layout, for a terminal whose window has none.
padding_x = 0
padding_y = 0
show_hidden = false
# name, size, time, or ext.
sort = "name"
sort_reverse = false
dirs_first = true
# How many rows a folded parent level may draw before the stack squeezes
# them into one crumb row.
fold_rows = 6
# How many rows the preview panel gets while it is open but not focused.
preview_rows = 10
# How far operations grows while focused, up to its own queue length.
ops_rows = 6
# A directory bigger than this is truncated rather than read whole.
max_entries = 50000

[ops]
# auto uses the trash where one was found at startup and falls back to a
# permanent delete; always and never decide the question outright.
trash = "auto"
confirm_delete = true
# What a copy or a move does when the destination already has that name:
# ask, skip, overwrite, or rename (give the new one a new name).
conflicts = "ask"
preserve_times = true

[preview]
# auto uses compatible STAR/AMP when installed, unless [open] command is set.
# staramp explicitly prefers it; external always uses the existing opener.
audio_player = "auto"
# Embedded transport buttons: auto draws pictures where supported, text
# always uses ASCII. With the player focused, `o` toggles this preference.
audio_buttons = "auto"
# The most a text preview reads, in bytes, and shows, in lines.
max_bytes = 262144
max_lines = 400
# A picture wider or taller than this, in pixels, is not decoded.
max_image_dimension = 4096
# The most entries a directory preview's summary counts before giving up.
dir_budget = 20000
# How a picture smaller than the panel is grown: 1x (natural size, centred),
# pixels (whole-number steps, hard edges), or smooth. `z` cycles them.
image_scale = "1x"

[open]
# argv, whitespace-split, never a shell line. Empty is the desktop's own
# opener -- `open` on macOS, `xdg-open` elsewhere.
command = ""
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_button_preferences_round_trip() {
        assert_eq!(Config::default().preview.audio_buttons, AudioButtons::Auto);
        for mode in [AudioButtons::Auto, AudioButtons::Text] {
            let c: Config =
                toml::from_str(&format!("[preview]\naudio_buttons = {:?}", mode.name())).unwrap();
            assert_eq!(c.preview.audio_buttons, mode);
            assert_eq!(
                toml::from_str::<Config>(&toml::to_string(&c).unwrap()).unwrap(),
                c
            );
            assert_eq!(mode.next().next(), mode);
        }
        assert!(toml::from_str::<Config>("[preview]\naudio_buttons = 'unknown'").is_err());
    }

    #[test]
    fn audio_player_modes_and_custom_opener_policy() {
        for (name, mode) in [
            ("auto", AudioPlayer::Auto),
            ("staramp", AudioPlayer::Staramp),
            ("external", AudioPlayer::External),
        ] {
            let c: Config = toml::from_str(&format!("[preview]\naudio_player = {name:?}")).unwrap();
            assert_eq!(c.preview.audio_player, mode);
            assert_eq!(
                toml::from_str::<Config>(&toml::to_string(&c).unwrap()).unwrap(),
                c
            );
        }
        assert!(AudioPlayer::Auto.embeds(""));
        assert!(!AudioPlayer::Auto.embeds("mpv"));
        assert!(AudioPlayer::Staramp.embeds("mpv"));
        assert!(!AudioPlayer::External.embeds(""));
        assert!(toml::from_str::<Config>("[preview]\naudio_player = 'unknown'").is_err());
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn a_partial_table_keeps_the_rest_of_its_defaults() {
        let c: Config = toml::from_str("[ui]\npadding_x = 2\n").unwrap();
        assert_eq!(c.ui.padding_x, 2);
        assert_eq!(c.ui.theme, Config::default().ui.theme);
        assert_eq!(c.ops, Ops::default());
    }

    #[test]
    fn the_template_parses_as_the_defaults() {
        let parsed: Config = toml::from_str(TEMPLATE).expect("the template must parse");
        assert_eq!(parsed, Config::default());
    }

    /// The name is written into `config.toml` by `z` and read back on the
    /// next run, so a mode that cannot survive the round trip is a setting
    /// that silently resets.
    #[test]
    fn every_picture_scale_round_trips_through_its_name() {
        for m in [Scale::One, Scale::Pixels, Scale::Smooth] {
            assert_eq!(m.name().parse::<Scale>().unwrap(), m, "{m} did not return");
            assert_eq!(m.to_string(), m.name());
            let toml = format!("[preview]\nimage_scale = {:?}\n", m.name());
            let c: Config = toml::from_str(&toml).expect("the name must parse");
            assert_eq!(c.preview.image_scale, m);
        }
        assert!("2x".parse::<Scale>().is_err());
    }

    /// Three modes, and `z` reaches all three and comes back.
    #[test]
    fn the_picture_scales_cycle() {
        let mut m = Scale::default();
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(m);
            m = m.next();
        }
        assert_eq!(seen, vec![Scale::One, Scale::Pixels, Scale::Smooth]);
        assert_eq!(m, Scale::One, "the cycle must come back round");
    }

    #[test]
    fn an_unknown_table_is_ignored_rather_than_refused() {
        let text = "\
[ui]
padding_x = 2

[layout]
some_future_key = true
";
        let c: Config = toml::from_str(text).expect("an unknown table still parses");
        assert_eq!(c.ui.padding_x, 2);
    }

    #[test]
    fn the_template_is_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        assert!(Config::write_template(&path).unwrap());
        std::fs::write(&path, "[ui]\npadding_x = 7\n").unwrap();
        assert!(!Config::write_template(&path).unwrap());
        assert_eq!(Config::load(&path).unwrap().ui.padding_x, 7);
    }

    #[test]
    fn a_missing_file_loads_as_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config::load(&dir.path().join("nothing.toml")).unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn every_setting_the_core_reads_is_carried_across() {
        let cfg = Config {
            ui: Ui {
                max_entries: 12,
                sort: fold::sort::SortKey::Size,
                sort_reverse: true,
                dirs_first: false,
                show_hidden: true,
                ..Ui::default()
            },
            ops: Ops {
                trash: fold::TrashMode::Never,
                conflicts: fold::ops::ConflictPolicy::Overwrite,
                preserve_times: false,
                ..Ops::default()
            },
            preview: Preview {
                max_bytes: 99,
                ..Preview::default()
            },
            open: Open {
                command: "xdg-open --new-window".into(),
            },
        };
        let core = cfg.core();

        assert_eq!(core.list.max_entries, 12);
        assert_eq!(core.list.sort.key, fold::sort::SortKey::Size);
        assert!(core.list.sort.reverse);
        assert!(!core.list.sort.dirs_first);
        assert!(core.list.show_hidden);
        assert_eq!(core.trash, fold::TrashMode::Never);
        assert_eq!(core.conflicts, fold::ops::ConflictPolicy::Overwrite);
        assert!(!core.preserve_times);
        assert_eq!(core.preview.max_bytes, 99);
        assert_eq!(core.open.command, vec!["xdg-open", "--new-window"]);
    }

    #[test]
    fn the_defaults_agree_with_the_cores_defaults() {
        let core = Config::default().core();
        assert_eq!(core.list.max_entries, 50_000);
        assert_eq!(core.trash, fold::TrashMode::Auto);
        assert_eq!(core.conflicts, fold::ops::ConflictPolicy::Ask);
        assert!(core.preserve_times);
        assert!(core.open.command.is_empty());
    }
}
