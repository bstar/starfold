//! Showing what a file is, before it is opened.
//!
//! [`Preview`] is a value, not a widget: it is built on `starfold-io` from a
//! path and a cancel flag, and the panel that draws it never touches a
//! filesystem itself. `starkit::image::RgbaImage` is allowed here even though
//! nothing under `src/fold/` may draw -- it is a decoded picture, not a
//! terminal crate, the same distinction that lets `jiff` and `serde` live
//! here too.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::summary::DirSummary;

/// What the preview panel has to show for a path.
#[derive(Debug, Clone)]
pub enum Preview {
    /// Nothing selected, or nothing built yet.
    Empty,
    Dir(DirSummary),
    Text {
        head: String,
        truncated: bool,
    },
    Image {
        data: Arc<starkit::image::RgbaImage>,
    },
    /// A hexdump, sixteen bytes to a row, for anything that is not text and
    /// not a picture.
    Binary {
        rows: Vec<String>,
        truncated: bool,
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

/// Build a preview for `path`, checking `cancel` as it goes.
///
/// `// TODO(1d)`: this is the bootstrap stub. It always returns
/// [`Preview::Empty`]; the real dispatch on file kind and extension -- text
/// head, image decode, directory summary, hexdump, symlink resolution -- is
/// Phase 1d's.
pub fn build(path: &std::path::Path, cfg: &PreviewConfig, cancel: &AtomicBool) -> Preview {
    let _ = (path, cfg, cancel);
    Preview::Empty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_config_matches_the_documented_limits() {
        let c = PreviewConfig::default();
        assert_eq!(c.max_bytes, 262_144);
        assert_eq!(c.max_lines, 400);
        assert_eq!(c.max_image_dimension, 4096);
        assert_eq!(c.dir_budget, 20_000);
    }

    #[test]
    fn the_stub_builds_nothing_yet() {
        let dir = tempfile::tempdir().unwrap();
        let got = build(
            dir.path(),
            &PreviewConfig::default(),
            &AtomicBool::new(false),
        );
        assert!(matches!(got, Preview::Empty));
    }
}
