//! Native experimental presentation; all filesystem work stays in fold workers.
use super::*;
use crate::fold::{ops::OpKind, tab::TabId};
use starkit::crossterm::event::{KeyEvent as TerminalKeyEvent, KeyModifiers};
use starkit::gpui::MouseButton;
use starkit::gpui::{prelude::*, *};
use starkit::visual::desktop::input::{InputEvent, NativeInput};
use starkit::visual::{
    desktop::{card, menu_item, meter, rgb24, tab},
    Tokens,
};

#[derive(Clone)]
struct FileDrag {
    sources: Vec<PathBuf>,
    label: String,
    source_pane: usize,
}
impl Render for FileDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_4()
            .py_2()
            .rounded_md()
            .bg(rgb(0x345373))
            .text_color(rgb(0xffffff))
            .shadow_md()
            .child(format!("Copy · {}", self.label))
    }
}
type ViewportKey = (TabId, (usize, FrameId));
struct FilenameTip {
    text: String,
    tokens: Tokens,
}
impl Render for FilenameTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        card(self.tokens).max_w(px(640.)).child(self.text.clone())
    }
}
type ImageIdentity = (usize, u32, u32, crate::config::Scale);
type ImageConversion = (ImageIdentity, RgbaImage, Arc<RgbaImage>);
type ImageSurface = (ImageIdentity, Arc<RenderImage>, Arc<RgbaImage>);
fn terminal_key(stroke: &Keystroke) -> Option<TerminalKeyEvent> {
    let modifiers = stroke.modifiers;
    let mut mods = KeyModifiers::empty();
    if modifiers.control || modifiers.platform {
        mods |= KeyModifiers::CONTROL;
    }
    if modifiers.alt {
        mods |= KeyModifiers::ALT;
    }
    if modifiers.shift {
        mods |= KeyModifiers::SHIFT;
    }
    let code = match stroke.key.as_str() {
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "enter" => KeyCode::Enter,
        "escape" => KeyCode::Esc,
        "tab" if modifiers.shift => KeyCode::BackTab,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "space" => KeyCode::Char(' '),
        key if key.starts_with('f') && key[1..].parse::<u8>().is_ok() => {
            KeyCode::F(key[1..].parse().ok()?)
        }
        _ => {
            let text = stroke.key_char.as_deref().unwrap_or(&stroke.key);
            let mut chars = text.chars();
            let character = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            KeyCode::Char(if modifiers.shift && stroke.key_char.is_none() {
                character.to_ascii_uppercase()
            } else {
                character
            })
        }
    };
    Some(TerminalKeyEvent::new(code, mods))
}
struct Desktop {
    app: super::App,
    focus: FocusHandle,
    field: Option<Entity<NativeInput>>,
    field_used: bool,
    field_focus: bool,
    authorization: Option<OpId>,
    authorizing: bool,
    auth_result: Option<crossbeam_channel::Receiver<std::result::Result<(), String>>>,
    auth_cancel: Arc<std::sync::atomic::AtomicBool>,
    auth_error: Option<String>,
    menu_rows: u16,
    operations_scroll: ScrollHandle,
    shown_operation: Option<usize>,
    dialog_scroll: ScrollHandle,
    dialog_cursor: Option<(&'static str, usize)>,
    dialog_scroll_pending: bool,
    cell_width: f32,
    tool_bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    lists: [UniformListScrollHandle; 2],
    tabs_scroll: ScrollHandle,
    visible_tab: Option<TabId>,
    rendered_frames: [Option<ViewportKey>; 2],
    operation_details: Option<String>,
    cache: starkit::visual::SurfaceCache,
    icons: Vec<(starkit::visual::SurfaceKey, Arc<RenderImage>)>,
    icon_theme: String,
    drag_source: Option<(usize, Point<Pixels>)>,
    drag_scroll: Instant,
    scrollbar: Option<usize>,
    image: Option<ImageSurface>,
    image_pending: Option<crossbeam_channel::Receiver<ImageConversion>>,
    metrics: starkit::visual::Metrics,
    input_metrics: starkit::visual::Metrics,
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.auth_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.remember_scroll();
        self.app.save_workspace(true);
        self.app.session_writer = None;
        self.app.stop_audio();
        self.app.core.send(Command::Shutdown);
        tracing::info!(
            frames = self.metrics.frames,
            p95_ms = self.metrics.p95_ms(),
            input_p95_ms = self.input_metrics.p95_ms(),
            "desktop render metrics"
        );
    }
}
pub fn run(core: Handle, cfg: Config, path: PathBuf, session: Option<PathBuf>) -> Result<()> {
    let mut app = super::App::new(core, cfg, path, session, Graphics::disabled());
    app.native_clipboard = true;
    let mut initial = Some(app);
    Application::new().run(move |cx| {
        starkit::visual::desktop::input::install_bindings(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(800.)),
                    cx,
                ))),
                app_id: Some("starfold-visual".into()),
                window_min_size: Some(size(px(660.), px(660.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("STAR/FOLD · visual experiment".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("STAR/FOLD · visual experiment");
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus);
                    cx.spawn(async move |entity, cx| loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(50))
                            .await;
                        if entity
                            .update(cx, |this: &mut Desktop, cx| {
                                let before = (
                                    this.app.seen_version,
                                    this.app.view.running_bar.clone(),
                                    this.app.note.as_ref().map(|n| n.0.clone()),
                                );
                                this.app.tick();
                                if let Some(result) = this
                                    .image_pending
                                    .as_ref()
                                    .and_then(|rx| rx.try_recv().ok())
                                {
                                    let (identity, bgra, source) = result;
                                    tracing::debug!(
                                        width = identity.1,
                                        height = identity.2,
                                        "native image preview surface"
                                    );
                                    this.image = Some((
                                        identity,
                                        Arc::new(RenderImage::new(vec![
                                            starkit::image::Frame::new(bgra),
                                        ])),
                                        source,
                                    ));
                                    this.image_pending = None;
                                    cx.notify();
                                }
                                this.poll_authorization();
                                if this.app.quit {
                                    cx.quit();
                                    return;
                                }
                                if let Some(text) = this.app.clipboard_output.take() {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                                }
                                if !cx.has_active_drag() {
                                    this.drag_source = None;
                                }
                                if let Some((pane, offset)) = this.drag_source {
                                    this.lists[pane].0.borrow().base_handle.set_offset(offset);
                                }
                                let after = (
                                    this.app.seen_version,
                                    this.app.view.running_bar.clone(),
                                    this.app.note.as_ref().map(|n| n.0.clone()),
                                );
                                if before != after
                                    || std::mem::take(&mut this.app.repaint)
                                    || this.app.editor.is_some()
                                    || this.app.audio_here()
                                    || this.app.view.loading
                                    || this.app.overlays.is_open()
                                    || this.app.tab_picker.is_some()
                                    || this.app.places.is_some()
                                    || this.app.view.unmounting.is_some()
                                    || (this.app.layout.preview_open
                                        && this.app.view.preview.is_none()
                                        && this.app.view.cursor_path.is_some())
                                {
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    })
                    .detach();
                    Desktop {
                        cache: Default::default(),
                        icons: vec![],
                        icon_theme: String::new(),
                        drag_source: None,
                        drag_scroll: Instant::now(),
                        scrollbar: None,
                        app: initial.take().unwrap(),
                        focus,
                        field: None,
                        field_used: false,
                        field_focus: false,
                        authorization: None,
                        authorizing: false,
                        auth_result: None,
                        auth_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        auth_error: None,
                        menu_rows: 20,
                        operations_scroll: ScrollHandle::new(),
                        shown_operation: None,
                        dialog_scroll: ScrollHandle::new(),
                        dialog_cursor: None,
                        dialog_scroll_pending: false,
                        cell_width: 8.4,
                        tool_bounds: Default::default(),
                        lists: [
                            UniformListScrollHandle::new(),
                            UniformListScrollHandle::new(),
                        ],
                        tabs_scroll: ScrollHandle::new(),
                        visible_tab: None,
                        rendered_frames: [None, None],
                        operation_details: None,
                        image: None,
                        image_pending: None,
                        metrics: Default::default(),
                        input_metrics: Default::default(),
                    }
                })
            },
        )
        .expect("open experimental window");
        cx.activate(true);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
    Ok(())
}
impl Desktop {
    fn module_header(&self, module: ModuleId, pane: usize, cx: &mut Context<Self>) -> AnyElement {
        let tokens = Tokens::from_theme(&self.app.theme);
        let words = if module == ModuleId::Operations {
            panels::operations::header_words(
                self.app.core.state().queue.is_paused(),
                self.app.running_op_id().is_some(),
                self.app.layout.focus() == module,
            )
        } else {
            panels::words(module)
        };
        div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .flex_wrap()
            .gap_1()
            .items_center()
            .flex_shrink_0()
            .child(
                div()
                    .flex_1()
                    .min_w(px(110.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .line_clamp(1)
                    .child(if module == ModuleId::Stack {
                        if self.app.commander {
                            if pane == 0 {
                                "STAR/FOLD · LEFT"
                            } else {
                                "STAR/FOLD · RIGHT"
                            }
                        } else {
                            "STAR/FOLD"
                        }
                    } else {
                        module.title()
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.app.layout.focus_set(module);
                            if module == ModuleId::Stack && this.app.commander {
                                this.app.focus_pane(pane);
                            }
                            cx.notify();
                        }),
                    ),
            )
            .children(words.into_iter().map(|word| {
                let label = starkit::chrome::header::Word::word(word).into_owned();
                menu_item(label.clone(), tokens)
                    .flex_shrink_0()
                    .px_1()
                    .py_1()
                    .id(SharedString::from(label))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if module == ModuleId::Stack && this.app.commander {
                            this.app.focus_pane(pane);
                        }
                        this.app.layout.focus_set(module);
                        this.app.word_click(word);
                        this.app.refresh();
                        cx.notify();
                    }))
            }))
            .into_any_element()
    }
    fn context_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let cell_width = self.cell_width;
        use crate::ui::{
            overlays::Overlay,
            popup::{Answer as PopupAnswer, Entry},
        };
        let mut area = self.app.layout.last.as_ref()?.area;
        area.height = self.menu_rows;
        let panels = match self.app.overlays.current_mut()? {
            Overlay::Context(menu) | Overlay::Drop(menu) => menu.popup.native_panels(area),
            _ => return None,
        };
        let tokens = Tokens::from_theme(&self.app.theme);
        let overlay = div()
            .absolute()
            .inset_0()
            .id("native-context")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.app.overlays.close();
                    cx.notify();
                }),
            )
            .children(panels.into_iter().map(|panel| {
                let level = panel.level;
                card(tokens)
                    .absolute()
                    .shadow_lg()
                    .p_1()
                    .flex()
                    .flex_col()
                    .left(px(12. + f32::from(panel.rect.x) * cell_width))
                    .top(px(12. + f32::from(panel.rect.y) * 28.))
                    .w(px(f32::from(panel.rect.width) * cell_width))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .children(panel.rows.into_iter().map(|row| {
                        let (label, enabled, danger, submenu) = match row.entry {
                            Entry::Action {
                                label,
                                enabled,
                                danger,
                                ..
                            } => (label, enabled, danger, false),
                            Entry::Submenu { label, enabled, .. } => (label, enabled, false, true),
                            Entry::Separator => {
                                return div()
                                    .h(px(1.))
                                    .my_1()
                                    .bg(rgb24(tokens.border))
                                    .into_any_element()
                            }
                        };
                        menu_item(label, tokens)
                            .h(px(28.))
                            .py_1()
                            .min_w(px(0.))
                            .text_ellipsis()
                            .line_clamp(1)
                            .when(row.selected && enabled, |item| {
                                item.bg(rgb24(tokens.selected))
                            })
                            .when(!enabled, |item| item.text_color(rgb24(tokens.muted)))
                            .when(danger, |item| item.text_color(rgb24(self.app.theme.error)))
                            .when(submenu, |item| item.child(" ›"))
                            .id(("context-row", level * 65536 + row.index))
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                if *hovered {
                                    if let Some(Overlay::Context(menu) | Overlay::Drop(menu)) =
                                        this.app.overlays.current_mut()
                                    {
                                        menu.popup.native_hover(level, row.index);
                                        cx.notify();
                                    }
                                }
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                let answer = match this.app.overlays.current_mut() {
                                    Some(Overlay::Context(menu)) => {
                                        match menu.popup.native_activate(level, row.index) {
                                            PopupAnswer::Selected(action) => {
                                                Some(Answer::Context(menu.target.clone(), action))
                                            }
                                            _ => None,
                                        }
                                    }
                                    Some(Overlay::Drop(menu)) => {
                                        match menu.popup.native_activate(level, row.index) {
                                            PopupAnswer::Selected(
                                                crate::ui::overlays::context::Action::Copy,
                                            ) => Some(Answer::Drop(OpKind::Copy)),
                                            PopupAnswer::Selected(
                                                crate::ui::overlays::context::Action::Move,
                                            ) => Some(Answer::Drop(OpKind::Move)),
                                            _ => None,
                                        }
                                    }
                                    _ => None,
                                };
                                if let Some(answer) = answer {
                                    this.app.overlays.close();
                                    this.app.after_overlay_answer(answer);
                                    this.app.refresh();
                                }
                                cx.notify();
                            }))
                            .into_any_element()
                    }))
            }));
        Some(overlay.into_any_element())
    }
    fn cancel_authorization(&mut self) {
        self.auth_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.authorization = None;
        self.authorizing = false;
        self.auth_result = None;
        self.auth_error = None;
        self.field = None;
        self.app.repaint = true;
    }
    fn poll_authorization(&mut self) {
        if self.authorization.is_none() {
            if let Some(op) = self.app.pending_elevated_delete.take() {
                self.authorization = Some(op);
                self.field = None;
                self.auth_error = None;
                self.app.repaint = true;
            }
        }
        let result = self
            .auth_result
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    Some(Err("Authorization ended unexpectedly".into()))
                }
                Err(crossbeam_channel::TryRecvError::Empty) => None,
            });
        if let Some(result) = result {
            self.auth_result = None;
            self.authorizing = false;
            match result {
                Ok(()) => {
                    if let Some(op) = self.authorization.take() {
                        self.app.core.send(Command::QueueElevatedDelete(op));
                    }
                    self.field = None;
                }
                Err(error) => {
                    self.auth_error = Some(error);
                    self.field = None;
                }
            }
            self.app.repaint = true;
        }
    }
    fn authorize(&mut self, cx: &mut Context<Self>) {
        if self.authorizing {
            return;
        }
        let Some(field) = &self.field else {
            return;
        };
        let mut password = field.read(cx).text().to_owned();
        field.update(cx, |input, _| {
            input.sync("", 0, Tokens::from_theme(&self.app.theme))
        });
        self.auth_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled = self.auth_cancel.clone();
        let (sender, receiver) = crossbeam_channel::bounded(1);
        self.auth_result = Some(receiver);
        self.authorizing = true;
        self.auth_error = None;
        let spawn = std::thread::Builder::new().name("starfold-native-auth".into()).spawn(move || {
            use std::io::Write;
            use std::process::{Command as Process, Stdio};
            let result = (|| -> std::result::Result<(), String> {
                let mut child = Process::new("sudo").args(["-S", "-v", "-p", ""])
                    .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped())
                    .spawn().map_err(|error| format!("Could not run sudo: {error}"))?;
                let write = child.stdin.take().ok_or_else(|| "Could not open authorization input".to_string())
                    .and_then(|mut stdin| stdin.write_all(password.as_bytes()).and_then(|_| stdin.write_all(b"\n")).map_err(|error| error.to_string()));
                password.clear();
                if let Err(error) = write {
                    if matches!(child.try_wait(), Ok(Some(status)) if status.success()) { return Ok(()); }
                    let _ = child.kill(); let _ = child.wait(); return Err(error);
                }
                let start = Instant::now();
                loop {
                    if cancelled.load(std::sync::atomic::Ordering::Relaxed) || start.elapsed() > Duration::from_secs(60) {
                        let _ = child.kill(); let _ = child.wait(); return Err("Authorization cancelled or timed out".into());
                    }
                    match child.try_wait() {
                        Ok(Some(status)) if status.success() => { let _ = child.wait(); return Ok(()); }
                        Ok(Some(_)) => { let _ = child.wait(); return Err("Authorization denied. Check your password and sudo permissions.".into()); }
                        Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                        Err(error) => { let _ = child.kill(); let _ = child.wait(); return Err(error.to_string()); }
                    }
                }
            })();
            password.clear();
            let _ = sender.send(result);
        });
        if let Err(error) = spawn {
            self.authorizing = false;
            self.auth_result = None;
            self.auth_error = Some(error.to_string());
        }
        self.app.repaint = true;
    }
    fn authorization_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.authorization?;
        let tokens = Tokens::from_theme(&self.app.theme);
        let (text, cursor) = self
            .field
            .as_ref()
            .map(|field| (field.read(cx).text().to_owned(), field.read(cx).cursor()))
            .unwrap_or_default();
        let field = self.native_text(&text, cursor, cx);
        field.update(cx, |input, _| input.set_secret(true));
        let mut body = card(tokens)
            .w(px(500.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child("Administrator authorization")
            .child("Enter your sudo password to retry the confirmed delete.")
            .child(field);
        if let Some(error) = &self.auth_error {
            body = body.child(
                div()
                    .text_color(rgb24(self.app.theme.fold.error_fg))
                    .child(error.clone()),
            );
        }
        if self.authorizing {
            body = body.child("Authorizing…");
        }
        body = body.child(
            div()
                .flex()
                .gap_2()
                .child(
                    menu_item("Authorize", tokens)
                        .id("authorize")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.authorize(cx);
                            cx.notify();
                        })),
                )
                .child(
                    menu_item("Cancel", tokens)
                        .id("cancel-authorization")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.cancel_authorization();
                            window.focus(&this.focus);
                            cx.notify();
                        })),
                ),
        );
        Some(
            starkit::visual::desktop::dialog(tokens)
                .id("authorization-dialog")
                .child(body)
                .into_any_element(),
        )
    }
    fn reveal_dialog(&mut self, kind: &'static str, cursor: usize) {
        if self.dialog_cursor != Some((kind, cursor)) {
            self.dialog_cursor = Some((kind, cursor));
            self.dialog_scroll_pending = true;
        }
    }
    fn native_text(
        &mut self,
        text: &str,
        cursor: usize,
        cx: &mut Context<Self>,
    ) -> Entity<NativeInput> {
        let tokens = Tokens::from_theme(&self.app.theme);
        self.field_used = true;
        if let Some(field) = &self.field {
            field.update(cx, |input, _| {
                input.sync(text, cursor, tokens);
                input.set_navigation(false);
            });
            return field.clone();
        }
        let field = cx.new(|cx| NativeInput::new(text, cursor, tokens, cx));
        cx.subscribe(&field, |this, _, event, cx| {
            if this.authorization.is_some() {
                match event {
                    InputEvent::Submit => this.authorize(cx),
                    InputEvent::Cancel => this.cancel_authorization(),
                    _ => {}
                }
                cx.notify();
                return;
            }
            match event {
                InputEvent::Edited { text, cursor } => {
                    let input = if let Some(places) = &mut this.app.places {
                        places.native_input_mut()
                    } else if let Some(picker) = &mut this.app.tab_picker {
                        picker.native_input_mut()
                    } else if this.app.overlays.is_open() {
                        this.app.overlays.native_input_mut()
                    } else {
                        this.app.filter.as_mut()
                    };
                    if let Some(input) = input {
                        input.set_text(text.clone());
                        input.set_cursor(*cursor);
                    }
                    if this.app.filter.is_some()
                        && !this.app.overlays.is_open()
                        && this.app.places.is_none()
                        && this.app.tab_picker.is_none()
                    {
                        this.app.core.send(Command::SetFilter(text.clone()));
                        this.app.refresh();
                    }
                }
                InputEvent::Submit => this.modal_key(KeyCode::Enter),
                InputEvent::Cancel => this.modal_key(KeyCode::Esc),
                InputEvent::Mode => this.modal_key(KeyCode::Tab),
                InputEvent::Unhandled(stroke) => {
                    if let Some(key) = terminal_key(stroke) {
                        this.app.key(key);
                        this.app.refresh();
                    }
                }
            }
            cx.notify();
        })
        .detach();
        self.field = Some(field.clone());
        self.field_focus = true;
        field
    }
    fn tool_pointer(&mut self, position: Point<Pixels>, button: &str) {
        let bounds = self.tool_bounds.get();
        if !bounds.contains(&position) || !self.app.audio_here() {
            return;
        }
        self.app.layout.focus_set(ModuleId::Preview);
        let x = ((position.x - bounds.left()) / px(self.cell_width)).max(0.) as u16;
        let y = ((position.y - bounds.top()) / px(20.)).max(0.) as u16;
        if let Err(error) = self.app.audio.pointer(x, y, button) {
            self.app.audio_error = Some(error);
        }
    }
    fn begin_drop(&mut self, sources: Vec<PathBuf>, dest: PathBuf) {
        if self.app.overlays.is_open() {
            return;
        }
        if sources.is_empty()
            || sources
                .iter()
                .any(|source| source.parent() == Some(dest.as_path()) || dest.starts_with(source))
        {
            self.app
                .core
                .send(Command::Notify("Cannot copy an item onto itself".into()));
            return;
        }
        self.app.native_drop = Some((sources, dest));
        self.app.overlays.open_drop((4, 4));
        self.drag_source = None;
        self.app.repaint = true;
    }
    fn modal_key(&mut self, code: KeyCode) {
        if code == KeyCode::Char('c')
            && matches!(
                self.app.overlays.current(),
                Some(crate::ui::overlays::Overlay::Failure(_))
            )
        {
            self.app.act(crate::ui::keymap::Action::CopyOperations);
        } else {
            self.app
                .key(TerminalKeyEvent::new(code, KeyModifiers::NONE));
        }
        self.app.refresh();
    }
    fn native_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::ui::overlays::Overlay;
        use starkit::visual::desktop::dialog;
        let tokens = Tokens::from_theme(&self.app.theme);
        let current = self.app.overlays.current()?;
        let mut title = String::new();
        let mut lines = Vec::new();
        let mut field = None;
        let mut error = None;
        let mut buttons: Vec<(String, KeyCode)> = Vec::new();
        match current {
            Overlay::Rename(form) => {
                title = if form.recovery {
                    "Restore with another name"
                } else {
                    "Rename"
                }
                .into();
                lines.extend(form.detail.clone());
                field = Some((form.input.text().to_owned(), form.input.cursor()));
                error = form.error.map(str::to_owned);
                buttons.push(("Rename".into(), KeyCode::Enter));
            }
            Overlay::ConflictRename(sequence) => {
                title = "Rename incoming file".into();
                lines.extend(sequence.form.detail.clone());
                field = Some((
                    sequence.form.input.text().to_owned(),
                    sequence.form.input.cursor(),
                ));
                error = sequence.form.error.map(str::to_owned);
                buttons.push(("Continue".into(), KeyCode::Enter));
            }
            Overlay::Create(form) => {
                title = match form.kind {
                    crate::fold::create::Kind::File => "New file",
                    _ => "New directory",
                }
                .into();
                lines.push(form.dir.display().to_string());
                field = Some((form.input.text().to_owned(), form.input.cursor()));
                error = form.error.map(str::to_owned);
                buttons.push(("Create".into(), KeyCode::Enter));
            }
            Overlay::Destination(form) => {
                title = format!("{:?} destination", form.request.kind);
                lines.push(format!("{} item(s)", form.request.sources.len()));
                field = Some((form.input.text().to_owned(), form.input.cursor()));
                error = form.error.clone();
                if matches!(form.request.kind, OpKind::Compress(_)) {
                    buttons.push(("Change format".into(), KeyCode::Tab));
                }
                buttons.push(("Start".into(), KeyCode::Enter));
            }
            Overlay::Search(form) => {
                title = format!("Search {:?}", form.mode);
                field = Some((form.input.text().to_owned(), form.input.cursor()));
                error = form.error.map(str::to_owned);
                buttons.push(("Names / contents".into(), KeyCode::Tab));
                buttons.push(("Search".into(), KeyCode::Enter));
            }
            Overlay::Confirm(form) => {
                title.clone_from(&form.title);
                lines.clone_from(&form.body);
                buttons.push((form.yes.into(), KeyCode::Char('y')));
            }
            Overlay::TrashWarning(form) => {
                title = "Delete permanently?".into();
                lines.push("Trash is disabled for this location.".into());
                lines.push(format!("{} item(s) will be deleted.", form.sources.len()));
                buttons.push(("Delete".into(), KeyCode::Char('d')));
                buttons.push((
                    "Delete; skip warnings this session".into(),
                    KeyCode::Char('s'),
                ));
            }
            Overlay::Conflict(form) => {
                title = format!("{} names already exist", form.conflicts.len());
                lines = form
                    .conflicts
                    .iter()
                    .map(|conflict| conflict.dest.display().to_string())
                    .collect();
                buttons.extend([
                    ("Skip".into(), KeyCode::Char('s')),
                    ("Rename…".into(), KeyCode::Char('r')),
                    ("Replace".into(), KeyCode::Char('o')),
                ]);
            }
            Overlay::Failure(form) => {
                title = "Operation failed".into();
                lines = form.lines.iter().skip(form.scroll).cloned().collect();
                if form.retry.is_some() {
                    buttons.push(("Retry".into(), KeyCode::Char('r')));
                }
                if form.sudo_retry.is_some() {
                    buttons.push(("Retry as administrator…".into(), KeyCode::Char('s')));
                }
                buttons.push(("Copy operations output".into(), KeyCode::Char('c')));
            }
            Overlay::Help { scroll } => {
                title = "Keyboard shortcuts".into();
                lines = crate::ui::keymap::BINDINGS
                    .iter()
                    .skip(usize::from(*scroll))
                    .map(|binding| {
                        format!("{} · {} — {}", binding.group, binding.keys, binding.label)
                    })
                    .collect();
            }
            Overlay::Recovery(browser) => {
                title = if browser.mode == crate::fold::recovery::Mode::Trash {
                    "Trash · restore"
                } else {
                    "Undo"
                }
                .into();
                if let Some(form) = &browser.rename {
                    lines.extend(form.detail.clone());
                    field = Some((form.input.text().to_owned(), form.input.cursor()));
                    error = form.error.map(str::to_owned);
                } else {
                    error = browser.error.clone();
                    if browser.loading {
                        lines.push(format!("{} Reading…", self.app.reading_spinner()));
                    }
                    let items = browser
                        .items
                        .iter()
                        .map(|item| item.record.original.display().to_string())
                        .collect::<Vec<_>>();
                    let selected = browser.cursor;
                    self.reveal_dialog("recovery", selected);
                    let body = card(tokens)
                        .w(px(700.))
                        .max_w_full()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(title)
                        .children(lines.into_iter().map(|line| div().child(line)))
                        .children(error.map(|error| {
                            div()
                                .text_color(rgb24(self.app.theme.fold.error_fg))
                                .child(error)
                        }))
                        .child(
                            div()
                                .id("recovery-list")
                                .track_scroll(&self.dialog_scroll)
                                .max_h(px(350.))
                                .overflow_y_scroll()
                                .children(items.into_iter().enumerate().map(|(index, label)| {
                                    menu_item(label, tokens)
                                        .min_w(px(0.))
                                        .text_ellipsis()
                                        .line_clamp(1)
                                        .when(index == selected, |row| {
                                            row.bg(rgb24(tokens.selected))
                                        })
                                        .id(("recover-item", index))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(Overlay::Recovery(browser)) =
                                                this.app.overlays.current_mut()
                                            {
                                                browser.cursor = index;
                                            }
                                            cx.notify();
                                        }))
                                })),
                        )
                        .child(
                            div().flex().gap_2().children(
                                [
                                    ("Restore / undo", KeyCode::Enter),
                                    ("Refresh", KeyCode::Char('r')),
                                    ("Close", KeyCode::Esc),
                                ]
                                .into_iter()
                                .map(|(label, code)| {
                                    menu_item(label, tokens).id(label).on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.modal_key(code);
                                            cx.notify();
                                        },
                                    ))
                                }),
                            ),
                        );
                    return Some(dialog(tokens).child(body).into_any_element());
                }
                buttons.push(("Restore".into(), KeyCode::Enter));
            }
            Overlay::Sort(picker) => {
                title = "Sort by".into();
                let keys = [
                    crate::fold::sort::SortKey::Name,
                    crate::fold::sort::SortKey::Ext,
                    crate::fold::sort::SortKey::Type,
                    crate::fold::sort::SortKey::Size,
                    crate::fold::sort::SortKey::Time,
                    crate::fold::sort::SortKey::Created,
                    crate::fold::sort::SortKey::Accessed,
                ];
                let rows = keys
                    .iter()
                    .map(|key| {
                        format!(
                            "{} {}",
                            if *key == picker.order.key {
                                "●"
                            } else {
                                "○"
                            },
                            key.label()
                        )
                    })
                    .chain([
                        format!("Reverse order: {}", picker.order.reverse),
                        format!("Directories first: {}", picker.order.dirs_first),
                    ])
                    .collect::<Vec<_>>();
                let cursor = picker.cursor;
                let body = card(tokens)
                    .w(px(400.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(title)
                    .children(rows.into_iter().enumerate().map(|(index, label)| {
                        menu_item(label, tokens)
                            .when(index == cursor, |row| row.bg(rgb24(tokens.selected)))
                            .id(("sort-choice", index))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(Overlay::Sort(picker)) = this.app.overlays.current_mut()
                                {
                                    picker.cursor = index;
                                }
                                this.modal_key(KeyCode::Enter);
                                cx.notify();
                            }))
                    }));
                let body = body.child(menu_item("Cancel", tokens).id("cancel-sort").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.modal_key(KeyCode::Esc);
                        cx.notify();
                    }),
                ));
                return Some(dialog(tokens).child(body).into_any_element());
            }
            Overlay::Context(_) | Overlay::Drop(_) => return None,
        }
        buttons.push(("Cancel".into(), KeyCode::Esc));
        let mut body = card(tokens)
            .w(px(600.))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .gap_3()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().font_weight(FontWeight::BOLD).child(title))
            .child(
                div()
                    .id("dialog-details")
                    .max_h(px(280.))
                    .overflow_y_scroll()
                    .children(lines.into_iter().map(|line| div().child(line))),
            );
        if let Some((text, cursor)) = field {
            body = body.child(self.native_text(&text, cursor, cx));
        }
        if let Some(error) = error {
            body = body.child(
                div()
                    .text_color(rgb24(self.app.theme.fold.error_fg))
                    .child(error),
            );
        }
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .children(
                    buttons
                        .into_iter()
                        .enumerate()
                        .map(|(index, (label, code))| {
                            menu_item(label, tokens)
                                .id(("dialog-button", index))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.modal_key(code);
                                    cx.notify();
                                }))
                        }),
                ),
        );
        Some(
            dialog(tokens)
                .id("native-dialog")
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(body)
                .into_any_element(),
        )
    }
    fn workspace_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let cell_width = self.cell_width;
        use starkit::visual::desktop::dialog;
        let tokens = Tokens::from_theme(&self.app.theme);
        let mut body = card(tokens)
            .w(px(700.))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .gap_2()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        if let Some(places) = &self.app.places {
            use crate::ui::places::NativeView;
            body = body.child(div().font_weight(FontWeight::BOLD).child("Places"));
            match places.native_view() {
                NativeView::Browse {
                    query,
                    cursor,
                    selected,
                    items,
                    status,
                } => {
                    self.reveal_dialog("places", selected);
                    let field = self.native_text(&query, cursor, cx);
                    field.update(cx, |input, _| input.set_navigation(true));
                    body = body.child(field);
                    if let Some(status) = status {
                        body = body.child(status);
                    }
                    body = body.child(
                        div()
                            .id("places-list")
                            .track_scroll(&self.dialog_scroll)
                            .max_h(px(350.))
                            .overflow_y_scroll()
                            .children(items.into_iter().enumerate().map(|(index, item)| {
                                let label = format!("{} · {}", item.name, item.path.display());
                                menu_item(label, tokens)
                                    .min_w(px(0.))
                                    .text_ellipsis()
                                    .line_clamp(1)
                                    .when(index == selected, |row| row.bg(rgb24(tokens.selected)))
                                    .id(("place-item", index))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            if let Some(places) = &mut this.app.places {
                                                places.native_select(index);
                                            }
                                            cx.notify();
                                        }),
                                    )
                                    .on_click(cx.listener(
                                        move |this, event: &ClickEvent, _, cx| {
                                            if event.click_count() >= 2 {
                                                this.modal_key(KeyCode::Enter);
                                                cx.notify();
                                            }
                                        },
                                    ))
                            })),
                    );
                    body = body.child(
                        div().flex().flex_wrap().gap_1().children(
                            [
                                ("Open", KeyCode::Enter),
                                ("Details", KeyCode::F(3)),
                                ("Rename", KeyCode::F(2)),
                                ("Remove bookmark", KeyCode::Delete),
                                ("Refresh", KeyCode::F(5)),
                                ("Unmount…", KeyCode::F(6)),
                                ("Empty Trash…", KeyCode::F(7)),
                            ]
                            .into_iter()
                            .map(|(label, code)| {
                                menu_item(label, tokens).id(label).on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.modal_key(code);
                                        cx.notify();
                                    },
                                ))
                            }),
                        ),
                    );
                }
                NativeView::Form {
                    title,
                    detail,
                    text,
                    cursor,
                    error,
                } => {
                    body = body
                        .child(title)
                        .child(detail)
                        .child(self.native_text(&text, cursor, cx));
                    if let Some(error) = error {
                        body = body.child(
                            div()
                                .text_color(rgb24(self.app.theme.fold.error_fg))
                                .child(error),
                        );
                    }
                    body = body.child(menu_item("Save", tokens).id("save-bookmark").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.modal_key(KeyCode::Enter);
                            cx.notify();
                        }),
                    ));
                }
                NativeView::Details(lines) => {
                    body = body.child(
                        div()
                            .id("place-details")
                            .max_h(px(350.))
                            .overflow_y_scroll()
                            .children(lines.into_iter().map(|line| div().child(line))),
                    );
                }
                NativeView::Confirm(title, detail) => {
                    body = body.child(title).child(detail).child(
                        menu_item("Confirm", tokens)
                            .id("confirm-place")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.modal_key(KeyCode::Char('y'));
                                cx.notify();
                            })),
                    );
                }
            }
        } else if let Some(picker) = &mut self.app.tab_picker {
            use crate::ui::tabs::NativeView;
            let mut area = self.app.layout.last.as_ref()?.area;
            area.height = self.menu_rows;
            match picker.native_view(area) {
                NativeView::Menu(panels) => {
                    use crate::ui::popup::Entry;
                    return Some(
                        div()
                            .absolute()
                            .inset_0()
                            .id("tab-popup")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.modal_key(KeyCode::Esc);
                                    cx.notify();
                                }),
                            )
                            .children(panels.into_iter().map(|panel| {
                                let level = panel.level;
                                card(tokens)
                                    .absolute()
                                    .shadow_lg()
                                    .p_1()
                                    .flex()
                                    .flex_col()
                                    .left(px(12. + f32::from(panel.rect.x) * cell_width))
                                    .top(px(12. + f32::from(panel.rect.y) * 28.))
                                    .w(px(f32::from(panel.rect.width) * cell_width))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .children(panel.rows.into_iter().map(|row| {
                                        let (label, enabled) = match row.entry {
                                            Entry::Action { label, enabled, .. } => {
                                                (label, enabled)
                                            }
                                            Entry::Submenu { label, enabled, .. } => {
                                                (format!("{label} ›"), enabled)
                                            }
                                            Entry::Separator => {
                                                return div()
                                                    .h(px(1.))
                                                    .my_1()
                                                    .bg(rgb24(tokens.border))
                                                    .into_any_element()
                                            }
                                        };
                                        menu_item(label, tokens)
                                            .py_1()
                                            .h(px(28.))
                                            .when(row.selected && enabled, |item| {
                                                item.bg(rgb24(tokens.selected))
                                            })
                                            .when(!enabled, |item| {
                                                item.text_color(rgb24(tokens.muted))
                                            })
                                            .id(("tab-menu-row", level * 65536 + row.index))
                                            .on_hover(cx.listener(
                                                move |this, hovered: &bool, _, cx| {
                                                    if *hovered {
                                                        if let Some(picker) =
                                                            &mut this.app.tab_picker
                                                        {
                                                            picker.native_hover(level, row.index);
                                                            cx.notify();
                                                        }
                                                    }
                                                },
                                            ))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let answer =
                                                    this.app.tab_picker.as_mut().map(|picker| {
                                                        picker.native_activate(level, row.index)
                                                    });
                                                if let Some(answer) = answer {
                                                    this.app.tab_answer(answer);
                                                }
                                                cx.notify();
                                            }))
                                            .into_any_element()
                                    }))
                            }))
                            .into_any_element(),
                    );
                }
                NativeView::Rename(text, cursor) => {
                    body = body
                        .child("Rename tab")
                        .child(self.native_text(&text, cursor, cx));
                    body = body.child(menu_item("Rename", tokens).id("rename-tab").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.modal_key(KeyCode::Enter);
                            cx.notify();
                        }),
                    ));
                }
                NativeView::Browse(items, selected) => {
                    self.reveal_dialog("tabs", selected);
                    body = body.child("Tabs").child(
                        div()
                            .id("tab-picker-list")
                            .track_scroll(&self.dialog_scroll)
                            .max_h(px(350.))
                            .overflow_y_scroll()
                            .children(items.into_iter().enumerate().map(|(index, item)| {
                                menu_item(format!("{} · {}", item.label, item.location), tokens)
                                    .min_w(px(0.))
                                    .text_ellipsis()
                                    .line_clamp(1)
                                    .when(index == selected, |row| row.bg(rgb24(tokens.selected)))
                                    .id(("tab-picker-row", index))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(picker) = &mut this.app.tab_picker {
                                            picker.native_select(index);
                                        }
                                        this.modal_key(KeyCode::Right);
                                        cx.notify();
                                    }))
                            })),
                    );
                }
            }
        } else {
            return None;
        }
        body = body.child(
            menu_item("Back / close", tokens)
                .id("close-workspace-dialog")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.modal_key(KeyCode::Esc);
                    cx.notify();
                })),
        );
        Some(
            dialog(tokens)
                .id("workspace-dialog")
                .child(body)
                .into_any_element(),
        )
    }
    fn shared_modal(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(dialog) = self.authorization_dialog(cx) {
            return Some(dialog);
        }
        if let Some(menu) = self.context_menu(cx) {
            return Some(menu);
        }
        if let Some(dialog) = self.native_dialog(cx) {
            return Some(dialog);
        }
        if let Some(dialog) = self.workspace_dialog(cx) {
            return Some(dialog);
        }
        None
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .field
            .as_ref()
            .is_some_and(|field| field.read(cx).focus_handle(cx).is_focused(window))
        {
            return;
        }
        if self.operation_details.is_some() {
            if event.keystroke.key == "escape" {
                self.operation_details = None;
                cx.notify();
            }
            return;
        }
        let started = Instant::now();
        self.remember_scroll();
        let Some(key) = terminal_key(&event.keystroke) else {
            return;
        };
        let mods = key.modifiers;
        let code = key.code;
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('v') {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.app.paste_text(&text);
            }
            cx.notify();
            return;
        }
        self.app.key(key);
        if let Some(text) = self.app.clipboard_output.take() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        self.app.refresh();
        if self.app.quit {
            window.remove_window();
            cx.quit();
            return;
        }
        let pane = if self.app.commander {
            self.app.active_pane
        } else {
            0
        };
        if self.app.layout.focus() == ModuleId::Stack {
            self.reveal_cursor(pane);
        }
        self.input_metrics.frame(started.elapsed());
        cx.notify();
    }
    fn icon(
        &mut self,
        row: &panels::stack::Row,
        bg: starkit::theme::color::Rgb,
        tokens: Tokens,
    ) -> Option<Arc<RenderImage>> {
        use starkit::visual::{Icon, SurfaceKey};
        if self.icon_theme != self.app.theme_name {
            self.icons.clear();
            self.cache.clear();
            self.icon_theme = self.app.theme_name.clone();
        }
        let icon = match row.kind {
            panels::stack::Kind::Dir => Icon::Folder,
            panels::stack::Kind::Symlink { .. } => Icon::Link,
            _ => match row.ext.as_str() {
                "png" | "jpg" | "jpeg" | "webp" => Icon::Image,
                "mp3" | "flac" | "wav" => Icon::Audio,
                "mkv" | "mp4" | "webm" => Icon::Video,
                "zip" | "rar" | "tar" | "gz" => Icon::Archive,
                _ => Icon::File,
            },
        };
        let key = SurfaceKey {
            icon,
            width: 48,
            height: 48,
            foreground: tokens.accent,
            background: bg,
        };
        if let Some((_, image)) = self.icons.iter().find(|(k, _)| *k == key) {
            return Some(image.clone());
        }
        let mut rgba = self.cache.icon(key)?.to_rgba8();
        for pixel in rgba.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        let image = Arc::new(RenderImage::new(vec![starkit::image::Frame::new(rgba)]));
        if self.icons.len() >= 128 {
            self.icons.clear();
        }
        self.icons.push((key, image.clone()));
        Some(image)
    }
    fn remember_scroll(&mut self) {
        let tab = self.app.core.state().tabs.active().id;
        for (pane, frame) in self.rendered_frames.iter().enumerate() {
            if let Some((shown_tab, key)) = frame {
                if *shown_tab == tab {
                    let offset = self.lists[pane].0.borrow().base_handle.offset();
                    self.app
                        .scroll
                        .insert(*key, ((-offset.y / px(30.)).max(0.)) as usize);
                }
            }
        }
    }
    fn change_tab(&mut self, command: Command) {
        self.remember_scroll();
        match command {
            Command::CloseTab(id) => self.app.tab_action(crate::ui::tabs::Action::Close(id)),
            Command::SwitchTab(id) => self.app.tab_action(crate::ui::tabs::Action::Switch(id)),
            Command::NewTab { duplicate: false } => {
                self.app.tab_action(crate::ui::tabs::Action::New)
            }
            Command::NewTab { duplicate: true } => {
                let id = self.app.core.state().tabs.active().id;
                self.app.tab_action(crate::ui::tabs::Action::Duplicate(id));
            }
            command => self.app.change_tab(command),
        }
    }
    fn scrollbar_to(&mut self, pane: usize, y: Pixels) {
        let bounds = self.lists[pane].0.borrow().base_handle.bounds();
        let count = if self.app.commander {
            self.app.panes[pane].rows.len()
        } else {
            self.app.view.rows.len()
        };
        if bounds.size.height > px(0.) {
            let fraction = ((y - bounds.top()) / bounds.size.height).clamp(0., 1.);
            self.lists[pane].scroll_to_item_strict(
                (fraction * count.saturating_sub(1) as f32) as usize,
                ScrollStrategy::Top,
            );
        }
    }

    fn select(&mut self, pane: usize, index: usize) {
        self.app.layout.focus_set(ModuleId::Stack);
        if self.app.commander {
            self.app.focus_pane(pane);
        }
        self.app.core.send(Command::CursorTo(index));
        self.app.refresh();
    }
    fn reveal_cursor(&self, pane: usize) {
        let cursor = if self.app.commander {
            self.app.panes[pane].cursor
        } else {
            self.app.view.cursor
        };
        let handle = self.lists[pane].0.borrow().base_handle.clone();
        let mut offset = handle.offset();
        let top = px(cursor as f32 * 30.);
        let height = handle.bounds().size.height;
        if top < -offset.y {
            offset.y = -top;
        } else if top + px(30.) > -offset.y + height {
            offset.y = -(top + px(30.) - height).max(px(0.));
        }
        handle.set_offset(offset);
    }
    fn pane(&mut self, pane: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = Tokens::from_theme(&self.app.theme);
        let commander = self.app.commander;
        let (dir, count, cursor, loading, error, space) = if commander {
            let p = &self.app.panes[pane];
            (
                p.dir.clone(),
                p.rows.len(),
                p.cursor,
                p.loading,
                p.error.clone(),
                p.space,
            )
        } else {
            let v = &self.app.view;
            (
                v.active_dir.clone(),
                v.rows.len(),
                v.cursor,
                v.loading,
                v.error.clone(),
                v.space,
            )
        };
        let truncated = if commander {
            self.app.panes[pane].truncated
        } else {
            self.app.view.truncated
        };
        let frame = if commander {
            self.app.panes[pane].key
        } else {
            self.app.view.frame_id
        };
        let stamp = (self.app.core.state().tabs.active().id, frame);
        if !loading && self.rendered_frames[pane] != Some(stamp) {
            self.rendered_frames[pane] = Some(stamp);
            let row = self.app.scroll.get(&frame).copied().unwrap_or(cursor);
            let weak = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if this.rendered_frames[pane] == Some(stamp) {
                        let handle = this.lists[pane].0.borrow().base_handle.clone();
                        handle.set_offset(point(px(0.), -px(row as f32 * 30.)));
                        cx.notify();
                    }
                });
            });
        }
        let active = !commander || pane == self.app.active_pane;
        let destination = dir.clone();
        let external = dir.clone();
        let list = uniform_list(
            ("files", pane),
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                tracing::debug!(
                    pane,
                    first = range.start,
                    end = range.end,
                    "native visible rows"
                );
                range
                    .map(|i| {
                        let row = if this.app.commander {
                            this.app.panes[pane].rows[i].clone()
                        } else {
                            this.app.view.rows[i].clone()
                        };
                        let selected =
                            active && i == cursor && this.app.layout.focus() == ModuleId::Stack;
                        let marked = row.mark == panels::stack::Mark::Marked;
                        let bg = if selected {
                            this.app
                                .theme
                                .row_cursor_bg
                                .ensure_contrast(tokens.surface, 3.0)
                        } else if marked {
                            this.app.theme.fold.marked_bg
                        } else {
                            tokens.surface
                        };
                        let fg = if selected {
                            this.app.theme.row_cursor_fg.ensure_contrast(bg, 4.5)
                        } else if marked {
                            this.app.theme.fold.marked_fg.ensure_contrast(bg, 4.5)
                        } else {
                            panels::stack::kind_fg(&this.app.theme, row.kind)
                        };
                        // Capture actual paths from the current listing; display names are never reconstructed into paths.
                        let icon = this.icon(&row, bg, tokens);
                        let weak = cx.entity().downgrade();
                        let target = this.app.visual_target(pane, i);
                        let sources = target
                            .as_ref()
                            .map(|t| t.sources.clone())
                            .unwrap_or_default();
                        let click_target = target.clone();
                        let folder_target = target
                            .as_ref()
                            .filter(|t| t.directory)
                            .map(|t| t.path.clone());
                        let drag = FileDrag {
                            sources,
                            label: row.name.clone(),
                            source_pane: pane,
                        };
                        div()
                            .id(("row", i))
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_3()
                            .h(px(30.))
                            .w_full()
                            .bg(rgb24(bg))
                            .text_color(rgb24(fg))
                            .cursor_pointer()
                            .when_some(folder_target, |row, destination| {
                                let external = destination.clone();
                                row.drag_over::<FileDrag>(move |style, _, _, _| {
                                    style.bg(rgb24(tokens.selected))
                                })
                                .on_drop(cx.listener(move |this, info: &FileDrag, _, cx| {
                                    this.begin_drop(info.sources.clone(), destination.clone());
                                    this.app.refresh();
                                    cx.notify();
                                }))
                                .on_drop(cx.listener(
                                    move |this, paths: &ExternalPaths, _, cx| {
                                        this.begin_drop(paths.paths().to_vec(), external.clone());
                                        this.app.refresh();
                                        cx.notify();
                                    },
                                ))
                            })
                            .child(
                                div()
                                    .w(px(16.))
                                    .text_color(rgb24(if selected {
                                        fg
                                    } else {
                                        tokens.accent.ensure_contrast(bg, 4.5)
                                    }))
                                    .child(if marked { "●" } else { "" }),
                            )
                            .child(
                                div()
                                    .w(px(20.))
                                    .children(icon.map(|image| img(image).size(px(20.)))),
                            )
                            .child(
                                div()
                                    .id(("filename", i))
                                    .flex_1()
                                    .min_w(px(0.))
                                    .overflow_hidden()
                                    .line_clamp(1)
                                    .text_ellipsis()
                                    .tooltip({
                                        let text = row.name.clone();
                                        move |_, cx| {
                                            cx.new(|_| FilenameTip {
                                                text: text.clone(),
                                                tokens,
                                            })
                                            .into()
                                        }
                                    })
                                    .child(
                                        div()
                                            .w_full()
                                            .line_clamp(1)
                                            .text_ellipsis()
                                            .child(row.name),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(90.))
                                    .flex_shrink_0()
                                    .text_sm()
                                    .text_color(rgb24(tokens.muted))
                                    .child(row.size),
                            )
                            .when(
                                !commander || _window.viewport_size().width > px(1100.),
                                |item| {
                                    item.child(
                                        div()
                                            .w(px(70.))
                                            .flex_shrink_0()
                                            .text_color(rgb24(tokens.muted))
                                            .child(row.ext),
                                    )
                                    .child(
                                        div()
                                            .w(px(80.))
                                            .flex_shrink_0()
                                            .text_color(rgb24(tokens.muted))
                                            .child(row.time),
                                    )
                                },
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    this.select(pane, i);
                                    if event.modifiers.control || event.modifiers.platform {
                                        if let Some(target) = &click_target {
                                            this.app
                                                .core
                                                .send(Command::ToggleMarkPath(target.path.clone()));
                                        }
                                        this.app.refresh();
                                    }
                                    if event.click_count == 2 {
                                        this.app.act(crate::ui::keymap::Action::Activate);
                                    }
                                    cx.notify();
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    this.select(pane, i);
                                    this.app.open_file_menu(
                                        ((event.position.x - px(12.)) / px(this.cell_width)).max(0.)
                                            as u16,
                                        ((event.position.y - px(12.)) / px(28.)).max(0.) as u16,
                                    );
                                    cx.notify();
                                }),
                            )
                            .on_drag(drag, move |info: &FileDrag, _, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.drag_source = Some((
                                        pane,
                                        this.lists[pane].0.borrow().base_handle.offset(),
                                    ));
                                    cx.notify();
                                });
                                cx.new(|_| info.clone())
                            })
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .track_scroll(self.lists[pane].clone())
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .h_full()
        .min_h(px(0.))
        .flex_1();
        let mut panel = card(tokens)
            .id(("pane", pane))
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .gap_2()
            .border_color(rgb24(
                if active && self.app.layout.focus() == ModuleId::Stack {
                    tokens.accent
                } else {
                    tokens.border
                },
            ))
            .min_w(px(0.))
            .child(self.module_header(ModuleId::Stack, pane, cx))
            .child(
                div()
                    .id(("folded-levels", pane))
                    .flex()
                    .flex_col()
                    .max_h(px(200.))
                    .overflow_y_scroll()
                    .gap_1()
                    .children(self.app.visual_levels(pane).into_iter().map(|level| {
                        let label = level
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "/".into());
                        let context = level.cursor_name.unwrap_or_default();
                        div()
                            .id(("level", level.index))
                            .flex()
                            .gap_2()
                            .items_center()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_l_2()
                            .border_color(rgb24(tokens.border))
                            .text_color(rgb24(tokens.muted))
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb24(tokens.selected)))
                            .child("▸")
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .line_clamp(1)
                                    .text_ellipsis()
                                    .child(label),
                            )
                            .child(
                                div()
                                    .max_w(px(240.))
                                    .min_w(px(0.))
                                    .line_clamp(1)
                                    .text_ellipsis()
                                    .child(match level.count {
                                        Some(count) => format!("{count} items · {context}"),
                                        None => context,
                                    }),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.app.commander {
                                    this.app.focus_pane(pane);
                                }
                                this.app.core.send(Command::JumpTo(level.index));
                                this.app.refresh();
                                cx.notify();
                            }))
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_sm()
                            .line_clamp(1)
                            .text_ellipsis()
                            .child(format!(
                                "▾ {} · {} items{}",
                                dir.to_string_lossy(),
                                count,
                                if truncated {
                                    " · listing limit reached"
                                } else {
                                    ""
                                }
                            )),
                    )
                    .child(menu_item("↑", tokens).id("back").on_click(cx.listener(
                        move |this, _, _, cx| {
                            if this.app.commander {
                                this.app.focus_pane(pane);
                            }
                            this.app.core.send(Command::Back);
                            this.app.refresh();
                            cx.notify();
                        },
                    ))),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.app.commander {
                        this.app.focus_pane(pane);
                    }
                    cx.notify();
                }),
            )
            .drag_over::<FileDrag>(move |style, _, _, _| style.border_color(rgb24(tokens.accent)))
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<FileDrag>, _, cx| {
                    if event.bounds.contains(&event.event.position)
                        && event.drag(cx).source_pane != pane
                        && this.drag_scroll.elapsed() > Duration::from_millis(80)
                    {
                        let delta = if event.event.position.y < event.bounds.top() + px(65.) {
                            30.
                        } else if event.event.position.y > event.bounds.bottom() - px(65.) {
                            -30.
                        } else {
                            0.
                        };
                        if delta != 0. {
                            let handle = &this.lists[pane].0.borrow().base_handle;
                            let mut offset = handle.offset();
                            offset.y += px(delta);
                            handle.set_offset(offset);
                            this.drag_scroll = Instant::now();
                            cx.notify();
                        }
                    }
                }),
            )
            .on_drop(cx.listener(move |this, info: &FileDrag, _, cx| {
                this.begin_drop(info.sources.clone(), destination.clone());
                this.app.refresh();
                cx.notify();
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                this.begin_drop(paths.paths().to_vec(), external.clone());
                this.app.refresh();
                cx.notify();
            }));
        if (!self.app.commander || self.app.active_pane == pane) && self.app.filter.is_some() {
            let input = self.app.filter.as_ref().unwrap();
            let text = input.text().to_owned();
            let cursor = input.cursor();
            panel = panel.child(self.native_text(&text, cursor, cx));
        }
        panel = if loading {
            panel.child(
                div()
                    .flex_1()
                    .child(format!("{} Reading…", self.app.reading_spinner())),
            )
        } else if let Some(error) = error {
            panel.child(div().flex_1().child(error))
        } else if count == 0 {
            panel.child(
                div()
                    .flex_1()
                    .text_color(rgb24(tokens.muted))
                    .child("This level is empty"),
            )
        } else {
            panel.child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(list)
                    .child(
                        div()
                            .id(("scrollbar", pane))
                            .w(px(8.))
                            .h_full()
                            .rounded_full()
                            .bg(rgb24(tokens.border))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.scrollbar = Some(pane);
                                    this.scrollbar_to(pane, event.position.y);
                                    cx.notify();
                                }),
                            )
                            .on_mouse_move(cx.listener(
                                move |this, event: &MouseMoveEvent, _, cx| {
                                    if this.scrollbar == Some(pane)
                                        && event.pressed_button == Some(MouseButton::Left)
                                    {
                                        this.scrollbar_to(pane, event.position.y);
                                        cx.notify();
                                    }
                                },
                            )),
                    ),
            )
        };
        if let Some((total, free)) = space {
            panel = panel
                .child(meter(
                    if total > 0 {
                        1. - free as f32 / total as f32
                    } else {
                        0.
                    },
                    tokens,
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb24(tokens.muted))
                        .child(format!(
                            "{} available · {} total",
                            crate::fold::format::size(free),
                            crate::fold::format::size(total)
                        )),
                );
        }
        panel.into_any_element()
    }
}
impl Render for Desktop {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = Instant::now();
        self.field_used = false;
        self.remember_scroll();
        self.app.refresh();
        let viewport = _window.viewport_size();
        self.cell_width = starkit::visual::desktop::terminal::metrics(_window).0;
        self.menu_rows = ((viewport.height - px(24.)) / px(28.)).max(3.) as u16;
        self.app.layout.ops_active = self.app.view.ops_active || self.app.view.unmounting.is_some();
        self.app.layout.tabs_visible = true;
        self.app.layout.regions(
            Rect::new(
                0,
                0,
                ((viewport.width - px(24.)) / px(self.cell_width)).max(60.) as u16,
                ((viewport.height - px(24.)) / px(20.)).max(21.) as u16,
            ),
            (0, 0),
            (self.app.view.ops.len() + usize::from(self.app.view.unmounting.is_some()))
                .min(u16::MAX as usize) as u16,
        );
        let tokens = Tokens::from_theme(&self.app.theme);
        let active = self.app.core.state().tabs.active().id;
        let tabs = self.app.tab_items();
        if self.visible_tab != Some(active) {
            if let Some(index) = tabs.iter().position(|tab| tab.id == active) {
                self.tabs_scroll.scroll_to_item(index);
                let weak = cx.entity().downgrade();
                _window.on_next_frame(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if this.app.core.state().tabs.active().id == active {
                            this.tabs_scroll.scroll_to_item(index);
                            cx.notify();
                        }
                    });
                });
            }
            self.visible_tab = Some(active);
        }
        let tab_rail =
            div()
                .id("native-tab-rail")
                .flex_1()
                .min_w(px(0.))
                .track_scroll(&self.tabs_scroll)
                .overflow_x_scroll()
                .flex()
                .gap_2()
                .items_center()
                .children(tabs.into_iter().map(|item| {
                    tab(item.label, item.id == active, tokens)
                        .flex_shrink_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .id(("tab", item.id.0))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.app.open_tab_picker(Some(item.id));
                                if let Some(picker) = &mut this.app.tab_picker {
                                    picker.set_anchor((
                                        ((event.position.x - px(12.)) / px(this.cell_width)).max(0.)
                                            as u16,
                                        ((event.position.y - px(12.)) / px(28.)).max(0.) as u16,
                                    ));
                                }
                                cx.notify();
                            }),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.change_tab(Command::SwitchTab(item.id));
                            cx.notify();
                        }))
                        .child(menu_item("×", tokens).id(("close", item.id.0)).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.change_tab(Command::CloseTab(item.id));
                                cx.notify();
                            }),
                        ))
                }));
        let toolbar =
            div()
                .id("toolbar")
                .flex()
                .gap_2()
                .items_center()
                .child(tab_rail)
                .child(menu_item("+", tokens).id("new-tab").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.change_tab(Command::NewTab { duplicate: false });
                        cx.notify();
                    },
                )));
        let weights = self
            .app
            .layout
            .last
            .as_ref()
            .map(|regions| panels::COLUMN.map(|module| f32::from(regions.rect_of(module).height)))
            .unwrap_or([12., 4., 4.]);
        let total: f32 = weights.iter().sum();
        let budget = (viewport.height - px(120.)).max(px(300.));
        let mut heights = weights.map(|height| budget * (height / total));
        heights[1] = heights[1].max(px(84.));
        heights[2] = heights[2].max(px(96.));
        heights[0] = (budget - heights[1] - heights[2]).max(px(180.));
        let mut content = div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .h(heights[0])
            .flex_shrink_0()
            .min_h(px(0.))
            .gap_3()
            .child(self.pane(0, _window, cx));
        if self.app.commander {
            content = content.child(self.pane(1, _window, cx));
        }
        let mut modules = div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_h(px(0.))
            .gap_2()
            .child(content);
        {
            let mut preview = card(tokens)
                .flex()
                .flex_col()
                .w_full()
                .h(heights[1])
                .flex_shrink_0()
                .overflow_hidden()
                .gap_2()
                .border_color(rgb24(if self.app.layout.focus() == ModuleId::Preview {
                    tokens.accent
                } else {
                    tokens.border
                }))
                .child(self.module_header(ModuleId::Preview, 0, cx))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.app.layout.focus_set(ModuleId::Preview);
                        cx.notify();
                    }),
                )
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                    cx.stop_propagation();
                    let direction = if event.delta.pixel_delta(px(20.)).y < px(0.) {
                        1
                    } else {
                        -1
                    };
                    this.app.layout.focus_set(ModuleId::Preview);
                    this.app.request_pdf_pages(direction);
                    this.app.move_focused(direction * 3);
                    cx.notify();
                }));
            if self.app.editor.is_some() || self.app.audio_here() {
                let columns = ((viewport.width - px(48.)) / px(self.cell_width)).max(1.) as u16;
                let rows = ((heights[1] - px(50.)) / px(20.)).max(4.) as u16;
                let area = Rect::new(0, 0, columns, rows);
                let mut buffer = Buffer::empty(area);
                if let Some(editor) = &mut self.app.editor {
                    if let Err(error) = editor.render_native(area, &mut buffer, &self.app.theme) {
                        self.app
                            .finish_editor(Some(format!("Editor render failed: {error:#}")));
                    }
                } else {
                    self.app.draw_audio_cells(area, &mut buffer);
                }
                let bounds = self.tool_bounds.clone();
                preview = preview.child(
                    div()
                        .relative()
                        .id("embedded-tool")
                        .child(starkit::visual::desktop::terminal::surface(
                            &buffer,
                            self.cell_width,
                            20.,
                            tokens,
                        ))
                        .child(
                            canvas(
                                move |area, _, _| {
                                    bounds.set(area);
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .inset_0(),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                this.tool_pointer(event.position, "left");
                                cx.notify();
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                if this.app.audio.player_styles_available() {
                                    this.tool_pointer(event.position, "right");
                                }
                                cx.notify();
                            }),
                        )
                        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                            if event.pressed_button == Some(MouseButton::Left) {
                                this.tool_pointer(event.position, "drag");
                                cx.notify();
                            }
                        }))
                        .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                            if this.app.audio_here() {
                                cx.stop_propagation();
                                this.tool_pointer(
                                    event.position,
                                    if event.delta.pixel_delta(px(20.)).y < px(0.) {
                                        "scroll_down"
                                    } else {
                                        "scroll_up"
                                    },
                                );
                                cx.notify();
                            }
                        })),
                );
            } else if !self.app.layout.preview_open {
                preview = preview.child(
                    div()
                        .text_color(rgb24(tokens.muted))
                        .text_ellipsis()
                        .line_clamp(1)
                        .child(
                            self.app
                                .view
                                .preview_name
                                .clone()
                                .unwrap_or_else(|| "Nothing to preview".into()),
                        ),
                );
            } else if let Some(Preview::Image { data, .. }) = self.app.view.preview.as_deref() {
                let room = Rect::new(
                    0,
                    0,
                    ((viewport.width - px(48.)) / px(1.)).max(1.) as u16,
                    ((heights[1] - px(60.)) / px(1.)).max(1.) as u16,
                );
                let mode = self.app.cfg.preview.image_scale;
                let plan =
                    panels::preview::plan(mode, (data.width(), data.height()), room, Some((1, 1)));
                let width = u32::from(plan.rect.width).max(1);
                let height = u32::from(plan.rect.height).max(1);
                let identity = (Arc::as_ptr(data) as usize, width, height, mode);
                if self.image.as_ref().is_none_or(|(id, _, _)| *id != identity)
                    && self.image_pending.is_none()
                {
                    let source = data.clone();
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    self.image_pending = Some(rx);
                    std::thread::spawn(move || {
                        let filter = if mode == crate::config::Scale::Pixels {
                            starkit::image::imageops::FilterType::Nearest
                        } else {
                            starkit::image::imageops::FilterType::CatmullRom
                        };
                        let mut bgra = if source.width() == width && source.height() == height {
                            (*source).clone()
                        } else {
                            starkit::image::imageops::resize(source.as_ref(), width, height, filter)
                        };
                        for pixel in bgra.pixels_mut() {
                            pixel.0.swap(0, 2);
                        }
                        let _ = tx.send((identity, bgra, source));
                    });
                }
                if let Some((id, image, _)) = &self.image {
                    if *id == identity {
                        preview = preview.child(
                            div()
                                .flex_1()
                                .min_h(px(0.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .overflow_hidden()
                                .child(
                                    img(image.clone())
                                        .w(px(width as f32))
                                        .h(px(height as f32))
                                        .object_fit(ObjectFit::Contain),
                                ),
                        );
                    } else {
                        preview = preview.child("Preparing preview…");
                    }
                } else {
                    preview = preview.child("Preparing preview…");
                }
            } else {
                self.image = None;
                if let Some(p) = self.app.view.preview.as_deref() {
                    let width = ((viewport.width - px(48.)) / px(self.cell_width)).max(1.) as u16;
                    let lines = panels::preview::lines(p, width);
                    preview = preview.child(
                        div()
                            .id("preview-text")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_hidden()
                            .text_sm()
                            .children(
                                lines
                                    .into_iter()
                                    .skip(self.app.preview_scroll)
                                    .map(|line| div().child(line)),
                            ),
                    );
                } else {
                    preview = preview.child(div().text_color(rgb24(tokens.muted)).child(
                        if self.app.view.preview_name.is_some() {
                            format!("{} Reading preview…", self.app.reading_spinner())
                        } else {
                            "Nothing to preview".into()
                        },
                    ));
                }
            }
            modules = modules.child(preview);
        }
        let mut root = div()
            .relative()
            .font_family("sans-serif")
            .id("desktop")
            .capture_any_mouse_up(cx.listener(|this, _, _, _| {
                this.scrollbar = None;
            }))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgb24(tokens.background))
            .text_color(rgb24(tokens.foreground))
            .text_sm()
            .child(toolbar)
            .child(modules);
        {
            let rows = self.app.view.ops.clone();
            let ids = self.app.view.op_ids.clone();
            let fractions = {
                let state = self.app.core.state();
                state
                    .queue
                    .iter()
                    .map(|op| (op.id, op.progress.fraction() as f32))
                    .collect::<HashMap<_, _>>()
            };
            if self.app.layout.focus() == ModuleId::Operations
                && self.shown_operation != Some(self.app.ops_cursor)
            {
                self.shown_operation = Some(self.app.ops_cursor);
                let scroll = self.operations_scroll.clone();
                let cursor = self.app.ops_cursor + usize::from(self.app.view.unmounting.is_some());
                let weak = cx.entity().downgrade();
                _window.on_next_frame(move |_, cx| {
                    scroll.scroll_to_item(cursor);
                    let _ = weak.update(cx, |_, cx| cx.notify());
                });
            }
            root = root.child(
                card(tokens)
                    .h(heights[2])
                    .flex_shrink_0()
                    .id("operations")
                    .flex().flex_col().min_h(px(0.)).gap_1()
                    .border_color(rgb24(if self.app.layout.focus() == ModuleId::Operations { tokens.accent } else { tokens.border }))
                    .child(self.module_header(ModuleId::Operations, 0, cx))
                    .child(div().id("operations-list").flex_1().min_h(px(0.)).track_scroll(&self.operations_scroll).overflow_y_scroll()
                    .when(rows.is_empty(), |panel| panel.child(div().text_color(rgb24(tokens.muted)).child("No operations")))
                    .when_some(self.app.view.unmounting.clone(), |panel, name| panel.child(format!("{} Unmounting {name}", self.app.unmount_spinner())))
                    .children(rows.into_iter().enumerate().map(|(index, row)| {
                        let id = ids[index];
                        let finished = matches!(
                            row.tone,
                            panels::operations::Tone::Done | panels::operations::Tone::Failed
                        );
                        div().id(("operation-row", index))
                            .when(index == self.app.ops_cursor && self.app.layout.focus() == ModuleId::Operations, |item| item.bg(rgb24(tokens.selected)))
                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                this.app.layout.focus_set(ModuleId::Operations);
                                this.app.ops_cursor = index; cx.notify();
                            }))
                            .flex().flex_col().gap_1().py_1()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(div().flex_1().min_w(px(0.)).text_ellipsis().line_clamp(1).child(format!(
                                        "{} · {}",
                                        row.title,
                                        row.status
                                    )))
                                    .child(menu_item("Details", tokens).id(("operation-details", index)).on_click(cx.listener(move |this, _, _, cx| {
                                        this.app.layout.focus_set(ModuleId::Operations);
                                        this.app.ops_cursor = index;
                                        let failure = this.app.core.state().queue.iter().find(|op| op.id == id && op.status == OpStatus::Failed)
                                            .map(crate::ui::overlays::failure::Failure::from_op);
                                        if let Some(failure) = failure { this.app.overlays.open_failure(failure); }
                                        else { let state = this.app.core.state(); this.operation_details = operations_report(&state.queue, &state.home); }
                                        cx.notify();
                                    })))
                                    .child(
                                        menu_item(
                                            if finished { "Remove" } else { "Cancel" },
                                            tokens,
                                        )
                                        .id(("op-action", index))
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.app.core.send(if finished {
                                                    Command::RemoveOp(id)
                                                } else {
                                                    Command::Cancel(id)
                                                });
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            )
                            .child(meter(*fractions.get(&id).unwrap_or(&0.), tokens))
                            .children(row.bar.map(|bar| div().text_xs().line_clamp(1).text_ellipsis().child(bar)))
                    }))),
            );
        }
        if let Some(report) = &self.operation_details {
            root = root.child(
                starkit::visual::desktop::dialog(tokens).child(
                    card(tokens)
                        .w(px(700.))
                        .max_w_full()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .id("operation-report")
                                .max_h(px(400.))
                                .overflow_y_scroll()
                                .children(report.lines().map(|line| div().child(line.to_owned()))),
                        )
                        .child(menu_item("Copy output", tokens).id("copy-report").on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Some(report) = &this.operation_details {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        report.clone(),
                                    ));
                                }
                            }),
                        ))
                        .child(menu_item("Close", tokens).id("close-report").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.operation_details = None;
                                cx.notify();
                            }),
                        )),
                ),
            );
        }
        let note = self
            .app
            .note
            .as_ref()
            .map(|n| n.0.clone())
            .unwrap_or_else(|| {
                self.app.view.running_bar.clone().unwrap_or_else(|| {
                    format!(
                        "{} · ? help · Tab focus · c actions · y/p copy · dd delete · Alt+T theme",
                        self.app.view.marked
                    )
                })
            });
        root = root.child(
            div()
                .text_xs()
                .min_w(px(0.))
                .line_clamp(1)
                .text_ellipsis()
                .text_color(rgb24(tokens.muted))
                .child(note),
        );
        if let Some(modal) = self.shared_modal(cx) {
            root = root.child(modal);
        }
        if self.dialog_scroll_pending {
            self.dialog_scroll_pending = false;
            if let Some((_, cursor)) = self.dialog_cursor {
                let scroll = self.dialog_scroll.clone();
                let weak = cx.entity().downgrade();
                _window.on_next_frame(move |_, cx| {
                    scroll.scroll_to_item(cursor);
                    let _ = weak.update(cx, |_, cx| cx.notify());
                });
            }
        }
        if !self.app.overlays.is_open()
            && self.app.places.is_none()
            && self.app.tab_picker.is_none()
        {
            self.dialog_cursor = None;
        }
        if self.field_focus {
            if let Some(field) = &self.field {
                _window.focus(&field.read(cx).focus_handle(cx));
            }
            self.field_focus = false;
        }
        if !self.field_used {
            self.field = None;
            _window.focus(&self.focus);
        }
        self.metrics.frame(started.elapsed());
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    #[test]
    fn native_keys_preserve_shared_actions_and_ignore_unknown_named_keys() {
        for (native, terminal) in [
            ("alt-t", "alt+t"),
            ("ctrl-t", "ctrl+t"),
            ("ctrl-w", "ctrl+w"),
            ("shift-tab", "shift+tab"),
            ("shift-down", "shift+down"),
            ("ctrl-pageup", "ctrl+pgup"),
            ("alt-3", "alt+3"),
            ("f6", "F6"),
            ("escape", "esc"),
            ("shift-s", "S"),
            ("home", "home"),
            ("space", "space"),
        ] {
            let stroke = Keystroke::parse(native).unwrap();
            let key = terminal_key(&stroke).unwrap();
            assert!(
                starkit::keymap::KeySpec::parse(terminal)
                    .unwrap()
                    .matches(key),
                "{native}: {key:?}"
            );
        }
        let stroke = Keystroke::parse("capslock").unwrap();
        assert!(terminal_key(&stroke).is_none());
        let mut stroke = Keystroke::parse("a").unwrap();
        stroke.key_char = Some("界".into());
        assert_eq!(terminal_key(&stroke).unwrap().code, KeyCode::Char('界'));
    }
}
