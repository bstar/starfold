//! Shared graphical scene adapter over the established controller and hit geometry.
use super::*;
use starkit::terminal_graphics::{
    protocol::{Component, Input, Scene, ServerMessage, Span, Viewport},
    session::Controller,
};

#[derive(Default)]
pub(super) struct State {
    pub effects: Vec<ServerMessage>,
    pub output_pending: bool,
    image_source: Option<Arc<RgbaImage>>,
    thumbnail: Option<starkit::terminal_graphics::assets::Thumbnailer>,
    image: Option<(String, String)>,
    audio_images: std::collections::HashMap<String, String>,
    rendered_version: u64,
    drag: Option<Drag>,
    authorization: Option<Authorization>,
    pub(super) wire: Option<crossbeam_channel::Receiver<wire_dnd::Outgoing>>,
}
struct Drag {
    origin: (u16, u16),
    stack: usize,
    sources: Vec<PathBuf>,
    started: bool,
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

fn hex(rgb: starkit::theme::color::Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb.r, rgb.g, rgb.b)
}
fn color(c: starkit::ratatui::style::Color, fallback: &str) -> String {
    match c {
        starkit::ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.into(),
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
    pub(crate) fn enable_graphical(&mut self) {
        let (tx, rx) = crossbeam_channel::bounded(256);
        wire_dnd::graphical_output(tx);
        self.graphical = Some(State {
            wire: Some(rx),
            ..State::default()
        });
    }
    fn graphical_pointer(&mut self, action: &str, button: u8, x: u16, y: u16, modifiers: u8) {
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
                    origin: (x, y),
                    stack,
                    sources,
                    started: false,
                });
            }
        }
        if action == "drag" || action == "up" || action.starts_with("scroll") {
            if let Some(mut drag) = self.graphical.as_mut().unwrap().drag.take() {
                if action == "drag"
                    && (drag.origin.0.abs_diff(x) > 1 || drag.origin.1.abs_diff(y) > 0)
                {
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
                            self.core.send(Command::QueueDrop {
                                kind: OpKind::Copy,
                                sources: drag.sources,
                                dest,
                            });
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
    fn graphical_image(&mut self, scene: &mut Scene, regions: &Regions) {
        if self.overlays.is_open()
            || self.places.is_some()
            || self.tab_picker.is_some()
            || self.editor.is_some()
            || self.audio_here()
            || !self.layout.preview_open
        {
            return;
        }
        let source = match self.view.preview.as_deref() {
            Some(Preview::Image { data, .. }) => Some(Arc::clone(data)),
            _ => None,
        };
        let state = self.graphical.as_mut().unwrap();
        if let Some((id, png)) = state
            .thumbnail
            .as_ref()
            .and_then(|worker| worker.output.try_iter().last())
        {
            if state
                .image_source
                .as_ref()
                .is_some_and(|image| format!("preview-{:p}", Arc::as_ptr(image)) == id)
            {
                state.image = Some((id, png));
            }
        }
        let changed = match (&state.image_source, &source) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        if changed {
            state.image = None;
            state.image_source = source.clone();
            if let Some(source) = source {
                let id = format!("preview-{:p}", Arc::as_ptr(&source));
                state
                    .thumbnail
                    .get_or_insert_with(Default::default)
                    .request(id, source);
            }
        }

        if let Some((id, png)) = &state.image {
            let rect = panels::preview::content_rect(regions.rect_of(ModuleId::Preview));
            scene.components.push(Component::Image {
                rect: rect.into(),
                id: id.clone(),
                png: Some(png.clone()),
            });
        }
    }
}
impl Controller for App {
    fn frame_interval(&self) -> Duration {
        if self.graphical.as_ref().unwrap().rendered_version != self.seen_version {
            return Duration::ZERO;
        }
        let busy = self.view.loading
            || self.overlays.is_open()
            || self.places.is_some()
            || self.tab_picker.is_some()
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
            Duration::from_millis(33)
        } else {
            Duration::from_secs(1)
        }
    }
    fn output_pending(&mut self, pending: bool) {
        self.graphical.as_mut().unwrap().output_pending = pending;
    }
    fn attached(&mut self) {
        self.dnd.enabled = false;
    }
    fn detached(&mut self) {
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
        App::tick(self);
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
        self.audio_cell_size = Some((
            (viewport.width / u32::from(viewport.columns)).clamp(1, 64) as u16,
            (viewport.height / u32::from(viewport.rows)).clamp(1, 128) as u16,
        ));
        let area = Rect::new(0, 0, viewport.columns, viewport.rows);
        let mut buffer = Buffer::empty(area);
        self.draw(area, &mut buffer);
        self.graphical.as_mut().unwrap().rendered_version = self.seen_version;
        let mut scene = Scene::from_buffer(&buffer, viewport, 0);
        scene.accent = hex(self.theme.accent);
        scene.border = hex(self.theme.border);
        let Some(regions) = self.layout.last.clone() else {
            return scene;
        };
        use std::hash::{Hash, Hasher};
        let mut targets = std::collections::hash_map::DefaultHasher::new();
        self.view.active_dir.hash(&mut targets);
        self.view.frame_id.hash(&mut targets);
        self.view.op_ids.hash(&mut targets);
        for pane in &self.panes {
            pane.dir.hash(&mut targets);
            pane.key.hash(&mut targets);
        }
        let modal = self.overlays.is_open() || self.places.is_some() || self.tab_picker.is_some();
        let mut modal_rects = self.overlays.graphical_rects(area);
        if modal_rects.is_empty() {
            if self.places.is_some() {
                modal_rects.push(super::super::places::rect(area));
            } else if let Some(picker) = self.tab_picker.as_mut() {
                modal_rects = picker.graphical_rects(area);
            }
        }
        // All controller-rendered modal chrome gets a pixel border too.
        for y in 0..viewport.rows {
            for x in 0..viewport.columns {
                if !matches!(buffer[(x, y)].symbol(), "╔" | "┌" | "╭") {
                    continue;
                }
                let right = (x + 1..viewport.columns)
                    .find(|xx| matches!(buffer[(*xx, y)].symbol(), "╗" | "┐" | "╮"));
                if let Some(right) = right {
                    let bottom = (y + 1..viewport.rows)
                        .find(|yy| matches!(buffer[(x, *yy)].symbol(), "╚" | "└" | "╰"));
                    if let Some(bottom) = bottom {
                        scene.components.push(Component::Panel {
                            rect: starkit::terminal_graphics::Rect {
                                x,
                                y,
                                width: right - x + 1,
                                height: bottom - y + 1,
                            },
                            active: buffer[(x, y)].fg == panels::rgb(self.theme.border_focused),
                        });
                    }
                }
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
        if !modal {
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
                if let Some((total, available)) = view.space.filter(|(total, _)| *total > 0) {
                    let y = rect.bottom().saturating_sub(1);
                    let x = rect.x + 2;
                    let width = (x..rect.right())
                        .take_while(|x| buffer[(*x, y)].symbol() == "■")
                        .count() as u16;
                    if width > 0 {
                        scene.components.push(Component::Meter {
                            rect: starkit::terminal_graphics::Rect {
                                x,
                                y,
                                width,
                                height: 1,
                            },
                            value: ((u128::from(total.saturating_sub(available.min(total))) * 1000)
                                / u128::from(total)) as u16,
                            foreground: hex(self.theme.fold.progress_fg),
                            background: hex(self.theme.border),
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
                        let cell = &buffer[(list.x, y)];
                        let name_cell = &buffer[(
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
                        });
                    }
                }
            }
            let items = self.tab_items();
            let active = self.core.state().tabs.active().id;
            for (rect, hit) in &self.tab_hits {
                if let super::super::tabs::Hit::Tab(id) = hit {
                    if let Some(item) = items.iter().find(|item| item.id == *id) {
                        scene.components.push(Component::Tab {
                            rect: (*rect).into(),
                            label: item.label.clone(),
                            active: *id == active,
                        });
                    }
                }
            }
        }
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
        self.graphical_image(&mut scene, &regions);
        if !modal && self.audio_graphics_config().is_some() {
            if let Some(frame) = &self.audio_frame {
                use std::hash::{Hash, Hasher};
                let body = panels::preview::content_rect(regions.rect_of(ModuleId::Preview));
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
                        });
                        retained.insert(id);
                    }
                }
                cache.retain(|id, _| retained.contains(id));
            }
        }
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
                });
            }
        }
        scene
    }
    fn input(&mut self, input: Input) {
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
        match input {
            Input::Key { code, modifiers } => {
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
                modifiers,
            } => self.graphical_pointer(&action, button, x, y, modifiers),
            Input::Paste { text } => {
                if self.editor.is_some() {
                    self.editor_paste(&text);
                } else if self.overlays.paste(&text) {
                    self.repaint = true;
                } else if let Some(input) = self.filter.as_mut() {
                    input.paste(&text);
                    self.core.send(Command::SetFilter(input.text().into()));
                }
            }
            Input::Osc72 { text } => self.dnd_message(&text),
            Input::CancelPointer => {
                self.graphical.as_mut().unwrap().drag = None;
                self.dnd.drag_active = false;
                self.dnd.offer = None;
                self.dnd.hover = None;
                self.dnd.hover_coords = None;
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
    fn effects(&mut self) -> Vec<ServerMessage> {
        let state = self.graphical.as_mut().unwrap();
        let mut effects = std::mem::take(&mut state.effects);
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
        self.editor = None;
        self.stop_audio();
        self.session_writer = None;
        self.core.send(Command::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
