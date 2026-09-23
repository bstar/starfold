//! Showing what a file is, before it is opened.
//!
//! [`Preview`] is a value, not a widget: it is built on `starfold-io` from a
//! path and a cancel flag, and the panel that draws it never touches a
//! filesystem itself. `starkit::image::RgbaImage` is allowed here even though
//! nothing under `src/fold/` may draw -- it is a decoded picture, not a
//! terminal crate, the same distinction that lets `jiff` and `serde` live
//! here too.

pub mod connection;
pub mod directory;
mod image;
mod text;
use image::{build_image, looks_like_image};
use text::{is_binary, text_preview};
pub mod model;
pub mod providers;

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// What the preview panel has to show for a path.
#[derive(Debug, Clone)]
pub enum Preview {
    /// Nothing selected, or nothing built yet.
    Empty,
    Document(model::Document),
    Dir(directory::Tree),
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
    Symlink {
        target: PathBuf,
        broken: bool,
    },
    Error(String),
}

/// Limits `build` is kept inside, all from `[preview]` in `config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PreviewConfig {
    pub timeout_ms: u64,
    pub cache_bytes: usize,
    pub pdf_page_bytes: usize,
    pub max_bytes: u64,
    pub max_lines: usize,
    pub max_image_dimension: u32,
    pub dir_budget: usize,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 2000,
            cache_bytes: 32 * 1024 * 1024,
            pdf_page_bytes: 262_144,
            max_bytes: 262_144,
            max_lines: 400,
            max_image_dimension: 4096,
            dir_budget: 20_000,
        }
    }
}

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
        return Preview::Dir(directory::build(path, cfg, &|| {
            cancel.load(Ordering::Relaxed)
        }));
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

    if let Some(result) = providers::build(path, &head, 1, cfg) {
        return Preview::Document(result);
    }

    if looks_like_image(path, &head) {
        return build_image(path, full_len, cfg);
    }

    if is_binary(&head) {
        return Preview::Document(providers::metadata(path));
    }

    text_preview(&head, bytes_capped, full_len, cfg.max_lines)
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
    fn a_nul_blob_previews_as_metadata() {
        let f = Fixture::tree();
        let got = build(
            &f.path("blob.bin"),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        let Preview::Document(d) = got else {
            panic!("expected metadata")
        };
        assert!(d
            .fields
            .iter()
            .any(|f| f.label == "Type" && f.value == "application/octet-stream"));
        assert!(d.fields.iter().any(|f| f.label == "Size"));
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
    fn a_directory_previews_as_a_tree() {
        let f = Fixture::tree();
        let got = build(
            &f.path("projects/starwire"),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        match got {
            Preview::Dir(s) => {
                assert_eq!(s.summary.dirs, 2);
                assert_eq!(s.summary.files, 4);
            }
            other => panic!("expected a directory tree, got {other:?}"),
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
        assert!(
            matches!(got, Preview::Dir(s) if s.entries.is_empty() && s.notice.is_none() && s.summary == super::super::summary::DirSummary::default())
        );
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
