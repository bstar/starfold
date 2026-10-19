//! Supervised CBZ/CBR reader. Archive members are decoded in memory, never extracted.
use anyhow::{ensure, Result};
use starfold_preview_protocol::{Input, Limits, Presentation, Raster, Request, TextPage, Viewport};
use starkit::image::{self, RgbaImage};
use std::{
    cmp::Ordering,
    io::{self, Write},
    os::unix::ffi::OsStringExt,
    path::PathBuf,
};

const MAX_PAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;

struct Session {
    path: PathBuf,
    pages: Vec<(usize, starfold_archive::Entry)>,
    page: u32,
    decoded: Option<(u32, RgbaImage)>,
    limits: Limits,
    viewport: Viewport,
    zoom: f64,
    fit_width: bool,
    pan: f64,
    scroll: f64,
}
impl Session {
    fn open(path: Vec<u8>, limits: Limits, viewport: Viewport) -> Result<Self> {
        let path = PathBuf::from(std::ffi::OsString::from_vec(path));
        ensure!(
            matches!(
                starfold_archive::detect(&path)?,
                starfold_archive::Format::Zip | starfold_archive::Format::Rar
            ),
            "Comic must be a ZIP or RAR archive"
        );
        let (entries, partial) = starfold_archive::list(&path, MAX_ENTRIES)?;
        ensure!(!partial, "Comic exceeds {MAX_ENTRIES} archive entries");
        let mut pages: Vec<_> = entries
            .into_iter()
            .enumerate()
            .filter(|(_, entry)| {
                !entry.directory
                    && !entry
                        .name
                        .split('/')
                        .any(|s| s.starts_with('.') || s == "__MACOSX")
                    && std::path::Path::new(&entry.name)
                        .extension()
                        .is_some_and(|ext| {
                            ["jpg", "jpeg", "png", "webp", "gif", "bmp"]
                                .iter()
                                .any(|s| ext.eq_ignore_ascii_case(s))
                        })
            })
            .collect();
        pages.sort_by(|(_, a), (_, b)| {
            natural_cmp(&a.name.to_lowercase(), &b.name.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });
        ensure!(!pages.is_empty(), "Comic has no supported image pages");
        Ok(Self {
            path,
            pages,
            page: 1,
            decoded: None,
            limits,
            viewport,
            zoom: 1.0,
            fit_width: false,
            pan: 0.0,
            scroll: 0.0,
        })
    }
    fn input(&mut self, input: Input) {
        let old = self.page;
        match input {
            Input::Viewport { viewport } => self.viewport = viewport,
            Input::Page { page } => self.page = page,
            Input::Key { key } | Input::Action { action: key } => match key.as_str() {
                "n" | "right" | "pagedown" | "next" => self.page = self.page.saturating_add(1),
                "p" | "left" | "pageup" | "previous" => self.page = self.page.saturating_sub(1),
                "home" => self.page = 1,
                "end" => self.page = self.pages.len() as u32,
                "+" | "=" | "zoom_in" => self.zoom = (self.zoom * 1.25).min(4.0),
                "-" | "zoom_out" => self.zoom = (self.zoom / 1.25).max(0.25),
                "0" | "fit_page" => {
                    self.zoom = 1.0;
                    self.fit_width = false;
                    self.pan = 0.0;
                    self.scroll = 0.0;
                }
                "w" | "fit_width" => {
                    self.zoom = 1.0;
                    self.fit_width = true;
                    self.pan = 0.0;
                    self.scroll = 0.0;
                }
                "down" | "j" | "scroll_down" => self.scroll += 80.0,
                "up" | "k" | "scroll_up" => self.scroll = (self.scroll - 80.0).max(0.0),
                "h" | "scroll_left" => self.pan = (self.pan - 80.0).max(0.0),
                "l" | "scroll_right" => self.pan += 80.0,
                _ => {}
            },
            Input::Pointer { action, .. } if action == "scroll_down" => self.scroll += 80.0,
            Input::Pointer { action, .. } if action == "scroll_up" => {
                self.scroll = (self.scroll - 80.0).max(0.0)
            }
            _ => {}
        }
        self.page = self.page.clamp(1, self.pages.len() as u32);
        if self.page != old {
            self.scroll = 0.0;
            self.pan = 0.0;
        }
    }
    fn presentation(&mut self) -> Result<(Presentation, Vec<u8>)> {
        let (index, entry) = &self.pages[(self.page - 1) as usize];
        if self
            .decoded
            .as_ref()
            .is_none_or(|(page, _)| *page != self.page)
        {
            ensure!(
                entry
                    .bytes
                    .is_none_or(|bytes| bytes <= MAX_PAGE_BYTES as u64),
                "Comic page exceeds 32 MiB"
            );
            self.decoded = None;
            let mut data = Limited(Vec::new());
            starfold_archive::copy_member(&self.path, *index, &mut data, None)?;
            let mut reader =
                image::ImageReader::new(io::Cursor::new(&data.0)).with_guessed_format()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(16384);
            limits.max_image_height = Some(16384);
            limits.max_alloc = Some(128 * 1024 * 1024);
            reader.limits(limits);
            let image = reader.decode()?.to_rgba8();
            ensure!(
                image.as_raw().len() <= 128 * 1024 * 1024,
                "Comic page exceeds decoded image limit"
            );
            self.decoded = Some((self.page, image));
        }
        let source = &self.decoded.as_ref().unwrap().1;
        let cap = self.limits.image_dimension.clamp(1, 4096) as f64;
        let vw = self.viewport.width.max(1) as f64;
        let vh = self.viewport.height.max(1) as f64;
        let reduction = (cap / vw.max(vh))
            .min((8_000_000.0 / (vw * vh)).sqrt())
            .min(1.0);
        let width = (vw * reduction).floor().max(1.0) as u32;
        let height = (vh * reduction).floor().max(1.0) as u32;
        let fit = if self.fit_width {
            width as f64 / source.width() as f64
        } else {
            (width as f64 / source.width() as f64).min(height as f64 / source.height() as f64)
        };
        let scale = fit * self.zoom;
        self.pan = self
            .pan
            .min((source.width() as f64 * scale - width as f64).max(0.0));
        self.scroll = self
            .scroll
            .min((source.height() as f64 * scale - height as f64).max(0.0));
        let x = ((width as f64 - source.width() as f64 * scale) / 2.0).max(0.0) as u32;
        let y = if self.fit_width {
            0
        } else {
            ((height as f64 - source.height() as f64 * scale) / 2.0).max(0.0) as u32
        };
        let sx = (self.pan / scale).floor() as u32;
        let sy = (self.scroll / scale).floor() as u32;
        let sw = ((width as f64 / scale).ceil() as u32)
            .max(1)
            .min(source.width() - sx);
        let sh = ((height as f64 / scale).ceil() as u32)
            .max(1)
            .min(source.height() - sy);
        let cropped = image::imageops::crop_imm(source, sx, sy, sw, sh);
        let resized = image::imageops::resize(
            &*cropped,
            ((sw as f64 * scale).round() as u32).max(1).min(width - x),
            ((sh as f64 * scale).round() as u32).max(1).min(height - y),
            image::imageops::FilterType::Triangle,
        );
        let mut output =
            RgbaImage::from_pixel(width, height, background(&self.viewport.background));
        image::imageops::overlay(&mut output, &resized, x as i64, y as i64);
        let label = format!("Page {} / {} · {}", self.page, self.pages.len(), entry.name);
        let p = Presentation {
            kind: "Comic book".into(),
            fields: vec![("View".into(), label.clone())],
            pages: vec![TextPage {
                number: self.page,
                text: label,
                truncated: false,
            }],
            total_pages: Some(self.pages.len() as u32),
            next_page: (self.page < self.pages.len() as u32).then_some(self.page + 1),
            raster: Some(Raster {
                width,
                height,
                page: self.page,
            }),
            keys: [
                "n", "p", "left", "right", "pageup", "pagedown", "home", "end", "+", "=", "-", "0",
                "w", "up", "down", "j", "k", "h", "l",
            ]
            .map(str::to_string)
            .to_vec(),
            notice: Some(
                "n/p pages · +/- zoom · 0 fit page · w fit width · j/k scroll · h/l pan".into(),
            ),
            ..Default::default()
        };
        Ok((p, output.into_raw()))
    }
}
fn background(value: &str) -> image::Rgba<u8> {
    let value = value.strip_prefix('#').unwrap_or("");
    let rgb = u32::from_str_radix(value, 16).unwrap_or(0);
    image::Rgba([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255])
}
struct Limited(Vec<u8>);
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_PAGE_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("Comic page exceeds 32 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    while let (Some(&ac), Some(&bc)) = (a.first(), b.first()) {
        if ac.is_ascii_digit() && bc.is_ascii_digit() {
            let al = a.iter().take_while(|c| c.is_ascii_digit()).count();
            let bl = b.iter().take_while(|c| c.is_ascii_digit()).count();
            let az = a[..al].iter().take_while(|c| **c == b'0').count();
            let bz = b[..bl].iter().take_while(|c| **c == b'0').count();
            let cmp = (al - az)
                .cmp(&(bl - bz))
                .then_with(|| a[az..al].cmp(&b[bz..bl]));
            if cmp != Ordering::Equal {
                return cmp;
            }
            a = &a[al..];
            b = &b[bl..];
        } else {
            if ac != bc {
                return ac.cmp(&bc);
            }
            a = &a[1..];
            b = &b[1..];
        }
    }
    a.len().cmp(&b.len())
}
fn main() -> Result<()> {
    let mut session = None;
    starfold_preview_protocol::serve(
        "comic",
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
                    .ok_or_else(|| anyhow::anyhow!("No comic session"))?
                    .input(input),
                _ => anyhow::bail!("Unsupported request"),
            }
            session.as_mut().unwrap().presentation()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn limits() -> Limits {
        Limits {
            text_bytes: 4096,
            cache_bytes: 8_000_000,
            image_dimension: 1024,
        }
    }
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("book.CBZ");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, color) in [
            ("chapter/page10.png", 100),
            ("chapter/page2.png", 20),
            ("chapter/page1.png", 10),
            ("__MACOSX/cover.png", 255),
            (".hidden.png", 255),
        ] {
            let image = RgbaImage::from_pixel(20, 40, image::Rgba([color, 0, 0, 255]));
            let mut encoded = io::Cursor::new(Vec::new());
            image
                .write_to(&mut encoded, image::ImageFormat::Png)
                .unwrap();
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(encoded.get_ref()).unwrap();
        }
        zip.start_file("ComicInfo.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"<ComicInfo/>").unwrap();
        zip.finish().unwrap();
        (temp, path)
    }
    #[test]
    fn cbz_natural_pages_navigation_zoom_and_raster_limits() {
        use std::os::unix::ffi::OsStrExt;
        let (_temp, path) = fixture();
        let mut session = Session::open(
            path.as_os_str().as_bytes().to_vec(),
            limits(),
            Viewport::default(),
        )
        .unwrap();
        assert_eq!(session.pages.len(), 3);
        for (page, red) in [(1, 10), (2, 20), (3, 100)] {
            session.input(Input::Page { page });
            let (p, pixels) = session.presentation().unwrap();
            p.validate(&pixels).unwrap();
            assert_eq!(p.total_pages, Some(3));
            assert_eq!(p.raster.as_ref().unwrap().page, page);
            assert_eq!(session.decoded.as_ref().unwrap().1[(0, 0)][0], red);
        }
        session.input(Input::Key { key: "n".into() });
        assert_eq!(session.page, 3);
        session.input(Input::Key { key: "home".into() });
        session.input(Input::Key { key: "p".into() });
        assert_eq!(session.page, 1);
        session.input(Input::Key { key: "+".into() });
        session.input(Input::Key { key: "w".into() });
        session.input(Input::Key { key: "j".into() });
        let (p, pixels) = session.presentation().unwrap();
        p.validate(&pixels).unwrap();
        assert!(session.scroll > 0.0);
        session.input(Input::Key { key: "0".into() });
        assert!(!session.fit_width);
        assert_eq!(session.scroll, 0.0);
        session.limits.image_dimension = 1;
        let (p, bytes) = session.presentation().unwrap();
        p.validate(&bytes).unwrap();
        assert_eq!(bytes.len(), 4);
        assert_eq!(natural_cmp("page0002", "page10"), Ordering::Less);
    }
    #[test]
    fn empty_or_damaged_comics_have_clear_errors() {
        use std::os::unix::ffi::OsStrExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("empty.cbz");
        zip::ZipWriter::new(std::fs::File::create(&path).unwrap())
            .finish()
            .unwrap();
        assert!(Session::open(
            path.as_os_str().as_bytes().to_vec(),
            limits(),
            Viewport::default()
        )
        .err()
        .unwrap()
        .to_string()
        .contains("no supported image pages"));
        std::fs::write(&path, b"broken").unwrap();
        assert!(Session::open(
            path.as_os_str().as_bytes().to_vec(),
            limits(),
            Viewport::default()
        )
        .is_err());
    }
    #[test]
    fn real_comic_archive_when_requested() {
        use std::os::unix::ffi::OsStrExt;
        let Some(path) = std::env::var_os("STARFOLD_COMIC_TEST_FILE") else {
            return;
        };
        let mut session =
            Session::open(path.as_bytes().to_vec(), limits(), Viewport::default()).unwrap();
        for page in [1, 2, session.pages.len() as u32] {
            session.input(Input::Page { page });
            let (presentation, bytes) = session.presentation().unwrap();
            presentation.validate(&bytes).unwrap();
            assert_eq!(
                presentation.raster.unwrap().page,
                page.min(session.pages.len() as u32)
            );
        }
    }
}
