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
/// Decoded size limits: an icon is shown small, a thumbnail a little larger (4 bytes a pixel).
const MAX_ICON_SIDE: u32 = 256;
const MAX_THUMB_SIDE: u32 = 512;
const MAX_ALLOC: u64 = 16 << 20;
/// All decoded textures together stay under this; later pictures are skipped.
const MAX_TOTAL_BYTES: usize = 128 << 20;

pub struct ImageCache {
    /// Decoded bytes held by `thumbs` and `icons`.
    bytes: usize,
    cap: usize,
    /// By window ID: the picture's generation, its texture and its size in pixels.
    thumbs: HashMap<u32, (u64, TextureHandle, Vec2)>,
    icons: HashMap<String, (TextureHandle, Vec2)>,
    /// Never retried: (window ID, generation) of thumbnails and bundle IDs of icons.
    failed_thumbs: HashSet<(u32, u64)>,
    failed_icons: HashSet<String>,
}

impl Default for ImageCache {
    fn default() -> Self {
        ImageCache::with_cap(MAX_TOTAL_BYTES)
    }
}

fn image_bytes(size: Vec2) -> usize {
    (size.x as usize) * (size.y as usize) * 4
}

/// Decodes untrusted PNG or JPEG bytes to RGBA, within the size limits.
fn decode(bytes: &[u8], max_side: u32) -> Option<ColorImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_side);
    limits.max_image_height = Some(max_side);
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

    /// A cache whose decoded textures may total `cap` bytes.
    pub fn with_cap(cap: usize) -> ImageCache {
        ImageCache {
            bytes: 0,
            cap,
            thumbs: HashMap::new(),
            icons: HashMap::new(),
            failed_thumbs: HashSet::new(),
            failed_icons: HashSet::new(),
        }
    }

    /// Decoded bytes currently held.
    pub fn decoded_bytes(&self) -> usize {
        self.bytes
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
        // Only icons of apps that have a window are decoded.
        let wanted: HashSet<&str> = session
            .windows
            .iter()
            .map(|w| w.bundle_id.as_str())
            .collect();
        let keep = |b: &str| wanted.contains(b) && session.icons.contains_key(b);
        self.icons.retain(|b, _| keep(b));
        self.failed_icons.retain(|b| keep(b));
        self.bytes = self
            .thumbs
            .values()
            .map(|e| image_bytes(e.2))
            .sum::<usize>()
            + self.icons.values().map(|e| image_bytes(e.1)).sum::<usize>();

        let mut budget = DECODES_PER_CALL;
        let mut left = false;
        for (id, t) in &session.thumbnails {
            if self.thumbs.get(id).is_some_and(|e| e.0 == t.generation)
                || self.failed_thumbs.contains(&(*id, t.generation))
            {
                continue;
            }
            if self.bytes >= self.cap {
                continue;
            }
            if budget == 0 {
                left = true;
                continue;
            }
            budget -= 1;
            match decode(&t.jpeg, MAX_THUMB_SIDE) {
                Some(img) => {
                    let size = Vec2::new(img.size[0] as f32, img.size[1] as f32);
                    if self.bytes + image_bytes(size) > self.cap {
                        log::debug!("a thumbnail was skipped: the picture memory is full");
                        self.failed_thumbs.insert((*id, t.generation));
                        continue;
                    }
                    let tex = ctx.load_texture(
                        format!("thumb-{id}-{}", t.generation),
                        img,
                        TextureOptions::LINEAR,
                    );
                    self.bytes += image_bytes(size);
                    if let Some(old) = self.thumbs.insert(*id, (t.generation, tex, size)) {
                        self.bytes -= image_bytes(old.2);
                    }
                }
                None => {
                    log::debug!("a thumbnail could not be decoded");
                    self.failed_thumbs.insert((*id, t.generation));
                }
            }
        }
        for (bundle, png) in &session.icons {
            if !wanted.contains(bundle.as_str())
                || self.icons.contains_key(bundle)
                || self.failed_icons.contains(bundle)
            {
                continue;
            }
            if self.bytes >= self.cap {
                continue;
            }
            if budget == 0 {
                left = true;
                continue;
            }
            budget -= 1;
            match decode(png, MAX_ICON_SIDE) {
                Some(img) => {
                    let size = Vec2::new(img.size[0] as f32, img.size[1] as f32);
                    if self.bytes + image_bytes(size) > self.cap {
                        log::debug!("an app icon was skipped: the picture memory is full");
                        self.failed_icons.insert(bundle.clone());
                        continue;
                    }
                    let tex =
                        ctx.load_texture(format!("icon-{bundle}"), img, TextureOptions::LINEAR);
                    self.bytes += image_bytes(size);
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
        self.bytes = 0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_state::Thumb;
    use std::sync::Arc;
    use transom_proto::messages::WindowInfo;

    fn png(w: u32, h: u32) -> Arc<[u8]> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out.into()
    }

    fn window(id: u32, bundle: &str) -> WindowInfo {
        WindowInfo {
            id,
            title: "t".into(),
            app_name: "a".into(),
            bundle_id: bundle.into(),
            width: 1,
            height: 1,
        }
    }

    fn session() -> SessionState {
        SessionState::new("Mac")
    }

    fn thumb(s: &mut SessionState, id: u32, generation: u64, w: u32, h: u32) {
        s.thumbnails.insert(
            id,
            Thumb {
                jpeg: png(w, h),
                generation,
            },
        );
    }

    #[test]
    fn size_limits_for_icons_and_thumbnails() {
        let ctx = egui::Context::default();
        let mut c = ImageCache::new();
        let mut s = session();
        s.windows = vec![window(1, "a"), window(2, "b")];
        s.icons.insert("a".into(), png(256, 256));
        s.icons.insert("b".into(), png(257, 16));
        thumb(&mut s, 1, 1, 512, 512);
        thumb(&mut s, 2, 1, 513, 16);
        assert!(!c.prepare(&ctx, &s));
        assert!(c.icon("a").is_some());
        assert!(c.icon("b").is_none());
        assert!(c.thumbnail(1).is_some());
        assert!(c.thumbnail(2).is_none());
        assert_eq!(c.decoded_bytes(), 256 * 256 * 4 + 512 * 512 * 4);
    }

    #[test]
    fn only_icons_of_listed_windows_are_decoded() {
        let ctx = egui::Context::default();
        let mut c = ImageCache::new();
        let mut s = session();
        s.windows = vec![window(1, "a")];
        s.icons.insert("a".into(), png(8, 8));
        s.icons.insert("zzz".into(), png(8, 8));
        c.prepare(&ctx, &s);
        assert!(c.icon("a").is_some());
        assert!(c.icon("zzz").is_none());
        assert_eq!(c.decoded_bytes(), 8 * 8 * 4);
        // The window goes away: its icon is dropped and the count follows.
        s.windows.clear();
        c.prepare(&ctx, &s);
        assert!(c.icon("a").is_none());
        assert_eq!(c.decoded_bytes(), 0);
    }

    #[test]
    fn total_bytes_are_capped_without_a_repaint_loop() {
        let ctx = egui::Context::default();
        // Room for two 16x16 pictures (1 KiB each) and no third.
        let mut c = ImageCache::with_cap(2 * 16 * 16 * 4);
        let mut s = session();
        for id in 1..=3 {
            thumb(&mut s, id, 1, 16, 16);
        }
        assert!(
            !c.prepare(&ctx, &s),
            "skipped pictures must not ask for frames"
        );
        let held = (1..=3).filter(|i| c.thumbnail(*i).is_some()).count();
        assert_eq!(held, 2);
        assert_eq!(c.decoded_bytes(), 2 * 16 * 16 * 4);
        assert!(!c.prepare(&ctx, &s));
        // A newer generation replaces the old one without double counting.
        let id = (1..=3).find(|i| c.thumbnail(*i).is_some()).unwrap();
        thumb(&mut s, id, 2, 16, 16);
        c.prepare(&ctx, &s);
        // (Which picture gets the freed room depends on the map order.)
        let held = (1..=3).filter(|i| c.thumbnail(*i).is_some()).count();
        assert_eq!(held, 2);
        assert_eq!(c.decoded_bytes(), 2 * 16 * 16 * 4);
        c.clear();
        assert_eq!(c.decoded_bytes(), 0);
    }
}
