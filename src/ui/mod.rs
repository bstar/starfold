//! The terminal.
//!
//! Everything under here reaches ratatui and crossterm through
//! `starkit::ratatui::…` and `starkit::crossterm::…` rather than depending on
//! either directly -- see the crate doc. `app` is the last module to land
//! and is declared when it does.

pub mod app;
mod audio_graphics;
#[cfg(test)]
mod commander_tests;
pub mod dnd;
pub mod editor;
#[cfg(test)]
pub mod fake;
#[cfg(test)]
mod frames;
pub mod keymap;
pub mod layout;
pub mod overlays;
pub mod panels;
pub mod places;
pub mod status;
pub mod theme;

/// A compact terminal-safe spinner shared by Places and the activity row.
pub(crate) const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// One key per scrollbar the column can draw, shared with STAR/KIT's
/// `chrome::scrollbar::Scrollbars` so [`app::App`] can keep a single
/// instance that presses, drags and releases whichever bar the mouse is
/// over -- the stack listing, the preview, the operations queue, or the
/// conflict prompt while it is the one overlay open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    Stack,
    Commander(usize),
    Preview,
    Operations,
    Conflict,
}

/// The one `Scrollbars` every module records its bar with and the app's
/// mouse handling reads back, keyed by [`Bar`].
pub type Bars = starkit::chrome::scrollbar::Scrollbars<Bar>;
