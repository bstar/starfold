//! Preview owns focus and placement; extensions receive scoped content input.
use super::*;
use starfold_preview_protocol::{Input, Viewport};
impl App {
    fn preview_extension(&self) -> Option<&crate::fold::preview::extensions::Info> {
        self.view.preview.as_deref()?.extension()
    }
    pub(super) fn extension_input(&mut self, input: Input) -> bool {
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
        let viewport = Viewport {
            width: u32::from(content.width) * u32::from(cell.0),
            height: u32::from(content.height.saturating_sub(1)) * u32::from(cell.1),
            foreground: format!(
                "#{:02x}{:02x}{:02x}",
                self.theme.fg.r, self.theme.fg.g, self.theme.fg.b
            ),
            background: format!(
                "#{:02x}{:02x}{:02x}",
                self.theme.bg.r, self.theme.bg.g, self.theme.bg.b
            ),
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
