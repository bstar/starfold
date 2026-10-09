//! Native Commander chrome. All rectangles stay in the controller's geometry.
use starkit::native_surface::{PixelRect as R, Primitive, Surface};
use starkit::ratatui::layout::Rect;
use starkit::terminal_graphics::protocol::Component;
use starkit::theme::{color::Rgb, schema::Variant, Theme as Core};

#[derive(Debug, Clone)]
pub struct Colors {
    pub frame: Rgb,
    pub title: Rgb,
    pub inset: Rgb,
    pub foreground: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub highlight: Rgb,
    pub shadow: Rgb,
    pub button: Rgb,
}
impl Colors {
    pub fn new(theme: &Core) -> Self {
        let rgb = |s| Rgb::parse_hex(s).expect("rack palette");
        let values = match theme.id.as_str() {
            "terminal" | "winamp-classic" => Some([
                "#30333f", "#242732", "#0e1712", "#d8d8ce", "#9ba1af", "#a5d78d", "#656b7c",
                "#151821", "#494f5f",
            ]),
            "catppuccin-mocha" => Some([
                "#313244", "#1e1e2e", "#181d1b", "#cdd6f4", "#a6adc8", "#a6e3a1", "#626581",
                "#11111b", "#45475a",
            ]),
            "catppuccin-latte" => Some([
                "#ccd0da", "#bcc0cc", "#e6e9e0", "#343947", "#5c6371", "#386132", "#f3f4f7",
                "#6c7281", "#e3e6ec",
            ]),
            _ => None,
        };
        if let Some(v) = values {
            return Self {
                frame: rgb(v[0]),
                title: rgb(v[1]),
                inset: rgb(v[2]),
                foreground: rgb(v[3]),
                muted: rgb(v[4]),
                accent: rgb(v[5]),
                highlight: rgb(v[6]),
                shadow: rgb(v[7]),
                button: rgb(v[8]),
            };
        }
        let light = theme.variant == Variant::Light;
        Self {
            frame: theme.panel_bg.mix(theme.fg, 0.12),
            title: theme.panel_bg,
            inset: theme.bg.mix(theme.accent, 0.035),
            foreground: theme.fg,
            muted: theme.dim,
            accent: theme.accent,
            highlight: theme
                .panel_bg
                .mix(starkit::theme::WHITE, if light { 0.8 } else { 0.3 }),
            shadow: theme.panel_bg.mix(starkit::theme::BLACK, 0.4),
            button: theme.panel_bg.mix(theme.fg, 0.2),
        }
    }
    pub fn apply(&self, core: &mut Core) {
        core.panel_bg = self.inset;
        core.panel_fg = self.foreground.ensure_contrast(self.inset, 4.5);
        core.fg = core.panel_fg;
        core.dim = self.muted.ensure_contrast(self.inset, 4.5);
        core.empty_fg = core.dim;
        core.accent = self.accent.ensure_contrast(self.inset, 4.5);
        core.ok = core.accent;
        core.header_bg = self.title;
        core.header_fg = self.muted.ensure_contrast(self.title, 4.5);
        core.row_bg = self.inset;
        core.row_fg = core.fg;
        core.row_meta_fg = core.dim;
        core.row_selected_bg = self.inset.mix(self.accent, 0.28);
        core.row_cursor_bg = core.row_selected_bg;
        core.status_bg = self.inset;
        core.status_fg = core.dim;
        core.hint_key_fg = core.accent;
        core.hint_desc_fg = core.dim;
        core.border = self.shadow;
        core.border_focused = self.highlight;
    }
}
pub fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// Raised and recessed edges use the same bounded pixel rectangles as text.
pub fn bevel(s: &mut Surface, r: R, p: &Colors, pressed: bool) {
    if r.width < 2 || r.height < 2 {
        return;
    }
    s.fill(r, &hex(if pressed { p.title } else { p.button }), 0);
    let (hi, lo) = if pressed {
        (p.shadow, p.highlight)
    } else {
        (p.highlight, p.shadow)
    };
    s.fill(R::new(r.x, r.y, r.width, 1), &hex(hi), 0);
    s.fill(R::new(r.x, r.y, 1, r.height), &hex(hi), 0);
    s.fill(R::new(r.x, r.y + r.height - 1, r.width, 1), &hex(lo), 0);
    s.fill(R::new(r.x + r.width - 1, r.y, 1, r.height), &hex(lo), 0);
}
pub fn mono(s: &mut Surface, rect: R, text: impl Into<String>, color: Rgb, size: u16, bold: bool) {
    s.nodes.push(Primitive::Text {
        rect,
        text: text.into(),
        color: hex(color),
        size,
        bold,
        mono: true,
    });
}

/// Paint only the border cells, preserving previews, metadata and queue text.
/// Panels remain in the scene for scrolling identity and legacy consumers.
pub fn frame(rect: Rect, cell: (u16, u16), p: &Colors) -> Vec<Component> {
    let (cw, ch) = cell;
    if rect.width < 3 || rect.height < 3 {
        return vec![];
    }
    let mut parts = vec![];
    for (r, horizontal, leading) in [
        (Rect::new(rect.x, rect.y, rect.width, 1), true, true),
        (
            Rect::new(rect.x, rect.bottom() - 1, rect.width, 1),
            true,
            false,
        ),
        (
            Rect::new(rect.x, rect.y + 1, 1, rect.height - 2),
            false,
            true,
        ),
        (
            Rect::new(rect.right() - 1, rect.y + 1, 1, rect.height - 2),
            false,
            false,
        ),
    ] {
        let mut s = Surface::new(r.width * cw, r.height * ch, hex(p.frame));
        let color = hex(if leading { p.highlight } else { p.shadow });
        if horizontal {
            s.fill(
                R::new(0, if leading { 0 } else { s.height - 1 }, s.width, 1),
                &color,
                0,
            );
            let y = if leading { s.height - 1 } else { 0 };
            s.fill(
                R::new(cw, y, s.width.saturating_sub(2 * cw), 1),
                &hex(if leading { p.shadow } else { p.highlight }),
                0,
            );
        } else {
            s.fill(
                R::new(if leading { 0 } else { s.width - 1 }, 0, 1, s.height),
                &color,
                0,
            );
            s.fill(
                R::new(
                    if leading { s.width - 1 } else { 0 },
                    ch.min(s.height),
                    1,
                    s.height.saturating_sub(ch),
                ),
                &hex(if leading { p.shadow } else { p.highlight }),
                0,
            );
        }
        parts.push(Component::Surface {
            rect: r.into(),
            surface: s,
        });
    }
    parts
}

pub fn tab(
    rect: Rect,
    label: &str,
    number: Option<u32>,
    active: bool,
    close: Option<Rect>,
    cell: (u16, u16),
    p: &Colors,
) -> Component {
    let (cw, ch) = cell;
    let mut s = Surface::new(rect.width * cw, rect.height * ch, hex(p.frame));
    let height = s.height;
    let button = R::new(
        2.min(s.width),
        2.min(height),
        s.width.saturating_sub(5),
        height.saturating_sub(4),
    );
    bevel(&mut s, button, p, active);
    let text = number.map_or_else(|| label.to_string(), |n| format!("{n:02} {label}"));
    let end = close.map_or(s.width, |r| (r.x - rect.x) * cw);
    let x = 8.min(s.width);
    mono(
        &mut s,
        R::new(x, 0, end.saturating_sub(x + 3), height),
        text,
        if active { p.accent } else { p.foreground }
            .ensure_contrast(if active { p.title } else { p.button }, 4.5),
        starkit::native_surface::Metrics::from_cell(cw, ch).font,
        false,
    );
    if let Some(close) = close {
        mono(
            &mut s,
            R::new((close.x - rect.x) * cw, 0, close.width * cw, height),
            "×",
            p.muted.ensure_contrast(p.button, 4.5),
            starkit::native_surface::Metrics::from_cell(cw, ch).font,
            false,
        );
    }
    Component::Surface {
        rect: rect.into(),
        surface: s,
    }
}

pub struct Workspace<'a> {
    pub count: usize,
    pub marked: &'a str,
    pub location: &'a str,
    pub mode: &'a str,
    pub space: Option<(u64, u64)>,
}
pub fn workspace(rect: Rect, cell: (u16, u16), p: &Colors, view: Workspace<'_>) -> Component {
    let Workspace {
        count,
        marked,
        location,
        mode,
        space,
    } = view;
    let (cw, ch) = cell;
    let mut s = Surface::new(rect.width * cw, rect.height * ch, hex(p.frame));
    let font = starkit::native_surface::Metrics::from_cell(cw, ch).font;
    let title_height = ch.min(s.height);
    if ch < 10 || s.width < 100 {
        let width = s.width;
        let height = s.height;
        mono(
            &mut s,
            R::new(0, 0, width, height),
            location,
            p.foreground,
            font,
            false,
        );
        return Component::Surface {
            rect: rect.into(),
            surface: s,
        };
    }
    s.fill(
        R::new(
            1,
            1,
            s.width.saturating_sub(2),
            title_height.saturating_sub(1),
        ),
        &hex(p.title),
        0,
    );
    let title_width = s.width.saturating_sub(16);
    mono(
        &mut s,
        R::new(8, 0, title_width, title_height),
        "S T A R / F O L D",
        p.muted.ensure_contrast(p.title, 4.5),
        font,
        false,
    );
    let display_rows = rect.height.saturating_sub(3).min(5);
    let roomy = rect.height >= 9;
    let display = R::new(
        8,
        ch + if roomy { 6 } else { 0 },
        s.width.saturating_sub(16),
        display_rows * ch - if roomy { 10 } else { 0 },
    );
    bevel(&mut s, display, p, true);
    s.fill(
        R::new(
            display.x + 1,
            display.y + 1,
            display.width.saturating_sub(2),
            display.height.saturating_sub(2),
        ),
        &hex(p.inset),
        0,
    );
    let digit_width = (if roomy {
        display.height * 2 / 5
    } else {
        ch * 3 / 4
    })
    .max(6);
    let text = format!("{count:02}");
    let digits_width = (text.len() as u16 * digit_width).min(s.width / 4);
    let patterns = [
        "abcdef", "bc", "abdeg", "abcdg", "bcfg", "acdfg", "acdefg", "abc", "abcdefg", "abcdfg",
    ];
    let unit = (digits_width / (text.len() as u16).max(1)).max(3);
    for (i, digit) in text.bytes().enumerate() {
        let x = 14 + i as u16 * unit;
        let y = display.y + if roomy { 10 } else { 5 };
        let h = display
            .height
            .saturating_sub(if roomy { 36 } else { 10 })
            .max(8);
        let w = unit.saturating_sub(5).max(2);
        let segments = [
            R::new(x + 1, y, w, 2),
            R::new(x + w, y + 2, 2, h / 2 - 2),
            R::new(x + w, y + h / 2 + 1, 2, h / 2 - 2),
            R::new(x + 1, y + h - 2, w, 2),
            R::new(x, y + h / 2 + 1, 2, h / 2 - 2),
            R::new(x, y + 2, 2, h / 2 - 2),
            R::new(x + 1, y + h / 2 - 1, w, 2),
        ];
        for (segment, r) in "abcdefg".chars().zip(segments) {
            s.fill(
                r,
                &hex(if patterns[usize::from(digit - b'0')].contains(segment) {
                    p.accent
                } else {
                    p.inset.mix(p.accent, 0.12)
                }),
                0,
            );
        }
    }
    let x = digits_width + 40;
    let line_y = if roomy {
        display.y + display.height / 2 - ch
    } else {
        ch
    };
    let volume_width = if s.width >= 900 && space.is_some() {
        230.min(s.width / 4)
    } else {
        0
    };
    let width = s.width.saturating_sub(x + volume_width + 24);
    if rect.height >= 9 {
        mono(
            &mut s,
            R::new(14, display.y + display.height - ch, digits_width + 10, ch),
            "MARKED",
            p.accent,
            font.saturating_sub(4).max(1),
            false,
        );
        mono(
            &mut s,
            R::new(x, line_y.saturating_sub(ch), width, ch),
            mode,
            p.accent,
            font.saturating_sub(4).max(1),
            false,
        );
    }
    mono(
        &mut s,
        R::new(x, line_y, width, ch),
        location,
        p.accent,
        font,
        false,
    );
    mono(
        &mut s,
        R::new(x, line_y + ch, width, ch),
        if marked.is_empty() {
            "0 marked"
        } else {
            marked
        },
        p.muted,
        font.saturating_sub(2).max(1),
        false,
    );
    if let Some((total, available)) = space
        .filter(|(total, _)| *total > 0)
        .filter(|_| volume_width > 0)
    {
        let x = s.width - volume_width - 16;
        mono(
            &mut s,
            R::new(x, line_y, volume_width, ch),
            format!("{} FREE", crate::fold::format::size(available)),
            p.accent,
            font.saturating_sub(2).max(1),
            false,
        );
        let used = total.saturating_sub(available.min(total));
        let segments = volume_width / 6;
        for i in 0..segments {
            let lit = u128::from(i) * u128::from(total) < u128::from(used) * u128::from(segments);
            for j in 0..3.min(ch.saturating_sub(5) / 4) {
                s.fill(
                    R::new(x + i * 6, line_y + ch + 3 + j * 4, 4, 2),
                    &hex(if lit {
                        p.accent
                    } else {
                        p.inset.mix(p.accent, 0.12)
                    }),
                    0,
                );
            }
        }
    }
    Component::Surface {
        rect: rect.into(),
        surface: s,
    }
}
