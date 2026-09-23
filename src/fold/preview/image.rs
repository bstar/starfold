//! Bounded raster decoding, independent of preview scheduling and rendering.
use super::{Preview, PreviewConfig};
use std::{path::Path, sync::Arc};
/// A file above this size is not even opened for an image preview.
///
/// The dimension check inside `decode_limited` is what refuses a hostile
/// *header*; this refuses spending the time and memory to `read` a merely
/// enormous, honestly-encoded one before that check ever runs.
const MAX_IMAGE_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub(super) fn looks_like_image(path: &Path, head: &[u8]) -> bool {
    starkit::image::ImageFormat::from_path(path).is_ok()
        || starkit::image::guess_format(head).is_ok()
}

pub(super) fn build_image(path: &Path, full_len: u64, cfg: &PreviewConfig) -> Preview {
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
