//! Showing what a file is, before it is opened.
//!
//! [`Preview`] is a value, not a widget: it is built on `starfold-io` from a
//! path and a cancel flag, and the panel that draws it never touches a
//! filesystem itself. `starkit::image::RgbaImage` is allowed here even though
//! nothing under `src/fold/` may draw -- it is a decoded picture, not a
//! terminal crate, the same distinction that lets `jiff` and `serde` live
//! here too.

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::summary::{self, Budget, DirSummary};

/// What the preview panel has to show for a path.
#[derive(Debug, Clone)]
pub enum Preview {
    /// Nothing selected, or nothing built yet.
    Empty,
    Dir(DirSummary),
    Text {
        head: String,
        truncated: bool,
        /// The file's full length, even when `head` is only a prefix of it --
        /// the panel wants to say "1.2 KB" beside a preview that only shows
        /// the first few lines.
        bytes: u64,
        /// How many lines `head` actually holds, so the panel can say
        /// "showing 5 of many" without counting newlines itself.
        lines: usize,
    },
    Image {
        data: Arc<starkit::image::RgbaImage>,
        width: u32,
        height: u32,
        /// `"png"`, `"jpeg"`, and so on -- a label for the status line, not
        /// something re-derived from the extension every frame.
        format: &'static str,
    },
    /// A hexdump, sixteen bytes to a row, for anything that is not text and
    /// not a picture.
    Binary {
        rows: Vec<String>,
        truncated: bool,
        /// Guessed from the extension alone -- the only evidence there is --
        /// and `application/octet-stream` when nothing matches.
        mime: String,
    },
    Symlink {
        target: PathBuf,
        broken: bool,
    },
    Error(String),
}

/// Limits `build` is kept inside, all from `[preview]` in `config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewConfig {
    pub max_bytes: u64,
    pub max_lines: usize,
    pub max_image_dimension: u32,
    pub dir_budget: usize,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            max_bytes: 262_144,
            max_lines: 400,
            max_image_dimension: 4096,
            dir_budget: 20_000,
        }
    }
}

/// A file above this size is not even opened for an image preview.
///
/// The dimension check inside `decode_limited` is what refuses a hostile
/// *header*; this refuses spending the time and memory to `read` a merely
/// enormous, honestly-encoded one before that check ever runs.
const MAX_IMAGE_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Build a preview for `path`, checking `cancel` as it goes.
///
/// Never panics on hostile input -- everything short of a symlink or a
/// directory ends up read as bytes, and bytes from a file this program does
/// not control the origin of are handled the way `starcord`'s media decoder
/// handles bytes from a stranger's computer: sizes are bounded before
/// anything is allocated, and a decode failure is a [`Preview::Error`], not a
/// panic.
pub fn build(path: &Path, cfg: &PreviewConfig, cancel: &AtomicBool) -> Preview {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => return Preview::Error(e.to_string()),
    };

    if meta.file_type().is_symlink() {
        return symlink_preview(path);
    }

    if meta.is_dir() {
        let budget = Budget {
            max_entries: cfg.dir_budget,
            max_depth: 64,
        };
        return Preview::Dir(summary::summarize(path, &budget, cancel));
    }

    if !meta.is_file() {
        // A device node, a socket, a FIFO: `stat` can name it, but opening
        // one for a read can hang (a FIFO with nobody writing) or makes no
        // sense (a socket). Nothing here is worth a preview.
        return Preview::Error("not a regular file".to_string());
    }

    build_file(path, meta.len(), cfg, cancel)
}

fn symlink_preview(path: &Path) -> Preview {
    let target = match std::fs::read_link(path) {
        Ok(t) => t,
        Err(e) => return Preview::Error(e.to_string()),
    };
    // `exists` follows the link; a link whose target cannot be stat-ed --
    // gone, or a permission wall -- reports `false`, which is what "broken"
    // means here.
    let broken = !path.exists();
    Preview::Symlink { target, broken }
}

fn build_file(path: &Path, full_len: u64, cfg: &PreviewConfig, cancel: &AtomicBool) -> Preview {
    if full_len == 0 {
        return Preview::Empty;
    }
    if cancel.load(Ordering::Relaxed) {
        return Preview::Empty;
    }

    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => return Preview::Error(e.to_string()),
    };

    let mut head = Vec::new();
    if let Err(e) = file.take(cfg.max_bytes).read_to_end(&mut head) {
        return Preview::Error(e.to_string());
    }
    let bytes_capped = full_len > cfg.max_bytes;

    if cancel.load(Ordering::Relaxed) {
        return Preview::Empty;
    }

    if looks_like_image(path, &head) {
        return build_image(path, full_len, cfg);
    }

    if is_binary(&head) {
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .essence_str()
            .to_string();
        let (rows, rows_truncated) = hexdump(&head, cfg.max_lines);
        return Preview::Binary {
            rows,
            truncated: rows_truncated || bytes_capped,
            mime,
        };
    }

    text_preview(&head, bytes_capped, full_len, cfg.max_lines)
}

fn looks_like_image(path: &Path, head: &[u8]) -> bool {
    starkit::image::ImageFormat::from_path(path).is_ok()
        || starkit::image::guess_format(head).is_ok()
}

fn build_image(path: &Path, full_len: u64, cfg: &PreviewConfig) -> Preview {
    if full_len > MAX_IMAGE_FILE_BYTES {
        return Preview::Error(format!("image is {full_len} bytes, too large to preview"));
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return Preview::Error(e.to_string()),
    };
    match starkit::graphics::decode_limited(&bytes, cfg.max_image_dimension) {
        Ok(image) => {
            let format = starkit::image::ImageFormat::from_path(path)
                .ok()
                .or_else(|| starkit::image::guess_format(&bytes).ok())
                .map(format_name)
                .unwrap_or("image");
            let rgba = image.to_rgba8();
            let (width, height) = rgba.dimensions();
            Preview::Image {
                data: Arc::new(rgba),
                width,
                height,
                format,
            }
        }
        Err(e) => Preview::Error(format!("could not decode the image: {e}")),
    }
}

fn format_name(fmt: starkit::image::ImageFormat) -> &'static str {
    use starkit::image::ImageFormat;
    match fmt {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::Gif => "gif",
        ImageFormat::WebP => "webp",
        ImageFormat::Bmp => "bmp",
        _ => "image",
    }
}

/// A head is binary when it either contains a NUL byte -- the one thing a
/// text file never legitimately does -- or is not valid UTF-8 once a
/// trailing sequence that `max_bytes` may have cut mid-character is
/// forgiven. A genuinely invalid byte *earlier* in the head is not forgiven:
/// that is not a cut character, it is not text.
fn is_binary(head: &[u8]) -> bool {
    head.contains(&0) || utf8_prefix(head).is_none()
}

/// The longest valid-UTF-8 prefix of `head`, forgiving only an incomplete
/// multi-byte sequence at the very end -- what `max_bytes` cutting a file
/// mid-character looks like. Any other invalid byte returns `None`.
fn utf8_prefix(head: &[u8]) -> Option<&str> {
    match std::str::from_utf8(head) {
        Ok(s) => Some(s),
        Err(e) if e.error_len().is_none() => {
            // `error_len` is `None` exactly when the error is "ran out of
            // bytes", i.e. an incomplete sequence at the end.
            std::str::from_utf8(&head[..e.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
}

fn text_preview(head: &[u8], bytes_capped: bool, full_len: u64, max_lines: usize) -> Preview {
    let text = utf8_prefix(head).unwrap_or_default();
    let mut truncated = bytes_capped || text.len() < head.len();
    let mut out = String::new();
    let mut lines = 0usize;
    for line in text.split_inclusive('\n') {
        if lines >= max_lines {
            truncated = true;
            break;
        }
        out.push_str(line);
        lines += 1;
    }
    Preview::Text {
        head: out,
        truncated,
        bytes: full_len,
        lines,
    }
}

/// Sixteen bytes to a row: `00000000  48 65 6c 6c 6f 20 77 6f  72 6c 64 21 0a
/// 00 01 02  |Hello world!....|`. Capped at `max_rows`; the second return
/// value says whether more rows existed.
fn hexdump(bytes: &[u8], max_rows: usize) -> (Vec<String>, bool) {
    use std::fmt::Write as _;

    let mut rows = Vec::new();
    let mut truncated = false;
    for (i, chunk) in bytes.chunks(16).enumerate() {
        if i >= max_rows {
            truncated = true;
            break;
        }
        let offset = i * 16;
        let mut hex = String::with_capacity(16 * 3 + 1);
        for (j, b) in chunk.iter().enumerate() {
            if j == 8 {
                hex.push(' ');
            }
            let _ = write!(hex, "{b:02x} ");
        }
        while hex.len() < 16 * 3 + 1 {
            hex.push(' ');
        }
        let ascii: String = chunk
            .iter()
            .map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        rows.push(format!("{offset:08x}  {hex} |{ascii}|"));
    }
    (rows, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::testing::Fixture;
    use proptest::prelude::*;

    #[test]
    fn the_default_config_matches_the_documented_limits() {
        let c = PreviewConfig::default();
        assert_eq!(c.max_bytes, 262_144);
        assert_eq!(c.max_lines, 400);
        assert_eq!(c.max_image_dimension, 4096);
        assert_eq!(c.dir_budget, 20_000);
    }

    #[test]
    fn a_readme_previews_as_text_with_all_its_lines() {
        let f = Fixture::tree();
        let got = build(
            &f.path("projects/starwire/README.md"),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        match got {
            Preview::Text {
                lines, truncated, ..
            } => {
                assert_eq!(lines, 31);
                assert!(!truncated);
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_tight_line_cap_truncates_the_readme() {
        let f = Fixture::tree();
        let cfg = PreviewConfig {
            max_lines: 5,
            ..PreviewConfig::default()
        };
        let got = build(
            &f.path("projects/starwire/README.md"),
            &cfg,
            &AtomicBool::new(false),
        );
        match got {
            Preview::Text {
                lines, truncated, ..
            } => {
                assert_eq!(lines, 5);
                assert!(truncated);
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_tight_byte_cap_truncates_the_readme() {
        let f = Fixture::tree();
        let cfg = PreviewConfig {
            max_bytes: 20,
            ..PreviewConfig::default()
        };
        let got = build(
            &f.path("projects/starwire/README.md"),
            &cfg,
            &AtomicBool::new(false),
        );
        match got {
            Preview::Text { truncated, .. } => assert!(truncated),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_byte_cap_that_lands_mid_character_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accents.txt");
        // "é" is two bytes in UTF-8; 21 bytes cuts the eleventh one in half.
        std::fs::write(&path, "é".repeat(20)).unwrap();
        let cfg = PreviewConfig {
            max_bytes: 21,
            ..PreviewConfig::default()
        };
        let got = build(&path, &cfg, &AtomicBool::new(false));
        match got {
            Preview::Text { truncated, .. } => assert!(truncated),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_nul_blob_previews_as_a_hexdump() {
        let f = Fixture::tree();
        let got = build(
            &f.path("blob.bin"),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        match got {
            Preview::Binary { rows, mime, .. } => {
                assert!(rows[0].starts_with("00000000"));
                assert_eq!(mime, "application/octet-stream");
                // 256 bytes is exactly sixteen full rows; every one of them
                // should show sixteen byte pairs between the offset and the
                // ascii column, whatever the exact spacing is.
                assert_eq!(rows.len(), 16);
                for row in &rows {
                    let hex_part = &row[8..row.find('|').unwrap()];
                    assert_eq!(hex_part.split_whitespace().count(), 16);
                }
            }
            other => panic!("expected binary, got {other:?}"),
        }
    }

    #[test]
    fn a_png_decodes_to_its_pixel_dimensions() {
        let f = Fixture::tree();
        let got = build(
            &f.path("pictures/harbour.png"),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        match got {
            Preview::Image {
                width,
                height,
                format,
                ..
            } => {
                assert_eq!((width, height), (64, 48));
                assert_eq!(format, "png");
            }
            other => panic!("expected an image, got {other:?}"),
        }
    }

    #[test]
    fn a_png_header_claiming_absurd_dimensions_is_refused_without_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bomb.png");
        // The 8-byte PNG signature, a 4-byte chunk length, the "IHDR" tag,
        // and a 4+4 byte width/height claiming 40000x40000 -- 24 bytes, none
        // of the rest of the chunk (bit depth, colour type, CRC) present.
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&40_000u32.to_be_bytes());
        bytes.extend_from_slice(&40_000u32.to_be_bytes());
        assert_eq!(bytes.len(), 24);
        std::fs::write(&path, &bytes).unwrap();

        let got = build(&path, &PreviewConfig::default(), &AtomicBool::new(false));
        assert!(
            matches!(got, Preview::Error(_)),
            "expected an error, got {got:?}"
        );
    }

    #[test]
    fn a_directory_previews_as_a_summary() {
        let f = Fixture::tree();
        let got = build(
            &f.path("projects/starwire"),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        match got {
            Preview::Dir(s) => {
                assert_eq!(s.dirs, 2);
                assert_eq!(s.files, 4);
            }
            other => panic!("expected a directory summary, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_directory_previews_to_zeros() {
        let f = Fixture::tree();
        let got = build(
            &f.path("empty"),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        assert!(matches!(got, Preview::Dir(s) if s == DirSummary::default()));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_file_previews_as_not_broken() {
        let f = Fixture::tree();
        let got = build(
            &f.path("notes.txt"),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        match got {
            Preview::Symlink { broken, .. } => assert!(!broken),
            other => panic!("expected a symlink, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_previews_as_broken() {
        let f = Fixture::tree();
        let got = build(
            &f.path("dangling"),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        match got {
            Preview::Symlink { broken, .. } => assert!(broken),
            other => panic!("expected a symlink, got {other:?}"),
        }
    }

    #[test]
    fn a_zero_length_file_previews_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nothing");
        std::fs::write(&path, b"").unwrap();
        let got = build(&path, &PreviewConfig::default(), &AtomicBool::new(false));
        assert!(matches!(got, Preview::Empty));
    }

    #[test]
    fn a_missing_path_previews_as_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let got = build(
            &dir.path().join("nowhere"),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        assert!(matches!(got, Preview::Error(_)));
    }

    #[test]
    fn a_cancelled_build_returns_empty_even_for_a_big_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        std::fs::write(&path, "x".repeat(1_000_000)).unwrap();
        let got = build(&path, &PreviewConfig::default(), &AtomicBool::new(true));
        assert!(matches!(got, Preview::Empty));
    }

    proptest! {
        #[test]
        fn build_never_panics_over_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..=4096)) {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("arbitrary");
            std::fs::write(&path, &bytes).unwrap();
            let _ = build(&path, &PreviewConfig::default(), &AtomicBool::new(false));
        }
    }
}
