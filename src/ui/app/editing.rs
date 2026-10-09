//! Starting and ending the editor that occupies Preview.

use super::*;

impl App {
    pub(super) fn follow_preview_editor(&mut self) {
        if !self.layout.preview_open || self.audio_here() {
            return;
        }
        if let Some(editor) = &self.editor {
            if editor.interacted || editor.origin.is_none() {
                return;
            }
            if editor.origin.as_ref() == self.view.cursor_path.as_ref() {
                return;
            }
            self.editor = None;
            self.layout.editor_active = false;
        }
        if self.extension_interactive() {
            return;
        }
        let Some(path) = self.view.cursor_path.clone() else {
            return;
        };
        if !crate::fold::location::is_archive(&path)
            || self.extension_editor_closed_path.as_ref() == Some(&path)
        {
            return;
        }
        if !matches!(self.view.preview.as_deref(), Some(Preview::Text { .. })) {
            return;
        }
        let local = {
            let state = self.core.state();
            if !state.archive_editable.contains(&path)
                || !state.preview.as_ref().is_some_and(|(p, _)| p == &path)
            {
                return;
            }
            state.archive_materialized.get(&path).cloned()
        };
        let Some(local) = local else {
            return;
        };
        let focus = self.layout.focus();
        self.start_editor(local);
        if let Some(editor) = &mut self.editor {
            editor.origin = Some(path);
        }
        self.layout.focus_set(focus);
        // Loading a preview must not resize the panes.
        self.layout.editor_active = false;
    }

    pub(super) fn start_editor(&mut self, path: PathBuf) {
        let origin = crate::fold::location::is_archive(&path).then(|| path.clone());
        let path = if origin.is_some() {
            let state = self.core.state();
            if let Some(local) = state
                .archive_materialized
                .get(&path)
                .filter(|_| state.archive_editable.contains(&path))
            {
                local.clone()
            } else {
                drop(state);
                self.core.send(Command::Preview(path));
                self.layout.preview_open = true;
                return;
            }
        } else {
            path
        };
        if self.editor.is_some() {
            return;
        }
        let size = self
            .layout
            .last
            .as_ref()
            .map(|regions| {
                let body =
                    super::super::editor::preview_content_rect(regions.rect_of(ModuleId::Preview));
                (body.width.max(1), body.height.max(1))
            })
            .unwrap_or((80, 12));
        match Editor::start(path, size) {
            Ok(mut editor) => {
                editor.origin = origin;
                if self.audio_path.is_some() {
                    self.stop_audio();
                }
                self.editor_return_focus = Some(self.layout.focus());
                self.editor = Some(editor);
                self.layout.editor_active = true;
                self.layout.focus_set(ModuleId::Preview);
                self.filter = None;
                self.g_pending = false;
                self.repaint = true;
            }
            Err(error) => {
                self.note = Some((
                    format!("Could not open editor: {error:#}"),
                    NoteLevel::Error,
                    Instant::now(),
                ));
                self.repaint = true;
            }
        }
    }

    pub(super) fn editor_key(&mut self, key: KeyEvent) {
        let result = self.editor.as_mut().map(|editor| editor.key(key));
        if let Some(Err(error)) = result {
            self.finish_editor(Some(format!("Editor input failed: {error:#}")));
        }
    }

    pub(super) fn editor_paste(&mut self, text: &str) {
        let result = self.editor.as_mut().map(|editor| editor.paste(text));
        if let Some(Err(error)) = result {
            self.finish_editor(Some(format!("Editor paste failed: {error:#}")));
        }
    }

    pub(super) fn poll_editor(&mut self) {
        let result = self.editor.as_mut().map(Editor::poll);
        match result {
            Some(Ok(Some(status))) if status.success() => self.finish_editor(None),
            Some(Ok(Some(status))) => {
                self.finish_editor(Some(format!("Editor exited with {status}")));
            }
            Some(Err(error)) => {
                self.finish_editor(Some(format!("Editor failed: {error:#}")));
            }
            _ => {}
        }
    }

    pub(super) fn finish_archive_editor(&mut self) {
        self.finish_editor(None);
    }
    fn finish_editor(&mut self, error: Option<String>) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        if let Some(origin) = editor.origin.clone() {
            self.extension_editor_closed_path = Some(origin);
            self.core.send(Command::SyncArchiveEdits);
        }
        drop(editor);
        self.layout.editor_active = false;
        self.layout
            .focus_set(self.editor_return_focus.take().unwrap_or(ModuleId::Stack));
        self.last_preview_for = None;
        self.last_preview_stamp = None;
        self.core.send(Command::Reload);
        if let Some(error) = error {
            self.note = Some((error, NoteLevel::Error, Instant::now()));
        }
        self.repaint = true;
    }
}
