use std::ffi::{c_int, c_void};
use std::sync::OnceLock;

use ff::ffi::AVPixelFormat;
use ffmpeg_next as ff;
use transom_proto::video::VideoPacket;

use crate::annexb::length_prefixed_to_annexb;
use crate::frame::{colour_of, from_frame, Scaler};
use crate::hw::{self, hw_pix_fmt, Backend, DecoderConfig, HwDevice, HwKind, HwPolicy};
use crate::DecodedFrame;

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

pub struct Decoder {
    dec: ff::decoder::Video,
    hw: Option<(HwDevice, AVPixelFormat)>,
    hw_disabled: bool,
    hw_errors: u32,
    frame_threads: u32,
    shown: Option<Backend>,
    sets: Option<(u8, Vec<Vec<u8>>)>,
    waiting_for_keyframe: bool,
    scaler: Option<Scaler>,
    buf: Vec<u8>,
}

fn open_context(
    hw: Option<(&HwDevice, AVPixelFormat)>,
    frame_threads: u32,
) -> Result<ff::decoder::Video, VideoError> {
    let codec = ff::codec::decoder::find(ff::codec::Id::HEVC)
        .ok_or_else(|| VideoError::Ffmpeg("no HEVC decoder".into()))?;
    let mut ctx = ff::codec::context::Context::new_with_codec(codec);
    let frame_threading = hw.is_none() && frame_threads >= 2;
    if frame_threading {
        ctx.set_threading(ff::threading::Config {
            kind: ff::threading::Type::Frame,
            count: frame_threads as usize,
        });
    } else {
        ctx.set_threading(ff::threading::Config {
            kind: ff::threading::Type::Slice,
            count: 0,
        });
        // SAFETY: the context pointer is valid and owned by `ctx`; only a flag is set before opening.
        unsafe {
            (*ctx.as_mut_ptr()).flags |= ff::ffi::AV_CODEC_FLAG_LOW_DELAY as i32;
        }
    }
    if let Some((dev, fmt)) = hw {
        // SAFETY: the context pointer is valid and owned by `ctx`. The context owns `r` from here
        // and frees it in avcodec_free_context; `pick_format` is a plain extern "C" fn.
        unsafe {
            let r = dev.new_ref();
            if r.is_null() {
                return Err(VideoError::Ffmpeg("av_buffer_ref failed".into()));
            }
            let p = ctx.as_mut_ptr();
            (*p).hw_device_ctx = r;
            (*p).get_format = Some(hw::pick_format);
            (*p).opaque = fmt as c_int as isize as *mut c_void;
        }
    }
    Ok(ctx.decoder().video()?)
}

impl Decoder {
    pub fn new() -> Result<Self, VideoError> {
        Self::with_config(DecoderConfig::from_env())
    }

    pub fn with_config(cfg: DecoderConfig) -> Result<Self, VideoError> {
        init()?;
        let kind = match cfg.hw {
            HwPolicy::Off => {
                log::info!("hardware decoding off");
                None
            }
            HwPolicy::Auto => HwKind::native(),
            HwPolicy::Only(k) => Some(k),
        };
        let mut hw = None;
        if let Some(kind) = kind {
            let strict = matches!(cfg.hw, HwPolicy::Only(_));
            match ff::codec::decoder::find(ff::codec::Id::HEVC) {
                None => {}
                Some(codec) => match hw_pix_fmt(&codec, kind) {
                    None => {
                        let msg = format!("{kind}: HEVC decoder has no such hwaccel");
                        if strict {
                            log::warn!("{msg}");
                        } else {
                            log::info!("{msg}");
                        }
                    }
                    Some(fmt) => match HwDevice::create(kind) {
                        Ok(dev) => hw = Some((dev, fmt)),
                        Err(e) => {
                            let msg = format!(
                                "hardware decoding ({kind}) unavailable: {e}; using software"
                            );
                            if strict {
                                log::warn!("{msg}");
                            } else {
                                log::info!("{msg}");
                            }
                        }
                    },
                },
            }
        }
        // Placeholder context, replaced by open_with_fallback below.
        let mut d = Decoder {
            dec: open_context(None, cfg.sw_frame_threads)?,
            hw,
            hw_disabled: false,
            hw_errors: 0,
            frame_threads: cfg.sw_frame_threads,
            shown: None,
            sets: None,
            waiting_for_keyframe: true,
            scaler: None,
            buf: Vec::new(),
        };
        d.open_with_fallback()?;
        match &d.hw {
            Some((dev, _)) => log::info!("video decoder: HEVC via {} (hardware)", dev.kind),
            None if d.frame_threads >= 2 => log::info!(
                "video decoder: HEVC in software ({} frame threads)",
                d.frame_threads
            ),
            None => log::info!("video decoder: HEVC in software (slice threads)"),
        }
        Ok(d)
    }

    /// Backend of the last decoded frame; before the first frame, the configured one.
    pub fn backend(&self) -> Backend {
        if let Some(b) = self.shown {
            return b;
        }
        match &self.hw {
            Some((dev, _)) if !self.hw_disabled => Backend::Hardware(dev.kind),
            _ => Backend::Software,
        }
    }

    fn open_with_fallback(&mut self) -> Result<(), VideoError> {
        if !self.hw_disabled {
            if let Some((dev, fmt)) = &self.hw {
                match open_context(Some((dev, *fmt)), 0) {
                    Ok(d) => {
                        self.dec = d;
                        return Ok(());
                    }
                    Err(e) => {
                        log::warn!(
                            "hardware decoding ({}) could not open: {e}; using software",
                            dev.kind
                        );
                        self.hw = None;
                    }
                }
            }
        }
        self.dec = open_context(None, self.frame_threads)?;
        Ok(())
    }

    fn switch_to_software(&mut self, reason: &str) -> Result<(), VideoError> {
        if let Some((dev, _)) = &self.hw {
            log::warn!(
                "hardware decoding ({}) failed: {reason}; switching to software",
                dev.kind
            );
        }
        self.hw_disabled = true;
        self.hw_errors = 0;
        self.scaler = None;
        self.wait_for_keyframe();
        self.dec = open_context(None, self.frame_threads)?;
        Ok(())
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
    /// A keyframe: Annex B of the stored sets, then of `au`; a delta: Annex B of `au`. send_packet; on Err
    /// on software: wait_for_keyframe() and return Err; on hardware see the fallback policy (switch to
    /// software, re-send a keyframe once). Then receive_frame until it fails, keeping the last frame;
    /// on success with is_keyframe set waiting_for_keyframe = false. Returns the last frame (hardware
    /// frames downloaded to NV12), or Ok(None).
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
        let mut on_hw = self.hw.is_some() && !self.hw_disabled;
        if let Err(e) = self.dec.send_packet(&ff::Packet::copy(&self.buf)) {
            if on_hw && is_keyframe {
                self.switch_to_software(&e.to_string())?;
                on_hw = false;
                if let Err(e2) = self.dec.send_packet(&ff::Packet::copy(&self.buf)) {
                    self.wait_for_keyframe();
                    return Err(e2.into());
                }
            } else {
                if on_hw {
                    self.hw_errors += 1;
                    if self.hw_errors >= 3 {
                        self.switch_to_software(&e.to_string())?;
                    }
                }
                self.wait_for_keyframe();
                return Err(e.into());
            }
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
        let (full_range, bt601) = colour_of(&last);
        let was_hw_frame = hw::is_hw_frame(&last);
        let out = if was_hw_frame {
            match hw::download(&last) {
                Ok(dst) => {
                    drop(last);
                    dst
                }
                Err(e) => {
                    drop(last);
                    self.switch_to_software(&e.to_string())?;
                    self.wait_for_keyframe();
                    return Err(e);
                }
            }
        } else {
            last
        };
        let delivered = out.format();
        let f = match from_frame(out, full_range, bt601, timestamp, &mut self.scaler) {
            Ok(f) => f,
            Err(e) if was_hw_frame => {
                self.switch_to_software(&e.to_string())?;
                self.wait_for_keyframe();
                return Err(e);
            }
            Err(e) => return Err(e),
        };
        if !is_keyframe {
            self.hw_errors = 0;
        }
        let observed = match (&self.hw, was_hw_frame) {
            (Some((dev, _)), true) => Backend::Hardware(dev.kind),
            _ => Backend::Software,
        };
        if self.shown != Some(observed) {
            match (&self.hw, observed) {
                (Some((dev, _)), Backend::Software) if on_hw => log::warn!(
                    "{} did not accept the stream; FFmpeg decodes it in software",
                    dev.kind
                ),
                _ => log::info!(
                    "video decoding: {observed}, {delivered:?} {}x{}",
                    f.width,
                    f.height
                ),
            }
            self.shown = Some(observed);
        }
        Ok(Some(f))
    }

    pub fn wait_for_keyframe(&mut self) {
        self.waiting_for_keyframe = true;
    }

    /// Forget everything: a new decoder context, no sets, waiting for a keyframe. Hardware is retried.
    pub fn reset(&mut self) -> Result<(), VideoError> {
        self.hw_disabled = false;
        self.hw_errors = 0;
        self.shown = None;
        self.sets = None;
        self.scaler = None;
        self.waiting_for_keyframe = true;
        self.open_with_fallback()
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

#[cfg(test)]
mod tests {
    use super::*;
    use transom_proto::video::host_packets_from_annexb;

    #[test]
    fn switch_to_software_resumes_at_next_keyframe() {
        init().unwrap();
        let pk = host_packets_from_annexb(include_bytes!("../tests/fixtures/tiny.hevc"));
        let mut dec = Decoder::with_config(DecoderConfig::software()).unwrap();
        let (mut pics, mut resumed) = (0, None);
        for p in &pk {
            match p {
                VideoPacket::ParameterSets {
                    nal_length_size,
                    sets,
                } => dec.set_parameter_sets(*nal_length_size, sets),
                VideoPacket::Frame {
                    data,
                    is_keyframe,
                    timestamp,
                } => {
                    if pics == 3 {
                        dec.switch_to_software("test").unwrap();
                    }
                    let got = dec.decode(data, *is_keyframe, *timestamp).unwrap();
                    if pics >= 3 && !*is_keyframe && resumed.is_none() {
                        assert!(got.is_none(), "delta decoded before a keyframe");
                    }
                    if pics >= 3 && got.is_some() && resumed.is_none() {
                        assert!(*is_keyframe);
                        assert_eq!(dec.backend(), Backend::Software);
                        resumed = Some(pics);
                    }
                    pics += 1;
                }
                VideoPacket::Reset => {}
            }
        }
        assert!(resumed.is_some(), "frames never resumed");
    }
}
