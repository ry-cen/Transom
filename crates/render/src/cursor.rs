//! Turns the Mac's cursor shape (a PNG plus points-based hotspot and size) into a window cursor.
use image::imageops::{resize, FilterType};
use image::ImageFormat;

/// A cursor bitmap at the window's physical scale, ready for `winit::window::CustomCursor`.
pub struct CursorImage {
    pub rgba: Vec<u8>,
    pub width: u16,
    pub height: u16,
    pub hot_x: u16,
    pub hot_y: u16,
}

fn scaled_side(points: f32, scale_factor: f64) -> u32 {
    ((f64::from(points) * scale_factor).round().clamp(1.0, 256.0)) as u32
}

fn scaled_hot(points: f32, scale_factor: f64, side: u32) -> u16 {
    let v = (f64::from(points) * scale_factor).round();
    let max = f64::from(side.saturating_sub(1));
    // NaN clamps to 0 via the comparison below.
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, max) };
    v as u16
}

/// The Mac's cursor at this window's scale: PNG decoded (image::load_from_memory_with_format, Png),
/// resized with imageops::resize(FilterType::Triangle) to round(width_pt * scale_factor) x
/// round(height_pt * scale_factor), each side clamped to 1..=256; hotspot = round(hot_pt *
/// scale_factor) clamped to size-1. None if the PNG does not decode, a size is not finite or <= 0,
/// or scale_factor <= 0.
pub fn cursor_image(
    png: &[u8],
    hot: (f32, f32),
    size_points: (f32, f32),
    scale_factor: f64,
) -> Option<CursorImage> {
    let (wp, hp) = size_points;
    if !wp.is_finite() || !hp.is_finite() || wp <= 0.0 || hp <= 0.0 {
        return None;
    }
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    let img = image::load_from_memory_with_format(png, ImageFormat::Png).ok()?;
    let w = scaled_side(wp, scale_factor);
    let h = scaled_side(hp, scale_factor);
    let rgba = resize(&img.to_rgba8(), w, h, FilterType::Triangle);
    Some(CursorImage {
        rgba: rgba.into_raw(),
        width: w as u16,
        height: h as u16,
        hot_x: scaled_hot(hot.0, scale_factor, w),
        hot_y: scaled_hot(hot.1, scale_factor, h),
    })
}

impl CursorImage {
    /// The winit cursor source, or None if winit rejects the bitmap.
    pub fn to_source(&self) -> Option<winit::window::CustomCursorSource> {
        winit::window::CustomCursor::from_rgba(
            self.rgba.clone(),
            self.width,
            self.height,
            self.hot_x,
            self.hot_y,
        )
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::io::Cursor;

    fn png4() -> Vec<u8> {
        let img = RgbaImage::from_pixel(4, 4, Rgba([255, 0, 0, 255]));
        let mut v = Vec::new();
        img.write_to(&mut Cursor::new(&mut v), ImageFormat::Png)
            .unwrap();
        v
    }

    #[test]
    fn scales_to_physical() {
        let c = cursor_image(&png4(), (1.0, 1.0), (2.0, 2.0), 2.0).unwrap();
        assert_eq!((c.width, c.height), (4, 4));
        assert_eq!((c.hot_x, c.hot_y), (2, 2));
    }

    #[test]
    fn downscales_at_1x() {
        let c = cursor_image(&png4(), (0.0, 0.0), (2.0, 2.0), 1.0).unwrap();
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(c.rgba.len(), 16);
    }

    #[test]
    fn bad_png_is_none() {
        assert!(cursor_image(&[1, 2, 3], (0.0, 0.0), (2.0, 2.0), 1.0).is_none());
    }

    #[test]
    fn zero_size_is_none() {
        assert!(cursor_image(&png4(), (0.0, 0.0), (0.0, 2.0), 1.0).is_none());
        assert!(cursor_image(&png4(), (0.0, 0.0), (2.0, 2.0), 0.0).is_none());
    }

    #[test]
    fn hotspot_clamped() {
        let c = cursor_image(&png4(), (10.0, 10.0), (2.0, 2.0), 1.0).unwrap();
        assert_eq!((c.hot_x, c.hot_y), (1, 1));
    }
}
