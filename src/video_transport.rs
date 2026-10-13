//! Cached in-process KIT controls. No player/helper process or device is started.
use starkit::media_controls as controls;
use starkit::native_surface::Surface;
use std::cell::RefCell;
use std::collections::VecDeque;

pub use controls::CellTransport;
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub cells: Option<[u16; 2]>,
    pub width: u16,
    pub height: u16,
    pub theme: crate::audio_embed::Palette,
    pub playing: bool,
    pub paused: bool,
    pub volume: f32,
    pub pointer: Option<[u16; 2]>,
    pub movie: bool,
    pub picker: bool,
    pub entries: Vec<String>,
    pub selected: usize,
    pub title: String,
    pub font: u16,
}
impl Request {
    fn shared(&self) -> controls::Request {
        let p = &self.theme;
        controls::Request {
            cells: self.cells,
            width: self.width,
            height: self.height,
            theme: controls::Palette {
                bg: p.bg,
                inset_bg: None,
                fg: p.fg,
                muted: p.muted,
                accent: p.accent,
                selected: p.selected,
                border: p.border,
                error: p.error,
            },
            playing: self.playing,
            paused: self.paused,
            volume: self.volume,
            pointer: self.pointer,
            movie: self.movie,
            picker: self.picker,
            entries: self.entries.clone(),
            selected: self.selected,
            title: self.title.clone(),
            font: self.font,
        }
    }
}
#[derive(Default)]
pub struct Client {
    frame: Option<(Request, controls::Response)>,
    actions: RefCell<VecDeque<(String, Option<f32>)>>,
    error: RefCell<Option<String>>,
}
impl Client {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn render(&mut self, request: Request) -> Option<Surface> {
        if self.frame.as_ref().is_none_or(|(key, _)| key != &request) {
            match controls::render(&request.shared()) {
                Ok(response) => self.frame = Some((request, response)),
                Err(error) => {
                    *self.error.borrow_mut() = Some(format!("Video controls: {error:#}"));
                    self.frame = None;
                }
            }
        }
        self.frame.as_ref()?.1.surface.clone()
    }
    pub fn cells(&self, request: &Request) -> Option<CellTransport> {
        self.frame
            .as_ref()
            .filter(|(key, _)| key == request)?
            .1
            .cells
            .clone()
    }
    pub fn pointer(&self, mut request: Request, x: u16, y: u16) {
        request.pointer = Some([x, y]);
        match controls::render(&request.shared()) {
            Ok(response) => {
                if let Some(action) = response.action {
                    let mut actions = self.actions.borrow_mut();
                    if actions.len() < 8 {
                        actions.push_back((action, response.value));
                    }
                }
            }
            Err(error) => *self.error.borrow_mut() = Some(format!("Video controls: {error:#}")),
        }
    }
    pub fn poll(&self) -> (bool, Option<String>) {
        (false, self.error.borrow_mut().take())
    }
    pub fn action(&self) -> Option<(String, Option<f32>)> {
        self.actions.borrow_mut().pop_front()
    }
}
