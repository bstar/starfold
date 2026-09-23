//! Terminal-native raster overlays for the embedded player's transport.
//!
//! The child sends pixels, never terminal escapes. The host owns encoding,
//! placement and release through STAR/KIT's shared graphics cache.

use std::sync::Arc;

use starkit::graphics::{Graphics, ImageId};
use starkit::image::RgbaImage;
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::widgets::Widget as _;
use starkit::ratatui_image::Image;

use crate::audio_embed::TransportImage;

const MAX_IMAGES: usize = 5;

struct Cached {
    id: ImageId,
    pixels: Arc<RgbaImage>,
}

#[derive(Default)]
pub struct AudioGraphics {
    images: Vec<Cached>,
}

impl AudioGraphics {
    /// Draw native transport faces on top of the already-painted text body.
    /// Cell coordinates are relative to `body`; a malformed or clipped face
    /// is skipped rather than letting an image spill into another panel.
    pub fn draw(
        &mut self,
        images: &[TransportImage],
        body: Rect,
        graphics: &mut Graphics,
        buf: &mut Buffer,
    ) {
        if !graphics.pictures_available() {
            self.clear(graphics);
            return;
        }
        self.sync(images, graphics);
        for (image, cached) in images.iter().take(MAX_IMAGES).zip(&self.images) {
            let Some(area) = placement(body, image) else {
                continue;
            };
            if let Some(protocol) = graphics.raster(cached.id, area, |w, h| {
                pixels_for_area(cached.pixels.as_ref(), w, h)
            }) {
                Image::new(protocol).render(area, buf);
            }
        }
    }

    /// Release the host cache and any terminal-side uploaded images.
    pub fn clear(&mut self, graphics: &mut Graphics) {
        for image in self.images.drain(..) {
            graphics.forget(image.id);
        }
    }

    fn sync(&mut self, images: &[TransportImage], graphics: &mut Graphics) {
        let retained = images.len().min(MAX_IMAGES);
        while self.images.len() > retained {
            if let Some(old) = self.images.pop() {
                if !self.images.iter().any(|image| image.id == old.id) {
                    graphics.forget(old.id);
                }
            }
        }
        for (slot, image) in images.iter().take(MAX_IMAGES).enumerate() {
            let same = self.images.get(slot).is_some_and(|cached| {
                cached.pixels.width() == u32::from(image.pixel_width)
                    && cached.pixels.height() == u32::from(image.pixel_height)
                    && cached.pixels.as_raw() == &image.rgba
            });
            if same {
                continue;
            }
            let Some(pixels) = RgbaImage::from_raw(
                u32::from(image.pixel_width),
                u32::from(image.pixel_height),
                image.rgba.clone(),
            ) else {
                // Client validation should have rejected this already.
                continue;
            };
            let entry = Cached {
                id: ImageId::of(&(image.pixel_width, image.pixel_height, &image.rgba)),
                pixels: Arc::new(pixels),
            };
            if slot < self.images.len() {
                let old = std::mem::replace(&mut self.images[slot], entry);
                if !self.images.iter().any(|image| image.id == old.id) {
                    graphics.forget(old.id);
                }
            } else {
                self.images.push(entry);
            }
        }
    }
}

fn pixels_for_area(source: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    if source.width() == width && source.height() == height {
        source.clone()
    } else {
        starkit::image::imageops::resize(
            source,
            width,
            height,
            starkit::image::imageops::FilterType::Triangle,
        )
    }
}

fn placement(body: Rect, image: &TransportImage) -> Option<Rect> {
    if image.width == 0 || image.height == 0 {
        return None;
    }
    let right = image.x.checked_add(image.width)?;
    let bottom = image.y.checked_add(image.height)?;
    if right > body.width || bottom > body.height {
        return None;
    }
    Some(Rect::new(
        body.x.checked_add(image.x)?,
        body.y.checked_add(image.y)?,
        image.width,
        image.height,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(value: u8) -> TransportImage {
        TransportImage {
            x: 2,
            y: 1,
            width: 3,
            height: 2,
            pixel_width: 6,
            pixel_height: 4,
            rgba: vec![value; 6 * 4 * 4],
        }
    }

    #[test]
    fn unchanged_pixels_keep_the_same_allocation_and_id() {
        let mut cache = AudioGraphics::default();
        let mut graphics = Graphics::disabled();
        cache.sync(&[face(7)], &mut graphics);
        let first = Arc::clone(&cache.images[0].pixels);
        let id = cache.images[0].id;
        cache.sync(&[face(7)], &mut graphics);
        assert!(Arc::ptr_eq(&first, &cache.images[0].pixels));
        assert_eq!(cache.images[0].id, id);
        cache.sync(&[face(8)], &mut graphics);
        assert!(!Arc::ptr_eq(&first, &cache.images[0].pixels));
        assert_ne!(cache.images[0].id, id);
    }

    #[test]
    fn cache_is_bounded_and_clear_releases_all_faces() {
        let mut cache = AudioGraphics::default();
        let mut graphics = Graphics::disabled();
        cache.sync(&vec![face(1); 8], &mut graphics);
        assert_eq!(cache.images.len(), MAX_IMAGES);
        cache.clear(&mut graphics);
        assert!(cache.images.is_empty());
    }

    #[test]
    fn terminal_without_pictures_discards_cached_faces() {
        let mut cache = AudioGraphics::default();
        let mut graphics = Graphics::disabled();
        cache.sync(&[face(1)], &mut graphics);
        let body = Rect::new(0, 0, 15, 8);
        let mut buffer = Buffer::empty(body);
        cache.draw(&[face(1)], body, &mut graphics, &mut buffer);
        assert!(cache.images.is_empty());
    }

    #[test]
    fn placement_is_relative_and_cannot_leave_body() {
        let body = Rect::new(10, 20, 15, 8);
        assert_eq!(placement(body, &face(1)), Some(Rect::new(12, 21, 3, 2)));
        let mut outside = face(1);
        outside.x = 13;
        assert_eq!(placement(body, &outside), None);
        outside.x = u16::MAX;
        assert_eq!(placement(body, &outside), None);
    }

    #[test]
    fn raster_is_sized_to_current_cell_measurement() {
        let source = RgbaImage::from_raw(6, 4, vec![255; 6 * 4 * 4]).unwrap();
        assert_eq!(pixels_for_area(&source, 6, 4).dimensions(), (6, 4));
        assert_eq!(pixels_for_area(&source, 9, 6).dimensions(), (9, 6));
    }
}
