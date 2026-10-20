//! The frame type the renderer takes, and the validation that keeps a malformed frame from ever
//! reaching `write_texture` (which would be a wgpu validation panic).

/// The chroma planes of a 4:2:0 frame.
pub enum Chroma<'a> {
    /// Two planes, each ceil(w/2) x ceil(h/2) bytes.
    I420 {
        u: &'a [u8],
        u_stride: u32,
        v: &'a [u8],
        v_stride: u32,
    },
    /// One interleaved CbCr plane: ceil(w/2) byte pairs x ceil(h/2) rows.
    Nv12 { uv: &'a [u8], uv_stride: u32 },
}

/// One decoded picture as the renderer takes it: 8-bit 4:2:0, each row `stride` bytes apart
/// (stride >= the plane's row bytes). Chroma is ((width + 1) / 2) x ((height + 1) / 2).
pub struct YuvFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub y: &'a [u8],
    pub y_stride: u32,
    pub chroma: Chroma<'a>,
    /// Full (0-255) rather than limited (16-235) range.
    pub full_range: bool,
    /// BT.601 matrix rather than BT.709.
    pub bt601: bool,
}

/// Why a frame cannot be uploaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Width or height is 0.
    Empty,
    /// The named plane's stride is smaller than its row bytes.
    StrideTooSmall(&'static str),
    /// The named plane's buffer is shorter than `stride * (rows - 1) + row_bytes`.
    ShortPlane(&'static str),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Empty => write!(f, "frame has no pixels"),
            FrameError::StrideTooSmall(p) => write!(f, "{p} plane stride is smaller than a row"),
            FrameError::ShortPlane(p) => write!(f, "{p} plane is shorter than its stride and rows"),
        }
    }
}

impl std::error::Error for FrameError {}

/// One validated plane, ready for a texture and a `write_texture`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlaneCopy<'a> {
    pub data: &'a [u8],
    pub stride: u32,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
}

fn check<'a>(
    name: &'static str,
    data: &'a [u8],
    stride: u32,
    row_bytes: u32,
    rows: u32,
    width: u32,
    format: wgpu::TextureFormat,
) -> Result<PlaneCopy<'a>, FrameError> {
    if stride < row_bytes {
        return Err(FrameError::StrideTooSmall(name));
    }
    // wgpu's own write_texture requirement; rows >= 1 here.
    let need = stride as u64 * (rows as u64 - 1) + row_bytes as u64;
    if (data.len() as u64) < need {
        return Err(FrameError::ShortPlane(name));
    }
    Ok(PlaneCopy {
        data,
        stride,
        width,
        height: rows,
        format,
    })
}

/// The plane copies of `f`, in order: Y, then U and V (I420) or UV (NV12).
pub(crate) fn plane_copies<'a>(f: &YuvFrame<'a>) -> Result<Vec<PlaneCopy<'a>>, FrameError> {
    use wgpu::TextureFormat::{R8Unorm, Rg8Unorm};
    if f.width == 0 || f.height == 0 {
        return Err(FrameError::Empty);
    }
    let (cw, ch) = (f.width.div_ceil(2), f.height.div_ceil(2));
    let mut out = vec![check(
        "y", f.y, f.y_stride, f.width, f.height, f.width, R8Unorm,
    )?];
    match &f.chroma {
        Chroma::I420 {
            u,
            u_stride,
            v,
            v_stride,
        } => {
            out.push(check("u", u, *u_stride, cw, ch, cw, R8Unorm)?);
            out.push(check("v", v, *v_stride, cw, ch, cw, R8Unorm)?);
        }
        Chroma::Nv12 { uv, uv_stride } => {
            // Row bytes are 2 * cw (a byte pair per chroma texel).
            let row = cw.checked_mul(2).ok_or(FrameError::StrideTooSmall("uv"))?;
            out.push(check("uv", uv, *uv_stride, row, ch, cw, Rg8Unorm)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::TextureFormat::{R8Unorm, Rg8Unorm};

    fn need(stride: usize, rows: usize, row_bytes: usize) -> usize {
        stride * (rows - 1) + row_bytes
    }

    fn i420<'a>(
        w: u32,
        h: u32,
        y: &'a [u8],
        ys: u32,
        u: &'a [u8],
        v: &'a [u8],
        cs: u32,
    ) -> YuvFrame<'a> {
        YuvFrame {
            width: w,
            height: h,
            y,
            y_stride: ys,
            chroma: Chroma::I420 {
                u,
                u_stride: cs,
                v,
                v_stride: cs,
            },
            full_range: true,
            bt601: false,
        }
    }

    fn nv12<'a>(w: u32, h: u32, y: &'a [u8], ys: u32, uv: &'a [u8], uvs: u32) -> YuvFrame<'a> {
        YuvFrame {
            width: w,
            height: h,
            y,
            y_stride: ys,
            chroma: Chroma::Nv12 { uv, uv_stride: uvs },
            full_range: true,
            bt601: false,
        }
    }

    #[test]
    fn i420_odd_dims() {
        let y = vec![0u8; need(160, 97, 129)];
        let c = vec![0u8; need(80, 49, 65)];
        let copies = plane_copies(&i420(129, 97, &y, 160, &c, &c, 80)).unwrap();
        let got: Vec<_> = copies
            .iter()
            .map(|c| (c.width, c.height, c.format))
            .collect();
        assert_eq!(
            got,
            vec![(129, 97, R8Unorm), (65, 49, R8Unorm), (65, 49, R8Unorm)]
        );
    }

    #[test]
    fn nv12_odd_dims() {
        let y = vec![0u8; need(160, 97, 129)];
        let uv = vec![0u8; need(160, 49, 130)];
        let copies = plane_copies(&nv12(129, 97, &y, 160, &uv, 160)).unwrap();
        assert_eq!(copies.len(), 2);
        let c = &copies[1];
        assert_eq!((c.width, c.height, c.format), (65, 49, Rg8Unorm));
        assert_eq!(c.stride, 160);
    }

    #[test]
    fn nv12_stride_below_pair_bytes() {
        let y = vec![0u8; need(160, 97, 129)];
        let uv = vec![0u8; 129 * 49];
        let e = plane_copies(&nv12(129, 97, &y, 160, &uv, 129)).unwrap_err();
        assert_eq!(e, FrameError::StrideTooSmall("uv"));
    }

    #[test]
    fn y_stride_below_width() {
        let y = vec![0u8; 128 * 97];
        let c = vec![0u8; need(80, 49, 65)];
        let e = plane_copies(&i420(129, 97, &y, 128, &c, &c, 80)).unwrap_err();
        assert_eq!(e, FrameError::StrideTooSmall("y"));
    }

    #[test]
    fn short_plane() {
        let y = vec![0u8; need(160, 97, 129)];
        let u = vec![0u8; need(80, 49, 65)];
        let v = vec![0u8; need(80, 49, 65) - 1];
        let e = plane_copies(&i420(129, 97, &y, 160, &u, &v, 80)).unwrap_err();
        assert_eq!(e, FrameError::ShortPlane("v"));
    }

    #[test]
    fn tight_even_ok() {
        let y = vec![0u8; 128 * 96];
        let c = vec![0u8; 64 * 48];
        assert!(plane_copies(&i420(128, 96, &y, 128, &c, &c, 64)).is_ok());
    }

    #[test]
    fn empty_rejected() {
        let e = plane_copies(&i420(0, 96, &[], 0, &[], &[], 0)).unwrap_err();
        assert_eq!(e, FrameError::Empty);
    }

    #[test]
    fn huge_stride_no_overflow() {
        let y = vec![0u8; 1000];
        let c = vec![0u8; 100];
        let e = plane_copies(&i420(129, 97, &y, u32::MAX, &c, &c, 80)).unwrap_err();
        assert_eq!(e, FrameError::ShortPlane("y"));
    }
}
