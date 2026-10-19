//! Where the video frame sits in the window, and what a window position means in the frame.

/// A rectangle in physical pixels of the window's surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where a `video`-sized frame is drawn inside a `surface` (both width x height in physical
/// pixels): scaled to fit, centred, its aspect kept (black bars on two sides). None while either
/// size has a zero side.
pub fn letterbox(surface: (u32, u32), video: (u32, u32)) -> Option<Rect> {
    let (sw, sh) = (f64::from(surface.0), f64::from(surface.1));
    let (vw, vh) = (f64::from(video.0), f64::from(video.1));
    if sw <= 0.0 || sh <= 0.0 || vw <= 0.0 || vh <= 0.0 {
        return None;
    }
    let scale = (sw / vw).min(sh / vh);
    let (w, h) = (vw * scale, vh * scale);
    Some(Rect {
        x: (sw - w) / 2.0,
        y: (sh - h) / 2.0,
        width: w,
        height: h,
    })
}

impl Rect {
    /// The position (px, py), in the same physical pixels, as a fraction of this rectangle:
    /// (0, 0) its top left, (1, 1) its bottom right, beyond 0..1 outside it. This is the space
    /// every Sill pointer and scroll position is in (upstream Input.swift).
    pub fn fraction(&self, px: f64, py: f64) -> (f64, f64) {
        ((px - self.x) / self.width, (py - self.y) / self.height)
    }
}

/// Whether a fraction pair lies on the frame (both in 0..=1).
pub fn is_inside(fraction: (f64, f64)) -> bool {
    (0.0..=1.0).contains(&fraction.0) && (0.0..=1.0).contains(&fraction.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_pillarboxes_a_narrow_frame() {
        let r = letterbox((2000, 1000), (1000, 1000)).unwrap();
        assert_eq!(
            r,
            Rect {
                x: 500.0,
                y: 0.0,
                width: 1000.0,
                height: 1000.0
            }
        );
    }

    #[test]
    fn letterbox_bars_top_and_bottom_for_a_wide_frame() {
        let r = letterbox((1000, 1000), (2000, 1000)).unwrap();
        assert_eq!(
            r,
            Rect {
                x: 0.0,
                y: 250.0,
                width: 1000.0,
                height: 500.0
            }
        );
    }

    #[test]
    fn letterbox_needs_both_sizes() {
        assert!(letterbox((0, 10), (10, 10)).is_none());
        assert!(letterbox((10, 10), (10, 0)).is_none());
    }

    #[test]
    fn fraction_maps_corners_and_outside() {
        let r = Rect {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
        };
        assert_eq!(r.fraction(100.0, 50.0), (0.0, 0.0));
        assert_eq!(r.fraction(300.0, 150.0), (1.0, 1.0));
        assert_eq!(r.fraction(200.0, 100.0), (0.5, 0.5));
        assert!(!is_inside(r.fraction(99.0, 100.0)));
        assert!(is_inside(r.fraction(300.0, 150.0)));
    }
}
