//! Shared graphical scene adapter over the established controller and hit geometry.
use super::*;
use starkit::terminal_graphics::{
    protocol::{Component, Input, Scene, ServerMessage, Span, Viewport},
    session::Controller,
};
#[path = "rack_scene.rs"]
mod rack_scene;
use rack_scene::RackHit;

#[derive(Default)]
pub(super) struct State {
    pub effects: Vec<ServerMessage>,
    pub output_pending: bool,
    pub(super) cell_mode: bool,
    pub(super) presentation_switch: bool,
    pub(super) animated_images: bool,
    pub(super) surface_mode: bool,
    pub(super) skin_mode: bool,
    padded_chrome: bool,
    pub(super) pixel_layout: bool,
    pub(super) base_buffer: Option<Buffer>,
    pub(super) base_scrollbars: Vec<Component>,
    placements: Vec<starkit::terminal_graphics::placement::Placement>,
    rack_hits: Vec<(Rect, RackHit)>,
    viewport: Option<Viewport>,
    preserved_preview: Option<(u16, u16)>,
    pointer_capture: Option<starkit::terminal_graphics::placement::Placement>,
    video_capable: bool,
    video_player: bool,
    original_media: bool,
    video_mode: u8,
    video_fullscreen: Option<bool>,
    video_overlay_at: Option<Instant>,
    video_overlay_visible: bool,
    video_picker: Option<bool>,
    video_picker_index: usize,
    video_picker_rect: Option<Rect>,
    video_picker_controls: Option<crate::video_transport::Client>,
    video_picker_request: Option<crate::video_transport::Request>,
    video_subtitle_input: Option<String>,
    direct_play: Option<PathBuf>,
    direct_fullscreen: bool,
    local_media: bool,
    pub(super) audio_relay_capable: bool,
    pub(super) audio_local: bool,
    pub(super) audio_epoch: Option<u64>,
    video: Option<starkit::terminal_graphics::media::Host>,
    video_sequence: u64,
    loading_phase: Option<usize>,
    video_unmuted_volume: Option<u8>,
    video_scrub: Option<VideoScrub>,
    video_path: Option<PathBuf>,
    video_pending: Option<PathBuf>,
    video_expanded: Option<Option<u16>>,
    video_timeline: Option<Rect>,
    video_controls: Option<crate::video_transport::Client>,
    video_control_rect: Option<Rect>,
    video_control_request: Option<crate::video_transport::Request>,
    image_source: Option<Arc<RgbaImage>>,
    image_document: Option<(String, u64)>,
    image_sequence: u64,
    pub(super) image_zoom: Option<u16>,
    image_id: Option<String>,
    thumbnail: Option<starkit::terminal_graphics::assets::Thumbnailer>,
    image: Option<(String, String)>,
    audio_images: std::collections::HashMap<String, String>,
    rendered_version: u64,
    drag: Option<Drag>,
    resize_handle: Option<starkit::native_surface::PixelRect>,
    preview_resize: Option<PreviewResize>,
    authorization: Option<Authorization>,
    pub(super) wire: Option<crossbeam_channel::Receiver<wire_dnd::Outgoing>>,
}
impl State {
    pub(super) fn can_play_video(&self) -> bool {
        self.video_capable
    }
    pub(super) fn drag_sources(&self) -> Option<(usize, Vec<PathBuf>)> {
        self.drag.as_ref().map(|d| (d.stack, d.sources.clone()))
    }
    pub(super) fn drag_regions(&self) -> Option<&Regions> {
        self.drag.as_ref().and_then(|drag| drag.regions.as_ref())
    }
    pub(super) fn uses_pixel_layout(&self) -> bool {
        self.pixel_layout && self.padded_chrome && self.surface_mode && !self.cell_mode
    }
}
#[derive(Clone, Copy)]
struct PreviewResize {
    origin_y: i32,
    rows: i32,
    max_rows: i32,
    row_pixels: i32,
}
struct Drag {
    regions: Option<Regions>,
    origin: (u16, u16),
    stack: usize,
    sources: Vec<PathBuf>,
    started: bool,
    desktop: bool,
}
struct Authorization {
    op: OpId,
    child: std::process::Child,
    input: std::process::ChildStdin,
    secret: Vec<u8>,
}
impl Drop for Authorization {
    fn drop(&mut self) {
        self.secret.fill(0);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct VideoScrub {
    track: Rect,
    position: f64,
    paused: bool,
}

fn hex(rgb: starkit::theme::color::Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b)
}
fn color(c: starkit::ratatui::style::Color, fallback: &str) -> String {
    match c {
        starkit::ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.into(),
    }
}
/// Pixel presentation keeps controller cells as logical coordinates only. The
/// renderer and pointer adapter share these placements, including modal layers.
fn pixel_placements(
    regions: &Regions,
    _commander: bool,
    v: Viewport,
    modals: &[Rect],
    overlay: &[Span],
) -> Vec<starkit::terminal_graphics::placement::Placement> {
    use starkit::native_surface::{Metrics, PixelRect as R};
    use starkit::terminal_graphics::placement::{self, Placement};
    let cw = (v.width / u32::from(v.columns)).max(1) as u16;
    let ch = (v.height / u32::from(v.rows)).max(1) as u16;
    let metrics = Metrics::from_cell(cw, ch);
    // Kitty owns the space outside the terminal grid. Use the full app
    // background, keeping padding inside panels rather than around the frame.
    let area = R::new(0, 0, v.width as u16, v.height as u16);
    let mut sources = Vec::new();
    if let Some(tabs) = regions.tabs {
        sources.push(tabs);
    }
    let flexible = sources.len();
    let preview = regions.rect_of(ModuleId::Preview);
    let operations = regions.rect_of(ModuleId::Operations);
    let deck = preview.y == operations.y;
    sources.push(regions.rect_of(ModuleId::Stack));
    sources.push(if deck {
        Rect::new(preview.x, preview.y, regions.area.width, preview.height)
    } else {
        preview
    });
    if !deck {
        sources.push(operations);
    }
    sources.push(regions.status);
    let heights: Vec<_> = sources
        .iter()
        .map(|r| {
            if Some(*r) == regions.tabs {
                // Comfortable label/control height without an extra outer top margin.
                if r.height >= 5 {
                    r.height.saturating_mul(ch)
                } else {
                    ch.saturating_add(20).max(36)
                }
            } else if r.height == 2 {
                metrics.control
            } else {
                r.height.saturating_mul(ch)
            }
        })
        .collect();
    let mut targets = placement::column(area, metrics.gap, &heights, flexible);
    if regions.tabs.is_some() && targets.len() > flexible {
        // Keep the mockup’s breathing room between the filled tabs and pane.
        let previous_bottom = targets[flexible - 1].y + targets[flexible - 1].height;
        let pane = &mut targets[flexible];
        let gap = pane.y.saturating_sub(previous_bottom).saturating_sub(6);
        pane.y -= gap;
        pane.height = pane.height.saturating_add(gap);
    }
    let mut result = Vec::new();
    for (i, (source, target)) in sources.into_iter().zip(targets).enumerate() {
        if source.width == 0 || source.height == 0 || target.width == 0 || target.height == 0 {
            continue;
        }
        if deck && i == flexible + 1 {
            for (source, target) in [preview, operations]
                .into_iter()
                .zip(placement::split(target, metrics.gap))
            {
                result.push(Placement::new(source.into(), target));
            }
        } else {
            result.push(Placement::new(source.into(), target));
        }
    }
    for p in &mut result {
        if p.source.height >= 6
            && Some(p.source) != regions.tabs.map(Into::into)
            && p.source != regions.status.into()
        {
            p.padding = Some(placement::PanelPadding {
                inset: metrics.inset,
                gap: metrics.gap,
            });
        }
    }
    for source in modals {
        if source.width == 0 || source.height == 0 {
            continue;
        }
        let parent = result
            .iter()
            .rev()
            .find(|p| p.source.contains(source.x, source.y));
        let (x, y) = parent
            .map(|p| {
                (
                    p.target.x
                        + (u32::from(source.x - p.source.x) * u32::from(p.target.width)
                            / u32::from(p.source.width)) as u16,
                    p.target.y + p.row_edge(source.y - p.source.y).round() as u16,
                )
            })
            .unwrap_or((source.x.saturating_mul(cw), source.y.saturating_mul(ch)));
        let width = source.width.saturating_mul(cw).min(v.width as u16);
        let height = source.height.saturating_mul(ch).min(v.height as u16);
        result.push(Placement {
            source: (*source).into(),
            target: R::new(
                // Mouse reports identify terminal cells. A menu row straddling
                // two cells can otherwise make its visible Copy action hit Move.
                (x.min(v.width as u16 - width) / cw) * cw,
                (y.min(v.height as u16 - height) / ch) * ch,
                width,
                height,
            ),
            padding: None,
            overlay: Some(
                overlay
                    .iter()
                    .filter(|s| s.y >= source.y && s.y < source.bottom())
                    .cloned()
                    .collect(),
            ),
        });
    }
    result
}

pub(super) fn scrollbar(track: Rect, thumb: starkit::chrome::scrollbar::Thumb) -> Component {
    Component::Scrollbar {
        rect: track.into(),
        thumb: starkit::terminal_graphics::Rect {
            x: track.x,
            y: track.y.saturating_add(thumb.start),
            width: track.width,
            height: thumb.len,
        },
    }
}

fn key_code(code: &str) -> Option<KeyCode> {
    if let Some(text) = code.strip_prefix("char:") {
        let mut chars = text.chars();
        let c = chars.next()?;
        return chars.next().is_none().then_some(KeyCode::Char(c));
    }
    if let Some(number) = code.strip_prefix("f:") {
        return number.parse().ok().map(KeyCode::F);
    }
    Some(match code {
        "enter" => KeyCode::Enter,
        "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        _ => return None,
    })
}
impl App {
    fn native_header_hotkeys(&self, module: ModuleId) -> bool {
        self.filter.is_none()
            && self.places.is_none()
            && self.tab_picker.is_none()
            && !self.overlays.is_open()
            && !self.g_pending
            && self.editor.is_none()
            && (module == ModuleId::Stack || self.layout.focus() == module)
    }
    pub(crate) fn enable_graphical(&mut self) {
        let (tx, rx) = crossbeam_channel::bounded(256);
        wire_dnd::graphical_output(tx);
        self.graphical = Some(State {
            wire: Some(rx),
            surface_mode: true,
            ..State::default()
        });
    }
    /// Capture divider gestures before row, scrollbar, or file-drop hit testing.
    fn graphical_preview_resize(
        &mut self,
        action: &str,
        button: u8,
        pixel: Option<[u32; 2]>,
        x: u16,
        y: u16,
    ) -> bool {
        let state = self.graphical.as_ref().unwrap();
        let Some(v) = state.viewport else {
            return false;
        };
        let [px, py] = pixel.unwrap_or([
            (u32::from(x) * 2 + 1) * v.width / (u32::from(v.columns).max(1) * 2),
            (u32::from(y) * 2 + 1) * v.height / (u32::from(v.rows).max(1) * 2),
        ]);
        if let Some(capture) = state.preview_resize {
            if matches!(action, "drag" | "up") {
                let delta = (capture.origin_y - py as i32) / capture.row_pixels;
                let rows = (capture.rows + delta).clamp(2, capture.max_rows.max(2)) as u16;
                if self.layout.native_preview_rows != Some(rows) {
                    self.layout.native_preview_rows = Some(rows);
                    self.repaint = true;
                }
                if action == "up" {
                    self.graphical.as_mut().unwrap().preview_resize = None;
                }
            }
            return true;
        }
        if action != "down"
            || button != 0
            || state.drag.is_some()
            || self.bars.held().is_some()
            || self.overlays.is_open()
            || self.places.is_some()
            || self.tab_picker.is_some()
        {
            return false;
        }
        let hit = state.resize_handle.is_some_and(|r| {
            px >= u32::from(r.x)
                && px < u32::from(r.x) + u32::from(r.width)
                && py >= u32::from(r.y)
                && py < u32::from(r.y) + u32::from(r.height)
        });
        if !hit {
            return false;
        }
        let Some(regions) = &self.layout.last else {
            return false;
        };
        let rows = regions
            .rect_of(ModuleId::Preview)
            .height
            .saturating_sub(layout::COLLAPSED_ROWS + starkit::chrome::frame::extra_rows());
        let max_rows =
            rows.saturating_add(regions.rect_of(ModuleId::Stack).height.saturating_sub(10));
        self.graphical.as_mut().unwrap().preview_resize = Some(PreviewResize {
            origin_y: py as i32,
            rows: i32::from(rows),
            max_rows: i32::from(max_rows),
            row_pixels: (v.height / u32::from(v.rows).max(1)).max(1) as i32,
        });
        true
    }

    fn graphical_pointer(&mut self, action: &str, button: u8, x: u16, y: u16, modifiers: u8) {
        if action == "down" && self.video_words().is_some() {
            tracing::debug!(x, y, button, preview = ?self.layout.last.as_ref().map(|r| r.rect_of(ModuleId::Preview)), words = ?self.layout.last.as_ref().map(|r| starkit::chrome::header::slots(r.rect_of(ModuleId::Preview), &self.panel_words(ModuleId::Preview))), "Video preview pointer");
        }
        if self.video_pointer(action, button, x, y) {
            return;
        }
        if action == "down"
            && button == 0
            && modifiers == 0
            && !self.overlays.is_open()
            && self.places.is_none()
            && self.tab_picker.is_none()
            && self
                .graphical
                .as_ref()
                .is_some_and(|g| g.uses_pixel_layout())
        {
            let hit = self
                .graphical
                .as_ref()
                .unwrap()
                .rack_hits
                .iter()
                .find(|(r, _)| r.contains((x, y).into()))
                .map(|(_, hit)| *hit);
            if let Some(hit) = hit {
                self.rack_click(hit);
                self.repaint = true;
                return;
            }
            if let Some(regions) = self.layout.last.as_ref() {
                let pane = if self.commander {
                    usize::from(
                        x >= regions.rect_of(ModuleId::Stack).x
                            + regions.rect_of(ModuleId::Stack).width / 2,
                    )
                } else {
                    0
                };
                let rect = if self.commander {
                    pane_rect(regions.rect_of(ModuleId::Stack), pane)
                } else {
                    regions.rect_of(ModuleId::Stack)
                };
                let view = if self.commander {
                    self.pane_view(pane)
                } else {
                    self.stack_view()
                };
                let body = starkit::chrome::frame::body(rect, &panels::words(ModuleId::Stack));
                let mut rule = panels::stack::split(body, view.crumbs.len(), view.fold_rows).rule;
                rule.x += 2;
                rule.width = rule
                    .width
                    .saturating_sub(2 + breadcrumb_detail_width(&view));
                let path = if self.commander {
                    &self.panes[pane].dir
                } else {
                    &self.view.active_dir
                };
                if let Some((_, _, target)) = breadcrumb_slots(rule, path, &self.view.home)
                    .into_iter()
                    .find(|(r, _, _)| r.contains((x, y).into()))
                {
                    let current = path == &target;
                    if self.commander {
                        self.focus_pane(pane);
                    }
                    if !current {
                        self.core.send(Command::Push(target));
                    }
                    self.repaint = true;
                    return;
                }
            }
            if self.rack_background_click(x, y) {
                self.repaint = true;
                return;
            }
        }
        if modifiers == starkit::crossterm::event::KeyModifiers::CONTROL.bits()
            && matches!(action, "scroll_up" | "scroll_down")
            && self
                .layout
                .last
                .as_ref()
                .is_some_and(|r| r.hit(x, y) == Some(ModuleId::Preview))
            && matches!(self.view.preview.as_deref(), Some(Preview::Image { .. }))
            && !self.overlays.is_open()
            && self.places.is_none()
            && self.tab_picker.is_none()
        {
            self.zoom_preview_image(if action == "scroll_up" { 1 } else { -1 });
            return;
        }
        let button = match button {
            1 => MouseButton::Right,
            2 => MouseButton::Middle,
            _ => MouseButton::Left,
        };
        let modifiers = starkit::crossterm::event::KeyModifiers::from_bits_truncate(modifiers);
        if action == "down"
            && button == MouseButton::Left
            && modifiers.is_empty()
            && self.places.is_none()
            && self.tab_picker.is_none()
            && !self.overlays.is_open()
            && self.bars.held().is_none()
        {
            if let Some((stack, sources)) = self.dnd_sources(i32::from(x), i32::from(y)) {
                self.graphical.as_mut().unwrap().drag = Some(Drag {
                    regions: self.layout.last.clone(),
                    origin: (x, y),
                    stack,
                    sources,
                    started: false,
                    desktop: false,
                });
            }
        }
        if self.dnd.drag_active && matches!(action, "scroll_up" | "scroll_down") {
            self.dnd_scroll_destination(x, y, if action == "scroll_down" { 3 } else { -3 });
            return;
        }
        // Once Kitty takes ownership, its OSC 72 release is authoritative.
        // A parallel ordinary mouse release must not queue another copy.
        if self
            .graphical
            .as_ref()
            .unwrap()
            .drag
            .as_ref()
            .is_some_and(|d| d.desktop)
        {
            return;
        }
        if action == "drag" || action == "up" || action.starts_with("scroll") {
            if let Some(mut drag) = self.graphical.as_mut().unwrap().drag.take() {
                if matches!(action, "drag" | "up") && (drag.started || drag.origin != (x, y)) {
                    drag.started = true;
                    self.dnd.drag_active = true;
                    self.dnd.offer = Some(wire_dnd::Offer {
                        sources: drag.sources.clone(),
                        uri_text: String::new(),
                        source_stack: drag.stack,
                        source_tab: self.core.state().tabs.active().id,
                    });
                    self.dnd.hover = self.dnd_target(i32::from(x), i32::from(y));
                    self.dnd.hover_coords = Some((x, y));
                    self.dnd.hover_allowed = 1;
                }
                if action == "up" {
                    if drag.started {
                        if let Some(dest) = self.dnd_target(i32::from(x), i32::from(y)) {
                            self.dnd.choice = Some(wire_dnd::Choice {
                                dest,
                                own_sources: Some(drag.sources),
                                internal: true,
                                allowed: 3,
                                mime_index: None,
                                remote: false,
                                terminal_drop_open: false,
                            });
                            self.overlays.open_drop((x, y));
                            self.repaint = true;
                        }
                        self.dnd.drag_active = false;
                        self.dnd.offer = None;
                        self.dnd.hover = None;
                        self.dnd.hover_coords = None;
                        self.dnd_edge = None;
                        return;
                    }
                } else {
                    let consumed = drag.started;
                    self.graphical.as_mut().unwrap().drag = Some(drag);
                    if consumed {
                        return;
                    }
                }
            }
        }
        let kind = match action {
            "down" => MouseEventKind::Down(button),
            "up" => MouseEventKind::Up(button),
            "drag" => MouseEventKind::Drag(button),
            "move" => MouseEventKind::Moved,
            "scroll_up" => MouseEventKind::ScrollUp,
            "scroll_down" => MouseEventKind::ScrollDown,
            "scroll_left" => MouseEventKind::ScrollLeft,
            "scroll_right" => MouseEventKind::ScrollRight,
            _ => return,
        };
        self.mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers,
        });
    }
    fn graphical_tree(
        &mut self,
        scene: &mut Scene,
        regions: &Regions,
        viewport: Viewport,
        modal: bool,
    ) {
        if !self.graphical.as_ref().unwrap().uses_pixel_layout()
            || !self.layout.preview_open
            || self.editor.is_some()
        {
            return;
        }
        let Some(Preview::Dir(tree)) = self.view.preview.as_deref() else {
            return;
        };
        let rows = panels::preview::dir_lines(tree);
        let outer = regions.rect_of(ModuleId::Preview);
        let content = panels::preview::content_rect(outer);
        if content.width == 0 || content.height == 0 {
            return;
        }
        let placements = pixel_placements(regions, self.commander, viewport, &[], &[]);
        let Some(p) = placements.iter().find(|p| p.source == outer.into()) else {
            return;
        };
        let height = (p.row_edge(content.bottom() - outer.y) - p.row_edge(content.y - outer.y))
            .floor()
            .max(1.) as u16;
        let width = (f32::from(content.width) * f32::from(p.target.width)
            / f32::from(p.source.width))
        .floor()
        .max(1.) as u16;
        let cell = (
            (viewport.width / u32::from(viewport.columns).max(1)).max(1) as u16,
            (viewport.height / u32::from(viewport.rows).max(1)).max(1) as u16,
        );
        let (surface, scroll, visible) = native_tree_surface(
            &rows,
            self.preview_scroll,
            (width, height),
            cell,
            &self.theme,
        );
        self.preview_scroll = scroll;
        scene.components.push(Component::Surface {
            rect: content.into(),
            surface,
        });
        if !modal {
            if let Some(track) = self.bars.track_of(Bar::Preview) {
                self.bars.record_viewport(
                    Bar::Preview,
                    track,
                    rows.len() as u32,
                    scroll as u32,
                    visible as u32,
                );
            }
        }
    }

    fn graphical_image(&mut self, scene: &mut Scene, regions: &Regions) {
        if self.editor.is_some() || self.audio_here() || !self.layout.preview_open {
            return;
        }
        let details = self.movie_details_preview();
        let showing_details = details.is_some();
        let pdf =
            matches!(self.view.preview.as_deref(), Some(Preview::Document(d)) if d.kind == "PDF");
        let document = match self.view.preview.as_deref() {
            Some(Preview::Document(d)) if d.kind == "PDF" => d
                .extension
                .as_ref()
                .map(|info| (info.provider.clone(), info.session)),
            _ => None,
        };
        let source = match details.as_ref().or(self.view.preview.as_deref()) {
            Some(Preview::Video { poster, .. }) => Some(Arc::clone(&poster.pixels)),
            Some(Preview::Image { data, .. }) => Some(Arc::clone(data)),
            Some(Preview::Document(d)) => d.image.clone(),
            _ => None,
        };
        let movie_body = video_image_body(
            regions.rect_of(ModuleId::Preview),
            self.graphical.as_ref().unwrap().uses_pixel_layout(),
        );
        let rack_image = self.graphical.as_ref().unwrap().uses_pixel_layout()
            && regions.rect_of(ModuleId::Stack).width >= 100
            && regions.rect_of(ModuleId::Stack).height >= 18
            && regions.rect_of(ModuleId::Preview).height >= 14
            && matches!(self.view.preview.as_deref(), Some(Preview::Image { .. }));
        let state = self.graphical.as_mut().unwrap();
        if let Some((id, png)) = state
            .thumbnail
            .as_ref()
            .and_then(|worker| worker.output.try_iter().last())
        {
            if state.image_id.as_ref() == Some(&id) {
                state.image = Some((id, png));
            }
        }
        let changed = match (&state.image_source, &source) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        if changed {
            // Older frontends receive individual frames. Keep the last ready
            // frame visible while its replacement is encoded on the worker.
            let same_document = document.is_some() && document == state.image_document;
            if !same_document && (state.animated_images || self.preview_animation.is_none()) {
                state.image = None;
            }
            state.image_document = document;
            state.image_id = None;
            state.image_source = source.clone();
            if let Some(source) = source {
                state.image_sequence += 1;
                let id = format!("preview-{}", state.image_sequence);
                state.image_id = Some(id.clone());
                let worker = state.thumbnail.get_or_insert_with(Default::default);
                if let Some((animation, _, _)) = &self.preview_animation {
                    if state.animated_images {
                        worker.request_animation(id, Arc::clone(animation));
                    } else {
                        worker.request(id, source);
                    }
                } else if pdf {
                    worker.request_raster(id, source);
                } else {
                    worker.request(id, source);
                }
            }
        }

        if let Some(video) = &state.video {
            let rect = movie_body;
            scene.components.push(Component::Image {
                rect: rect.into(),
                id: format!("video-{}-{}", video.session, video.generation),
                png: None,
                zoom: 100,
                scale: starkit::terminal_graphics::protocol::ImageScale::Smooth,
            });
            return;
        }
        if let Some((id, png)) = &state.image {
            let mut rect = panels::preview::content_rect(regions.rect_of(ModuleId::Preview));
            if rack_image {
                rect = rack_scene::image_rect(regions.rect_of(ModuleId::Preview));
            } else if pdf {
                // Start at the top of the document body; reserve only the
                // bottom caption row, matching the cell reader's placement.
                rect = panels::preview::body(regions.rect_of(ModuleId::Preview));
                rect.height = rect.height.saturating_sub(1);
            } else if showing_details {
                rect = panels::preview::movie_card_rects(rect).0;
            } else if matches!(self.view.preview.as_deref(), Some(Preview::Video { .. })) {
                rect = movie_body;
            }
            scene.components.push(Component::Image {
                rect: rect.into(),
                id: id.clone(),
                png: Some(png.clone()),
                zoom: if showing_details || pdf {
                    100
                } else {
                    state.image_zoom.unwrap_or(100)
                },
                scale: match if showing_details || pdf {
                    crate::config::Scale::Smooth
                } else {
                    self.cfg.preview.image_scale
                } {
                    crate::config::Scale::One => {
                        starkit::terminal_graphics::protocol::ImageScale::One
                    }
                    crate::config::Scale::Pixels => {
                        starkit::terminal_graphics::protocol::ImageScale::Pixels
                    }
                    crate::config::Scale::Smooth => {
                        starkit::terminal_graphics::protocol::ImageScale::Smooth
                    }
                },
            });
        }
    }
}
fn native_tree_surface(
    rows: &[String],
    scroll: usize,
    size: (u16, u16),
    cell: (u16, u16),
    theme: &Theme,
) -> (starkit::native_surface::Surface, usize, usize) {
    use starkit::native_surface::{Metrics, PixelRect as R, Primitive, Surface};
    let (width, height) = size;
    let font = Metrics::from_cell(cell.0, cell.1).font;
    let row_height = ((f32::from(font) * 1.2).ceil() as u16 + 1).max(1);
    let visible = usize::from(height / row_height).clamp(1, 128);
    let scroll = scroll.min(rows.len().saturating_sub(visible));
    let mut surface = Surface::new(width, height, hex(theme.panel_bg));
    let branch_color = hex(theme.dim);
    for (index, line) in rows.iter().skip(scroll).take(visible).enumerate() {
        let y = index as u16 * row_height;
        let h = row_height.min(height.saturating_sub(y));
        if h == 0 {
            break;
        }
        let mut x = 0u16;
        let mut prefix = 0;
        for c in line.chars() {
            if !matches!(c, '│' | '├' | '└' | '─' | ' ') || x + cell.0 > width {
                break;
            }
            let center = x + cell.0 / 2;
            let middle = y + h / 2;
            if matches!(c, '│' | '├' | '└') {
                surface.fill(
                    R::new(center, y, 1, if c == '└' { h / 2 + 1 } else { h }),
                    &branch_color,
                    0,
                );
            }
            if matches!(c, '├' | '└' | '─') {
                let start = if c == '─' { x } else { center };
                surface.fill(
                    R::new(start, middle, x + cell.0 - start, 1),
                    &branch_color,
                    0,
                );
            }
            prefix += c.len_utf8();
            x += cell.0;
        }
        if x < width {
            surface.nodes.push(Primitive::Text {
                rect: R::new(x, y, width - x, h),
                text: line[prefix..].into(),
                color: hex(theme.row_fg),
                size: font,
                bold: false,
                mono: true,
            });
        }
    }
    (surface, scroll, visible)
}

fn breadcrumb_detail(view: &panels::stack::View<'_>) -> String {
    format!(
        "{}{}",
        if view.truncated { " (truncated)" } else { "" },
        view.filter.map(|f| format!("  /{f}")).unwrap_or_default()
    )
}
fn breadcrumb_detail_width(view: &panels::stack::View<'_>) -> u16 {
    starkit::wrap::width_of(&breadcrumb_detail(view))
}

/// Breadcrumb layout is shared by painting and pointer hit testing.
fn breadcrumb_slots(
    area: Rect,
    path: &std::path::Path,
    home: &std::path::Path,
) -> Vec<(Rect, String, PathBuf)> {
    let mut parts = if crate::fold::location::is_archive(path) {
        let mut locations = vec![];
        let mut location = crate::fold::location::Location::from_key(path).ok();
        while let Some(current) = location {
            let key = current.key();
            let label = if key == home {
                "~".into()
            } else {
                key.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "/".into())
            };
            locations.push((label, key.clone()));
            if key == home {
                break;
            }
            location = current.parent();
        }
        locations.reverse();
        locations
    } else {
        let (mut target, first, rest) = if let Ok(rest) = path.strip_prefix(home) {
            (home.to_path_buf(), "~".to_owned(), rest)
        } else {
            (
                PathBuf::from("/"),
                "/".to_owned(),
                path.strip_prefix("/").unwrap_or(path),
            )
        };
        let mut parts = vec![(first, target.clone())];
        for part in rest.components() {
            target.push(part.as_os_str());
            parts.push((
                part.as_os_str().to_string_lossy().into_owned(),
                target.clone(),
            ));
        }
        parts
    };
    let width = |parts: &[(String, PathBuf)]| -> usize {
        parts
            .iter()
            .map(|(label, _)| usize::from(starkit::wrap::width_of(label)))
            .sum::<usize>()
            + parts
                .iter()
                .take(parts.len().saturating_sub(1))
                .map(|(label, _)| if label == "/" { 1 } else { 3 })
                .sum::<usize>()
    };
    let mut collapsed = false;
    while parts.len() > 1 && width(&parts) + usize::from(collapsed) * 4 > usize::from(area.width) {
        parts.remove(0);
        collapsed = true;
    }
    if collapsed && area.width > 4 {
        let parent = crate::fold::location::Location::from_key(&parts[0].1)
            .ok()
            .and_then(|l| l.parent())
            .map(|l| l.key())
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        parts.insert(0, ("…".into(), parent));
    }
    let mut x = area.x;
    let mut slots = Vec::new();
    for (label, path) in parts {
        let width = starkit::wrap::width_of(&label).min(area.right().saturating_sub(x));
        if width == 0 {
            break;
        }
        let gap = if label == "/" { 1 } else { 3 };
        slots.push((
            Rect::new(x, area.y, width, 1),
            panels::fit(&label, width),
            path,
        ));
        x = x.saturating_add(width).saturating_add(gap);
    }
    slots
}

// Pixel movies have their own details/timeline below the picture.
// Keep the legacy metadata row for clients using cell geometry.
fn video_image_body(area: Rect, pixel_layout: bool) -> Rect {
    let mut body = video_body(area, pixel_layout);
    let controls = body.height >= 6;
    body.height = body.height.saturating_sub(if controls { 4 } else { 2 });
    // Match the header's gap with a row above playback details.
    if pixel_layout && controls {
        body.height = body.height.saturating_sub(1);
    }
    body
}

fn balance_video_padding(p: &mut starkit::terminal_graphics::placement::Placement) {
    if let Some(padding) = &mut p.padding {
        if p.source.height >= 8 {
            // The movie panel adds four pixels to the shared inset, and
            // the controls add another four pixels of border spacing.
            padding.inset = padding.inset.saturating_add(8);
            padding.gap = ((f32::from(p.target.height) - 2. * f32::from(padding.inset))
                / f32::from(p.source.height - 2))
            .round()
            .max(1.) as u16;
        }
    }
}

fn video_body(area: Rect, pixel_layout: bool) -> Rect {
    if pixel_layout {
        let mut body = panels::preview::body(area);
        // Pixel placement already supplies the bottom border inset. The
        // cell frame's extra blank footer row would add a second inset.
        if area.height >= 8 && starkit::chrome::frame::extra_rows() > 0 {
            body.height = body.height.saturating_add(1);
        }
        body
    } else {
        panels::preview::content_rect(area)
    }
}

fn native_header(
    rect: Rect,
    title: &str,
    words: &[panels::Word],
    theme: &Theme,
    cell: (u16, u16),
    hotkeys: bool,
    title_padding: u16,
) -> Component {
    let (cw, ch) = cell;
    use starkit::chrome::header::{self, Word as _};
    use starkit::native_surface::{Metrics, PixelRect as R, Surface};
    let area = Rect::new(
        rect.x + 1,
        rect.y + 1,
        rect.width.saturating_sub(2),
        1.min(rect.height.saturating_sub(2)),
    );
    let mut surface = Surface::new(
        area.width * cw,
        area.height.max(1) * ch,
        hex(theme.header_bg),
    );
    let font = Metrics::from_cell(cw, ch).font;
    let slots = header::slots(rect, words);
    let end = slots.iter().map(|(_, r)| r.x).min().unwrap_or(area.right());
    let inset = 12u16
        .saturating_sub(cw)
        .max(4)
        .saturating_add(title_padding);
    let title_width = end
        .saturating_sub(area.x)
        .saturating_mul(cw)
        .saturating_sub(inset + 8);
    surface
        .nodes
        .push(starkit::native_surface::Primitive::Text {
            rect: R::new(inset, 0, title_width, ch),
            text: starkit::text::truncate(title, usize::from(title_width / cw.max(1))),
            color: hex(theme.header_fg),
            size: font,
            bold: true,
            mono: true,
        });
    let title_hit_width = (starkit::wrap::width_of(title) * cw).min(title_width);
    if title_hit_width > 0 {
        surface.hits.push(starkit::native_surface::HitRegion {
            rect: R::new(inset, 0, title_hit_width, ch),
            action: "focus".into(),
        });
    }
    let shortcut = theme
        .rack
        .button
        .best_contrast_against(&[starkit::theme::WHITE, starkit::theme::BLACK]);
    for (word, slot) in slots {
        super::super::rack::bevel(
            &mut surface,
            R::new(
                (slot.x - area.x) * cw,
                1,
                slot.width * cw,
                ch.saturating_sub(2),
            ),
            &theme.rack,
            false,
        );
        surface.hits.push(starkit::native_surface::HitRegion {
            rect: R::new((slot.x - area.x) * cw, 0, slot.width * cw, ch),
            action: "header".into(),
        });
        // Use the same character advances as header hit testing. This keeps
        // inter-item spacing equal and mnemonic positions exact at every scale.
        for (offset, character) in word
            .word()
            .chars()
            .take(usize::from(slot.width))
            .enumerate()
        {
            let highlighted = hotkeys && word.mnemonic() == Some((character, offset as u16));
            surface
                .nodes
                .push(starkit::native_surface::Primitive::Text {
                    rect: R::new((slot.x - area.x + offset as u16) * cw, 0, cw, ch),
                    text: character.to_string(),
                    color: hex(if highlighted {
                        shortcut
                    } else {
                        theme
                            .rack
                            .foreground
                            .ensure_contrast(theme.rack.button, 4.5)
                    }),
                    size: font,
                    bold: highlighted,
                    mono: true,
                });
        }
    }
    Component::Surface {
        rect: area.into(),
        surface,
    }
}

impl Controller for App {
    fn supports_presentation_switch(&self) -> bool {
        true
    }
    fn presentation(&mut self, cells: bool) {
        let state = self.graphical.as_mut().unwrap();
        state.preserved_preview = self
            .layout
            .last
            .as_ref()
            .zip(state.viewport)
            .map(|(r, v)| (r.rect_of(ModuleId::Preview).height, v.rows));
        state.cell_mode = cells;
        state.placements.clear();
        state.rack_hits.clear();
        state.pointer_capture = None;
        state.base_buffer = None;
        state.base_scrollbars.clear();
        state.resize_handle = None;
        state.preview_resize = None;
        self.layout.last = None;
        self.extension_viewport_sent = None;
        self.bars.release();
        self.repaint = true;
        self.graphics
            .set_mode(if cells { Mode::Blocks } else { Mode::Off });
    }
    fn frame_interval(&self) -> Duration {
        let graphics = self.graphical.as_ref().unwrap();
        if graphics.rendered_version != self.seen_version
            || (self.layout.preview_open
                && self.editor.is_none()
                && !self.audio_here()
                && graphics
                    .thumbnail
                    .as_ref()
                    .is_some_and(|worker| !worker.output.is_empty()))
        {
            return Duration::ZERO;
        }
        let busy = (self.layout.preview_open
            && self.preview_animation.is_some()
            && !self.native_animation_playback())
            || self.view.loading
            || self.movie_loading_stage().is_some()
            || self.overlays.is_open()
            || self.places.is_some()
            || self.tab_picker.is_some()
            || self.graphical.as_ref().unwrap().video.is_some()
            || self.audio_path.is_some()
            || self.editor.is_some()
            || self.dnd.drag_active
            || self.graphical.as_ref().unwrap().authorization.is_some()
            || self.view.ops.iter().any(|op| {
                matches!(
                    op.tone,
                    panels::operations::Tone::Running | panels::operations::Tone::Pending
                )
            });
        if busy {
            Duration::from_millis(if std::env::var_os("SSH_CONNECTION").is_some() {
                67
            } else {
                33
            })
        } else {
            Duration::from_secs(1)
        }
    }
    fn output_pending(&mut self, pending: bool) {
        self.graphical.as_mut().unwrap().output_pending = pending;
    }
    fn attached(&mut self) {
        self.dnd.enabled = false;
        self.suspend_video();
        self.graphical.as_mut().unwrap().video_capable = false;
        self.graphical.as_mut().unwrap().cell_mode = false;
        self.graphical.as_mut().unwrap().animated_images = false;
        self.graphical.as_mut().unwrap().surface_mode = false;
        self.graphical.as_mut().unwrap().skin_mode = false;
        self.graphical.as_mut().unwrap().pixel_layout = false;
        self.graphics.set_mode(Mode::Off);
    }
    fn capabilities(
        &mut self,
        capabilities: starkit::terminal_graphics::capabilities::Capabilities,
    ) {
        if (!capabilities.audio_relay || capabilities.local_media)
            && self.graphical.as_ref().is_some_and(|s| s.audio_local)
        {
            let _ = self.audio.set_audio_local(false);
            self.close_audio_relay();
            self.graphical.as_mut().unwrap().audio_local = false;
        }
        if capabilities.audio_relay && !capabilities.local_media && !self.audio_here() {
            if let Err(error) = self.audio.set_audio_local(true) {
                self.audio_error = Some(error);
            } else {
                self.graphical.as_mut().unwrap().audio_local = true;
            }
        }
        // A new presentation connection needs a fresh audio epoch and credits.
        if capabilities.audio_relay && self.graphical.as_ref().is_some_and(|s| s.audio_local) {
            self.close_audio_relay();
            if let Err(error) = self.audio.set_audio_local(true) {
                self.audio_error = Some(error);
            }
        }
        self.graphical.as_mut().unwrap().video_capable = capabilities.video;
        self.graphical.as_mut().unwrap().video_player = capabilities.video_player;
        self.graphical.as_mut().unwrap().original_media = capabilities.original_media;
        self.graphical.as_mut().unwrap().local_media = capabilities.local_media;
        self.graphical.as_mut().unwrap().audio_relay_capable =
            capabilities.audio_relay && !capabilities.local_media;
        self.graphical.as_mut().unwrap().presentation_switch = capabilities.presentation_switch;
        let cells = capabilities.cell_presentation.unwrap_or(
            capabilities.image_transport
                == starkit::terminal_graphics::capabilities::ImageTransport::None,
        );
        self.graphical.as_mut().unwrap().cell_mode = cells;
        let state = self.graphical.as_mut().unwrap();
        if state.animated_images != capabilities.animated_images {
            state.image_source = None;
        }
        state.animated_images = capabilities.animated_images;
        self.graphical.as_mut().unwrap().surface_mode = capabilities.native_surfaces;
        self.graphical.as_mut().unwrap().skin_mode =
            capabilities.native_surfaces && capabilities.native_skins;
        self.graphical.as_mut().unwrap().pixel_layout = capabilities.pixel_layout;
        self.graphics
            .set_mode(if cells { Mode::Blocks } else { Mode::Off });
    }
    fn detached(&mut self) {
        self.suspend_video();
        self.graphical.as_mut().unwrap().video_capable = false;
        self.save_workspace(true);
        self.graphical.as_mut().unwrap().authorization = None;
        if let Some(op) = self.dnd.import_op {
            self.core.send(Command::Cancel(op));
        }
        if let Some(op) = self.dnd.export_op {
            self.core.send(Command::Cancel(op));
        }
        Controller::input(self, Input::CancelPointer);
    }
    fn tick(&mut self) {
        let _chrome =
            starkit::chrome::frame::padding_scope(self.graphical.as_ref().unwrap().padded_chrome);
        let _rack = panels::stack::rack_scope(
            self.layout.native_rack && self.layout.last.as_ref().is_some_and(layout::rack_browser),
        );
        App::tick(self);
        self.tick_video();
        let phase = self
            .movie_loading_stage()
            .map(|_| (self.animation_started.elapsed().as_millis() / 120) as usize);
        if self.graphical.as_ref().unwrap().loading_phase != phase {
            self.graphical.as_mut().unwrap().loading_phase = phase;
            self.repaint = true;
        }
        if let Some(op) = self.pending_elevated_delete.take() {
            // No controlling TTY exists on a headless SSH host. -S uses stdin
            // while retaining sudo's parent-process timestamp scope, shared by
            // this process's subsequent narrowly scoped -n delete workers.
            let child = std::process::Command::new("sudo")
                .args(["-S", "-v", "-p", ""])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            match child {
                Ok(mut child) => {
                    if let Some(input) = child.stdin.take() {
                        self.graphical.as_mut().unwrap().authorization = Some(Authorization {
                            op,
                            child,
                            input,
                            secret: vec![],
                        });
                    }
                }
                Err(error) => {
                    self.note = Some((
                        format!("Administrator authorization failed: {error}"),
                        NoteLevel::Error,
                        Instant::now(),
                    ))
                }
            }
        }
        let result = self
            .graphical
            .as_mut()
            .unwrap()
            .authorization
            .as_mut()
            .and_then(|auth| auth.child.try_wait().ok().flatten());
        if let Some(status) = result {
            let auth = self
                .graphical
                .as_mut()
                .unwrap()
                .authorization
                .take()
                .unwrap();
            if status.success() {
                self.core.send(Command::QueueElevatedDelete(auth.op));
            } else {
                self.note = Some((
                    "Administrator authorization was cancelled or denied".into(),
                    NoteLevel::Error,
                    Instant::now(),
                ));
            }
        }
    }
    fn scene(&mut self, viewport: Viewport) -> Scene {
        if self
            .graphical
            .as_ref()
            .unwrap()
            .viewport
            .is_some_and(|old| old != viewport)
        {
            Controller::input(self, Input::CancelPointer);
        }
        let state = self.graphical.as_mut().unwrap();
        state.placements.clear();
        state.viewport = Some(viewport);
        // Keep compact chrome at the established terminal floor.
        state.padded_chrome = !state.cell_mode
            && state.surface_mode
            && viewport.rows
                >= (layout::MIN_ROWS + 8)
                    .saturating_add(self.cfg.ui.padding_y.max(1).saturating_mul(2));
        // Native rows have independent cursor/mark indicators, so their fill
        // can stay subtle while labels retain readable text contrast.
        self.theme.set_graphical(!state.cell_mode);
        let _chrome = starkit::chrome::frame::padding_scope(state.padded_chrome);
        if let Some((height, rows)) = state.preserved_preview.take() {
            self.layout.native_preview_rows = Some(
                ((u32::from(height) * u32::from(viewport.rows) / u32::from(rows.max(1))) as u16)
                    .saturating_sub(layout::COLLAPSED_ROWS + starkit::chrome::frame::extra_rows()),
            );
        }
        self.audio_cell_size = Some((
            (viewport.width / u32::from(viewport.columns)).clamp(1, 64) as u16,
            (viewport.height / u32::from(viewport.rows)).clamp(1, 128) as u16,
        ));
        if self.graphical.as_ref().unwrap().video_fullscreen.is_some() {
            return self.fullscreen_video_scene(viewport);
        }
        let area = Rect::new(0, 0, viewport.columns, viewport.rows);
        let mut buffer = Buffer::empty(area);
        self.graphical.as_mut().unwrap().base_buffer = None;
        self.graphical.as_mut().unwrap().base_scrollbars.clear();
        self.draw(area, &mut buffer);
        self.graphical.as_mut().unwrap().rendered_version = self.seen_version;
        let base_buffer = self.graphical.as_mut().unwrap().base_buffer.take();
        let background = base_buffer.as_ref().unwrap_or(&buffer);
        let base_scrollbars = std::mem::take(&mut self.graphical.as_mut().unwrap().base_scrollbars);
        // Only pixel layers carry popup spans separately. Cell/legacy painters
        // still receive the complete foreground buffer, while row styling uses
        // the unoccluded snapshot whenever one is available.
        let pixels = self.graphical.as_ref().unwrap().uses_pixel_layout();
        let mut scene = Scene::from_buffer(if pixels { background } else { &buffer }, viewport, 0);
        scene.accent = hex(self.theme.accent);
        scene.border = hex(self.theme.border);
        let Some(regions) = self.layout.last.clone() else {
            return scene;
        };
        let _rack = panels::stack::rack_scope(pixels && layout::rack_browser(&regions));
        use std::hash::{Hash, Hasher};
        let mut targets = std::collections::hash_map::DefaultHasher::new();
        self.view.active_dir.hash(&mut targets);
        self.view.frame_id.hash(&mut targets);
        self.view.op_ids.hash(&mut targets);
        for pane in &self.panes {
            pane.dir.hash(&mut targets);
            pane.key.hash(&mut targets);
        }
        let mut scroll_targets = targets.clone();
        self.core.state().tabs.active().id.hash(&mut scroll_targets);
        self.audio_path.hash(&mut scroll_targets);
        self.audio_here().hash(&mut scroll_targets);
        let modal = self.overlays.is_open() || self.places.is_some() || self.tab_picker.is_some();
        let mut modal_rects = self.overlays.graphical_rects(area);
        if modal_rects.is_empty() {
            if self.places.is_some() {
                modal_rects.push(super::super::places::rect(area));
            } else if let Some(picker) = self.tab_picker.as_mut() {
                modal_rects = picker.graphical_rects(area);
            }
        }
        let native = !self.graphical.as_ref().unwrap().cell_mode
            && self.graphical.as_ref().unwrap().surface_mode;
        let cw = (viewport.width / u32::from(viewport.columns)).max(1) as u16;
        let ch = (viewport.height / u32::from(viewport.rows)).max(1) as u16;
        if native && self.graphical.as_ref().unwrap().padded_chrome {
            for module in [ModuleId::Stack, ModuleId::Preview, ModuleId::Operations] {
                if module == ModuleId::Stack {
                    continue;
                }
                let rect = regions.rect_of(module);
                if module == ModuleId::Operations && rect.height == 2 {
                    use starkit::native_surface::{Metrics, PixelRect as R, Surface};
                    let mut surface =
                        Surface::new(rect.width * cw, rect.height * ch, hex(self.theme.panel_bg));
                    let text = if let Some(progress) = &self.view.running_bar {
                        let title = self
                            .view
                            .ops
                            .iter()
                            .find(|row| row.tone == panels::operations::Tone::Running)
                            .map(|row| row.title.as_str())
                            .unwrap_or("select to inspect");
                        format!("Operations · {progress} · {title}")
                    } else if let Some(row) = self
                        .view
                        .ops
                        .iter()
                        .find(|row| row.tone == panels::operations::Tone::Running)
                        .or_else(|| {
                            self.view
                                .ops
                                .iter()
                                .rev()
                                .find(|row| row.tone == panels::operations::Tone::Failed)
                        })
                    {
                        format!(
                            "Operations · {} · {} · select to inspect",
                            row.status, row.title
                        )
                    } else if self.dnd.receiving_uri {
                        "Operations · receiving file list… · select to inspect".into()
                    } else if let Some(name) = &self.view.unmounting {
                        format!("Operations · unmounting {name} · select to inspect")
                    } else if self.view.ops.is_empty() {
                        "Operations · idle".to_string()
                    } else {
                        format!(
                            "Operations · {} items · select to inspect",
                            self.view.ops.len()
                        )
                    };
                    let text_width =
                        (starkit::wrap::width_of(&text) * cw).min(surface.width.saturating_sub(24));
                    if text_width > 0 {
                        surface.hits.push(starkit::native_surface::HitRegion {
                            rect: R::new(12.min(surface.width), 0, text_width, surface.height),
                            action: "operations".into(),
                        });
                    }
                    surface.text(
                        R::new(
                            12.min(surface.width),
                            0,
                            surface.width.saturating_sub(24),
                            surface.height,
                        ),
                        text,
                        &hex(self.theme.dim),
                        Metrics::from_cell(cw, ch).font,
                        false,
                    );
                    scene.components.push(Component::Surface {
                        rect: rect.into(),
                        surface,
                    });
                } else {
                    scene.components.push(Component::Panel {
                        rect: rect.into(),
                        active: self.layout.focus() == module,
                    });
                    let title = if module == ModuleId::Preview {
                        if self.audio_here() {
                            "STAR/AMP".to_string()
                        } else {
                            format!(
                                "Preview · {}",
                                self.view.preview_name.as_deref().unwrap_or("")
                            )
                        }
                    } else {
                        "Operations".into()
                    };
                    let words = if module == ModuleId::Operations {
                        let view = self.operations_view();
                        panels::operations::header_words(view.paused, view.active, view.focused)
                    } else {
                        self.panel_words(module)
                    };
                    scene.components.push(native_header(
                        rect,
                        &title,
                        &words,
                        &self.theme,
                        (cw, ch),
                        self.native_header_hotkeys(module),
                        0,
                    ));
                }
            }
            scene.components.push(Component::Surface {
                rect: regions.status.into(),
                surface: status::native_surface(
                    regions.status,
                    &self.status_view(Instant::now()),
                    cw,
                    ch,
                ),
            });
        }
        if !native || !self.graphical.as_ref().unwrap().padded_chrome {
            for module in [ModuleId::Preview, ModuleId::Operations] {
                scene.components.push(Component::Panel {
                    rect: regions.rect_of(module).into(),
                    active: self.layout.focus() == module,
                });
            }
        }
        {
            let pane_count = if self.commander { self.panes.len() } else { 1 };
            for pane in 0..pane_count {
                let rect = if self.commander {
                    pane_rect(regions.rect_of(ModuleId::Stack), pane)
                } else {
                    regions.rect_of(ModuleId::Stack)
                };
                let view = if self.commander {
                    self.pane_view(pane)
                } else {
                    self.stack_view()
                };
                let body = starkit::chrome::frame::body(rect, &panels::words(ModuleId::Stack));
                let list = panels::stack::split(body, view.crumbs.len(), view.fold_rows).list;
                // Existing column arithmetic determines metadata boundaries.
                let width = panels::stack::graphical_name_width(list.width);
                scene.components.push(Component::Panel {
                    rect: rect.into(),
                    active: view.focused,
                });
                if native && self.graphical.as_ref().unwrap().padded_chrome {
                    // Proportional fonts give spaces a narrower advance than
                    // letters; keep the separator visibly separated as in TUI.
                    let brand = panels::HEADING.replace(" / ", "  /  ");
                    let title = if self.commander && regions.tabs.is_some_and(|r| r.height >= 5) {
                        if pane == 0 {
                            "COMMANDER · LEFT".into()
                        } else {
                            "COMMANDER · RIGHT".into()
                        }
                    } else if self.commander {
                        if pane == 0 {
                            format!("{brand} · Left")
                        } else {
                            "Right".into()
                        }
                    } else {
                        brand
                    };
                    scene.components.push(native_header(
                        rect,
                        &title,
                        &self.panel_words(ModuleId::Stack),
                        &self.theme,
                        (cw, ch),
                        self.native_header_hotkeys(ModuleId::Stack),
                        0,
                    ));
                    if self.graphical.as_ref().unwrap().uses_pixel_layout() {
                        use starkit::native_surface::{Metrics, PixelRect as R, Surface};
                        let rule =
                            panels::stack::split(body, view.crumbs.len(), view.fold_rows).rule;
                        let mut area = rule;
                        area.x += 2;
                        area.width = area
                            .width
                            .saturating_sub(2 + breadcrumb_detail_width(&view));
                        let path = if self.commander {
                            &self.panes[pane].dir
                        } else {
                            &self.view.active_dir
                        };
                        let slots = breadcrumb_slots(area, path, &self.view.home);
                        let mut surface =
                            Surface::new(rule.width * cw, ch, hex(self.theme.panel_bg));
                        for (index, (r, label, target)) in slots.iter().enumerate() {
                            let mut glyphs: Vec<(String, u16)> = Vec::new();
                            for c in label.chars() {
                                let text = c.to_string();
                                let width = starkit::wrap::width_of(&text);
                                if width == 0 {
                                    if let Some((text, _)) = glyphs.last_mut() {
                                        text.push(c);
                                    }
                                } else {
                                    glyphs.push((text, width));
                                }
                            }
                            let mut offset = 0;
                            for (text, width) in glyphs {
                                surface
                                    .nodes
                                    .push(starkit::native_surface::Primitive::Text {
                                        rect: R::new(
                                            (r.x - rule.x + offset) * cw,
                                            0,
                                            width * cw,
                                            ch,
                                        ),
                                        text,
                                        color: hex(self.theme.fold.crumb_active_fg),
                                        size: Metrics::from_cell(cw, ch).font,
                                        bold: target == path,
                                        mono: true,
                                    });
                                offset += width;
                            }
                            if index + 1 < slots.len() {
                                surface.text(
                                    R::new((r.right() - rule.x) * cw, 0, 3 * cw, ch),
                                    if label == "/" { " " } else { " / " },
                                    &hex(self.theme.dim),
                                    Metrics::from_cell(cw, ch).font,
                                    false,
                                );
                            }
                        }
                        let detail = breadcrumb_detail(&view);
                        let detail_width = starkit::wrap::width_of(&detail).min(rule.width);
                        if detail_width > 0 {
                            surface.text(
                                R::new((rule.width - detail_width) * cw, 0, detail_width * cw, ch),
                                detail,
                                &hex(self.theme.dim),
                                Metrics::from_cell(cw, ch).font,
                                false,
                            );
                        }
                        scene.components.push(Component::Surface {
                            rect: rule.into(),
                            surface,
                        });
                    }
                    if let Some((total, available)) = view.space.filter(|(total, _)| *total > 0) {
                        use starkit::native_surface::{Metrics, PixelRect as R, Surface};
                        let footer = Rect::new(
                            rect.x + 1,
                            rect.bottom() - 2,
                            rect.width.saturating_sub(2),
                            1,
                        );
                        let mut surface =
                            Surface::new(footer.width * cw, ch, hex(self.theme.panel_bg));
                        let inset = 12u16.saturating_sub(cw).max(4);
                        let width = 96.min(surface.width / 4);
                        let top = ch.saturating_sub(4) / 2;
                        surface.fill(
                            R::new(inset, top, width, 4.min(ch)),
                            &hex(self.theme.border),
                            2,
                        );
                        let used = (u128::from(total.saturating_sub(available.min(total)))
                            * u128::from(width)
                            / u128::from(total)) as u16;
                        surface.fill(
                            R::new(inset, top, used, 4.min(ch)),
                            &hex(self.theme.fold.progress_fg),
                            2,
                        );
                        let x = inset + width + 12;
                        let text = format!(
                            "{} free / {} total",
                            crate::fold::format::size(available),
                            crate::fold::format::size(total)
                        );
                        surface.text(
                            R::new(x, 0, surface.width.saturating_sub(x + 8), ch),
                            text,
                            &hex(self.theme.dim),
                            Metrics::from_cell(cw, ch).font,
                            false,
                        );
                        scene.components.push(Component::Surface {
                            rect: footer.into(),
                            surface,
                        });
                    }
                }
                if !view.loading && view.error.is_none() {
                    for (index, row) in view
                        .rows
                        .iter()
                        .enumerate()
                        .skip(view.scroll)
                        .take(usize::from(list.height))
                    {
                        let y = list.y + (index - view.scroll) as u16;
                        let cell = &background[(list.x, y)];
                        let name_cell = &background[(
                            list.x.saturating_add(3).min(list.right().saturating_sub(1)),
                            y,
                        )];
                        let foreground = color(name_cell.fg, &hex(self.theme.fg));
                        let background = color(cell.bg, &hex(self.theme.panel_bg));
                        let icon = match row.kind {
                            panels::stack::Kind::Dir => "folder",
                            panels::stack::Kind::Symlink { .. } => "link",
                            _ => match row.ext.as_str() {
                                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => "image",
                                "mp3" | "flac" | "wav" | "ogg" => "audio",
                                "mp4" | "mkv" | "webm" => "video",
                                "zip" | "tar" | "gz" | "rar" | "7z" => "archive",
                                _ => "file",
                            },
                        };
                        scene.components.push(Component::ListRow {
                            rect: starkit::terminal_graphics::Rect {
                                x: list.x,
                                y,
                                width,
                                height: 1,
                            },
                            label: row.name.clone(),
                            icon: icon.into(),
                            foreground,
                            background,
                            selected: view.focused && index == view.cursor,
                            marked: row.mark == panels::stack::Mark::Marked,
                            marking: row.mark != panels::stack::Mark::None,
                        });
                    }
                }
            }
            let items = self.tab_items();
            let active = self.core.state().tabs.active().id;
            for (rect, hit) in &self.tab_hits {
                let control = match hit {
                    super::super::tabs::Hit::Previous => Some("‹"),
                    super::super::tabs::Hit::Next => Some("›"),
                    super::super::tabs::Hit::New => Some("+"),
                    _ => None,
                };
                if let Some(label) = control {
                    scene.components.push(Component::Tab {
                        rect: (*rect).into(),
                        label: label.into(),
                        number: None,
                        active: false,
                        close: None,
                    });
                }
                if let super::super::tabs::Hit::Tab(id) = hit {
                    if let Some((index, item)) =
                        items.iter().enumerate().find(|(_, item)| item.id == *id)
                    {
                        scene.components.push(Component::Tab {
                            rect: (*rect).into(),
                            label: item.label.clone(),
                            number: Some(index as u32 + 1),
                            active: *id == active,
                            close: self.tab_hits.iter().find_map(|(rect, hit)| match hit {
                                super::super::tabs::Hit::Close(close_id) if close_id == id => {
                                    Some((*rect).into())
                                }
                                _ => None,
                            }),
                        });
                    }
                }
            }
        }
        if pixels {
            use super::super::rack;
            // Keep the Panel/Tab markers for stable interaction identities;
            // the native chrome paints over their legacy outlines and tabs.
            let components = std::mem::take(&mut scene.components);
            if let Some(tabs) = regions.tabs.filter(|r| r.height >= 5) {
                let view = if self.commander {
                    self.pane_view(self.active_pane)
                } else {
                    self.stack_view()
                };
                let count = self.core.state().selection.len();
                let mode = if self.commander {
                    if self.active_pane == 0 {
                        "LEFT PANE · COMMANDER"
                    } else {
                        "RIGHT PANE · COMMANDER"
                    }
                } else {
                    "FOLD"
                };
                scene.components.push(rack::workspace(
                    tabs,
                    (cw, ch),
                    &self.theme.rack,
                    rack::Workspace {
                        count,
                        marked: &self.view.marked,
                        location: &home_relative(&self.view.active_dir, &self.view.home),
                        mode,
                        space: view.space,
                    },
                ));
            }
            for component in components {
                match &component {
                    Component::Panel { rect, .. } => {
                        let rect = Rect::new(rect.x, rect.y, rect.width, rect.height);
                        scene.components.push(component);
                        scene
                            .components
                            .extend(rack::frame(rect, (cw, ch), &self.theme.rack));
                    }
                    Component::Tab {
                        rect,
                        label,
                        number,
                        active,
                        close,
                    } => {
                        let convert = |r: starkit::terminal_graphics::Rect| {
                            Rect::new(r.x, r.y, r.width, r.height)
                        };
                        let painted = rack::tab(
                            convert(*rect),
                            label,
                            *number,
                            *active,
                            close.map(convert),
                            (cw, ch),
                            &self.theme.rack,
                        );
                        scene.components.push(component);
                        scene.components.push(painted);
                    }
                    _ => scene.components.push(component),
                }
            }
        }
        self.graphical_tree(&mut scene, &regions, viewport, modal);
        if pixels {
            self.rack_scene(&mut scene, &regions, (cw, ch));
        }
        self.video_picker_scene(&mut scene);
        // Draw pixel scrollbars from the exact geometry used for pointer grabs.
        // Add before modal chrome so menus can cover the underlying track.
        let current_scrollbars: Vec<_> = self
            .bars
            .visible()
            .map(|(track, thumb)| scrollbar(track, thumb))
            .collect();
        let overlay_scrollbars: Vec<_> = if base_buffer.is_some() {
            scene.components.extend(base_scrollbars.iter().cloned());
            current_scrollbars
                .into_iter()
                .filter(|bar| !base_scrollbars.contains(bar))
                .collect()
        } else {
            scene.components.extend(current_scrollbars);
            Vec::new()
        };
        for component in &scene.components {
            match component {
                Component::ListRow { rect, label, .. } => {
                    (rect.x, rect.y, rect.width, rect.height).hash(&mut targets);
                    label.hash(&mut targets);
                }
                Component::Tab { rect, label, .. } => {
                    (rect.x, rect.y, rect.width, rect.height).hash(&mut targets);
                    label.hash(&mut targets);
                }
                _ => {}
            }
        }
        if modal {
            for rect in &modal_rects {
                for y in rect.y..rect.bottom().min(area.bottom()) {
                    for x in rect.x..rect.right().min(area.right()) {
                        buffer[(x, y)].symbol().hash(&mut targets);
                    }
                }
            }
        }
        scene.interaction = targets.finish();
        let extension_rect = panels::preview::content_rect(regions.rect_of(ModuleId::Preview));
        self.extension_viewport(
            extension_rect,
            (
                (viewport.width / u32::from(viewport.columns).max(1)).max(1) as u16,
                (viewport.height / u32::from(viewport.rows).max(1)).max(1) as u16,
            ),
        );
        self.graphical_image(&mut scene, &regions);
        if self.layout.preview_open {
            if let Some(Preview::Document(d)) = self.view.preview.as_deref() {
                if let Some(surface) = d
                    .surface
                    .as_ref()
                    .filter(|_| !self.graphical.as_ref().unwrap().cell_mode)
                {
                    scene.components.push(Component::Surface {
                        rect: extension_rect.into(),
                        surface: surface.as_ref().clone(),
                    });
                }
            }
        }
        if self.audio_graphics_config().is_some() {
            if let Some(frame) = &self.audio_frame {
                use std::hash::{Hash, Hasher};
                let body = audio_body(regions.rect_of(ModuleId::Preview));
                if let Some(surface) = &frame.surface {
                    scene.components.push(Component::Surface {
                        rect: body.into(),
                        surface: surface.clone(),
                    });
                }
                let cache = &mut self.graphical.as_mut().unwrap().audio_images;
                let mut retained = std::collections::HashSet::new();
                for image in frame.images.iter().take(5) {
                    if image.x.saturating_add(image.width) > body.width
                        || image.y.saturating_add(image.height) > body.height
                    {
                        continue;
                    }
                    let mut hash = std::collections::hash_map::DefaultHasher::new();
                    image.rgba.hash(&mut hash);
                    (image.pixel_width, image.pixel_height).hash(&mut hash);
                    let id = format!("player-{:x}", hash.finish());
                    if !cache.contains_key(&id) {
                        if let Some(pixels) = RgbaImage::from_raw(
                            u32::from(image.pixel_width),
                            u32::from(image.pixel_height),
                            image.rgba.clone(),
                        ) {
                            if let Ok(png) = starkit::terminal_graphics::assets::encode_png(&pixels)
                            {
                                cache.insert(id.clone(), png);
                            }
                        }
                    }
                    if let Some(png) = cache.get(&id) {
                        scene.components.push(Component::Image {
                            rect: starkit::terminal_graphics::Rect {
                                x: body.x + image.x,
                                y: body.y + image.y,
                                width: image.width,
                                height: image.height,
                            },
                            id: id.clone(),
                            png: Some(png.clone()),
                            scale: Default::default(),
                            zoom: 100,
                        });
                        retained.insert(id);
                    }
                }
                cache.retain(|id, _| retained.contains(id));
            }
        }
        if modal {
            for rect in modal_rects
                .iter()
                .copied()
                .map(starkit::terminal_graphics::Rect::from)
            {
                scene.components.retain(|component| !matches!(component, Component::Panel { rect: r, .. } if *r == rect));
                let menu = matches!(
                    self.overlays.current(),
                    Some(Overlay::Context(_) | Overlay::Drop(_) | Overlay::Sort(_))
                );
                scene.components.push(if menu {
                    Component::Menu { rect }
                } else {
                    Component::Dialog {
                        rect,
                        title: String::new(),
                    }
                });
            }
        }
        scene.components.extend(overlay_scrollbars);
        if let Some(auth) = self.graphical.as_ref().unwrap().authorization.as_ref() {
            let width = viewport.columns.saturating_sub(4).min(64);
            let x = viewport.columns.saturating_sub(width) / 2;
            let y = viewport.rows.saturating_sub(6) / 2;
            scene.components.clear();
            scene.components.push(Component::Panel {
                rect: starkit::terminal_graphics::Rect {
                    x,
                    y,
                    width,
                    height: 6,
                },
                active: true,
            });
            if self.graphical.as_ref().unwrap().cell_mode {
                for line in 0..6 {
                    let text = if line == 0 || line == 5 {
                        format!(
                            "{}{}{}",
                            if line == 0 { '┌' } else { '└' },
                            "─".repeat(usize::from(width.saturating_sub(2))),
                            if line == 0 { '┐' } else { '┘' }
                        )
                    } else {
                        format!("│{}│", " ".repeat(usize::from(width.saturating_sub(2))))
                    };
                    scene.spans.push(Span {
                        x,
                        y: y + line,
                        text,
                        foreground: hex(self.theme.fg),
                        background: hex(self.theme.panel_bg),
                        bold: false,
                        modifiers: 0,
                    });
                }
            }
            for (line, text) in [
                "Administrator password · Esc cancels".to_string(),
                "".into(),
                "•".repeat(
                    std::str::from_utf8(&auth.secret)
                        .map(|s| s.chars().count())
                        .unwrap_or(0)
                        .min(40),
                ),
            ]
            .into_iter()
            .enumerate()
            {
                scene.spans.push(Span {
                    x: x + 2,
                    y: y + 1 + line as u16,
                    text: format!(
                        "{text:width$}",
                        width = usize::from(width.saturating_sub(4))
                    ),
                    foreground: hex(self.theme.fg),
                    background: hex(self.theme.panel_bg),
                    bold: line == 0,
                    modifiers: 0,
                });
            }
        }
        if native && self.graphical.as_ref().unwrap().authorization.is_none() {
            let mut clickable = Vec::new();
            if modal {
                clickable.extend(self.overlays.pointer_regions(area));
                if let Some(places) = self.places.as_ref() {
                    clickable.extend(places.pointer_regions(area));
                }
                if let Some(picker) = self.tab_picker.as_mut() {
                    clickable.extend(picker.pointer_regions(area));
                }
            } else {
                clickable.extend(
                    self.graphical
                        .as_ref()
                        .unwrap()
                        .rack_hits
                        .iter()
                        .map(|(r, _)| *r),
                );
                clickable.extend(self.tab_hits.iter().map(|(rect, _)| *rect));
                let status = self.status_view(Instant::now());
                for x in regions.status.x..regions.status.right() {
                    if super::super::status::native_hit(
                        regions.status,
                        &status,
                        x,
                        regions.status.y,
                    )
                    .is_some()
                    {
                        clickable.push(Rect::new(x, regions.status.y, 1, 1));
                    }
                }
                for module in [ModuleId::Stack, ModuleId::Preview, ModuleId::Operations] {
                    let count = if module == ModuleId::Stack && self.commander {
                        2
                    } else {
                        1
                    };
                    for pane in 0..count {
                        let rect = if count == 2 {
                            pane_rect(regions.rect_of(module), pane)
                        } else {
                            regions.rect_of(module)
                        };
                        let words = if module == ModuleId::Operations {
                            let view = self.operations_view();
                            panels::operations::header_words(view.paused, view.active, view.focused)
                        } else {
                            self.panel_words(module)
                        };
                        clickable.extend(
                            starkit::chrome::header::slots(rect, &words)
                                .into_iter()
                                .map(|(_, rect)| rect),
                        );
                        if module == ModuleId::Stack {
                            let view = if self.commander {
                                self.pane_view(pane)
                            } else {
                                self.stack_view()
                            };
                            let body = starkit::chrome::frame::body(rect, &panels::words(module));
                            let mut rule =
                                panels::stack::split(body, view.crumbs.len(), view.fold_rows).rule;
                            rule.x += 2;
                            rule.width = rule
                                .width
                                .saturating_sub(2 + breadcrumb_detail_width(&view));
                            let path = if self.commander {
                                &self.panes[pane].dir
                            } else {
                                &self.view.active_dir
                            };
                            let list =
                                panels::stack::split(body, view.crumbs.len(), view.fold_rows).list;
                            if !view.loading && view.error.is_none() {
                                clickable.extend(
                                    view.rows
                                        .iter()
                                        .skip(view.scroll)
                                        .take(usize::from(list.height))
                                        .enumerate()
                                        .map(|(index, _)| {
                                            Rect::new(list.x, list.y + index as u16, list.width, 1)
                                        }),
                                );
                            }
                            clickable.extend(
                                breadcrumb_slots(rule, path, &self.view.home)
                                    .into_iter()
                                    .map(|(rect, _, _)| rect),
                            );
                        }
                    }
                }
            }
            scene.pointer_regions = clickable.into_iter().map(Into::into).collect();
        }
        self.video_scene(&mut scene, &regions);
        let state = self.graphical.as_mut().unwrap();
        state.placements.clear();
        state.resize_handle = None;
        state.viewport = Some(viewport);
        if state.uses_pixel_layout() && state.authorization.is_none() {
            let overlay_spans = if modal {
                Scene::from_buffer(&buffer, viewport, 0).spans
            } else {
                Vec::new()
            };
            scene.placements = pixel_placements(
                &regions,
                self.commander,
                viewport,
                &modal_rects,
                &overlay_spans,
            );
            for p in &scene.placements {
                (
                    p.source.x,
                    p.source.y,
                    p.source.width,
                    p.source.height,
                    p.target.x,
                    p.target.y,
                    p.target.width,
                    p.target.height,
                )
                    .hash(&mut targets);
            }
            scene.interaction = targets.finish();
            if let Some(rect) = state
                .video_picker_rect
                .filter(|_| state.video_picker.is_some())
            {
                let mut placement = starkit::terminal_graphics::placement::Placement::new(
                    rect.into(),
                    starkit::native_surface::PixelRect::new(
                        rect.x * cw,
                        rect.y * ch,
                        rect.width * cw,
                        rect.height * ch,
                    ),
                );
                placement.overlay = Some(vec![]);
                scene.placements.push(placement);
            }
            if matches!(self.view.preview.as_deref(), Some(Preview::Video { .. })) {
                if let Some(p) = scene
                    .placements
                    .iter_mut()
                    .find(|p| p.source == regions.rect_of(ModuleId::Preview).into())
                {
                    balance_video_padding(p);
                }
            }
            state.placements = scene.placements.clone();
            if !modal && self.layout.preview_open {
                if let Some(p) = scene
                    .placements
                    .iter()
                    .find(|p| p.source == regions.rect_of(ModuleId::Preview).into())
                {
                    // One terminal mouse row straddles the border and pane gap.
                    // Keep the grab strip clear of header controls below it.
                    let top = p.target.y.saturating_sub(8);
                    let handle = starkit::native_surface::PixelRect::new(
                        p.target.x,
                        top,
                        p.target.width,
                        16,
                    );
                    state.resize_handle = Some(handle);
                    scene.resize_handles.push(handle);
                }
            }
        }
        if let Some(video) = state.video.as_mut() {
            if let Some(p) = state
                .placements
                .iter()
                .find(|p| p.source == regions.rect_of(ModuleId::Preview).into())
            {
                video.set_bounds(
                    u32::from(p.target.width.saturating_sub(24)),
                    u32::from(p.target.height.saturating_sub(80)),
                );
            }
        }
        // Wheel events target panes, not the file currently painted at that row.
        // Keep scrolling through in-flight frames, but invalidate on modal,
        // directory, tab, pane geometry or embedded-controller changes.
        if !modal && state.authorization.is_none() && self.editor.is_none() {
            for component in &scene.components {
                if let Component::Panel { rect, .. } | Component::Tab { rect, .. } = component {
                    (rect.x, rect.y, rect.width, rect.height).hash(&mut scroll_targets);
                }
            }
            for p in &scene.placements {
                (
                    p.source.x,
                    p.source.y,
                    p.source.width,
                    p.source.height,
                    p.target.x,
                    p.target.y,
                    p.target.width,
                    p.target.height,
                )
                    .hash(&mut scroll_targets);
            }
            scene.scroll_interaction = Some(scroll_targets.finish());
        }
        scene
    }
    fn input(&mut self, input: Input) {
        let _chrome =
            starkit::chrome::frame::padding_scope(self.graphical.as_ref().unwrap().padded_chrome);
        let _rack = panels::stack::rack_scope(
            self.layout.native_rack && self.layout.last.as_ref().is_some_and(layout::rack_browser),
        );
        if self.graphical.as_ref().unwrap().authorization.is_some() {
            use std::io::Write;
            let mut cancel = false;
            if let Input::Paste { text } = &input {
                let auth = self
                    .graphical
                    .as_mut()
                    .unwrap()
                    .authorization
                    .as_mut()
                    .unwrap();
                for c in text.chars().filter(|c| !c.is_control()) {
                    if auth.secret.len() + c.len_utf8() > 4096 {
                        break;
                    }
                    let mut bytes = [0; 4];
                    auth.secret
                        .extend_from_slice(c.encode_utf8(&mut bytes).as_bytes());
                    bytes.fill(0);
                }
            }
            if let Input::Key { code, modifiers } = &input {
                if let Some(code) = key_code(code) {
                    let auth = self
                        .graphical
                        .as_mut()
                        .unwrap()
                        .authorization
                        .as_mut()
                        .unwrap();
                    match code {
                        KeyCode::Esc => cancel = true,
                        KeyCode::Char(c)
                            if *modifiers == 0
                                || *modifiers
                                    == starkit::crossterm::event::KeyModifiers::SHIFT.bits() =>
                        {
                            let mut bytes = [0; 4];
                            if auth.secret.len() < 4096 {
                                auth.secret
                                    .extend_from_slice(c.encode_utf8(&mut bytes).as_bytes());
                            }
                            bytes.fill(0);
                        }
                        KeyCode::Enter => {
                            let _ = auth.input.write_all(&auth.secret);
                            let _ = auth.input.write_all(b"\n");
                            auth.secret.fill(0);
                            auth.secret.clear();
                        }
                        KeyCode::Backspace => {
                            let mut length = auth.secret.len().saturating_sub(1);
                            while length > 0 && auth.secret[length] & 0xc0 == 0x80 {
                                length -= 1;
                            }
                            auth.secret[length..].fill(0);
                            auth.secret.truncate(length);
                        }
                        _ => {}
                    }
                }
            }
            if cancel {
                self.graphical.as_mut().unwrap().authorization = None;
                self.note = Some((
                    "Administrator authorization cancelled".into(),
                    NoteLevel::Warning,
                    Instant::now(),
                ));
            }
            return;
        }
        if let Input::Paste { text } = &input {
            if self.extension_editor_paste(text) {
                return;
            }
        }
        match input {
            Input::Notice { message } => {
                if let Some(body) = message.strip_prefix("STARFOLD_UPDATE\n") {
                    if body.len() <= 128 * 1024 && self.pending_update_notices.len() < 4 {
                        self.pending_update_notices.push_back(
                            body.lines()
                                .map(|s| s.chars().filter(|c| !c.is_control()).collect())
                                .collect(),
                        );
                        self.repaint = true;
                    }
                    return;
                }
                self.note = Some((message.clone(), NoteLevel::Warning, Instant::now()));
                let state = self.graphical.as_mut().unwrap();
                if let Some(video) = &mut state.video {
                    video.warning = Some(message.clone());
                }
                self.repaint = true;
            }
            Input::Play { path } => self.launch_movie(path),
            Input::Key { code, modifiers } => {
                self.reveal_video_controls();
                if self.video_picker_key(&code, modifiers) {
                    return;
                }
                if self.graphical.as_ref().unwrap().video_scrub.is_some() {
                    self.cancel_video_scrub();
                    if code == "escape" {
                        return;
                    }
                }
                if self.video_key(&code, modifiers) {
                    return;
                }
                // Navigation or opening a modal ends the captured file gesture.
                if self.graphical.as_ref().unwrap().drag.is_some()
                    || self.graphical.as_ref().unwrap().preview_resize.is_some()
                {
                    Controller::input(self, Input::CancelPointer);
                    if code == "escape" {
                        return;
                    }
                }
                if let Some(code) = key_code(&code) {
                    self.key(KeyEvent::new(
                        code,
                        starkit::crossterm::event::KeyModifiers::from_bits_truncate(modifiers),
                    ));
                }
            }
            Input::Pointer {
                action,
                button,
                x,
                y,
                pixel,
                modifiers,
            } => {
                if self.graphical.as_ref().unwrap().video_fullscreen.is_some() {
                    self.reveal_video_controls();
                    self.video_pointer(&action, button, x, y);
                    return;
                }
                if self.video_picker_pointer(&action, button, x, y) {
                    return;
                }
                if self.overlays.is_open() && matches!(action.as_str(), "down" | "up") {
                    tracing::debug!(%action, button, x, y, ?pixel, "Graphical modal pointer received");
                }
                if self.graphical_preview_resize(&action, button, pixel, x, y) {
                    return;
                }
                if self.graphical.as_ref().unwrap().drag.is_some()
                    && (self.overlays.is_open()
                        || self.places.is_some()
                        || self.tab_picker.is_some())
                {
                    Controller::input(self, Input::CancelPointer);
                    return;
                }
                let state = self.graphical.as_ref().unwrap();
                if state.placements.is_empty() {
                    self.graphical_pointer(&action, button, x, y, modifiers);
                } else if let Some(v) = state.viewport {
                    let project = |p: &starkit::terminal_graphics::placement::Placement,
                                   captured| {
                        match pixel {
                            Some([px, py]) if px < v.width && py < v.height => {
                                p.pointer_pixels(px, py, captured)
                            }
                            Some(_) => None,
                            None => p.pointer(v, x, y, captured),
                        }
                    };
                    let captured = (self.bars.held().is_some() || state.video_scrub.is_some())
                        && matches!(action.as_str(), "drag" | "up");
                    let placement = if captured {
                        state.pointer_capture.as_ref()
                    } else {
                        state
                            .placements
                            .iter()
                            .rev()
                            .find(|p| project(p, false).is_some())
                    }
                    .cloned();
                    if let Some(p) = placement {
                        if let Some((sx, sy)) = project(&p, captured) {
                            if self.overlays.is_open() && action == "down" {
                                tracing::debug!(sx, sy, source = ?p.source, target = ?p.target, "Graphical modal pointer projected");
                            }
                            if action == "down" {
                                self.graphical.as_mut().unwrap().pointer_capture = Some(p);
                            }
                            self.graphical_pointer(&action, button, sx, sy, modifiers);
                        }
                    } else if action == "up" {
                        Controller::input(self, Input::CancelPointer);
                    } else if action == "drag" {
                        self.dnd.hover = None;
                        self.dnd.hover_coords = None;
                        self.repaint = true;
                    }
                    if action == "up" {
                        self.graphical.as_mut().unwrap().pointer_capture = None;
                    }
                }
            }
            Input::Paste { text } => {
                if let Some(input) = &mut self.graphical.as_mut().unwrap().video_subtitle_input {
                    input.extend(
                        text.chars()
                            .filter(|c| !c.is_control())
                            .take(4096usize.saturating_sub(input.len())),
                    );
                    self.repaint = true;
                    return;
                }
                if self.editor.is_some() {
                    self.editor_paste(&text);
                } else if self.overlays.paste(&text) {
                    self.repaint = true;
                } else if let Some(input) = self.filter.as_mut() {
                    input.paste(&text);
                    self.core.send(Command::SetFilter(input.text().into()));
                }
            }
            Input::Osc72 { text } => {
                let kind = wire_dnd::Message::parse(&text).and_then(|m| m.get("t"));
                self.dnd_message(&text);
                if kind == Some("o") && self.dnd.drag_active {
                    let state = self.graphical.as_mut().unwrap();
                    if state.drag.is_none() {
                        if let Some(offer) = self.dnd.offer.as_ref() {
                            state.drag = Some(Drag {
                                regions: self.layout.last.clone(),
                                origin: (0, 0),
                                stack: offer.source_stack,
                                sources: offer.sources.clone(),
                                started: true,
                                desktop: true,
                            });
                        }
                    }
                    if let Some(drag) = state.drag.as_mut() {
                        drag.desktop = true;
                        drag.started = true;
                    }
                }
                if matches!(kind, Some("M" | "E" | "R"))
                    || (kind == Some("e") && !self.dnd.drag_active)
                {
                    self.graphical.as_mut().unwrap().drag = None;
                    self.graphical.as_mut().unwrap().pointer_capture = None;
                }
            }
            Input::CancelPointer => {
                self.cancel_video_scrub();
                self.graphical.as_mut().unwrap().preview_resize = None;
                self.graphical.as_mut().unwrap().pointer_capture = None;
                if self
                    .graphical
                    .as_ref()
                    .unwrap()
                    .drag
                    .as_ref()
                    .is_some_and(|drag| !drag.desktop)
                {
                    self.graphical.as_mut().unwrap().drag = None;
                    self.dnd.drag_active = false;
                    self.dnd.offer = None;
                    self.dnd.hover = None;
                    self.dnd.hover_coords = None;
                }
                // Desktop OSC 72 transfers have their own lifetime; switching
                // windows must not erase their source offer or received data.
                self.mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
                    column: 0,
                    row: 0,
                    modifiers: starkit::crossterm::event::KeyModifiers::empty(),
                });
            }
            _ => {}
        }
    }
    fn media(&mut self, message: starkit::terminal_graphics::media::ToHost) {
        use starkit::terminal_graphics::media::ToHost;
        let state = self.graphical.as_mut().unwrap();
        match &message {
            ToHost::AudioCredit {
                session,
                epoch,
                blocks,
            } if state.audio_local
                && *session == self.audio_activation
                && state.audio_epoch == Some(*epoch) =>
            {
                if let Err(error) = self
                    .audio
                    .control("audio_credit", Some((*blocks).min(8) as f64))
                {
                    self.audio_error = Some(error);
                }
                return;
            }
            ToHost::AudioError {
                session,
                epoch,
                message,
            } if *session == self.audio_activation && state.audio_epoch == Some(*epoch) => {
                self.audio_error = Some(message.clone());
                // A local-device failure must not start sound on the host.
                let _ = self.audio.control("pause", None);
                self.close_audio_relay();
                return;
            }
            _ => {}
        }
        if let Some(video) = &mut self.graphical.as_mut().unwrap().video {
            video.receive(message);
        }
    }
    fn media_chunks(&mut self) -> Vec<ServerMessage> {
        use starkit::terminal_graphics::media::ToClient;
        let mut messages = Vec::new();
        let state = self.graphical.as_ref().unwrap();
        for _ in 0..2 {
            let Some((epoch, samples)) = self.audio.take_audio_block() else {
                break;
            };
            if state.audio_local && state.audio_epoch == Some(epoch) {
                messages.push(ServerMessage::Media {
                    message: ToClient::AudioChunk {
                        session: self.audio_activation,
                        epoch,
                        samples,
                    },
                });
            }
        }
        // The session's bounded media lane reserves room for two messages.
        if messages.is_empty() {
            messages.extend(
                self.graphical
                    .as_mut()
                    .unwrap()
                    .video
                    .as_mut()
                    .map(|v| v.chunks())
                    .unwrap_or_default(),
            );
        }
        messages
    }
    fn effects(&mut self) -> Vec<ServerMessage> {
        let state = self.graphical.as_mut().unwrap();
        let mut effects = std::mem::take(&mut state.effects);
        if let Some(video) = &mut state.video {
            effects.extend(video.effects());
        }
        if let Some(rx) = &state.wire {
            effects.extend(rx.try_iter().take(32).map(|out| ServerMessage::Osc72 {
                id: 0,
                meta: out.meta,
                payload: out.payload,
            }));
        }
        effects
    }
    fn closed(&self) -> bool {
        self.quit
    }
    fn shutdown(&mut self) {
        self.save_workspace(true);
        self.stop_video();
        self.editor = None;
        self.stop_audio();
        self.session_writer = None;
        self.core.send(Command::Shutdown);
    }
}

impl App {
    pub(super) fn graphical_video_snapshot(&self) -> Option<session::VideoSession> {
        let state = self.graphical.as_ref()?;
        let video = state.video.as_ref()?;
        let remember = |selection: &starkit::media::tracks::Selection| {
            use starkit::media::tracks::Selection;
            match selection {
                Selection::Auto => session::TrackSelection::Auto,
                Selection::Off => session::TrackSelection::Off,
                Selection::Stream(index) => session::TrackSelection::Stream(*index),
                Selection::External(path) => session::TrackSelection::External(path.clone()),
            }
        };
        Some(session::VideoSession {
            path: state.video_path.clone()?,
            position: state
                .video_scrub
                .as_ref()
                .map_or(video.position, |scrub| scrub.position),
            paused: video.paused || video.finished,
            volume: video.volume,
            mode: state.video_mode,
            audio: remember(&video.options.audio),
            subtitle: remember(&video.options.subtitle),
        })
    }

    fn suspend_video(&mut self) {
        let saved = self.video_resume_snapshot();
        let rows = self.layout.native_preview_rows;
        self.stop_video();
        self.layout.native_preview_rows = rows;
        self.resume_video = saved;
    }

    pub(super) fn audio_output_word(&self) -> Option<panels::Word> {
        let state = self.graphical.as_ref()?;
        (self.audio_here() && state.audio_relay_capable)
            .then_some(panels::Word::AudioOutput(state.audio_local))
    }
    pub(super) fn close_audio_relay(&mut self) {
        let Some(state) = self.graphical.as_mut() else {
            return;
        };
        if state.audio_epoch.take().is_some() {
            state.effects.push(ServerMessage::Media {
                message: starkit::terminal_graphics::media::ToClient::AudioClose {
                    session: self.audio_activation,
                },
            });
        }
        self.audio.discard_audio_blocks();
    }
    pub(super) fn toggle_audio_output(&mut self) {
        if self.audio_output_word().is_none() {
            return;
        }
        if !self.audio.audio_relay_available() {
            self.note = Some((
                "Update STAR/AMP for local SSH audio playback".into(),
                NoteLevel::Warning,
                Instant::now(),
            ));
            self.repaint = true;
            return;
        }
        let local = !self.graphical.as_ref().unwrap().audio_local;
        self.close_audio_relay();
        if let Err(error) = self.audio.set_audio_local(local) {
            self.audio_error = Some(error);
        } else {
            self.graphical.as_mut().unwrap().audio_local = local;
            self.audio_error = None;
        }
        self.repaint = true;
    }
    pub(super) fn movie_details_preview(&self) -> Option<Preview> {
        let state = self.graphical.as_ref()?;
        if state.video.is_some() || state.video_pending.is_some() {
            return None;
        }
        match self.view.preview.as_deref()? {
            Preview::Video { metadata, .. } => Some(Preview::Document((**metadata).clone())),
            _ => None,
        }
    }
    fn pending_movie_error(&self, pending: &Path) -> Option<String> {
        if let Some(Preview::Error(error)) = self.view.preview.as_deref() {
            return Some(error.clone());
        }
        let state = self.core.state();
        let (path, preview) = state.preview.as_ref()?;
        if path != pending {
            return None;
        }
        match preview.as_ref() {
            Preview::Error(error) => Some(error.clone()),
            Preview::Document(document) if document.kind != "Loading preview…" => {
                Some(document.notice.clone().unwrap_or_else(|| {
                    "Movie preview could not start; try opening it again".into()
                }))
            }
            _ => None,
        }
    }
    fn movie_loading_stage(&self) -> Option<&'static str> {
        if !self.layout.preview_open {
            return None;
        }
        let state = self.graphical.as_ref()?;
        if let Some(video) = &state.video {
            return (video.buffering && !video.finished).then_some(if video.position > 0. {
                "Buffering movie…"
            } else {
                "Starting playback…"
            });
        }
        if matches!(self.view.preview.as_deref(), Some(Preview::Error(_)))
            || state
                .video_pending
                .as_ref()
                .is_some_and(|path| self.pending_movie_error(path).is_some())
        {
            return None;
        }
        if state.video_pending.is_some()
            || state.direct_play.is_some()
            || (self.view.preview.is_none()
                && self.last_preview_for.as_ref().is_some_and(|path| {
                    crate::fold::file_type::classify(path, &[])
                        == crate::fold::file_type::FileType::Video
                }))
        {
            return Some("Loading movie…");
        }
        if state.image.is_none()
            && state.video_pending.is_some()
            && matches!(self.view.preview.as_deref(), Some(Preview::Video { .. }))
        {
            return Some("Preparing preview…");
        }
        None
    }
    pub(super) fn draw_movie_loading(&self, area: Rect, buf: &mut Buffer) {
        let Some(stage) = self.movie_loading_stage() else {
            return;
        };
        // Visible in the empty picture area until the client has a frame;
        // the timeline status remains visible during later buffering too.
        let body = panels::preview::content_rect(area);
        if body.is_empty() {
            return;
        }
        let style = Style::default()
            .fg(panels::rgb(self.theme.accent))
            .bg(panels::rgb(self.theme.panel_bg));
        for y in body.y..body.bottom() {
            buf.set_string(body.x, y, " ".repeat(usize::from(body.width)), style);
        }
        let text = starkit::text::truncate(
            &format!("{} {stage}", self.unmount_spinner()),
            usize::from(body.width),
        );
        let x = body.x + body.width.saturating_sub(starkit::wrap::width_of(&text)) / 2;
        buf.set_string(x, body.y + body.height / 2, text, style);
    }
    pub(super) fn video_words(&self) -> Option<Vec<panels::Word>> {
        let state = self.graphical.as_ref()?;
        if !state.video_capable
            || !matches!(self.view.preview.as_deref(), Some(Preview::Video { .. }))
        {
            return None;
        }
        let paused = state.video.as_ref().is_none_or(|v| v.paused || v.finished);
        let volume = state.video.as_ref().map_or(80, |v| v.volume);
        let mut words = vec![
            panels::Word::VideoPlay(paused),
            panels::Word::VideoMute(volume),
            panels::Word::VideoVolumeDown,
            panels::Word::VideoVolumeUp,
            panels::Word::VideoExpand(state.video_expanded.is_some()),
            panels::Word::Close,
        ];
        if state.original_media && !state.local_media {
            words.push(panels::Word::VideoMode(state.video_mode));
        }
        Some(words)
    }
    pub(super) fn stop_video(&mut self) {
        self.resume_video = None;
        let Some(state) = self.graphical.as_mut() else {
            return;
        };
        if let Some(video) = state.video.take() {
            state.effects.push(video.close());
        }
        if state.video_fullscreen.take().is_some() && state.video_player {
            state
                .effects
                .push(ServerMessage::Fullscreen { enabled: false });
        }
        state.video_picker = None;
        state.video_picker_controls = None;
        state.video_subtitle_input = None;
        state.video_scrub = None;
        state.video_path = None;
        state.video_pending = None;
        state.direct_play = None;
        state.direct_fullscreen = false;
        state.video_timeline = None;
        state.video_controls = None;
        state.video_control_rect = None;
        state.video_control_request = None;
        if let Some(rows) = state.video_expanded.take() {
            self.layout.native_preview_rows = rows;
        }
    }
    pub(super) fn activate_video_entry(&mut self, path: PathBuf) {
        if self
            .resume_video
            .as_ref()
            .is_some_and(|saved| saved.path != path)
        {
            self.resume_video = None;
        }
        if self.audio_path.is_some() {
            self.stop_audio();
        }
        self.layout.preview_open = true;
        self.layout.focus_set(ModuleId::Preview);
        if !matches!(self.view.preview.as_deref(), Some(Preview::Video { path: ready, .. }) if ready == &path)
        {
            self.last_preview_for = None;
        }
        self.graphical.as_mut().unwrap().video_pending = Some(path);
        self.repaint = true;
    }
    fn tick_video(&mut self) {
        if let Some(saved) = &self.resume_video {
            let state = self.graphical.as_mut().unwrap();
            if state.can_play_video()
                && self.layout.preview_open
                && state.video.is_none()
                && state.direct_play.is_none()
                && state.video_pending.is_none()
            {
                state.video_mode = saved.mode.min(2);
                state.direct_play = Some(saved.path.clone());
                state.direct_fullscreen = false;
                if let Some(parent) = saved.path.parent() {
                    if parent != self.core.state().active_frame().dir {
                        self.core.send(Command::Push(parent.into()));
                    }
                }
            }
        }
        if let Some(path) = self.graphical.as_ref().unwrap().direct_play.clone() {
            let found = {
                let state = self.core.state();
                state
                    .rows(state.active_frame())
                    .iter()
                    .position(|entry| entry.path == path)
            };
            if let Some(index) = found {
                if self.view.cursor_path.as_ref() == Some(&path) {
                    self.graphical.as_mut().unwrap().direct_play = None;
                    self.activate_video_entry(path);
                } else {
                    self.core.send(Command::CursorTo(index));
                }
            } else {
                let ready = {
                    let state = self.core.state();
                    !state.loading
                        && state.active_listing().is_some()
                        && path.parent() == Some(state.active_frame().dir.as_path())
                };
                if ready {
                    self.resume_video = None;
                    let state = self.graphical.as_mut().unwrap();
                    state.direct_play = None;
                    state.direct_fullscreen = false;
                    self.note = Some((
                        format!("Movie not found: {}", path.display()),
                        NoteLevel::Error,
                        Instant::now(),
                    ));
                    self.repaint = true;
                }
            }
        }
        let mut actions = Vec::new();
        if let Some(controls) = self
            .graphical
            .as_ref()
            .and_then(|g| g.video_controls.as_ref())
        {
            let (changed, error) = controls.poll();
            self.repaint |= changed;
            if let Some(error) = error {
                self.note = Some((error, NoteLevel::Warning, Instant::now()));
            }
            while let Some(action) = controls.action() {
                actions.push(action);
            }
        }
        if let Some(controls) = self
            .graphical
            .as_ref()
            .unwrap()
            .video_picker_controls
            .as_ref()
        {
            let (changed, _) = controls.poll();
            self.repaint |= changed;
            while let Some(action) = controls.action() {
                actions.push(action);
            }
        }
        for (action, value) in actions {
            self.video_transport_action(&action, value);
        }
        if let Some(pending) = self.graphical.as_ref().unwrap().video_pending.clone() {
            let cursor_matches = self.view.cursor_path.as_ref().is_some_and(|cursor| {
                cursor == &pending
                    || self.core.state().archive_materialized.get(cursor) == Some(&pending)
            });
            if !self.layout.preview_open || !cursor_matches {
                self.graphical.as_mut().unwrap().video_pending = None;
            } else if matches!(self.view.preview.as_deref(), Some(Preview::Video { path, .. }) if path == &pending)
            {
                self.graphical.as_mut().unwrap().video_pending = None;
                self.video_action(panels::Word::VideoPlay(true));
                if let Some(saved) = self.resume_video.take() {
                    if let Some(video) = self.graphical.as_mut().unwrap().video.as_mut() {
                        let restore = |selection: session::TrackSelection| {
                            use starkit::media::tracks::Selection;
                            match selection {
                                session::TrackSelection::Auto => Selection::Auto,
                                session::TrackSelection::Off => Selection::Off,
                                session::TrackSelection::Stream(index) => Selection::Stream(index),
                                session::TrackSelection::External(path) => {
                                    Selection::External(path)
                                }
                            }
                        };
                        let position = if saved.position.is_finite() {
                            saved.position.max(0.)
                        } else {
                            0.
                        };
                        let duration = match self.view.preview.as_deref() {
                            Some(Preview::Video { poster, .. }) => poster.duration,
                            _ => 0.,
                        };
                        video.restore_playback(
                            if duration > 0. {
                                position.min(duration)
                            } else {
                                position
                            },
                            starkit::media::tracks::PlaybackOptions {
                                audio: restore(saved.audio),
                                subtitle: restore(saved.subtitle),
                            },
                            saved.paused,
                            saved.volume,
                        );
                    }
                }
                if self.graphical.as_mut().unwrap().direct_fullscreen {
                    self.graphical.as_mut().unwrap().direct_fullscreen = false;
                    self.toggle_video_fullscreen(true);
                }
            } else if let Some(error) = self.pending_movie_error(&pending) {
                self.graphical.as_mut().unwrap().video_pending = None;
                self.graphical.as_mut().unwrap().direct_play = None;
                self.resume_video = None;
                self.note = Some((error, NoteLevel::Error, Instant::now()));
                self.repaint = true;
            }
        }
        let path = match self.view.preview.as_deref() {
            Some(Preview::Video { path, .. }) => Some(path),
            _ => None,
        };
        let stop = self
            .graphical
            .as_ref()
            .unwrap()
            .video_path
            .as_ref()
            .is_some_and(|p| Some(p) != path)
            || !self.layout.preview_open
            || self.editor.is_some()
            || self.audio_here();
        if stop {
            self.stop_video();
            return;
        }
        let state = self.graphical.as_mut().unwrap();
        if state.video_mode == 0 {
            if let Some(video) = state.video.as_mut().filter(|v| v.original && v.finished) {
                if let Some(error) = video
                    .warning
                    .clone()
                    .filter(|v| v.starts_with("Video preview:") || v.starts_with("Original media:"))
                {
                    video.set_original(false);
                    self.note = Some((
                        format!("Original playback failed; using Preview: {error}"),
                        NoteLevel::Warning,
                        Instant::now(),
                    ));
                    self.repaint = true;
                }
            }
        }
        let visible = state.video_picker.is_some()
            || state.video_subtitle_input.is_some()
            || state.video.as_ref().is_none_or(|v| v.paused || v.finished)
            || state
                .video_overlay_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(3));
        if visible != state.video_overlay_visible {
            state.video_overlay_visible = visible;
            self.repaint = true;
        }
    }

    pub(super) fn video_action(&mut self, word: panels::Word) {
        if self.video_words().is_none() {
            return;
        }
        tracing::debug!(?word, "Video preview action");
        self.layout.focus_set(ModuleId::Preview);
        match word {
            panels::Word::VideoMode(_) => self.cycle_video_mode(),
            panels::Word::VideoPlay(_) => {
                if self.graphical.as_ref().unwrap().video.is_none() {
                    let Some(Preview::Video { path, .. }) = self.view.preview.as_deref() else {
                        return;
                    };
                    let path = path.clone();
                    // The poster is already loaded. Audio cleanup normally
                    // invalidates it; reloading introduces a Loading state
                    // that would cancel this newly started video.
                    if self.audio_path.is_some() {
                        let preview_for = self.last_preview_for.clone();
                        self.stop_audio();
                        self.last_preview_for = preview_for;
                    }
                    let state = self.graphical.as_mut().unwrap();
                    state.video_sequence += 1;
                    state.video = Some(starkit::terminal_graphics::media::Host::new_with_original(
                        path.clone(),
                        format!("video-{}-1", state.video_sequence),
                        state.video_sequence,
                        state.local_media,
                        state.original_media && state.video_mode != 2,
                    ));
                    state.video.as_mut().unwrap().set_control(false, 80);
                    if let Some(placement) = state.placements.iter().find(|p| {
                        self.layout
                            .last
                            .as_ref()
                            .is_some_and(|r| p.source == r.rect_of(ModuleId::Preview).into())
                    }) {
                        if let Some(video) = state.video.as_mut() {
                            video.set_bounds(
                                u32::from(placement.target.width.saturating_sub(24)),
                                u32::from(placement.target.height.saturating_sub(80)),
                            );
                        }
                    }
                    state.video_path = Some(path);
                } else {
                    let v = self.graphical.as_mut().unwrap().video.as_mut().unwrap();
                    if v.finished {
                        v.restart(0.0);
                        v.set_control(false, v.volume);
                    } else {
                        v.set_control(!v.paused, v.volume);
                    }
                }
            }
            panels::Word::VideoMute(_)
            | panels::Word::VideoVolumeDown
            | panels::Word::VideoVolumeUp => {
                let state = self.graphical.as_mut().unwrap();
                if let Some(v) = state.video.as_mut() {
                    let volume = match word {
                        panels::Word::VideoMute(_) => {
                            if v.volume == 0 {
                                state.video_unmuted_volume.unwrap_or(80)
                            } else {
                                state.video_unmuted_volume = Some(v.volume);
                                0
                            }
                        }
                        panels::Word::VideoVolumeDown => v.volume.saturating_sub(10),
                        _ => v.volume.saturating_add(10).min(100),
                    };
                    v.set_control(v.paused, volume);
                }
            }
            panels::Word::VideoExpand(_) => {
                let state = self.graphical.as_mut().unwrap();
                if let Some(rows) = state.video_expanded.take() {
                    self.layout.native_preview_rows = rows;
                } else {
                    state.video_expanded = Some(self.layout.native_preview_rows);
                    self.layout.native_preview_rows = Some(u16::MAX);
                }
            }
            _ => {}
        }
        self.repaint = true;
    }
    fn video_stream_label(video: &starkit::terminal_graphics::media::Host) -> String {
        if video.local_playback() {
            return "local".into();
        }
        if video.original {
            let bitrate = if video.source_bitrate > 0 {
                format!(" · {:.1} Mb/s", video.source_bitrate as f64 / 1_000_000.0)
            } else {
                String::new()
            };
            return format!(
                "SSH stream · Original{bitrate} · buffer {:.1} MiB · dropped {}",
                video.buffered_bytes as f64 / 1_048_576.0,
                video.dropped_frames
            );
        }
        format!(
            "SSH stream · Preview · up to {}p · H.264/AAC",
            match video.quality {
                starkit::media::Quality::Low => 360,
                starkit::media::Quality::Balanced => 480,
                starkit::media::Quality::High => 720,
            }
        )
    }
    fn cycle_video_mode(&mut self) {
        let state = self.graphical.as_mut().unwrap();
        if !state.original_media || state.local_media {
            return;
        }
        let next = (state.video_mode + 1) % 3;
        if next != 2
            && state.video.as_ref().is_some_and(|video| {
                !matches!(
                    video.options.subtitle,
                    starkit::media::tracks::Selection::Off
                        | starkit::media::tracks::Selection::Auto
                )
            })
        {
            self.note = Some((
                "Turn subtitles off before selecting Original mode".into(),
                NoteLevel::Warning,
                Instant::now(),
            ));
            self.repaint = true;
            return;
        }
        state.video_mode = next;
        if let Some(video) = &mut state.video {
            video.set_original(state.video_mode != 2);
        }
        self.repaint = true;
    }
    fn video_key(&mut self, code: &str, modifiers: u8) -> bool {
        if self.g_pending || self.d_pending {
            return false;
        }
        if modifiers & !starkit::crossterm::event::KeyModifiers::SHIFT.bits() != 0
            || self.layout.focus() != ModuleId::Preview
            || self.overlays.is_open()
            || self.video_words().is_none()
        {
            return false;
        }
        let code = code.strip_prefix("char:").unwrap_or(code);
        if self.extension_key(code) {
            return true;
        }
        match code {
            "p" | "P" | " " | "space" => self.video_action(panels::Word::VideoPlay(true)),
            "a" | "A" if self.graphical.as_ref().unwrap().video_player => {
                self.open_video_picker(true)
            }
            "s" | "S" if self.graphical.as_ref().unwrap().video_player => {
                self.open_video_picker(false)
            }
            "a" | "A" if !self.graphical.as_ref().unwrap().video_player => {
                self.video_action(panels::Word::VideoMute(0))
            }
            "o" | "O" if self.graphical.as_ref().unwrap().original_media => self.cycle_video_mode(),
            "m" | "M" => self.video_action(panels::Word::VideoMute(0)),
            "+" | "=" => self.video_action(panels::Word::VideoVolumeUp),
            "-" => self.video_action(panels::Word::VideoVolumeDown),
            "f" | "F" if self.graphical.as_ref().unwrap().video_player => self
                .toggle_video_fullscreen(
                    code == "f"
                        && modifiers & starkit::crossterm::event::KeyModifiers::SHIFT.bits() == 0,
                ),
            "f" | "F" if !self.graphical.as_ref().unwrap().video_player => {
                self.video_action(panels::Word::VideoExpand(false))
            }
            "e" | "E" => self.video_action(panels::Word::VideoExpand(false)),
            "escape" if self.graphical.as_ref().unwrap().video_fullscreen.is_some() => {
                self.exit_video_fullscreen()
            }
            "q" | "Q" if self.graphical.as_ref().unwrap().video_fullscreen.is_some() => {
                self.stop_video()
            }
            "escape" if self.graphical.as_ref().unwrap().video_expanded.is_some() => {
                self.video_action(panels::Word::VideoExpand(true))
            }
            "left" | "right" => {
                let duration = match self.view.preview.as_deref() {
                    Some(Preview::Video { poster, .. }) => poster.duration,
                    _ => 0.0,
                };
                if let Some(v) = self.graphical.as_mut().unwrap().video.as_mut() {
                    v.restart(
                        (v.position + if code == "left" { -5.0 } else { 5.0 })
                            .clamp(0.0, duration.max(0.0)),
                    );
                }
            }
            _ => return false,
        }
        true
    }
    pub(super) fn extension_media_actions(
        &mut self,
        actions: &[crate::fold::preview::extensions::Action],
    ) {
        use starfold_preview_protocol::MediaAction as A;
        if self.video_words().is_none() {
            return;
        }
        for scoped in actions {
            let action = &scoped.action;
            match action {
                A::PlayPause => self.video_action(panels::Word::VideoPlay(true)),
                A::VolumeUp => self.video_action(panels::Word::VideoVolumeUp),
                A::VolumeDown => self.video_action(panels::Word::VideoVolumeDown),
                A::Mute => self.video_action(panels::Word::VideoMute(0)),
                A::Expand => self.video_action(panels::Word::VideoExpand(false)),
                A::WindowFullscreen => {
                    if self.graphical.as_ref().unwrap().video_player {
                        self.toggle_video_fullscreen(false);
                    } else {
                        self.video_action(panels::Word::VideoExpand(false));
                    }
                }
                A::Fullscreen => {
                    if self.graphical.as_ref().unwrap().video_player {
                        self.toggle_video_fullscreen(true);
                    } else {
                        self.video_action(panels::Word::VideoExpand(false));
                    }
                }
                A::AudioTracks => {
                    if self.graphical.as_ref().unwrap().video_player {
                        self.open_video_picker(true);
                    } else {
                        self.video_action(panels::Word::VideoMute(0));
                    }
                }
                A::Subtitles => {
                    if self.graphical.as_ref().unwrap().video_player {
                        self.open_video_picker(false);
                    }
                }
                A::StreamMode => self.cycle_video_mode(),
                A::ExitFullscreen => self.exit_video_fullscreen(),
                A::Stop => self.stop_video(),
                A::SeekForward | A::SeekBackward => {
                    let duration = match self.view.preview.as_deref() {
                        Some(Preview::Video { poster, .. }) => poster.duration,
                        _ => 0.0,
                    };
                    if let Some(video) = self.graphical.as_mut().unwrap().video.as_mut() {
                        video.restart(
                            (video.position
                                + if *action == A::SeekBackward {
                                    -5.0
                                } else {
                                    5.0
                                })
                            .clamp(0.0, duration),
                        );
                    }
                }
            }
        }
        self.repaint = true;
    }
    fn video_transport_action(&mut self, action: &str, value: Option<f32>) {
        match action {
            "fullscreen" => self.toggle_video_fullscreen(true),
            "audio_tracks" => self.open_video_picker(true),
            "subtitle_tracks" => self.open_video_picker(false),
            "picker_close" => {
                let state = self.graphical.as_mut().unwrap();
                state.video_picker = None;
                state.video_subtitle_input = None;
            }
            action if action.starts_with("track:") => {
                if let Ok(index) = action[6..].parse::<usize>() {
                    self.choose_video_track(index);
                }
            }
            "play" => {
                let video = self.graphical.as_ref().unwrap().video.as_ref();
                if video.is_none_or(|v| v.paused || v.finished) {
                    self.video_action(panels::Word::VideoPlay(true));
                }
            }
            "pause" => {
                if self
                    .graphical
                    .as_ref()
                    .unwrap()
                    .video
                    .as_ref()
                    .is_some_and(|v| !v.paused && !v.finished)
                {
                    self.video_action(panels::Word::VideoPlay(false));
                }
            }
            "stop" => self.stop_video(),
            "previous" | "next" => {
                let duration = match self.view.preview.as_deref() {
                    Some(Preview::Video { poster, .. }) => poster.duration,
                    _ => 0.0,
                };
                if let Some(video) = self.graphical.as_mut().unwrap().video.as_mut() {
                    video.restart(
                        (video.position + if action == "previous" { -5.0 } else { 5.0 })
                            .clamp(0.0, duration.max(0.0)),
                    );
                }
            }
            "volume" => {
                if let Some(value) = value.filter(|v| v.is_finite()) {
                    if let Some(video) = self.graphical.as_mut().unwrap().video.as_mut() {
                        video.set_control(
                            video.paused,
                            (value.clamp(0.0, 1.0) * 100.0).round() as u8,
                        );
                    }
                }
            }
            _ => return,
        }
        self.repaint = true;
    }
    fn cancel_video_scrub(&mut self) {
        let state = self.graphical.as_mut().unwrap();
        if let Some(scrub) = state.video_scrub.take() {
            if let Some(video) = state.video.as_mut() {
                video.set_control(scrub.paused, video.volume);
            }
            state.pointer_capture = None;
            self.repaint = true;
        }
    }
    fn video_pointer(&mut self, action: &str, button: u8, x: u16, y: u16) -> bool {
        if self.video_picker_pointer(action, button, x, y) {
            return true;
        }
        if button != 0 || self.overlays.is_open() || self.video_words().is_none() {
            return false;
        }
        let duration = match self.view.preview.as_deref() {
            Some(Preview::Video { poster, .. }) => poster.duration,
            _ => 0.0,
        };
        let state = self.graphical.as_mut().unwrap();
        if matches!(action, "drag" | "up") {
            let Some(scrub) = state.video_scrub.as_mut() else {
                return false;
            };
            scrub.position = f64::from(
                x.clamp(scrub.track.x, scrub.track.right().saturating_sub(1)) - scrub.track.x,
            ) / f64::from(scrub.track.width.max(1))
                * duration;
            if action == "up" {
                let scrub = state.video_scrub.take().unwrap();
                if let Some(video) = state.video.as_mut() {
                    video.restart(scrub.position);
                    video.set_control(scrub.paused, video.volume);
                }
                state.pointer_capture = None;
            }
            self.repaint = true;
            return true;
        }
        if action != "down" {
            return false;
        }
        let state = self.graphical.as_ref().unwrap();
        if let (Some(rect), Some(request), Some(controls)) = (
            state.video_control_rect,
            state.video_control_request.as_ref(),
            state.video_controls.as_ref(),
        ) {
            if rect.contains((x, y).into()) {
                let px = (u32::from(x - rect.x) * 2 + 1) * u32::from(request.width)
                    / (u32::from(rect.width) * 2);
                let py = (u32::from(y - rect.y) * 2 + 1) * u32::from(request.height)
                    / (u32::from(rect.height) * 2);
                controls.pointer(request.clone(), px as u16, py as u16);
                self.layout.focus_set(ModuleId::Preview);
                return true;
            }
        }
        let Some(track) = self.graphical.as_ref().unwrap().video_timeline else {
            return false;
        };
        if !track.contains((x, y).into()) {
            return false;
        }
        self.layout.focus_set(ModuleId::Preview);
        let duration = match self.view.preview.as_deref() {
            Some(Preview::Video { poster, .. }) => poster.duration,
            _ => 0.0,
        };
        let state = self.graphical.as_mut().unwrap();
        if let Some(video) = state.video.as_mut() {
            state.video_scrub = Some(VideoScrub {
                track,
                position: f64::from(x - track.x) / f64::from(track.width.max(1)) * duration,
                paused: video.paused,
            });
            video.set_control(true, video.volume);
            self.repaint = true;
        }
        true
    }

    fn reveal_video_controls(&mut self) {
        let state = self.graphical.as_mut().unwrap();
        state.video_overlay_at = Some(Instant::now());
        if !state.video_overlay_visible {
            state.video_overlay_visible = true;
            self.repaint = true;
        }
    }
    fn launch_movie(&mut self, path: String) {
        if !self.graphical.as_ref().unwrap().video_player {
            self.note = Some((
                "Update the graphical client for movie playback".into(),
                NoteLevel::Warning,
                Instant::now(),
            ));
            return;
        }
        let path = if let Some(rest) = path.strip_prefix("~/") {
            self.view.home.join(rest)
        } else {
            PathBuf::from(path)
        };
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| self.view.home.clone())
                .join(path)
        };
        let Some(parent) = path.parent() else { return };
        self.core.send(Command::ClearFilter);
        self.core.send(Command::SetHidden(true));
        self.core.send(Command::Push(parent.into()));
        let state = self.graphical.as_mut().unwrap();
        state.direct_play = Some(path);
        state.direct_fullscreen = true;
        self.repaint = true;
    }
    fn toggle_video_fullscreen(&mut self, desktop: bool) {
        if self.graphical.as_ref().unwrap().video.is_none() {
            self.video_action(panels::Word::VideoPlay(true));
        }
        let state = self.graphical.as_mut().unwrap();
        if state.video_fullscreen == Some(desktop) {
            self.exit_video_fullscreen();
            return;
        }
        if !state.video_player {
            return;
        }
        state.video_fullscreen = Some(desktop);
        state
            .effects
            .push(ServerMessage::Fullscreen { enabled: desktop });
        self.layout.focus_set(ModuleId::Preview);
        self.reveal_video_controls();
        self.repaint = true;
    }
    fn exit_video_fullscreen(&mut self) {
        let state = self.graphical.as_mut().unwrap();
        if state.video_fullscreen.take().is_some() {
            state
                .effects
                .push(ServerMessage::Fullscreen { enabled: false });
        }
        state.video_picker = None;
        state.video_picker_rect = None;
        state.video_subtitle_input = None;
        self.repaint = true;
    }
    fn fullscreen_video_scene(&mut self, viewport: Viewport) -> Scene {
        let area = Rect::new(0, 0, viewport.columns, viewport.rows);
        let mut scene = Scene::from_buffer(&Buffer::empty(area), viewport, 0);
        scene.spans.clear();
        scene.background = "#000000".into();
        scene.accent = hex(self.theme.accent);
        scene.border = hex(self.theme.border);
        let state = self.graphical.as_mut().unwrap();
        if let Some(video) = &mut state.video {
            video.set_bounds(viewport.width, viewport.height);
            scene.components.push(Component::Image {
                rect: area.into(),
                id: format!("video-{}-{}", video.session, video.generation),
                png: None,
                scale: Default::default(),
                zoom: 100,
            });
        }
        if let Some(regions) = self.layout.last.clone() {
            self.video_scene(&mut scene, &regions);
        }
        self.video_picker_scene(&mut scene);
        let state = self.graphical.as_ref().unwrap();
        scene.interaction = state
            .video
            .as_ref()
            .map_or(0, |v| v.generation)
            .wrapping_add(if state.video_picker.is_some() {
                1 << 32
            } else {
                0
            });
        scene
    }
    fn video_choices(&self, audio: bool) -> Vec<(String, starkit::media::tracks::Selection)> {
        use starkit::media::tracks::Selection;
        let mut entries = vec![
            (
                if audio {
                    "Automatic"
                } else {
                    "Forced tracks automatically"
                }
                .into(),
                Selection::Auto,
            ),
            ("Off".into(), Selection::Off),
        ];
        if let Some(video) = self.graphical.as_ref().unwrap().video.as_ref() {
            let tracks = if audio {
                &video.tracks.audio
            } else {
                &video.tracks.subtitles
            };
            entries.extend(
                tracks
                    .iter()
                    .map(|track| (track.label.clone(), track.selection.clone())),
            );
        }
        if !audio {
            entries.push(("Load subtitle file…".into(), Selection::Off));
        }
        entries
    }
    fn open_video_picker(&mut self, audio: bool) {
        if self.graphical.as_ref().unwrap().video.is_none() {
            self.video_action(panels::Word::VideoPlay(true));
        }
        let entries = self.video_choices(audio);
        let state = self.graphical.as_mut().unwrap();
        let selection = state.video.as_ref().map(|v| {
            if audio {
                &v.options.audio
            } else {
                &v.options.subtitle
            }
        });
        state.video_picker_index = selection
            .and_then(|selection| entries.iter().position(|(_, s)| s == selection))
            .unwrap_or(0);
        state.video_picker = Some(audio);
        state.video_subtitle_input = None;
        self.reveal_video_controls();
        self.repaint = true;
    }
    fn choose_video_track(&mut self, index: usize) {
        let Some(audio) = self.graphical.as_ref().unwrap().video_picker else {
            return;
        };
        let entries = self.video_choices(audio);
        if !audio
            && index > 1
            && self
                .graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .is_some_and(|v| v.original)
        {
            self.note = Some((
                "Subtitles currently require Preview mode; press O to change mode".into(),
                NoteLevel::Warning,
                Instant::now(),
            ));
            self.repaint = true;
            return;
        }
        if !audio && index + 1 == entries.len() {
            self.graphical.as_mut().unwrap().video_subtitle_input = Some(String::new());
            self.repaint = true;
            return;
        }
        let Some((_, selection)) = entries.get(index) else {
            return;
        };
        if !audio
            && self
                .graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .is_some_and(|v| v.original)
            && !matches!(
                selection,
                starkit::media::tracks::Selection::Off | starkit::media::tracks::Selection::Auto
            )
        {
            self.note = Some((
                "Subtitles currently require Preview mode; press O to change mode".into(),
                NoteLevel::Warning,
                Instant::now(),
            ));
            self.repaint = true;
            return;
        }
        if let Some(video) = self.graphical.as_mut().unwrap().video.as_mut() {
            if let Err(error) = video.select(audio, selection.clone()) {
                video.warning = Some(error.to_string());
            }
        }
        let state = self.graphical.as_mut().unwrap();
        state.video_picker = None;
        state.video_picker_rect = None;
        self.repaint = true;
    }
    fn video_picker_key(&mut self, code: &str, modifiers: u8) -> bool {
        if self.graphical.as_ref().unwrap().video_picker.is_none() {
            return false;
        }
        let code = code.strip_prefix("char:").unwrap_or(code);
        if self
            .graphical
            .as_ref()
            .unwrap()
            .video_subtitle_input
            .is_some()
        {
            match code {
                "escape" => self.graphical.as_mut().unwrap().video_subtitle_input = None,
                "backspace" => {
                    self.graphical
                        .as_mut()
                        .unwrap()
                        .video_subtitle_input
                        .as_mut()
                        .unwrap()
                        .pop();
                }
                "enter" => {
                    let text = self
                        .graphical
                        .as_mut()
                        .unwrap()
                        .video_subtitle_input
                        .take()
                        .unwrap();
                    let path = if let Some(rest) = text.strip_prefix("~/") {
                        self.view.home.join(rest)
                    } else {
                        PathBuf::from(text)
                    };
                    if let Some(video) = self.graphical.as_mut().unwrap().video.as_mut() {
                        if let Err(error) = video.load_subtitle(path) {
                            video.warning = Some(error.to_string());
                        }
                    }
                    self.graphical.as_mut().unwrap().video_picker = None;
                }
                value
                    if modifiers & !starkit::crossterm::event::KeyModifiers::SHIFT.bits() == 0
                        && value.chars().count() == 1 =>
                {
                    let input = self
                        .graphical
                        .as_mut()
                        .unwrap()
                        .video_subtitle_input
                        .as_mut()
                        .unwrap();
                    if input.len() < 4096 {
                        input.push_str(value);
                    }
                }
                _ => {}
            }
        } else {
            let len = self
                .video_choices(self.graphical.as_ref().unwrap().video_picker.unwrap())
                .len();
            let index = self.graphical.as_ref().unwrap().video_picker_index;
            match code {
                "escape" => {
                    self.graphical.as_mut().unwrap().video_picker = None;
                }
                "down" | "j" => {
                    self.graphical.as_mut().unwrap().video_picker_index =
                        (index + 1).min(len.saturating_sub(1))
                }
                "up" | "k" => {
                    self.graphical.as_mut().unwrap().video_picker_index = index.saturating_sub(1)
                }
                "enter" | "space" | " " => self.choose_video_track(index),
                _ => {}
            }
        }
        self.repaint = true;
        true
    }
    fn video_picker_pointer(&mut self, action: &str, button: u8, x: u16, y: u16) -> bool {
        let state = self.graphical.as_ref().unwrap();
        if state.video_picker.is_none() {
            return false;
        }
        if action == "down" && button == 0 {
            if let (Some(rect), Some(request), Some(controls)) = (
                state.video_picker_rect,
                state.video_picker_request.as_ref(),
                state.video_picker_controls.as_ref(),
            ) {
                if rect.contains(starkit::ratatui::layout::Position::new(x, y)) {
                    let cw = request.width / rect.width.max(1);
                    let ch = request.height / rect.height.max(1);
                    controls.pointer(
                        request.clone(),
                        (x - rect.x) * cw + cw / 2,
                        (y - rect.y) * ch + ch / 2,
                    );
                }
            }
        }
        true
    }
    fn video_picker_scene(&mut self, scene: &mut Scene) {
        let Some(audio) = self.graphical.as_ref().unwrap().video_picker else {
            return;
        };
        let entries = self
            .video_choices(audio)
            .into_iter()
            .map(|(label, _)| label)
            .collect::<Vec<_>>();
        let cw = (scene.viewport.width / u32::from(scene.viewport.columns)).max(1) as u16;
        let ch = (scene.viewport.height / u32::from(scene.viewport.rows)).max(1) as u16;
        let font = (ch * 84 / 100).max(10);
        let width = scene.viewport.columns.saturating_sub(4).clamp(1, 80);
        let desired = ((entries.len() + 1) as u16).saturating_mul(font + 12) / ch + 2;
        let height = desired.min(scene.viewport.rows.saturating_sub(2)).max(1);
        let rect = Rect::new(
            (scene.viewport.columns - width) / 2,
            (scene.viewport.rows - height) / 2,
            width,
            height,
        );
        let state = self.graphical.as_mut().unwrap();
        let title = if let Some(input) = &state.video_subtitle_input {
            format!("Subtitle path: {input} · Enter to load")
        } else if audio {
            "Audio tracks".into()
        } else {
            "Subtitles".into()
        };
        let request = crate::video_transport::Request {
            cells: state.cell_mode.then_some([width, height]),
            width: width.saturating_mul(cw),
            height: height.saturating_mul(ch),
            theme: crate::audio_embed::Palette {
                bg: [
                    self.theme.panel_bg.r,
                    self.theme.panel_bg.g,
                    self.theme.panel_bg.b,
                ],
                fg: [self.theme.fg.r, self.theme.fg.g, self.theme.fg.b],
                muted: [self.theme.dim.r, self.theme.dim.g, self.theme.dim.b],
                accent: [
                    self.theme.accent.r,
                    self.theme.accent.g,
                    self.theme.accent.b,
                ],
                selected: [
                    self.theme.row_selected_bg.r,
                    self.theme.row_selected_bg.g,
                    self.theme.row_selected_bg.b,
                ],
                border: [
                    self.theme.border.r,
                    self.theme.border.g,
                    self.theme.border.b,
                ],
                error: [self.theme.error.r, self.theme.error.g, self.theme.error.b],
            },
            playing: false,
            paused: false,
            volume: 0.0,
            pointer: None,
            movie: true,
            picker: true,
            entries: if state.video_subtitle_input.is_some() {
                vec!["Type a path on the movie’s host; Enter loads it; Esc returns".into()]
            } else {
                entries
            },
            selected: state.video_picker_index,
            title,
            font,
        };
        let helper = state
            .video_picker_controls
            .get_or_insert_with(crate::video_transport::Client::new);
        if let Some(surface) = helper.render(request.clone()) {
            if state.cell_mode {
                if let Some(cells) = helper.cells(&request) {
                    transport_cell_spans(scene, rect, &cells);
                }
            }
            // Menu marker creates a dedicated overlay placement in ordinary Preview.
            if state.video_fullscreen.is_none() {
                scene.components.push(Component::Menu { rect: rect.into() });
            }
            scene.components.push(Component::Surface {
                rect: rect.into(),
                surface,
            });
        }
        state.video_picker_request = Some(request);
        state.video_picker_rect = Some(rect);
        scene.pointer_regions.push(rect.into());
    }

    fn video_scene(&mut self, scene: &mut Scene, regions: &Regions) {
        if self.movie_details_preview().is_some() {
            return;
        }
        if self.video_words().is_none() {
            return;
        }
        let Some(Preview::Video { poster, .. }) = self.view.preview.as_deref() else {
            return;
        };
        let full = self.graphical.as_ref().unwrap().video_fullscreen.is_some();
        if full && !self.graphical.as_ref().unwrap().video_overlay_visible {
            let state = self.graphical.as_mut().unwrap();
            state.video_control_rect = None;
            state.video_timeline = None;
            return;
        }
        let mut rect = if full {
            Rect::new(
                1,
                0,
                scene.viewport.columns.saturating_sub(2),
                scene.viewport.rows,
            )
        } else {
            video_body(
                regions.rect_of(ModuleId::Preview),
                self.graphical.as_ref().unwrap().uses_pixel_layout(),
            )
        };
        if rect.height < 3 {
            return;
        }
        let controls_rect = (rect.height >= 6)
            .then(|| Rect::new(rect.x, rect.bottom() - 2, rect.width.min(256), 2));
        rect.y += rect.height - if controls_rect.is_some() { 3 } else { 1 };
        rect.height = 1;
        let spinner = self.unmount_spinner();
        let state = self.graphical.as_mut().unwrap();
        state.video_timeline = Some(rect);
        let position = state.video_scrub.as_ref().map_or_else(
            || state.video.as_ref().map_or(0.0, |v| v.position),
            |scrub| scrub.position,
        );
        let cw = (scene.viewport.width / u32::from(scene.viewport.columns.max(1))) as u16;
        let ch = (scene.viewport.height / u32::from(scene.viewport.rows.max(1))) as u16;
        let surface = starkit::media::timeline(
            rect.width.saturating_mul(cw).max(1),
            ch.max(1),
            position,
            poster.duration,
            hex(self.theme.panel_bg),
            if self.theme.variant == starkit::theme::Variant::Dark {
                let bg = self.theme.panel_bg;
                hex(starkit::theme::color::Rgb::new(
                    bg.r.saturating_add(16),
                    bg.g.saturating_add(16),
                    bg.b.saturating_add(16),
                ))
            } else {
                hex(self.theme.dim)
            },
            hex(self.theme.accent),
        );
        if state.cell_mode {
            scene.components.push(Component::Meter {
                rect: rect.into(),
                value: if poster.duration > 0. {
                    (position / poster.duration * 1000.).clamp(0., 1000.) as u16
                } else {
                    0
                },
                foreground: hex(self.theme.accent),
                background: hex(self.theme.border),
            });
            scene
                .spans
                .retain(|s| s.y != rect.y || s.x < rect.x || s.x >= rect.right());
        }
        scene.components.push(Component::Surface {
            rect: rect.into(),
            surface,
        });
        if let Some(video) = state.video.as_ref() {
            let mut text = format!(
                "{:02}:{:02} / {:02}:{:02} · {}% volume · {} · source {}×{}",
                position as u64 / 60,
                position as u64 % 60,
                poster.duration as u64 / 60,
                poster.duration as u64 % 60,
                video.volume,
                Self::video_stream_label(video),
                poster.width,
                poster.height,
            );
            if video.buffering && !video.finished {
                text = format!(
                    "{spinner} {} · {text}",
                    if video.position > 0. {
                        "Buffering movie…"
                    } else {
                        "Starting playback…"
                    }
                );
            }
            if let Some(warning) = &video.warning {
                text.push_str(&format!(" · {warning}"));
            }
            let info = Rect::new(rect.x, rect.y.saturating_sub(1), rect.width, 1);
            if state.cell_mode {
                scene
                    .spans
                    .retain(|s| s.y != info.y || s.x < info.x || s.x >= info.right());
                scene.spans.push(Span {
                    x: info.x,
                    y: info.y,
                    text: starkit::text::truncate(&text, usize::from(info.width)),
                    foreground: hex(self.theme.dim),
                    background: hex(self.theme.panel_bg),
                    bold: false,
                    modifiers: 0,
                });
            }
            let mut surface = starkit::native_surface::Surface::new(
                info.width.saturating_mul(cw).max(1),
                ch.max(1),
                hex(self.theme.panel_bg),
            );
            surface.text(
                starkit::native_surface::PixelRect::new(0, 0, surface.width, surface.height),
                starkit::text::truncate(&text, usize::from(info.width)),
                &hex(self.theme.dim),
                starkit::native_surface::Metrics::from_cell(cw, ch).font,
                false,
            );
            scene.components.push(Component::Surface {
                rect: info.into(),
                surface,
            });
        }
        if let Some(controls_rect) = controls_rect {
            let rgb = |c: starkit::theme::color::Rgb| [c.r, c.g, c.b];
            let request = crate::video_transport::Request {
                cells: state
                    .cell_mode
                    .then_some([controls_rect.width, controls_rect.height]),
                width: controls_rect.width.saturating_mul(cw).clamp(1, 8192),
                height: controls_rect.height.saturating_mul(ch).clamp(1, 128),
                theme: crate::audio_embed::Palette {
                    bg: rgb(self.theme.panel_bg),
                    fg: rgb(self.theme.panel_fg),
                    muted: rgb(self.theme.dim),
                    accent: rgb(self.theme.accent),
                    selected: rgb(self.theme.row_selected_bg),
                    border: rgb(self.theme.border),
                    error: rgb(self.theme.error),
                },
                playing: state
                    .video
                    .as_ref()
                    .is_some_and(|v| !v.paused && !v.finished),
                paused: state
                    .video
                    .as_ref()
                    .is_some_and(|v| v.paused && !v.finished),
                volume: f32::from(state.video.as_ref().map_or(80, |v| v.volume)) / 100.0,
                pointer: None,
                movie: state.video_player,
                picker: false,
                entries: vec![],
                selected: 0,
                title: String::new(),
                font: (ch * 84 / 100).max(10),
            };
            let controls = state
                .video_controls
                .get_or_insert_with(crate::video_transport::Client::new);
            if let Some(surface) = controls.render(request.clone()) {
                if state.cell_mode {
                    if let Some(cells) = controls.cells(&request) {
                        transport_cell_spans(scene, controls_rect, &cells);
                    }
                }
                scene.components.push(Component::Surface {
                    rect: controls_rect.into(),
                    surface,
                });
                scene.pointer_regions.push(controls_rect.into());
                state.video_control_rect = Some(controls_rect);
                state.video_control_request = Some(request);
            } else {
                state.video_control_rect = None;
                state.video_control_request = None;
            }
        } else {
            state.video_controls = None;
            state.video_control_rect = None;
            state.video_control_request = None;
        }
        scene.pointer_regions.push(rect.into());
    }
}

fn transport_cell_spans(
    scene: &mut Scene,
    rect: Rect,
    grid: &crate::video_transport::CellTransport,
) {
    for y in 0..grid.rows.min(rect.height) {
        let mut covered_until = 0;
        for x in 0..grid.columns.min(rect.width) {
            let cell = &grid.cells[usize::from(y) * usize::from(grid.columns) + usize::from(x)];
            if x < covered_until {
                continue;
            }
            covered_until = x.saturating_add(starkit::wrap::width_of(&cell.symbol));
            let hex = |[r, g, b]: [u8; 3]| format!("#{r:02x}{g:02x}{b:02x}");
            scene.spans.push(Span {
                x: rect.x + x,
                y: rect.y + y,
                text: cell.symbol.clone(),
                foreground: hex(cell.fg),
                background: hex(cell.bg),
                bold: false,
                modifiers: cell.modifiers,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn switching_presentation_preserves_controller_state_and_theme_shortcuts() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        fake.pump();
        app.refresh();
        app.enable_graphical();
        app.graphical.as_mut().unwrap().presentation_switch = true;
        app.graphical.as_mut().unwrap().video_capable = true;
        let viewport = Viewport::default();
        Controller::scene(&mut app, viewport);
        let cursor = app.view.cursor_path.clone();
        let theme = app.theme_name.clone();
        Controller::input(
            &mut app,
            Input::Key {
                code: "f:8".into(),
                modifiers: 0,
            },
        );
        assert_ne!(app.theme_name, theme);
        Controller::input(
            &mut app,
            Input::Key {
                code: "f:9".into(),
                modifiers: 0,
            },
        );
        assert!(Controller::effects(&mut app)
            .iter()
            .any(|e| matches!(e, ServerMessage::TogglePresentation)));
        Controller::presentation(&mut app, true);
        assert!(app.graphical.as_ref().unwrap().cell_mode);
        assert!(app.graphical.as_ref().unwrap().can_play_video());
        assert_eq!(app.view.cursor_path, cursor);
        let cells = Controller::scene(
            &mut app,
            Viewport {
                generation: 2,
                ..viewport
            },
        );
        assert!(cells.placements.is_empty());
        Controller::presentation(&mut app, false);
        Controller::scene(
            &mut app,
            Viewport {
                generation: 3,
                ..viewport
            },
        );
        assert_eq!(app.view.cursor_path, cursor);
    }

    #[test]
    fn missing_saved_video_is_reported_and_does_not_retry_forever() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        fake.pump();
        app.refresh();
        app.enable_graphical();
        app.graphical.as_mut().unwrap().video_capable = true;
        app.layout.preview_open = true;
        app.resume_video = Some(session::VideoSession {
            path: fake.home().join("missing-movie.mkv"),
            position: 37.,
            ..Default::default()
        });
        app.tick_video();
        assert!(app.resume_video.is_none());
        assert!(app.graphical.as_ref().unwrap().direct_play.is_none());
        assert!(app.note.as_ref().unwrap().0.contains("Movie not found"));
    }

    #[test]
    fn archive_video_pending_tracks_member_identity() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(
            core,
            cfg,
            dir.path().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.layout.preview_open = true;
        let local = dir.path().join("movie.mkv");
        let key = crate::fold::location::Location::Archive {
            source: crate::fold::location::ArchiveSource {
                file: dir.path().join("movies.zip"),
                nested: vec![],
            },
            directory: PathBuf::new(),
            member: Some(crate::fold::location::Member {
                index: 0,
                name: "movie.mkv".into(),
            }),
        }
        .key();
        fake.state_mut()
            .archive_materialized
            .insert(key.clone(), local.clone());
        app.view.cursor_path = Some(key);
        app.graphical.as_mut().unwrap().video_pending = Some(local.clone());
        app.tick_video();
        assert_eq!(
            app.graphical.as_ref().unwrap().video_pending.as_ref(),
            Some(&local)
        );
        app.view.cursor_path = Some(dir.path().join("other.txt"));
        app.tick_video();
        assert!(app.graphical.as_ref().unwrap().video_pending.is_none());
    }

    #[test]
    fn video_resume_survives_detach_and_shutdown_at_saved_position() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let dir = tempfile::tempdir().unwrap();
        let saved_path = dir.path().join("session.toml");
        let mut app = App::new(
            core,
            cfg,
            dir.path().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.session_writer = session::Writer::acquire(saved_path.clone()).unwrap();
        app.enable_graphical();
        let path = fake.home().join("movie.mkv");
        app.view.cursor_path = Some(path.clone());
        app.view.preview = Some(Arc::new(Preview::Video {
            metadata: std::sync::Arc::new(crate::fold::preview::model::Document::new(
                "Movie details",
            )),
            extension: None,
            path: path.clone(),
            poster: starkit::media::Poster {
                pixels: Arc::new(RgbaImage::new(32, 24)),
                duration: 60.,
                width: 32,
                height: 24,
                audio: true,
            },
        }));
        app.layout.preview_open = true;
        app.layout.native_preview_rows = Some(17);
        let state = app.graphical.as_mut().unwrap();
        state.video_capable = true;
        state.video_player = true;
        state.local_media = true;
        app.video_action(panels::Word::VideoPlay(true));
        let options = starkit::media::tracks::PlaybackOptions {
            audio: starkit::media::tracks::Selection::Stream(2),
            subtitle: starkit::media::tracks::Selection::Off,
        };
        app.graphical
            .as_mut()
            .unwrap()
            .video
            .as_mut()
            .unwrap()
            .restore_playback(37.125, options.clone(), true, 65);
        app.tick_video();
        assert!(app.status_view(Instant::now()).progress.is_none());
        app.view.running_bar = Some("Copying files · 50%".into());
        assert_eq!(
            app.status_view(Instant::now()).progress,
            Some("Copying files · 50%")
        );
        app.view.running_bar = None;
        let remembered = app.graphical_video_snapshot().unwrap();
        Controller::detached(&mut app);
        assert_eq!(app.resume_video, Some(remembered.clone()));
        assert_eq!(app.layout.native_preview_rows, Some(17));
        app.tick_video();
        assert!(app.graphical.as_ref().unwrap().video.is_none());
        assert_eq!(app.resume_video, Some(remembered.clone()));
        Controller::attached(&mut app);
        app.graphical.as_mut().unwrap().video_capable = true;
        app.activate_video_entry(path);
        app.tick_video();
        let video = app.graphical.as_ref().unwrap().video.as_ref().unwrap();
        assert_eq!(video.position, 37.125);
        assert!(video.paused);
        assert_eq!(video.volume, 65);
        assert_eq!(video.options, options);
        Controller::shutdown(&mut app);
        let saved = session::load(&saved_path);
        assert_eq!(saved.tabs[saved.active_tab].video, Some(remembered));
        assert_eq!(saved.tabs[saved.active_tab].native_preview_rows, Some(17));
    }

    use super::*;
    #[test]
    fn frontend_update_notice_waits_for_dialog_and_does_not_set_video_warning() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.graphical = Some(State::default());
        app.overlays.open_help();
        Controller::input(&mut app, Input::Notice { message: "STARFOLD_UPDATE\nPresentation machine update\nUpdated to STAR/FOLD 9.0.0\nPlayback improved".into() });
        app.tick();
        assert!(matches!(
            app.overlays.current(),
            Some(super::super::super::overlays::Overlay::Help { .. })
        ));
        assert!(app.note.is_none());
        app.overlays.close();
        app.tick();
        let Some(super::super::super::overlays::Overlay::Update(notice)) = app.overlays.current()
        else {
            panic!("expected update dialog")
        };
        assert!(notice.lines.iter().any(|s| s.contains("9.0.0")));
        assert!(notice.lines.iter().any(|s| s == "Playback improved"));
    }
    #[test]
    fn failed_local_audio_does_not_switch_on_host_output() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.graphical = Some(State {
            audio_local: true,
            audio_epoch: Some(7),
            ..State::default()
        });
        let session = app.audio_activation;
        Controller::media(
            &mut app,
            starkit::terminal_graphics::media::ToHost::AudioError {
                session,
                epoch: 7,
                message: "No local device".into(),
            },
        );
        assert!(app.graphical.as_ref().unwrap().audio_local);
        assert!(app.graphical.as_ref().unwrap().audio_epoch.is_none());
        assert_eq!(app.audio_error.as_deref(), Some("No local device"));
    }
    #[test]
    fn audio_output_toggle_is_only_offered_to_remote_capable_clients() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.graphical = Some(State::default());
        app.audio_path = Some(std::path::PathBuf::from("/music/test.wav"));
        app.audio_tab = Some(app.core.state().tabs.active().id);
        assert!(app.audio_output_word().is_none());
        app.graphical.as_mut().unwrap().audio_relay_capable = true;
        assert_eq!(
            app.audio_output_word(),
            Some(panels::Word::AudioOutput(false))
        );
        app.graphical.as_mut().unwrap().audio_local = true;
        assert_eq!(
            app.audio_output_word(),
            Some(panels::Word::AudioOutput(true))
        );
        app.graphical.as_mut().unwrap().cell_mode = true;
        assert_eq!(
            app.audio_output_word(),
            Some(panels::Word::AudioOutput(true))
        );
    }

    #[test]
    fn fullscreen_track_selection_and_escape_preserve_playback_and_layout() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        let state = app.graphical.as_mut().unwrap();
        state.video_capable = true;
        state.video_player = true;
        state.local_media = true;
        app.view.preview = Some(Arc::new(Preview::Video {
            metadata: std::sync::Arc::new(crate::fold::preview::model::Document::new(
                "Movie details",
            )),
            extension: None,
            path: fake.home().join("movie.mkv"),
            poster: starkit::media::Poster {
                pixels: Arc::new(RgbaImage::new(32, 24)),
                duration: 60.0,
                width: 32,
                height: 24,
                audio: true,
            },
        }));
        app.layout.preview_open = true;
        app.layout.focus_set(ModuleId::Preview);
        let viewport = Viewport::default();
        Controller::scene(&mut app, viewport);
        let rows = app.layout.native_preview_rows;
        assert!(app.video_key("char:f", 0));
        assert_eq!(app.graphical.as_ref().unwrap().video_fullscreen, Some(true));
        let scene = Controller::scene(&mut app, viewport);
        assert!(scene.components.iter().any(|c|matches!(c,Component::Image {rect,..}if rect.width==viewport.columns&&rect.height==viewport.rows)));
        assert!(!scene
            .components
            .iter()
            .any(|c| matches!(c, Component::ListRow { .. } | Component::Tab { .. })));
        assert_eq!(rows, app.layout.native_preview_rows);
        assert!(app.video_key("char:a", 0));
        let generation = app
            .graphical
            .as_ref()
            .unwrap()
            .video
            .as_ref()
            .unwrap()
            .generation;
        assert!(app.video_picker_key("down", 0));
        assert!(app.video_picker_key("enter", 0));
        let video = app.graphical.as_ref().unwrap().video.as_ref().unwrap();
        assert_eq!(video.options.audio, starkit::media::tracks::Selection::Off);
        assert!(video.generation > generation);
        assert!(app.video_key("escape", 0));
        assert!(app.graphical.as_ref().unwrap().video_fullscreen.is_none());
        assert!(app.graphical.as_ref().unwrap().video.is_some());
        assert_eq!(rows, app.layout.native_preview_rows);
        // Legacy terminal input expresses Shift+F as an uppercase character.
        assert!(app.video_key("char:F", 0));
        assert_eq!(
            app.graphical.as_ref().unwrap().video_fullscreen,
            Some(false)
        );
        assert!(app.video_key("escape", 0));
        assert!(app.video_key(
            "char:f",
            starkit::crossterm::event::KeyModifiers::SHIFT.bits()
        ));
        assert_eq!(
            app.graphical.as_ref().unwrap().video_fullscreen,
            Some(false)
        );
    }

    #[test]
    fn original_quality_is_negotiated_and_explicit_preview_remains_available() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.graphical.as_mut().unwrap().video_capable = true;
        app.graphical.as_mut().unwrap().original_media = true;
        app.view.preview = Some(Arc::new(Preview::Video {
            metadata: std::sync::Arc::new(crate::fold::preview::model::Document::new(
                "Movie details",
            )),
            extension: None,
            path: fake.home().join("original.mkv"),
            poster: starkit::media::Poster {
                pixels: Arc::new(RgbaImage::new(32, 24)),
                duration: 60.0,
                width: 3840,
                height: 2160,
                audio: true,
            },
        }));
        app.layout.preview_open = true;
        app.video_action(panels::Word::VideoPlay(true));
        assert!(
            app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .original
        );
        assert!(app
            .video_words()
            .unwrap()
            .contains(&panels::Word::VideoMode(0)));
        app.word_click(panels::Word::VideoMode(0));
        assert_eq!(app.graphical.as_ref().unwrap().video_mode, 1);
        let generation = app
            .graphical
            .as_ref()
            .unwrap()
            .video
            .as_ref()
            .unwrap()
            .generation;
        app.word_click(panels::Word::VideoMode(1));
        let state = app.graphical.as_ref().unwrap();
        assert_eq!(state.video_mode, 2);
        assert!(!state.video.as_ref().unwrap().original);
        assert!(state.video.as_ref().unwrap().generation > generation);
        app.graphical
            .as_mut()
            .unwrap()
            .video
            .as_mut()
            .unwrap()
            .options
            .subtitle = starkit::media::tracks::Selection::Stream(4);
        app.word_click(panels::Word::VideoMode(2));
        assert_eq!(app.graphical.as_ref().unwrap().video_mode, 2);
        assert!(
            !app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .original
        );
        app.graphical
            .as_mut()
            .unwrap()
            .video
            .as_mut()
            .unwrap()
            .options
            .subtitle = starkit::media::tracks::Selection::Off;
        app.word_click(panels::Word::VideoMode(2));
        assert!(
            app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .original
        );
        app.stop_video();
        app.graphical.as_mut().unwrap().original_media = false;
        app.video_action(panels::Word::VideoPlay(true));
        assert!(
            !app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .original
        );
        assert!(!app
            .video_words()
            .unwrap()
            .iter()
            .any(|w| matches!(w, panels::Word::VideoMode(_))));
    }

    #[test]
    fn video_activation_waits_for_the_selected_poster_without_starting_amp() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.graphical.as_mut().unwrap().video_capable = true;
        let path = fake.home().join("movie.mkv");
        app.view.cursor_path = Some(path.clone());
        app.activate_video_entry(path.clone());
        app.tick_video();
        assert!(app.audio_path.is_none());
        assert!(app.graphical.as_ref().unwrap().video.is_none());
        assert_eq!(
            app.graphical.as_ref().unwrap().video_pending.as_ref(),
            Some(&path)
        );
        app.view.preview = Some(Arc::new(Preview::Video {
            metadata: std::sync::Arc::new(crate::fold::preview::model::Document::new(
                "Movie details",
            )),
            extension: None,
            path: path.clone(),
            poster: starkit::media::Poster {
                pixels: Arc::new(RgbaImage::new(32, 24)),
                duration: 60.0,
                width: 32,
                height: 24,
                audio: true,
            },
        }));
        app.tick_video();
        assert!(app.audio_path.is_none());
        let state = app.graphical.as_ref().unwrap();
        assert!(state.video_pending.is_none());
        assert_eq!(state.video_path.as_ref(), Some(&path));
        assert_eq!(state.video.as_ref().unwrap().volume, 80);
        assert!(!state.video.as_ref().unwrap().paused);
        app.stop_video();
        app.activate_video_entry(path);
        app.view.cursor_path = None;
        app.tick_video();
        assert!(app.graphical.as_ref().unwrap().video_pending.is_none());
    }

    #[test]
    fn video_preview_mouse_keys_seek_and_expand_preserve_height() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        let state = app.graphical.as_mut().unwrap();
        state.video_capable = true;
        state.local_media = true;
        state.surface_mode = true;
        state.pixel_layout = true;
        app.layout.preview_open = true;
        app.view.preview = Some(Arc::new(Preview::Video {
            metadata: std::sync::Arc::new(crate::fold::preview::model::Document::new(
                "Movie details",
            )),
            extension: None,
            path: fake.home().join("movie.mp4"),
            poster: starkit::media::Poster {
                pixels: Arc::new(RgbaImage::new(32, 24)),
                duration: 60.0,
                width: 32,
                height: 24,
                audio: true,
            },
        }));
        let viewport = Viewport {
            columns: 100,
            rows: 40,
            width: 1200,
            height: 800,
            ..Viewport::default()
        };
        let first = Controller::scene(&mut app, viewport);
        assert!(
            app.graphical.as_ref().unwrap().video.is_none(),
            "Selecting a video must not start playback"
        );
        let regions = app.layout.last.clone().unwrap();
        let height = regions.rect_of(ModuleId::Preview).height;
        let play = starkit::chrome::header::slots(
            regions.rect_of(ModuleId::Preview),
            &app.panel_words(ModuleId::Preview),
        )
        .into_iter()
        .find(|(w, _)| matches!(w, panels::Word::VideoPlay(true)))
        .unwrap()
        .1;
        app.last_preview_for = Some(fake.home().join("movie.mp4"));
        let placement = first
            .placements
            .iter()
            .find(|p| p.source == regions.rect_of(ModuleId::Preview).into())
            .unwrap();
        let px = u32::from(placement.target.x)
            + u32::from(play.x - placement.source.x + play.width / 2)
                * u32::from(placement.target.width)
                / u32::from(placement.source.width);
        let row = play.y - placement.source.y;
        let py = u32::from(placement.target.y)
            + ((placement.row_edge(row) + placement.row_edge(row + 1)) / 2.0) as u32;
        Controller::input(
            &mut app,
            Input::Pointer {
                action: "down".into(),
                button: 0,
                x: 0,
                y: 0,
                pixel: Some([px, py]),
                modifiers: 0,
            },
        );
        let v = app.graphical.as_ref().unwrap().video.as_ref().unwrap();
        assert_eq!(
            app.last_preview_for,
            Some(fake.home().join("movie.mp4")),
            "Starting video must not invalidate the loaded preview"
        );
        assert_eq!(v.volume, 80);
        assert!(!v.paused);
        assert!(app.video_key("char:p", 0));
        assert!(
            app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .paused
        );
        assert!(app.video_key("char:-", 0));
        assert!(app
            .panel_words(ModuleId::Preview)
            .contains(&panels::Word::VideoMute(70)));
        assert!(app.video_key(
            "char:A",
            starkit::crossterm::event::KeyModifiers::SHIFT.bits()
        ));
        assert_eq!(
            app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .volume,
            0
        );
        assert!(app.video_key("char:m", 0));
        assert_eq!(
            app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .volume,
            70
        );
        assert!(app.video_key("char:a", 0));
        assert_eq!(
            app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .volume,
            0
        );
        assert!(app.video_key("char:m", 0));
        assert_eq!(
            app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .volume,
            70
        );
        let previous = app
            .graphical
            .as_ref()
            .unwrap()
            .video
            .as_ref()
            .unwrap()
            .generation;
        assert!(app.video_key("right", 0));
        assert_eq!(
            app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .position,
            5.0
        );
        assert!(
            app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .generation
                > previous
        );
        Controller::scene(&mut app, viewport);
        assert_eq!(
            app.layout
                .last
                .as_ref()
                .unwrap()
                .rect_of(ModuleId::Preview)
                .height,
            height
        );
        assert!(app.video_key("char:e", 0));
        Controller::scene(&mut app, viewport);
        assert!(
            app.layout
                .last
                .as_ref()
                .unwrap()
                .rect_of(ModuleId::Preview)
                .height
                > height
        );
        assert!(app.video_key("escape", 0));
        Controller::scene(&mut app, viewport);
        assert_eq!(
            app.layout
                .last
                .as_ref()
                .unwrap()
                .rect_of(ModuleId::Preview)
                .height,
            height
        );
        assert!(!first.pointer_regions.is_empty());
        let track = app.graphical.as_ref().unwrap().video_timeline.unwrap();
        app.graphical
            .as_mut()
            .unwrap()
            .video
            .as_mut()
            .unwrap()
            .set_control(false, 80);
        let generation = app
            .graphical
            .as_ref()
            .unwrap()
            .video
            .as_ref()
            .unwrap()
            .generation;
        assert!(app.video_pointer("down", 0, track.x, track.y));
        for offset in 1..20 {
            assert!(app.video_pointer("drag", 0, track.x + offset, track.y));
        }
        let video = app.graphical.as_ref().unwrap().video.as_ref().unwrap();
        assert_eq!(
            video.generation, generation,
            "Dragging must not restart the encoder"
        );
        assert!(video.paused);
        assert!(app.video_pointer("up", 0, track.x + track.width / 2, track.y));
        let video = app.graphical.as_ref().unwrap().video.as_ref().unwrap();
        assert_eq!(
            video.generation,
            generation + 1,
            "Release must seek exactly once"
        );
        assert!(!video.paused);
        assert!(video.position > 25.0 && video.position <= 30.0);
        assert!(app.video_pointer("down", 0, track.x, track.y));
        Controller::input(&mut app, Input::CancelPointer);
        let video = app.graphical.as_ref().unwrap().video.as_ref().unwrap();
        assert_eq!(video.generation, generation + 1, "Cancel must not seek");
        assert!(!video.paused);
        app.graphical
            .as_mut()
            .unwrap()
            .video
            .as_mut()
            .unwrap()
            .set_control(true, 80);
        assert!(app.video_pointer("down", 0, track.x, track.y));
        assert!(app.video_pointer("up", 0, track.x + 1, track.y));
        assert!(
            app.graphical
                .as_ref()
                .unwrap()
                .video
                .as_ref()
                .unwrap()
                .paused
        );
        app.layout.preview_open = false;
        app.tick_video();
        assert!(app.graphical.as_ref().unwrap().video.is_none());
    }
    #[test]
    fn native_breadcrumbs_have_spaced_separators_and_real_ancestor_targets() {
        let home = std::path::Path::new("/home/test");
        let slots = breadcrumb_slots(
            Rect::new(3, 5, 80, 1),
            &home.join("Pictures/Pixel Art"),
            home,
        );
        assert_eq!(
            slots
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["~", "Pictures", "Pixel Art"]
        );
        assert_eq!(slots[0].2, home);
        assert_eq!(slots[1].2, home.join("Pictures"));
        assert_eq!(slots[1].0.x - slots[0].0.right(), 3);
        for width in 1..30 {
            let slots = breadcrumb_slots(
                Rect::new(3, 5, width, 1),
                &home.join("Pictures/Pixel Art"),
                home,
            );
            assert!(slots.iter().all(|(r, _, _)| r.right() <= 3 + width));
            assert_eq!(slots.last().unwrap().2, home.join("Pictures/Pixel Art"));
        }
        let slots = breadcrumb_slots(
            Rect::new(0, 0, 80, 1),
            std::path::Path::new("/run/media"),
            home,
        );
        assert_eq!(slots[0].2, std::path::Path::new("/"));
        assert_eq!(slots[1].0.x, 2, "root slash is not duplicated");
    }

    #[test]
    fn native_breadcrumb_click_navigates_only_its_pane() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let left = fake.home().join("Pictures/Pixel Art");
        let right = fake.home().join("Documents");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.graphical.as_mut().unwrap().pixel_layout = true;
        app.core.send(Command::RestoreCommander {
            dirs: [left.clone(), right.clone()],
            active: 1,
            enabled: true,
        });
        fake.pump();
        app.tick();
        Controller::scene(
            &mut app,
            Viewport {
                columns: 160,
                rows: 40,
                ..Viewport::default()
            },
        );
        let rect = pane_rect(
            app.layout.last.as_ref().unwrap().rect_of(ModuleId::Stack),
            0,
        );
        let body = starkit::chrome::frame::body(rect, &panels::words(ModuleId::Stack));
        let mut rule = panels::stack::split(body, 0, 0).rule;
        rule.x += 2;
        rule.width = rule.width.saturating_sub(2);
        let slots = breadcrumb_slots(rule, &left, &app.view.home);
        let target = slots
            .iter()
            .find(|(_, label, _)| label == "Pictures")
            .unwrap()
            .0;
        app.graphical_pointer("down", 0, target.x, target.y, 0);
        fake.pump();
        app.tick();
        assert_eq!(app.panes[0].dir, fake.home().join("Pictures"));
        assert_eq!(app.panes[1].dir, right);
        assert!(app.graphical.as_ref().unwrap().drag.is_none());
    }

    fn physical_cell(app: &App, x: u16, y: u16) -> (u16, u16) {
        let state = app.graphical.as_ref().unwrap();
        if state.placements.is_empty() {
            return (x, y);
        }
        let v = state.viewport.unwrap();
        let p = state
            .placements
            .iter()
            .rev()
            .find(|p| p.source.contains(x, y))
            .unwrap();
        for cy in 0..v.rows {
            for cx in 0..v.columns {
                if p.pointer(v, cx, cy, false) == Some((x, y)) {
                    return (cx, cy);
                }
            }
        }
        panic!("logical cell {x},{y} is unreachable through cell-pointer input: {p:?}");
    }

    #[test]
    fn scaled_destination_buttons_use_original_mouse_position() {
        for scale in [115u32, 125, 150, 200] {
            for kind in [OpKind::Copy, OpKind::Move] {
                let cfg = Config::default();
                let (core, fake) = crate::ui::fake::handle(cfg.core());
                let source = fake.home().join("source.txt");
                let dest = fake.home().join("dest");
                std::fs::write(&source, b"mouse copy").unwrap();
                std::fs::create_dir(&dest).unwrap();
                let mut app = App::new(
                    core,
                    cfg,
                    fake.home().join("config.toml"),
                    None,
                    Graphics::disabled(),
                );
                app.enable_graphical();
                let state = app.graphical.as_mut().unwrap();
                state.pixel_layout = true;
                state.surface_mode = true;
                let viewport = Viewport {
                    columns: (251 * 100 / scale) as u16,
                    rows: (80 * 100 / scale) as u16,
                    width: 1757,
                    height: 1280,
                    generation: 1,
                };
                app.overlays
                    .open_destination(crate::ui::overlays::context::Request {
                        archive_options: Default::default(),
                        kind,
                        sources: vec![source.clone()],
                        destination: dest.clone(),
                    });
                let scene = Controller::scene(&mut app, viewport);
                let placement = scene
                    .placements
                    .iter()
                    .find(|p| p.overlay.is_some())
                    .unwrap();
                let span = placement
                    .overlay
                    .as_ref()
                    .unwrap()
                    .iter()
                    .find(|span| span.text.contains("enter "))
                    .unwrap();
                let col = span.x + span.text.find("enter ").unwrap() as u16 + 3;
                let row = span.y;
                // A real terminal cell centre over the visible action, not a
                // hand-picked logical coordinate known to satisfy hit testing.
                let (cx, cy, px, py) = (0..80u16)
                    .find_map(|cy| {
                        (0..251u16).find_map(|cx| {
                            let px = u32::from(cx) * 7 + 3;
                            let py = u32::from(cy) * 16 + 8;
                            (placement.pointer_pixels(px, py, false) == Some((col, row)))
                                .then_some((cx, cy, px, py))
                        })
                    })
                    .expect("visible action has a terminal mouse cell");
                Controller::input(
                    &mut app,
                    Input::Pointer {
                        action: "down".into(),
                        button: 0,
                        x: (u32::from(cx) * u32::from(viewport.columns) / 251) as u16,
                        y: (u32::from(cy) * u32::from(viewport.rows) / 80) as u16,
                        pixel: Some([px, py]),
                        modifiers: 0,
                    },
                );
                assert!(
                    !app.overlays.is_open(),
                    "scale {scale}: action did not execute"
                );
                fake.pump();
                app.tick();
                fake.pump();
                app.tick();
                assert_eq!(
                    std::fs::read(dest.join("source.txt")).unwrap(),
                    b"mouse copy"
                );
                assert_eq!(source.exists(), kind == OpKind::Copy);
            }
        }
    }

    #[test]
    fn graphical_fallback_keeps_popup_spans_when_pixel_layout_is_unavailable() {
        for (cells, surfaces, rows) in [(true, true, 40), (false, true, 24), (false, false, 40)] {
            let cfg = Config::default();
            let (core, fake) = crate::ui::fake::handle(cfg.core());
            let mut app = App::new(
                core,
                cfg,
                fake.home().join("config.toml"),
                None,
                Graphics::disabled(),
            );
            app.enable_graphical();
            let state = app.graphical.as_mut().unwrap();
            state.pixel_layout = true;
            state.cell_mode = cells;
            state.surface_mode = surfaces;
            let v = Viewport {
                rows,
                height: u32::from(rows) * 20,
                ..Viewport::default()
            };
            Controller::scene(&mut app, v);
            Controller::input(
                &mut app,
                Input::Key {
                    code: "char:c".into(),
                    modifiers: 0,
                },
            );
            let menu = Controller::scene(&mut app, v);
            assert!(menu.placements.is_empty());
            assert!(
                menu.spans.iter().any(|s| s.text.contains("Rename")),
                "Fallback lost popup content"
            );
        }
    }

    #[test]
    fn pixel_layout_rows_tabs_and_popup_remain_reachable_after_zoom() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let directory = fake.home().join("listing");
        std::fs::create_dir(&directory).unwrap();
        for i in 0..100 {
            std::fs::write(directory.join(format!("file-{i:03}")), b"test").unwrap();
        }
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.graphical.as_mut().unwrap().pixel_layout = true;
        app.core.send(Command::RestoreCommander {
            dirs: [directory.clone(), directory],
            active: 0,
            enabled: true,
        });
        fake.pump();
        app.tick();
        for (columns, rows, cw, ch) in [
            (100, 40, 8, 16),
            (120, 48, 10, 20),
            (90, 35, 16, 32),
            (128, 62, 7, 16),
        ] {
            let v = Viewport {
                columns,
                rows,
                width: u32::from(columns) * cw + 3,
                height: u32::from(rows) * ch + 1,
                generation: 1,
            };
            let scene = Controller::scene(&mut app, v);
            assert!(!scene.placements.is_empty());
            for p in &scene.placements {
                p.validate(v).unwrap();
            }
            for c in &scene.components {
                if let Component::ListRow { rect, .. } = c {
                    physical_cell(&app, rect.x + 4, rect.y);
                }
            }
            let row = scene
                .components
                .iter()
                .find_map(|c| match c {
                    Component::ListRow { rect, .. } => Some(*rect),
                    _ => None,
                })
                .unwrap();
            let (x, y) = physical_cell(&app, row.x + 4, row.y);
            Controller::input(
                &mut app,
                Input::Pointer {
                    pixel: None,
                    action: "down".into(),
                    button: 1,
                    x,
                    y,
                    modifiers: 0,
                },
            );
            let popup = Controller::scene(&mut app, v);
            assert!(popup.placements.iter().any(|p| p.overlay.is_some()));
            for row in scene.components.iter().filter(|c| {
                matches!(
                    c,
                    Component::ListRow {
                        selected: false,
                        ..
                    } | Component::Scrollbar { .. }
                )
            }) {
                assert!(
                    popup.components.contains(row),
                    "Popup changed a background row or scrollbar: {row:?}"
                );
            }
            let overlay = popup
                .placements
                .iter()
                .find(|p| p.overlay.is_some())
                .unwrap();
            // Every pixel in an action row must resolve to that action even
            // after Kitty quantizes it to the terminal cell centre. Checking
            // only whether some cell can reach the action missed this bug.
            assert_eq!(u32::from(overlay.target.y) % ch, 0);
            for row in 1..overlay.source.height - 1 {
                for offset in 0..ch {
                    let py = u32::from(overlay.target.y) + u32::from(row) * ch + offset;
                    let cell_center = (py / ch) * ch + ch / 2;
                    let px = u32::from(overlay.target.x) + 3 * cw + cw / 2;
                    assert_eq!(
                        overlay.pointer_pixels(px, cell_center, false).unwrap().1,
                        overlay.source.y + row,
                        "visible popup action and cell mouse target disagree"
                    );
                }
            }
            for y in overlay.source.y + 1..overlay.source.y + overlay.source.height - 1 {
                physical_cell(&app, overlay.source.x + 3, y);
            }
            Controller::input(
                &mut app,
                Input::Key {
                    code: "escape".into(),
                    modifiers: 0,
                },
            );
        }
    }

    #[test]
    fn native_file_row_pointer_regions_include_metadata_in_both_panes() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.graphical.as_mut().unwrap().pixel_layout = true;
        for commander in [false, true] {
            app.core.send(Command::RestoreCommander {
                dirs: [fake.home().to_path_buf(), fake.home().to_path_buf()],
                active: 0,
                enabled: commander,
            });
            fake.pump();
            app.tick();
            let scene = Controller::scene(
                &mut app,
                Viewport {
                    columns: 160,
                    rows: 40,
                    width: 1600,
                    height: 800,
                    generation: 1,
                },
            );
            let mut count = 0;
            for component in &scene.components {
                if let Component::ListRow { rect, .. } = component {
                    count += 1;
                    assert!(scene.pointer_regions.iter().any(|r| r.x == rect.x
                        && r.y == rect.y
                        && r.height == 1
                        && r.width > rect.width));
                }
            }
            assert!(count > 0);
        }
    }

    #[test]
    fn native_rack_controls_copy_to_the_clicked_pane_and_reveal_the_queue_cursor() {
        fn click(app: &mut App, predicate: impl Fn(RackHit) -> bool) {
            let rect = app
                .graphical
                .as_ref()
                .unwrap()
                .rack_hits
                .iter()
                .find(|(_, hit)| predicate(*hit))
                .unwrap()
                .0;
            let (x, y) = physical_cell(app, rect.x + rect.width / 2, rect.y);
            for action in ["down", "up"] {
                Controller::input(
                    app,
                    Input::Pointer {
                        pixel: None,
                        action: action.into(),
                        button: 0,
                        x,
                        y,
                        modifiers: 0,
                    },
                );
            }
        }
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.graphical.as_mut().unwrap().pixel_layout = true;
        let source = fake.fixture.path("projects/starwire");
        let dest = fake.fixture.path("empty");
        app.core.send(Command::RestoreCommander {
            dirs: [source.clone(), dest.clone()],
            active: 0,
            enabled: true,
        });
        fake.pump();
        app.tick();
        let cursor = app
            .view
            .rows
            .iter()
            .position(|r| r.name == "README.md")
            .unwrap();
        app.core.send(Command::CursorTo(cursor));
        fake.pump();
        app.tick();
        let viewport = Viewport {
            columns: 160,
            rows: 60,
            width: 1600,
            height: 1200,
            generation: 1,
        };
        Controller::scene(&mut app, viewport);
        click(&mut app, |hit| {
            matches!(hit, RackHit::Pane(0, Action::Yank))
        });
        fake.pump();
        app.tick();
        Controller::scene(&mut app, viewport);
        click(&mut app, |hit| {
            matches!(hit, RackHit::Pane(1, Action::Paste))
        });
        fake.pump();
        app.tick();
        assert_eq!(
            std::fs::read(dest.join("README.md")).unwrap(),
            std::fs::read(source.join("README.md")).unwrap()
        );
        assert_eq!(app.active_pane, 1);
        Controller::scene(&mut app, viewport);
        // Removed per-pane header controls must not remain invisible targets.
        let _padding = starkit::chrome::frame::padding_scope(true);
        let pane = pane_rect(
            app.layout.last.as_ref().unwrap().rect_of(ModuleId::Stack),
            0,
        );
        let sort = starkit::chrome::header::slots(pane, &app.panel_words(ModuleId::Stack))
            .into_iter()
            .find(|(word, _)| matches!(word, panels::Word::Sort))
            .unwrap()
            .1;
        let (x, y) = physical_cell(&app, sort.x, sort.y);
        Controller::input(
            &mut app,
            Input::Pointer {
                pixel: None,
                action: "down".into(),
                button: 0,
                x,
                y,
                modifiers: 0,
            },
        );
        assert!(!app.overlays.is_open());
        for index in 0..9 {
            let target = fake.home().join(format!("copy-{index}"));
            std::fs::create_dir(&target).unwrap();
            app.core.send(Command::QueueOperation {
                kind: OpKind::Copy,
                sources: vec![source.join("README.md")],
                dest: Some(target),
            });
        }
        fake.pump();
        app.tick();
        app.layout.focus_set(ModuleId::Operations);
        app.ops_cursor = app.view.ops.len() - 1;
        app.clamp_scrolls();
        Controller::scene(&mut app, viewport);
        let last = app.view.ops.len() - 1;
        click(
            &mut app,
            |hit| matches!(hit, RackHit::Operation(i) if i == last),
        );
        assert_eq!(app.ops_cursor, last);
        assert!(app.ops_scroll > 0);
        let regions = app.layout.last.clone().unwrap();
        assert_eq!(app.page_size() as usize, app.operations_visible(&regions));
        let above = app.ops_scroll;
        let queue = app.rack_queue_rect(&regions).unwrap();
        app.scroll_at(&regions, queue.x + 2, queue.y, -3);
        app.clamp_scrolls();
        assert!(app.ops_scroll < above);
        Controller::scene(&mut app, viewport);
        assert!(app
            .graphical
            .as_ref()
            .unwrap()
            .rack_hits
            .iter()
            .any(|(_, hit)| matches!(hit, RackHit::Operation(i) if *i == app.ops_cursor)));
        click(&mut app, |hit| {
            matches!(hit, RackHit::Word(panels::Word::Filter))
        });
        for ch in "README".chars() {
            Controller::input(
                &mut app,
                Input::Key {
                    code: format!("char:{ch}"),
                    modifiers: 0,
                },
            );
        }
        fake.pump();
        app.tick();
        let filtered = Controller::scene(&mut app, viewport);
        assert_eq!(app.view.rows.len(), 1);
        assert_eq!(app.view.rows[0].name, "README.md");
        assert!(filtered.components.iter().any(|component| matches!(component,
            Component::Surface { surface, .. } if surface.nodes.iter().any(|node| matches!(node,
                starkit::native_surface::Primitive::Text { text, .. } if text == "FILTER / README│")))));
        Controller::input(
            &mut app,
            Input::Key {
                code: "escape".into(),
                modifiers: 0,
            },
        );
        fake.pump();
        app.tick();
        assert!(app.filter.is_none());
        assert!(app.view.rows.len() > 1);
    }

    #[test]
    fn rack_native_frames_preserve_pointer_projection_and_cell_fallback() {
        use starkit::terminal_graphics::renderer::{RenderMessage, Renderer};
        let mut renderer = Renderer::spawn().unwrap();
        for theme in ["terminal", "catppuccin-mocha", "catppuccin-latte"] {
            let mut cfg = Config::default();
            cfg.ui.theme = theme.into();
            cfg.preview.image_scale = crate::config::Scale::Smooth;
            let (core, fake) = crate::ui::fake::handle(cfg.core());
            let mut app = App::new(
                core,
                cfg,
                fake.home().join("config.toml"),
                None,
                Graphics::disabled(),
            );
            app.enable_graphical();
            app.graphical.as_mut().unwrap().pixel_layout = true;
            let pictures = fake.fixture.path("pictures");
            for name in ["Exports", "RAW"] {
                std::fs::create_dir(pictures.join(name)).unwrap();
            }
            std::fs::copy(pictures.join("harbour.png"), pictures.join("blue-hour.png")).unwrap();
            std::fs::copy(
                fake.fixture.path("projects/starwire/README.md"),
                pictures.join("field-notes.md"),
            )
            .unwrap();
            app.core.send(Command::RestoreCommander {
                dirs: [pictures.clone(), fake.home().to_path_buf()],
                active: 0,
                enabled: true,
            });
            fake.pump();
            app.tick();
            let readme = pictures.join("harbour.png");
            let cursor = app
                .view
                .rows
                .iter()
                .position(|r| r.name == "harbour.png")
                .unwrap();
            app.core.send(Command::CursorTo(cursor));
            app.core.send(Command::ToggleMarkPath(readme.clone()));
            app.core.send(Command::Preview(readme.clone()));
            app.core.send(Command::QueueOperation {
                kind: OpKind::Copy,
                sources: vec![readme],
                dest: Some(fake.home().to_path_buf()),
            });
            fake.pump();
            app.tick();
            let readme = pictures.join("harbour.png");
            if !app.core.state().selection.is_marked(&readme) {
                app.core.send(Command::ToggleMarkPath(readme));
                fake.pump();
                app.tick();
            }
            for (columns, rows, width, height) in [
                (140, 57, 1400, 1147),
                (160, 60, 1600, 1200),
                (80, 36, 800, 720),
                (60, 21, 600, 420),
            ] {
                let viewport = Viewport {
                    columns,
                    rows,
                    width,
                    height,
                    generation: 1,
                };
                let mut scene = Controller::scene(&mut app, viewport);
                if app.layout.preview_open {
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while app.graphical.as_ref().unwrap().image.is_none() {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(10));
                        scene = Controller::scene(&mut app, viewport);
                    }
                }
                let regions = app.layout.last.clone().unwrap();
                let preview = regions.rect_of(ModuleId::Preview);
                let operations = regions.rect_of(ModuleId::Operations);
                assert_eq!(preview.y == operations.y, columns >= 100);
                for placement in &scene.placements {
                    placement.validate(viewport).unwrap();
                }
                for component in &scene.components {
                    if let Component::Surface { surface, .. } = component {
                        surface.validate().unwrap();
                    }
                }
                for (module, rect) in [
                    (ModuleId::Preview, preview),
                    (ModuleId::Operations, operations),
                ] {
                    if let Some(placement) =
                        scene.placements.iter().find(|p| p.source == rect.into())
                    {
                        let x = u32::from(placement.target.x + placement.target.width / 2);
                        let y = u32::from(placement.target.y + placement.target.height / 2);
                        let (col, row) = placement.pointer_pixels(x, y, false).unwrap();
                        assert_eq!(regions.hit(col, row), Some(module));
                    }
                }
                renderer.scene(&scene).unwrap();
                loop {
                    match renderer
                        .output
                        .recv_timeout(Duration::from_secs(20))
                        .unwrap()
                    {
                        RenderMessage::Frame {
                            pixels: Some(image),
                            ..
                        } => {
                            assert_eq!(image.dimensions(), (width, height));
                            if let Some(dir) = std::env::var_os("STARFOLD_RACK_CAPTURES") {
                                std::fs::create_dir_all(&dir).unwrap();
                                image
                                    .save(
                                        PathBuf::from(dir)
                                            .join(format!("native-{theme}-{columns}x{rows}.png")),
                                    )
                                    .unwrap();
                            }
                            break;
                        }
                        RenderMessage::Error { message } => panic!("{message}"),
                        _ => {}
                    }
                }
            }
            let cells = Viewport {
                columns: 100,
                rows: 30,
                width: 1000,
                height: 600,
                generation: 2,
            };
            Controller::presentation(&mut app, true);
            Controller::scene(&mut app, cells);
            let regions = app.layout.last.as_ref().unwrap();
            assert_ne!(
                regions.rect_of(ModuleId::Preview).y,
                regions.rect_of(ModuleId::Operations).y
            );
            assert!(!app.theme.graphical_rows);
        }
    }

    #[test]
    fn native_chrome_surfaces_fit_and_deck_allocation_stays_stable() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.graphical.as_mut().unwrap().pixel_layout = true;
        for (columns, rows, width, height) in [
            (80, 36, 640, 576),
            (160, 54, 1600, 1080),
            (220, 70, 2640, 1680),
        ] {
            let viewport = Viewport {
                columns,
                rows,
                width,
                height,
                generation: 1,
            };
            let scene = Controller::scene(&mut app, viewport);
            let surfaces = scene
                .components
                .iter()
                .filter_map(|c| match c {
                    Component::Surface { surface, .. } => Some(surface),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(!surfaces.is_empty());
            for surface in surfaces {
                surface.validate().unwrap();
            }
            let ops_height = app
                .layout
                .last
                .as_ref()
                .unwrap()
                .rect_of(ModuleId::Operations)
                .height;
            if columns < 100 {
                assert_eq!(ops_height, 2);
            } else {
                assert!(ops_height >= 6);
            }
            app.layout.focus_set(ModuleId::Operations);
            let scene = Controller::scene(&mut app, viewport);
            assert_eq!(
                app.layout
                    .last
                    .as_ref()
                    .unwrap()
                    .rect_of(ModuleId::Operations)
                    .height,
                ops_height
            );
            for component in scene.components {
                if let Component::Surface { surface, .. } = component {
                    surface.validate().unwrap();
                }
            }
            app.layout.focus_set(ModuleId::Stack);
            app.view.ops_active = true;
            app.view.ops = vec![panels::operations::OpRow {
                title: "DELETE fixture".into(),
                status: "Permission denied".into(),
                bar: None,
                tone: panels::operations::Tone::Failed,
            }];
            for _ in 0..2 {
                let failed = Controller::scene(&mut app, viewport);
                assert_eq!(
                    app.layout
                        .last
                        .as_ref()
                        .unwrap()
                        .rect_of(ModuleId::Operations)
                        .height,
                    ops_height
                );
                for placement in &failed.placements {
                    placement.validate(viewport).unwrap();
                }
                assert!(failed.spans.iter().any(|span| span.text.contains("Permission denied")) || failed.components.iter().any(|component| matches!(component,
                    Component::Surface { surface, .. } if surface.nodes.iter().any(|primitive| matches!(primitive,
                        starkit::native_surface::Primitive::Text { text, .. } if text.contains("Permission denied"))))));
            }
            app.view.ops_active = false;
            app.view.ops.clear();
        }
    }

    #[test]
    fn preview_divider_drag_keeps_focus_geometry_and_never_starts_file_drag() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.graphical.as_mut().unwrap().pixel_layout = true;
        let v = Viewport {
            width: 1000,
            height: 800,
            columns: 100,
            rows: 50,
            ..Viewport::default()
        };
        let original = Controller::scene(&mut app, v);
        let before = app.layout.last.clone().unwrap();
        app.layout.focus_set(ModuleId::Preview);
        let focused = Controller::scene(&mut app, v);
        assert_eq!(original.placements, focused.placements);
        app.layout.focus_set(ModuleId::Stack);
        Controller::scene(&mut app, v);
        let handle = original.resize_handles[0];
        let y = u32::from(handle.y) + 8;
        let send = |app: &mut App, action: &str, y| {
            Controller::input(
                app,
                Input::Pointer {
                    action: action.into(),
                    button: 0,
                    x: 50,
                    y: 0,
                    pixel: Some([500, y]),
                    modifiers: 0,
                },
            )
        };
        send(&mut app, "down", y);
        assert!(app.graphical.as_ref().unwrap().preview_resize.is_some());
        assert!(app.graphical.as_ref().unwrap().drag.is_none());
        send(&mut app, "drag", y - 64);
        Controller::scene(&mut app, v);
        let enlarged = app.layout.last.clone().unwrap();
        assert_eq!(
            enlarged.rect_of(ModuleId::Preview).height,
            before.rect_of(ModuleId::Preview).height + 4
        );
        assert_eq!(
            enlarged.rect_of(ModuleId::Preview).bottom(),
            before.rect_of(ModuleId::Preview).bottom()
        );
        send(&mut app, "drag", y - 64);
        Controller::scene(&mut app, v);
        assert_eq!(
            enlarged,
            app.layout.last.clone().unwrap(),
            "redraw must not move the drag origin"
        );
        send(&mut app, "up", y - 64);
        assert!(app.graphical.as_ref().unwrap().preview_resize.is_none());
        app.layout.focus_set(ModuleId::Preview);
        Controller::scene(&mut app, v);
        assert_eq!(enlarged, app.layout.last.clone().unwrap());
        app.layout.focus_set(ModuleId::Stack);
        let scene = Controller::scene(&mut app, v);
        let y = u32::from(scene.resize_handles[0].y) + 8;
        send(&mut app, "down", y);
        send(&mut app, "drag", 799);
        Controller::scene(&mut app, v);
        assert!(
            app.layout
                .last
                .as_ref()
                .unwrap()
                .rect_of(ModuleId::Preview)
                .height
                >= 8
        );
        Controller::input(&mut app, Input::CancelPointer);
        assert!(app.graphical.as_ref().unwrap().preview_resize.is_none());
        assert!(app.bars.held().is_none());
        assert!(!app.dnd.drag_active);
        assert!(app.core.state().queue.is_empty());
    }

    #[test]
    fn native_tree_rows_are_compact_and_branches_connect_at_row_edges() {
        use starkit::native_surface::Primitive;
        let theme = crate::ui::theme::tests_support::theme("terminal");
        let rows = vec![
            "│   ├── one".into(),
            "│   └── two".into(),
            "└── three".into(),
        ];
        let (surface, scroll, visible) = native_tree_surface(&rows, 0, (300, 34), (8, 18), &theme);
        surface.validate().unwrap();
        assert_eq!((scroll, visible), (0, 2));
        let stems: Vec<_> = surface
            .nodes
            .iter()
            .filter_map(|n| match n {
                Primitive::Fill { rect, .. } if rect.x == 4 => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(stems[0].y + stems[0].height, stems[1].y);
        assert_eq!(stems[0].height, 17);
        let (surface, scroll, _) =
            native_tree_surface(&rows, usize::MAX, (300, 34), (8, 18), &theme);
        assert_eq!(scroll, 1);
        assert!(surface
            .nodes
            .iter()
            .any(|n| matches!(n, Primitive::Text { text, .. } if text == "three")));
    }

    #[test]
    fn movie_details_are_replaced_by_playback_without_changing_preview_height() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        fake.pump();
        app.refresh();
        app.enable_graphical();
        let state = app.graphical.as_mut().unwrap();
        state.cell_mode = true;
        state.video_capable = true;
        let path = fake.home().join("movie.mkv");
        let mut metadata = crate::fold::preview::model::Document::new("Movie details");
        metadata.field("Title", "Private Resort (1985)");
        metadata.field("Year", "1985");
        metadata.field("Cast", "Rob Morrow, Johnny Depp");
        metadata.image = Some(Arc::new(RgbaImage::from_pixel(
            40,
            60,
            starkit::image::Rgba([10, 20, 30, 255]),
        )));
        metadata.field("IMDb", "https://www.imdb.com/title/tt0089839/");
        metadata.notice = Some("Enter to play in Preview".into());
        app.view.cursor_path = Some(path.clone());
        app.view.preview = Some(Arc::new(Preview::Video {
            metadata: Arc::new(metadata),
            path: path.clone(),
            extension: None,
            poster: starkit::media::Poster {
                pixels: Arc::new(RgbaImage::new(32, 24)),
                duration: 60.,
                width: 1920,
                height: 1080,
                audio: true,
            },
        }));
        app.layout.preview_open = true;
        for cell_mode in [true, false] {
            app.graphical.as_mut().unwrap().cell_mode = cell_mode;
            let scene = Controller::scene(&mut app, Viewport::default());
            let text = scene
                .spans
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>();
            for detail in [
                "Private Resort",
                "1985",
                "Johnny Depp",
                "imdb.com/title/tt0089839",
            ] {
                assert!(
                    text.contains(detail),
                    "Poster suppressed {detail} in cell_mode={cell_mode}: {text}"
                );
            }
            for component in &scene.components {
                if let Component::Image { rect, .. } = component {
                    assert!(rect.width <= 64, "Poster covers the movie details");
                }
            }
        }
        assert_eq!(app.movie_loading_stage(), None);
        let height = app.layout.native_preview_rows;
        app.activate_video_entry(path);
        app.tick_video();
        assert!(app.movie_details_preview().is_none());
        assert!(app.graphical.as_ref().unwrap().video.is_some());
        let scene = Controller::scene(&mut app, Viewport::default());
        assert!(scene
            .components
            .iter()
            .any(|c| matches!(c, Component::Image { .. })));
        assert_eq!(app.layout.native_preview_rows, height);
    }

    #[test]
    fn movie_loading_stops_on_extension_failure_and_can_be_retried() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.graphical.as_mut().unwrap().video_capable = true;
        let path = fake.home().join("movie.mkv");
        let mut loading = crate::fold::preview::model::Document::new("Loading preview…");
        loading.notice = Some("Loading…".into());
        fake.state_mut().preview = Some((path.clone(), Arc::new(Preview::Document(loading))));
        app.view.cursor_path = Some(path.clone());
        app.activate_video_entry(path.clone());
        assert_eq!(app.movie_loading_stage(), Some("Loading movie…"));
        app.tick_video();
        assert!(app.graphical.as_ref().unwrap().video_pending.is_some());
        assert!(app.note.is_none());
        let mut document = crate::fold::preview::model::Document::new("Video");
        document.notice = Some("Preview extension unavailable: Preview timed out".into());
        let preview = Arc::new(Preview::Document(document));
        fake.state_mut().preview = Some((path.clone(), preview.clone()));
        app.view.cursor_path = Some(path.clone());
        app.view.preview = Some(preview);
        app.activate_video_entry(path.clone());
        assert_eq!(app.movie_loading_stage(), None);
        app.tick_video();
        assert!(app.graphical.as_ref().unwrap().video_pending.is_none());
        assert!(app.note.as_ref().unwrap().0.contains("Preview timed out"));
        assert!(app.graphical.as_ref().unwrap().video.is_none());
        fake.state_mut().preview = None;
        app.view.preview = None;
        app.activate_video_entry(path);
        assert_eq!(app.movie_loading_stage(), Some("Loading movie…"));
        assert!(app.graphical.as_ref().unwrap().video_pending.is_some());
    }

    #[test]
    fn movie_loading_indicator_covers_open_decode_buffering_and_errors() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        fake.pump();
        app.refresh();
        app.enable_graphical();
        app.layout.preview_open = true;
        app.graphical.as_mut().unwrap().cell_mode = true;
        let path = fake.home().join("movie.mkv");
        app.last_preview_for = Some(path.clone());
        app.view.preview = None;
        assert_eq!(app.movie_loading_stage(), Some("Loading movie…"));
        let area = Rect::new(0, 0, 80, 16);
        let mut buffer = Buffer::empty(area);
        app.draw_movie_loading(area, &mut buffer);
        assert!(buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .contains("Loading movie…"));
        app.view.preview = Some(Arc::new(Preview::Video {
            metadata: std::sync::Arc::new(crate::fold::preview::model::Document::new(
                "Movie details",
            )),
            extension: None,
            path: path.clone(),
            poster: starkit::media::Poster {
                pixels: Arc::new(RgbaImage::new(32, 24)),
                duration: 60.,
                width: 1920,
                height: 1080,
                audio: true,
            },
        }));
        assert_eq!(app.movie_loading_stage(), None);
        app.graphical.as_mut().unwrap().video = Some(starkit::terminal_graphics::media::Host::new(
            path,
            "video-test".into(),
            17,
            true,
        ));
        assert_eq!(app.movie_loading_stage(), Some("Starting playback…"));
        {
            let video = app.graphical.as_mut().unwrap().video.as_mut().unwrap();
            video.position = 12.;
        }
        assert_eq!(app.movie_loading_stage(), Some("Buffering movie…"));
        {
            let video = app.graphical.as_mut().unwrap().video.as_mut().unwrap();
            video.buffering = false;
        }
        assert_eq!(app.movie_loading_stage(), None);
        {
            let video = app.graphical.as_mut().unwrap().video.as_mut().unwrap();
            video.buffering = true;
            video.finished = true;
        }
        assert_eq!(app.movie_loading_stage(), None);
        app.graphical.as_mut().unwrap().video = None;
        app.view.preview = Some(Arc::new(Preview::Error("Cannot open movie".into())));
        assert_eq!(app.movie_loading_stage(), None);
    }

    #[test]
    fn cell_movie_scene_does_not_bake_a_second_poster_into_its_spans() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        fake.pump();
        app.refresh();
        app.enable_graphical();
        app.graphical.as_mut().unwrap().cell_mode = true;
        app.view.preview = Some(Arc::new(Preview::Video {
            metadata: std::sync::Arc::new(crate::fold::preview::model::Document::new(
                "Movie details",
            )),
            extension: None,
            path: fake.home().join("movie.mkv"),
            poster: starkit::media::Poster {
                pixels: Arc::new(RgbaImage::from_pixel(
                    32,
                    24,
                    starkit::image::Rgba([255, 0, 255, 255]),
                )),
                duration: 60.,
                width: 1920,
                height: 1080,
                audio: true,
            },
        }));
        app.layout.preview_open = true;
        let viewport = Viewport {
            columns: 100,
            rows: 40,
            width: 1000,
            height: 800,
            ..Viewport::default()
        };
        let scene = Controller::scene(&mut app, viewport);
        let rect = app.layout.last.as_ref().unwrap().rect_of(ModuleId::Preview);
        assert!(rect.height > 4);
        assert!(
            !scene
                .spans
                .iter()
                .any(|span| rect.contains((span.x, span.y).into())
                    && span.text.chars().any(|c| matches!(c, '▀' | '▄'))),
            "Cell preview painted a half-block poster behind the frontend image"
        );
    }

    #[test]
    fn movie_letterbox_has_equal_gaps_above_and_below() {
        use starkit::native_surface::PixelRect;
        use starkit::terminal_graphics::placement::{PanelPadding, Placement};
        let _padding = starkit::chrome::frame::padding_scope(true);
        for rows in [12, 24, 40] {
            let area = Rect::new(0, 0, 160, rows);
            let mut p = Placement::new(area.into(), PixelRect::new(0, 0, 1600, rows * 20));
            p.padding = Some(PanelPadding { inset: 12, gap: 8 });
            balance_video_padding(&mut p);
            let picture = video_image_body(area, true);
            let body = video_body(area, true);
            let top_inset = p.row_edge(1);
            let bottom_inset = f32::from(p.target.height) - p.row_edge(body.bottom());
            assert!(
                (top_inset - bottom_inset).abs() < 0.001,
                "title/control border insets differ: {top_inset}/{bottom_inset}"
            );
            let details_row = body.bottom() - 4;
            let above = p.row_edge(picture.y) - p.row_edge(2);
            let below = p.row_edge(details_row) - p.row_edge(picture.bottom());
            assert!(
                (above - below).abs() <= 1.,
                "unequal movie padding: {above}/{below}"
            );
        }
    }

    #[test]
    fn native_video_body_uses_panel_content_without_a_blank_metadata_row() {
        let _padding = starkit::chrome::frame::padding_scope(true);
        let area = Rect::new(0, 0, 160, 40);
        let native = video_body(area, true);
        let legacy = video_body(area, false);
        assert_eq!(native.y, panels::preview::body(area).y);
        assert_eq!(native.height, panels::preview::body(area).height + 1);
        assert_eq!(legacy, panels::preview::content_rect(area));
        assert_eq!(native.y + 1, legacy.y);
        assert_eq!(native.bottom(), legacy.bottom() + 1);
    }

    #[test]
    fn native_header_truncates_long_titles_before_the_controls() {
        use starkit::native_surface::Primitive;
        let _padding = starkit::chrome::frame::padding_scope(true);
        let theme = crate::ui::theme::tests_support::theme("terminal");
        let title = format!("Preview · {}.mkv", "Very long movie title 界 ".repeat(20));
        for width in [60, 100, 160] {
            let words = panels::words(ModuleId::Preview);
            let Component::Surface { surface, .. } = native_header(
                Rect::new(0, 0, width, 30),
                &title,
                &words,
                &theme,
                (8, 18),
                false,
                0,
            ) else {
                unreachable!()
            };
            surface.validate().unwrap();
            let Primitive::Text {
                rect, text, mono, ..
            } = &surface.nodes[0]
            else {
                unreachable!()
            };
            assert!(*mono);
            assert!(text.ends_with('…'));
            assert!(
                starkit::ratatui::text::Line::raw(text.as_str()).width()
                    <= usize::from(rect.width / 8)
            );
            assert!(surface
                .nodes
                .iter()
                .skip(1)
                .all(|node| node.rect().x >= rect.x + rect.width));
        }
    }

    #[test]
    fn graphical_menu_spacing_and_shortcuts_share_mouse_slots_in_both_themes() {
        use starkit::chrome::header::{self, Word as _};
        use starkit::native_surface::Primitive;
        let _padding = starkit::chrome::frame::padding_scope(true);
        let rect = Rect::new(0, 0, 100, 20);
        let words: Vec<_> = panels::words(ModuleId::Stack)
            .into_iter()
            .filter(|w| *w != panels::Word::Back)
            .collect();
        for name in ["terminal", "catppuccin-latte"] {
            let theme = crate::ui::theme::tests_support::theme(name);
            let shortcut = hex(theme
                .rack
                .button
                .best_contrast_against(&[starkit::theme::WHITE, starkit::theme::BLACK]));
            for enabled in [false, true] {
                let Component::Surface { surface, .. } =
                    native_header(rect, "Files", &words, &theme, (8, 18), enabled, 0)
                else {
                    unreachable!()
                };
                surface.validate().unwrap();
                for (word, slot) in header::slots(rect, &words) {
                    assert_ne!(word, panels::Word::Back);
                    for (offset, c) in word.word().chars().enumerate() {
                        let expected_x = (slot.x - 1 + offset as u16) * 8;
                        let node = surface
                            .nodes
                            .iter()
                            .skip(1)
                            .find(|node| matches!(node, Primitive::Text { rect, .. } if rect.x == expected_x))
                            .unwrap();
                        let Primitive::Text {
                            rect,
                            text,
                            color,
                            bold,
                            mono,
                            ..
                        } = node
                        else {
                            unreachable!()
                        };
                        assert_eq!(rect.width, 8);
                        assert_eq!(*text, c.to_string());
                        assert!(*mono);
                        let highlighted = enabled && word.mnemonic() == Some((c, offset as u16));
                        assert_eq!(*bold, highlighted);
                        assert_eq!(
                            *color,
                            if highlighted {
                                shortcut.clone()
                            } else {
                                hex(theme
                                    .rack
                                    .foreground
                                    .ensure_contrast(theme.rack.button, 4.5))
                            }
                        );
                        assert_eq!(
                            header::hit(
                                Rect::new(0, 0, 100, 20),
                                &words,
                                slot.x + offset as u16,
                                slot.y
                            ),
                            Some(word)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn image_preview_zoom_buttons_keys_and_reset_are_bounded() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.view.preview = Some(Arc::new(Preview::Image {
            data: Arc::new(RgbaImage::new(2, 1)),
            width: 2,
            height: 1,
            format: "png",
        }));
        app.layout.focus_set(ModuleId::Preview);
        app.word_click(panels::Word::ZoomIn);
        assert_eq!(app.graphical.as_ref().unwrap().image_zoom, Some(125));
        app.key(KeyEvent::new(
            KeyCode::Char('-'),
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.graphical.as_ref().unwrap().image_zoom, Some(100));
        for _ in 0..30 {
            app.word_click(panels::Word::ZoomIn);
        }
        assert_eq!(app.graphical.as_ref().unwrap().image_zoom, Some(800));
        for _ in 0..30 {
            app.word_click(panels::Word::ZoomOut);
        }
        assert_eq!(app.graphical.as_ref().unwrap().image_zoom, Some(25));
        app.key(KeyEvent::new(
            KeyCode::Char('0'),
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.graphical.as_ref().unwrap().image_zoom, None);
        app.cfg.preview.image_scale = crate::config::Scale::Pixels;
        app.word_click(panels::Word::ZoomIn);
        assert_eq!(app.graphical.as_ref().unwrap().image_zoom, Some(200));
        assert!(app
            .panel_words(ModuleId::Preview)
            .contains(&panels::Word::ZoomReset(200)));
        app.word_click(panels::Word::ZoomReset(200));
        assert_eq!(app.graphical.as_ref().unwrap().image_zoom, None);
    }

    #[test]
    fn graphical_preview_scale_control_updates_image_policy_and_saved_setting() {
        use starkit::terminal_graphics::protocol::ImageScale;
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        let mut scene = Controller::scene(&mut app, Viewport::default());
        let regions = app.layout.last.clone().unwrap();
        let source = Arc::new(RgbaImage::new(2, 1));
        app.view.preview = Some(Arc::new(Preview::Image {
            data: source.clone(),
            width: 2,
            height: 1,
            format: "png",
        }));
        app.layout.preview_open = true;
        let state = app.graphical.as_mut().unwrap();
        state.image_source = Some(source);
        state.image = Some(("test".into(), "payload".into()));
        for expected in [ImageScale::Pixels, ImageScale::Smooth, ImageScale::One] {
            let word = app.panel_words(ModuleId::Preview)[0];
            assert!(matches!(word, panels::Word::PictureScale(_)));
            app.word_click(word);
            scene.components.clear();
            app.graphical_image(&mut scene, &regions);
            assert!(scene
                .components
                .iter()
                .any(|c| matches!(c, Component::Image { scale, .. } if *scale == expected)));
            assert!(std::fs::read_to_string(&app.cfg_path)
                .unwrap()
                .contains(app.cfg.preview.image_scale.name()));
        }
    }

    #[test]
    fn pdf_reader_fills_its_body_without_using_pixel_art_scaling() {
        use starkit::terminal_graphics::protocol::ImageScale;
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.layout.preview_open = true;
        let mut scene = Controller::scene(&mut app, Viewport::default());
        let regions = app.layout.last.clone().unwrap();
        let source = Arc::new(RgbaImage::new(200, 50));
        let mut document = crate::fold::preview::model::Document::new("PDF");
        document.image = Some(source.clone());
        document.extension = Some(Box::new(crate::fold::preview::extensions::Info {
            provider: "pdf".into(),
            revision: "test".into(),
            session: 10,
            sequence: 1,
            keys: vec![],
            interactive: false,
            modified: false,
            actions: vec![],
        }));
        let mut scrolled = document.clone();
        app.view.preview = Some(Arc::new(Preview::Document(document)));
        app.cfg.preview.image_scale = crate::config::Scale::One;
        let state = app.graphical.as_mut().unwrap();
        state.image_source = Some(source);
        state.image = Some(("pdf-page".into(), "payload".into()));
        state.image_document = Some(("pdf".into(), 10));
        scene.components.clear();
        app.graphical_image(&mut scene, &regions);
        let body = panels::preview::body(regions.rect_of(ModuleId::Preview));
        assert!(scene.components.iter().any(|component| matches!(component,
            Component::Image { rect, scale: ImageScale::Smooth, .. }
            if rect.x == body.x && rect.y == body.y && rect.width == body.width
                && rect.height == body.height.saturating_sub(1))));
        assert_eq!(app.cfg.preview.image_scale, crate::config::Scale::One);
        scrolled.image = Some(Arc::new(RgbaImage::new(1800, 50)));
        app.view.preview = Some(Arc::new(Preview::Document(scrolled.clone())));
        scene.components.clear();
        app.graphical_image(&mut scene, &regions);
        assert!(
            scene.components.iter().any(|component| matches!(component,
            Component::Image { id, .. } if id == "pdf-page")),
            "scrolling must retain the ready page"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while app
            .graphical
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .is_some_and(|(id, _)| id == "pdf-page")
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
            scene.components.clear();
            app.graphical_image(&mut scene, &regions);
        }
        let state = app.graphical.as_ref().unwrap();
        let png = &state.image.as_ref().unwrap().1;
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png)
            .unwrap();
        assert_eq!(
            starkit::image::load_from_memory(&bytes).unwrap().width(),
            1800
        );
        scrolled.extension.as_mut().unwrap().session = 11;
        scrolled.image = Some(Arc::new(RgbaImage::new(1800, 50)));
        app.view.preview = Some(Arc::new(Preview::Document(scrolled)));
        scene.components.clear();
        app.graphical_image(&mut scene, &regions);
        assert!(
            !scene
                .components
                .iter()
                .any(|component| matches!(component, Component::Image { .. })),
            "a different document must never retain the old page"
        );
    }

    #[test]
    fn completed_graphical_thumbnail_wakes_an_idle_scene() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        Controller::scene(&mut app, Viewport::default());
        let idle = Controller::frame_interval(&app);
        assert!(idle > Duration::ZERO);
        let worker = starkit::terminal_graphics::assets::Thumbnailer::default();
        worker.request("ready".into(), Arc::new(RgbaImage::new(8, 8)));
        let deadline = Instant::now() + Duration::from_secs(5);
        while worker.output.is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        app.graphical.as_mut().unwrap().thumbnail = Some(worker);
        assert_eq!(Controller::frame_interval(&app), Duration::ZERO);
        app.layout.preview_open = false;
        assert_eq!(Controller::frame_interval(&app), idle);
        app.layout.preview_open = true;
        let state = app.graphical.as_mut().unwrap();
        state.thumbnail.as_ref().unwrap().output.try_recv().unwrap();
        assert_eq!(Controller::frame_interval(&app), idle);
    }

    #[test]
    fn graphical_drag_preserves_marks_scroll_ownership_and_copy_identity() {
        for pixels in [false, true] {
            for fast in [false, true] {
                for desktop in [false, true] {
                    graphical_drag_fixture(pixels, fast, desktop);
                }
            }
        }
    }
    fn graphical_drag_fixture(pixel_layout: bool, fast: bool, desktop: bool) {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let source = fake.home().join("source");
        let destination = fake.home().join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        for i in 0..60 {
            std::fs::write(
                source.join(format!("source-{i:02}")),
                format!("payload {i}"),
            )
            .unwrap();
            std::fs::write(destination.join(format!("filler-{i:02}")), b"filler").unwrap();
        }
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        app.graphical.as_mut().unwrap().pixel_layout = pixel_layout;
        app.core.send(Command::RestoreCommander {
            dirs: [source.clone(), destination.clone()],
            active: 0,
            enabled: true,
        });
        fake.pump();
        app.tick();
        let viewport = Viewport {
            columns: 100,
            rows: 40,
            ..Viewport::default()
        };
        let scene = Controller::scene(&mut app, viewport);
        let row = scene
            .components
            .iter()
            .find_map(|c| match c {
                Component::ListRow { rect, label, .. } if label == "source-00" => Some(*rect),
                _ => None,
            })
            .unwrap();
        let pointer = |app: &mut App, action: &str, x, y| {
            let (x, y) = physical_cell(app, x, y);
            Controller::input(
                app,
                Input::Pointer {
                    pixel: None,
                    action: action.into(),
                    button: 0,
                    x,
                    y,
                    modifiers: 0,
                },
            )
        };
        pointer(&mut app, "down", row.x + 4, row.y);
        pointer(&mut app, "up", row.x + 4, row.y);
        // Mark advances to the next row in the established file-manager UI.
        for code in ["char: ", "char: "] {
            Controller::input(
                &mut app,
                Input::Key {
                    code: code.into(),
                    modifiers: 0,
                },
            );
            fake.pump();
            app.tick();
        }
        let scene = Controller::scene(&mut app, viewport);
        let source_track = app.bars.track_of(Bar::Commander(0)).unwrap();
        let target_track = app.bars.track_of(Bar::Commander(1)).unwrap();
        for track in [source_track, target_track] {
            assert!(scene.components.iter().any(|component| matches!(component,
                Component::Scrollbar { rect, thumb } if *rect == track.into()
                    && thumb.y >= rect.y && thumb.y + thumb.height <= rect.y + rect.height)));
        }
        let source_key = app.panes[0].key;
        let target_key = app.panes[1].key;
        let source_scroll = app.scroll.get(&source_key).copied().unwrap_or(0);
        let target_scroll = app.scroll.get(&target_key).copied().unwrap_or(0);

        // Grabbing the bar must never capture file sources, even across panes.
        pointer(&mut app, "down", source_track.x, source_track.y);
        assert_eq!(app.bars.held(), Some(Bar::Commander(0)));
        assert!(app.graphical.as_ref().unwrap().drag.is_none());
        if pixel_layout {
            Controller::scene(
                &mut app,
                Viewport {
                    width: viewport.width + 1,
                    generation: 2,
                    ..viewport
                },
            );
            assert!(app.bars.held().is_none());
            assert!(app.graphical.as_ref().unwrap().pointer_capture.is_none());
            Controller::scene(&mut app, viewport);
            pointer(&mut app, "down", source_track.x, source_track.y);
            assert_eq!(app.bars.held(), Some(Bar::Commander(0)));
        }
        pointer(
            &mut app,
            "drag",
            target_track.x - 4,
            target_track.bottom() - 1,
        );
        assert!(!app.dnd.drag_active);
        pointer(
            &mut app,
            "up",
            target_track.x - 4,
            target_track.bottom() - 1,
        );
        Controller::input(
            &mut app,
            Input::Key {
                code: "home".into(),
                modifiers: 0,
            },
        );
        fake.pump();
        app.tick();
        Controller::scene(&mut app, viewport);

        pointer(&mut app, "down", row.x + 4, row.y);
        pointer(
            &mut app,
            "drag",
            target_track.x - 4,
            target_track.bottom() - 1,
        );
        assert_eq!(app.dnd.offer.as_ref().unwrap().sources.len(), 2);
        let source_cursor = app.view.cursor_path.clone();
        app.dnd_autoscroll();
        assert_eq!(
            app.active_pane, 0,
            "destination hover must not steal pane focus"
        );
        assert_eq!(app.view.cursor_path, source_cursor);
        assert_eq!(app.scroll[&target_key], target_scroll + 1);
        pointer(
            &mut app,
            "scroll_down",
            target_track.x - 4,
            target_track.y + 3,
        );
        assert_eq!(
            app.scroll[&target_key],
            target_scroll + 4,
            "wheel scrolls the receiving pane during drag"
        );
        assert_eq!(app.active_pane, 0);
        pointer(
            &mut app,
            "scroll_down",
            row.x + 4,
            source_track.bottom() - 1,
        );
        assert_eq!(app.scroll[&source_key], source_scroll);
        pointer(&mut app, "drag", row.x + 4, source_track.bottom() - 1);
        app.dnd_autoscroll();
        assert_eq!(app.scroll[&source_key], source_scroll);
        assert!(
            app.dnd.hover.is_none(),
            "copying into the source directory must be rejected"
        );
        pointer(&mut app, "up", row.x + 4, row.y);
        fake.pump();
        app.tick();
        assert!(app.view.ops.is_empty(), "self-drop must not queue a copy");

        Controller::scene(&mut app, viewport);
        pointer(&mut app, "down", row.x + 4, row.y);
        pointer(&mut app, "drag", row.x + 5, row.y);
        assert!(app.dnd.drag_active, "one cell of movement starts the drag");
        let held = app.layout.last.clone();
        let preview_open = app.layout.preview_open;
        app.layout.preview_open = !preview_open;
        app.view.ops_active = true;
        Controller::scene(&mut app, viewport);
        assert_eq!(
            app.layout.last, held,
            "background changes must not move the drag target"
        );
        app.layout.preview_open = preview_open;
        app.view.ops_active = false;
        Controller::input(
            &mut app,
            Input::Key {
                code: "escape".into(),
                modifiers: 0,
            },
        );
        assert!(!app.dnd.drag_active);
        assert!(app.graphical.as_ref().unwrap().drag.is_none());
        pointer(&mut app, "down", row.x + 4, row.y);
        if desktop {
            app.dnd.enabled = true;
            Controller::input(
                &mut app,
                Input::Osc72 {
                    text: format!("t=o:x={}:y={}", row.x + 4, row.y),
                },
            );
        }
        if desktop {
            Controller::input(&mut app, Input::CancelPointer);
            assert!(
                app.dnd.offer.is_some(),
                "focus loss must preserve desktop source"
            );
            assert!(app.graphical.as_ref().unwrap().drag.is_some());
        }
        if !fast {
            pointer(&mut app, "drag", target_track.x - 4, target_track.y + 3);
            if desktop {
                Controller::input(
                    &mut app,
                    Input::Osc72 {
                        text: format!(
                            "t=m:x={}:y={}:o=1:i=1;text/uri-list",
                            target_track.x - 4,
                            target_track.y + 3
                        ),
                    },
                );
            }
            assert_eq!(app.dnd.hover.as_ref(), Some(&destination));
            if desktop {
                Controller::effects(&mut app);
                Controller::input(
                    &mut app,
                    Input::Osc72 {
                        text: format!(
                            "t=m:x={}:y={}:o=1:i=1;text/uri-list",
                            target_track.x - 4,
                            target_track.y + 3
                        ),
                    },
                );
                assert!(
                    Controller::effects(&mut app).is_empty(),
                    "unchanged hover must not cause an acceptance feedback loop"
                );
            }
            Controller::scene(&mut app, viewport);
        }
        // Fast terminals may coalesce motion: a displaced release must also drop.
        pointer(&mut app, "up", target_track.x - 4, target_track.y + 3);
        if desktop {
            assert!(app.core.state().queue.is_empty());
            assert!(
                !app.overlays.is_open(),
                "ordinary mouse release must not finish Kitty's gesture"
            );
            Controller::input(
                &mut app,
                Input::Osc72 {
                    text: format!(
                        "t=M:x={}:y={}:o=1:i=1;text/uri-list",
                        target_track.x - 4,
                        target_track.y + 3
                    ),
                },
            );
        }
        assert!(
            app.overlays.is_open(),
            "graphical drop must offer the ASCII copy/move choice"
        );
        assert!(
            app.core.state().queue.is_empty(),
            "no copy before choosing an operation"
        );
        Controller::input(
            &mut app,
            Input::Key {
                code: "char:c".into(),
                modifiers: 0,
            },
        );
        fake.pump();
        app.tick();
        fake.pump();
        app.tick();
        for i in 0..2 {
            let name = format!("source-{i:02}");
            assert!(
                destination.join(&name).exists(),
                "missing {name}: {:?}",
                app.view.ops
            );
            assert_eq!(
                std::fs::read(destination.join(&name)).unwrap(),
                std::fs::read(source.join(name)).unwrap()
            );
        }
        assert!(!destination.join("source-02").exists());
        assert!(!app.dnd.drag_active);
        assert!(app.graphical.as_ref().unwrap().drag.is_none());
    }

    #[test]
    fn menus_preserve_graphical_rows_and_ignore_background_progress() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        let before = Controller::scene(&mut app, Viewport::default());
        Controller::input(
            &mut app,
            Input::Key {
                code: "char:c".into(),
                modifiers: 0,
            },
        );
        app.view.ops = vec![panels::operations::OpRow {
            title: "COPY test".into(),
            status: "copying 10%".into(),
            bar: None,
            tone: panels::operations::Tone::Running,
        }];
        let menu = Controller::scene(&mut app, Viewport::default());
        assert!(matches!(
            menu.components.last(),
            Some(Component::Menu { .. })
        ));
        let rows = |scene: &Scene| {
            scene
                .components
                .iter()
                .filter(|c| matches!(c, Component::ListRow { .. }))
                .count()
        };
        assert!(rows(&before) > 0);
        assert_eq!(rows(&before), rows(&menu));
        app.view.ops[0].status = "copying 75%".into();
        let progress = Controller::scene(&mut app, Viewport::default());
        assert_eq!(menu.interaction, progress.interaction);
    }
    #[test]
    fn large_listing_reuses_rows_and_transmits_only_the_visible_range() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        {
            let mut state = fake.state_mut();
            let dir = state.active_frame().dir.clone();
            let template = state
                .active_listing()
                .unwrap()
                .entries
                .iter()
                .find(|e| e.kind == EntryKind::File)
                .unwrap()
                .clone();
            let mut listing = (**state.active_listing().unwrap()).clone();
            listing.entries = (0..100_000)
                .map(|i| {
                    let mut entry = template.clone();
                    entry.display = format!("file-{i:06}.txt");
                    entry.path = dir.join(&entry.display);
                    entry
                })
                .collect();
            state.listings.insert(dir, Arc::new(listing));
            state.active_frame_mut().rows = (0..100_000).collect();
            state.active_frame_mut().cursor = 0;
            state.version += 1;
        }
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.enable_graphical();
        let pointer = app.view.rows.as_ptr();
        let mut times = vec![];
        for _ in 0..100 {
            let started = Instant::now();
            Controller::input(
                &mut app,
                Input::Key {
                    code: "down".into(),
                    modifiers: 0,
                },
            );
            let scene = Controller::scene(&mut app, Viewport::default());
            times.push(started.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(
                app.view.rows.as_ptr(),
                pointer,
                "Cursor changes must reuse the listing presentation"
            );
            assert!(
                scene
                    .components
                    .iter()
                    .filter(|c| matches!(c, Component::ListRow { .. }))
                    .count()
                    < 40
            );
            assert!(
                serde_json::to_vec(&scene).unwrap().len() < 100_000,
                "SSH scenes must not contain the full listing"
            );
        }
        times.sort_by(f64::total_cmp);
        println!(
            "100,000 entries: controller + scene p95 {:.3} ms",
            times[94]
        );
    }
}
