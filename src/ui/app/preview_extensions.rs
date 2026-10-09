//! Preview owns focus and placement; extensions receive scoped content input.
use super::*;
use starfold_preview_protocol::{Input, Viewport};
#[derive(Clone, Debug)]
pub(super) enum EditorTransition {
    Preview(PathBuf),
    Action(Action),
    Tab(crate::ui::tabs::Action),
    Activate,
    Context(
        crate::ui::overlays::context::Target,
        crate::ui::overlays::context::Action,
    ),
}
impl App {
    fn preview_extension(&self) -> Option<&crate::fold::preview::extensions::Info> {
        self.view.preview.as_deref()?.extension()
    }
    pub(super) fn extension_dirty(&self) -> bool {
        self.preview_extension()
            .is_some_and(|info| info.interactive && info.modified)
    }
    pub(super) fn editor_transition(&mut self, next: EditorTransition) -> bool {
        if !self.extension_interactive() {
            return false;
        }
        if self.extension_editor_inflight.is_some() || !self.extension_editor_inputs.is_empty() {
            self.extension_editor_transition = Some(next);
            self.extension_transition_wait = true;
            return true;
        }
        if !self.extension_dirty() {
            return false;
        }
        self.extension_editor_transition = Some(next);
        self.extension_transition_wait = false;
        self.overlays.open_unsaved(
            self.view
                .preview_name
                .clone()
                .unwrap_or_else(|| "the editor buffers".into()),
        );
        self.repaint = true;
        true
    }
    pub(super) fn editor_resolve_changes(&mut self, save: bool) {
        if self.extension_editor_transition.is_none() {
            return;
        }
        self.extension_resolving_changes = true;
        self.extension_transition_wait = true;
        self.extension_input(Input::Action {
            action: if save {
                "editor-save"
            } else {
                "editor-discard"
            }
            .into(),
        });
    }
    fn editor_continue(&mut self, next: EditorTransition) {
        match next {
            EditorTransition::Preview(path) => self.core.send(Command::Preview(path)),
            EditorTransition::Action(action) => self.act(action),
            EditorTransition::Tab(action) => self.tab_action(action),
            EditorTransition::Activate => self.activate_entry(),
            EditorTransition::Context(target, action) => self.context_action(target, action),
        }
    }
    pub(super) fn editor_transition_pump(&mut self) {
        if !self.extension_transition_wait
            || self.extension_editor_inflight.is_some()
            || !self.extension_editor_inputs.is_empty()
        {
            return;
        }
        let Some(next) = self.extension_editor_transition.take() else {
            return;
        };
        self.extension_transition_wait = false;
        if self.extension_resolving_changes {
            self.extension_resolving_changes = false;
            if self.extension_dirty() {
                self.note = Some((
                    "Could not save editor changes; keeping the editor open".into(),
                    NoteLevel::Error,
                    Instant::now(),
                ));
                self.layout.focus_set(ModuleId::Preview);
                return;
            }
            self.editor_continue(next);
        } else if !self.editor_transition(next.clone()) {
            self.editor_continue(next);
        }
    }
    pub(super) fn extension_interactive(&self) -> bool {
        self.preview_extension()
            .is_some_and(|info| info.interactive)
    }
    pub(super) fn extension_editor_key(&mut self, key: KeyEvent) -> bool {
        if !self.extension_interactive()
            || self.layout.focus() != ModuleId::Preview
            || self.overlays.is_open()
            || self.places.is_some()
            || self.tab_picker.is_some()
            || self.editor.is_some()
        {
            return false;
        }
        if let Some(action @ (Action::FocusNext | Action::FocusPrev | Action::FocusPreview)) =
            keymap::resolve(key)
        {
            self.act(action);
            self.repaint = true;
            return true;
        }
        if key.code == KeyCode::F(6) {
            self.layout.focus_set(ModuleId::Stack);
            self.repaint = true;
            return true;
        }
        let mut name = match key.code {
            KeyCode::Char(' ') => "space".into(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "enter".into(),
            KeyCode::Esc => "escape".into(),
            KeyCode::Backspace => "backspace".into(),
            KeyCode::Tab => "tab".into(),
            KeyCode::BackTab => "backtab".into(),
            KeyCode::Delete => "delete".into(),
            KeyCode::Left => "left".into(),
            KeyCode::Right => "right".into(),
            KeyCode::Up => "up".into(),
            KeyCode::Down => "down".into(),
            KeyCode::Home => "home".into(),
            KeyCode::End => "end".into(),
            KeyCode::PageUp => "pageup".into(),
            KeyCode::PageDown => "pagedown".into(),
            KeyCode::F(n) => format!("<F{n}>"),
            _ => return true,
        };
        let mut prefixes = Vec::new();
        use starkit::crossterm::event::KeyModifiers;
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            prefixes.push("ctrl");
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            prefixes.push("alt");
        }
        if key.modifiers.contains(KeyModifiers::SUPER) {
            prefixes.push("super");
        }
        if key.modifiers.contains(KeyModifiers::SHIFT) && !matches!(key.code, KeyCode::Char(_)) {
            prefixes.push("shift");
        }
        if !prefixes.is_empty() {
            name = format!("{}+{name}", prefixes.join("+"));
        }
        self.extension_input(Input::Key { key: name })
    }
    pub(super) fn extension_editor_paste(&mut self, text: &str) -> bool {
        if !self.extension_interactive()
            || self.layout.focus() != ModuleId::Preview
            || self.overlays.is_open()
        {
            return false;
        }
        if text.len() > 65536 {
            self.note = Some((
                "Paste exceeds editor limit (64 KiB)".into(),
                NoteLevel::Warning,
                Instant::now(),
            ));
            return true;
        }
        self.extension_input(Input::Paste { text: text.into() })
    }
    pub(super) fn preview_extension_poll_ready(&self) -> bool {
        self.extension_editor_inputs.is_empty()
            && self.extension_editor_inflight.is_none()
            && self
                .extension_editor_polled
                .is_none_or(|at| at.elapsed() >= std::time::Duration::from_millis(80))
    }
    pub(super) fn extension_editor_pump(&mut self) {
        let Some(info) = self
            .preview_extension()
            .filter(|info| info.interactive)
            .cloned()
        else {
            if self.extension_editor_inflight.is_some() {
                self.extension_editor_closed_path = self
                    .core
                    .state()
                    .preview
                    .as_ref()
                    .map(|(path, _)| path.clone());
            }
            self.extension_editor_inputs.clear();
            self.extension_editor_inflight = None;
            return;
        };
        if self
            .extension_editor_inflight
            .is_some_and(|(session, sequence)| session != info.session || info.sequence > sequence)
        {
            self.extension_editor_inflight = None;
        }
        if self.extension_editor_inflight.is_none() {
            if let Some(input) = self.extension_editor_inputs.pop_front() {
                if self.extension_send_input(input) {
                    self.extension_editor_inflight = Some((info.session, info.sequence));
                    self.extension_editor_polled = Some(Instant::now());
                }
            }
        }
    }
    pub(super) fn extension_input(&mut self, input: Input) -> bool {
        if self.extension_interactive() {
            // Keep the latest pending position, but never cross a press/release
            // or keyboard event: those delimit separate editor gestures.
            if let Input::Pointer { action, button, .. } = &input {
                if action == "drag" {
                    if let Some(Input::Pointer {
                        action: previous_action,
                        button: previous_button,
                        ..
                    }) = self.extension_editor_inputs.back()
                    {
                        if previous_action == "drag" && previous_button == button {
                            *self.extension_editor_inputs.back_mut().unwrap() = input;
                            self.extension_editor_pump();
                            return true;
                        }
                    }
                }
            }
            if self.extension_editor_inputs.len() >= 512 {
                self.note = Some((
                    "Editor input queue full; wait for Neovim".into(),
                    NoteLevel::Warning,
                    Instant::now(),
                ));
                return true;
            }
            self.extension_editor_inputs.push_back(input);
            self.extension_editor_pump();
            true
        } else {
            self.extension_send_input(input)
        }
    }
    fn extension_send_input(&mut self, input: Input) -> bool {
        if self.preview_extension().is_none() || !self.layout.preview_open {
            return false;
        }
        let request = {
            let state = self.core.state();
            state
                .preview
                .as_ref()
                .map(|(path, _)| (path.clone(), state.preview_generation))
        };
        if let Some((path, generation)) = request {
            self.core.send(Command::PreviewInput {
                path,
                generation,
                input,
            });
            self.repaint = true;
            return true;
        }
        false
    }
    pub(super) fn extension_key(&mut self, key: &str) -> bool {
        if self.layout.focus() != ModuleId::Preview
            || self.overlays.is_open()
            || self.editor.is_some()
            || self.audio_here()
        {
            return false;
        }
        if !self
            .preview_extension()
            .is_some_and(|info| info.keys.iter().any(|k| k == key))
        {
            return false;
        }
        self.extension_input(Input::Key { key: key.into() })
    }
    pub(super) fn extension_viewport(&mut self, content: Rect, cell: (u16, u16)) {
        let Some(info) = self.preview_extension() else {
            self.extension_viewport_sent = None;
            return;
        };
        #[cfg(feature = "terminal-graphics")]
        let cell_mode = !self.graphical.as_ref().is_some_and(|g| !g.cell_mode);
        #[cfg(not(feature = "terminal-graphics"))]
        let cell_mode = true;
        let pdf =
            matches!(self.view.preview.as_deref(), Some(Preview::Document(d)) if d.kind == "PDF");
        let viewport = Viewport {
            corner_radius: if pdf {
                self.cfg.preview.video_radius()
            } else {
                0
            },
            cells: cell_mode.then_some([content.width, content.height]),
            width: u32::from(content.width) * u32::from(cell.0),
            height: u32::from(if pdf {
                content.height
            } else {
                content.height.saturating_sub(1)
            }) * u32::from(cell.1),
            foreground: format!(
                "#{:02x}{:02x}{:02x}",
                self.theme.fg.r, self.theme.fg.g, self.theme.fg.b
            ),
            background: {
                let background = if pdf {
                    self.theme.panel_bg
                } else {
                    self.theme.bg
                };
                format!(
                    "#{:02x}{:02x}{:02x}",
                    background.r, background.g, background.b
                )
            },
        };
        if viewport.width == 0 || viewport.height == 0 {
            return;
        }
        let stamp = (info.session, viewport.clone());
        if self.extension_viewport_sent.as_ref() != Some(&stamp) {
            self.extension_viewport_sent = Some(stamp);
            self.extension_input(Input::Viewport { viewport });
        }
    }
    pub(super) fn extension_pointer(
        &mut self,
        action: &str,
        button: u8,
        x: u16,
        y: u16,
        content: Rect,
    ) -> bool {
        if self.overlays.is_open()
            || self.editor.is_some()
            || self.audio_here()
            || self.places.is_some()
            || self.tab_picker.is_some()
            || self.bars.held().is_some()
            || !content.contains((x, y).into())
        {
            return false;
        }
        let Some(Preview::Document(d)) = self.view.preview.as_deref() else {
            return false;
        };
        if d.extension.is_none() {
            return false;
        }
        #[cfg(feature = "terminal-graphics")]
        let cells = self.graphical.as_ref().is_none_or(|g| g.cell_mode);
        #[cfg(not(feature = "terminal-graphics"))]
        let cells = true;
        if cells && d.cells.is_some() {
            self.layout.focus_set(ModuleId::Preview);
            return self.extension_input(Input::Pointer {
                action: action.into(),
                button,
                x: u32::from(x - content.x),
                y: u32::from(y - content.y),
            });
        }
        let (width, height) = d
            .surface
            .as_ref()
            .map(|s| (u32::from(s.width), u32::from(s.height)))
            .or_else(|| d.image.as_ref().map(|i| i.dimensions()))
            .unwrap_or((u32::from(content.width) * 8, u32::from(content.height) * 16));
        let px = (u32::from(x - content.x) * 2 + 1) * width / (u32::from(content.width).max(1) * 2);
        let py =
            (u32::from(y - content.y) * 2 + 1) * height / (u32::from(content.height).max(1) * 2);
        if action == "down" && button == 0 {
            if let Some(hit) = d.surface.as_ref().and_then(|s| s.hit(px as u16, py as u16)) {
                let action = hit.action.clone();
                self.layout.focus_set(ModuleId::Preview);
                return self.extension_input(Input::Action { action });
            }
        }
        if d.image.is_some() || d.surface.is_some() {
            self.layout.focus_set(ModuleId::Preview);
            return self.extension_input(Input::Pointer {
                action: action.into(),
                x: px,
                y: py,
                button,
            });
        }
        false
    }
    pub(super) fn extension_updated(&mut self) {
        let Some(info) = self.preview_extension().cloned() else {
            self.extension_response_seen = None;
            return;
        };
        let stamp = (info.session, info.sequence);
        if self.extension_response_seen == Some(stamp) {
            return;
        }
        self.extension_response_seen = Some(stamp);
        if !info.actions.is_empty() {
            self.core.send(Command::PreviewAck {
                session: info.session,
                sequence: info.sequence,
            });
        }
        #[cfg(feature = "terminal-graphics")]
        if self.graphical.is_some() {
            self.extension_media_actions(&info.actions);
        }
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    #[test]
    fn real_editor_keeps_burst_input_and_pinned_file_through_saves() {
        let (Ok(helper), Ok(nvim)) = (
            std::env::var("STARFOLD_TEST_NVIM_HELPER"),
            std::env::var("STARFOLD_TEST_NVIM"),
        ) else {
            return;
        };
        let mut cfg = Config::default();
        cfg.preview
            .extensions
            .providers
            .push(crate::fold::preview::extensions::Provider {
                id: "nvim".into(),
                command: vec![helper, nvim],
                extensions: vec!["txt".into()],
                mime_types: vec![],
                priority: 0,
            });
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let path = fake.home().join("editor.txt");
        std::fs::write(&path, "original\n").unwrap();
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.core.send(Command::Reload);
        fake.pump();
        let index = {
            let state = app.core.state();
            state
                .rows(state.active_frame())
                .iter()
                .position(|entry| entry.path == path)
                .unwrap()
        };
        app.core.send(Command::CursorTo(index));
        app.core.send(Command::Preview(path.clone()));
        fake.pump();
        app.refresh();
        app.layout.preview_open = true;
        app.layout.focus_set(ModuleId::Preview);
        assert!(app.extension_interactive());
        let key = |c| {
            KeyEvent::new(
                KeyCode::Char(c),
                starkit::crossterm::event::KeyModifiers::NONE,
            )
        };
        app.key(key('i'));
        let text = "abcdefghij".repeat(10);
        for c in text.chars() {
            app.key(key(c));
        }
        fn flush(app: &mut App, fake: &crate::ui::fake::Fake) {
            let deadline = Instant::now() + std::time::Duration::from_secs(10);
            loop {
                fake.pump();
                app.tick();
                if app.extension_editor_inputs.is_empty() && app.extension_editor_inflight.is_none()
                {
                    break;
                }
                assert!(Instant::now() < deadline, "Editor queue failed to drain");
            }
        }
        flush(&mut app, &fake);
        app.key(KeyEvent::new(
            KeyCode::Esc,
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        flush(&mut app, &fake);
        assert!(app.preview_extension().unwrap().modified);
        app.layout.focus_set(ModuleId::Stack);
        app.act(Action::TogglePreview);
        app.act(Action::Quit);
        assert!(app.layout.preview_open);
        assert!(!app.quit);
        assert!(app.overlays.is_open());
        app.key(KeyEvent::new(
            KeyCode::Esc,
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        app.core
            .send(Command::Preview(fake.home().join("different.txt")));
        fake.pump();
        app.refresh();
        assert_eq!(app.core.state().preview.as_ref().unwrap().0, path);
        assert_eq!(app.view.preview_name.as_deref(), Some("editor.txt"));
        app.layout.focus_set(ModuleId::Preview);
        for c in ":w".chars() {
            app.key(key(c));
        }
        app.key(KeyEvent::new(
            KeyCode::Enter,
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        flush(&mut app, &fake);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{text}original\n")
        );
        assert!(
            app.extension_interactive(),
            "Saving must retain the existing editor session across identity changes"
        );
        for c in ":q!".chars() {
            app.key(key(c));
        }
        app.key(KeyEvent::new(
            KeyCode::Enter,
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        flush(&mut app, &fake);
        assert!(!app.extension_interactive());
        app.core.send(Command::Reload);
        fake.pump();
        app.tick();
        fake.pump();
        app.refresh();
        assert!(
            !app.extension_interactive(),
            "Closing must not reopen the same file when its saved mtime reaches the listing"
        );
    }
    fn editor_fixture() -> Option<(App, crate::ui::fake::Fake, PathBuf)> {
        let helper = std::env::var("STARFOLD_TEST_NVIM_HELPER").ok()?;
        let nvim = std::env::var("STARFOLD_TEST_NVIM").ok()?;
        let mut cfg = Config::default();
        cfg.preview
            .extensions
            .providers
            .push(crate::fold::preview::extensions::Provider {
                id: "nvim".into(),
                command: vec![helper, nvim],
                extensions: vec!["txt".into()],
                mime_types: vec![],
                priority: 0,
            });
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let path = fake.home().join("editor.txt");
        std::fs::write(&path, "original\n").unwrap();
        std::fs::write(fake.home().join("second.txt"), "second\n").unwrap();
        std::fs::create_dir(fake.home().join("browse")).unwrap();
        let mut app = App::new(
            core,
            cfg,
            fake.home().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.core.send(Command::Reload);
        fake.pump();
        select(&mut app, &path);
        app.core.send(Command::Preview(path.clone()));
        fake.pump();
        app.refresh();
        app.layout.preview_open = true;
        app.layout.focus_set(ModuleId::Stack);
        Some((app, fake, path))
    }
    #[test]
    fn archive_selection_loads_real_editor_and_save_stages_before_publication() {
        use std::io::{Read, Write};
        let Some((mut app, fake, _)) = editor_fixture() else {
            return;
        };
        let file = fake.home().join("editable.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
        zip.start_file("settings.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"original\n").unwrap();
        zip.finish().unwrap();
        let original = std::fs::read(&file).unwrap();
        let root = crate::fold::location::Location::Filesystem(file.clone())
            .enter_archive()
            .unwrap()
            .key();
        app.core.send(Command::Push(root.clone()));
        fake.pump();
        app.refresh();
        let member = app.core.state().active_listing().unwrap().entries[0]
            .path
            .clone();
        select(&mut app, &member);
        app.core.send(Command::Preview(member.clone()));
        fake.pump();
        app.refresh();
        settle(&mut app, &fake, |app| app.extension_interactive());
        assert_eq!(app.core.state().preview.as_ref().unwrap().0, member);
        edit(&mut app, &fake);
        for c in ":w".chars() {
            app.extension_input(Input::Key { key: c.to_string() });
        }
        app.extension_input(Input::Key {
            key: "enter".into(),
        });
        settle(&mut app, &fake, |app| {
            !app.extension_dirty()
                && app.extension_editor_inputs.is_empty()
                && app.extension_editor_inflight.is_none()
        });
        app.core.send(Command::SyncArchiveEdits);
        fake.pump();
        assert_eq!(std::fs::read(&file).unwrap(), original);
        assert!(crate::fold::archive::edit::pending(&root) > 0);
        app.layout.focus_set(ModuleId::Stack);
        app.key(KeyEvent::new(
            KeyCode::Char('s'),
            starkit::crossterm::event::KeyModifiers::CONTROL,
        ));
        fake.pump();
        let mut published = zip::ZipArchive::new(std::fs::File::open(file).unwrap()).unwrap();
        let mut text = String::new();
        published
            .by_name("settings.txt")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "changed original\n");
        assert_eq!(crate::fold::archive::edit::pending(&root), 0);
    }

    #[test]
    fn editor_drag_coalesces_positions_without_losing_gesture_boundaries() {
        let Some((mut app, fake, _)) = editor_fixture() else {
            return;
        };
        settle(&mut app, &fake, |app| app.extension_interactive());
        let info = app.preview_extension().unwrap();
        app.extension_editor_inflight = Some((info.session, info.sequence));
        let pointer = |action: &str, x| Input::Pointer {
            action: action.into(),
            button: 0,
            x,
            y: 0,
        };
        app.extension_input(pointer("down", 0));
        for x in 1..2000 {
            app.extension_input(pointer("drag", x));
        }
        app.extension_input(pointer("up", 1999));
        app.extension_input(pointer("down", 2));
        app.extension_input(pointer("drag", 3));
        assert_eq!(app.extension_editor_inputs.len(), 5);
        assert!(matches!(
            app.extension_editor_inputs[1],
            Input::Pointer { x: 1999, .. }
        ));
        assert!(
            matches!(&app.extension_editor_inputs[2], Input::Pointer { action, .. } if action == "up")
        );
        assert!(app.note.is_none());
    }
    fn select(app: &mut App, path: &std::path::Path) {
        let index = {
            let state = app.core.state();
            state
                .rows(state.active_frame())
                .iter()
                .position(|entry| entry.path == path)
                .unwrap()
        };
        app.core.send(Command::CursorTo(index));
        app.refresh();
    }
    fn settle(app: &mut App, fake: &crate::ui::fake::Fake, ready: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            fake.pump();
            app.tick();
            if ready(app) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Preview did not settle: cursor={:?}, requested={:?}, focus_hold={:?}, navigation={}, transition={:?}, preview={:?}, note={:?}",
                app.view.cursor_path, app.last_preview_for, app.preview_focus_navigation,
                app.core.state().navigation_generation(), app.extension_editor_transition,
                app.core.state().preview.as_ref().map(|(path, _)| path), app.note
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    fn edit(app: &mut App, fake: &crate::ui::fake::Fake) {
        app.layout.focus_set(ModuleId::Preview);
        app.extension_input(Input::Key { key: "i".into() });
        app.extension_input(Input::Paste {
            text: "changed ".into(),
        });
        app.extension_input(Input::Key {
            key: "escape".into(),
        });
        settle(app, fake, |app| {
            app.extension_dirty() && app.extension_editor_inputs.is_empty()
        });
        app.layout.focus_set(ModuleId::Stack);
    }
    #[test]
    fn commander_focus_keeps_clean_and_dirty_editor_sessions() {
        let Some((mut app, fake, path)) = editor_fixture() else {
            return;
        };
        let right = fake.home().join("browse");
        std::fs::write(right.join("other.txt"), "other pane\n").unwrap();
        app.core.send(Command::RestoreCommander {
            dirs: [fake.home().into(), right],
            active: 0,
            enabled: true,
        });
        fake.pump();
        select(&mut app, &path);
        settle(&mut app, &fake, |app| app.extension_interactive());
        let session = app.preview_extension().unwrap().session;
        for dirty in [false, true] {
            if dirty {
                edit(&mut app, &fake);
            }
            app.focus_pane(0);
            app.key(KeyEvent::new(
                KeyCode::BackTab,
                starkit::crossterm::event::KeyModifiers::NONE,
            ));
            fake.pump();
            app.tick();
            assert_eq!(app.active_pane, 1);
            assert_ne!(app.view.cursor_path.as_ref(), Some(&path));
            app.key(KeyEvent::new(
                KeyCode::BackTab,
                starkit::crossterm::event::KeyModifiers::NONE,
            ));
            fake.pump();
            app.tick();
            assert_eq!(app.layout.focus(), ModuleId::Preview);
            assert_eq!(app.core.state().preview.as_ref().unwrap().0, path);
            assert_eq!(app.preview_extension().unwrap().session, session);
            assert_eq!(app.extension_dirty(), dirty);
            assert!(!app.overlays.is_open());
        }
        app.key(KeyEvent::new(
            KeyCode::F(7),
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.layout.focus(), ModuleId::Preview);
    }

    #[test]
    fn clean_editor_follows_files_reuses_session_across_directory_and_allows_quit() {
        let Some((mut app, fake, path)) = editor_fixture() else {
            return;
        };
        let session = app.preview_extension().unwrap().session;
        app.layout.focus_set(ModuleId::Preview);
        let theme = app.theme_name.clone();
        app.key(KeyEvent::new(
            KeyCode::Char('t'),
            starkit::crossterm::event::KeyModifiers::ALT,
        ));
        assert_ne!(
            app.theme_name, theme,
            "Application theme shortcut must bypass the editor"
        );
        assert!(!app.extension_dirty());
        app.key(KeyEvent::new(
            KeyCode::Tab,
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.layout.focus(), ModuleId::Stack);
        app.key(KeyEvent::new(
            KeyCode::BackTab,
            starkit::crossterm::event::KeyModifiers::SHIFT,
        ));
        assert_eq!(app.layout.focus(), ModuleId::Preview);
        app.layout.focus_set(ModuleId::Stack);
        let second = fake.home().join("second.txt");
        select(&mut app, &second);
        settle(&mut app, &fake, |app| {
            app.preview_extension()
                .is_some_and(|info| info.session == session)
                && app
                    .core
                    .state()
                    .preview
                    .as_ref()
                    .is_some_and(|(p, _)| p == &second)
        });
        let directory = fake.home().join("browse");
        select(&mut app, &directory);
        settle(&mut app, &fake, |app| {
            matches!(app.view.preview.as_deref(), Some(Preview::Dir(_)))
        });
        select(&mut app, &path);
        settle(&mut app, &fake, |app| {
            app.preview_extension()
                .is_some_and(|info| info.session == session)
        });
        app.layout.focus_set(ModuleId::Preview);
        app.key(KeyEvent::new(
            KeyCode::Char('c'),
            starkit::crossterm::event::KeyModifiers::CONTROL,
        ));
        settle(&mut app, &fake, |app| app.quit);
        assert!(!app.overlays.is_open());
    }
    #[test]
    fn dirty_quit_prompts_and_save_discard_cancel_and_write_failure_preserve_choices() {
        for choice in ['s', 'd', 'c', 'f'] {
            let Some((mut app, fake, path)) = editor_fixture() else {
                return;
            };
            edit(&mut app, &fake);
            if choice == 'f' {
                app.extension_input(Input::Key {
                    key: "escape".into(),
                });
                for c in ":set readonly".chars() {
                    app.extension_input(Input::Key { key: c.to_string() });
                }
                app.extension_input(Input::Key {
                    key: "enter".into(),
                });
                settle(&mut app, &fake, |app| {
                    app.extension_editor_inputs.is_empty()
                        && app.extension_editor_inflight.is_none()
                });
            }
            app.act(Action::Quit);
            settle(&mut app, &fake, |app| app.overlays.is_open());
            assert!(!app.quit);
            if choice == 'c' {
                app.key(KeyEvent::new(
                    KeyCode::Esc,
                    starkit::crossterm::event::KeyModifiers::NONE,
                ));
                assert!(!app.quit && app.extension_dirty());
                assert_eq!(std::fs::read_to_string(&path).unwrap(), "original\n");
            } else {
                let key = if choice == 'f' { 's' } else { choice };
                app.key(KeyEvent::new(
                    KeyCode::Char(key),
                    starkit::crossterm::event::KeyModifiers::NONE,
                ));
                settle(&mut app, &fake, |app| {
                    app.quit
                        || (app.extension_editor_transition.is_none()
                            && !app.extension_transition_wait)
                });
                if choice == 'f' {
                    assert!(
                        !app.quit && app.extension_dirty(),
                        "Failed writes must retain the editor"
                    );
                    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original\n");
                } else {
                    assert!(app.quit);
                    assert_eq!(
                        std::fs::read_to_string(&path).unwrap(),
                        if choice == 's' {
                            "changed original\n"
                        } else {
                            "original\n"
                        }
                    );
                }
            }
        }
    }
    #[test]
    fn modified_preview_prompts_before_following_cursor_and_cancel_keeps_buffer() {
        let Some((mut app, fake, path)) = editor_fixture() else {
            return;
        };
        edit(&mut app, &fake);
        let second = fake.home().join("second.txt");
        select(&mut app, &second);
        settle(&mut app, &fake, |app| app.overlays.is_open());
        assert_eq!(app.core.state().preview.as_ref().unwrap().0, path);
        app.key(KeyEvent::new(
            KeyCode::Esc,
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        app.tick();
        assert!(!app.overlays.is_open());
        assert!(app.extension_dirty());
        select(&mut app, &path);
        app.tick();
        select(&mut app, &second);
        settle(&mut app, &fake, |app| app.overlays.is_open());
        app.key(KeyEvent::new(
            KeyCode::Char('s'),
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        settle(&mut app, &fake, |app| {
            app.extension_interactive()
                && app
                    .core
                    .state()
                    .preview
                    .as_ref()
                    .is_some_and(|(p, _)| p == &second)
        });
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "changed original\n"
        );
        edit(&mut app, &fake);
        app.act(Action::TogglePreview);
        settle(&mut app, &fake, |app| app.overlays.is_open());
        app.key(KeyEvent::new(
            KeyCode::Char('d'),
            starkit::crossterm::event::KeyModifiers::NONE,
        ));
        settle(&mut app, &fake, |app| !app.layout.preview_open);
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "second\n");
    }
}
