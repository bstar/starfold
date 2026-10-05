//! Bounded raster decoding, independent of preview scheduling and rendering.
use super::{Preview, PreviewConfig};
use starkit::image::{DynamicImage, ImageFormat, ImageReader, Limits, RgbaImage};
use std::fs;
use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Ordinary camera photos decode in-process. Larger rasters use libvips'
/// shrink-on-load path, so STAR/FOLD never needs their full-size RGBA buffer.
const MAX_IN_PROCESS_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_IN_PROCESS_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SOURCE_PIXELS: u64 = 512 * 1024 * 1024;
const MAX_PREVIEW_DIMENSION: u32 = 4096;
const MAX_THUMBNAIL_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_HELPER_SOURCE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

pub(super) fn looks_like_image(path: &Path, head: &[u8]) -> bool {
    ImageFormat::from_path(path).is_ok() || starkit::image::guess_format(head).is_ok()
}

pub(super) fn build_image(
    path: &Path,
    full_len: u64,
    cfg: &PreviewConfig,
    cancel: &std::sync::atomic::AtomicBool,
) -> Preview {
    if ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .ok()
        .and_then(|r| r.format())
        == Some(ImageFormat::Gif)
        && full_len <= MAX_IN_PROCESS_FILE_BYTES
    {
        let decoded = decode_animation(path, cfg, cancel);
        if let Err(error) = &decoded {
            tracing::warn!(path = %path.display(), %error, "GIF animation unavailable; using still preview");
        }
        if let Ok(sequence) = decoded {
            if sequence.truncated {
                tracing::warn!(path = %path.display(), frames = sequence.len(), "GIF preview truncated at memory/frame limit");
            }
            if sequence.len() > 1 {
                return Preview::Animation(Arc::new(sequence));
            }
        }
    }
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Preview::Empty;
    }
    match decode_preview(path, full_len, cfg) {
        Ok((rgba, format)) => {
            let (width, height) = rgba.dimensions();
            Preview::Image {
                data: Arc::new(rgba),
                width,
                height,
                format,
            }
        }
        Err(e) => Preview::Error(format!("could not preview the image: {e}")),
    }
}

fn decode_animation(
    path: &Path,
    cfg: &PreviewConfig,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<starkit::animation::Animation, String> {
    // Bound both encoded input and accumulated composited frames.
    let deadline = Instant::now() + Duration::from_millis(cfg.timeout_ms.max(15_000));
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_IN_PROCESS_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_IN_PROCESS_FILE_BYTES {
        return Err("GIF input exceeds limit".into());
    }
    starkit::animation::Animation::gif(&bytes, 64_000_000, || {
        cancel.load(std::sync::atomic::Ordering::Relaxed) || Instant::now() >= deadline
    })
    .map_err(|e| e.to_string())
}

fn decode_preview(
    path: &Path,
    full_len: u64,
    cfg: &PreviewConfig,
) -> Result<(RgbaImage, &'static str), String> {
    let target = cfg.max_image_dimension.clamp(1, MAX_PREVIEW_DIMENSION);
    // Some decoders read an entire JPEG merely to discover its dimensions.
    // Send a large encoded file straight to the isolated thumbnail helper.
    if full_len > MAX_IN_PROCESS_FILE_BYTES {
        if full_len > MAX_HELPER_SOURCE_BYTES {
            return Err("encoded image exceeds the preview safety limit".into());
        }
        return thumbnail_with_vips(path, target, cfg.timeout_ms)
            .map(|image| (image, source_format(path)));
    }
    let reader = ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|e| e.to_string())?;
    let format = reader.format().map(format_name).unwrap_or("image");
    let (width, height) = reader.into_dimensions().map_err(|e| e.to_string())?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_SOURCE_PIXELS {
        return Err(format!(
            "{width}×{height} pixels exceeds the preview safety limit"
        ));
    }
    let large = width > target || height > target;
    let image = if pixels <= MAX_IN_PROCESS_PIXELS {
        match decode_in_process(path) {
            Ok(image) if large => image.thumbnail(target, target).to_rgba8(),
            Ok(image) => image.to_rgba8(),
            Err(e) if large => thumbnail_with_vips(path, target, cfg.timeout_ms)
                .map_err(|fallback| format!("{e}; thumbnail fallback: {fallback}"))?,
            Err(e) => return Err(e),
        }
    } else {
        thumbnail_with_vips(path, target, cfg.timeout_ms)?
    };
    Ok((image, format))
}

fn source_format(path: &Path) -> &'static str {
    ImageFormat::from_path(path)
        .ok()
        .or_else(|| {
            let mut head = [0u8; 32];
            let n = fs::File::open(path).ok()?.read(&mut head).ok()?;
            starkit::image::guess_format(&head[..n]).ok()
        })
        .map(format_name)
        .unwrap_or("image")
}

fn decode_in_process(path: &Path) -> Result<DynamicImage, String> {
    let mut reader = ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|e| e.to_string())?;
    let mut limits = Limits::default();
    limits.max_image_width = None;
    limits.max_image_height = None;
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|e| e.to_string())
}

fn thumbnail_with_vips(path: &Path, target: u32, timeout_ms: u64) -> Result<RgbaImage, String> {
    let output = tempfile::Builder::new()
        .suffix(".png")
        .tempfile()
        .map_err(|e| e.to_string())?;
    let mut child = Command::new("vipsthumbnail")
        .arg("--size")
        .arg(format!("{target}x{target}"))
        .arg("--path")
        .arg(output.path())
        .arg("--")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("vipsthumbnail is unavailable: {e}"))?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(15_000));
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => return Err(format!("vipsthumbnail exited with {status}")),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("thumbnail generation timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    }
    let length = fs::metadata(output.path())
        .map_err(|e| e.to_string())?
        .len();
    if length > MAX_THUMBNAIL_FILE_BYTES {
        return Err("thumbnail output is too large".into());
    }
    starkit::graphics::open_limited(output.path(), target)
        .map(|image| image.to_rgba8())
        .map_err(|e| e.to_string())
}

fn format_name(fmt: ImageFormat) -> &'static str {
    match fmt {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::Gif => "gif",
        ImageFormat::WebP => "webp",
        ImageFormat::Bmp => "bmp",
        _ => "image",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_gif(path: &Path, frames: usize) {
        let mut encoder =
            starkit::image::codecs::gif::GifEncoder::new(fs::File::create(path).unwrap());
        encoder
            .set_repeat(starkit::image::codecs::gif::Repeat::Infinite)
            .unwrap();
        for index in 0..frames {
            encoder
                .encode_frame(starkit::image::Frame::from_parts(
                    RgbaImage::from_pixel(
                        4,
                        4,
                        starkit::image::Rgba([index as u8 * 200, 30, 40, 255]),
                    ),
                    0,
                    0,
                    starkit::image::Delay::from_numer_denom_ms(100, 1),
                ))
                .unwrap();
        }
    }

    #[test]
    fn gif_preview_animates_by_signature_and_static_gifs_stay_images() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("animation.png");
        write_gif(&path, 2);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let result = build_image(
            &path,
            fs::metadata(&path).unwrap().len(),
            &PreviewConfig::default(),
            &cancel,
        );
        let Preview::Animation(sequence) = result else {
            panic!("{result:?}")
        };
        assert_eq!(sequence.len(), 2);
        assert_ne!(sequence.frame(0).unwrap(), sequence.frame(1).unwrap());
        write_gif(&path, 1);
        assert!(matches!(
            build_image(
                &path,
                fs::metadata(&path).unwrap().len(),
                &PreviewConfig::default(),
                &cancel
            ),
            Preview::Image { format: "gif", .. }
        ));
    }

    #[test]
    fn damaged_gif_returns_an_error_and_cancelled_preview_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.gif");
        fs::write(&path, b"GIF89a").unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert!(matches!(
            build_image(&path, 6, &PreviewConfig::default(), &cancel),
            Preview::Error(_)
        ));
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(
            build_image(&path, 6, &PreviewConfig::default(), &cancel),
            Preview::Empty
        ));
    }

    #[test]
    fn libvips_thumbnail_stays_within_the_preview_limit_when_available() {
        match Command::new("vipsthumbnail").arg("--version").output() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => panic!("could not check vipsthumbnail: {e}"),
            Ok(output) => assert!(output.status.success()),
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/starfold.png");
        let image = thumbnail_with_vips(&path, 32, 1000).unwrap();
        assert_eq!(image.dimensions(), (32, 32));
        let cfg = PreviewConfig {
            max_image_dimension: 32,
            ..PreviewConfig::default()
        };
        let (image, format) = decode_preview(&path, MAX_IN_PROCESS_FILE_BYTES + 1, &cfg).unwrap();
        assert_eq!(image.dimensions(), (32, 32));
        assert_eq!(format, "png");
    }
}
