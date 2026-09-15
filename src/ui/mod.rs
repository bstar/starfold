//! The terminal.
//!
//! Everything under here reaches ratatui and crossterm through
//! `starkit::ratatui::…` and `starkit::crossterm::…` rather than depending on
//! either directly -- see the crate doc. `app` is the last module to land
//! and is declared when it does.

pub mod fake;
pub mod keymap;
pub mod layout;
pub mod overlays;
pub mod panels;
pub mod status;
pub mod theme;
