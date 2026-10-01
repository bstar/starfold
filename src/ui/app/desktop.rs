//! Native experimental presentation; all filesystem work stays in fold workers.
use super::*;
use crate::fold::{
    ops::{ConflictPolicy, OpKind},
    sort::SortKey,
    tab::TabId,
};
use starkit::gpui::MouseButton;
use starkit::gpui::{prelude::*, *};
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
enum Prompt {
    Filter(usize),
    Path(Vec<PathBuf>),
    TabName(TabId),
    Rename(PathBuf),
}
type ViewportKey = (TabId, (usize, FrameId));
struct Desktop {
    app: super::App,
    focus: FocusHandle,
    lists: [UniformListScrollHandle; 2],
    tabs_scroll: ScrollHandle,
    visible_tab: Option<TabId>,
    rendered_frames: [Option<ViewportKey>; 2],
    prompt: Option<Prompt>,
    input: String,
    menu: bool,
    menu_index: usize,
    tab_menu: Option<TabId>,
    menu_target: Option<visual::Target>,
    input_selected: bool,
    sort_menu: bool,
    operation_details: Option<String>,
    cache: starkit::visual::SurfaceCache,
    icons: Vec<(starkit::visual::SurfaceKey, Arc<RenderImage>)>,
    icon_theme: String,
    drag_source: Option<(usize, Point<Pixels>)>,
    drag_scroll: Instant,
    scrollbar: Option<usize>,
    menu_pos: Point<Pixels>,
    image: Option<(usize, Arc<RenderImage>, Arc<RgbaImage>)>,
    metrics: starkit::visual::Metrics,
    input_metrics: starkit::visual::Metrics,
}
impl Drop for Desktop {
    fn drop(&mut self) {
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
    let app = super::App::new(core, cfg, path, session, Graphics::disabled());
    let mut initial = Some(app);
    Application::new().run(move |cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(800.)),
                    cx,
                ))),
                app_id: Some("starfold-visual".into()),
                window_min_size: Some(size(px(660.), px(420.))),
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
                                if !cx.has_active_drag() {
                                    this.drag_source = None;
                                }
                                if let Some((pane, offset)) = this.drag_source {
                                    this.lists[pane].0.borrow().base_handle.set_offset(offset);
                                }
                                // Native controls handle collision policy rather than exposing terminal overlays.
                                this.app.overlays.close();
                                let after = (
                                    this.app.seen_version,
                                    this.app.view.running_bar.clone(),
                                    this.app.note.as_ref().map(|n| n.0.clone()),
                                );
                                if before != after {
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
                        menu_pos: Point::default(),
                        app: initial.take().unwrap(),
                        focus,
                        lists: [
                            UniformListScrollHandle::new(),
                            UniformListScrollHandle::new(),
                        ],
                        tabs_scroll: ScrollHandle::new(),
                        visible_tab: None,
                        rendered_frames: [None, None],
                        prompt: None,
                        input: String::new(),
                        menu: false,
                        menu_index: 0,
                        tab_menu: None,
                        menu_target: None,
                        input_selected: false,
                        sort_menu: false,
                        operation_details: None,
                        image: None,
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
        self.app.change_tab(command);
    }
    fn cycle_tab(&mut self, delta: i32) {
        self.remember_scroll();
        self.app.cycle_tab(delta);
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
        if self.app.commander {
            self.app.focus_pane(pane);
        }
        self.app.core.send(Command::CursorTo(index));
        self.app.refresh();
    }
    fn sources(&self) -> Vec<PathBuf> {
        let state = self.app.core.state();
        if !state.selection.is_empty() {
            state.selection.paths().map(PathBuf::from).collect()
        } else {
            state
                .cursor_entry()
                .map(|e| vec![e.path.clone()])
                .unwrap_or_default()
        }
    }
    fn copy(&mut self) {
        if self.app.commander {
            let dest = self.app.panes[1 - self.app.active_pane].dir.clone();
            self.app.core.send(Command::QueueOperation {
                kind: OpKind::Copy,
                sources: self.sources(),
                dest: Some(dest),
            });
            self.app.refresh();
        } else {
            self.prompt = Some(Prompt::Path(self.sources()));
            self.input = self.app.view.active_dir.to_string_lossy().into();
        }
    }
    fn menu_key(&mut self, key: &str, choices: &[&str]) -> String {
        match key {
            "down" | "j" => {
                self.menu_index = (self.menu_index + 1) % choices.len();
                String::new()
            }
            "up" | "k" => {
                self.menu_index = (self.menu_index + choices.len() - 1) % choices.len();
                String::new()
            }
            "enter" => choices[self.menu_index % choices.len()].into(),
            _ => key.into(),
        }
    }
    fn menu_option(&self, label: impl Into<SharedString>, index: usize, tokens: Tokens) -> Div {
        menu_item(label, tokens).when(self.menu_index == index, |option| {
            option.bg(rgb24(tokens.selected))
        })
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke.key;
        let mods = event.keystroke.modifiers;
        let input_started = Instant::now();
        if self.prompt.is_some() {
            if (mods.control || mods.platform) && key == "a" {
                self.input_selected = true;
                cx.notify();
                return;
            }
            if self.input_selected
                && (key == "backspace"
                    || (!mods.control && !mods.platform && event.keystroke.key_char.is_some()))
            {
                self.input.clear();
                self.input_selected = false;
            }

            match key.as_str() {
                "escape" => self.prompt = None,
                "backspace" => {
                    self.input.pop();
                }
                "enter" => match self.prompt.take().unwrap() {
                    Prompt::Filter(pane) => {
                        if self.app.commander {
                            self.app.focus_pane(pane);
                        }
                        self.app.core.send(Command::SetFilter(self.input.clone()));
                    }
                    Prompt::Path(sources) => self.app.core.send(Command::QueueOperation {
                        kind: OpKind::Copy,
                        sources,
                        dest: Some(PathBuf::from(&self.input)),
                    }),
                    Prompt::TabName(id) => self
                        .app
                        .core
                        .send(Command::RenameTab(id, self.input.clone())),
                    Prompt::Rename(from) => {
                        let to = from.with_file_name(&self.input);
                        self.app.core.send(Command::QueueRename { from, to });
                    }
                },
                _ => {
                    if mods.platform || mods.control {
                        if key == "v" {
                            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                                self.input.push_str(&text);
                            }
                        }
                    } else if let Some(text) = &event.keystroke.key_char {
                        self.input.push_str(text);
                    }
                }
            }
            self.app.refresh();
            cx.notify();
            return;
        }
        let pending = self
            .app
            .core
            .state()
            .queue
            .iter()
            .find(|op| op.status == OpStatus::NeedsPolicy)
            .map(|op| op.id);
        if let Some(id) = pending {
            match key.as_str() {
                "s" => self
                    .app
                    .core
                    .send(Command::SetPolicy(id, ConflictPolicy::Skip)),
                "r" => self
                    .app
                    .core
                    .send(Command::SetPolicy(id, ConflictPolicy::RenameNew)),
                "o" => self
                    .app
                    .core
                    .send(Command::SetPolicy(id, ConflictPolicy::Overwrite)),
                "escape" => self.app.core.send(Command::Cancel(id)),
                _ => {}
            }
            self.app.refresh();
            cx.notify();
            return;
        }
        if let Some(id) = self.tab_menu {
            let key = self.menu_key(key, &["r", "d", "x"]);
            match key.as_str() {
                "r" => {
                    self.input = self.app.core.state().tab_label(id);
                    self.prompt = Some(Prompt::TabName(id));
                    self.input_selected = true;
                    self.tab_menu = None;
                }
                "d" => {
                    self.change_tab(Command::SwitchTab(id));
                    self.change_tab(Command::NewTab { duplicate: true });
                    self.tab_menu = None;
                }
                "x" => {
                    self.change_tab(Command::CloseTab(id));
                    self.tab_menu = None;
                }
                "escape" => self.tab_menu = None,
                _ => {}
            }
            cx.notify();
            return;
        }
        if self.menu {
            let key = self.menu_key(key, &["o", "m", "c", "r"]);
            if key == "escape" {
                self.menu = false;
            } else if matches!(key.as_str(), "o" | "m" | "c" | "r") {
                if let Some(target) = self.menu_target.take() {
                    match key.as_str() {
                        "o" => self.app.core.send(if target.directory {
                            Command::Push(target.path)
                        } else {
                            Command::OpenExternal(target.path)
                        }),
                        "m" => self.app.core.send(Command::ToggleMarkPath(target.path)),
                        "c" => self.app.core.send(Command::QueueOperation {
                            kind: OpKind::Copy,
                            sources: target.sources,
                            dest: Some(target.destination),
                        }),
                        "r" => {
                            self.input = target
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into();
                            self.prompt = Some(Prompt::Rename(target.path));
                            self.input_selected = true;
                        }
                        _ => {}
                    }
                }
                self.menu = false;
            }
            self.app.refresh();
            cx.notify();
            return;
        }
        if self.sort_menu {
            let key = self.menu_key(key, &["1", "2", "3", "4", "5", "6", "7", "h"]);
            let keys = [
                SortKey::Name,
                SortKey::Size,
                SortKey::Time,
                SortKey::Created,
                SortKey::Accessed,
                SortKey::Ext,
                SortKey::Type,
            ];
            if key == "escape" {
                self.sort_menu = false;
            } else if key == "h" {
                let mut sort = self.app.view.sort;
                sort.reverse = !sort.reverse;
                self.app.core.send(Command::SetSort(sort));
            } else if let Ok(index) = key.parse::<usize>() {
                if let Some(key) = index.checked_sub(1).and_then(|i| keys.get(i)) {
                    let mut sort = self.app.view.sort;
                    sort.key = *key;
                    self.app.core.send(Command::SetSort(sort));
                    self.sort_menu = false;
                }
            }
            self.app.refresh();
            cx.notify();
            return;
        }
        if mods.alt && key == "t" {
            self.app.cycle_theme(false);
        } else if (mods.control || mods.platform) && key == "t" {
            self.change_tab(Command::NewTab { duplicate: false });
        } else if (mods.control || mods.platform) && key == "w" {
            let id = self.app.core.state().tabs.active().id;
            self.change_tab(Command::CloseTab(id));
        } else if mods.alt && key == "up" {
            let levels = self.app.visual_levels(if self.app.commander {
                self.app.active_pane
            } else {
                0
            });
            if let Some(level) = levels.last() {
                self.app.core.send(Command::JumpTo(level.index));
            }
        } else if mods.alt && key == "down" {
            self.app.core.send(Command::Forward);
        } else if mods.control && key == "pagedown" {
            self.cycle_tab(1);
        } else if mods.control && key == "pageup" {
            self.cycle_tab(-1);
        } else {
            match key.as_str() {
                "escape" => {
                    self.operation_details = None;
                    self.menu = false;
                    self.tab_menu = None;
                    self.sort_menu = false;
                    self.app.core.send(Command::ClearFilter);
                }
                "tab" => {
                    if self.app.commander {
                        self.app.focus_pane(1 - self.app.active_pane);
                    }
                }
                "j" | "down" => self.app.core.send(Command::CursorBy(1)),
                "k" | "up" => self.app.core.send(Command::CursorBy(-1)),
                "h" | "left" | "backspace" => self.app.core.send(Command::Back),
                "l" | "right" | "enter" => self.app.core.send(Command::Enter),
                "space" => self.app.core.send(Command::ToggleMark),
                "/" => {
                    self.prompt = Some(Prompt::Filter(self.app.active_pane));
                    self.input = self.app.view.filter.clone();
                    self.input_selected = true;
                }
                "s" => {
                    self.sort_menu = !self.sort_menu;
                    self.menu_index = 0;
                }
                "c" => self.copy(),
                "f2" => {
                    let id = self.app.core.state().tabs.active().id;
                    self.input = self.app.core.state().tab_label(id);
                    self.prompt = Some(Prompt::TabName(id));
                    self.input_selected = true;
                }
                "f6" => self.app.core.send(Command::ToggleView),
                "f5" => self.app.core.send(Command::Reload),
                "q" => {
                    window.remove_window();
                    cx.quit();
                    return;
                }
                _ => {}
            }
        }
        self.app.refresh();
        let pane = if self.app.commander {
            self.app.active_pane
        } else {
            0
        };
        self.reveal_cursor(pane);
        let weak = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.reveal_cursor(pane);
                cx.notify();
            });
        });
        self.input_metrics.frame(input_started.elapsed());
        cx.notify();
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
                        let selected = active && i == cursor;
                        let marked = row.mark == panels::stack::Mark::Marked;
                        let bg = if selected {
                            tokens.selected
                        } else if marked {
                            this.app.theme.fold.marked_bg
                        } else {
                            tokens.surface
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
                        let context_target = target;
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
                            .text_color(rgb24(if selected {
                                tokens.selected_text
                            } else {
                                tokens.foreground
                            }))
                            .cursor_pointer()
                            .when_some(folder_target, |row, destination| {
                                let external = destination.clone();
                                row.drag_over::<FileDrag>(move |style, _, _, _| {
                                    style.bg(rgb24(tokens.selected))
                                })
                                .on_drop(cx.listener(move |this, info: &FileDrag, _, cx| {
                                    this.app.core.send(Command::QueueDrop {
                                        kind: OpKind::Copy,
                                        sources: info.sources.clone(),
                                        dest: destination.clone(),
                                    });
                                    this.app.refresh();
                                    cx.notify();
                                }))
                                .on_drop(cx.listener(
                                    move |this, paths: &ExternalPaths, _, cx| {
                                        this.app.core.send(Command::QueueDrop {
                                            kind: OpKind::Copy,
                                            sources: paths.paths().to_vec(),
                                            dest: external.clone(),
                                        });
                                        this.app.refresh();
                                        cx.notify();
                                    },
                                ))
                            })
                            .child(
                                div()
                                    .w(px(16.))
                                    .text_color(rgb24(tokens.accent))
                                    .child(if marked { "●" } else { "" }),
                            )
                            .child(
                                div()
                                    .w(px(20.))
                                    .children(icon.map(|image| img(image).size(px(20.)))),
                            )
                            .child(div().flex_1().overflow_hidden().child(row.name))
                            .child(
                                div()
                                    .w(px(90.))
                                    .text_sm()
                                    .text_color(rgb24(tokens.muted))
                                    .child(row.size),
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
                                        if let Some(target) = &click_target {
                                            this.app.core.send(if target.directory {
                                                Command::Push(target.path.clone())
                                            } else {
                                                Command::OpenExternal(target.path.clone())
                                            });
                                        }
                                    }
                                    cx.notify();
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    this.select(pane, i);
                                    this.menu_target = context_target.clone();
                                    this.menu_pos = event.position;
                                    this.menu = true;
                                    this.menu_index = 0;
                                    this.tab_menu = None;
                                    this.sort_menu = false;
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
            .border_color(rgb24(if active { tokens.accent } else { tokens.border }))
            .min_w(px(240.))
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
                            .child(div().flex_1().child(label))
                            .child(match level.count {
                                Some(count) => format!("{count} items · {context}"),
                                None => context,
                            })
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
                    .child(div().flex_1().text_sm().child(format!(
                        "▾ {} · {} items{}",
                        dir.to_string_lossy(),
                        count,
                        if truncated {
                            " · listing limit reached"
                        } else {
                            ""
                        }
                    )))
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
                if this.app.commander && info.source_pane == pane {
                    this.app
                        .core
                        .send(Command::Notify("Cannot copy into the source pane".into()));
                } else {
                    this.app.core.send(Command::QueueDrop {
                        kind: OpKind::Copy,
                        sources: info.sources.clone(),
                        dest: destination.clone(),
                    });
                }
                this.app.refresh();
                cx.notify();
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                this.app.core.send(Command::QueueDrop {
                    kind: OpKind::Copy,
                    sources: paths.paths().to_vec(),
                    dest: external.clone(),
                });
                this.app.refresh();
                cx.notify();
            }));
        panel = if loading {
            panel.child(div().flex_1().child("Reading…"))
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
        self.remember_scroll();
        self.app.refresh();
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
                                this.tab_menu = Some(item.id);
                                this.menu_index = 0;
                                this.menu = false;
                                this.sort_menu = false;
                                this.menu_pos = event.position;
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
                )))
                .child(menu_item("Sort", tokens).id("sort").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.sort_menu = !this.sort_menu;
                        this.menu_index = 0;
                        this.menu = false;
                        this.tab_menu = None;
                        cx.notify();
                    },
                )))
                .child(menu_item("Copy", tokens).id("copy").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.copy();
                        cx.notify();
                    },
                )))
                .child(
                    menu_item("Commander", tokens)
                        .id("commander")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.app.core.send(Command::ToggleView);
                            this.app.refresh();
                            cx.notify();
                        })),
                );
        let mut content = div()
            .flex()
            .flex_1()
            .min_h(px(100.))
            .gap_3()
            .child(self.pane(0, _window, cx));
        if self.app.commander {
            content = content.child(self.pane(1, _window, cx));
        } else {
            let mut preview = card(tokens)
                .flex()
                .flex_col()
                .w(px(360.))
                .gap_2()
                .child("Preview");
            if let Some(Preview::Image { data, .. }) = self.app.view.preview.as_deref() {
                let identity = Arc::as_ptr(data) as usize;
                if self.image.as_ref().is_none_or(|(id, _, _)| *id != identity) {
                    tracing::debug!(
                        width = data.width(),
                        height = data.height(),
                        "native image preview surface"
                    );
                    let mut bgra = (**data).clone();
                    for pixel in bgra.pixels_mut() {
                        pixel.0.swap(0, 2);
                    }
                    self.image = Some((
                        identity,
                        Arc::new(RenderImage::new(vec![starkit::image::Frame::new(bgra)])),
                        data.clone(),
                    ));
                }
                preview = preview.child(
                    img(self.image.as_ref().unwrap().1.clone())
                        .w_full()
                        .max_h(px(480.))
                        .object_fit(ObjectFit::Contain),
                );
            } else {
                self.image = None;
                if let Some(p) = self.app.view.preview.as_deref() {
                    let lines = panels::preview::lines(p, 48);
                    preview = preview.child(
                        div()
                            .id("preview-text")
                            .overflow_y_scroll()
                            .text_sm()
                            .children(lines.into_iter().map(|line| div().child(line))),
                    );
                }
            }
            content = content.child(preview);
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.menu || this.tab_menu.is_some() || this.sort_menu {
                        this.menu = false;
                        this.tab_menu = None;
                        this.sort_menu = false;
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .bg(rgb24(tokens.background))
            .text_color(rgb24(tokens.foreground))
            .text_sm()
            .child(toolbar)
            .child(content);
        if self.sort_menu {
            root = root.child(
                card(tokens)
                    .flex()
                    .gap_2()
                    .children(
                        [
                            SortKey::Name,
                            SortKey::Size,
                            SortKey::Time,
                            SortKey::Created,
                            SortKey::Accessed,
                            SortKey::Ext,
                            SortKey::Type,
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(index, key)| {
                            self.menu_option(
                                format!("{} · {}", key.label(), index + 1),
                                index,
                                tokens,
                            )
                            .id(key.label())
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let mut order = this.app.view.sort;
                                    order.key = key;
                                    this.app.core.send(Command::SetSort(order));
                                    this.sort_menu = false;
                                    this.app.refresh();
                                    cx.notify();
                                },
                            ))
                        }),
                    )
                    .child(
                        self.menu_option("High / low · H", 7, tokens)
                            .id("reverse")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let mut sort = this.app.view.sort;
                                sort.reverse = !sort.reverse;
                                this.app.core.send(Command::SetSort(sort));
                                this.app.refresh();
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(id) = self.tab_menu {
            root = root.child(
                card(tokens)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .absolute()
                    .left(self.menu_pos.x)
                    .top(self.menu_pos.y)
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .child(
                        self.menu_option("Rename tab · R", 0, tokens)
                            .id("tab-rename")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.input = this.app.core.state().tab_label(id);
                                this.input_selected = true;
                                this.prompt = Some(Prompt::TabName(id));
                                this.tab_menu = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.menu_option("Duplicate tab · D", 1, tokens)
                            .id("tab-duplicate")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.change_tab(Command::SwitchTab(id));
                                this.change_tab(Command::NewTab { duplicate: true });
                                this.tab_menu = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.menu_option("Close tab · X", 2, tokens)
                            .id("tab-close")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.change_tab(Command::CloseTab(id));
                                this.tab_menu = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        if self.menu {
            root = root.child(
                card(tokens)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .absolute()
                    .left(self.menu_pos.x)
                    .top(self.menu_pos.y)
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.menu_option("Open · O", 0, tokens).id("open").on_click(
                        cx.listener(|this, _, _, cx| {
                            if let Some(target) = this.menu_target.take() {
                                this.app.core.send(if target.directory {
                                    Command::Push(target.path)
                                } else {
                                    Command::OpenExternal(target.path)
                                });
                            }
                            this.menu = false;
                            cx.notify();
                        }),
                    ))
                    .child(self.menu_option("Mark · M", 1, tokens).id("mark").on_click(
                        cx.listener(|this, _, _, cx| {
                            if let Some(target) = this.menu_target.take() {
                                this.app.core.send(Command::ToggleMarkPath(target.path));
                            }
                            this.menu = false;
                            cx.notify();
                        }),
                    ))
                    .child(
                        self.menu_option("Copy · C", 2, tokens)
                            .id("copy-menu")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(target) = this.menu_target.take() {
                                    this.app.core.send(Command::QueueOperation {
                                        kind: OpKind::Copy,
                                        sources: target.sources,
                                        dest: Some(target.destination),
                                    });
                                    this.app.refresh();
                                }
                                this.menu = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.menu_option("Rename · R", 3, tokens)
                            .id("rename")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(path) =
                                    this.menu_target.take().map(|target| target.path)
                                {
                                    this.input = path
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into();
                                    this.prompt = Some(Prompt::Rename(path));
                                    this.input_selected = true;
                                }
                                this.menu = false;
                                cx.notify();
                            })),
                    ),
            );
        }
        if !self.app.view.ops.is_empty() {
            let rows = self.app.view.ops.clone();
            let ids = self.app.view.op_ids.clone();
            let (paused, fractions) = {
                let state = self.app.core.state();
                (
                    state.queue.is_paused(),
                    state
                        .queue
                        .iter()
                        .map(|op| (op.id, op.progress.fraction() as f32))
                        .collect::<HashMap<_, _>>(),
                )
            };
            root = root.child(
                card(tokens)
                    .max_h(px(180.))
                    .id("operations")
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().child("Operations"))
                            .when(paused, |header| {
                                header.child(menu_item("Resume", tokens).id("resume").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.app.core.send(Command::Run);
                                        cx.notify();
                                    }),
                                ))
                            })
                            .child(menu_item("Copy output", tokens).id("copy-output").on_click(
                                cx.listener(|this, _, _, cx| {
                                    let state = this.app.core.state();
                                    if let Some(report) =
                                        operations_report(&state.queue, &state.home)
                                    {
                                        cx.write_to_clipboard(ClipboardItem::new_string(report));
                                    }
                                }),
                            ))
                            .child(menu_item("Details", tokens).id("op-details").on_click(
                                cx.listener(|this, _, _, cx| {
                                    let state = this.app.core.state();
                                    this.operation_details =
                                        operations_report(&state.queue, &state.home);
                                    cx.notify();
                                }),
                            )),
                    )
                    .children(rows.into_iter().enumerate().map(|(index, row)| {
                        let id = ids[index];
                        let finished = matches!(
                            row.tone,
                            panels::operations::Tone::Done | panels::operations::Tone::Failed
                        );
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .py_1()
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(div().flex_1().child(format!(
                                        "{} · {} {}",
                                        row.title,
                                        row.status,
                                        row.bar.unwrap_or_default()
                                    )))
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
                    })),
            );
        }
        if let Some(report) = &self.operation_details {
            root = root.child(
                card(tokens)
                    .id("operation-details")
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .children(report.lines().map(|line| div().child(line.to_owned())))
                    .child(
                        menu_item("Close details", tokens)
                            .id("close-details")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.operation_details = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        let note = self
            .app
            .note
            .as_ref()
            .map(|n| n.0.clone())
            .unwrap_or_else(|| {
                "Alt+T theme · F6 view · / filter · Space mark · C copy · F2 rename tab".into()
            });
        root = root.child(div().text_xs().text_color(rgb24(tokens.muted)).child(note));
        if let Some(prompt) = &self.prompt {
            let label = match prompt {
                Prompt::Filter(_) => "Filter",
                Prompt::Path(_) => "Copy destination",
                Prompt::TabName(_) => "Rename tab",
                Prompt::Rename(_) => "Rename file",
            };
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(0x00000088))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        card(tokens)
                            .w(px(480.))
                            .child(label)
                            .child(
                                div()
                                    .bg(rgb24(if self.input_selected {
                                        tokens.selected
                                    } else {
                                        tokens.surface
                                    }))
                                    .child(format!("{}│", self.input)),
                            )
                            .child("Enter to apply · Esc to cancel"),
                    ),
            );
        }
        let collisions: Vec<_> = self
            .app
            .core
            .state()
            .queue
            .iter()
            .filter(|op| op.status == OpStatus::NeedsPolicy)
            .map(|op| (op.id, op.plan.as_ref().map_or(0, |p| p.conflicts.len())))
            .collect();
        for (id, count) in collisions
            .into_iter()
            .take(usize::from(self.prompt.is_none()))
        {
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(0x00000088))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        card(tokens)
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(format!("{count} destination names already exist"))
                            .children(
                                [
                                    (ConflictPolicy::Skip, "Skip · S"),
                                    (ConflictPolicy::RenameNew, "Keep both (1) · R"),
                                    (ConflictPolicy::Overwrite, "Replace · O"),
                                ]
                                .into_iter()
                                .map(|(policy, label)| {
                                    menu_item(label, tokens).id(label).on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.app.core.send(Command::SetPolicy(id, policy));
                                            cx.notify();
                                        },
                                    ))
                                }),
                            )
                            .child(
                                menu_item("Cancel · Esc", tokens)
                                    .id("conflict-cancel")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.app.core.send(Command::Cancel(id));
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        self.metrics.frame(started.elapsed());
        root
    }
}
