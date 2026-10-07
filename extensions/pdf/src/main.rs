use anyhow::{ensure, Result};
use hayro::hayro_interpret::util::TransformExt;
use hayro::{
    hayro_interpret::InterpreterSettings,
    hayro_syntax::Pdf,
    kurbo::Affine,
    vello_cpu::{Pixmap, RenderContext},
    RenderCache, RenderSettings,
};
use starfold_preview_protocol::{Input, Limits, Presentation, Raster, Request, TextPage, Viewport};
use std::{
    io::{self, Write},
    os::unix::ffi::OsStringExt,
    path::PathBuf,
};

struct Session {
    pdf: Pdf,
    text: lopdf::Document,
    page: u32,
    text_mode: bool,
    fit_width: bool,
    zoom: f64,
    scroll: f64,
    pan: f64,
    viewport: Viewport,
    limits: Limits,
    cache: std::collections::VecDeque<(String, Vec<u8>)>,
}
impl Session {
    fn open(path: Vec<u8>, limits: Limits, viewport: Viewport) -> Result<Self> {
        let path = PathBuf::from(std::ffi::OsString::from_vec(path));
        ensure!(
            std::fs::metadata(&path)?.len() <= 256 * 1024 * 1024,
            "PDF exceeds input limit"
        );
        let bytes = std::fs::read(&path)?;
        let text = lopdf::Document::load_mem(&bytes)?;
        ensure!(
            !text.is_encrypted(),
            "Password-protected PDF; open externally"
        );
        let pdf = Pdf::new(bytes).map_err(|e| anyhow::anyhow!("Cannot render PDF: {e:?}"))?;
        ensure!(!pdf.pages().is_empty(), "PDF has no pages");
        Ok(Self {
            pdf,
            text,
            page: 1,
            text_mode: false,
            fit_width: false,
            zoom: 1.0,
            scroll: 0.0,
            pan: 0.0,
            viewport,
            limits,
            cache: Default::default(),
        })
    }
    fn input(&mut self, input: Input) {
        match input {
            Input::Viewport { viewport } => self.viewport = viewport,
            Input::Page { page } => {
                self.page = page;
                self.scroll = 0.0;
            }
            Input::Key { key } | Input::Action { action: key } => match key.as_str() {
                "right" | "pagedown" | "n" | "next" => {
                    self.page = self.page.saturating_add(1);
                    self.scroll = 0.0;
                    self.pan = 0.0;
                }
                "left" | "pageup" | "p" | "previous" => {
                    self.page = self.page.saturating_sub(1).max(1);
                    self.scroll = 0.0;
                    self.pan = 0.0;
                }
                "+" | "=" | "zoom_in" => self.zoom = (self.zoom * 1.25).min(4.0),
                "-" | "zoom_out" => self.zoom = (self.zoom / 1.25).max(0.25),
                "0" | "fit_page" => {
                    self.zoom = 1.0;
                    self.fit_width = false;
                    self.scroll = 0.0;
                    self.pan = 0.0;
                }
                "w" | "fit_width" => {
                    self.zoom = 1.0;
                    self.fit_width = true;
                    self.scroll = 0.0;
                    self.pan = 0.0;
                }
                "t" | "text" => {
                    self.text_mode = !self.text_mode;
                    self.scroll = 0.0;
                    self.pan = 0.0;
                }
                "h" | "scroll_left" => self.pan = (self.pan - 40.0).max(0.0),
                "l" | "scroll_right" => self.pan += 40.0,
                "down" | "j" | "scroll_down" => self.scroll += 40.0,
                "up" | "k" | "scroll_up" => self.scroll = (self.scroll - 40.0).max(0.0),
                _ => {}
            },
            Input::Pointer { action, .. } if action == "scroll_down" => self.scroll += 40.0,
            Input::Pointer { action, .. } if action == "scroll_up" => {
                self.scroll = (self.scroll - 40.0).max(0.0)
            }
            _ => {}
        }
        self.page = self.page.clamp(1, self.pdf.pages().len() as u32);
    }
    fn presentation(&mut self) -> Result<(Presentation, Vec<u8>)> {
        let total = self.pdf.pages().len() as u32;
        let mut p = Presentation {
            kind: "PDF".into(),
            total_pages: Some(total),
            next_page: (self.page < total).then_some(self.page + 1),
            keys: [
                "right", "left", "pagedown", "pageup", "n", "p", "+", "=", "-", "0", "w", "t",
                "down", "up", "j", "k", "h", "l",
            ]
            .map(str::to_string)
            .to_vec(),
            ..Default::default()
        };
        let mut out = Limited {
            bytes: vec![],
            cap: self.limits.text_bytes.min(262_144),
            truncated: false,
        };
        let extracted = pdf_extract::output_doc_page(
            &self.text,
            &mut pdf_extract::PlainTextOutput::new(&mut out as &mut dyn Write),
            self.page,
        );
        if extracted.is_err() && !out.truncated {
            p.notice = Some("Text extraction unavailable; rendered page remains available".into());
        }
        p.pages.push(TextPage {
            number: self.page,
            text: String::from_utf8_lossy(&out.bytes).into_owned(),
            truncated: out.truncated,
        });
        if self.text_mode {
            p.keys
                .retain(|key| !["up", "down", "j", "k", "h", "l"].contains(&key.as_str()));
            return Ok((p, vec![]));
        }
        let page = &self.pdf.pages()[(self.page - 1) as usize];
        let (pw, ph) = page.render_dimensions();
        ensure!(
            pw.is_finite() && ph.is_finite() && pw > 0.0 && ph > 0.0,
            "Invalid PDF page dimensions; use text mode"
        );
        let cap = self.limits.image_dimension.clamp(1, 4096);
        let width = self.viewport.width.clamp(1, cap).min(2048);
        let height = self.viewport.height.clamp(1, cap).min(2048);
        let fit = if self.fit_width {
            width as f64 / pw as f64
        } else {
            (width as f64 / pw as f64).min(height as f64 / ph as f64)
        };
        let scale = fit * self.zoom;
        self.scroll = self
            .scroll
            .min((ph as f64 * scale - height as f64).max(0.0));
        self.pan = self.pan.min((pw as f64 * scale - width as f64).max(0.0));
        let x = if pw as f64 * scale > width as f64 {
            -self.pan
        } else {
            ((width as f64 - pw as f64 * scale) / 2.0).max(0.0)
        };
        let y = if self.fit_width || self.zoom > 1.0 {
            -self.scroll
        } else {
            ((height as f64 - ph as f64 * scale) / 2.0).max(0.0)
        };
        let cache_key = format!(
            "{}:{width}:{height}:{}:{}:{}:{}",
            self.page,
            self.fit_width,
            self.zoom.to_bits(),
            self.scroll.to_bits(),
            self.pan.to_bits()
        );
        p.raster = Some(Raster {
            width,
            height,
            page: self.page,
        });
        p.fields.push((
            "View".into(),
            format!(
                "Page {}/{total} · {:.0}% · n/p pages · +/- zoom · w width · 0 fit · t text",
                self.page,
                self.zoom * 100.0
            ),
        ));
        if let Some(index) = self.cache.iter().position(|(key, _)| key == &cache_key) {
            let cached = self.cache.remove(index).unwrap();
            let pixels = cached.1.clone();
            self.cache.push_back(cached);
            return Ok((p, pixels));
        }
        let mut context = RenderContext::new(width as u16, height as u16);
        let transform = Affine::translate((x, y))
            * Affine::scale(scale)
            * page.initial_transform(true).to_kurbo();
        hayro::render_into(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &mut context,
            transform,
        );
        context.flush();
        let mut pixmap = Pixmap::new(width as u16, height as u16);
        context.render_with(
            &mut pixmap,
            &mut Default::default(),
            hayro::vello_cpu::RasterizerSettings {
                target_init: hayro::vello_cpu::TargetInit::Clear(
                    hayro::vello_cpu::color::palette::css::WHITE,
                ),
                ..Default::default()
            },
        );
        let pixels = pixmap.data_as_u8_slice().to_vec();
        self.cache.push_back((cache_key, pixels.clone()));
        while self.cache.len() > 24
            || self
                .cache
                .iter()
                .map(|(_, bytes)| bytes.len())
                .sum::<usize>()
                > self.limits.cache_bytes.min(128 * 1024 * 1024)
        {
            self.cache.pop_front();
        }
        Ok((p, pixels))
    }
}
// The extraction library may write an entire page in one call.
struct Limited {
    bytes: Vec<u8>,
    cap: usize,
    truncated: bool,
}
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = bytes.len().min(self.cap.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&bytes[..n]);
        if n < bytes.len() {
            self.truncated = true;
            return Err(io::Error::other("page text limit"));
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn main() -> Result<()> {
    let mut session = None;
    starfold_preview_protocol::serve(
        "pdf",
        env!("CARGO_PKG_VERSION"),
        &["documents", "raster", "input"],
        |request| {
            match request {
                Request::Open {
                    path,
                    limits,
                    viewport,
                } => session = Some(Session::open(path, limits, viewport)?),
                Request::Input { input } => session
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("No PDF session"))?
                    .input(input),
                _ => anyhow::bail!("Unsupported request"),
            }
            let s = session.as_mut().unwrap();
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.presentation())) {
                Ok(Ok(result)) => Ok(result),
                failed => {
                    let notice = match failed {
                        Ok(Err(e)) => format!("Rendering unavailable: {e}; t toggles text mode"),
                        _ => "PDF renderer failed; t toggles text mode".into(),
                    };
                    s.text_mode = true;
                    let (mut p, bytes) = s.presentation()?;
                    p.notice = Some(notice);
                    Ok((p, bytes))
                }
            }
        },
    )
}
