//! Lazily decoded, bounded thumbnail and icon textures behind `view::ImageLookup`
//! (docs/ui-spec.md section 8.3). Pictures come from the Mac, so they are untrusted: decoding
//! is size-limited and at most `DECODES_PER_CALL` happen per frame.

use crate::gui::view::{Image, ImageLookup};
use crate::session_state::SessionState;
use egui::{ColorImage, TextureHandle, TextureOptions, Vec2};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;

/// At most this many pictures are decoded per `prepare` call (thumbnails first).
const DECODES_PER_CALL: usize = 8;
const MAX_SIDE: u32 = 1024;
const MAX_ALLOC: u64 = 16 << 20;

#[derive(Default)]
pub struct ImageCache {
    /// By window ID: the picture's generation, its texture and its size in pixels.
    thumbs: HashMap<u32, (u64, TextureHandle, Vec2)>,
    icons: HashMap<String, (TextureHandle, Vec2)>,
    /// Never retried: (window ID, generation) of thumbnails and bundle IDs of icons.
    failed_thumbs: HashSet<(u32, u64)>,
    failed_icons: HashSet<String>,
}

/// Decodes untrusted PNG or JPEG bytes to RGBA, within the size limits.
fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

impl ImageCache {
    pub fn new() -> ImageCache {
        ImageCache::default()
    }

    /// Drops pictures the session no longer has, then decodes up to eight missing ones.
    /// Returns true when work remains, so the caller asks for another frame.
    pub fn prepare(&mut self, ctx: &egui::Context, session: &SessionState) -> bool {
        self.thumbs.retain(|id, (generation, _, _)| {
            session
                .thumbnails
                .get(id)
                .is_some_and(|t| t.generation == *generation)
        });
        self.failed_thumbs.retain(|(id, generation)| {
            session
                .thumbnails
                .get(id)
                .is_some_and(|t| t.generation == *generation)
        });
        self.icons.retain(|b, _| session.icons.contains_key(b));
        self.failed_icons.retain(|b| session.icons.contains_key(b));

        let mut budget = DECODES_PER_CALL;
        let mut left = false;
        for (id, t) in &session.thumbnails {
            if self.thumbs.get(id).is_some_and(|e| e.0 == t.generation)
                || self.failed_thumbs.contains(&(*id, t.generation))
            {
                continue;
            }
            if budget == 0 {
                left = true;
                continue;
            }
            budget -= 1;
            match decode(&t.jpeg) {
                Some(img) => {
                    let size = Vec2::new(img.size[0] as f32, img.size[1] as f32);
                    let tex = ctx.load_texture(
                        format!("thumb-{id}-{}", t.generation),
                        img,
                        TextureOptions::LINEAR,
                    );
                    self.thumbs.insert(*id, (t.generation, tex, size));
                }
                None => {
                    log::debug!("a thumbnail could not be decoded");
                    self.failed_thumbs.insert((*id, t.generation));
                }
            }
        }
        for (bundle, png) in &session.icons {
            if self.icons.contains_key(bundle) || self.failed_icons.contains(bundle) {
                continue;
            }
            if budget == 0 {
                left = true;
                continue;
            }
            budget -= 1;
            match decode(png) {
                Some(img) => {
                    let size = Vec2::new(img.size[0] as f32, img.size[1] as f32);
                    let tex =
                        ctx.load_texture(format!("icon-{bundle}"), img, TextureOptions::LINEAR);
                    self.icons.insert(bundle.clone(), (tex, size));
                }
                None => {
                    log::debug!("an app icon could not be decoded");
                    self.failed_icons.insert(bundle.clone());
                }
            }
        }
        left
    }

    /// Forgets everything (the stream stopped).
    pub fn clear(&mut self) {
        self.thumbs.clear();
        self.icons.clear();
        self.failed_thumbs.clear();
        self.failed_icons.clear();
    }
}

impl ImageLookup for ImageCache {
    fn thumbnail(&self, window_id: u32) -> Option<Image> {
        self.thumbs.get(&window_id).map(|(_, t, size)| Image {
            id: t.id(),
            size: *size,
        })
    }

    fn icon(&self, bundle_id: &str) -> Option<Image> {
        self.icons.get(bundle_id).map(|(t, size)| Image {
            id: t.id(),
            size: *size,
        })
    }
}
