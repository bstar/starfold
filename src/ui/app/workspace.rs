//! UI contexts are parked by TabId; live processes are never serialized.
use super::*;
use crate::fold::tab::TabId;
use crate::ui::tabs::{self, Action as TabAction};

pub(super) struct UiContext {
    layout: LayoutState,
    scroll: HashMap<(usize, FrameId), usize>,
    filter: Option<TextInput>,
    preview_scroll: usize,
    preview_page: Option<u32>,
    editor: Option<Editor>,
    editor_return_focus: Option<ModuleId>,
}

impl App {
    fn fresh_tab_ui(&self) -> UiContext {
        UiContext {
            layout: LayoutState::new(
                self.cfg.ui.preview_rows,
                self.cfg.ui.ops_rows,
                self.cfg.ui.fold_rows,
            ),
            scroll: HashMap::new(),
            filter: None,
            preview_scroll: 0,
            preview_page: None,
            editor: None,
            editor_return_focus: None,
        }
    }
    pub(super) fn init_workspaces(&mut self) {
        let saved = self
            .session_path
            .as_ref()
            .map(|p| session::load(p))
            .unwrap_or_default();
        let restored = self.core.state().tab_snapshots();
        let identities: Vec<_> = self.core.state().tabs.tabs.iter().map(|t| t.id).collect();
        for (index, id) in identities.iter().enumerate() {
            let mut context = self.fresh_tab_ui();
            if let Some(tab) = saved.tabs.get(index) {
                context.layout.focus_set(match tab.focus.as_str() {
                    "preview" => ModuleId::Preview,
                    "operations" => ModuleId::Operations,
                    _ => ModuleId::Stack,
                });
                context.layout.preview_open = tab.preview_open;
                let actual = &restored[index];
                let same_location = tab
                    .stacks
                    .get(tab.active_stack)
                    .and_then(|stack| stack.frames.get(stack.active))
                    .map(|frame| &frame.dir)
                    == actual
                        .stacks
                        .get(actual.active_stack)
                        .and_then(|stack| stack.frames.get(stack.active))
                        .map(|frame| &frame.dir);
                if same_location {
                    context.preview_scroll = tab.preview_scroll;
                    context.preview_page = tab.preview_page;
                }
                if let Some(search) = actual.search.as_ref().and(tab.search.as_ref()) {
                    context.scroll.insert((3, FrameId(1)), search.scroll);
                }
                for (stack_index, stack) in tab.stacks.iter().enumerate() {
                    for (frame_index, frame) in stack.frames.iter().enumerate() {
                        let same_frame = actual
                            .stacks
                            .get(stack_index)
                            .and_then(|stack| stack.frames.get(frame_index))
                            .is_some_and(|current| current.dir == frame.dir);
                        if same_frame {
                            context
                                .scroll
                                .insert((stack_index, FrameId(frame_index as u64)), frame.scroll);
                        }
                    }
                }
            }
            self.tab_ui.insert(*id, context);
        }
        let active = self.core.state().tabs.active().id;
        if let Some(context) = self.tab_ui.remove(&active) {
            self.restore_tab_ui(context);
        }
        if let Some(path) = self.session_path.clone() {
            match session::Writer::acquire(path) {
                Ok(Some(writer)) => self.session_writer = Some(writer),
                Ok(None) => {
                    self.note = Some((
                        "Another window owns tab restoration; this window will not overwrite it"
                            .into(),
                        NoteLevel::Info,
                        Instant::now(),
                    ))
                }
                Err(error) => {
                    tracing::warn!("workspace persistence unavailable: {error:#}");
                    self.note = Some((
                        "Could not save tab workspace; see log".into(),
                        NoteLevel::Warning,
                        Instant::now(),
                    ));
                }
            }
        }
    }
    fn preview_position(&self) -> (Option<u32>, usize) {
        if let Some(page) = self.pending_preview_page {
            return (Some(page), self.preview_scroll);
        }
        if let Some(Preview::Document(document)) = self.view.preview.as_deref() {
            if let crate::fold::preview::model::Content::Pages(pages) = &document.content {
                let width = self
                    .layout
                    .last
                    .as_ref()
                    .map(|r| panels::preview::content_rect(r.rect_of(ModuleId::Preview)).width)
                    .unwrap_or(80);
                let mut offset = self.preview_scroll;
                for (index, page) in pages.iter().enumerate() {
                    let rows = panels::preview::page_rows(page, width);
                    if offset < rows || index + 1 == pages.len() {
                        return (Some(page.number), offset);
                    }
                    offset = offset.saturating_sub(rows);
                }
            }
        }
        (None, self.preview_scroll)
    }
    fn take_tab_ui(&mut self) -> UiContext {
        let fresh = self.fresh_tab_ui();
        let (preview_page, preview_scroll) = self.preview_position();
        UiContext {
            layout: std::mem::replace(&mut self.layout, fresh.layout),
            scroll: std::mem::take(&mut self.scroll),
            filter: self.filter.take(),
            preview_scroll,
            preview_page,
            editor: self.editor.take(),
            editor_return_focus: self.editor_return_focus.take(),
        }
    }
    fn restore_tab_ui(&mut self, context: UiContext) {
        self.layout = context.layout;
        self.layout.last = None;
        self.scroll = context.scroll;
        if let Some(search) = &self.core.state().search {
            self.scroll
                .entry((3, FrameId(search.generation)))
                .or_insert(search.view);
        }
        self.filter = context.filter;
        self.preview_scroll = context.preview_scroll;
        self.pending_preview_page = context.preview_page;
        self.view.preview = None;
        self.editor = context.editor;
        self.editor_return_focus = context.editor_return_focus;
        self.layout.audio_active = self.audio_here();
        self.last_preview_for = None;
        self.last_preview_stamp = None;
        self.pdf_requested = None;
        self.preserve_preview_scroll = true;
        self.g_pending = false;
        self.d_pending = false;
        self.tab_hits.clear();
        self.bars = Bars::new();
        self.clicks = ClickTracker::new();
        self.forget_picture();
        self.audio_presentation = None;
        self.seen_version = u64::MAX;
        self.repaint = true;
    }
    fn change_tab(&mut self, command: Command) {
        let old = self.core.state().tabs.active().id;
        let context = self.take_tab_ui();
        self.tab_ui.insert(old, context);
        self.core.send(command);
        let active = self.core.state().tabs.active().id;
        let context = self
            .tab_ui
            .remove(&active)
            .unwrap_or_else(|| self.fresh_tab_ui());
        self.restore_tab_ui(context);
        self.refresh();
    }
    pub(super) fn audio_here(&self) -> bool {
        self.audio_path.is_some()
            && self
                .audio_tab
                .is_none_or(|id| id == self.core.state().tabs.active().id)
    }
    pub(super) fn tab_items(&self) -> Vec<tabs::Item> {
        let state = self.core.state();
        state
            .tabs
            .tabs
            .iter()
            .map(|tab| {
                let mut badges = vec![];
                let search = tab
                    .context
                    .as_ref()
                    .and_then(|c| c.search.as_ref())
                    .or_else(|| {
                        (tab.id == state.tabs.active().id)
                            .then_some(state.search.as_ref())
                            .flatten()
                    });
                if search.is_some_and(|s| !s.errors.is_empty())
                    || tab.stacks.iter().any(|stack| {
                        state
                            .listing_of(&stack.active().dir)
                            .is_some_and(|listing| listing.error.is_some())
                    })
                {
                    badges.push("!".into());
                }
                for op in state
                    .queue
                    .iter()
                    .filter(|op| op.origin_tab == Some(tab.id))
                {
                    match op.status {
                        OpStatus::Running => badges.push(op.progress.percent()),
                        OpStatus::Planning | OpStatus::Queued => badges.push("…".into()),
                        OpStatus::NeedsPolicy | OpStatus::Failed => badges.push("!".into()),
                        _ => {}
                    }
                }
                if state.tab_search_running(tab.id) {
                    badges.push("search".into());
                }
                if self.audio_path.is_some() && self.audio_tab == Some(tab.id) {
                    badges.push("♫".into());
                }
                if self.tab_ui.get(&tab.id).is_some_and(|c| c.editor.is_some()) {
                    badges.push("edit".into());
                }
                badges.dedup();
                tabs::Item {
                    id: tab.id,
                    label: state.tab_label(tab.id),
                    location: home_relative(&tab.active_stack().active().dir, &state.home),
                    badge: badges.join(" "),
                }
            })
            .collect()
    }
    pub(super) fn open_tab_picker(&mut self, menu: Option<TabId>) {
        if self.dnd.drag_active || self.dnd.choice.is_some() {
            return;
        }
        self.tab_picker = Some(match menu {
            Some(id) => tabs::Picker::menu(self.tab_items(), id),
            None => tabs::Picker::new(self.tab_items(), self.core.state().tabs.active().id),
        });
        let anchor = menu
            .and_then(|id| {
                self.tab_hits.iter().find_map(|(r, h)| {
                    matches!(h,tabs::Hit::Tab(tab) if *tab==id).then_some((r.x + 3, r.y + 1))
                })
            })
            .unwrap_or((0, 0));
        let reopen = self.core.state().can_reopen_tab();
        self.tab_picker.as_mut().unwrap().configure(anchor, reopen);
        self.repaint = true;
    }
    pub(super) fn tab_answer(&mut self, answer: tabs::Answer) {
        match answer {
            // The normal frame diff draws cursor and input changes. A full
            // repaint clears the terminal and makes every menu key flicker.
            tabs::Answer::Consumed => return,
            tabs::Answer::Action(action) => {
                self.tab_picker = None;
                self.tab_action(action);
            }
            tabs::Answer::Renamed(id, name) => {
                self.tab_picker = None;
                self.core.send(Command::RenameTab(id, name));
                self.refresh();
            }
        }
        self.repaint = true;
    }
    pub(super) fn tab_hit(&mut self, hit: tabs::Hit, menu: bool) {
        match hit {
            tabs::Hit::Close(id) if menu => self.open_tab_picker(Some(id)),
            tabs::Hit::Close(id) => self.tab_action(TabAction::Close(id)),
            tabs::Hit::Tab(id) if menu => self.open_tab_picker(Some(id)),
            tabs::Hit::Tab(id) => self.tab_action(TabAction::Switch(id)),
            tabs::Hit::New => self.tab_action(TabAction::New),
            tabs::Hit::Previous => self.cycle_tab(-1),
            tabs::Hit::Next => self.cycle_tab(1),
        }
    }
    pub(super) fn cycle_tab(&mut self, delta: i32) {
        let id = {
            let state = self.core.state();
            let index = (state.tabs.index() as i64 + i64::from(delta))
                .rem_euclid(state.tabs.tabs.len() as i64) as usize;
            state.tabs.tabs[index].id
        };
        self.tab_action(TabAction::Switch(id));
    }
    pub(super) fn tab_action(&mut self, action: TabAction) {
        if self.dnd.drag_active || self.dnd.choice.is_some() {
            return;
        }
        match action {
            TabAction::New => {
                let preview_open = self.layout.preview_open;
                self.change_tab(Command::NewTab { duplicate: false });
                self.layout.preview_open = preview_open;
            }
            TabAction::Switch(id) => {
                if id != self.core.state().tabs.active().id {
                    self.change_tab(Command::SwitchTab(id));
                }
            }
            TabAction::Duplicate(id) => {
                if id != self.core.state().tabs.active().id {
                    self.change_tab(Command::SwitchTab(id));
                }
                let source = self.core.state().tabs.active().id;
                let mut saved = self.workspace_snapshot();
                let source_saved = saved.tabs.remove(saved.active_tab);
                self.change_tab(Command::NewTab { duplicate: true });
                // Duplicate presentation, without duplicating a live editor.
                self.layout.focus_set(match source_saved.focus.as_str() {
                    "preview" => ModuleId::Preview,
                    "operations" => ModuleId::Operations,
                    _ => ModuleId::Stack,
                });
                self.layout.preview_open = source_saved.preview_open;
                self.preview_scroll = source_saved.preview_scroll;
                self.pending_preview_page = source_saved.preview_page;
                if let Some(context) = self.tab_ui.get(&source) {
                    self.scroll = context.scroll.clone();
                }
                if let Some(saved) = &source_saved.search {
                    if let Some(search) = &self.core.state().search {
                        self.scroll
                            .insert((3, FrameId(search.generation)), saved.scroll);
                    }
                }
            }
            TabAction::Close(id) => {
                if self.core.state().tabs.tabs.len() == 1 {
                    self.core.send(Command::CloseTab(id));
                    return;
                }
                let editor = if id == self.core.state().tabs.active().id {
                    self.editor.is_some()
                } else {
                    self.tab_ui.get(&id).is_some_and(|c| c.editor.is_some())
                };
                if editor {
                    self.overlays.open_confirm(Confirm {
                        title: "close tab".into(),
                        body: vec!["Close the editor and tab? Unsaved edits may be lost.".into()],
                        yes: "close",
                        no: "keep",
                        pending: Pending::CloseTab(id),
                    });
                } else {
                    self.finish_tab_close(id);
                }
            }
            TabAction::Move(id, delta) => {
                self.core.send(Command::MoveTab(id, delta));
                self.refresh();
            }
            TabAction::Reopen => {
                let old = self.core.state().tabs.active().id;
                self.change_tab(Command::ReopenTab);
                if self.core.state().tabs.active().id != old {
                    if let Some(mut context) = self.closed_tab_ui.pop() {
                        context.scroll.clear();
                        {
                            let state = self.core.state();
                            for (stack_index, stack) in
                                state.tabs.active().stacks.iter().enumerate()
                            {
                                for frame in stack.frames() {
                                    context.scroll.insert((stack_index, frame.id), frame.view);
                                }
                            }
                        }
                        self.restore_tab_ui(context);
                        self.refresh();
                    }
                }
            }
            TabAction::Rename(id) => self.open_tab_picker(Some(id)),
            TabAction::Dismiss => {}
        }
        self.repaint = true;
    }
    pub(super) fn finish_tab_close(&mut self, id: TabId) {
        let rows = if id == self.core.state().tabs.active().id {
            &self.scroll
        } else if let Some(context) = self.tab_ui.get(&id) {
            &context.scroll
        } else {
            return;
        };
        let rows = rows
            .iter()
            .map(|(&(stack, frame), &scroll)| (stack, frame, scroll))
            .collect();
        self.core.send(Command::RememberTabScroll { tab: id, rows });
        let active = self.core.state().tabs.active().id;
        if active == id {
            self.editor = None;
            self.layout.editor_active = false;
        }
        self.change_tab(Command::CloseTab(id));
        if let Some(mut context) = self.tab_ui.remove(&id) {
            context.editor = None;
            context.layout.editor_active = false;
            self.closed_tab_ui.push(context);
            if self.closed_tab_ui.len() > 10 {
                self.closed_tab_ui.remove(0);
            }
        }
        if self.audio_tab == Some(id) {
            self.audio_tab = Some(self.core.state().tabs.active().id);
            self.layout.audio_active = true;
        }
    }
    pub(super) fn poll_parked_editors(&mut self) {
        for (id, context) in &mut self.tab_ui {
            let result = context.editor.as_mut().map(Editor::poll);
            if let Some(Err(error)) = &result {
                tracing::warn!(tab = id.0, "background editor failed: {error:#}");
            }
            if let Some(Ok(Some(status))) = &result {
                tracing::info!(tab = id.0, %status, "background editor exited");
            }
            if matches!(result, Some(Ok(Some(_)) | Err(_))) {
                context.editor = None;
                context.layout.editor_active = false;
                if let Some(focus) = context.editor_return_focus.take() {
                    context.layout.focus_set(focus);
                }
            }
        }
    }
    fn workspace_snapshot(&self) -> session::Session {
        let state = self.core.state();
        let mut tabs = state.tab_snapshots();
        for tab in &mut tabs {
            let id = TabId(tab.id);
            let (layout, scroll, preview_scroll) = if id == state.tabs.active().id {
                (&self.layout, &self.scroll, self.preview_scroll)
            } else if let Some(c) = self.tab_ui.get(&id) {
                (&c.layout, &c.scroll, c.preview_scroll)
            } else {
                continue;
            };
            tab.focus = match layout.focus() {
                ModuleId::Preview => "preview",
                ModuleId::Operations => "operations",
                _ => "stack",
            }
            .into();
            tab.preview_open = layout.preview_open;
            tab.preview_scroll = preview_scroll;
            tab.preview_page = if id == state.tabs.active().id {
                let (page, offset) = self.preview_position();
                tab.preview_scroll = offset;
                page
            } else {
                self.tab_ui.get(&id).and_then(|c| c.preview_page)
            };
            let core_tab = state.tabs.tabs.iter().find(|t| t.id == id).unwrap();
            if let Some(saved) = &mut tab.search {
                let search = if id == state.tabs.active().id {
                    state.search.as_ref()
                } else {
                    core_tab.context.as_ref().and_then(|c| c.search.as_ref())
                };
                if let Some(search) = search {
                    saved.scroll = scroll
                        .get(&(3, FrameId(search.generation)))
                        .copied()
                        .unwrap_or(search.view);
                }
            }
            for (index, stack) in tab.stacks.iter_mut().enumerate() {
                for (frame, core_frame) in
                    stack.frames.iter_mut().zip(core_tab.stacks[index].frames())
                {
                    frame.scroll = scroll
                        .get(&(index, core_frame.id))
                        .copied()
                        .unwrap_or(core_frame.view);
                }
            }
        }
        session::Session {
            version: Some(2),
            active_tab: state.tabs.index(),
            tabs,
            last_dir: Some(state.tabs.active().stacks[0].active().dir.clone()),
            commander: Some(state.commander),
            commander_active: Some(state.commander_pane),
            commander_left: state
                .tabs
                .active()
                .stacks
                .get(1)
                .map(|s| s.active().dir.clone()),
            commander_right: state
                .tabs
                .active()
                .stacks
                .get(2)
                .map(|s| s.active().dir.clone()),
            show_hidden: Some(state.show_hidden),
            sort: Some(state.sort),
            commander_left_sort: Some(state.pane_sorts[0]),
            commander_right_sort: Some(state.pane_sorts[1]),
        }
    }
    pub(super) fn save_workspace(&mut self, force: bool) {
        if self.session_writer.is_none()
            || (!force && self.session_checked.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.session_checked = Instant::now();
        let session = self.workspace_snapshot();
        if force || self.session_saved.as_ref() != Some(&session) {
            self.session_writer.as_ref().unwrap().save(session.clone());
            self.session_saved = Some(session);
        }
    }
}

pub(super) fn operation_row(op: &Op, state: &crate::fold::State) -> panels::operations::OpRow {
    let mut row = build_op_row(op, &state.home, state.queue.is_paused());
    if !op.origin_name.is_empty()
        && (state.tabs.tabs.len() > 1 || op.origin_tab != Some(state.tabs.active().id))
    {
        row.title = format!("[{}] {}", op.origin_name, row.title);
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fake;
    use starkit::crossterm::event::KeyModifiers;
    fn app() -> (App, fake::Fake, tempfile::TempDir) {
        let cfg = Config::default();
        let (core, fake) = fake::handle(cfg.core());
        let dir = tempfile::tempdir().unwrap();
        (
            App::new(
                core,
                cfg,
                dir.path().join("config.toml"),
                None,
                Graphics::disabled(),
            ),
            fake,
            dir,
        )
    }
    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }
    #[test]
    fn shortcuts_and_mouse_switch_independent_ui_contexts_at_floor() {
        let (mut app, _, _) = app();
        let first = app.core.state().tabs.active().id;
        app.preview_scroll = 37;
        app.layout.preview_open = false;
        app.layout.focus_set(ModuleId::Operations);
        app.key(ctrl(KeyCode::Char('t')));
        let second = app.core.state().tabs.active().id;
        assert_ne!(first, second);
        assert_eq!(app.preview_scroll, 0);
        app.preview_scroll = 5;
        app.key(ctrl(KeyCode::PageUp));
        assert_eq!(app.preview_scroll, 37);
        assert!(!app.layout.preview_open);
        assert_eq!(app.layout.focus(), ModuleId::Operations);
        let area = Rect::new(0, 0, 60, 21);
        let mut buffer = Buffer::empty(area);
        app.draw(area, &mut buffer);
        let (rect, _) = app
            .tab_hits
            .iter()
            .find(|(_, hit)| matches!(hit, tabs::Hit::Tab(id) if *id == second))
            .unwrap();
        let (x, y) = (rect.x, rect.y);
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.core.state().tabs.active().id, second);
        assert_eq!(app.preview_scroll, 5);
        app.key(ctrl(KeyCode::Char('w')));
        assert_eq!(app.core.state().tabs.tabs.len(), 1);
        app.draw(area, &mut buffer);
        assert!(app.layout.last.as_ref().unwrap().tabs.is_none());
    }
    #[test]
    fn tab_close_button_closes_its_tab_without_switching_to_it() {
        let (mut app, _, _) = app();
        let first = app.core.state().tabs.active().id;
        app.key(ctrl(KeyCode::Char('t')));
        let second = app.core.state().tabs.active().id;
        let area = Rect::new(0, 0, 60, 21);
        app.draw(area, &mut Buffer::empty(area));
        let (rect, _) = app
            .tab_hits
            .iter()
            .find(|(_, hit)| matches!(hit, tabs::Hit::Close(id) if *id == first))
            .unwrap();
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 1,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.core.state().tabs.tabs.len(), 1);
        assert_eq!(app.core.state().tabs.active().id, second);
        assert!(app.tab_picker.is_none());
        app.draw(area, &mut Buffer::empty(area));
        assert!(app.tab_hits.is_empty());
    }
    #[test]
    fn tab_right_click_can_rename_an_inactive_tab_with_file_menus_disabled() {
        let (mut app, _, _) = app();
        app.cfg.ui.right_click = false;
        let first = app.core.state().tabs.active().id;
        app.key(ctrl(KeyCode::Char('t')));
        let second = app.core.state().tabs.active().id;
        let area = Rect::new(0, 0, 60, 21);
        app.draw(area, &mut Buffer::empty(area));
        let (rect, _) = app
            .tab_hits
            .iter()
            .find(|(_, hit)| matches!(hit, tabs::Hit::Tab(id) if *id == first))
            .unwrap();
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: rect.x + 3,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(app.tab_picker.is_some());
        for code in [KeyCode::Down, KeyCode::Down, KeyCode::Enter] {
            app.key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        for c in " renamed".chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.core.state().tab_label(first).ends_with(" renamed"));
        assert_eq!(app.core.state().tabs.active().id, second);
        assert!(app.tab_picker.is_none());
    }
    #[test]
    fn scrollbar_and_drag_ownership_prevent_tab_switches() {
        let (mut app, _, _) = app();
        let first = app.core.state().tabs.active().id;
        app.dnd.drag_active = true;
        app.key(ctrl(KeyCode::Char('t')));
        assert_eq!(app.core.state().tabs.tabs.len(), 1);
        app.dnd.drag_active = false;
        app.key(ctrl(KeyCode::Char('t')));
        app.dnd.drag_active = true;
        app.tab_action(TabAction::Switch(first));
        assert_ne!(app.core.state().tabs.active().id, first);
    }
    #[test]
    fn audio_has_one_owner_and_survives_switching_and_owner_closure() {
        let (mut app, _, _) = app();
        let first = app.core.state().tabs.active().id;
        app.audio_path = Some("/music/playing.flac".into());
        app.audio_tab = Some(first);
        app.tab_action(TabAction::New);
        assert!(!app.audio_here());
        assert_eq!(app.audio_path, Some("/music/playing.flac".into()));
        app.tab_action(TabAction::Switch(first));
        assert!(app.audio_here());
        app.tab_action(TabAction::Close(first));
        assert!(app.audio_here());
        assert_eq!(app.audio_tab, Some(app.core.state().tabs.active().id));
    }
    #[test]
    fn editor_survives_switch_and_close_confirmation_accepts_keys() {
        use std::ffi::OsString;
        let (mut app, _, dir) = app();
        app.tab_action(TabAction::New);
        let owner = app.core.state().tabs.active().id;
        let path = dir.path().join("edit.txt");
        std::fs::write(&path, "").unwrap();
        let argv = vec![
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from("printf READY; IFS= read -r line"),
        ];
        app.editor = Some(Editor::spawn(path, (40, 8), argv).unwrap());
        app.layout.editor_active = true;
        app.key(ctrl(KeyCode::PageUp));
        assert!(app.editor.is_none());
        assert!(app.tab_ui.get(&owner).unwrap().editor.is_some());
        app.key(ctrl(KeyCode::PageDown));
        assert!(app.editor.is_some());
        app.tab_action(TabAction::Close(owner));
        assert!(app.overlays.is_open());
        app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(app.core.state().tabs.tabs.len(), 1);
        assert!(app.editor.is_none());
    }
    #[test]
    fn closed_tab_reopens_actual_scroll_after_frame_ids_are_reallocated() {
        let (mut app, _, _) = app();
        app.tab_action(TabAction::New);
        app.core.send(Command::Push("/second".into()));
        app.core.send(Command::JumpTo(0));
        app.core.send(Command::Push("/third".into()));
        let frame = app.core.state().active_frame().id;
        assert_eq!(frame, FrameId(2));
        app.scroll.insert((0, frame), 13);
        let id = app.core.state().tabs.active().id;
        app.tab_action(TabAction::Close(id));
        app.tab_action(TabAction::Reopen);
        let new_frame = app.core.state().active_frame().id;
        assert_eq!(new_frame, FrameId(1));
        assert_eq!(app.scroll.get(&(0, new_frame)), Some(&13));
    }
    #[test]
    fn workspace_snapshot_and_restore_keep_ui_and_never_marks_or_processes() {
        let (mut app, _, dir) = app();
        app.core.send(Command::ToggleMark);
        app.tab_action(TabAction::New);
        app.layout.focus_set(ModuleId::Preview);
        app.preview_scroll = 31;
        app.pending_preview_page = Some(8);
        let id = app.core.state().tabs.active().id;
        app.core.send(Command::RenameTab(id, "project two".into()));
        let saved = app.workspace_snapshot();
        let path = dir.path().join("session.toml");
        saved.save(&path).unwrap();
        let cfg = Config::default();
        let (core, _) = fake::handle(cfg.core());
        core.send(Command::RestoreTabs(saved.tabs, saved.active_tab));
        let restored = App::new(
            core,
            cfg,
            dir.path().join("config.toml"),
            Some(path),
            Graphics::disabled(),
        );
        assert_eq!(restored.core.state().tabs.tabs.len(), 2);
        assert!(restored.core.state().selection.is_empty());
        assert!(restored.core.state().queue.iter().next().is_none());
        assert_eq!(restored.preview_scroll, 31);
        assert_eq!(restored.pending_preview_page, Some(8));
        assert_eq!(restored.layout.focus(), ModuleId::Preview);
        assert!(restored.audio_path.is_none());
        assert!(restored.editor.is_none());
    }
    #[test]
    fn pdf_position_restores_by_visible_page_within_a_larger_cache() {
        use crate::fold::preview::model::{Content, Document, Page};
        let (mut app, fake, _) = app();
        let pages: Vec<_> = (4..=9)
            .map(|number| Page {
                number,
                text: "line one\nline two\nline three".into(),
                truncated: false,
            })
            .collect();
        let mut document = Document::new("PDF");
        document.content = Content::Pages(pages.clone());
        document.total_pages = Some(9);
        let prefix = pages[..3]
            .iter()
            .map(|p| panels::preview::page_rows(p, 80))
            .sum::<usize>();
        app.view.preview = Some(Arc::new(Preview::Document(document.clone())));
        app.preview_scroll = prefix + 2;
        assert_eq!(app.preview_position(), (Some(7), 2));
        app.tab_action(TabAction::New);
        app.key(ctrl(KeyCode::PageUp));
        assert_eq!(app.pending_preview_page, Some(7));
        assert_eq!(app.preview_scroll, 2);
        let path = app.core.state().cursor_entry().unwrap().path.clone();
        app.last_preview_for = Some(path.clone());
        app.last_preview_stamp = app.core.state().cursor_entry().map(|e| (e.len, e.modified));
        {
            let mut state = fake.state_mut();
            state.preview = Some((path, Arc::new(Preview::Document(document))));
            state.version += 1;
        }
        app.tick();
        assert_eq!(app.pending_preview_page, None);
        assert_eq!(app.preview_scroll, prefix + 2);
    }
    #[test]
    fn search_scroll_survives_duplicate_restart_and_reopen_with_new_generation() {
        let (mut app, fake, dir) = app();
        app.core.send(Command::StartSearch("star".into()));
        fake.pump();
        app.refresh();
        let generation = app.core.state().search.as_ref().unwrap().generation;
        app.scroll.insert((3, FrameId(generation)), 6);
        let first = app.core.state().tabs.active().id;
        let saved = app.workspace_snapshot();
        assert_eq!(saved.tabs[0].search.as_ref().unwrap().scroll, 6);
        app.tab_action(TabAction::Duplicate(first));
        let generation = app.core.state().search.as_ref().unwrap().generation;
        assert_eq!(app.scroll.get(&(3, FrameId(generation))), Some(&6));
        let id = app.core.state().tabs.active().id;
        app.tab_action(TabAction::Close(id));
        app.tab_action(TabAction::Reopen);
        let generation = app.core.state().search.as_ref().unwrap().generation;
        assert_eq!(app.scroll.get(&(3, FrameId(generation))), Some(&6));
        let path = dir.path().join("search-session.toml");
        saved.save(&path).unwrap();
        let cfg = Config::default();
        let (core, _) = fake::handle(cfg.core());
        core.send(Command::RestoreTabs(saved.tabs, 0));
        let restored = App::new(
            core,
            cfg,
            dir.path().join("config.toml"),
            Some(path),
            Graphics::disabled(),
        );
        let generation = restored.core.state().search.as_ref().unwrap().generation;
        assert_eq!(restored.scroll.get(&(3, FrameId(generation))), Some(&6));
    }
    #[test]
    fn cli_override_does_not_apply_old_location_scroll_or_preview() {
        let (mut app, _, dir) = app();
        app.tab_action(TabAction::New);
        app.preview_scroll = 31;
        app.pending_preview_page = Some(8);
        let frame = app.core.state().active_frame().id;
        app.scroll.insert((0, frame), 20);
        let saved = app.workspace_snapshot();
        let path = dir.path().join("override-session.toml");
        saved.save(&path).unwrap();
        let (tabs, active) = crate::restore_workspace(&saved, Some("/new/project".into()));
        let cfg = Config::default();
        let (core, _) = fake::handle(cfg.core());
        core.send(Command::RestoreTabs(tabs, active));
        let restored = App::new(
            core,
            cfg,
            dir.path().join("config.toml"),
            Some(path),
            Graphics::disabled(),
        );
        assert_eq!(restored.preview_scroll, 0);
        assert_eq!(restored.pending_preview_page, None);
        assert_eq!(restored.scroll.get(&(0, FrameId(0))), None);
        assert_eq!(
            restored.core.state().active_frame().dir,
            PathBuf::from("/new/project")
        );
    }
}
