//! Decoded pictures handed to the renderer.
use ffmpeg_next as ff;

use crate::decoder::VideoError;
use crate::hw::is_hw_frame;

pub(crate) type Scaler = (ff::format::Pixel, u32, u32, ff::software::scaling::Context);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    I420,
    Nv12,
}

/// Owns the FFmpeg frame (a refcounted decoder buffer, a hardware download or a swscale output).
/// Planes are 8-bit 4:2:0; chroma is ceil(w/2) x ceil(h/2).
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub full_range: bool,
    pub bt601: bool,
    pub timestamp: f64,
    frame: ff::frame::Video,
    layout: Layout,
}

/// One plane: `data.len() == stride * rows`.
#[derive(Clone, Copy, Debug)]
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub stride: u32,
}

pub enum Planes<'a> {
    I420 {
        y: Plane<'a>,
        u: Plane<'a>,
        v: Plane<'a>,
    },
    Nv12 {
        y: Plane<'a>,
        uv: Plane<'a>,
    },
}

impl DecodedFrame {
    pub fn planes(&self) -> Planes<'_> {
        let h = self.height as usize;
        let plane = |i: usize, rows: usize| {
            let stride = self.frame.stride(i);
            Plane {
                data: &self.frame.data(i)[..stride * rows],
                stride: stride as u32,
            }
        };
        let ch = h.div_ceil(2);
        match self.layout {
            Layout::I420 => Planes::I420 {
                y: plane(0, h),
                u: plane(1, ch),
                v: plane(2, ch),
            },
            Layout::Nv12 => Planes::Nv12 {
                y: plane(0, h),
                uv: plane(1, ch),
            },
        }
    }
}

/// (full_range, bt601) from the decoder's source frame (not a hardware download).
pub(crate) fn colour_of(src: &ff::frame::Video) -> (bool, bool) {
    use ff::format::Pixel;
    let full_range = src.format() == Pixel::YUVJ420P
        || matches!(
            src.color_range(),
            ff::color::Range::JPEG | ff::color::Range::Unspecified
        );
    let bt601 = matches!(
        src.color_space(),
        ff::color::Space::BT470BG | ff::color::Space::SMPTE170M
    );
    (full_range, bt601)
}

/// Checks plane `i` (row bytes x rows) before any ffmpeg-next accessor touches it.
fn plane_ok(frame: &ff::frame::Video, i: usize, row_bytes: usize, rows: usize) -> bool {
    // SAFETY: the frame pointer is valid; only plain fields are read.
    let (linesize, data) = unsafe {
        let p = frame.as_ptr();
        ((*p).linesize[i], (*p).data[i])
    };
    if linesize <= 0 || data.is_null() {
        return false;
    }
    let linesize = linesize as usize;
    linesize >= row_bytes
        && linesize
            .checked_mul(rows)
            .is_some_and(|need| frame.data(i).len() >= need)
}

pub(crate) fn from_frame(
    frame: ff::frame::Video,
    full_range: bool,
    bt601: bool,
    timestamp: f64,
    scaler: &mut Option<Scaler>,
) -> Result<DecodedFrame, VideoError> {
    use ff::format::Pixel;
    let (w, h) = (frame.width(), frame.height());
    if w == 0 || h == 0 {
        return Err(VideoError::BadData("empty frame"));
    }
    if is_hw_frame(&frame) {
        return Err(VideoError::BadData("hardware frame"));
    }
    let fmt = frame.format();
    let (frame, layout) = match fmt {
        Pixel::NV12 => (frame, Layout::Nv12),
        Pixel::YUV420P | Pixel::YUVJ420P => (frame, Layout::I420),
        _ => {
            let stale = match scaler {
                Some((f, sw, sh, _)) => *f != fmt || *sw != w || *sh != h,
                None => true,
            };
            if stale {
                let ctx = ff::software::scaling::Context::get(
                    fmt,
                    w,
                    h,
                    Pixel::YUV420P,
                    w,
                    h,
                    ff::software::scaling::Flags::BILINEAR,
                )?;
                log::info!("video: converting {fmt:?} {w}x{h} with swscale");
                *scaler = Some((fmt, w, h, ctx));
            }
            let Some((_, _, _, ctx)) = scaler.as_mut() else {
                return Err(VideoError::BadData("no scaler"));
            };
            let mut out = ff::frame::Video::empty();
            ctx.run(&frame, &mut out)?;
            (out, Layout::I420)
        }
    };
    let (wu, hu) = (w as usize, h as usize);
    let (cw, ch) = (wu.div_ceil(2), hu.div_ceil(2));
    let ok = match layout {
        Layout::Nv12 => plane_ok(&frame, 0, wu, hu) && plane_ok(&frame, 1, 2 * cw, ch),
        Layout::I420 => {
            plane_ok(&frame, 0, wu, hu)
                && plane_ok(&frame, 1, cw, ch)
                && plane_ok(&frame, 2, cw, ch)
        }
    };
    if !ok {
        return Err(VideoError::BadData("bad plane"));
    }
    Ok(DecodedFrame {
        width: w,
        height: h,
        full_range,
        bt601,
        timestamp,
        frame,
        layout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ff::format::Pixel;

    fn synth(fmt: Pixel) -> ff::frame::Video {
        crate::init().unwrap();
        ff::frame::Video::new(fmt, 129, 97)
    }

    fn fill(f: &mut ff::frame::Video, i: usize, rows: usize, k: usize) {
        let stride = f.stride(i);
        let d = f.data_mut(i);
        for r in 0..rows {
            for c in 0..stride {
                d[r * stride + c] = (r * k + c) as u8;
            }
        }
    }

    #[test]
    fn nv12_odd_dims_layout() {
        let mut f = synth(Pixel::NV12);
        fill(&mut f, 0, 97, 3);
        fill(&mut f, 1, 49, 5);
        let mut sc = None;
        let d = from_frame(f, true, false, 0.0, &mut sc).unwrap();
        let Planes::Nv12 { y, uv } = d.planes() else {
            panic!("not nv12");
        };
        assert!(y.stride > 129, "y stride {}", y.stride);
        assert!(uv.stride >= 130);
        assert_eq!(uv.data.len(), uv.stride as usize * 49);
        assert_eq!(y.data.len(), y.stride as usize * 97);
        assert_eq!(y.data[96 * y.stride as usize + 128], (96 * 3 + 128) as u8);
        assert_eq!(uv.data[48 * uv.stride as usize + 129], (48 * 5 + 129) as u8);
    }

    #[test]
    fn i420_odd_dims_layout() {
        let mut f = synth(Pixel::YUV420P);
        fill(&mut f, 0, 97, 3);
        fill(&mut f, 1, 49, 5);
        fill(&mut f, 2, 49, 7);
        let mut sc = None;
        let d = from_frame(f, true, false, 0.0, &mut sc).unwrap();
        let Planes::I420 { y, u, v } = d.planes() else {
            panic!("not i420");
        };
        assert!(y.stride > 129, "y stride {}", y.stride);
        assert!(u.stride >= 65 && v.stride >= 65);
        assert_eq!(u.data.len(), u.stride as usize * 49);
        assert_eq!(v.data.len(), v.stride as usize * 49);
        assert_eq!(y.data[96 * y.stride as usize + 128], (96 * 3 + 128) as u8);
        assert_eq!(u.data[48 * u.stride as usize + 64], (48 * 5 + 64) as u8);
        assert_eq!(v.data[48 * v.stride as usize + 64], (48 * 7 + 64) as u8);
    }

    #[test]
    fn other_formats_use_scaler() {
        let f = synth(Pixel::YUV444P);
        let mut sc = None;
        let d = from_frame(f, true, false, 0.0, &mut sc).unwrap();
        let Planes::I420 { u, .. } = d.planes() else {
            panic!("not i420");
        };
        assert!(u.data.len() >= u.stride as usize * 49);
        assert!(sc.is_some());
    }

    #[test]
    fn negative_stride_rejected() {
        let mut f = synth(Pixel::NV12);
        // SAFETY: only a plain field of a valid frame is changed.
        unsafe {
            (*f.as_mut_ptr()).linesize[0] = -(*f.as_ptr()).linesize[0];
        }
        let mut sc = None;
        assert!(matches!(
            from_frame(f, true, false, 0.0, &mut sc),
            Err(VideoError::BadData(_))
        ));
    }

    #[test]
    fn zero_size_rejected() {
        let mut sc = None;
        assert!(from_frame(ff::frame::Video::empty(), true, false, 0.0, &mut sc).is_err());
    }
}
