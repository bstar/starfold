//! Starting and ending the editor that occupies Preview.

use super::*;

impl App {
    pub(super) fn start_editor(&mut self, path: PathBuf) {
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
            Ok(editor) => {
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

    fn finish_editor(&mut self, error: Option<String>) {
        let Some(editor) = self.editor.take() else {
            return;
        };
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
