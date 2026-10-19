//! Native rack contents, painted from the controller's cached views.
use super::*;
use crate::ui::rack::{self, Colors};
use starkit::native_surface::{PixelRect as R, Surface};

#[derive(Clone, Copy)]
pub(super) enum RackHit {
    Action(Action),
    Word(panels::Word),
    Pane(usize, Action),
    Operation(usize),
}

pub(super) fn pixel_bounds(
    rect: Rect,
    cell: (u16, u16),
    placements: &[starkit::terminal_graphics::placement::Placement],
) -> R {
    placements
        .iter()
        .find(|p| {
            rect.x >= p.source.x
                && rect.y >= p.source.y
                && rect.right() <= p.source.x + p.source.width
                && rect.bottom() <= p.source.y + p.source.height
        })
        .map(|p| Canvas::project(p, rect))
        .unwrap_or(R::new(
            rect.x * cell.0,
            rect.y * cell.1,
            rect.width * cell.0,
            rect.height * cell.1,
        ))
}

struct Canvas<'a> {
    rect: Rect,
    cell: (u16, u16),
    colors: &'a Colors,
    surface: Surface,
    placement: Option<starkit::terminal_graphics::placement::Placement>,
    target: R,
    hits: Vec<(Rect, RackHit)>,
}
impl<'a> Canvas<'a> {
    fn new(
        rect: Rect,
        cell: (u16, u16),
        colors: &'a Colors,
        placements: &[starkit::terminal_graphics::placement::Placement],
    ) -> Self {
        let placement = placements
            .iter()
            .find(|p| {
                rect.x >= p.source.x
                    && rect.y >= p.source.y
                    && rect.right() <= p.source.x + p.source.width
                    && rect.bottom() <= p.source.y + p.source.height
            })
            .cloned();
        let target = pixel_bounds(rect, cell, placements);
        let surface = colors
            .style()
            .panel(target.width.max(1), target.height.max(1), "");
        Self {
            rect,
            cell,
            colors,
            surface,
            placement,
            target,
            hits: vec![],
        }
    }
    fn project(p: &starkit::terminal_graphics::placement::Placement, r: Rect) -> R {
        let x = |column: u16| {
            p.target.x
                + (u32::from(column.saturating_sub(p.source.x)) * u32::from(p.target.width)
                    / u32::from(p.source.width.max(1))) as u16
        };
        let y = |row: u16| p.target.y + p.row_edge(row.saturating_sub(p.source.y)).round() as u16;
        R::new(
            x(r.x),
            y(r.y),
            x(r.right()).saturating_sub(x(r.x)),
            y(r.bottom()).saturating_sub(y(r.y)),
        )
    }
    fn pixels(&self, r: Rect) -> R {
        if let Some(p) = &self.placement {
            let mut pixels = Self::project(p, r);
            pixels.x = pixels.x.saturating_sub(self.target.x);
            pixels.y = pixels.y.saturating_sub(self.target.y);
            return pixels;
        }
        R::new(
            (r.x - self.rect.x) * self.cell.0,
            (r.y - self.rect.y) * self.cell.1,
            r.width * self.cell.0,
            r.height * self.cell.1,
        )
    }
    fn fill(&mut self, r: Rect, color: starkit::theme::color::Rgb) {
        self.surface.fill(self.pixels(r), &rack::hex(color), 0);
    }
    fn text(&mut self, r: Rect, text: impl Into<String>, accent: bool, small: bool) {
        let color = if accent {
            self.colors.accent
        } else {
            self.colors.muted
        };
        let font = if small { 11 } else { 12 };
        let mut pixels = self.pixels(r);
        pixels.x += 6;
        pixels.width = pixels.width.saturating_sub(12);
        starkit::native_surface::classic::label(
            &mut self.surface,
            pixels,
            text,
            &rack::hex(color),
            font,
            false,
        );
    }
    fn filename(&mut self, r: Rect, name: String, highlighted: bool) {
        let mut px = self.pixels(r);
        px.x += 6;
        px.width = px.width.saturating_sub(12);
        let font = 11;
        rack::mono(
            &mut self.surface,
            px,
            name,
            if highlighted {
                self.colors.accent
            } else {
                self.colors.foreground
            },
            font,
            highlighted,
        );
    }
    fn meter(&mut self, r: Rect, fraction: f64) {
        let px = self.pixels(r);
        let count = px.width / 8;
        for index in 0..count {
            self.surface.fill(
                R::new(px.x + index * 8, px.y + px.height / 2, 6, 4.min(px.height)),
                &rack::hex(if f64::from(index) < fraction * f64::from(count) {
                    self.colors.accent
                } else {
                    self.colors.inset
                }),
                0,
            );
        }
    }
    fn well(&mut self, r: Rect) {
        self.surface
            .fill(self.pixels(r), &rack::hex(self.colors.inset), 0);
    }
    fn button(&mut self, r: Rect, label: &str, hit: RackHit, pressed: bool) {
        let mut px = self.pixels(r);
        px.x += 2;
        px.y += 2;
        px.width = px.width.saturating_sub(4);
        px.height = px.height.saturating_sub(4);
        self.colors
            .style()
            .button(&mut self.surface, px, label, "fold-control", pressed);
        self.hits.push((r, hit));
    }
    fn header_action(&mut self, r: Rect, label: &str, hit: RackHit, hotkeys: bool) {
        // AMP's title-strip actions are quiet labels; ordinary toolbar buttons
        // retain KIT's shared button artwork below the list.
        let mut pixels = self.pixels(r);
        pixels.y = 7;
        pixels.height = 24.min(self.surface.height.saturating_sub(7));
        starkit::native_surface::classic::label(
            &mut self.surface,
            pixels,
            label,
            &rack::hex(self.colors.muted),
            10,
            false,
        );
        if let RackHit::Word(word) = hit {
            native_mnemonic(
                &mut self.surface,
                pixels,
                label,
                word.mnemonic(),
                hotkeys,
                &rack::hex(
                    self.colors
                        .title
                        .best_contrast_against(&[starkit::theme::WHITE, starkit::theme::BLACK]),
                ),
            );
        }
        self.hits.push((r, hit));
    }
    fn title(&mut self, label: &str) {
        self.surface = self
            .colors
            .style()
            .panel(self.target.width, self.target.height, label);
    }
    fn finish(self, scene: &mut Scene, hits: &mut Vec<(Rect, RackHit)>) {
        // A fully native pane replaces its cell-content surface, including
        // the frame; sending both would paint and transmit the same pane twice.
        scene.components.retain(|component| {
            !matches!(component,
                Component::Surface { rect, .. } if *rect == self.rect.into()
            )
        });
        scene.components.push(Component::Surface {
            rect: self.rect.into(),
            surface: self.surface,
        });
        hits.extend(self.hits);
    }
}

/// Ordinary image inspection uses a thumbnail beside its metadata. Movie,
/// document and embedded-player geometries remain owned by their renderers.
pub(super) fn image_rect(area: Rect) -> Rect {
    let body = panels::preview::content_rect(area);
    Rect::new(
        body.x,
        body.y,
        body.width * 3 / 10,
        body.height.saturating_sub(2),
    )
}

/// Use the same KIT panel as AMP even when content still comes from cells.
/// Content and header controls share the controller's physical projection.
pub(super) fn pane(
    scene: &Scene,
    rect: Rect,
    title: &str,
    words: &[panels::Word],
    colors: &Colors,
    cell: (u16, u16),
    hotkeys: bool,
) -> Component {
    use starkit::chrome::header::{self, Word as _};
    let mut c = Canvas::new(rect, cell, colors, &scene.placements);
    c.title(title);
    if rect.width > 2 && rect.height > 3 {
        let body = Rect::new(rect.x + 1, rect.y + 2, rect.width - 2, rect.height - 3);
        c.well(body);
        let mut labels = Vec::new();
        for span in &scene.spans {
            if span.y < body.y || span.y >= body.bottom() {
                continue;
            }
            let mut x = span.x;
            let mut start = None;
            let mut end = 0;
            let mut column = 0;
            for (byte, cluster) in starkit::wrap::clusters(&span.text) {
                let width = starkit::wrap::width_of(cluster);
                if x >= body.x && x + width <= body.right() {
                    if start.is_none() {
                        start = Some(byte);
                        column = x;
                    }
                    end = byte + cluster.len();
                }
                x = x.saturating_add(width);
            }
            let Some(start) = start else {
                continue;
            };
            let text = &span.text[start..end];
            let background = c.pixels(Rect::new(column, span.y, starkit::wrap::width_of(text), 1));
            c.surface.fill(background, &span.background, 0);
            // Cell rows carry padding to the terminal's right edge. It is
            // background, not content for the native font to ellipsize.
            let text = text.trim_end_matches(' ');
            if text.is_empty() {
                continue;
            }
            let column = if title.starts_with("PREVIEW") && span.y == panels::preview::body(rect).y
            {
                // Cell-aligned metadata can otherwise receive a tiny native
                // box at the right edge. Give it a full readable summary row.
                body.x
            } else {
                column
            };
            let pixels = c.pixels(Rect::new(column, span.y, body.right() - column, 1));
            labels.push((pixels, text, &span.foreground, span.bold));
        }
        // Finish backgrounds before native text: later cell-padding spans
        // must not paint over glyphs shaped beyond their old cell allocation.
        for (pixels, text, foreground, bold) in labels {
            starkit::native_surface::classic::label(
                &mut c.surface,
                pixels,
                text,
                foreground,
                12,
                bold,
            );
        }
    }
    let slots = header::slots(rect, words);
    let end = slots
        .iter()
        .map(|(word, slot)| {
            if *word == panels::Word::Close {
                native_close_rect(c.surface.width, c.surface.height).x
            } else {
                c.pixels(*slot).x
            }
        })
        .min()
        .unwrap_or(c.surface.width.saturating_sub(18));
    for node in &mut c.surface.nodes {
        if let starkit::native_surface::Primitive::Text { rect, .. } = node {
            // The title's full shaping box ends before the header actions.
            if rect.y == 7 {
                rect.width = end.saturating_sub(rect.x + 8);
            }
        }
    }
    for (word, slot) in slots {
        let mut pixels = c.pixels(slot);
        pixels.y = 7;
        pixels.height = 24.min(c.surface.height.saturating_sub(7));
        if word == panels::Word::Close {
            pixels = native_close_rect(c.surface.width, c.surface.height);
            starkit::native_surface::style::close_button(
                &mut c.surface,
                pixels,
                &colors.style().colors,
                "close",
                false,
            );
        } else {
            starkit::native_surface::classic::label(
                &mut c.surface,
                pixels,
                word.word(),
                &rack::hex(colors.muted),
                10,
                false,
            );
            native_mnemonic(
                &mut c.surface,
                pixels,
                &word.word(),
                word.mnemonic(),
                hotkeys,
                &rack::hex(
                    colors
                        .title
                        .best_contrast_against(&[starkit::theme::WHITE, starkit::theme::BLACK]),
                ),
            );
        }
    }
    Component::Surface {
        rect: rect.into(),
        surface: c.surface,
    }
}

impl App {
    pub(super) fn rack_drop_target(&self, scene: &mut Scene, regions: &Regions, cell: (u16, u16)) {
        if self.overlays.is_open() || self.places.is_some() || self.tab_picker.is_some() {
            return;
        }
        let Some((row, label)) = self.drop_target_row(regions) else {
            return;
        };
        let target = pixel_bounds(row, cell, &scene.placements);
        if target.width == 0 || target.height == 0 {
            return;
        }
        let mut surface = Surface::new(
            target.width,
            target.height,
            rack::hex(self.theme.row_cursor_bg),
        )
        .with_physical_text();
        starkit::native_surface::classic::label(
            &mut surface,
            R::new(8, 0, target.width.saturating_sub(16), target.height),
            label,
            &rack::hex(self.theme.row_cursor_fg),
            11,
            true,
        );
        scene.components.push(Component::Surface {
            rect: row.into(),
            surface,
        });
    }

    pub(in crate::ui::app) fn rack_queue_rect(&self, regions: &Regions) -> Option<Rect> {
        let stack = regions.rect_of(ModuleId::Stack);
        let area = regions.rect_of(ModuleId::Operations);
        (self.layout.native_rack && stack.width >= 100 && stack.height >= 10 && area.height >= 14)
            .then(|| Rect::new(area.x + 2, area.y + 4, area.width - 4, area.height - 8))
    }

    pub(super) fn rack_background_click(&mut self, x: u16, y: u16) -> bool {
        if self.graphical.as_ref().unwrap().rack_hits.is_empty() {
            return false;
        }
        let Some(regions) = self.layout.last.as_ref() else {
            return false;
        };
        if [
            Bar::Commander(0),
            Bar::Commander(1),
            Bar::Stack,
            Bar::Operations,
            Bar::Preview,
        ]
        .iter()
        .any(|bar| {
            self.bars
                .track_of(*bar)
                .is_some_and(|r| r.contains((x, y).into()))
        }) {
            return false;
        }
        let stack = regions.rect_of(ModuleId::Stack);
        if stack.contains((x, y).into()) {
            let pane = usize::from(self.commander && x >= stack.x + stack.width / 2);
            let rect = if self.commander {
                pane_rect(stack, pane)
            } else {
                stack
            };
            let view = if self.commander {
                self.pane_view(pane)
            } else {
                self.stack_view()
            };
            let split = panels::stack::split(
                starkit::chrome::frame::body(rect, &panels::words(ModuleId::Stack)),
                view.crumbs.len(),
                view.fold_rows,
            );
            if split.list.contains((x, y).into()) || split.crumbs.contains((x, y).into()) {
                return false;
            }
            if self.commander {
                self.focus_pane(pane);
            }
            return true;
        }
        if self.rack_queue_rect(regions).is_some()
            && regions
                .rect_of(ModuleId::Operations)
                .contains((x, y).into())
        {
            self.layout.focus_set(ModuleId::Operations);
            return true;
        }
        if matches!(self.view.preview.as_deref(), Some(Preview::Image { .. }))
            && regions.rect_of(ModuleId::Preview).height >= 14
            && regions.rect_of(ModuleId::Preview).contains((x, y).into())
        {
            self.layout.focus_set(ModuleId::Preview);
            return true;
        }
        self.layout.native_rack && regions.status.contains((x, y).into())
    }
    pub(super) fn rack_click(&mut self, hit: RackHit) {
        match hit {
            RackHit::Action(action) => self.act(action),
            RackHit::Word(word) => {
                if matches!(
                    word,
                    panels::Word::Filter
                        | panels::Word::Actions
                        | panels::Word::Places
                        | panels::Word::Hidden
                ) {
                    self.layout.focus_set(ModuleId::Stack);
                }
                self.word_click(word);
            }
            RackHit::Pane(pane, action) => {
                if self.commander {
                    self.focus_pane(pane);
                }
                self.act(action);
            }
            RackHit::Operation(index) => {
                self.layout.focus_set(ModuleId::Operations);
                self.ops_cursor = index;
            }
        }
    }

    /// Paint the switcher with the exact placement used for pointer projection.
    pub(super) fn workspace_scene(&self, scene: &mut Scene, regions: &Regions, cell: (u16, u16)) {
        use crate::ui::tabs::Hit;
        let Some(rect) = regions.tabs else {
            return;
        };
        let p = &self.theme.rack;
        let mut c = Canvas::new(rect, cell, p, &scene.placements);
        c.surface = p
            .style()
            .surface(c.target.width.max(1), c.target.height.max(1));
        let sidebar = regions.workspace_sidebar;
        let background = starkit::theme::Resolve::core(&self.theme).bg;
        if sidebar {
            c.surface.background = rack::hex(background);
        }
        if sidebar {
            let width = c.surface.width;
            rack::mono(
                &mut c.surface,
                R::new(10, 0, width.saturating_sub(20), 26),
                "WORKSPACES",
                p.muted,
                11,
                true,
            );
        }
        let items = self.tab_items();
        let active = self.core.state().tabs.active().id;
        for (rect, hit) in &self.tab_hits {
            let r = c.pixels(*rect);
            match hit {
                Hit::Tab(id) => {
                    let Some((index, item)) = items.iter().enumerate().find(|(_, i)| i.id == *id)
                    else {
                        continue;
                    };
                    let selected = *id == active;
                    if sidebar {
                        if selected {
                            // A quiet inset highlight leaves the sidebar on
                            // the application's background, without a panel face.
                            c.surface.fill(
                                R::new(
                                    r.x + 6,
                                    r.y + 2,
                                    r.width.saturating_sub(12),
                                    r.height.saturating_sub(4),
                                ),
                                &rack::hex(background.mix(p.accent, 0.05)),
                                4,
                            );
                        }
                        let text = R::new(r.x + 10, r.y, r.width.saturating_sub(20), r.height / 2);
                        rack::mono(
                            &mut c.surface,
                            text,
                            &item.label,
                            if selected { p.foreground } else { p.muted },
                            11,
                            selected,
                        );
                        rack::mono(
                            &mut c.surface,
                            R::new(text.x, r.y + r.height / 2, text.width, r.height / 2),
                            &item.location,
                            p.muted,
                            10,
                            false,
                        );
                    } else {
                        let tile = R::new(
                            r.x + 4,
                            r.y + 4,
                            22.min(r.width),
                            r.height.saturating_sub(8),
                        );
                        if selected {
                            c.surface.fill(tile, &rack::hex(p.accent), 0);
                        }
                        c.surface.text_centered(
                            tile,
                            format!("{:02}", index + 1),
                            &rack::hex(if selected { p.inset } else { p.muted }),
                            11,
                            selected,
                        );
                        rack::mono(
                            &mut c.surface,
                            R::new(r.x + 32, r.y, r.width.saturating_sub(40), r.height),
                            &item.label,
                            if selected { p.foreground } else { p.muted },
                            11,
                            selected,
                        );
                        c.surface.fill(
                            R::new(
                                r.x + r.width.saturating_sub(1),
                                r.y + 7,
                                1,
                                r.height.saturating_sub(14),
                            ),
                            &rack::hex(p.shadow),
                            0,
                        );
                    }
                }
                Hit::NavigationMode => {
                    let label = if self.cfg.ui.workspace_navigation
                        == crate::config::WorkspaceNavigation::Sidebar
                    {
                        if sidebar {
                            "Numbered sessions"
                        } else {
                            "Numbered"
                        }
                    } else {
                        "Sidebar"
                    };
                    if sidebar {
                        rack::mono(
                            &mut c.surface,
                            R::new(r.x + 10, r.y, r.width.saturating_sub(20), r.height),
                            label,
                            p.muted,
                            11,
                            false,
                        );
                    } else {
                        p.style()
                            .tool(&mut c.surface, r, label, "workspace-layout", false);
                    }
                }
                Hit::New | Hit::Manage => {
                    let label = if matches!(hit, Hit::New) {
                        if sidebar {
                            "+ New"
                        } else {
                            "+"
                        }
                    } else {
                        "Manage"
                    };
                    if sidebar {
                        rack::mono(
                            &mut c.surface,
                            R::new(r.x + 10, r.y, r.width.saturating_sub(16), r.height),
                            label,
                            p.muted,
                            11,
                            false,
                        );
                    } else {
                        p.style()
                            .tool(&mut c.surface, r, label, "workspace-control", false);
                    }
                }
                _ => {}
            }
        }
        c.finish(scene, &mut vec![]);
    }

    /// A single header-height strip, sharing the pane title palette and font.
    pub(super) fn footer_scene(&mut self, scene: &mut Scene, regions: &Regions, cell: (u16, u16)) {
        let p = &self.theme.rack;
        let area = regions.status;
        let mut c = Canvas::new(area, cell, p, &scene.placements);
        c.surface = p.style().surface(c.target.width, c.target.height);
        c.surface.background = rack::hex(p.title);
        let width = c.surface.width;
        c.surface
            .fill(R::new(0, 0, width, 1), &rack::hex(p.highlight), 0);
        let columns = |pixels: u16| -> u16 {
            (u32::from(pixels) * u32::from(area.width)).div_ceil(u32::from(width.max(1))) as u16
        };
        let margin = columns(10).max(1);
        let mut right = area.right().saturating_sub(margin);
        for (label, hit, pixels) in [
            ("? Help", RackHit::Action(Action::Help), 72),
            ("/ Filter", RackHit::Word(panels::Word::Filter), 84),
            ("b Places", RackHit::Word(panels::Word::Places), 84),
        ] {
            let span = columns(pixels);
            if right < area.x + columns(240) + span {
                continue;
            }
            right -= span;
            let rect = Rect::new(right, area.y, span, area.height);
            let r = c.pixels(rect);
            rack::mono(&mut c.surface, r, label, p.muted, 11, false);
            c.hits.push((rect, hit));
        }
        let mode = if self.commander {
            if self.active_pane == 0 {
                "LEFT"
            } else {
                "RIGHT"
            }
        } else {
            "FOLD"
        };
        let left = area.x + margin;
        let mode_width = columns(58);
        let mode_rect = c.pixels(Rect::new(left, area.y, mode_width, area.height));
        rack::mono(&mut c.surface, mode_rect, mode, p.foreground, 11, true);
        let status = self.status_view(Instant::now());
        let fresh = status
            .note
            .filter(|(_, _, at)| status.now.duration_since(*at) < status::NOTE_FOR);
        let message = fresh
            .map(|(text, _, _)| text.as_str())
            .unwrap_or_else(|| status.progress.unwrap_or("Ready"));
        let marked = if self.view.marked.is_empty() {
            "0 marked"
        } else {
            &self.view.marked
        };
        let text = format!("{marked}  ·  {message}");
        let start = left + mode_width;
        let r = c.pixels(Rect::new(
            start,
            area.y,
            right.saturating_sub(start + margin),
            area.height,
        ));
        let color = match fresh.map(|(_, level, _)| level) {
            Some(NoteLevel::Error) => self.theme.error,
            Some(NoteLevel::Warning) => self.theme.warn,
            _ => p.muted,
        };
        rack::mono(&mut c.surface, r, text, color, 11, false);
        if fresh.is_none() && status.progress.is_some() {
            c.hits.push((
                Rect::new(
                    start,
                    area.y,
                    right.saturating_sub(start + margin),
                    area.height,
                ),
                RackHit::Action(Action::FocusOperations),
            ));
        }
        let mut hits = vec![];
        c.finish(scene, &mut hits);
        self.graphical.as_mut().unwrap().rack_hits.extend(hits);
    }

    pub(super) fn rack_scene(&mut self, scene: &mut Scene, regions: &Regions, cell: (u16, u16)) {
        self.graphical.as_mut().unwrap().rack_hits.clear();
        let stack = regions.rect_of(ModuleId::Stack);
        if stack.width < 100 || stack.height < 10 {
            return;
        }
        let palette = self.theme.rack.clone();
        let p = &palette;
        let mut hits = vec![];
        for pane in 0..if self.commander { 2 } else { 1 } {
            let rect = if self.commander {
                pane_rect(stack, pane)
            } else {
                stack
            };
            let mut c = Canvas::new(rect, cell, p, &scene.placements);
            c.title(if self.commander {
                if pane == 0 {
                    "FILES · LEFT"
                } else {
                    "FILES · RIGHT"
                }
            } else {
                "FILES"
            });
            let controls = [
                ("Places", RackHit::Word(panels::Word::Places), 10),
                ("Filter", RackHit::Word(panels::Word::Filter), 10),
                ("Actions", RackHit::Word(panels::Word::Actions), 11),
                ("Help", RackHit::Action(Action::Help), 8),
            ];
            let controls = if rect.width >= 70 {
                &controls[..]
            } else {
                &controls[1..]
            };
            let total: u16 = controls.iter().map(|(_, _, width)| width).sum();
            let mut x = rect.right().saturating_sub(total + 1);
            for &(label, hit, width) in controls {
                c.header_action(
                    Rect::new(x, rect.y + 1, width, 1),
                    label,
                    hit,
                    self.native_header_hotkeys(ModuleId::Stack),
                );
                x += width;
            }
            let view = if self.commander {
                self.pane_view(pane)
            } else {
                self.stack_view()
            };
            let body = starkit::chrome::frame::body(rect, &panels::words(ModuleId::Stack));
            let split = panels::stack::split(body, view.crumbs.len(), view.fold_rows);
            let list = split.list;
            for span in &scene.spans {
                if split.crumbs.contains((span.x, span.y).into()) {
                    c.text(
                        Rect::new(span.x, span.y, split.crumbs.right() - span.x, 1),
                        &span.text,
                        false,
                        true,
                    );
                }
            }
            let well_x = rect.x + 1;
            let well_right = rect.right() - 1;
            c.well(Rect::new(
                well_x,
                split.rule.y,
                well_right - well_x,
                list.bottom().saturating_sub(split.rule.y) + 1,
            ));
            let path = if self.commander {
                &self.panes[pane].dir
            } else {
                &self.view.active_dir
            };
            let path_area = Rect::new(
                split.rule.x + 2,
                split.rule.y,
                split
                    .rule
                    .width
                    .saturating_sub(2 + breadcrumb_detail_width(&view)),
                1,
            );
            let crumbs = breadcrumb_slots(path_area, path, &self.view.home);
            let crumb_font = starkit::native_surface::Metrics::from_cell(cell.0, cell.1)
                .font
                .min(11);
            for (index, (r, label, target)) in crumbs.iter().enumerate() {
                let mut px = c.pixels(*r);
                // Preserve the full interior at fractional placement edges.
                px.width = px
                    .width
                    .saturating_add(2)
                    .min(c.surface.width.saturating_sub(px.x));
                starkit::native_surface::classic::label(
                    &mut c.surface,
                    px,
                    label,
                    &rack::hex(if target == path {
                        p.foreground
                    } else {
                        p.muted
                    }),
                    crumb_font,
                    false,
                );
                if index + 1 < crumbs.len() && label != "/" {
                    let gap = c.pixels(Rect::new(r.right(), r.y, 3, 1));
                    c.surface
                        .text_centered(gap, "/", &rack::hex(p.muted), crumb_font, false);
                }
            }
            let editing = (pane == self.active_pane)
                .then_some(self.filter.as_ref())
                .flatten();
            if let Some(input) = editing {
                let mut query = input.text().to_string();
                query.insert(input.cursor(), '│');
                c.fill(split.rule, p.inset);
                c.text(split.rule, format!("FILTER / {query}"), true, false);
                c.hits
                    .push((split.rule, RackHit::Word(panels::Word::Filter)));
            } else if let Some(query) = view.filter {
                c.fill(split.rule, p.inset);
                c.text(split.rule, format!("FILTER / {query}"), true, false);
                c.hits
                    .push((split.rule, RackHit::Word(panels::Word::Filter)));
            }
            // Native typography has a physical size; terminal columns can
            // become much narrower when the font or window geometry changes.
            // Reserve actual pixel space for metadata, then project that
            // allocation back to the same logical row geometry.
            let available = c.pixels(list).width.max(1);
            let columns = |pixels: u16| {
                (u32::from(pixels) * u32::from(list.width)).div_ceil(u32::from(available)) as u16
            };
            let size_width = if available >= 240 { columns(88) } else { 0 };
            let time_width = if available >= 520 { columns(104) } else { 0 };
            let kind_width = if available >= 360 { columns(64) } else { 0 };
            let name_width = list
                .width
                .saturating_sub(size_width + time_width + kind_width);
            let size_x = list.x + name_width;
            let time_x = size_x + size_width;
            let kind_x = time_x + time_width;
            let heading = Rect::new(list.x, list.y - 1, list.width, 1);
            c.fill(
                Rect::new(well_x, heading.y, well_right - well_x, 1),
                p.title,
            );
            c.text(
                Rect::new(list.x, heading.y, name_width, 1),
                "NAME",
                false,
                true,
            );
            if size_width > 0 {
                c.text(
                    Rect::new(size_x, heading.y, size_width, 1),
                    "SIZE",
                    false,
                    true,
                );
            }
            if kind_width > 0 {
                c.text(
                    Rect::new(kind_x, heading.y, kind_width, 1),
                    "TYPE",
                    false,
                    true,
                );
            }
            if time_width > 0 {
                c.text(
                    Rect::new(time_x, heading.y, time_width, 1),
                    "MODIFIED",
                    false,
                    true,
                );
            }
            for (index, row) in view
                .rows
                .iter()
                .enumerate()
                .skip(view.scroll)
                .take(usize::from(list.height))
            {
                let y = list.y + (index - view.scroll) as u16;
                let selected = view.focused && index == view.cursor;
                if selected {
                    c.fill(
                        Rect::new(list.x, y, list.width, 1),
                        p.inset.mix(p.accent, 0.10),
                    );
                }
                let icon = row.icon();
                let marked = row.mark == panels::stack::Mark::Marked;
                c.filename(
                    Rect::new(list.x, y, name_width, 1),
                    format!("{} {icon} {}", if marked { "■" } else { " " }, row.name),
                    selected || marked,
                );
                if size_width > 0 {
                    c.text(Rect::new(size_x, y, size_width, 1), &row.size, false, true);
                }
                if kind_width > 0 {
                    c.text(Rect::new(kind_x, y, kind_width, 1), &row.ext, false, true);
                }
                if time_width > 0 {
                    c.text(Rect::new(time_x, y, time_width, 1), &row.time, false, true);
                }
            }
            if view.loading || view.error.is_some() {
                c.text(
                    list,
                    view.error.unwrap_or("Reading directory…"),
                    false,
                    false,
                );
            }
            let marked = view
                .rows
                .iter()
                .filter(|r| r.mark == panels::stack::Mark::Marked)
                .count();
            c.text(
                Rect::new(list.x, list.bottom(), list.width, 1),
                format!(
                    "{} entries · {marked} marked{}",
                    view.rows.len(),
                    if pane == self.active_pane {
                        " · active"
                    } else {
                        ""
                    }
                ),
                true,
                true,
            );
            let toolbar_y = list.bottom() + 1;
            let mut bx = body.x;
            for (label, action, width) in [
                ("↑ Up", Action::Pop, 8),
                ("Yank", Action::Yank, 8),
                ("Paste", Action::Paste, 9),
                ("Move", Action::QueueMove, 8),
            ] {
                if bx + width <= body.right() {
                    c.button(
                        Rect::new(bx, toolbar_y, width, 1),
                        label,
                        RackHit::Pane(pane, action),
                        false,
                    );
                    bx += width;
                }
            }
            if bx + 10 <= body.right() {
                c.button(
                    Rect::new(bx, toolbar_y, 10, 1),
                    "Hidden",
                    RackHit::Word(panels::Word::Hidden),
                    self.view.show_hidden,
                );
            }
            c.text(
                Rect::new(body.x, toolbar_y + 1, body.width, 1),
                view.space
                    .map(|(_, free)| {
                        format!(
                            "{} free · {}",
                            crate::fold::format::size(free),
                            home_relative(path, &self.view.home)
                        )
                    })
                    .unwrap_or_else(|| home_relative(path, &self.view.home)),
                false,
                true,
            );
            c.finish(scene, &mut hits);
        }
        self.rack_operations(scene, regions, cell, &mut hits);
        self.rack_inspection(scene, regions, cell, &mut hits);
        if let Some(tabs) = regions
            .tabs
            .filter(|r| !regions.workspace_sidebar && r.height >= 9)
        {
            let row = Rect::new(tabs.x + 1, tabs.bottom() - 2, 13, 2);
            let mut c = Canvas::new(row, cell, p, &scene.placements);
            let mut x = row.x;
            for (label, action, width) in [
                ("↑", Action::Pop, 4),
                ("⌂", Action::GoHome, 4),
                ("↻", Action::Reload, 4),
            ] {
                c.button(
                    Rect::new(x, row.y, width, 2),
                    label,
                    RackHit::Action(action),
                    false,
                );
                x += width;
            }
            c.finish(scene, &mut hits);
            let row = Rect::new(tabs.right() - 16, tabs.bottom() - 2, 15, 2);
            let mut c = Canvas::new(row, cell, p, &scene.placements);
            c.button(
                row,
                if self.commander { "Commander" } else { "Fold" },
                RackHit::Action(Action::ToggleView),
                true,
            );
            c.finish(scene, &mut hits);
        }
        self.graphical.as_mut().unwrap().rack_hits = hits;
    }

    fn rack_operations(
        &mut self,
        scene: &mut Scene,
        regions: &Regions,
        cell: (u16, u16),
        hits: &mut Vec<(Rect, RackHit)>,
    ) {
        let area = regions.rect_of(ModuleId::Operations);
        if area.height < 14 {
            return;
        }
        let p = &self.theme.rack;
        let mut c = Canvas::new(area, cell, p, &scene.placements);
        c.title("Operations");
        let view = self.operations_view();
        let (fraction, estimate) = {
            let state = self.core.state();
            let running = state.queue.iter().find(|op| op.status == OpStatus::Running);
            let fraction = running.map(|op| op.progress.fraction()).unwrap_or_else(|| {
                if !view.rows.is_empty()
                    && view
                        .rows
                        .iter()
                        .all(|r| r.tone == panels::operations::Tone::Done)
                {
                    1.0
                } else {
                    0.0
                }
            });
            let estimate = running
                .filter(|op| matches!(op.kind, OpKind::Copy | OpKind::Move))
                .and_then(|op| op.progress.transfer_estimate());
            (fraction, estimate)
        };
        let summary = estimate
            .map(|(rate, eta)| {
                format!(
                    "{} operations  ·  {}/s{}",
                    view.rows.len(),
                    crate::fold::format::size(rate),
                    eta.map(|eta| format!(
                        "  ·  {}:{:02} remaining",
                        eta.as_secs() / 60,
                        eta.as_secs() % 60
                    ))
                    .unwrap_or_default()
                )
            })
            .unwrap_or_else(|| format!("{} operations", view.rows.len()));
        c.text(
            Rect::new(area.x + 2, area.y + 2, area.width - 4, 1),
            summary,
            false,
            true,
        );
        let queue = self.rack_queue_rect(regions).unwrap();
        c.text(
            Rect::new(queue.x, queue.y - 1, queue.width, 1),
            if view.paused {
                "Queue · paused"
            } else {
                "Queue"
            },
            false,
            true,
        );
        c.well(queue);
        let mut y = queue.y;
        for activity in [view.incoming.as_deref(), view.unmounting]
            .into_iter()
            .flatten()
        {
            if y + 2 > queue.bottom() {
                break;
            }
            c.text(
                Rect::new(queue.x + 1, y, queue.width - 2, 2),
                activity,
                true,
                true,
            );
            y += 2;
        }
        for (index, row) in view.rows.iter().enumerate().skip(view.scroll) {
            if y + 2 > queue.bottom() {
                break;
            }
            let r = Rect::new(queue.x + 1, y, queue.width - 2, 2);
            if view.focused && view.cursor == index {
                c.fill(r, p.inset.mix(p.accent, 0.10));
            }
            c.text(Rect::new(r.x, y, r.width, 1), &row.title, true, true);
            c.text(Rect::new(r.x, y + 1, r.width, 1), &row.status, false, true);
            c.hits.push((r, RackHit::Operation(index)));
            y += 2;
        }
        if view.rows.is_empty() && view.incoming.is_none() && view.unmounting.is_none() {
            c.text(queue, "No queued operations", false, true);
        }
        c.meter(
            Rect::new(area.x + 2, area.bottom() - 4, area.width - 11, 1),
            fraction,
        );
        c.text(
            Rect::new(area.right() - 8, area.bottom() - 4, 6, 1),
            format!("{:.0}%", fraction * 100.0),
            true,
            true,
        );
        let mut x = area.x + 2;
        for (label, action, width) in [
            ("Stop", Action::CancelRun, 9),
            ("Resume", Action::RunQueue, 10),
            ("Clear", Action::ClearQueue, 9),
        ] {
            c.button(
                Rect::new(x, area.bottom() - 3, width, 2),
                label,
                RackHit::Action(action),
                false,
            );
            x += width;
        }
        let total = view.rows.len() as u32;
        let above = view.scroll as u32;
        let visible = (queue.height / 2)
            .saturating_sub(
                u16::from(view.incoming.is_some()) + u16::from(view.unmounting.is_some()),
            )
            .max(1);
        c.finish(scene, hits);
        self.bars.record_viewport(
            Bar::Operations,
            Rect::new(queue.right() - 1, queue.y, 1, queue.height),
            total,
            above,
            u32::from(visible),
        );
    }

    fn rack_inspection(
        &self,
        scene: &mut Scene,
        regions: &Regions,
        cell: (u16, u16),
        hits: &mut Vec<(Rect, RackHit)>,
    ) {
        let area = regions.rect_of(ModuleId::Preview);
        if area.height < 14
            || !self.layout.preview_open
            || self.audio_here()
            || self.editor.is_some()
        {
            return;
        }
        let Some(Preview::Image {
            width,
            height,
            format,
            ..
        }) = self.view.preview.as_deref()
        else {
            return;
        };
        let p = &self.theme.rack;
        let mut c = Canvas::new(area, cell, p, &scene.placements);
        c.title("INSPECTION / SELECTED FILE");
        let art = image_rect(area);
        c.well(art);
        let info = Rect::new(
            art.right() + 2,
            art.y,
            area.right().saturating_sub(art.right() + 4),
            art.height,
        );
        c.text(
            Rect::new(info.x, info.y, info.width, 2),
            self.view.preview_name.as_deref().unwrap_or("Image"),
            true,
            false,
        );
        c.text(
            Rect::new(info.x, info.y + 2, info.width, 1),
            format!("{} · {width} × {height}", format.to_uppercase()),
            false,
            true,
        );
        let row = self
            .view
            .rows
            .iter()
            .find(|r| Some(r.name.trim_end_matches('/')) == self.view.preview_name.as_deref());
        for (i, (label, value)) in [
            (
                "SIZE",
                row.map(|r| r.size.clone()).unwrap_or_else(|| "—".into()),
            ),
            (
                "MODIFIED",
                row.map(|r| r.time.clone()).unwrap_or_else(|| "—".into()),
            ),
            ("SCALE", format!("{:?}", self.cfg.preview.image_scale)),
        ]
        .into_iter()
        .enumerate()
        {
            let y = info.y + 4 + i as u16 * 2;
            if y + 2 <= info.bottom() {
                c.text(Rect::new(info.x, y, info.width, 1), label, false, true);
                c.text(Rect::new(info.x, y + 1, info.width, 1), value, true, true);
            }
        }
        c.button(
            Rect::new(area.x + 2, area.bottom() - 3, 12, 2),
            "Open",
            RackHit::Action(Action::OpenExternal),
            false,
        );
        c.button(
            Rect::new(area.x + 15, area.bottom() - 3, 10, 2),
            "Yank",
            RackHit::Action(Action::Yank),
            false,
        );
        c.button(
            Rect::new(area.x + 26, area.bottom() - 3, 10, 2),
            "Scale",
            RackHit::Word(panels::Word::PictureScale(self.cfg.preview.image_scale)),
            false,
        );
        c.button(
            Rect::new(area.right() - 12, area.bottom() - 3, 10, 2),
            "Close",
            RackHit::Word(panels::Word::Close),
            false,
        );
        c.finish(scene, hits);
    }
}
