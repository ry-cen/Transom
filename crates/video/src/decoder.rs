use std::sync::OnceLock;

use ffmpeg_next as ff;
use transom_proto::video::VideoPacket;

use crate::annexb::length_prefixed_to_annexb;

#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("FFmpeg: {0}")]
    Ffmpeg(String),
    #[error("bad access unit: {0}")]
    BadData(&'static str),
}

impl From<ff::Error> for VideoError {
    fn from(e: ff::Error) -> Self {
        VideoError::Ffmpeg(e.to_string())
    }
}

/// ffmpeg init + log level Fatal, once.
pub fn init() -> Result<(), VideoError> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(|| {
        ff::init().map_err(|e| e.to_string())?;
        ff::log::set_level(ff::log::Level::Fatal);
        Ok(())
    })
    .clone()
    .map_err(VideoError::Ffmpeg)
}

/// One decoded picture as three planar 4:2:0 planes.
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub y_stride: u32,
    pub u: Vec<u8>,
    pub u_stride: u32,
    pub v: Vec<u8>,
    pub v_stride: u32,
    pub full_range: bool,
    pub bt601: bool,
    pub timestamp: f64,
}

type Scaler = (ff::format::Pixel, u32, u32, ff::software::scaling::Context);

pub struct Decoder {
    dec: ff::decoder::Video,
    sets: Option<(u8, Vec<Vec<u8>>)>,
    waiting_for_keyframe: bool,
    scaler: Option<Scaler>,
    buf: Vec<u8>,
}

fn open_context() -> Result<ff::decoder::Video, VideoError> {
    let codec = ff::codec::decoder::find(ff::codec::Id::HEVC)
        .ok_or_else(|| VideoError::Ffmpeg("no HEVC decoder".into()))?;
    let mut ctx = ff::codec::context::Context::new_with_codec(codec);
    ctx.set_threading(ff::threading::Config {
        kind: ff::threading::Type::Slice,
        count: 0,
    });
    // SAFETY: the context pointer is valid and owned by `ctx`; only a flag is set before opening.
    unsafe {
        (*ctx.as_mut_ptr()).flags |= ff::ffi::AV_CODEC_FLAG_LOW_DELAY as i32;
    }
    Ok(ctx.decoder().video()?)
}

fn copy_plane(frame: &ff::frame::Video, i: usize, rows: usize) -> Option<(Vec<u8>, u32)> {
    let stride = frame.stride(i);
    let data = frame.data(i).get(..stride * rows)?;
    Some((data.to_vec(), stride as u32))
}

impl Decoder {
    pub fn new() -> Result<Self, VideoError> {
        init()?;
        Ok(Decoder {
            dec: open_context()?,
            sets: None,
            waiting_for_keyframe: true,
            scaler: None,
            buf: Vec::new(),
        })
    }

    /// Same bytes as stored: nothing. Different: store them and wait_for_keyframe().
    pub fn set_parameter_sets(&mut self, nal_length_size: u8, sets: &[Vec<u8>]) {
        if let Some((n, s)) = &self.sets {
            if *n == nal_length_size && s.as_slice() == sets {
                return;
            }
        }
        self.sets = Some((nal_length_size, sets.to_vec()));
        self.wait_for_keyframe();
    }

    /// Ok(None) while no parameter sets are known or while waiting for a keyframe and `is_keyframe` is false.
    /// A keyframe: Annex B of the stored sets, then of `au`; a delta: Annex B of `au`. send_packet; on Err:
    /// wait_for_keyframe() and return Err. Then receive_frame until it fails, keeping the last frame;
    /// on success with is_keyframe set waiting_for_keyframe = false. Returns the last frame converted, or Ok(None).
    pub fn decode(
        &mut self,
        au: &[u8],
        is_keyframe: bool,
        timestamp: f64,
    ) -> Result<Option<DecodedFrame>, VideoError> {
        let Some((nal_len, sets)) = &self.sets else {
            return Ok(None);
        };
        if self.waiting_for_keyframe && !is_keyframe {
            return Ok(None);
        }
        let nal_len = *nal_len;
        self.buf.clear();
        let conv = {
            if is_keyframe {
                for s in sets {
                    self.buf.extend_from_slice(&[0, 0, 0, 1]);
                    self.buf.extend_from_slice(s);
                }
            }
            length_prefixed_to_annexb(au, nal_len, &mut self.buf)
        };
        if let Err(e) = conv {
            self.wait_for_keyframe();
            return Err(e);
        }
        if let Err(e) = self.dec.send_packet(&ff::Packet::copy(&self.buf)) {
            self.wait_for_keyframe();
            return Err(e.into());
        }
        if is_keyframe {
            self.waiting_for_keyframe = false;
        }
        let mut frame = ff::frame::Video::empty();
        let mut last = ff::frame::Video::empty();
        let mut got = false;
        while self.dec.receive_frame(&mut frame).is_ok() {
            std::mem::swap(&mut frame, &mut last);
            got = true;
        }
        if !got {
            return Ok(None);
        }
        self.convert(&last, timestamp).map(Some)
    }

    pub fn wait_for_keyframe(&mut self) {
        self.waiting_for_keyframe = true;
    }

    /// Forget everything: a new decoder context, no sets, waiting for a keyframe.
    pub fn reset(&mut self) -> Result<(), VideoError> {
        self.dec = open_context()?;
        self.sets = None;
        self.scaler = None;
        self.waiting_for_keyframe = true;
        Ok(())
    }

    fn convert(
        &mut self,
        frame: &ff::frame::Video,
        timestamp: f64,
    ) -> Result<DecodedFrame, VideoError> {
        use ff::format::Pixel;
        let (w, h) = (frame.width(), frame.height());
        let fmt = frame.format();
        let full_range = fmt == Pixel::YUVJ420P
            || matches!(
                frame.color_range(),
                ff::color::Range::JPEG | ff::color::Range::Unspecified
            );
        let bt601 = matches!(
            frame.color_space(),
            ff::color::Space::BT470BG | ff::color::Space::SMPTE170M
        );
        let bad = VideoError::BadData("short plane");
        let converted;
        let src = if matches!(fmt, Pixel::YUV420P | Pixel::YUVJ420P) {
            frame
        } else {
            let stale = match &self.scaler {
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
                self.scaler = Some((fmt, w, h, ctx));
            }
            let Some((_, _, _, ctx)) = self.scaler.as_mut() else {
                return Err(bad);
            };
            let mut out = ff::frame::Video::empty();
            ctx.run(frame, &mut out)?;
            converted = out;
            &converted
        };
        let rows_c = (h as usize).div_ceil(2);
        let (y, y_stride) =
            copy_plane(src, 0, h as usize).ok_or(VideoError::BadData("short plane"))?;
        let (u, u_stride) = copy_plane(src, 1, rows_c).ok_or(VideoError::BadData("short plane"))?;
        let (v, v_stride) = copy_plane(src, 2, rows_c).ok_or(VideoError::BadData("short plane"))?;
        Ok(DecodedFrame {
            width: w,
            height: h,
            y,
            y_stride,
            u,
            u_stride,
            v,
            v_stride,
            full_range,
            bt601,
            timestamp,
        })
    }
}

/// Runs a Decoder on a new thread named "transom-video" (created inside the thread). For each packet:
/// ParameterSets -> set_parameter_sets; Frame -> decode; Reset -> reset. A decode Err is logged at debug and the
/// loop continues. A decoded frame is handed to `on_frame` only when `rx.is_empty()` (the newest wins) or
/// after 10 frames in a row were not handed over. Returns when the channel disconnects. An init failure is
/// logged at error and the thread drains and drops packets until disconnect.
pub fn spawn_decoder(
    rx: crossbeam_channel::Receiver<VideoPacket>,
    mut on_frame: impl FnMut(DecodedFrame) + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("transom-video".into())
        .spawn(move || {
            let mut dec = match Decoder::new() {
                Ok(d) => d,
                Err(e) => {
                    log::error!("video decoder init failed: {e}");
                    while rx.recv().is_ok() {}
                    return;
                }
            };
            let mut skipped = 0u32;
            while let Ok(packet) = rx.recv() {
                match packet {
                    VideoPacket::ParameterSets {
                        nal_length_size,
                        sets,
                    } => dec.set_parameter_sets(nal_length_size, &sets),
                    VideoPacket::Frame {
                        data,
                        is_keyframe,
                        timestamp,
                    } => match dec.decode(&data, is_keyframe, timestamp) {
                        Ok(Some(f)) => {
                            if rx.is_empty() || skipped >= 10 {
                                skipped = 0;
                                on_frame(f);
                            } else {
                                skipped += 1;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => log::debug!("decode error: {e}"),
                    },
                    VideoPacket::Reset => {
                        if let Err(e) = dec.reset() {
                            log::error!("video decoder reset failed: {e}");
                        }
                    }
                }
            }
        })
        .expect("spawning the transom-video thread")
}
