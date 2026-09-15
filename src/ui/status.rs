//! The one row at the bottom.
//!
//! Three fields, copied from STAR/CORD's `ui/status.rs`. The left is a fixed
//! reminder that the help exists. The middle is transient: a note for three
//! seconds, else the running operation's bar, else the key hints. The right
//! is **stable** -- what is marked, where you are, what can draw pictures --
//! because it is the part somebody glances at without stopping what they are
//! doing, and a field that moves is a field that has to be read rather than
//! glanced at.
//!
//! Its geometry is computed once, by [`fields`], and both the renderer and
//! the mouse come through it.

use std::time::{Duration, Instant};

use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Modifier, Style};

use super::panels::{fit, rgb, width_of};
use super::theme::Theme;
use crate::fold::handle::NoteLevel;

/// How long a note holds the middle field before the progress bar or the
/// hints come back.
pub const NOTE_FOR: Duration = Duration::from_secs(3);

/// What a click on the status line lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Help,
    /// The `location` word inside the stable right-hand field.
    Location,
    /// The running operation's bar, while it is what the middle field shows.
    Progress,
}

pub struct View<'a> {
    pub theme: &'a Theme,
    pub note: Option<&'a (String, NoteLevel, Instant)>,
    pub now: Instant,
    /// `"COPYING ████████░░ 78%"`, while an operation is running.
    pub progress: Option<&'a str>,
    pub hints: &'a str,
    /// `"2 marked · 14.2 MB"`, or empty when nothing is marked.
    pub marked: &'a str,
    pub location: &'a str,
    pub graphics: &'a str,
}

/// What the middle field is presently showing -- decides both its colour and
/// whether a click on it means [`Hit::Progress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MiddleKind {
    Note(NoteLevel),
    Progress,
    Hints,
}

impl View<'_> {
    /// The middle field: a fresh note first (urgent and transient), else the
    /// running op's bar (urgent and ongoing), else the key hints (what is
    /// there the rest of the time).
    fn middle(&self) -> (String, MiddleKind) {
        if let Some((text, level, at)) = self.note {
            if self.now.duration_since(*at) < NOTE_FOR {
                return (text.clone(), MiddleKind::Note(*level));
            }
        }
        if let Some(p) = self.progress {
            return (p.to_string(), MiddleKind::Progress);
        }
        (self.hints.to_string(), MiddleKind::Hints)
    }

    /// The stable right-hand field: only the parts that have anything to
    /// say, in a fixed order, so nothing to the right of an empty one shifts
    /// when it appears.
    ///
    /// Held to `max_w` columns: the graphics word goes first, then the
    /// location loses its head -- `…/scratchpad/tree` -- because the end of a
    /// path is the part that says where you are. The marks are never cut.
    /// Returns the field and the location as it was drawn.
    fn right(&self, max_w: u16) -> (String, String) {
        let join = |parts: &[&str]| {
            parts
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("  ")
        };
        let full = join(&[self.marked, self.location, self.graphics]);
        if width_of(&full) <= max_w {
            return (full, self.location.to_string());
        }
        let without = join(&[self.marked, self.location]);
        if width_of(&without) <= max_w {
            return (without, self.location.to_string());
        }
        let marked_w = if self.marked.is_empty() {
            0
        } else {
            width_of(self.marked) + 2
        };
        let room = max_w.saturating_sub(marked_w);
        let location = elide_head(self.location, room);
        (join(&[self.marked, &location]), location)
    }
}

/// `…/the/end` -- the tail of `text` that fits in `width`, with an ellipsis
/// in front of it.
fn elide_head(text: &str, width: u16) -> String {
    if width_of(text) <= width {
        return text.to_string();
    }
    if width < 2 {
        return String::new();
    }
    let keep = usize::from(width - 1);
    let clusters: Vec<&str> = starkit::wrap::clusters(text).map(|(_, c)| c).collect();
    let mut back = Vec::new();
    let mut used = 0usize;
    for c in clusters.iter().rev() {
        let cw = usize::from(width_of(c));
        if used + cw > keep {
            break;
        }
        back.push(*c);
        used += cw;
    }
    let mut out = String::from("\u{2026}");
    for c in back.iter().rev() {
        out.push_str(c);
    }
    out
}

/// The fewest columns the middle field keeps against a long right-hand one:
/// enough for a running operation's bar and its percentage.
const MIDDLE_MIN: u16 = 24;

const HELP: &str = "? help";

/// Where each field sits. The renderer draws from this and the mouse tests
/// against it, so a word that was not drawn cannot be clicked.
pub struct Fields {
    pub help: Rect,
    /// Whichever of note / progress / hints is currently shown.
    pub middle: Rect,
    /// The `location` word inside the right-hand field, when it has one.
    pub location: Rect,
}

pub fn fields(area: Rect, v: &View<'_>) -> Fields {
    let empty_at = |r: Rect| Rect {
        width: 0,
        height: 0,
        ..r
    };
    if area.height == 0 || area.width == 0 {
        return Fields {
            help: empty_at(area),
            middle: empty_at(area),
            location: empty_at(area),
        };
    }

    let help_w = width_of(HELP).min(area.width);
    let help = Rect {
        x: area.x,
        y: area.y,
        width: help_w,
        height: 1,
    };

    let max_right = area
        .width
        .saturating_sub(help_w + 2)
        .saturating_sub(MIDDLE_MIN)
        .max(area.width.saturating_sub(help_w + 2) / 2);
    let (right, drawn_location) = v.right(max_right);
    let right_w = width_of(&right).min(area.width.saturating_sub(help_w + 2));
    let right_x = area.x + area.width - right_w;

    let location = if drawn_location.is_empty() || right_w == 0 {
        empty_at(area)
    } else {
        // Whatever the right field joins in front of `location` -- `marked`
        // plus its own separator, in the same order `right()` builds it.
        let lead_w = if v.marked.is_empty() {
            0
        } else {
            width_of(v.marked) + 2
        };
        let lead_w = lead_w.min(right_w);
        let loc_w = width_of(&drawn_location).min(right_w.saturating_sub(lead_w));
        Rect {
            x: right_x + lead_w,
            y: area.y,
            width: loc_w,
            height: 1,
        }
    };

    let reserved_right = if right_w > 0 { right_w + 2 } else { 0 };
    let middle_w = area
        .width
        .saturating_sub(help_w + 2)
        .saturating_sub(reserved_right);
    let middle = Rect {
        x: area.x + help_w + 2,
        y: area.y,
        width: middle_w,
        height: if middle_w > 0 { 1 } else { 0 },
    };

    Fields {
        help,
        middle,
        location,
    }
}

pub fn hit(area: Rect, v: &View<'_>, x: u16, y: u16) -> Option<Hit> {
    let f = fields(area, v);
    let inside = |r: Rect| r.width > 0 && y == r.y && x >= r.x && x < r.x + r.width;
    if inside(f.help) {
        return Some(Hit::Help);
    }
    if inside(f.location) {
        return Some(Hit::Location);
    }
    if inside(f.middle) && matches!(v.middle().1, MiddleKind::Progress) {
        return Some(Hit::Progress);
    }
    None
}

pub fn render(area: Rect, buf: &mut Buffer, v: &View<'_>) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let t = v.theme;
    let base = Style::default().fg(rgb(t.status_fg)).bg(rgb(t.status_bg));
    buf.set_style(area, base);

    let f = fields(area, v);
    let (middle_text, kind) = v.middle();
    let max_right = area
        .width
        .saturating_sub(f.help.width + 2)
        .saturating_sub(MIDDLE_MIN)
        .max(area.width.saturating_sub(f.help.width + 2) / 2);
    let (right, _) = v.right(max_right);

    if f.help.width > 0 {
        buf.set_string(
            f.help.x,
            f.help.y,
            "?",
            Style::default()
                .fg(rgb(t.hint_key_fg))
                .bg(rgb(t.hint_key_bg))
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            f.help.x + 1,
            f.help.y,
            " help",
            base.fg(rgb(t.hint_desc_fg)),
        );
    }

    if f.middle.width > 0 {
        let style = match kind {
            MiddleKind::Note(NoteLevel::Error) => base.fg(rgb(t.error)),
            MiddleKind::Note(NoteLevel::Warning) => base.fg(rgb(t.warn)),
            MiddleKind::Note(NoteLevel::Info) => base.fg(rgb(t.accent)),
            MiddleKind::Progress => base.fg(rgb(t.fold.progress_fg)).bg(rgb(t.fold.progress_bg)),
            MiddleKind::Hints => base,
        };
        buf.set_string(
            f.middle.x,
            f.middle.y,
            fit(&middle_text, f.middle.width),
            style,
        );
    }

    if !right.is_empty() {
        let w = width_of(&right).min(area.width);
        let x = area.x + area.width - w;
        buf.set_string(x, area.y, &right, base);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::tests_support::theme;

    fn view<'a>(
        theme: &'a Theme,
        note: Option<&'a (String, NoteLevel, Instant)>,
        now: Instant,
    ) -> View<'a> {
        View {
            theme,
            note,
            now,
            progress: Some("COPYING \u{2588}\u{2588}\u{2588}\u{2591}\u{2591} 60%"),
            hints: "/ filter  space mark  enter open",
            marked: "2 marked \u{b7} 14.2 MB",
            location: "~/projects/starwire",
            graphics: "kitty",
        }
    }

    #[test]
    fn the_middle_field_prefers_a_fresh_note_then_progress_then_hints() {
        let t = theme("terminal");
        let at = Instant::now();
        let note = ("saved".to_string(), NoteLevel::Info, at);

        let fresh = view(&t, Some(&note), at + Duration::from_secs(1));
        assert_eq!(fresh.middle().0, "saved");

        let stale_but_running = view(&t, Some(&note), at + NOTE_FOR + Duration::from_millis(1));
        assert!(stale_but_running.middle().0.contains("60%"));

        let mut idle = view(&t, None, at);
        idle.progress = None;
        assert_eq!(idle.middle().0, idle.hints);
    }

    #[test]
    fn the_right_field_is_stable_and_help_sits_at_the_left_edge() {
        let t = theme("terminal");
        let v = view(&t, None, Instant::now());
        let area = Rect::new(0, 0, 80, 1);
        let f = fields(area, &v);
        assert_eq!(f.help.x, 0);
        assert!(f.location.width > 0, "location should have been drawn");
        assert!(v.right(200).0.contains(v.location));
    }

    #[test]
    fn a_click_on_help_answers_help() {
        let t = theme("terminal");
        let v = view(&t, None, Instant::now());
        let area = Rect::new(0, 0, 80, 1);
        assert_eq!(hit(area, &v, 0, 0), Some(Hit::Help));
    }
}
