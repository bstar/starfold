//! Native Commander chrome. All rectangles stay in the controller's geometry.
use starkit::native_surface::{PixelRect as R, Primitive, Surface};
use starkit::ratatui::layout::Rect;
use starkit::terminal_graphics::protocol::Component;
use starkit::theme::{color::Rgb, Theme as Core};

#[derive(Clone)]
pub struct Colors {
    pub frame: Rgb,
    pub title: Rgb,
    pub inset: Rgb,
    pub foreground: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub highlight: Rgb,
    pub shadow: Rgb,
    shared: std::sync::Arc<starkit::native_surface::style::Style>,
    native_skins: bool,
}
impl std::fmt::Debug for Colors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Colors")
            .field("frame", &self.frame)
            .field("inset", &self.inset)
            .finish_non_exhaustive()
    }
}
impl Colors {
    pub fn palette(theme: &Core) -> starkit::native_surface::style::Palette {
        use starkit::native_surface::style::{Palette, REFERENCE_PALETTE, SILVER_PALETTE};
        match theme.id.as_str() {
            "winamp-classic" => return REFERENCE_PALETTE,
            "winamp-silver" => return SILVER_PALETTE,
            _ => {}
        }
        let mut palette = Palette::from_theme(theme);
        // Match AMP's graphical interpretation of Kanagawa: raised slate
        // frames around darker wells, rather than terminal-base-colored frames.
        if theme.name == "Kanagawa" {
            palette.bg = [theme.header_bg.r, theme.header_bg.g, theme.header_bg.b];
            let inset = theme.bg.mix(Rgb::new(0, 0, 0), 0.12);
            palette.inset_bg = Some([inset.r, inset.g, inset.b]);
        }
        palette
    }

    pub fn from_palette(palette: starkit::native_surface::style::Palette) -> Self {
        use starkit::native_surface::style::Style;
        let mut style = Style::new(palette);
        style.metrics.small_font = 12;
        let rgb = |s: &str| Rgb::parse_hex(s).expect("KIT workspace color");
        Self {
            frame: rgb(&style.colors.panel),
            title: rgb(&style.colors.title),
            inset: rgb(&style.colors.inset),
            foreground: rgb(&style.colors.ink),
            muted: rgb(&style.colors.dim),
            accent: rgb(&style.colors.accent),
            highlight: rgb(&style.colors.highlight),
            shadow: rgb(&style.colors.shadow),
            shared: std::sync::Arc::new(style),
            native_skins: true,
        }
    }

    pub fn style(&self) -> &starkit::native_surface::style::Style {
        &self.shared
    }

    /// AMP's original one-pixel corner mask and exact selection color. Keep
    /// this in physical pixels so terminal zoom never thickens the outline.
    pub fn focus(&self, surface: &mut Surface, rect: R) {
        if rect.width < 24 || rect.height < 24 {
            return;
        }
        let palette = self.shared.palette;
        let color = starkit::native_surface::style::hex(
            if palette == starkit::native_surface::style::SILVER_PALETTE {
                palette.fg
            } else {
                palette.accent
            },
        );
        if self.native_skins {
            static MASK: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
            let (asset, png) = MASK.get_or_init(|| {
                let image = starkit::image::load_from_memory(include_bytes!(
                    "../../assets/chrome/amp-focus-rounded.png"
                ))
                .expect("original AMP focus mask")
                .to_rgba8();
                starkit::terminal_graphics::assets::encode_skin(&image)
                    .expect("bounded AMP focus mask")
            });
            surface.assets.insert(asset.clone(), Some(png.clone()));
            surface.nodes.push(Primitive::Sprite {
                rect,
                asset: asset.clone(),
                source: R::new(0, 0, 24, 24),
                insets: Some([6; 4]),
                tint: Some(color),
            });
        } else {
            for edge in [
                R::new(rect.x + 3, rect.y, rect.width - 6, 1),
                R::new(rect.x + 3, rect.y + rect.height - 1, rect.width - 6, 1),
                R::new(rect.x, rect.y + 3, 1, rect.height - 6),
                R::new(rect.x + rect.width - 1, rect.y + 3, 1, rect.height - 6),
            ] {
                surface.fill(edge, &color, 0);
            }
        }
    }

    pub fn set_native_skins(&mut self, supported: bool) {
        if self.native_skins != supported {
            self.shared =
                std::sync::Arc::new(self.shared.as_ref().clone().with_native_skins(supported));
            self.native_skins = supported;
        }
    }

    pub fn apply(&self, core: &mut Core) {
        let palette = self.style().palette;
        let rgb = |[r, g, b]: [u8; 3]| Rgb::new(r, g, b);
        core.bg = rgb(palette.bg);
        core.error = rgb(palette.error);
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
        core.row_selected_bg = rgb(palette.selected);
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
    p.style().frame(s, r, pressed);
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

pub fn tab(
    rect: Rect,
    label: &str,
    _number: Option<u32>,
    active: bool,
    close: Option<Rect>,
    cell: (u16, u16),
    p: &Colors,
) -> Component {
    let (cw, ch) = cell;
    let mut s = p.style().surface(rect.width * cw, rect.height * ch);
    let height = s.height;
    if close.is_none() {
        let bounds = R::new(0, 0, s.width, height);
        s.text_centered(bounds, label, &hex(p.muted), 11, false);
        return Component::Surface {
            rect: rect.into(),
            surface: s,
        };
    }
    let face = R::new(2, 2, s.width.saturating_sub(4), height.saturating_sub(4));
    // KIT's compact tools give tabs a quiet face and an active underline.
    // Legacy Tab markers are removed before painting this surface.
    p.style().tool(&mut s, face, "", "tab", active);
    let end = close.map_or(s.width, |r| (r.x - rect.x) * cw);
    let x = 10.min(s.width);
    mono(
        &mut s,
        R::new(x, 0, end.saturating_sub(x + 4), height),
        label,
        if active { p.foreground } else { p.muted },
        11,
        false,
    );
    if let Some(close) = close {
        starkit::native_surface::style::close_button(
            &mut s,
            R::new((close.x - rect.x) * cw, 0, close.width * cw, height),
            &p.style().colors,
            "close-tab",
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
