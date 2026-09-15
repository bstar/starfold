//! The terminal.
//!
//! Everything under here reaches ratatui and crossterm through
//! `starkit::ratatui::…` and `starkit::crossterm::…` rather than depending on
//! either directly -- see the crate doc. `app`, `layout`, `theme`, `status`,
//! `overlays` and `fake` are later phases' files and are deliberately not
//! declared yet: a module list that only grows is a smaller diff to review
//! than one that is written whole and then pruned.

pub mod keymap;
pub mod panels;
pub mod theme;
