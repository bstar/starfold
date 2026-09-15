//! The terminal.
//!
//! Everything under here reaches ratatui and crossterm through
//! `starkit::ratatui::…` and `starkit::crossterm::…` rather than depending on
//! either directly -- see the crate doc. `app` is the last module to land
//! and is declared when it does.

pub mod app;
#[cfg(test)]
pub mod fake;
#[cfg(test)]
mod frames;
pub mod keymap;
pub mod layout;
pub mod overlays;
pub mod panels;
pub mod status;
pub mod theme;

/// One key per scrollbar the column can draw, shared with STAR/KIT's
/// `chrome::scrollbar::Scrollbars` so [`app::App`] can keep a single
/// instance that presses, drags and releases whichever bar the mouse is
/// over -- the stack listing, the preview, the operations queue, or the
/// conflict prompt while it is the one overlay open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    Stack,
    Preview,
    Operations,
    Conflict,
}

/// The one `Scrollbars` every module records its bar with and the app's
/// mouse handling reads back, keyed by [`Bar`].
pub type Bars = starkit::chrome::scrollbar::Scrollbars<Bar>;
