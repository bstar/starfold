//! Everything that knows what a directory, a selection and a queued
//! operation are, and nothing that knows a terminal exists.
//!
//! **Nothing under `src/fold/` may reach for `ratatui`, `crossterm` or the UI
//! module.** The test at the bottom of this file greps the module's own
//! sources and fails on the line that broke it. That is the boundary
//! `starfold list` is built on: a headless listing has to run with no TTY
//! attached, and a change to how a row is drawn must not be able to break how
//! one is read.
//!
//! The shape:
//!
//! - [`entry`] and [`listing`] read a directory; [`sort`] orders it,
//!   [`filter`] narrows it, [`format`] spells its sizes and times.
//! - [`stack`] and [`tab`] hold the levels a fold has drilled through.
//! - [`selection`] is what is marked, wherever the fold has since moved.
//! - [`ops`] is the transactional queue; [`preview`] and [`summary`] are what
//!   the PREVIEW panel shows.
//! - [`state`] is the truth, and `state::apply` is the only thing that writes
//!   to it.
//! - [`worker`] owns the two threads; [`handle`] is the contract between them
//!   and the terminal.

pub mod archive;
pub mod create;
pub mod file_type;
mod process;

pub mod entry;
pub mod filter;
pub mod format;
pub mod handle;
pub mod listing;
pub mod open;
pub mod ops;
pub mod places;
pub mod preview;
pub mod selection;
pub mod sort;
pub mod stack;
pub mod state;
pub mod summary;
pub mod tab;
#[cfg(test)]
pub mod testing;
pub mod watch;
pub mod worker;

// Named here so the UI imports from `fold` rather than from `fold::handle`
// and `fold::state`; neither has a consumer until Phase 3a's `ui::app`.
#[allow(unused_imports)]
pub use handle::{Command, Event, Handle, HandleParts, Note, NoteLevel};
#[allow(unused_imports)]
pub use state::State;

/// How aggressively a delete reaches for the trash instead of removing a file
/// for good.
///
/// `Auto` is the default and the one most people want: use the trash where
/// `trash_available` found one at startup, and fall back to a permanent
/// delete -- with the confirmation `[ops] confirm_delete` controls -- where
/// there is none. `Always` and `Never` exist for a person who has decided the
/// question themselves and does not want the answer to depend on which
/// machine this is running on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrashMode {
    #[default]
    Auto,
    Always,
    Never,
}

/// argv for opening a file with something other than this program. Never a
/// shell line -- a file called `-x` or one with a space in its name is a
/// single argument, not a chance to inject one.
///
/// Defined here rather than in `open.rs`, which is Phase 1d's file and does
/// not exist yet: [`FoldConfig`] has to name a type for its `open` field
/// before that module is written, and a config-shaped struct with nowhere
/// else to live belongs beside the config it fills rather than forward-
/// declared in the file that will eventually use it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenConfig {
    pub command: Vec<String>,
}

/// Everything the terminal hands the core once, at startup.
///
/// Built by `config::Config::core`, which is the one place a key in
/// `config.toml` becomes a setting here -- nothing under `src/fold/` reads a
/// config file itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FoldConfig {
    pub list: listing::ListConfig,
    pub preview: preview::PreviewConfig,
    pub trash: TrashMode,
    pub conflicts: ops::ConflictPolicy,
    pub preserve_times: bool,
    pub open: OpenConfig,
}

#[cfg(test)]
mod tests {
    /// The rule this module exists to keep. See `discord/mod.rs` in STAR/CORD,
    /// which this is copied from: a grep rather than a crate boundary,
    /// because `starfold` is a single binary crate and there is no `fold`
    /// crate for Cargo to keep `ratatui` out of.
    #[test]
    fn nothing_in_the_core_knows_about_the_terminal() {
        const FORBIDDEN: &[&str] = &[
            "use ratatui",   // NO-TERMINAL-HERE
            "ratatui::",     // NO-TERMINAL-HERE
            "use crossterm", // NO-TERMINAL-HERE
            "crossterm::",   // NO-TERMINAL-HERE
            "crate::ui",     // NO-TERMINAL-HERE
        ];

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("fold");
        let mut offences = Vec::new();
        let mut files = 0usize;

        walk(&root, &mut |path| {
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                return;
            }
            files += 1;
            let Ok(text) = std::fs::read_to_string(path) else {
                return;
            };
            for (number, line) in text.lines().enumerate() {
                if line.contains("NO-TERMINAL-HERE") {
                    continue;
                }
                for needle in FORBIDDEN {
                    if line.contains(needle) {
                        offences.push(format!(
                            "{}:{}: {}",
                            path.display(),
                            number + 1,
                            line.trim()
                        ));
                    }
                }
            }
        });

        assert!(
            files > 10,
            "only {files} files were scanned; the walk is wrong"
        );
        assert!(
            offences.is_empty(),
            "the fold core reached for the terminal:\n{}",
            offences.join("\n")
        );
    }

    fn walk(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path)) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, f);
            } else {
                f(&path);
            }
        }
    }
}
