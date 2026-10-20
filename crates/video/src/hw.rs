//! Hardware decoding support: policy parsing, device ownership, format selection and frame download.
use std::ffi::c_int;
use std::ptr;

use ff::ffi::{AVBufferRef, AVCodecContext, AVPixelFormat};
use ffmpeg_next as ff;

use crate::decoder::VideoError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwKind {
    VideoToolbox,
    D3d11va,
    Vaapi,
}

impl HwKind {
    /// The hardware API of this OS, if any.
    pub fn native() -> Option<HwKind> {
        if cfg!(target_os = "macos") {
            Some(HwKind::VideoToolbox)
        } else if cfg!(target_os = "windows") {
            Some(HwKind::D3d11va)
        } else if cfg!(target_os = "linux") {
            Some(HwKind::Vaapi)
        } else {
            None
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            HwKind::VideoToolbox => "videotoolbox",
            HwKind::D3d11va => "d3d11va",
            HwKind::Vaapi => "vaapi",
        }
    }

    pub(crate) fn device_type(self) -> ff::ffi::AVHWDeviceType {
        match self {
            HwKind::VideoToolbox => ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
            HwKind::D3d11va => ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA,
            HwKind::Vaapi => ff::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
        }
    }
}

impl std::fmt::Display for HwKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HwKind::VideoToolbox => "VideoToolbox",
            HwKind::D3d11va => "D3D11VA",
            HwKind::Vaapi => "VAAPI",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwPolicy {
    Auto,
    Off,
    Only(HwKind),
}

impl HwPolicy {
    pub fn parse(value: Option<&str>) -> HwPolicy {
        let Some(v) = value else {
            return HwPolicy::Auto;
        };
        let v = v.trim().to_ascii_lowercase();
        match v.as_str() {
            "0" | "off" | "no" | "false" | "sw" | "software" => HwPolicy::Off,
            "videotoolbox" => HwPolicy::Only(HwKind::VideoToolbox),
            "d3d11va" => HwPolicy::Only(HwKind::D3d11va),
            "vaapi" => HwPolicy::Only(HwKind::Vaapi),
            _ => HwPolicy::Auto,
        }
    }

    pub fn from_env() -> HwPolicy {
        let value = std::env::var("TRANSOM_HWDEC").ok();
        let policy = HwPolicy::parse(value.as_deref());
        if policy == HwPolicy::Auto {
            if let Some(v) = value.as_deref() {
                let t = v.trim();
                if !t.is_empty() && t != "1" && !t.eq_ignore_ascii_case("auto") {
                    log::warn!("TRANSOM_HWDEC={v:?} not recognised; using automatic selection");
                }
            }
        }
        policy
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecoderConfig {
    pub hw: HwPolicy,
    /// Software frame threads; below 2 means slice threading with low delay.
    pub sw_frame_threads: u32,
}

impl DecoderConfig {
    pub fn from_env() -> Self {
        DecoderConfig {
            hw: HwPolicy::from_env(),
            sw_frame_threads: parse_frame_threads(
                std::env::var("TRANSOM_SWDEC_FRAME_THREADS").ok().as_deref(),
            ),
        }
    }

    pub fn software() -> Self {
        DecoderConfig {
            hw: HwPolicy::Off,
            sw_frame_threads: 0,
        }
    }
}

/// Trimmed u32; None, invalid, 0 and 1 give 0; 2..=16 as is; larger values are clamped to 16.
pub fn parse_frame_threads(value: Option<&str>) -> u32 {
    match value.and_then(|v| v.trim().parse::<u32>().ok()) {
        Some(n) if n >= 2 => n.min(16),
        _ => 0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Hardware(HwKind),
    Software,
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Backend::Hardware(k) => write!(f, "{k} (hardware)"),
            Backend::Software => f.write_str("software"),
        }
    }
}

/// The pixel format of the first device-context hwaccel config of `codec` for `kind`.
pub(crate) fn hw_pix_fmt(codec: &ff::Codec, kind: HwKind) -> Option<AVPixelFormat> {
    let method = ff::ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as c_int;
    for i in 0.. {
        // SAFETY: the codec pointer is valid; FFmpeg returns null past the last config.
        let cfg = unsafe { ff::ffi::avcodec_get_hw_config(codec.as_ptr(), i) };
        if cfg.is_null() {
            return None;
        }
        // SAFETY: non-null, points to static codec data.
        let cfg = unsafe { &*cfg };
        if cfg.methods & method != 0 && cfg.device_type == kind.device_type() {
            return Some(cfg.pix_fmt);
        }
    }
    None
}

/// Owns one reference to an FFmpeg hardware device.
pub(crate) struct HwDevice {
    buf: *mut AVBufferRef,
    pub kind: HwKind,
}

// SAFETY: AVBufferRef reference counting is atomic, and the device is only used from the thread
// that owns the Decoder (contexts hold their own references).
unsafe impl Send for HwDevice {}

impl HwDevice {
    pub(crate) fn create(kind: HwKind) -> Result<HwDevice, VideoError> {
        let mut p: *mut AVBufferRef = ptr::null_mut();
        // SAFETY: `p` is a valid out pointer; a null device string selects the default device.
        let ret = unsafe {
            ff::ffi::av_hwdevice_ctx_create(
                &mut p,
                kind.device_type(),
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        };
        if ret < 0 {
            return Err(VideoError::Ffmpeg(format!(
                "{kind} device: {}",
                ff::Error::from(ret)
            )));
        }
        if p.is_null() {
            return Err(VideoError::Ffmpeg(format!("{kind} device: null device")));
        }
        Ok(HwDevice { buf: p, kind })
    }

    /// A new reference to the device; the caller must null-check it and owns it.
    pub(crate) fn new_ref(&self) -> *mut AVBufferRef {
        // SAFETY: `buf` is a valid reference owned by self.
        unsafe { ff::ffi::av_buffer_ref(self.buf) }
    }
}

impl Drop for HwDevice {
    fn drop(&mut self) {
        // SAFETY: `buf` is owned by self and unreffed exactly once.
        unsafe { ff::ffi::av_buffer_unref(&mut self.buf) }
    }
}

/// get_format callback: the wanted hardware format (stored in `opaque`) if offered, else the last
/// entry of the list (the software format by FFmpeg convention), else NONE.
///
/// # Safety
/// Called by libavcodec with a valid context and a NONE-terminated list.
/// No logging, allocation or panics: unwinding out of extern "C" aborts.
/// Reading `*fmts.add(j)` as the enum is acceptable: the value comes from the same FFmpeg whose
/// headers generated the bindings.
pub(crate) unsafe extern "C" fn pick_format(
    s: *mut AVCodecContext,
    fmts: *const AVPixelFormat,
) -> AVPixelFormat {
    if s.is_null() || fmts.is_null() {
        return AVPixelFormat::AV_PIX_FMT_NONE;
    }
    let wanted = (*s).opaque as isize as c_int;
    let raw = fmts as *const c_int;
    let none = AVPixelFormat::AV_PIX_FMT_NONE as c_int;
    let mut i = 0usize;
    let mut last: Option<usize> = None;
    while *raw.add(i) != none {
        if *raw.add(i) == wanted {
            return *fmts.add(i);
        }
        last = Some(i);
        i += 1;
    }
    match last {
        Some(j) => *fmts.add(j),
        None => AVPixelFormat::AV_PIX_FMT_NONE,
    }
}

/// Downloads a hardware frame to CPU memory (NV12 for 8-bit 4:2:0). No swscale.
pub(crate) fn download(src: &ff::frame::Video) -> Result<ff::frame::Video, VideoError> {
    let mut dst = ff::frame::Video::empty();
    // SAFETY: both frames are valid; dst is empty so FFmpeg picks the transfer format and allocates.
    let ret = unsafe { ff::ffi::av_hwframe_transfer_data(dst.as_mut_ptr(), src.as_ptr(), 0) };
    if ret < 0 {
        return Err(ff::Error::from(ret).into());
    }
    Ok(dst)
}

pub(crate) fn is_hw_frame(f: &ff::frame::Video) -> bool {
    // SAFETY: the frame pointer is valid for the lifetime of `f`.
    unsafe { !(*f.as_ptr()).hw_frames_ctx.is_null() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_off_values() {
        for v in ["0", " off ", "NO", "false", "sw", "Software"] {
            assert_eq!(HwPolicy::parse(Some(v)), HwPolicy::Off, "{v}");
        }
    }

    #[test]
    fn parse_auto_values() {
        for v in [None, Some(""), Some("1"), Some("auto"), Some("bogus")] {
            assert_eq!(HwPolicy::parse(v), HwPolicy::Auto, "{v:?}");
        }
    }

    #[test]
    fn parse_kinds() {
        assert_eq!(
            HwPolicy::parse(Some("VideoToolbox")),
            HwPolicy::Only(HwKind::VideoToolbox)
        );
        assert_eq!(
            HwPolicy::parse(Some("d3d11va")),
            HwPolicy::Only(HwKind::D3d11va)
        );
        assert_eq!(
            HwPolicy::parse(Some("vaapi")),
            HwPolicy::Only(HwKind::Vaapi)
        );
    }

    #[test]
    fn parse_frame_threads_values() {
        assert_eq!(parse_frame_threads(None), 0);
        assert_eq!(parse_frame_threads(Some("0")), 0);
        assert_eq!(parse_frame_threads(Some("1")), 0);
        assert_eq!(parse_frame_threads(Some("x")), 0);
        assert_eq!(parse_frame_threads(Some("4")), 4);
        assert_eq!(parse_frame_threads(Some(" 8 ")), 8);
        assert_eq!(parse_frame_threads(Some("99")), 16);
    }

    #[test]
    fn native_kind_matches_os() {
        let expected = if cfg!(target_os = "macos") {
            Some(HwKind::VideoToolbox)
        } else if cfg!(target_os = "windows") {
            Some(HwKind::D3d11va)
        } else if cfg!(target_os = "linux") {
            Some(HwKind::Vaapi)
        } else {
            None
        };
        assert_eq!(HwKind::native(), expected);
    }

    #[test]
    fn foreign_device_creation_fails() {
        crate::init().unwrap();
        let foreign = if cfg!(target_os = "macos") {
            HwKind::D3d11va
        } else {
            HwKind::VideoToolbox
        };
        assert!(HwDevice::create(foreign).is_err());
    }
}
